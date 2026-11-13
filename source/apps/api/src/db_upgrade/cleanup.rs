//! Remove only the recorded ready attempt's PG15 copy; keep all reports and target data.
use super::{engine::Engine,state::State,error::{Error,Result},files};
use std::{fs::{self,File},path::{Path,PathBuf}};
use std::os::{fd::AsRawFd,unix::fs::{MetadataExt,OpenOptionsExt}};
fn invalid()->Error{Error::refusal("unsupported_layout")}
// Anchor deletion to opened directories; only the recorded UUID and fixed pg15 name are used.
fn directory_handle(path:&Path)->Result<Option<File>> {
    match fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC).open(path) {
        Ok(file)=>Ok(Some(file)),Err(error)if error.kind()==std::io::ErrorKind::NotFound=>Ok(None),Err(_)=>Err(invalid()),
    }
}
fn descriptor_path(file:&File)->PathBuf{Path::new("/proc/self/fd").join(file.as_raw_fd().to_string())}
fn attempt_handle(engine:&Engine,state:&State)->Result<Option<File>> {
    state.validate().map_err(|_|invalid())?;let id=state.attempt_id.as_deref().filter(|id|super::state::uuid(id)).ok_or_else(invalid)?;
    files::real_path(&engine.roots.db).map_err(|_|invalid())?;
    let root=directory_handle(&engine.roots.db)?.ok_or_else(invalid)?;
    let Some(parent)=directory_handle(&descriptor_path(&root).join(".nwui-work"))? else{return Ok(None)};
    directory_handle(&descriptor_path(&parent).join(id))
}
pub(super) fn working_copy_present(engine:&Engine,state:&State)->Result<bool> {
    if state.mode!="migration"||state.attempt_id.is_none(){return Ok(false);}
    let Some(attempt)=attempt_handle(engine,state)? else{return Ok(false)};
    match fs::symlink_metadata(descriptor_path(&attempt).join("pg15")) {
        Ok(metadata)if metadata.is_dir()&&!metadata.file_type().is_symlink()=>Ok(true),
        Err(error)if error.kind()==std::io::ErrorKind::NotFound=>Ok(false),_=>Err(invalid()),
    }
}
pub(super) fn cleanup_ready(engine:&Engine,state:&State)->Result<()> {
    if !state.ready||state.mode!="migration"{return Ok(());}
    let Some(attempt)=attempt_handle(engine,state)? else{return Ok(())};
    let path=descriptor_path(&attempt).join("pg15");
    let metadata=match fs::symlink_metadata(&path){Ok(value)=>value,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>return Ok(()),Err(_)=>return Err(invalid())};
    let owner=fs::symlink_metadata(engine.roots.db.join("17/data")).map_err(|_|invalid())?.uid();
    if !metadata.is_dir()||metadata.file_type().is_symlink()||metadata.uid()!=owner||metadata.mode()&0o022!=0{return Err(invalid());}
    match fs::symlink_metadata(path.join("postmaster.pid")){Err(error)if error.kind()==std::io::ErrorKind::NotFound=>{},Ok(_)=>return Err(Error::refusal("storage_busy")),Err(_)=>return Err(invalid())}
    // Linux remove_dir_all uses no-follow descriptor-relative traversal, including racing links.
    fs::remove_dir_all(&path).and_then(|_|attempt.sync_all()).map_err(|_|Error{code:"cleanup_failed",message:"준비 완료 상태는 유지됩니다. PG15 작업 사본 정리를 완료하지 못했습니다. status로 확인하고 run을 다시 실행하세요.",exit:1,detail:None})
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    use super::super::{layout,state::{Phase,Operation,LastError}};
    use std::os::unix::fs::PermissionsExt;
    fn fixture()->(tempfile::TempDir,Engine,State,PathBuf) {
        let temp=tempfile::tempdir().unwrap();let root=temp.path();
        let engine=Engine{source_db:root.join("source-db"),source_app:root.join("source-app"),roots:layout::Roots{db:root.join("db"),app:root.join("app"),architecture:"arm64".into()}};
        for path in [&engine.source_db,&engine.source_app,&engine.roots.db,&engine.roots.app]{fs::create_dir(path).unwrap();fs::write(path.join("keep"),b"unchanged").unwrap();}
        let state:State=serde_json::from_str(include_str!("examples/ready.json")).unwrap();state.validate().unwrap();
        let work=engine.roots.db.join(".nwui-work").join(state.attempt_id.as_deref().unwrap());fs::create_dir_all(work.join("pg15")).unwrap();fs::set_permissions(work.join("pg15"),fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(work.join("pg15/keep"),b"copy").unwrap();fs::write(work.join("report.log"),b"retained report").unwrap();
        fs::create_dir_all(engine.roots.db.join("17/data")).unwrap();fs::write(engine.roots.db.join("17/data/keep"),b"target").unwrap();
        files::private_dir(&engine.roots.db.join(".nwui")).unwrap();files::atomic_json(&engine.roots.db.join(".nwui/migration-v1.json"),&state).unwrap();
        let other=work.parent().unwrap().join(uuid::Uuid::new_v4().to_string());fs::create_dir_all(other.join("pg15")).unwrap();fs::write(other.join("pg15/keep"),b"other attempt").unwrap();
        (temp,engine,state,work)
    }
    fn preserved(engine:&Engine,work:&Path)->Vec<String> {
        [&engine.source_db,&engine.source_app,&engine.roots.app,&engine.roots.db.join("17"),&engine.roots.db.join(".nwui"),&work.join("report.log")].map(|p|files::snapshot(p).unwrap()).to_vec()
    }
    #[test]
    fn ready_cleanup_is_idempotent_and_preserves_sources_target_records_reports_and_other_attempts() {
        let (_temp,engine,state,work)=fixture();let before=preserved(&engine,&work);
        let others=fs::read_dir(work.parent().unwrap()).unwrap().map(|e|e.unwrap().path()).filter(|p|p!=&work).map(|p|{let hash=files::snapshot(&p).unwrap();(p,hash)}).collect::<Vec<_>>();
        assert!(working_copy_present(&engine,&state).unwrap());cleanup_ready(&engine,&state).unwrap();assert!(!working_copy_present(&engine,&state).unwrap());
        cleanup_ready(&engine,&state).unwrap();assert_eq!(before,preserved(&engine,&work));for(path,hash)in others{assert_eq!(hash,files::snapshot(&path).unwrap());}
    }
    #[test]
    fn postgres_group_readable_copy_permissions_do_not_block_success_cleanup() {
        let (_temp,engine,state,work)=fixture();fs::set_permissions(work.join("pg15"),fs::Permissions::from_mode(0o750)).unwrap();cleanup_ready(&engine,&state).unwrap();assert!(!work.join("pg15").exists());
    }
    #[test]
    fn writable_or_foreign_owned_copy_roots_are_not_adopted_for_removal() {
        for foreign_owner in [false,true] {
            let (_temp,engine,state,work)=fixture();
            if foreign_owner{std::os::unix::fs::chown(work.join("pg15"),Some(65534),None).unwrap();}else{fs::set_permissions(work.join("pg15"),fs::Permissions::from_mode(0o777)).unwrap();}
            let before=files::snapshot(&work).unwrap();assert_eq!(cleanup_ready(&engine,&state).unwrap_err().code,"unsupported_layout");assert_eq!(before,files::snapshot(&work).unwrap());
        }
    }
    #[test]
    fn failed_and_nonready_attempts_never_remove_the_copy() {
        let (_temp,engine,mut state,work)=fixture();let before=files::snapshot(&work).unwrap();
        state.ready=false;state.phase=Phase::Converted;state.operation=Operation::Idle;state.validate().unwrap();cleanup_ready(&engine,&state).unwrap();
        state.operation=Operation::Failed;state.last_error=Some(LastError{code:"interrupted".into(),message:"interrupted".into()});state.validate().unwrap();cleanup_ready(&engine,&state).unwrap();
        assert_eq!(before,files::snapshot(&work).unwrap());
    }
    #[test]
    fn a_copy_root_symlink_refuses_without_touching_its_target_or_the_link() {
        let (_temp,engine,state,work)=fixture();fs::remove_dir_all(work.join("pg15")).unwrap();std::os::unix::fs::symlink(&engine.source_db,work.join("pg15")).unwrap();
        let before=files::snapshot(&engine.source_db).unwrap();assert_eq!(cleanup_ready(&engine,&state).unwrap_err().code,"unsupported_layout");
        assert_eq!(before,files::snapshot(&engine.source_db).unwrap());assert_eq!(fs::read_link(work.join("pg15")).unwrap(),engine.source_db);
    }
    #[test]
    fn an_attempt_root_symlink_and_a_forged_attempt_id_refuse_without_deletion() {
        let (temp,engine,mut state,work)=fixture();let outside=temp.path().join("outside");fs::rename(&work,&outside).unwrap();std::os::unix::fs::symlink(&outside,&work).unwrap();
        let before=files::snapshot(&outside).unwrap();assert_eq!(cleanup_ready(&engine,&state).unwrap_err().code,"unsupported_layout");assert_eq!(before,files::snapshot(&outside).unwrap());
        state.attempt_id=Some("../../outside".into());assert_eq!(cleanup_ready(&engine,&state).unwrap_err().code,"unsupported_layout");assert_eq!(before,files::snapshot(&outside).unwrap());
    }
    #[test]
    fn links_inside_the_copy_are_unlinked_without_following_them() {
        let (_temp,engine,state,work)=fixture();std::os::unix::fs::symlink(&engine.source_db,work.join("pg15/escape")).unwrap();
        let before=files::snapshot(&engine.source_db).unwrap();cleanup_ready(&engine,&state).unwrap();assert!(!work.join("pg15").exists());assert_eq!(before,files::snapshot(&engine.source_db).unwrap());
    }
    #[test]
    fn a_possible_live_postmaster_keeps_the_copy_and_ready_records_unchanged() {
        let (_temp,engine,state,work)=fixture();fs::write(work.join("pg15/postmaster.pid"),b"123\n").unwrap();let before=files::snapshot(&work).unwrap();let records=preserved(&engine,&work);
        assert_eq!(cleanup_ready(&engine,&state).unwrap_err().code,"storage_busy");assert_eq!(before,files::snapshot(&work).unwrap());assert_eq!(records,preserved(&engine,&work));
    }
    #[test]
    fn a_partial_removal_can_finish_without_pg15_control_files() {
        let (_temp,engine,state,work)=fixture();fs::remove_file(work.join("pg15/keep")).unwrap();cleanup_ready(&engine,&state).unwrap();assert!(!work.join("pg15").exists());
    }
}
