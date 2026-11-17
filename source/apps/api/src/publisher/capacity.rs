//! Per-operation archive admission; never borrows the upgrade runner's emergency pools.
use super::{journal::Operation,store::PublisherStore};
use crate::{config::RuntimeConfig,upgrade::StepError};
use anyhow::{Context,Result};
use std::{collections::{BTreeMap,BTreeSet},fs::File,os::{fd::AsRawFd,unix::fs::MetadataExt},path::{Path,PathBuf}};

pub(super) const OUTPUT:&str="설정 보관 공간이 부족해 적용하지 않았습니다. 현재 설정은 그대로입니다. 디스크 공간을 확보한 뒤 다시 시도하세요.";
pub(super) fn full()->anyhow::Error {StepError{code:"publisher_storage_full",message:OUTPUT,detail:None}.into()}
pub(super) fn stage_error(error:anyhow::Error)->anyhow::Error {
    if error.chain().filter_map(|cause|cause.downcast_ref::<std::io::Error>()).any(|error|matches!(error.raw_os_error(),Some(libc::ENOSPC)|Some(libc::EDQUOT))){full()}else{error}
}
pub(super) fn is_full(error:&anyhow::Error)->bool {error.downcast_ref::<StepError>().is_some_and(|error|error.code=="publisher_storage_full")}
pub(super) fn result()->super::protocol::OpResult {super::protocol::OpResult::Failed{code:"publisher_storage_full".into(),output:OUTPUT.into()}}
pub(super) fn step(error:anyhow::Error)->StepError {
    if is_full(&error){StepError{code:"publisher_storage_full",message:OUTPUT,detail:None}}else{super::store::step_error("publish_failed",error)}
}

