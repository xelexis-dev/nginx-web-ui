//! W1-owned target-root rename; A.16's per-entry helper keeps its closed alias table.
use super::{Branch,Journal,MovePhase,MoveRecord,Step,StepError,StepState,StorageLocks};
use crate::db_upgrade::{files,layout::Roots,postgres};
use std::{fs::{self,File,OpenOptions},os::{fd::AsRawFd,unix::fs::{DirBuilderExt,MetadataExt,OpenOptionsExt,PermissionsExt}},path::{Path,PathBuf}};
fn mismatch()->StepError {StepError {code:"pair_mismatch",message:"대상 데이터의 위치와 복구 기록이 다릅니다. 모든 위치를 보존했습니다.",detail:None}}
fn io_error(e:std::io::Error)->StepError {StepError {code:match e.raw_os_error(){Some(libc::ENOSPC|libc::EDQUOT)=>"insufficient_space",Some(libc::EROFS)=>"storage_read_only",Some(libc::EINTR)=>"interrupted",_=>"io_error"},message:"대상 데이터 위치 변경을 완료하지 못했습니다. 복구 기록과 데이터를 보존했습니다.",detail:None}}
fn exists(path:&Path)->Result<bool,StepError> {match fs::symlink_metadata(path){Ok(_)=>Ok(true),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>Ok(false),Err(e)=>Err(io_error(e))}}
pub(super) fn directory(path:&Path)->Result<File,StepError> {
    files::real_path(path).map_err(|_|mismatch())?;let f=OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW).open(path).map_err(io_error)?;let m=f.metadata().map_err(io_error)?;
    if !m.is_dir() || m.mode()&0o022!=0 || !(m.uid()==0 || postgres::postgres_ids().is_ok_and(|(uid,_)|m.uid()==uid)){return Err(mismatch());}Ok(f)
}
fn work(roots:&Roots,j:&Journal)->Result<PathBuf,StepError> {
    let id=&j.load().attempt_id;if !super::types::canonical_generation(id){return Err(mismatch());}directory(&roots.db.join(".nwui-auto-work"))?;Ok(roots.db.join(".nwui-auto-work").join(id))
}
fn check(roots:&Roots,locks:&StorageLocks,j:&Journal,id:&str)->Result<(),StepError> {
    if j.load().branch!=Branch::Upgrade || !j.belongs_to(&roots.app) || !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app)
        || !crate::db_upgrade::state::identifier(id) {return Err(mismatch());}super::replacement::precommit(&roots.app,j)
}
fn publication(j:&Journal,id:&str)->Result<bool,StepError> {
    let record=j.load().records.iter().rev().filter(|r|r.step==Step::P5Publish && matches!(r.state,StepState::Intent|StepState::Done)).filter_map(|r|r.data.as_ref()).find(|v|v.get("targetEvent").and_then(|s|s.as_str())==Some("publishing"));
    if let Some(record)=record {if record.get("systemIdentifier").and_then(|s|s.as_str())!=Some(id) || record.get("attemptId").and_then(|s|s.as_str())!=Some(j.load().attempt_id.as_str()){return Err(mismatch());}}
    Ok(record.is_some())
}
fn read_id(path:&Path)->Result<String,StepError> {
    let control=postgres::read_control(17,path).map_err(|e|StepError {code:e.code,message:e.message,detail:None})?;
    control.validate().and_then(|_|control.require_cold()).map_err(|e|StepError {code:e.code,message:e.message,detail:None})?;Ok(control.system_identifier)
}
#[cfg(test)]fn checkpoint(point:&str) {if std::env::var("NWUI_TARGET_KILL_POINT").as_deref()==Ok(point){unsafe{libc::kill(libc::getpid(),libc::SIGKILL);}}}
#[cfg(not(test))]fn checkpoint(_point:&str) {}
fn move_root(locks:&StorageLocks,j:&mut Journal,source:&File,dest:&File,source_name:&std::ffi::CStr,dest_name:&std::ffi::CStr,entry:&str,from:&str,to:&str,id:&str,read:&dyn Fn(&Path)->Result<String,StepError>,source_path:&Path,dest_path:&Path)->Result<(),StepError> {
    if !locks.holds_a() || !locks.holds_d() {return Err(mismatch());}
    let a=source.metadata().map_err(io_error)?;let b=dest.metadata().map_err(io_error)?;if a.dev()!=b.dev() || a.ino()==b.ino(){return Err(mismatch());}
    let previous=j.load().moves.iter().rev().find(|r|r.record.phase==MovePhase::Target && r.record.entry==entry).map(|r|r.record.clone());
    if previous.as_ref().is_some_and(|r|r.from!=from || r.to!=to || !matches!(r.state,StepState::Intent|StepState::Done)){return Err(mismatch());}
    let src=exists(source_path)?;let dst=exists(dest_path)?;
    if src==dst || previous.is_none() && !src || previous.as_ref().is_some_and(|r|r.state==StepState::Done) && src {return Err(mismatch());}
    let actual=if src{source_path}else{dest_path};let opened=directory(actual)?;if opened.metadata().map_err(io_error)?.mode()&0o7777!=0o700 || read(actual)?!=id{return Err(mismatch());}
    if previous.is_none(){j.record_move(MoveRecord {phase:MovePhase::Target,entry:entry.into(),from:from.into(),to:to.into(),state:StepState::Intent})?;checkpoint("intent");}
    if src {
        if unsafe{libc::renameat(source.as_raw_fd(),source_name.as_ptr(),dest.as_raw_fd(),dest_name.as_ptr())}!=0{return Err(io_error(std::io::Error::last_os_error()));}checkpoint("rename");
    }
    source.sync_all().map_err(io_error)?;checkpoint("source-fsync");dest.sync_all().map_err(io_error)?;checkpoint("dest-fsync");
    if exists(source_path)? || !exists(dest_path)? || read(dest_path)?!=id {return Err(mismatch());}
    if !previous.is_some_and(|r|r.state==StepState::Done) {j.record_move(MoveRecord {phase:MovePhase::Target,entry:entry.into(),from:from.into(),to:to.into(),state:StepState::Done})?;}Ok(())
}
/// Only after the private PG17 has stopped. The cold control identifier is re-read here.
pub(crate) fn publish(roots:&Roots,locks:&StorageLocks,j:&mut Journal,id:&str)->Result<(),StepError> {publish_with(roots,locks,j,id,&read_id)}
fn publish_with(roots:&Roots,locks:&StorageLocks,j:&mut Journal,id:&str,read:&dyn Fn(&Path)->Result<String,StepError>)->Result<(),StepError> {
    check(roots,locks,j,id)?;let w=work(roots,j)?;let staged=w.join("data");let parent=roots.db.join("17");
    if !publication(j,id)? {
        if exists(&parent)? || !exists(&staged)? {return Err(mismatch());}directory(&staged)?;if read(&staged)?!=id{return Err(mismatch());}
        j.record(Step::P5Publish,StepState::Intent,Some(serde_json::json!({"targetEvent":"publishing","systemIdentifier":id,"attemptId":j.load().attempt_id})))?;
    }
    // Root owns this container directory; postgres must traverse it to reach its 0700 data root.
    if !exists(&parent)? {fs::DirBuilder::new().mode(0o755).create(&parent).map_err(io_error)?;File::open(&roots.db).and_then(|f|f.sync_all()).map_err(io_error)?;}
    if fs::read_dir(&parent).map_err(io_error)?.any(|r|r.map_or(true,|e|e.file_name()!="data")){return Err(mismatch());}
    let source=directory(&w)?;let dest=directory(&parent)?;
    // An intent-owned mkdir may have run under umask 077 or died before the chmod.
    let metadata=dest.metadata().map_err(io_error)?;
    if metadata.uid()==0 && metadata.mode()&0o7777==0o700 {dest.set_permissions(fs::Permissions::from_mode(0o755)).map_err(io_error)?;dest.sync_all().map_err(io_error)?;}
    File::open(&roots.db).and_then(|f|f.sync_all()).map_err(io_error)?;
    move_root(locks,j,&source,&dest,c"data",c"data","data","W","D17",id,read,&staged,&parent.join("data"))
}
/// Pre-commit R-restore only: keep the uncommitted PG17 under W/target, never delete it here.
pub(crate) fn quarantine(roots:&Roots,locks:&StorageLocks,j:&mut Journal,id:&str)->Result<(),StepError> {quarantine_with(roots,locks,j,id,&read_id)}
fn quarantine_with(roots:&Roots,locks:&StorageLocks,j:&mut Journal,id:&str,read:&dyn Fn(&Path)->Result<String,StepError>)->Result<(),StepError> {
    check(roots,locks,j,id)?;if !publication(j,id)? {return Err(mismatch());}
    let w=work(roots,j)?;let parent=roots.db.join("17");let source=directory(&parent)?;let dest=directory(&w)?;
    move_root(locks,j,&source,&dest,c"data",c"target","target","D17","W",id,read,&parent.join("data"),&w.join("target"))
}
#[cfg(test)]
mod tests {
    use super::*;
    use super::super::journal::Candidate;
    use std::os::unix::fs::{PermissionsExt,symlink};
    fn fixture()->(tempfile::TempDir,Roots,StorageLocks,Journal,PathBuf) {
        let t=tempfile::tempdir().unwrap();let roots=Roots {db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for p in [&roots.db,&roots.app] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let j={let fixture_journal=Journal::begin(&roots.app,&locks,Branch::Upgrade,Candidate {revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};
        let parent=roots.db.join(".nwui-auto-work");fs::create_dir(&parent).unwrap();let w=work(&roots,&j).unwrap();fs::create_dir(&w).unwrap();fs::set_permissions(&w,fs::Permissions::from_mode(0o700)).unwrap();fs::create_dir(w.join("data")).unwrap();fs::set_permissions(w.join("data"),fs::Permissions::from_mode(0o700)).unwrap();fs::write(w.join("data/identity"),"123456").unwrap();(t,roots,locks,j,w)
    }
    fn identity(path:&Path)->Result<String,StepError>{fs::read_to_string(path.join("identity")).map_err(io_error)}
    fn retry_locks(roots:&Roots)->StorageLocks {
        let until=std::time::Instant::now()+std::time::Duration::from_secs(3);loop {match StorageLocks::acquire(&roots.db,&roots.app) {Ok(v)=>return v,Err(e)if e.code=="storage_busy" && std::time::Instant::now()<until=>std::thread::sleep(std::time::Duration::from_millis(5)),Err(e)=>panic!("{e}")}}
    }
    #[test]
    fn w1_target_resumes_intent_owned_root_private_parent_without_widening_data_mode() {
        let (_t,roots,locks,mut j,w)=fixture();
        j.record(Step::P5Publish,StepState::Intent,Some(serde_json::json!({"targetEvent":"publishing","systemIdentifier":"123456","attemptId":j.load().attempt_id}))).unwrap();
        let parent=roots.db.join("17");fs::create_dir(&parent).unwrap();fs::set_permissions(&parent,fs::Permissions::from_mode(0o700)).unwrap();
        publish_with(&roots,&locks,&mut j,"123456",&identity).unwrap();assert_eq!(fs::metadata(&parent).unwrap().mode()&0o7777,0o755);assert_eq!(fs::metadata(parent.join("data")).unwrap().mode()&0o7777,0o700);assert!(!w.join("data").exists());
    }
    #[test]
    fn w1_target_kill_child() {
        let Some(base)=std::env::var_os("NWUI_TARGET_TEST_ROOT")else{return;};let base=PathBuf::from(base);let roots=Roots {db:base.join("db"),app:base.join("app"),architecture:"arm64".into()};let locks=retry_locks(&roots);let mut j=Journal::open(&roots.app,&locks).unwrap();
        if std::env::var("NWUI_TARGET_TEST_OPERATION").as_deref()==Ok("quarantine") {quarantine_with(&roots,&locks,&mut j,"123456",&identity).unwrap();}else{publish_with(&roots,&locks,&mut j,"123456",&identity).unwrap();}
    }
    #[test]
    fn w1_target_real_kills_resume_both_root_renames_and_parent_syncs() {
        use std::os::unix::process::ExitStatusExt;
        for operation in ["publish","quarantine"] {for point in ["intent","rename","source-fsync","dest-fsync"] {
            let (t,roots,locks,mut j,w)=fixture();let inode=fs::metadata(w.join("data/identity")).unwrap().ino();
            if operation=="quarantine" {publish_with(&roots,&locks,&mut j,"123456",&identity).unwrap();}drop(j);drop(locks);
            let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::target::tests::w1_target_kill_child","--nocapture"])
                .env("NWUI_TARGET_TEST_ROOT",t.path()).env("NWUI_TARGET_TEST_OPERATION",operation).env("NWUI_TARGET_KILL_POINT",point).status().unwrap();assert_eq!(status.signal(),Some(9),"{operation}/{point}");
            let locks=retry_locks(&roots);let mut j=Journal::open(&roots.app,&locks).unwrap();let final_path=if operation=="quarantine" {quarantine_with(&roots,&locks,&mut j,"123456",&identity).unwrap();w.join("target/identity")}else{publish_with(&roots,&locks,&mut j,"123456",&identity).unwrap();roots.db.join("17/data/identity")};
            assert_eq!(fs::metadata(&final_path).unwrap().ino(),inode);assert_eq!(fs::read(final_path).unwrap(),b"123456");assert!(!roots.db.join(".nwui").exists());
        }}
    }
    #[test]
    fn w1_target_publish_and_quarantine_resume_idempotently_preserving_inode_and_bytes() {
        let (_t,roots,locks,mut j,w)=fixture();let before=fs::metadata(w.join("data/identity")).unwrap().ino();publish_with(&roots,&locks,&mut j,"123456",&identity).unwrap();publish_with(&roots,&locks,&mut j,"123456",&identity).unwrap();
        assert_eq!(before,fs::metadata(roots.db.join("17/data/identity")).unwrap().ino());assert!(!roots.db.join(".nwui").exists());
        quarantine_with(&roots,&locks,&mut j,"123456",&identity).unwrap();quarantine_with(&roots,&locks,&mut j,"123456",&identity).unwrap();assert_eq!(before,fs::metadata(w.join("target/identity")).unwrap().ino());assert!(!roots.db.join("17/data").exists());
    }
    #[test]
    fn w1_target_foreign_parent_symlink_identity_or_committed_selector_is_never_adopted() {
        for change in ["parent","symlink","identity","selector"] {let (_t,roots,locks,mut j,w)=fixture();match change {"parent"=>fs::create_dir(roots.db.join("17")).unwrap(),"symlink"=>{fs::rename(w.join("data"),w.join("foreign")).unwrap();symlink(w.join("foreign"),w.join("data")).unwrap();},"identity"=>fs::write(w.join("data/identity"),"different").unwrap(),_=>fs::write(roots.app.join(".nwui-auto/active-v1.json"),"committed").unwrap()};
            assert!(publish_with(&roots,&locks,&mut j,"123456",&identity).is_err());assert!(w.join("data").exists());assert!(!roots.db.join("17/data").exists());
        }
    }
}
