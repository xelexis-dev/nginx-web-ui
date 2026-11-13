//! A.25(i): one attempt's diagnostics, outside the disposable W tree.
use super::{Journal,Step,StepState,StepError,StorageLocks};
use crate::db_upgrade::{layout::Roots,postgres};
use std::{fs,path::{Path,PathBuf},os::unix::fs::MetadataExt};
fn unsafe_logs()->StepError{StepError{code:"unsupported_layout",message:"이전 작업의 진단 파일을 안전하게 확인하지 못했습니다. 기존 파일을 보존했습니다.",detail:Some("diagnostic logs".into())}}
fn io(e:std::io::Error)->StepError{super::backup_format::io_error(e)}
fn exists(path:&Path)->Result<bool,StepError>{match fs::symlink_metadata(path){Ok(_)=>Ok(true),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>Ok(false),Err(e)=>Err(io(e))}}
pub(super) fn path(db:&Path,id:&str,name:&str)->Result<PathBuf,StepError>{
    if !super::types::canonical_generation(id){return Err(unsafe_logs());}
    let suffix=["check-","convert-"].into_iter().find_map(|prefix|name.strip_prefix(prefix)).ok_or_else(unsafe_logs)?;
    if !super::types::canonical_generation(suffix){return Err(unsafe_logs());}
    Ok(db.join(".nwui-auto-work/logs").join(id).join(name))
}
pub(super) fn owned_path(db:&Path,j:&Journal,name:&str,check:bool)->Result<PathBuf,StepError>{
    let step=if check{Step::P2Preflight}else{Step::P3Convert};
    if j.load().branch!=super::Branch::Upgrade || !name.starts_with(if check{"check-"}else{"convert-"}) {return Err(unsafe_logs());}
    let r=j.load().records.iter().rev().find(|r|r.step==step).ok_or_else(unsafe_logs)?;
    if r.state!=StepState::Intent || r.data.as_ref().and_then(|v|v.get("logName")).and_then(|v|v.as_str())!=Some(name){return Err(unsafe_logs());}
    path(db,&j.load().attempt_id,name)
}
fn metadata(path:&Path,device:u64,directory:bool,shared:bool)->Result<fs::Metadata,StepError>{
    let m=fs::symlink_metadata(path).map_err(io)?;let(uid,gid)=postgres::postgres_ids().map_err(|_|unsafe_logs())?;
    if m.dev()!=device||m.mode()&0o022!=0{return Err(unsafe_logs());}
    if m.is_dir(){if !((m.uid()==uid&&m.gid()==gid)||(shared&&m.uid()==0&&m.gid()==0))||m.mode()&0o7777!=0o700{return Err(unsafe_logs());}}
    else if directory||!m.is_file()||!(m.uid()==0||m.uid()==uid&&m.gid()==gid)||!(1..=2).contains(&m.nlink()){return Err(unsafe_logs());}Ok(m)
}
fn tree(path:&Path,device:u64,depth:u8,count:&mut u64,shared:bool)->Result<(),StepError>{
    *count=count.checked_add(1).ok_or_else(unsafe_logs)?;
    if *count>super::backup_format::MAX_ENTRIES || depth>64 {return Err(unsafe_logs());}
    if metadata(path,device,false,shared)?.is_dir(){for child in fs::read_dir(path).map_err(io)?{tree(&child.map_err(io)?.path(),device,depth+1,count,shared)?;}}Ok(())
}
/// Read-only classifier proof; validate every child before a later pruning pass mutates any.
pub(super) fn validate(db:&Path)->Result<(),StepError>{
    let logs=db.join(".nwui-auto-work/logs");if !exists(&logs)?{return Ok(());}
    crate::db_upgrade::files::real_path(&logs).map_err(|_|unsafe_logs())?;
    let shared=crate::db_upgrade::shared_folder::path_is_shared(db);let device=fs::symlink_metadata(db).map_err(io)?.dev();metadata(&logs,device,true,shared)?;
    for child in fs::read_dir(&logs).map_err(io)?{
        let child=child.map_err(io)?;let name=child.file_name();let name=name.to_str().ok_or_else(unsafe_logs)?;
        if !super::types::canonical_generation(name){return Err(unsafe_logs());}
        metadata(&child.path(),device,true,shared)?;tree(&child.path(),device,0,&mut 0,shared)?;
    }Ok(())
}
fn directory(path:&Path,device:u64,shared:bool)->Result<(),StepError>{
    if !exists(path)?{postgres::directory(path).map_err(|e|StepError{code:e.code,message:e.message,detail:None})?;}
    metadata(path,device,true,shared)?;Ok(())
}
/// Caller already persisted this step's Intent. Existing step directories are never adopted.
pub(super) fn create(roots:&Roots,locks:&StorageLocks,j:&Journal,name:&str,check:bool)->Result<PathBuf,StepError>{
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app){return Err(unsafe_logs());}
    let current=Journal::open(&roots.app,locks)?;
    if serde_json::to_value(current.load()).map_err(|_|unsafe_logs())?!=serde_json::to_value(j.load()).map_err(|_|unsafe_logs())?{return Err(unsafe_logs());}
    let target=owned_path(&roots.db,j,name,check)?;if exists(&target)?{return Err(unsafe_logs());}
    validate(&roots.db)?;
    let shared=crate::db_upgrade::shared_folder::path_is_shared(&roots.db);let device=fs::symlink_metadata(&roots.db).map_err(io)?.dev();let parent=roots.db.join(".nwui-auto-work");directory(&parent,device,shared)?;
    let logs=parent.join("logs");directory(&logs,device,shared)?;
    let opened=super::target::directory(&logs)?;
    for child in fs::read_dir(&logs).map_err(io)?{
        let child=child.map_err(io)?;let name=child.file_name();let name=name.to_str().ok_or_else(unsafe_logs)?;
        if name!=j.load().attempt_id{super::cleanup::delete_entry(&opened,name)?;checkpoint();}
    }
    directory(target.parent().ok_or_else(unsafe_logs)?,device,shared)?;
    postgres::directory(&target).map_err(|e|StepError{code:e.code,message:e.message,detail:None})?;metadata(&target,device,true,shared)?;Ok(target)
}
/// A P2 check's disposable target must be absent before its profile Done is committed.
pub(super) fn retire_check(roots:&Roots,locks:&StorageLocks,j:&Journal,name:&str)->Result<(),StepError>{
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app){return Err(unsafe_logs());}
    owned_path(&roots.db,j,name,true)?;
    let work=super::target::directory(&roots.db.join(".nwui-auto-work").join(&j.load().attempt_id))?;
    super::cleanup::delete_entry(&work,"data")
}
#[cfg(test)]fn checkpoint(){if std::env::var("NWUI_LOG_PRUNE_KILL").as_deref()==Ok("yes"){unsafe{libc::kill(libc::getpid(),libc::SIGKILL);}}}
#[cfg(not(test))]fn checkpoint(){}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{Branch,journal::Candidate};
    use std::os::unix::fs::{PermissionsExt,symlink};
    fn fixture()->(tempfile::TempDir,Roots,StorageLocks,Journal,String){
        let t=tempfile::tempdir().unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};postgres::directory(&roots.db).unwrap();crate::db_upgrade::files::private_dir(&roots.app).unwrap();let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j=Journal::begin(&roots.app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();j.ensure_app_pool().unwrap();let name=format!("check-{}",uuid::Uuid::new_v4());j.record(Step::P2Preflight,StepState::Intent,Some(serde_json::json!({"logName":name}))).unwrap();(t,roots,locks,j,name)
    }
    #[test]
    fn logs_are_intent_owned_outside_work_and_old_attempts_are_pruned_without_touching_current(){
        let(_t,r,l,mut j,name)=fixture();let check=create(&r,&l,&j,&name,true).unwrap();postgres::private_file(&check.join("preserved"),b"current").unwrap();
        let parent=r.db.join(".nwui-auto-work/logs");let old=parent.join(uuid::Uuid::new_v4().to_string());postgres::directory(&old).unwrap();postgres::private_file(&old.join("old"),b"prior").unwrap();
        let convert=format!("convert-{}",uuid::Uuid::new_v4());j.record(Step::P3Convert,StepState::Intent,Some(serde_json::json!({"logName":convert}))).unwrap();let log=create(&r,&l,&j,&convert,false).unwrap();assert_eq!(log.parent(),check.parent());assert!(!old.exists());assert_eq!(fs::read(check.join("preserved")).unwrap(),b"current");assert!(!r.db.join(".nwui-auto-work").join(&j.load().attempt_id).exists());assert!(create(&r,&l,&j,&convert,false).is_err());validate(&r.db).unwrap();
    }
    #[test]
    fn killed_root_link_probe_is_prunable_but_extra_aliases_are_preserved(){
        for extra in [false,true]{
            let(_t,r,l,j,name)=fixture();let parent=r.db.join(".nwui-auto-work");postgres::directory(&parent).unwrap();let logs=parent.join("logs");postgres::directory(&logs).unwrap();let old=logs.join(uuid::Uuid::new_v4().to_string());postgres::directory(&old).unwrap();
            let file=old.join("linkprobe");fs::write(&file,b"probe").unwrap();fs::set_permissions(&file,fs::Permissions::from_mode(0o600)).unwrap();assert_eq!(fs::symlink_metadata(&file).unwrap().uid(),0);fs::hard_link(&file,old.join("linkprobe.link")).unwrap();
            if extra{fs::hard_link(&file,old.join("third")).unwrap();assert!(create(&r,&l,&j,&name,true).is_err());assert_eq!(fs::read(&file).unwrap(),b"probe");}else{validate(&r.db).unwrap();create(&r,&l,&j,&name,true).unwrap();assert!(!old.exists());}
        }
    }
    #[test]
    fn foreign_or_unsafe_logs_refuse_before_any_prune(){
        for case in ["name","symlink","owner","fifo","world-writable"]{
            let(_t,r,l,j,name)=fixture();let parent=r.db.join(".nwui-auto-work");postgres::directory(&parent).unwrap();let logs=parent.join("logs");postgres::directory(&logs).unwrap();let good=logs.join(uuid::Uuid::new_v4().to_string());postgres::directory(&good).unwrap();postgres::private_file(&good.join("keep"),b"prior").unwrap();let bad=logs.join(if case=="name"{"not-a-uuid".into()}else{uuid::Uuid::new_v4().to_string()});
            if case=="symlink"{symlink(&good,&bad).unwrap();}else{postgres::directory(&bad).unwrap();match case{"owner"=>{let c=std::ffi::CString::new(bad.as_os_str().as_encoded_bytes()).unwrap();assert_eq!(unsafe{libc::chown(c.as_ptr(),0,0)},0);},"fifo"=>{let c=std::ffi::CString::new(bad.join("pipe").as_os_str().as_encoded_bytes()).unwrap();assert_eq!(unsafe{libc::mkfifo(c.as_ptr(),0o600)},0);},"world-writable"=>fs::set_permissions(&bad,fs::Permissions::from_mode(0o777)).unwrap(),_=>{}}}
            assert!(validate(&r.db).is_err(),"{case}");assert!(create(&r,&l,&j,&name,true).is_err(),"{case}");assert_eq!(fs::read(good.join("keep")).unwrap(),b"prior");assert!(fs::symlink_metadata(&bad).is_ok());
        }
    }
    #[test]
    fn log_grammar_and_step_intent_are_checked_before_creating_files(){
        let(_t,r,l,j,name)=fixture();for bad in ["check-../foreign".into(),format!("check-{}",uuid::Uuid::new_v4().simple()),format!("other-{}",uuid::Uuid::new_v4()),name.replace("check-","convert-")]{assert!(create(&r,&l,&j,&bad,true).is_err());}assert!(!r.db.join(".nwui-auto-work").exists());assert!(owned_path(&r.db,&j,&name,false).is_err());
    }
    #[test]
    fn check_scratch_removal_leaves_its_diagnostics_and_intent(){
        let(_t,r,l,j,name)=fixture();let logs=create(&r,&l,&j,&name,true).unwrap();postgres::private_file(&logs.join("check-report"),b"keep").unwrap();let w=r.db.join(".nwui-auto-work").join(&j.load().attempt_id);postgres::directory(&w).unwrap();postgres::directory(&w.join("data")).unwrap();postgres::private_file(&w.join("data/scratch"),b"remove").unwrap();retire_check(&r,&l,&j,&name).unwrap();assert!(!w.join("data").exists());assert_eq!(fs::read(logs.join("check-report")).unwrap(),b"keep");assert_eq!(j.load().records.last().unwrap().state,StepState::Intent);
    }
    #[test]
    fn prune_child(){
        let Ok(root)=std::env::var("NWUI_LOG_PRUNE_ROOT")else{return};let r=Roots{db:Path::new(&root).join("db"),app:Path::new(&root).join("app"),architecture:"arm64".into()};let l=StorageLocks::acquire(&r.db,&r.app).unwrap();let j=Journal::open(&r.app,&l).unwrap();create(&r,&l,&j,&std::env::var("NWUI_LOG_NAME").unwrap(),true).unwrap();panic!("kill not reached");
    }
    #[test]
    fn kill_mid_prune_resumes_and_keeps_original_data(){
        use std::os::unix::process::ExitStatusExt;
        let(t,r,l,j,name)=fixture();let parent=r.db.join(".nwui-auto-work");postgres::directory(&parent).unwrap();let logs=parent.join("logs");postgres::directory(&logs).unwrap();for _ in 0..2{let old=logs.join(uuid::Uuid::new_v4().to_string());postgres::directory(&old).unwrap();postgres::private_file(&old.join("old"),b"prior").unwrap();}postgres::private_file(&r.db.join("original"),b"untouched").unwrap();drop(j);drop(l);
        let result=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::diagnostics::tests::prune_child","--nocapture"]).env("NWUI_LOG_PRUNE_ROOT",t.path()).env("NWUI_LOG_NAME",&name).env("NWUI_LOG_PRUNE_KILL","yes").status().unwrap();assert_eq!(result.signal(),Some(9));
        let l=StorageLocks::acquire(&r.db,&r.app).unwrap();let j=Journal::open(&r.app,&l).unwrap();let log=create(&r,&l,&j,&name,true).unwrap();assert!(log.is_dir());assert_eq!(fs::read_dir(&logs).unwrap().count(),1);assert_eq!(fs::read(r.db.join("original")).unwrap(),b"untouched");
    }
}

#[cfg(test)]mod desktop_shared_tests{
 use super::*;use std::os::unix::fs::PermissionsExt;
 #[test]fn desktop_shared_diagnostic_directory_accepts_only_root_0700_exception(){
  let t=tempfile::tempdir().unwrap();let p=t.path();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();let dev=fs::metadata(p).unwrap().dev();
  assert!(metadata(p,dev,true,true).is_ok());assert!(metadata(p,dev,true,false).is_err());
  fs::set_permissions(p,fs::Permissions::from_mode(0o755)).unwrap();assert!(metadata(p,dev,true,true).is_err());
  fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();let c=std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).unwrap();assert_eq!(unsafe{libc::chown(c.as_ptr(),1234,1234)},0);assert!(metadata(p,dev,true,true).is_err());
 }
}
