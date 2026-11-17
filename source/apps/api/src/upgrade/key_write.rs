//! A.13 root-only, intent-owned key writes; no hard links or load_or_create.
use super::{Branch,Journal,KeySource,Step,StepError,StepState,StorageLocks};
use crate::db_upgrade::{files,key_file::{self,KeyPolicy},layout::Roots,paths};
use serde::{Deserialize,Serialize};
use sha2::{Digest,Sha256};
use std::{ffi::CString,fs::{self,File},io::{self,Read,Write},os::{fd::{AsRawFd,FromRawFd},unix::fs::{MetadataExt,PermissionsExt}},path::{Path,PathBuf}};

#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="kebab-case",tag="kind",deny_unknown_fields)]
pub(crate) enum Purpose {Fresh,Staged {generation:String},Publication {generation:String}}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub(crate) enum Mode {Prepare,Publish}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub(crate) enum Progress {Prepared,Done}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Intent {id:String,purpose:Purpose,target:PathBuf,temp_name:String,sha256:String}
#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
enum Action {Intent,Prepared,Done,RetireIntent,Retired}
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {action:Action,intent:Intent}
fn mismatch()->StepError {StepError {code:"pair_mismatch",message:"키 파일의 복구 기록과 실제 파일이 다릅니다. 기존 파일을 보존했습니다.",detail:None}}
fn missing()->StepError {StepError {code:"key_missing",message:"기록된 암호화 키가 없습니다. 새 키를 만들지 않았습니다.",detail:None}}
fn key_error()->StepError {StepError {code:"key_handle_failed",message:"암호화 키 경로 또는 파일을 확인하지 못했습니다. 기존 파일을 보존했습니다.",detail:None}}
fn io_error(error:io::Error)->StepError {
    let code=match error.raw_os_error() {Some(libc::EROFS)=>"storage_read_only",Some(libc::ENOSPC|libc::EDQUOT)=>"insufficient_space",Some(libc::EINTR)=>"interrupted",_=>"io_error"};
    StepError {code,message:"키 파일 작업을 완료하지 못했습니다. 복구 기록과 기존 파일을 보존했습니다.",detail:None}
}
fn digest(bytes:&[u8])->String {hex::encode(Sha256::digest(bytes))}
fn tag(p:&Purpose)->&'static str {match p {Purpose::Fresh=>"fresh",Purpose::Staged{..}=>"stage",Purpose::Publication{..}=>"publish"}}
fn temp_name(p:&Purpose,id:&str)->String {format!(".nwui-key-{}-{id}.init",tag(p))}
fn events(j:&Journal)->Result<Vec<Event>,StepError> {
    j.load().records.iter().filter_map(|r|r.data.as_ref().and_then(|d|d.get("keyWrite"))).map(|v| {
        let e:Event=serde_json::from_value(v.clone()).map_err(|_|mismatch())?;
        if uuid::Uuid::parse_str(&e.intent.id).is_err() || e.intent.temp_name!=temp_name(&e.intent.purpose,&e.intent.id)
            || e.intent.sha256.len()!=64 || !e.intent.sha256.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {return Err(mismatch());}
        if !e.intent.target.is_absolute() || e.intent.target.file_name().is_none() || e.intent.target.as_os_str().as_encoded_bytes().contains(&0) {return Err(mismatch());}
        if let Purpose::Staged{generation}|Purpose::Publication{generation}=&e.intent.purpose {if !super::types::canonical_generation(generation){return Err(mismatch());}}
        Ok(e)
    }).collect()
}
fn record(j:&mut Journal,action:Action,intent:&Intent)->Result<(),StepError> {
    let label=match (&intent.purpose,action) {
        (_,Action::RetireIntent)=>"key-temp-retire-intent",(_,Action::Retired)=>"key-temp-retired",
        (Purpose::Fresh,Action::Intent)=>"key-create-intent",(Purpose::Fresh,Action::Done)=>"key-created",
        (Purpose::Staged{..},Action::Intent)=>"key-staged",(Purpose::Staged{..},Action::Done)=>"key-staged-done",
        (Purpose::Publication{..},Action::Intent)=>"key-publish-intent",(Purpose::Publication{..},Action::Prepared)=>"key-publish-prepared",
        (Purpose::Publication{..},Action::Done)=>"key-published",_=>return Err(mismatch()),
    };
    let step=match intent.purpose {Purpose::Fresh=>if j.load().branch==Branch::Fresh {Step::F1}else{Step::Y3},_=>Step::P4Target};
    let state=if matches!(action,Action::Intent|Action::RetireIntent) {StepState::Intent}else{StepState::Done};
    j.record(step,state,Some(serde_json::json!({"keyEvent":label,"keyWrite":Event {action,intent:intent.clone()}})))
}
fn target(roots:&Roots,p:&Purpose)->Result<PathBuf,StepError> {
    let canonical=paths::startup_data_key_file(&roots.app).map_err(|e|StepError {code:e.code,message:e.message,detail:None})?;
    let path=match p {Purpose::Fresh|Purpose::Publication{..}=>canonical,Purpose::Staged{generation}=>{
        if !super::types::canonical_generation(generation){return Err(mismatch());}
        KeySource::Staged {generation:generation.clone()}.path(&roots.app,None)
    }};
    if path.file_name().is_none() || path.as_os_str().as_encoded_bytes().contains(&0) {return Err(key_error());}Ok(path)
}
fn policy(roots:&Roots,p:&Purpose,path:&Path)->KeyPolicy {
    let source=match p {Purpose::Staged{generation}=>KeySource::Staged {generation:generation.clone()},_=>KeySource::Canonical};
    key_file::policy_for(&source,&roots.app,path)
}
fn name(path:&Path)->Result<CString,StepError> {CString::new(path.file_name().ok_or_else(key_error)?.as_encoded_bytes()).map_err(|_|key_error())}
fn stat(parent:&File,leaf:&CString)->Result<Option<libc::stat>,StepError> {
    let mut m=std::mem::MaybeUninit::uninit();
    if unsafe{libc::fstatat(parent.as_raw_fd(),leaf.as_ptr(),m.as_mut_ptr(),libc::AT_SYMLINK_NOFOLLOW)}==0 {return Ok(Some(unsafe{m.assume_init()}));}
    let e=io::Error::last_os_error();if e.kind()==io::ErrorKind::NotFound {Ok(None)}else{Err(io_error(e))}
}
fn regular(m:&libc::stat)->bool {m.st_mode&libc::S_IFMT==libc::S_IFREG && m.st_uid==0}
fn read_temp(parent:&File,leaf:&CString,m:&libc::stat,sha:&str,links:libc::nlink_t)->Result<Option<Vec<u8>>,StepError> {
    if !regular(m) || m.st_nlink!=links {return Err(mismatch());}
    if m.st_mode&0o7777!=0o600 || m.st_size<1 || m.st_size>4096 {return Ok(None);}
    let fd=unsafe{libc::openat(parent.as_raw_fd(),leaf.as_ptr(),libc::O_RDONLY|libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC)};
    if fd<0{return Err(io_error(io::Error::last_os_error()));}
    let mut f=unsafe{File::from_raw_fd(fd)};let meta=f.metadata().map_err(io_error)?;
    if meta.dev()!=m.st_dev || meta.ino()!=m.st_ino || !meta.is_file() || meta.uid()!=0 || meta.mode()&0o7777!=0o600 || meta.nlink()!=links as u64 {return Err(mismatch());}
    let mut bytes=Vec::new();(&mut f).take(4097).read_to_end(&mut bytes).map_err(io_error)?;
    if bytes.len()>4096 || digest(&bytes)!=sha {return Ok(None);}
    if crate::crypto::parse_key_file(Path::new("owned-key-temp"),std::str::from_utf8(&bytes).map_err(|_|key_error())?).is_err(){return Ok(None);}
    f.sync_all().map_err(io_error)?;Ok(Some(bytes))
}
fn validate_target(roots:&Roots,p:&Purpose,intent:&Intent,m:&libc::stat)->Result<(),StepError> {
    if !regular(m) || m.st_mode&0o7777!=0o600 || m.st_nlink!=1 {return Err(mismatch());}
    let k=key_file::read_key_file(&intent.target,policy(roots,p,&intent.target)).map_err(|_|key_error())?;
    if digest(&k.bytes)!=intent.sha256 {return Err(mismatch());}Ok(())
}
fn unlink(parent:&File,leaf:&CString)->Result<(),StepError> {
    if unsafe{libc::unlinkat(parent.as_raw_fd(),leaf.as_ptr(),0)}!=0 {return Err(io_error(io::Error::last_os_error()));}
    parent.sync_all().map_err(io_error)
}
fn retire(j:&mut Journal,parent:&File,intent:&Intent,temp:&CString)->Result<(),StepError> {
    record(j,Action::RetireIntent,intent)?;checkpoint("retire-intent");
    if let Some(m)=stat(parent,temp)? {if !regular(&m) || m.st_nlink!=1 {return Err(mismatch());}unlink(parent,temp)?;}
    checkpoint("retire-unlink");record(j,Action::Retired,intent)?;Ok(())
}
fn staged_bytes(roots:&Roots,j:&Journal,generation:&str)->Result<Vec<u8>,StepError> {
    let p=Purpose::Staged {generation:generation.into()};
    let e=events(j)?.into_iter().rev().find(|e|e.intent.purpose==p).ok_or_else(missing)?;
    if e.action!=Action::Done || e.intent.target!=target(roots,&p)? {return Err(missing());}
    let k=key_file::read_key_file(&e.intent.target,policy(roots,&p,&e.intent.target)).map_err(|_|key_error())?;
    if digest(&k.bytes)!=e.intent.sha256{return Err(mismatch());}Ok(k.bytes)
}
#[cfg(test)] fn checkpoint(point:&str) {if std::env::var("NWUI_KEY_WRITE_KILL_POINT").as_deref()==Ok(point) {unsafe{libc::kill(libc::getpid(),libc::SIGKILL);}}}
#[cfg(not(test))] fn checkpoint(_point:&str) {}

