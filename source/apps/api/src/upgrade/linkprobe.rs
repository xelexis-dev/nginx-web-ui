//! P2 Link/Copy selection: only the owned check log is writable; L and W are observed.
use super::StepError;
use crate::db_upgrade::migration::UpgradeMode;
use std::path::Path;

#[derive(Debug,PartialEq,Eq)]
pub(crate) struct Probe {pub mode:UpgradeMode,pub copy_reason:Option<&'static str>}

fn copy(reason:&'static str)->Probe {Probe{mode:UpgradeMode::Copy,copy_reason:Some(reason)}}
fn invalid()->StepError {StepError{code:"unsupported_layout",message:"하드 링크 검사에 사용할 데이터 경로를 확인하세요. 원본 파일은 변경하지 않았습니다.",detail:None}}
fn cleanup_error()->StepError {StepError{code:"io_error",message:"하드 링크 검사 파일을 정리하지 못했습니다. 작업 경로를 확인하세요.",detail:Some("link_probe_cleanup".into())}}
fn io_error(e:std::io::Error)->StepError {
    StepError{code:match e.raw_os_error(){Some(libc::ENOSPC|libc::EDQUOT)=>"insufficient_space",Some(libc::EROFS)=>"storage_read_only",Some(libc::EACCES)=>"permission_denied",Some(libc::EINTR)=>"interrupted",Some(libc::EEXIST)=>"unsupported_layout",_=>"io_error"},message:"하드 링크 검사를 완료하지 못했습니다. 작업 로그와 저장 공간·권한을 확인하세요." ,detail:Some(format!("link_probe: {e}"))}
}
fn directory(path:&Path)->Result<std::fs::File,StepError> {
    use std::os::unix::fs::OpenOptionsExt;
    crate::db_upgrade::files::real_path(path).map_err(|e|if matches!(e.kind(),std::io::ErrorKind::InvalidData|std::io::ErrorKind::NotFound){invalid()}else{io_error(e)})?;
    std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC).open(path).map_err(|e|if matches!(e.raw_os_error(),Some(libc::ENOTDIR|libc::ELOOP)){invalid()}else{io_error(e)})
}
// Same anchored-FD mount identity rule as cleanup::mount_id; never fall back to st_dev.
fn mount_id(file:&std::fs::File)->Result<u64,StepError> {
    use std::{io::Read,os::fd::AsRawFd};let mut text=String::new();
    std::fs::File::open(format!("/proc/self/fdinfo/{}",file.as_raw_fd())).map_err(io_error)?.take(4097).read_to_string(&mut text).map_err(io_error)?;
    if text.len()>4096{return Err(invalid());}let mut ids=text.lines().filter_map(|s|s.strip_prefix("mnt_id:")).map(str::trim);
    let id=ids.next().and_then(|s|s.parse::<u64>().ok()).ok_or_else(invalid)?;if ids.next().is_some(){return Err(invalid());}Ok(id)
}
fn entry_path(parent:&std::fs::File,name:&std::ffi::CStr)->std::path::PathBuf {
    use std::os::{fd::AsRawFd,unix::ffi::OsStrExt};
    Path::new("/proc/self/fd").join(parent.as_raw_fd().to_string()).join(std::ffi::OsStr::from_bytes(name.to_bytes()))
}
fn remove_probe(parent:&std::fs::File,name:&std::ffi::CStr,original:&std::fs::File)->Result<(),StepError> {
    use std::os::{fd::AsRawFd,unix::fs::MetadataExt};
    let cleanup=|e|{let mut error=io_error(e);error.detail=Some("link_probe_cleanup".into());error};
    let m=match std::fs::symlink_metadata(entry_path(parent,name)){Ok(m)=>m,Err(e)if e.kind()==std::io::ErrorKind::NotFound=>return Ok(()),Err(e)=>return Err(cleanup(e))};
    let expected=original.metadata().map_err(cleanup)?;
    if !m.is_file()||(m.dev(),m.ino())!=(expected.dev(),expected.ino()){return Err(cleanup_error());}
    if unsafe{libc::unlinkat(parent.as_raw_fd(),name.as_ptr(),0)}!=0{return Err(cleanup(std::io::Error::last_os_error()));}Ok(())
}
/// Caller supplies P2's check-<uuid> directory under its Intent, before pg_upgrade --check.
/// Only expected link limitations select Copy; all other failures abort P2.
pub(crate) fn probe(source_l:&Path,work_w:&Path,check_log_k:&Path)->Result<Probe,StepError> {
    use std::os::{fd::{AsRawFd,FromRawFd},unix::fs::MetadataExt};
    let id=check_log_k.file_name().and_then(|n|n.to_str()).and_then(|n|n.strip_prefix("check-")).ok_or_else(invalid)?;
    if uuid::Uuid::parse_str(id).ok().is_none_or(|u|u.to_string()!=id){return Err(invalid());}
    let source=directory(source_l)?;let work=directory(work_w)?;let check=directory(check_log_k)?;
    let original_dir=source.metadata().map_err(io_error)?;let check_dir=check.metadata().map_err(io_error)?;
    if (original_dir.dev(),original_dir.ino())==(check_dir.dev(),check_dir.ino()){return Err(invalid());}
    let work_mount=mount_id(&work)?;
    if mount_id(&source)?!=work_mount||mount_id(&check)?!=work_mount{return Ok(copy("different_mount"));}
    let fd=unsafe{libc::openat(check.as_raw_fd(),c"linkprobe".as_ptr(),libc::O_WRONLY|libc::O_CREAT|libc::O_EXCL|libc::O_NOFOLLOW|libc::O_CLOEXEC,0o600)};
    if fd<0{return Err(io_error(std::io::Error::last_os_error()));}
    let original=unsafe{std::fs::File::from_raw_fd(fd)};let mut linked=false;
    let outcome=(|| {
        let before=original.metadata().map_err(io_error)?;if !before.is_file(){return Err(invalid());}
        if unsafe{libc::linkat(check.as_raw_fd(),c"linkprobe".as_ptr(),check.as_raw_fd(),c"linkprobe.link".as_ptr(),0)}!=0 {
            let error=std::io::Error::last_os_error();return match error.raw_os_error(){
                Some(libc::EXDEV)=>Ok(copy("different_mount")),
                Some(errno)if [libc::EPERM,libc::ENOTSUP,libc::EOPNOTSUPP,libc::EMLINK].contains(&errno)=>Ok(copy("hard_link_failed")),
                _=>Err(io_error(error)),
            };
        }
        linked=true;let target=std::fs::symlink_metadata(entry_path(&check,c"linkprobe.link")).map_err(io_error)?;
        if !target.is_file()||(target.dev(),target.ino())!=(before.dev(),before.ino()){return Err(invalid());}
        Ok(Probe{mode:UpgradeMode::Link,copy_reason:None})
    })();
    // Both names belong only to K; never unlink a substituted foreign entry.
    let target_cleanup=if linked{remove_probe(&check,c"linkprobe.link",&original)}else{Ok(())};let source_cleanup=remove_probe(&check,c"linkprobe",&original);
    target_cleanup.and(source_cleanup)?;outcome
}

#[cfg(all(test,target_os="linux"))]
mod tests {
    use super::*;
    use std::{fs,os::{fd::AsRawFd,unix::{fs::{MetadataExt,PermissionsExt,symlink},process::CommandExt}},path::PathBuf,process::Command};
    fn entries(p:&Path)->Vec<std::ffi::OsString>{let mut v=fs::read_dir(p).unwrap().map(|e|e.unwrap().file_name()).collect::<Vec<_>>();v.sort();v}
    fn metadata(p:&Path)->(u32,u32,u32,u64,u64,u64,(i64,i64),(i64,i64),(i64,i64)) {let m=fs::metadata(p).unwrap();(m.uid(),m.gid(),m.mode(),m.len(),m.nlink(),m.ino(),(m.atime(),m.atime_nsec()),(m.mtime(),m.mtime_nsec()),(m.ctime(),m.ctime_nsec()))}
    fn dirs(root:&Path)->(PathBuf,PathBuf,PathBuf){let l=root.join("source");let w=root.join("work");let k=w.join(format!("check-{}",uuid::Uuid::new_v4()));fs::create_dir_all(&l).unwrap();fs::create_dir_all(&k).unwrap();(l,w,k)}
    fn observed_mount(p:&Path)->u64 {let f=directory(p).unwrap();fs::read_to_string(format!("/proc/self/fdinfo/{}",f.as_raw_fd())).unwrap().lines().find_map(|s|s.strip_prefix("mnt_id:")).unwrap().trim().parse().unwrap()}
    #[test]
    fn same_mount_probes_only_in_check_log_and_preserves_original_bytes_and_metadata(){
        let t=tempfile::tempdir().unwrap();let(l,w,k)=dirs(t.path());fs::write(l.join("keep"),b"unchanged").unwrap();let bytes=fs::read(l.join("keep")).unwrap();let before=(metadata(&l),metadata(&l.join("keep")));
        let selected=probe(&l,&w,&k).unwrap();assert_eq!((metadata(&l),metadata(&l.join("keep"))),before);assert_eq!(selected,Probe{mode:UpgradeMode::Link,copy_reason:None});assert_eq!(fs::read(l.join("keep")).unwrap(),bytes);assert_eq!(entries(&l),vec![std::ffi::OsString::from("keep")]);assert!(entries(&k).is_empty());
    }
    #[test]
    fn root_layout_original_can_contain_the_owned_work_tree_without_original_metadata_changes(){
        let t=tempfile::tempdir().unwrap();let w=t.path().join(".nwui-auto-work/attempt");let k=w.join(format!("check-{}",uuid::Uuid::new_v4()));fs::create_dir_all(&k).unwrap();fs::write(t.path().join("PG_VERSION"),b"15\n").unwrap();let before=(metadata(t.path()),metadata(&t.path().join("PG_VERSION")));
        assert_eq!(probe(t.path(),&w,&k).unwrap().mode,UpgradeMode::Link);assert_eq!((metadata(t.path()),metadata(&t.path().join("PG_VERSION"))),before);assert!(entries(&k).is_empty());assert_eq!(fs::read(t.path().join("PG_VERSION")).unwrap(),b"15\n");
    }
    #[test]
    fn source_and_work_may_alias_but_check_log_must_never_be_the_original(){
        let t=tempfile::tempdir().unwrap();let(l,_w,k)=dirs(t.path());let before=metadata(&l);assert_eq!(probe(&l,&l,&k).unwrap().mode,UpgradeMode::Link);assert_eq!(metadata(&l),before);assert!(probe(&l,&l,&l).is_err());
    }
    #[test]
    fn unsafe_or_missing_directories_are_refused_without_probe_files(){
        let t=tempfile::tempdir().unwrap();let(l,w,k)=dirs(t.path());let alias=t.path().join("alias");symlink(&k,&alias).unwrap();let file=t.path().join("file");fs::write(&file,b"keep").unwrap();
        for bad in [&alias,&file,&t.path().join("missing")]{assert!(probe(&l,&w,bad).is_err());assert!(entries(&l).is_empty());assert!(entries(&k).is_empty());}
    }
    #[test]
    fn fixed_probe_names_are_exclusive_and_preexisting_entries_are_not_removed(){
        for name in ["linkprobe","linkprobe.link"]{let t=tempfile::tempdir().unwrap();let(l,w,k)=dirs(t.path());fs::write(k.join(name),b"foreign").unwrap();let before=metadata(&l);assert!(probe(&l,&w,&k).is_err());assert_eq!(metadata(&l),before);assert_eq!(fs::read(k.join(name)).unwrap(),b"foreign");assert_eq!(entries(&k),vec![std::ffi::OsString::from(name)]);}
    }
    #[test]
    fn cleanup_refuses_to_unlink_a_substituted_foreign_file(){
        let t=tempfile::tempdir().unwrap();let original=fs::File::create(t.path().join("linkprobe")).unwrap();fs::rename(t.path().join("linkprobe"),t.path().join("owned-original")).unwrap();fs::write(t.path().join("linkprobe"),b"foreign").unwrap();
        let e=remove_probe(&directory(t.path()).unwrap(),c"linkprobe",&original).unwrap_err();assert_eq!(e.detail.as_deref(),Some("link_probe_cleanup"));assert_eq!(fs::read(t.path().join("linkprobe")).unwrap(),b"foreign");
    }
    #[test]
    fn permission_probe_child(){
        let Some(root)=std::env::var_os("NWUI_LINKPROBE_CHILD_ROOT")else{return};let root=Path::new(&root);let l=root.join("source");let w=root.join("work");let k=fs::read_dir(&w).unwrap().next().unwrap().unwrap().path();assert_ne!(unsafe{libc::geteuid()},0);
        let expected=std::env::var("NWUI_LINKPROBE_CHILD_EXPECT").unwrap();let result=probe(&l,&w,&k);if expected=="link"{assert_eq!(result.unwrap().mode,UpgradeMode::Link);}else{assert_eq!(result.unwrap_err().code,"permission_denied");}
    }
    #[test]
    fn readonly_original_and_work_still_link_but_check_log_eacces_is_a_failure(){
        if unsafe{libc::geteuid()}!=0{return;}
        for expected in ["link","permission_denied"]{let t=tempfile::tempdir().unwrap();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o711)).unwrap();let(l,w,k)=dirs(t.path());fs::set_permissions(&l,fs::Permissions::from_mode(0o555)).unwrap();fs::set_permissions(&w,fs::Permissions::from_mode(0o555)).unwrap();fs::set_permissions(&k,fs::Permissions::from_mode(if expected=="link"{0o777}else{0o555})).unwrap();let before=metadata(&l);
            let r=Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::linkprobe::tests::permission_probe_child","--nocapture","--test-threads=1"]).uid(65534).gid(65534).env("NWUI_LINKPROBE_CHILD_ROOT",t.path()).env("NWUI_LINKPROBE_CHILD_EXPECT",expected).output().unwrap();assert!(r.status.success(),"{expected}: {} {}",String::from_utf8_lossy(&r.stdout),String::from_utf8_lossy(&r.stderr));assert_eq!(metadata(&l),before);assert!(entries(&l).is_empty());assert!(entries(&k).is_empty());}
    }
    #[test]
    #[ignore="owned two-tmpfs fixture: NWUI_LINKPROBE_LEFT and NWUI_LINKPROBE_RIGHT"]
    fn two_tmpfs_and_a_separate_check_mount_choose_copy_without_touching_original(){
        let(Some(left),Some(right))=(std::env::var_os("NWUI_LINKPROBE_LEFT"),std::env::var_os("NWUI_LINKPROBE_RIGHT"))else{return};let left=tempfile::tempdir_in(left).unwrap();let right=tempfile::tempdir_in(right).unwrap();let k=right.path().join(format!("check-{}",uuid::Uuid::new_v4()));fs::create_dir(&k).unwrap();assert_ne!(observed_mount(left.path()),observed_mount(right.path()));let before=metadata(left.path());
        assert_eq!(probe(left.path(),right.path(),&k).unwrap(),Probe{mode:UpgradeMode::Copy,copy_reason:Some("different_mount")});assert_eq!(metadata(left.path()),before);assert_eq!(probe(left.path(),left.path(),&k).unwrap().copy_reason,Some("different_mount"));assert_eq!(metadata(left.path()),before);assert!(entries(left.path()).is_empty());assert!(entries(&k).is_empty());
    }
    #[test]
    #[ignore="owned two-bind fixture: NWUI_LINKPROBE_BIND_L and NWUI_LINKPROBE_BIND_W"]
    fn bind_mounted_original_has_same_device_but_a_different_mount_id_and_chooses_copy(){
        let(Some(l),Some(w))=(std::env::var_os("NWUI_LINKPROBE_BIND_L"),std::env::var_os("NWUI_LINKPROBE_BIND_W"))else{return};let l=tempfile::tempdir_in(l).unwrap();let w=tempfile::tempdir_in(w).unwrap();let k=w.path().join(format!("check-{}",uuid::Uuid::new_v4()));fs::create_dir(&k).unwrap();assert_eq!(fs::metadata(l.path()).unwrap().dev(),fs::metadata(w.path()).unwrap().dev());assert_ne!(observed_mount(l.path()),observed_mount(w.path()));let before=metadata(l.path());
        let result=probe(l.path(),w.path(),&k).unwrap();assert_eq!(metadata(l.path()),before);assert_eq!(result,Probe{mode:UpgradeMode::Copy,copy_reason:Some("different_mount")});assert!(entries(l.path()).is_empty());assert!(entries(&k).is_empty());
    }
    #[test]
    #[ignore="owned inode-limited tmpfs: NWUI_LINKPROBE_INODES"]
    fn an_exhausted_check_filesystem_is_insufficient_space_not_copy(){
        let Some(root)=std::env::var_os("NWUI_LINKPROBE_INODES")else{return};let t=tempfile::tempdir_in(root).unwrap();let(l,w,k)=dirs(t.path());let mut count=0;
        loop {match fs::File::create(t.path().join(format!("pressure-{count}"))){Ok(_)=>count+=1,Err(e)=>{assert_eq!(e.raw_os_error(),Some(libc::ENOSPC));break;}}}
        let before=metadata(&l);let error=probe(&l,&w,&k).unwrap_err();assert_eq!(error.code,"insufficient_space");assert_eq!(metadata(&l),before);assert!(entries(&l).is_empty());assert!(entries(&k).is_empty());
    }
    #[test]
    #[ignore="own process-local seccomp linkat EPERM fixture: NWUI_LINKPROBE_EPERM=1"]
    fn linkat_eperm_selects_copy_and_cleans_only_check_log(){
        if std::env::var("NWUI_LINKPROBE_EPERM").as_deref()!=Ok("1"){return;}
        let t=tempfile::tempdir().unwrap();let(l,w,k)=dirs(t.path());let before=metadata(&l);
        let mut filter=[libc::sock_filter{code:(libc::BPF_LD|libc::BPF_W|libc::BPF_ABS)as u16,jt:0,jf:0,k:0},libc::sock_filter{code:(libc::BPF_JMP|libc::BPF_JEQ|libc::BPF_K)as u16,jt:0,jf:1,k:libc::SYS_linkat as u32},libc::sock_filter{code:(libc::BPF_RET|libc::BPF_K)as u16,jt:0,jf:0,k:libc::SECCOMP_RET_ERRNO|libc::EPERM as u32},libc::sock_filter{code:(libc::BPF_RET|libc::BPF_K)as u16,jt:0,jf:0,k:libc::SECCOMP_RET_ALLOW}];let program=libc::sock_fprog{len:filter.len()as u16,filter:filter.as_mut_ptr()};
        assert_eq!(unsafe{libc::prctl(libc::PR_SET_NO_NEW_PRIVS,1,0,0,0)},0);assert_eq!(unsafe{libc::prctl(libc::PR_SET_SECCOMP,libc::SECCOMP_MODE_FILTER,&program as *const libc::sock_fprog,0,0)},0);
        let result=probe(&l,&w,&k).unwrap();assert_eq!(metadata(&l),before);assert_eq!(result,Probe{mode:UpgradeMode::Copy,copy_reason:Some("hard_link_failed")});assert!(entries(&l).is_empty());assert!(entries(&k).is_empty());
    }
}
