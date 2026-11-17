//! Historical guard reader for committed PG17 cleanup; no guard writer or restore.
use super::StepError;
use crate::db_upgrade::postgres;
use std::{fs::File,io::Read,os::{fd::{AsRawFd,FromRawFd},unix::fs::MetadataExt}};
const MARKER:&std::ffi::CStr=c"UPGRADED-TO-PG17.txt";
fn mismatch()->StepError {StepError{code:"pair_mismatch",message:"이전 버전의 구동 차단 기록과 실제 원본 위치가 다릅니다. 모든 사본을 보존했습니다.",detail:None}}
fn io_error(e:std::io::Error)->StepError {super::backup_format::io_error(e)}
fn stat(parent:&File,name:&std::ffi::CStr)->Result<Option<libc::stat>,StepError> {
    let mut m=std::mem::MaybeUninit::uninit();if unsafe{libc::fstatat(parent.as_raw_fd(),name.as_ptr(),m.as_mut_ptr(),libc::AT_SYMLINK_NOFOLLOW)}==0{return Ok(Some(unsafe{m.assume_init()}));}
    let e=std::io::Error::last_os_error();if e.kind()==std::io::ErrorKind::NotFound{Ok(None)}else{Err(io_error(e))}
}
fn regular(m:&libc::stat)->bool {m.st_mode&libc::S_IFMT==libc::S_IFREG && m.st_nlink==1 && m.st_mode&0o022==0 && (m.st_uid==0 || postgres::postgres_ids().is_ok_and(|(uid,_)|uid==m.st_uid))}
fn read(parent:&File,name:&std::ffi::CStr,max:u64)->Result<(File,Vec<u8>),StepError> {
    let fd=unsafe{libc::openat(parent.as_raw_fd(),name.as_ptr(),libc::O_RDONLY|libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC)};
    if fd<0{return Err(io_error(std::io::Error::last_os_error()));}let mut f=unsafe{File::from_raw_fd(fd)};let m=f.metadata().map_err(io_error)?;
    let snapshot=stat(parent,name)?.ok_or_else(mismatch)?;
    if !regular(&snapshot) || m.dev()!=snapshot.st_dev || m.ino()!=snapshot.st_ino || m.len()>max {return Err(mismatch());}
    let mut bytes=Vec::new();(&mut f).take(max+1).read_to_end(&mut bytes).map_err(io_error)?;if bytes.len()as u64>max{return Err(mismatch());}Ok((f,bytes))
}
fn content(backup:&str,generation:&str)->String {
    format!("NWUI: PostgreSQL 17 upgrade. Do not start an older image on these volumes.\nBackup: {backup}\nGeneration: {generation}\nStop the container and follow the documented PG15 restore procedure. Keep both volumes and the backup.\n")
}
pub(crate) fn verify_marker(parent:&File,backup:&str,generation:&str)->Result<(),StepError> {
    if !super::types::canonical_generation(backup) || !super::types::canonical_generation(generation){return Err(mismatch());}
    let m=parent.metadata().map_err(io_error)?;
    if !m.is_dir() || m.mode()&0o022!=0 || !(m.uid()==0 || postgres::postgres_ids().is_ok_and(|(uid,_)|uid==m.uid())){return Err(mismatch());}
    let expected=content(backup,generation);let (_,bytes)=read(parent,MARKER,expected.len()as u64)?;
    if bytes!=expected.as_bytes(){return Err(mismatch());}Ok(())
}

#[cfg(test)] mod tests {use super::*;use std::fs;
    #[test]
    fn w1_guard_read_only_marker_verification_reuses_exact_bytes_and_rejects_tampering() {
        let t=tempfile::tempdir().unwrap();let parent=super::super::target::directory(t.path()).unwrap();let backup=uuid::Uuid::new_v4().to_string();let generation=uuid::Uuid::new_v4().to_string();
        fs::write(t.path().join("UPGRADED-TO-PG17.txt"),content(&backup,&generation)).unwrap();let before=crate::db_upgrade::files::snapshot(t.path()).unwrap();verify_marker(&parent,&backup,&generation).unwrap();assert_eq!(crate::db_upgrade::files::snapshot(t.path()).unwrap(),before);
        assert!(verify_marker(&parent,&backup,&uuid::Uuid::new_v4().to_string()).is_err());assert!(verify_marker(&parent,"../escape",&generation).is_err());assert_eq!(crate::db_upgrade::files::snapshot(t.path()).unwrap(),before);
        fs::write(t.path().join("UPGRADED-TO-PG17.txt"),b"foreign").unwrap();assert!(verify_marker(&parent,&backup,&generation).is_err());assert_eq!(fs::read(t.path().join("UPGRADED-TO-PG17.txt")).unwrap(),b"foreign");
    }
}