pub(crate) fn write_locked(locks:&StorageLocks,roots:&Roots,j:&mut Journal,p:Purpose,mode:Mode)->Result<Progress,StepError> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app){return Err(mismatch());}
    match (&p,j.load().branch,mode) {
        (Purpose::Fresh,Branch::Fresh|Branch::Ready,Mode::Publish)=>{},
        (Purpose::Staged{..},Branch::Upgrade,Mode::Publish)=>{},
        (Purpose::Publication{generation},Branch::Upgrade,_) if super::types::canonical_generation(generation)=>{},
        _=>return Err(mismatch()),
    }
    let committed=j.load().records.iter().any(|r|r.step==Step::P5Commit && r.state==StepState::Done && r.data.is_none());
    if matches!(p,Purpose::Publication{..}) && (mode==Mode::Publish && !committed || mode==Mode::Prepare && committed) {return Err(mismatch());}
    let path=target(roots,&p)?;
    let publication_done=events(j)?.into_iter().rev().find(|e|e.intent.purpose==p).is_some_and(|e|e.action==Action::Done);
    let published_bytes=match &p {Purpose::Publication{generation} if !publication_done=>Some(staged_bytes(roots,j,generation)?),_=>None};
    for _ in 0..3 {
        let previous=events(j)?.into_iter().rev().find(|e|e.intent.purpose==p);
        let active=previous.filter(|e|e.action!=Action::Retired);
        if matches!(p,Purpose::Publication{..}) && mode==Mode::Publish && active.as_ref().is_none_or(|e|!matches!(e.action,Action::Prepared|Action::Done)) {return Err(mismatch());}
        let (intent,new_bytes,new)=if let Some(e)=&active {
            if e.intent.target!=path {return Err(mismatch());}
            if let Some(bytes)=&published_bytes {if digest(bytes)!=e.intent.sha256{return Err(mismatch());}}
            (e.intent.clone(),None,false)
        } else {
            if fs::symlink_metadata(&path).is_ok(){return Err(mismatch());}
            let bytes=if let Some(bytes)=&published_bytes {bytes.clone()}else{
                let mut raw=[0u8;32];getrandom::fill(&mut raw).map_err(|_|key_error())?;hex::encode(raw).into_bytes()
            };
            let id=uuid::Uuid::new_v4().to_string();let temp_name=temp_name(&p,&id);
            if fs::symlink_metadata(path.parent().ok_or_else(key_error)?.join(&temp_name)).is_ok(){return Err(mismatch());}
            let intent=Intent {id,purpose:p.clone(),target:path.clone(),temp_name,sha256:digest(&bytes)};
            record(j,Action::Intent,&intent)?;checkpoint("intent");(intent,Some(bytes),true)
        };
        if let Purpose::Staged{generation}=&p {
            files::private_dir(&roots.app.join(".nwui-auto/gen")).map_err(io_error)?;
            files::private_dir(&roots.app.join(".nwui-auto/gen").join(generation)).map_err(io_error)?;
        }
        let parent=key_file::validate_parent(&path,policy(roots,&p,&path)).map_err(|_|key_error())?;
        let leaf=name(&path)?;let temp=CString::new(intent.temp_name.as_str()).map_err(|_|mismatch())?;
        let target_meta=stat(&parent,&leaf)?;let temp_meta=stat(&parent,&temp)?;
        if let Some(target_meta)=target_meta {
            if matches!(p,Purpose::Publication{..}) && !committed {return Err(mismatch());}
            if let Some(temp_meta)=temp_meta {
                if !regular(&target_meta) || target_meta.st_mode&0o7777!=0o600 || target_meta.st_dev!=temp_meta.st_dev || target_meta.st_ino!=temp_meta.st_ino {return Err(mismatch());}
                if target_meta.st_nlink!=2 || read_temp(&parent,&temp,&temp_meta,&intent.sha256,2)?.is_none(){return Err(mismatch());}
                unlink(&parent,&temp)?;
            }
            let m=stat(&parent,&leaf)?.ok_or_else(missing)?;validate_target(roots,&p,&intent,&m)?;
            if active.as_ref().is_none_or(|e|e.action!=Action::Done) {record(j,Action::Done,&intent)?;checkpoint("done");}
            return Ok(Progress::Done);
        }
        if active.as_ref().is_some_and(|e|e.action==Action::Done){return Err(missing());}
        if active.as_ref().is_some_and(|e|e.action==Action::RetireIntent) {retire(j,&parent,&intent,&temp)?;continue;}
        let bytes=if let Some(m)=temp_meta {
            match read_temp(&parent,&temp,&m,&intent.sha256,1)? {Some(bytes)=>bytes,None=>{
                if matches!(p,Purpose::Publication{..}) && mode==Mode::Publish {return Err(key_error());}
                retire(j,&parent,&intent,&temp)?;continue;
            }}
        } else if new {new_bytes.ok_or_else(missing)?} else {
            if matches!(p,Purpose::Publication{..}) && mode==Mode::Publish {return Err(missing());}
            retire(j,&parent,&intent,&temp)?;continue;
        };
        if temp_meta.is_none() {
            let fd=unsafe{libc::openat(parent.as_raw_fd(),temp.as_ptr(),libc::O_WRONLY|libc::O_CREAT|libc::O_EXCL|libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC,0o600 as libc::mode_t)};
            if fd<0 {return Err(io_error(io::Error::last_os_error()));}
            let mut f=unsafe{File::from_raw_fd(fd)};checkpoint("create");
            f.set_permissions(fs::Permissions::from_mode(0o600)).map_err(io_error)?;
            f.write_all(&bytes).map_err(io_error)?;checkpoint("write");f.sync_all().map_err(io_error)?;checkpoint("file-fsync");
        }
        if mode==Mode::Prepare {
            parent.sync_all().map_err(io_error)?;
            if active.as_ref().is_none_or(|e|e.action!=Action::Prepared) {record(j,Action::Prepared,&intent)?;}checkpoint("prepared");return Ok(Progress::Prepared);
        }
        if stat(&parent,&leaf)?.is_some(){return Err(mismatch());}
        if unsafe{libc::renameat(parent.as_raw_fd(),temp.as_ptr(),parent.as_raw_fd(),leaf.as_ptr())}!=0 {return Err(io_error(io::Error::last_os_error()));}
        checkpoint("rename");parent.sync_all().map_err(io_error)?;checkpoint("parent-fsync");
        let m=stat(&parent,&leaf)?.ok_or_else(missing)?;validate_target(roots,&p,&intent,&m)?;
        record(j,Action::Done,&intent)?;checkpoint("done");return Ok(Progress::Done);
    }
    Err(key_error())
}