pub(super) struct Write {pub path:PathBuf,pub bytes:u64,pub inodes:u64}
pub(super) struct StageBudget<'a> {
    pub config:&'a RuntimeConfig,pub operation:Operation,pub previous:&'a BTreeMap<String,Vec<u8>>,
    pub validation:Vec<Write>,pub validation_dirs:Vec<PathBuf>,pub a8_clear:bool,
}
impl<'a> StageBudget<'a> {
    pub fn new(config:&'a RuntimeConfig,operation:&Operation,previous:&'a BTreeMap<String,Vec<u8>>,a8_clear:bool)->Self {
        Self{config,operation:operation.clone(),previous,validation:Vec::new(),validation_dirs:Vec::new(),a8_clear}
    }
    pub fn planned(config:&'a RuntimeConfig,id:&str,previous:&'a BTreeMap<String,Vec<u8>>,a8_clear:bool)->Result<Self> {
        let mut operation=Operation::planned(id,&super::store::digest(&[]));
        operation.boot_identity=Some(crate::upgrade::boot_identity::current()?);
        operation.previous=Some(id.into());operation.next=Some(id.into());
        Ok(Self::new(config,&operation,previous,a8_clear))
    }
}
#[derive(Clone,Copy)]
pub(super) struct Available {pub bytes:u64,pub inodes:Option<u64>,pub block_size:u64}
#[derive(Debug,Clone,Copy)]
pub(super) struct Need {pub bytes:u64,pub inodes:u64,#[cfg(test)]pub restore_bytes:u64}
impl Need {
    pub fn fits(self,available:Available)->bool {available.bytes>=self.bytes && available.inodes.is_none_or(|count|count>=self.inodes)}
}
#[cfg(test)]
pub(super) type Probe=std::sync::Arc<dyn Fn()->Result<Available>+Send+Sync>;
pub(super) fn available(root:&File)->Result<Available> {
    let mut value=std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe{libc::fstatvfs(root.as_raw_fd(),value.as_mut_ptr())}!=0{return Err(std::io::Error::last_os_error().into());}
    let value=unsafe{value.assume_init()};anyhow::ensure!(value.f_frsize!=0,"Invalid publisher filesystem block size");
    Ok(Available{block_size:value.f_frsize as u64,bytes:(value.f_bavail as u64).checked_mul(value.f_frsize as u64).context("Publisher capacity overflow")?,inodes:if value.f_files==0{None}else{Some(value.f_favail as u64)}})
}
fn device(path:&Path)->Result<u64> {
    let mut existing=path;
    loop {match std::fs::metadata(existing){Ok(meta)=>return Ok(meta.dev()),Err(error)if error.kind()==std::io::ErrorKind::NotFound=>existing=existing.parent().context("Missing capacity ancestor")?,Err(error)=>return Err(error.into())}}
}
fn target(config:&RuntimeConfig,name:&str)->Result<PathBuf> {
    super::store::payload_path(name)?;
    let(root,relative)=if let Some(name)=name.strip_prefix("conf/"){(&config.nginx_conf_dir,name)}else if let Some(name)=name.strip_prefix("stream/"){(&config.nginx_stream_dir,name)}else{(&config.certs_dir,name.strip_prefix("certs/").unwrap())};
    Ok(root.join(relative))
}
fn add(total:&mut u64,value:u64)->Result<()> {*total=total.checked_add(value).ok_or_else(full)?;Ok(())}
fn rounded(bytes:u64,block_size:u64)->Result<u64> {bytes.div_ceil(block_size).checked_mul(block_size).ok_or_else(full)}
pub(super) fn needed(store:&PublisherStore,id:&str,files:&BTreeMap<String,Vec<u8>>,manifest_bytes:usize,budget:&StageBudget<'_>,block_size:u64)->Result<Need> {
    anyhow::ensure!(block_size>0,"Invalid publisher filesystem block size");
    let disk=std::fs::metadata(&store.root)?.dev();let generation=store.root.join("payloads").join(id);
    let mut writes=Vec::new();
    for(name,bytes)in files{writes.push(Write{path:generation.join(name),bytes:bytes.len() as u64,inodes:1});}
    writes.push(Write{path:generation.join("manifest.json"),bytes:manifest_bytes as u64,inodes:1});
    let mut record=budget.operation.clone();record.result=None;record.next=Some(id.into());
    let record_bytes=(serde_json::to_vec(&record)?.len() as u64).checked_add(super::server::MAX_RESPONSE as u64).ok_or_else(full)?;
    writes.push(Write{path:store.root.join("ops").join(format!("{}.json",record.id)),bytes:record_bytes,inodes:2});
    let mut selected=store.manifest()?;selected.previous=selected.current.take();selected.current=Some(id.into());
    writes.push(Write{path:store.root.join("manifest.json"),bytes:serde_json::to_vec(&selected)?.len() as u64,inodes:2});
    #[cfg(test)]
    let mut restore_bytes=0;
    for(_is_restore,set)in [(false,files),(true,budget.previous)] {
        for(name,bytes)in set{let path=target(budget.config,name)?;#[cfg(test)]if _is_restore&&device(path.parent().context("Missing live parent")?)?==disk{add(&mut restore_bytes,rounded(bytes.len() as u64,block_size)?)?;}writes.push(Write{path,bytes:bytes.len() as u64,inodes:1});}
    }
    let mut need=Need{bytes:0,inodes:0,#[cfg(test)]restore_bytes};let mut directories=BTreeSet::new();
    for write in writes.iter().chain(&budget.validation) {
        if device(write.path.parent().context("Missing write parent")?)?!=disk{continue;}
        add(&mut need.bytes,rounded(write.bytes,block_size)?.checked_mul(write.inodes).ok_or_else(full)?)?;add(&mut need.inodes,write.inodes)?;
        let mut parent=write.path.parent();
        while let Some(path)=parent{match std::fs::metadata(path){Ok(_)=>break,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>{directories.insert(path.to_owned());parent=path.parent();},Err(error)=>return Err(error.into())}}
    }
    for directory in &budget.validation_dirs {
        if device(directory)?!=disk{continue;}
        let mut parent=Some(directory.as_path());
        while let Some(path)=parent{match std::fs::metadata(path){Ok(_)=>break,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>{directories.insert(path.to_owned());parent=path.parent();},Err(error)=>return Err(error.into())}}
    }
    add(&mut need.inodes,directories.len() as u64)?;add(&mut need.bytes,(directories.len() as u64).checked_mul(block_size).ok_or_else(full)?)?;Ok(need)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{dispatcher::Dispatcher,journal::{Admission,Phase},protocol::{Actor,Op,OpResult,Request},store::{Origin,FileDigest,PayloadManifest}};
    use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
    fn request(op:Op)->Request {Request{v:1,id:uuid::Uuid::new_v4().to_string(),op,deadline_ms:280000,actor:Actor{via: None,user_id:None,label:None,ip:None}}}
    fn manifest_size(id:&str,files:&BTreeMap<String,Vec<u8>>)->usize {
        serde_json::to_vec(&PayloadManifest{version:1,id:id.into(),origin:Origin::Publisher,derived_from:None,reason:None,model_digest:None,certificate_digests:BTreeMap::new(),files:files.iter().map(|(name,bytes)|(name.clone(),FileDigest{sha256:super::super::store::digest(bytes),bytes:bytes.len() as u64})).collect()}).unwrap().len()
    }
    #[test]
    fn a29_files_round_individually_and_directories_spend_blocks() {
        let (_root,config,store,old,_)=super::super::recovery::tests::durable_fixture();let previous=store.payload(&old).unwrap().files;
        let id=uuid::Uuid::new_v4().to_string();let operation=Operation::planned(&uuid::Uuid::new_v4().to_string(),&super::super::store::digest(&[]));
        let mut budget=StageBudget::new(&config,&operation,&previous,true);
        let baseline=needed(&store,&id,&previous,manifest_size(&id,&previous),&budget,4096).unwrap();
        budget.validation.push(Write{path:store.root.join("tiny-one"),bytes:1,inodes:1});
        budget.validation.push(Write{path:store.root.join("tiny-two"),bytes:1,inodes:1});
        budget.validation.push(Write{path:store.root.join("record"),bytes:4097,inodes:2});
        budget.validation_dirs.push(store.root.join("new-directory"));
        let changed=needed(&store,&id,&previous,manifest_size(&id,&previous),&budget,4096).unwrap();
        assert_eq!(changed.bytes-baseline.bytes,2*4096+2*8192+4096);assert_eq!(changed.inodes-baseline.inodes,5);
    }
    #[tokio::test]
    async fn a29_low_space_is_terminal_without_payload_live_pointer_or_a8_changes() {
        for uncertain in [false,true] {
            let (_root,config,mut store,old,_)=super::super::recovery::tests::durable_fixture();
            let previous=store.payload(&old).unwrap().files;super::super::files::restore_payload_files(&config,&previous).await.unwrap();
            if uncertain{super::super::confirmation::begin_uncertain(&config).await.unwrap();}
            let live=super::super::files::capture_payload_files(&config).await.unwrap();let marker=std::fs::read(config.nginx_conf_dir.join(".nwui-apply-uncertain")).ok();
            let pointers=std::fs::read(store.root.join("manifest.json")).unwrap();let protected=store.payload(&old).unwrap().manifest_sha256;
            store.capacity_probe=Some(Arc::new(||Ok(Available{block_size:4096,bytes:0,inodes:Some(0)})));
            let dispatcher=Dispatcher::new(config.clone(),store.clone());dispatcher.ready.store(true,Ordering::Release);
            let new=uuid::Uuid::new_v4().to_string();let staged=new.clone();let req=request(Op::ApplySaved{label:None,expected_saved_digest:None});let operation_id=req.id.clone();
            let writer=store.clone();let result=dispatcher.dispatch(req,move |_,operation,_|async move {
                let budget=StageBudget::new(&config,&operation,&previous,!uncertain);
                match writer.stage_with_budget(&staged,Origin::Publisher,None,BTreeMap::new(),live,&budget){Err(error)if is_full(&error)=>super::result(),Err(error)=>panic!("{error:#}"),Ok(_)=>OpResult::Ok{apply:None,skipped:Vec::new(),uncertain:false,phase:"done".into()}}
            }).await;
            assert!(matches!(result,OpResult::Failed{ref code,ref output}if code=="publisher_storage_full"&&output==OUTPUT));
            assert!(!store.root.join("payloads").join(new).exists());assert_eq!(std::fs::read(store.root.join("manifest.json")).unwrap(),pointers);
            assert_eq!(store.payload(&old).unwrap().manifest_sha256,protected);
            assert_eq!(super::super::files::capture_payload_files(&dispatcher.config).await.unwrap(),store.payload(&old).unwrap().files);
            assert_eq!(std::fs::read(dispatcher.config.nginx_conf_dir.join(".nwui-apply-uncertain")).ok(),marker);
            let closed=Operation::load(&store,&operation_id).unwrap();assert_eq!(closed.phase,Phase::Done);assert_eq!(closed.result.unwrap()["code"],"publisher_storage_full");
        }
    }
    #[tokio::test]
    async fn a29_restore_headroom_and_inodes_are_required_before_any_stage_entry() {
        let (_root,config,mut store,old,_)=super::super::recovery::tests::durable_fixture();let previous=store.payload(&old).unwrap().files;
        let operation=Operation::planned(&uuid::Uuid::new_v4().to_string(),&super::super::store::digest(&[]));
        for inodes in [false,true] {
            let id=uuid::Uuid::new_v4().to_string();let budget=StageBudget::new(&config,&operation,&previous,true);
            let need=needed(&store,&id,&previous,manifest_size(&id,&previous),&budget,4096).unwrap();assert!(need.restore_bytes>0);
            let available=Available{block_size:4096,bytes:if inodes{need.bytes}else{need.bytes-need.restore_bytes},inodes:if inodes{Some(need.inodes-1)}else{None}};
            store.capacity_probe=Some(Arc::new(move||Ok(available)));
            let error=store.stage_with_budget(&id,Origin::Publisher,None,BTreeMap::new(),previous.clone(),&budget).err().expect("capacity refusal");
            assert!(is_full(&error));assert!(!store.root.join("payloads").join(id).exists());
        }
    }
    #[tokio::test]
    async fn a29_exact_capacity_and_unknown_inode_counters_admit_without_extra_limits() {
        let (_root,config,mut store,old,_)=super::super::recovery::tests::durable_fixture();let previous=store.payload(&old).unwrap().files;
        let operation=Operation::planned(&uuid::Uuid::new_v4().to_string(),&super::super::store::digest(&[]));
        for known_inodes in [false,true] {
            let id=uuid::Uuid::new_v4().to_string();let budget=StageBudget::new(&config,&operation,&previous,true);
            let need=needed(&store,&id,&previous,manifest_size(&id,&previous),&budget,4096).unwrap();let available=Available{block_size:4096,bytes:need.bytes,inodes:known_inodes.then_some(need.inodes)};
            store.capacity_probe=Some(Arc::new(move||Ok(available)));store.stage_with_budget(&id,Origin::Publisher,None,BTreeMap::new(),previous.clone(),&budget).unwrap();
        }
    }
    #[tokio::test]
    async fn a29_one_protected_collection_then_recheck_can_admit() {
        let (_root,config,mut store,old,_)=super::super::recovery::tests::durable_fixture();let previous=store.payload(&old).unwrap().files;let expired=uuid::Uuid::new_v4().to_string();
        store.stage(&expired,Origin::Publisher,None,BTreeMap::new(),previous.clone()).unwrap();
        let Admission::New(mut operation)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&super::super::store::digest(&[])).unwrap()else{panic!()};
        operation.phase=Phase::Done;operation.next=Some(expired.clone());operation.created_at=chrono::Utc::now()-chrono::Duration::hours(48);operation.save(&store).unwrap();
        let expired_path=store.root.join("payloads").join(&expired);let calls=Arc::new(AtomicUsize::new(0));let observed=calls.clone();
        store.capacity_probe=Some(Arc::new(move||{observed.fetch_add(1,Ordering::SeqCst);Ok(Available{block_size:4096,bytes:if expired_path.exists(){0}else{u64::MAX},inodes:None})}));
        let current=Operation::planned(&uuid::Uuid::new_v4().to_string(),&super::super::store::digest(&[]));let budget=StageBudget::new(&config,&current,&previous,true);let id=uuid::Uuid::new_v4().to_string();
        store.stage_with_budget(&id,Origin::Publisher,None,BTreeMap::new(),previous.clone(),&budget).unwrap();
        assert!(!store.root.join("payloads").join(expired).exists());assert_eq!(calls.load(Ordering::SeqCst),2);assert!(store.payload(&old).is_ok());
    }
    #[tokio::test]
    async fn a29_read_only_commands_never_probe_capacity() {
        let (_root,config,mut store,_,_)=super::super::recovery::tests::durable_fixture();store.capacity_probe=Some(Arc::new(||panic!("read-only command cannot allocate a payload")));
        let dispatcher=Dispatcher::new(config,store);dispatcher.ready.store(true,Ordering::Release);
        for op in [Op::Hello,Op::ValidateLive,Op::OperationStatus{operation_id:uuid::Uuid::new_v4().to_string()}] {
            let result=dispatcher.dispatch(request(op),|_,_,_|async{panic!("read-only commands never dispatch a writer")}).await;
            assert!(matches!(result,OpResult::Ok{..}|OpResult::Unknown));
        }
    }
    #[tokio::test]
    async fn a29_validation_copies_marker_and_main_records_are_all_budgeted_on_their_filesystem() {
        let (_root,config,store,old,_)=super::super::recovery::tests::durable_fixture();let previous=store.payload(&old).unwrap().files;
        let id=uuid::Uuid::new_v4().to_string();let operation=Operation::planned(&uuid::Uuid::new_v4().to_string(),&super::super::store::digest(&[]));
        let mut budget=StageBudget::new(&config,&operation,&previous,true);let baseline=needed(&store,&id,&previous,manifest_size(&id,&previous),&budget,4096).unwrap();
        let generation=store.root.join("payloads").join(&id);let view=generation.join(".test");
        for(prefix,name,bytes)in [("conf","server-copy.conf",19),("stream","stream-copy.conf",23),("","marker",29)] {
            budget.validation.push(Write{path:view.join(prefix).join(name),bytes,inodes:1});
        }
        budget.validation_dirs=vec![view.clone(),view.join("conf"),view.join("stream")];
        let main=super::super::staging::capacity_writes(&config.nginx_main_conf,&config,&generation,&id).unwrap();assert_eq!(main.len(),2);
        let bytes=budget.validation.iter().chain(&main).map(|write|rounded(write.bytes,4096).unwrap()*write.inodes).sum::<u64>()+3*4096;budget.validation.extend(main);
        let full=needed(&store,&id,&previous,manifest_size(&id,&previous),&budget,4096).unwrap();
        assert_eq!(full.bytes,baseline.bytes+bytes);assert!(full.inodes>=baseline.inodes+3+1+2+3);
    }
    #[tokio::test]
    async fn a29_guard_capacity_refusal_preserves_live_payloads_and_entry_a8_then_allows_retry() {
        let (root,mut config,mut store,old,_)=super::super::recovery::tests::durable_fixture();
        config.nginx_quarantine_dir=root.path().join("quarantine");let previous=store.payload(&old).unwrap().files;
        super::super::files::restore_payload_files(&config,&previous).await.unwrap();
        std::fs::write(config.nginx_conf_dir.join("server-fixture.conf"),b"server { listen 3001; }\n").unwrap();
        let live=super::super::files::capture_payload_files(&config).await.unwrap();let pointers=std::fs::read(store.root.join("manifest.json")).unwrap();
        let count=std::fs::read_dir(store.root.join("payloads")).unwrap().count();store.capacity_probe=Some(Arc::new(||Ok(Available{block_size:4096,bytes:0,inodes:Some(0)})));
        let error=super::super::boot::guard(&config,&store).await.unwrap_err();assert_eq!(error.code,"publisher_storage_full");
        assert_eq!(super::super::files::capture_payload_files(&config).await.unwrap(),live);
        assert_eq!(std::fs::read(store.root.join("manifest.json")).unwrap(),pointers);assert_eq!(std::fs::read_dir(store.root.join("payloads")).unwrap().count(),count);
        assert_eq!(std::fs::read(config.nginx_conf_dir.join(".nwui-apply-uncertain")).unwrap(),b"nginx generation is unconfirmed\n");
        let records=super::super::journal::operations(&store).unwrap();assert_eq!(records.len(),1);assert_eq!(records[0].phase,Phase::Done);assert_eq!(records[0].result.as_ref().unwrap()["code"],"publisher_storage_full");
        store.capacity_probe=Some(Arc::new(||Ok(Available{block_size:4096,bytes:u64::MAX,inodes:None})));
        let report=super::super::boot::guard(&config,&store).await.unwrap();assert_eq!(report.reserved_files,1);assert!(!config.nginx_conf_dir.join("server-fixture.conf").exists());
        assert!(super::super::confirmation::uncertain(&config).await);
    }
    #[tokio::test]
    async fn a29_guard_with_nothing_to_isolate_never_probes_or_stages_on_a_full_archive() {
        let (root,mut config,mut store,old,_)=super::super::recovery::tests::durable_fixture();config.nginx_quarantine_dir=root.path().join("quarantine");
        super::super::files::restore_payload_files(&config,&store.payload(&old).unwrap().files).await.unwrap();
        let count=std::fs::read_dir(store.root.join("payloads")).unwrap().count();store.capacity_probe=Some(Arc::new(||panic!("unchanged guard must not stage")));
        let report=super::super::boot::guard(&config,&store).await.unwrap();assert_eq!(report.reserved_files+report.internal_target_files+report.invalid_files,0);
        assert_eq!(std::fs::read_dir(store.root.join("payloads")).unwrap().count(),count);
    }

    #[cfg(target_os="linux")]
    #[tokio::test]
    async fn a29_live_and_restore_targets_on_another_filesystem_do_not_spend_archive_space() {
        let (_root,mut config,store,old,_)=super::super::recovery::tests::durable_fixture();let previous=store.payload(&old).unwrap().files;
        let id=uuid::Uuid::new_v4().to_string();let operation=Operation::planned(&uuid::Uuid::new_v4().to_string(),&super::super::store::digest(&[]));
        super::super::files::restore_payload_files(&config,&previous).await.unwrap();
        let local=needed(&store,&id,&previous,manifest_size(&id,&previous),&StageBudget::new(&config,&operation,&previous,true),4096).unwrap();
        let remote=tempfile::tempdir_in("/dev/shm").unwrap();assert_ne!(std::fs::metadata(remote.path()).unwrap().dev(),std::fs::metadata(&store.root).unwrap().dev());config.certs_dir=remote.path().to_owned();
        let other=needed(&store,&id,&previous,manifest_size(&id,&previous),&StageBudget::new(&config,&operation,&previous,true),4096).unwrap();
        let cert_bytes=previous.iter().filter(|(name,_)|name.starts_with("certs/")).map(|(_,bytes)|rounded(bytes.len() as u64,4096).unwrap()).sum::<u64>();
        assert_eq!(other.bytes,local.bytes-2*cert_bytes);
    }

}
