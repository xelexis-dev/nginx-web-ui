//! Read-only capacity checks on the already-held volume FDs; floor calculation belongs to P0b/P2.
use super::{StepError,StorageLocks};
use crate::db_upgrade::layout::Roots;
use std::os::fd::{AsRawFd,BorrowedFd};
#[derive(Clone,Copy,Debug)]
pub(crate) struct Requirement {pub bytes:u64,pub inodes:u64}
#[derive(Clone,Copy,Debug)]
pub(crate) struct Available {pub bytes:u64,pub inodes:Option<u64>}
pub(crate) struct Checked {pub shared_filesystem:bool,pub db:Available,pub app:Available,app_block_bytes:u64}
fn error(code:&'static str)->StepError {StepError{code,message:"이전 작업에 필요한 저장 공간과 파일 수 여유를 확인하세요. 원본은 변경하지 않았습니다.",detail:None}}
fn stats(fd:BorrowedFd<'_>)->Result<(u64,Available,u64),StepError> {
    let mut metadata=std::mem::MaybeUninit::<libc::stat>::uninit();let mut storage=std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe{libc::fstat(fd.as_raw_fd(),metadata.as_mut_ptr())}<0 || unsafe{libc::fstatvfs(fd.as_raw_fd(),storage.as_mut_ptr())}<0{return Err(error("io_error"));}
    let metadata=unsafe{metadata.assume_init()};let storage=unsafe{storage.assume_init()};
    if metadata.st_mode&libc::S_IFMT!=libc::S_IFDIR || storage.f_frsize==0{return Err(error("unsupported_layout"));}
    let bytes=(storage.f_bavail as u64).checked_mul(storage.f_frsize as u64).ok_or_else(||error("unsupported_layout"))?;
    Ok((metadata.st_dev as u64,Available{bytes,inodes:if storage.f_files==0{None}else{Some(storage.f_favail as u64)}},storage.f_frsize as u64))
}
fn enough(have:Available,need:Requirement)->Result<(),StepError> {
    if have.bytes<need.bytes{return Err(error("insufficient_space"));}
    if have.inodes.is_some_and(|count|count<need.inodes){return Err(error("insufficient_inodes"));}Ok(())
}
fn verify(shared:bool,db:Available,app:Available,db_need:Requirement,app_need:Requirement)->Result<(),StepError> {
    if shared {
        let need=Requirement{bytes:db_need.bytes.checked_add(app_need.bytes).ok_or_else(||error("unsupported_layout"))?,inodes:db_need.inodes.checked_add(app_need.inodes).ok_or_else(||error("unsupported_layout"))?};
        let have=Available{bytes:db.bytes.min(app.bytes),inodes:match(db.inodes,app.inodes){(Some(a),Some(b))=>Some(a.min(b)),(known,None)|(None,known)=>known}};
        enough(have,need)
    }else{enough(db,db_need)?;enough(app,app_need)}
}
/// Caller supplies the phase's remaining requirements, not a blindly repeated initial budget.
/// No allocation, probe file, network, source scan or journal change is performed.
pub(crate) fn check(roots:&Roots,locks:&StorageLocks,db_need:Requirement,app_need:Requirement)->Result<Checked,StepError> {
    let checked=read_with(roots,locks,stats)?;
    verify(checked.shared_filesystem,checked.db,checked.app,db_need,app_need)?;Ok(checked)
}
fn read_with(roots:&Roots,locks:&StorageLocks,probe:impl Fn(BorrowedFd<'_>)->Result<(u64,Available,u64),StepError>)->Result<Checked,StepError>{
    if !locks.holds_d()||!locks.matches_roots(&roots.db,&roots.app){return Err(error("storage_busy"));}
    let(d,db,_)=probe(locks.d_fd().ok_or_else(||error("storage_busy"))?)?;let(a,app,app_block_bytes)=probe(locks.a_fd())?;
    Ok(Checked{shared_filesystem:d==a,db,app,app_block_bytes})
}

#[cfg(test)]
mod tests {
    use super::*;use std::{fs,os::unix::fs::PermissionsExt};
    #[test]
    fn w1_space_combines_shared_bytes_and_inodes_but_keeps_split_filesystems_independent() {
        let have=Available{bytes:10,inodes:Some(10)};let need=Requirement{bytes:6,inodes:6};assert_eq!(verify(true,have,have,need,need).unwrap_err().code,"insufficient_space");verify(false,have,have,need,need).unwrap();
        let need=Requirement{bytes:1,inodes:6};assert_eq!(verify(true,have,have,need,need).unwrap_err().code,"insufficient_inodes");let exact=Requirement{bytes:5,inodes:5};verify(true,have,have,exact,exact).unwrap();
        assert_eq!(verify(true,have,Available{bytes:9,inodes:Some(10)},exact,exact).unwrap_err().code,"insufficient_space");
    }
    #[test]
    fn w1_space_unknown_inode_counters_are_not_treated_as_empty_and_totals_never_wrap() {
        let have=Available{bytes:u64::MAX,inodes:None};let need=Requirement{bytes:1,inodes:10};verify(true,have,have,need,need).unwrap();assert_eq!(verify(true,have,Available{bytes:u64::MAX,inodes:Some(1)},need,need).unwrap_err().code,"insufficient_inodes");
        for inodes in [false,true] {let huge=Requirement{bytes:if inodes{0}else{u64::MAX},inodes:if inodes{u64::MAX}else{0}};assert_eq!(verify(true,have,have,huge,need).unwrap_err().code,"unsupported_layout");}
    }
    #[test]
    #[cfg(target_os="linux")]
    fn w1_space_real_distinct_filesystems_are_not_accidentally_aggregated() {
        let db=tempfile::tempdir_in("/dev/shm").unwrap();let app=tempfile::tempdir().unwrap();let roots=Roots{db:db.path().to_owned(),app:app.path().to_owned(),architecture:"arm64".into()};let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let zero=Requirement{bytes:0,inodes:0};let checked=check(&roots,&locks,zero,zero).unwrap();assert!(!checked.shared_filesystem);assert!(checked.db.bytes>0 && checked.app.bytes>0);assert!(fs::read_dir(&roots.db).unwrap().next().is_none());assert!(fs::read_dir(&roots.app).unwrap().next().is_none());
    }
    #[test]
    fn w1_space_real_held_fd_check_is_read_only_and_refuses_other_roots_or_missing_d_custody() {
        let t=tempfile::tempdir().unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for root in [&roots.db,&roots.app]{fs::create_dir(root).unwrap();fs::set_permissions(root,fs::Permissions::from_mode(0o700)).unwrap();}let mut locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let before=(crate::db_upgrade::files::snapshot(&roots.db).unwrap(),crate::db_upgrade::files::snapshot(&roots.app).unwrap());let zero=Requirement{bytes:0,inodes:0};let checked=check(&roots,&locks,zero,zero).unwrap();assert!(checked.shared_filesystem);assert!(checked.db.bytes>0 && checked.app.bytes>0);
        let mut other=roots.clone();other.app=t.path().join("other");fs::create_dir(&other.app).unwrap();assert_eq!(check(&other,&locks,zero,zero).err().unwrap().code,"storage_busy");locks.release_d();assert_eq!(check(&roots,&locks,zero,zero).err().unwrap().code,"storage_busy");assert_eq!(before,(crate::db_upgrade::files::snapshot(&roots.db).unwrap(),crate::db_upgrade::files::snapshot(&roots.app).unwrap()));
    }
}

#[cfg(test)] mod restored_manifest_tests {use super::*;use super::super::backup::manifest_bytes;
    fn sizing_header()->super::super::backup_format::Header{
        use super::super::backup_format::{Header,Root,SourceLocation};
        Header{version:1,backup_id:uuid::Uuid::new_v4().to_string(),source:"L".into(),created_at:"2026-10-29T00:00:00Z".into(),source_location:SourceLocation::Root,control_state:"shut down".into(),system_identifier:"123456".into(),checkpoint_sha256:"0".repeat(64),root:Root{mode:0o700,uid:0,gid:0}}
    }
    #[test]
    fn manifest_measurement_propagates_walk_errors_and_serializer_refusals(){
        use super::super::backup_format::{Entry,Kind,MAX_LINE};let header=sizing_header();
        assert_eq!(manifest_bytes(&header,std::iter::once(Err(error("io_error")))).unwrap_err().code,"io_error");
        let entry=Entry{kind:Kind::File,path:"p".repeat(MAX_LINE+1),bytes:1,mode:0o600,uid:0,gid:0,sha256:None};
        assert_eq!(manifest_bytes(&header,[Ok(entry)]).unwrap_err().code,"backup_manifest_too_large");
        let bad_dir=Entry{kind:Kind::Dir,path:"base".into(),bytes:0,mode:0o700,uid:0,gid:0,sha256:Some("0".repeat(64))};
        assert_eq!(manifest_bytes(&header,[Ok(bad_dir)]).unwrap_err().code,"verification_failed");
        let mut invalid=header;invalid.version=2;assert_eq!(manifest_bytes(&invalid,std::iter::empty()).unwrap_err().code,"verification_failed");
    }
    #[test]
    fn exact_manifest_serialization_exceeds_the_old_estimate_for_long_escaped_paths(){
        use super::super::backup_format::{StreamWriter,Entry,Kind};
        let header=sizing_header();
        let long=format!("base/{}/file\"\\가\n",vec!["p".repeat(200);12].join("/"));
        let mut entries=vec![Entry{kind:Kind::Dir,path:"base".into(),bytes:0,mode:0o700,uid:0,gid:0,sha256:None},Entry{kind:Kind::File,path:long,bytes:7,mode:0o600,uid:0,gid:0,sha256:Some("0".repeat(64))}];
        entries[1].sha256=None;let sized=manifest_bytes(&header,entries.iter().cloned().map(Ok)).unwrap();
        entries[1].sha256=Some("a".repeat(64));let mut actual=StreamWriter::new(Vec::new(),&header).unwrap();for e in &entries{actual.entry(e).unwrap();}let (bytes,written)=actual.finish().unwrap();assert_eq!(sized,written.bytes);assert_eq!(written.bytes,bytes.len()as u64);assert!(sized>entries.len()as u64*512);

    }
}