pub(super) fn source(j:&Journal)->Result<KeySource,StepError> {
    let mut source=KeySource::Canonical;let mut pending=false;
    for e in events(j)? {
        match (&e.intent.purpose,e.action) {
            (Purpose::Staged{generation},Action::Intent)=>{
                if !super::types::canonical_generation(generation){return Err(mismatch());}
                pending=true;source=KeySource::Staged {generation:generation.clone()};
            },
            (Purpose::Staged{generation},Action::Done)=>{
                if !matches!(&source,KeySource::Staged{generation:g}if g==generation){return Err(mismatch());}pending=false;
            },
            (Purpose::Publication{generation},Action::Done)=>{
                if pending || !matches!(&source,KeySource::Staged{generation:g}if g==generation){return Err(mismatch());}
                source=KeySource::Canonical;
            },
            _=>{},
        }
    }
    if pending {Err(missing())}else{Ok(source)}
}

// No Debug/Serialize: this owns the validated key bytes and SecretBox.
pub(crate) struct EnsuredKey {pub journal:Journal,pub source:KeySource,pub key:key_file::ValidatedKeyFile}
/// Upgrade callers pass required=true if inspection saw a key; false is allowed only after
/// inspection accepted key=None (no sealed values). Fresh/ready pass the pair's required flag.
/// The coordinator must await K before starting restore; cancellation only sets its stop flag.
pub(crate) async fn ensure_key_locked(roots:Roots,locks:std::sync::Arc<StorageLocks>,mut journal:Journal,required:bool)->Result<EnsuredKey,StepError> {
    #[cfg(test)] let origin=std::thread::current().id();
    tokio::task::spawn_blocking(move|| {
        #[cfg(test)] assert_ne!(origin,std::thread::current().id(),"K blocked the async caller thread");
        let (source,key)=ensure_sync(&roots,&locks,&mut journal,required)?;
        Ok(EnsuredKey {journal,source,key})
    }).await.map_err(|_|key_error())?
}
fn ensure_sync(roots:&Roots,locks:&StorageLocks,j:&mut Journal,required:bool)->Result<(KeySource,key_file::ValidatedKeyFile),StepError> {
    // The raw getter's empty/relative check happens before any key/journal mutation.
    let canonical=target(roots,&Purpose::Fresh)?;
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app){return Err(mismatch());}
    let history=events(j)?;
    let stage=history.iter().rev().find_map(|e|match &e.intent.purpose {Purpose::Staged{generation}=>Some((generation.clone(),e.action)),_=>None});
    let committed=j.load().records.iter().any(|r|r.step==Step::P5Commit && r.state==StepState::Done && r.data.is_none());
    let source=if j.load().branch==Branch::Upgrade {
        if let Some((generation,action))=stage {
            let publication=Purpose::Publication {generation:generation.clone()};
            let published=history.iter().rev().find(|e|e.intent.purpose==publication).is_some_and(|e|e.action==Action::Done);
            if published && !committed {return Err(mismatch());}
            if !published {
                if (committed || required) && action!=Action::Done {return Err(missing());}
                write_locked(locks,roots,j,Purpose::Staged {generation:generation.clone()},Mode::Publish)?;
                write_locked(locks,roots,j,publication,if committed {Mode::Publish}else{Mode::Prepare})?;
            }
            if committed {KeySource::Canonical}else{KeySource::Staged {generation}}
        } else {
            match key_file::read_key_file(&canonical,key_file::policy_for(&KeySource::Canonical,&roots.app,&canonical)) {
                Ok(key)=>return Ok((KeySource::Canonical,key)),
                Err(key_file::KeyFileError::Missing) if !required && !committed=>{},
                Err(key_file::KeyFileError::Missing)=>return Err(missing()),Err(_)=>return Err(key_error()),
            }
            let generation=uuid::Uuid::new_v4().to_string();
            write_locked(locks,roots,j,Purpose::Staged {generation:generation.clone()},Mode::Publish)?;
            write_locked(locks,roots,j,Purpose::Publication {generation:generation.clone()},Mode::Prepare)?;
            KeySource::Staged {generation}
        }
    } else {
        let own_intent=history.iter().any(|e|e.intent.purpose==Purpose::Fresh);
        match key_file::read_key_file(&canonical,key_file::policy_for(&KeySource::Canonical,&roots.app,&canonical)) {
            Ok(key) if !own_intent=>return Ok((KeySource::Canonical,key)),
            Ok(_)=>{},Err(key_file::KeyFileError::Missing) if !required=>{},
            Err(key_file::KeyFileError::Missing)=>return Err(missing()),Err(_)=>return Err(key_error()),
        }
        write_locked(locks,roots,j,Purpose::Fresh,Mode::Publish)?;KeySource::Canonical
    };
    let path=match &source {KeySource::Canonical=>canonical.clone(),KeySource::Staged{..}=>source.path(&roots.app,None)};
    let key=key_file::read_key_file(&path,key_file::policy_for(&source,&roots.app,&canonical)).map_err(|e|if e==key_file::KeyFileError::Missing {missing()}else{key_error()})?;
    let expected=events(j)?.into_iter().rev().find(|e|e.action==Action::Done && e.intent.target==path).ok_or_else(mismatch)?;
    if digest(&key.bytes)!=expected.intent.sha256 {return Err(mismatch());}
    Ok((source,key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{crypto::SecretBox,db_upgrade::{paths,files}};
    use super::super::{Branch,Step,StepState,journal::Candidate};
    use std::{fs,path::{Path,PathBuf},os::unix::{fs::{MetadataExt,PermissionsExt},process::ExitStatusExt}};
    #[test]
    fn w1_key_write_rejects_generation_aliases_before_any_mutation() {
        let (_t,r)=fixture(Branch::Upgrade);let (locks,mut j)=open(&r);
        let id=uuid::Uuid::parse_str("3d521ab4-34ad-44e8-b7eb-882c0feff693").unwrap();
        for generation in [id.simple().to_string(),id.braced().to_string(),id.urn().to_string(),id.hyphenated().to_string().to_uppercase()] {
            assert!(uuid::Uuid::parse_str(&generation).is_ok());
            let before=j.load().records.len();
            let error=write_locked(&locks,&r,&mut j,Purpose::Staged {generation:generation.clone()},Mode::Publish).unwrap_err();
            assert_eq!(error.code,"pair_mismatch");assert_eq!(j.load().records.len(),before);
            assert!(!r.app.join(".nwui-auto/gen").join(generation).exists());
        }
    }
    #[test]
    fn w1_key_write_journal_rejects_generation_aliases_without_path_panics() {
        let (_t,r)=fixture(Branch::Upgrade);let (_locks,mut j)=open(&r);
        let purpose=Purpose::Staged {generation:"3d521ab434ad44e8b7eb882c0feff693".into()};
        let id=uuid::Uuid::new_v4().to_string();
        let intent=Intent {temp_name:temp_name(&purpose,&id),id,purpose,target:r.app.join("not-opened"),sha256:"0".repeat(64)};
        record(&mut j,Action::Intent,&intent).unwrap();
        assert_eq!(j.key_source().unwrap_err().code,"pair_mismatch");
    }
    #[tokio::test(flavor="current_thread")]
    async fn w1_key_ensure_fresh_and_ready_offload_and_preserve_the_created_key() {
        for branch in [Branch::Fresh,Branch::Ready] {
            let (_t,r)=fixture(branch);let (locks,j)=open(&r);let locks=std::sync::Arc::new(locks);
            let result=ensure_key_locked(r.clone(),locks.clone(),j,false).await.unwrap();
            assert_eq!(result.source,KeySource::Canonical);let before=valid(&canonical(&r));
            assert_eq!(result.key.bytes,before);
            let result=ensure_key_locked(r.clone(),locks,result.journal,true).await.unwrap();
            assert_eq!(result.key.bytes,before);assert_eq!(valid(&canonical(&r)),before);
        }
    }
    #[tokio::test(flavor="current_thread")]
    async fn w1_key_ensure_keyed_upgrade_does_not_stage_or_rewrite() {
        let (_t,r)=fixture(Branch::Upgrade);
        SecretBox::load_or_create(&canonical(&r)).unwrap();let before=valid(&canonical(&r));
        let (locks,j)=open(&r);let count=j.load().records.len();
        let result=ensure_key_locked(r.clone(),std::sync::Arc::new(locks),j,true).await.unwrap();
        assert_eq!(result.source,KeySource::Canonical);assert_eq!(result.key.bytes,before);
        assert_eq!(result.journal.load().records.len(),count);assert!(!r.app.join(".nwui-auto/gen").exists());
    }
    #[tokio::test(flavor="current_thread")]
    async fn w1_key_ensure_required_missing_key_never_regenerates_on_any_branch() {
        for branch in [Branch::Fresh,Branch::Ready,Branch::Upgrade] {
            let (_t,r)=fixture(branch);let (locks,j)=open(&r);let locks=std::sync::Arc::new(locks);
            let count=j.load().records.len();
            let error=ensure_key_locked(r.clone(),locks.clone(),j,true).await.err().unwrap();
            assert_eq!(error.code,"key_missing");assert!(!canonical(&r).exists());
            assert_eq!(Journal::open(&r.app,&locks).unwrap().load().records.len(),count);
        }
    }
    #[tokio::test(flavor="current_thread")]
    async fn w1_key_ensure_keyless_upgrade_prepares_then_rolls_forward_the_same_bytes() {
        let (_t,r)=fixture(Branch::Upgrade);let (locks,j)=open(&r);let locks=std::sync::Arc::new(locks);
        let mut result=ensure_key_locked(r.clone(),locks.clone(),j,false).await.unwrap();
        assert!(matches!(result.source,KeySource::Staged{..}));assert!(!canonical(&r).exists());
        let before=result.key.bytes;assert_eq!(result.journal.key_source().unwrap(),result.source);
        assert!(events(&result.journal).unwrap().iter().any(|e|matches!(e.intent.purpose,Purpose::Publication{..}) && e.action==Action::Prepared));
        result.journal.record(Step::P5Commit,StepState::Done,None).unwrap();
        let result=ensure_key_locked(r.clone(),locks.clone(),result.journal,false).await.unwrap();
        assert_eq!(result.source,KeySource::Canonical);assert_eq!(result.key.bytes,before);assert_eq!(valid(&canonical(&r)),before);
        let result=ensure_key_locked(r.clone(),locks,result.journal,true).await.unwrap();
        assert_eq!(result.key.bytes,before);
    }
    const CHILD:&str="NWUI_KEY_WRITE_FIXTURE";
    fn roots(base:&Path)->Roots {Roots {db:base.join("db"),app:base.join("app"),architecture:paths::architecture().unwrap().into()}}
    fn fixture(branch:Branch)->(tempfile::TempDir,Roots) {
        let t=tempfile::tempdir().unwrap();let r=roots(t.path());
        for p in [&r.db,&r.app] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        files::private_dir(&r.app.join("secrets")).unwrap();
        let locks=StorageLocks::acquire(&r.db,&r.app).unwrap();
        {let fixture_journal=Journal::begin(&r.app,&locks,branch,Candidate {revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};
        drop(locks);(t,r)
    }
    fn lock_fixture(r:&Roots)->StorageLocks {
        // Parallel validator forks briefly inherit another test's lock before _exit/exec.
        let deadline=std::time::Instant::now()+std::time::Duration::from_secs(3);
        loop {match StorageLocks::acquire(&r.db,&r.app) {
            Ok(locks)=>return locks,
            Err(error) if error.code=="storage_busy" && std::time::Instant::now()<deadline=>std::thread::sleep(std::time::Duration::from_millis(5)),
            Err(error)=>panic!("fixture lock did not become available: {error}"),
        }}
    }
    fn open(r:&Roots)->(StorageLocks,Journal) {
        let locks=lock_fixture(r);let j=Journal::open(&r.app,&locks).unwrap();(locks,j)
    }
    #[test]
    fn w1_key_write_fixture_accounts_for_a_live_fork_inheriting_its_lock() {
        use std::io::Write;
        let (_t,r)=fixture(Branch::Upgrade);let (locks,_j)=open(&r);
        let (read,mut release)=std::os::unix::net::UnixStream::pair().unwrap();
        let pid=unsafe{libc::fork()};assert!(pid>=0);
        if pid==0 {unsafe {
            libc::close(release.as_raw_fd());let mut byte=0u8;
            let ok=libc::read(read.as_raw_fd(),(&mut byte as *mut u8).cast(),1)==1;
            libc::_exit(if ok {0}else{2});
        }}
        drop(read);drop(locks);
        assert_eq!(StorageLocks::acquire(&r.db,&r.app).err().unwrap().code,"storage_busy");
        let releaser=std::thread::spawn(move|| {std::thread::sleep(std::time::Duration::from_millis(20));release.write_all(b"x").unwrap();});
        let locks=lock_fixture(&r);releaser.join().unwrap();
        let mut status=0;assert_eq!(unsafe{libc::waitpid(pid,&mut status,0)},pid);assert!(libc::WIFEXITED(status));assert_eq!(libc::WEXITSTATUS(status),0);
        assert!(locks.holds_a() && locks.holds_d());
    }
    fn canonical(r:&Roots)->PathBuf {r.app.join("secrets/data_key")}
    fn valid(path:&Path)->Vec<u8> {
        let m=fs::symlink_metadata(path).unwrap();assert!(m.is_file());assert_eq!(m.uid(),0);assert_eq!(m.mode()&0o7777,0o600);assert_eq!(m.nlink(),1);
        SecretBox::load_existing(path).unwrap();fs::read(path).unwrap()
    }
    #[test]
    fn w1_key_write_fresh_is_journaled_and_never_overwrites_a_canonical() {
        let (_t,r)=fixture(Branch::Fresh);let (locks,mut j)=open(&r);
        assert_eq!(write_locked(&locks,&r,&mut j,Purpose::Fresh,Mode::Publish).unwrap(),Progress::Done);
        let before=valid(&canonical(&r));assert!(j.load().records.iter().any(|record|record.data.as_ref().is_some_and(|data|data.get("keyWrite").is_some())));
        assert_eq!(write_locked(&locks,&r,&mut j,Purpose::Fresh,Mode::Publish).unwrap(),Progress::Done);
        assert_eq!(fs::read(canonical(&r)).unwrap(),before);
    }
    #[test]
    fn w1_key_write_publication_is_prepared_before_commit_and_reuses_staged_bytes() {
        let (_t,r)=fixture(Branch::Upgrade);let (locks,mut j)=open(&r);let generation=uuid::Uuid::new_v4().to_string();
        let staged=Purpose::Staged {generation:generation.clone()};let publication=Purpose::Publication {generation:generation.clone()};
        write_locked(&locks,&r,&mut j,staged,Mode::Publish).unwrap();let bytes=valid(&r.app.join(".nwui-auto/gen").join(&generation).join("data_key"));
        assert_eq!(write_locked(&locks,&r,&mut j,publication.clone(),Mode::Prepare).unwrap(),Progress::Prepared);assert!(!canonical(&r).exists());
        assert_eq!(write_locked(&locks,&r,&mut j,publication.clone(),Mode::Publish).unwrap_err().code,"pair_mismatch");assert!(!canonical(&r).exists());
        j.record(Step::P5Commit,StepState::Done,None).unwrap();
        assert_eq!(write_locked(&locks,&r,&mut j,publication,Mode::Publish).unwrap(),Progress::Done);assert_eq!(valid(&canonical(&r)),bytes);
    }
    #[test]
    fn w1_key_write_different_canonical_and_unowned_temp_are_preserved() {
        let (_t,r)=fixture(Branch::Fresh);let (locks,mut j)=open(&r);
        fs::write(canonical(&r),"ab".repeat(32)).unwrap();fs::set_permissions(canonical(&r),fs::Permissions::from_mode(0o600)).unwrap();
        let foreign=r.app.join("secrets/.nwui-key-foreign.init");fs::write(&foreign,"unowned").unwrap();let before=files::snapshot(&r.app.join("secrets")).unwrap();
        assert_eq!(write_locked(&locks,&r,&mut j,Purpose::Fresh,Mode::Publish).unwrap_err().code,"pair_mismatch");
        assert_eq!(files::snapshot(&r.app.join("secrets")).unwrap(),before);
    }
    #[test]
    fn w1_key_write_child_checkpoint() {
        let Some(base)=std::env::var_os(CHILD) else {return};let r=roots(Path::new(&base));let (locks,mut j)=open(&r);
        let purpose=std::env::var("NWUI_KEY_WRITE_PURPOSE").ok().map(|v|serde_json::from_str(&v).unwrap()).unwrap_or(Purpose::Fresh);
        let mode=if std::env::var("NWUI_KEY_WRITE_MODE").as_deref()==Ok("prepare") {Mode::Prepare}else{Mode::Publish};
        write_locked(&locks,&r,&mut j,purpose,mode).unwrap();panic!("child did not stop at the requested checkpoint");
    }
    fn kill_and_resume(point:&str) {
        let (t,r)=fixture(Branch::Fresh);
        let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::key_write::tests::w1_key_write_child_checkpoint","--nocapture"])
            .env(CHILD,t.path()).env("NWUI_KEY_WRITE_KILL_POINT",point).output().unwrap();
        assert_eq!(output.status.signal(),Some(libc::SIGKILL),"{point}: {}",String::from_utf8_lossy(&output.stderr));
        let existing=fs::read(canonical(&r)).ok();let (locks,mut j)=open(&r);
        assert_eq!(write_locked(&locks,&r,&mut j,Purpose::Fresh,Mode::Publish).unwrap(),Progress::Done);
        let bytes=valid(&canonical(&r));if let Some(existing)=existing {assert_eq!(bytes,existing);}
        assert_eq!(fs::read_dir(r.app.join("secrets")).unwrap().count(),1);
    }
    #[test] fn w1_key_write_kill_after_intent(){kill_and_resume("intent");}
    #[test] fn w1_key_write_kill_after_create(){kill_and_resume("create");}
    #[test] fn w1_key_write_kill_after_write(){kill_and_resume("write");}
    #[test] fn w1_key_write_kill_after_file_fsync(){kill_and_resume("file-fsync");}
    #[test] fn w1_key_write_kill_after_rename(){kill_and_resume("rename");}
    #[test] fn w1_key_write_kill_after_parent_fsync(){kill_and_resume("parent-fsync");}
    #[test] fn w1_key_write_kill_after_done(){kill_and_resume("done");}
    fn child(base:&Path,purpose:&Purpose,mode:Mode,point:&str) {
        let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::key_write::tests::w1_key_write_child_checkpoint","--nocapture"])
            .env(CHILD,base).env("NWUI_KEY_WRITE_KILL_POINT",point).env("NWUI_KEY_WRITE_PURPOSE",serde_json::to_string(purpose).unwrap())
            .env("NWUI_KEY_WRITE_MODE",if mode==Mode::Prepare {"prepare"}else{"publish"}).output().unwrap();
        assert_eq!(output.status.signal(),Some(libc::SIGKILL),"{point}: {}",String::from_utf8_lossy(&output.stderr));
    }
    fn prepared(r:&Roots,j:&mut Journal,locks:&StorageLocks,generation:&str)->(Purpose,PathBuf,Vec<u8>) {
        write_locked(locks,r,j,Purpose::Staged {generation:generation.into()},Mode::Publish).unwrap();
        let bytes=valid(&r.app.join(".nwui-auto/gen").join(generation).join("data_key"));
        let publication=Purpose::Publication {generation:generation.into()};
        write_locked(locks,r,j,publication.clone(),Mode::Prepare).unwrap();
        let e=events(j).unwrap().into_iter().rev().find(|e|e.intent.purpose==publication).unwrap();
        (publication,r.app.join("secrets").join(e.intent.temp_name),bytes)
    }
    #[test]
    fn w1_key_write_partial_staging_gets_a_new_intent_before_any_key_use() {
        let (t,r)=fixture(Branch::Upgrade);let generation=uuid::Uuid::new_v4().to_string();let purpose=Purpose::Staged {generation:generation.clone()};
        child(t.path(),&purpose,Mode::Publish,"create");let (locks,mut j)=open(&r);
        let old=events(&j).unwrap().last().unwrap().intent.clone();assert_eq!(j.key_source().unwrap_err().code,"key_missing");
        write_locked(&locks,&r,&mut j,purpose.clone(),Mode::Publish).unwrap();
        let done=events(&j).unwrap().last().unwrap().intent.clone();assert_ne!(old.id,done.id);assert_ne!(old.sha256,done.sha256);
        assert!(!old.target.parent().unwrap().join(old.temp_name).exists());assert_eq!(j.key_source().unwrap(),KeySource::Staged {generation});valid(&done.target);
    }
    #[test]
    fn w1_key_write_partial_publication_reuses_the_immutable_staged_bytes() {
        let (t,r)=fixture(Branch::Upgrade);let generation=uuid::Uuid::new_v4().to_string();let publication=Purpose::Publication {generation:generation.clone()};
        let (locks,mut j)=open(&r);write_locked(&locks,&r,&mut j,Purpose::Staged {generation:generation.clone()},Mode::Publish).unwrap();
        let stage=r.app.join(".nwui-auto/gen").join(&generation).join("data_key");let bytes=valid(&stage);drop(j);drop(locks);
        child(t.path(),&publication,Mode::Prepare,"create");let (locks,mut j)=open(&r);let old=events(&j).unwrap().last().unwrap().intent.clone();
        write_locked(&locks,&r,&mut j,publication.clone(),Mode::Prepare).unwrap();let new=events(&j).unwrap().last().unwrap().intent.clone();
        assert_ne!(old.temp_name,new.temp_name);assert_eq!(old.sha256,new.sha256);assert_eq!(valid(&stage),bytes);assert!(!canonical(&r).exists());
        j.record(Step::P5Commit,StepState::Done,None).unwrap();write_locked(&locks,&r,&mut j,publication.clone(),Mode::Publish).unwrap();
        assert_eq!(valid(&canonical(&r)),bytes);assert_eq!(j.key_source().unwrap(),KeySource::Canonical);
        // P6 may remove this derived copy; a completed publication must no longer depend on it.
        fs::remove_file(&stage).unwrap();assert_eq!(write_locked(&locks,&r,&mut j,publication,Mode::Publish).unwrap(),Progress::Done);
    }
    #[test]
    fn w1_key_write_invalid_staged_key_after_done_is_preserved_and_refused() {
        let (_t,r)=fixture(Branch::Upgrade);let (locks,mut j)=open(&r);let generation=uuid::Uuid::new_v4().to_string();
        let purpose=Purpose::Staged {generation:generation.clone()};write_locked(&locks,&r,&mut j,purpose.clone(),Mode::Publish).unwrap();
        let path=r.app.join(".nwui-auto/gen").join(&generation).join("data_key");fs::write(&path,"invalid-staged-key").unwrap();
        let before=files::snapshot(&r.app).unwrap();
        assert!(write_locked(&locks,&r,&mut j,purpose,Mode::Publish).is_err());
        assert!(write_locked(&locks,&r,&mut j,Purpose::Publication {generation},Mode::Prepare).is_err());
        assert_eq!(files::snapshot(&r.app).unwrap(),before);assert!(!canonical(&r).exists());
    }
    #[test]
    fn w1_key_write_two_present_names_only_reconcile_the_same_owned_inode() {
        for same_inode in [true,false] {
            let (_t,r)=fixture(Branch::Upgrade);let (locks,mut j)=open(&r);let generation=uuid::Uuid::new_v4().to_string();
            let (publication,temp,bytes)=prepared(&r,&mut j,&locks,&generation);
            if same_inode {fs::hard_link(&temp,canonical(&r)).unwrap();}else{fs::write(canonical(&r),&bytes).unwrap();fs::set_permissions(canonical(&r),fs::Permissions::from_mode(0o600)).unwrap();}
            j.record(Step::P5Commit,StepState::Done,None).unwrap();let before=files::snapshot(&r.app.join("secrets")).unwrap();
            let result=write_locked(&locks,&r,&mut j,publication,Mode::Publish);
            if same_inode {assert_eq!(result.unwrap(),Progress::Done);assert!(!temp.exists());assert_eq!(valid(&canonical(&r)),bytes);}
            else {assert_eq!(result.unwrap_err().code,"pair_mismatch");assert_eq!(files::snapshot(&r.app.join("secrets")).unwrap(),before);}
        }
    }
    #[test]
    fn w1_key_write_different_published_bytes_never_change_either_name() {
        for alias in [true,false] {
            let (_t,r)=fixture(Branch::Upgrade);let (locks,mut j)=open(&r);let generation=uuid::Uuid::new_v4().to_string();
            let (publication,temp,_bytes)=prepared(&r,&mut j,&locks,&generation);
            if alias {fs::hard_link(&temp,canonical(&r)).unwrap();}else{fs::remove_file(&temp).unwrap();}
            fs::write(canonical(&r),"cd".repeat(32)).unwrap();fs::set_permissions(canonical(&r),fs::Permissions::from_mode(0o600)).unwrap();
            j.record(Step::P5Commit,StepState::Done,None).unwrap();let before=files::snapshot(&r.app).unwrap();
            assert_eq!(write_locked(&locks,&r,&mut j,publication,Mode::Publish).unwrap_err().code,"pair_mismatch");
            assert_eq!(files::snapshot(&r.app).unwrap(),before);
        }
    }
    #[test]
    fn w1_key_write_foreign_name_patterns_are_not_ownership() {
        let (_t,r)=fixture(Branch::Fresh);let (locks,mut j)=open(&r);let foreign=r.app.join("secrets").join(format!(".nwui-key-fresh-{}.init",uuid::Uuid::new_v4()));
        fs::write(&foreign,"unowned").unwrap();write_locked(&locks,&r,&mut j,Purpose::Fresh,Mode::Publish).unwrap();
        assert_eq!(fs::read(&foreign).unwrap(),b"unowned");valid(&canonical(&r));
    }

}
