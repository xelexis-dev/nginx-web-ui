//! Fixed emergency space; allocation is explicit and releases never refill a pool.
use super::{Journal, StepError, StorageLocks};
use crate::db_upgrade::{files, layout::Roots, postgres};
use std::{ffi::CString, fs::{self, File}, io::{self, Write}, os::{fd::{AsRawFd, FromRawFd}, unix::fs::MetadataExt}, path::{Path, PathBuf}};
const DB_BYTES:u64=32*1024*1024;
#[derive(Debug,Default,PartialEq,Eq)]
pub(crate) struct ReserveReport {pub app_bytes:u64,pub app_files:u8,pub db_bytes:u64,pub db_files:u8}
fn unsafe_pool()->StepError {StepError{code:"unsupported_layout",message:"복구용 예약 공간의 구성을 확인하지 못했습니다. 기존 내용을 보존했습니다.",detail:None}}
fn io_error(e:io::Error)->StepError {StepError{code:if matches!(e.raw_os_error(),Some(libc::ENOSPC|libc::EDQUOT)){"insufficient_space"}else{"io_error"},message:"복구용 예약 공간을 처리하지 못했습니다. 기존 데이터를 보존했습니다.",detail:None}}
fn capacity()->StepError {StepError{code:"insufficient_space",message:"복구용 예약 공간이 부족합니다. 데이터 변경을 시작하지 않았습니다.",detail:Some(format!("appBytes={} dbBytes={}",8*2*1024*1024,8*DB_BYTES))}}
fn check(roots:&Roots,locks:&StorageLocks,j:&Journal,exclusive:bool)->Result<(),StepError> {
    if !locks.matches_app(&roots.app) || !j.belongs_to(&roots.app) || exclusive && !locks.matches_roots(&roots.db,&roots.app) {return Err(unsafe_pool());}
    let current=Journal::open(&roots.app,locks)?;
    if serde_json::to_vec(current.load()).ok()!=serde_json::to_vec(j.load()).ok(){return Err(unsafe_pool());}
    files::real_path(&roots.db).map_err(|_|unsafe_pool())?;Ok(())
}
fn fdpath(dir:&File)->PathBuf {PathBuf::from(format!("/proc/self/fd/{}",dir.as_raw_fd()))}
fn owned(uid:u32)->bool {uid==0 || postgres::postgres_ids().is_ok_and(|(id,_)|uid==id)}
fn directory(parent:&File,name:&std::ffi::CStr)->Result<Option<File>,StepError> {
    let fd=unsafe{libc::openat(parent.as_raw_fd(),name.as_ptr(),libc::O_RDONLY|libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC)};
    if fd<0 {let e=io::Error::last_os_error();return if e.kind()==io::ErrorKind::NotFound{Ok(None)}else{Err(unsafe_pool())};}
    let f=unsafe{File::from_raw_fd(fd)};let m=f.metadata().map_err(io_error)?;
    if !owned(m.uid()) || m.mode()&0o022!=0 || m.dev()!=parent.metadata().map_err(io_error)?.dev() {return Err(unsafe_pool());}Ok(Some(f))
}
fn db_root(roots:&Roots)->Result<File,StepError> {super::target::directory(&roots.db)}
fn db_pool(roots:&Roots)->Result<Option<(File,File)>,StepError> {
    let db=db_root(roots)?;let Some(parent)=directory(&db,c".nwui-auto-work")?else{return Ok(None)};
    Ok(directory(&parent,c"reserve")?.map(|pool|(parent,pool)))
}
fn leaf(pool:&File,n:u8,write:bool)->Result<Option<File>,StepError> {
    let name=CString::new(n.to_string()).unwrap();let fd=unsafe{libc::openat(pool.as_raw_fd(),name.as_ptr(),(if write{libc::O_RDWR}else{libc::O_RDONLY})|libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC)};
    if fd<0 {let e=io::Error::last_os_error();return if e.kind()==io::ErrorKind::NotFound{Ok(None)}else{Err(unsafe_pool())};}
    let f=unsafe{File::from_raw_fd(fd)};let m=f.metadata().map_err(io_error)?;
    if !m.is_file() || !owned(m.uid()) || m.mode()&0o022!=0 || m.nlink()!=1 || m.len()>DB_BYTES {return Err(unsafe_pool());}Ok(Some(f))
}
fn inventory(pool:&File)->Result<(u64,u8),StepError> {
    let mut bytes=0;let mut count=0;
    for entry in fs::read_dir(fdpath(pool)).map_err(io_error)? {
        let name=entry.map_err(io_error)?.file_name();let n=name.to_str().filter(|s|s.len()==1).and_then(|s|s.parse::<u8>().ok()).filter(|n|*n<8).ok_or_else(unsafe_pool)?;
        let f=leaf(pool,n,false)?.ok_or_else(unsafe_pool)?;bytes+=f.metadata().map_err(io_error)?.len();count+=1;
    }Ok((bytes,count))
}
fn unlink(pool:&File,n:u8)->Result<bool,StepError> {
    if leaf(pool,n,false)?.is_none(){return Ok(false);}
    let name=CString::new(n.to_string()).unwrap();if unsafe{libc::unlinkat(pool.as_raw_fd(),name.as_ptr(),0)}!=0{return Err(io_error(io::Error::last_os_error()));}
    pool.sync_all().map_err(io_error)?;Ok(true)
}
fn create_directory(parent:&File,name:&std::ffi::CStr,uid:u32,gid:u32)->Result<File,StepError> {
    if let Some(f)=directory(parent,name)?{return Ok(f);}
    if unsafe{libc::mkdirat(parent.as_raw_fd(),name.as_ptr(),0o700)}!=0{return Err(io_error(io::Error::last_os_error()));}
    let f=directory(parent,name)?.ok_or_else(unsafe_pool)?;
    if unsafe{libc::fchown(f.as_raw_fd(),uid,gid)}!=0{return Err(io_error(io::Error::last_os_error()));}
    f.sync_all().and_then(|_|parent.sync_all()).map_err(io_error)?;Ok(f)
}
fn space(roots:&Roots,locks:&StorageLocks,j:&Journal)->Result<(),StepError> {
    let (_,a_files)=j.app_reserves()?;
    let (_,d_files)=match db_pool(roots)?{Some((_,p))=>inventory(&p)?,None=>(0,0)};
    // Budget the fixed pools before allocating any inode; original data is never a reserve.
    let app_pool=roots.app.join(".nwui-auto/reserve");let db_pool=roots.db.join(".nwui-auto-work/reserve");
    let remaining=|pool:&Path,size:u64|->Result<u64,StepError>{let mut total=0;for n in 0..8{match fs::symlink_metadata(pool.join(n.to_string())){Ok(m)=>total+=size.saturating_sub(m.len().min(m.blocks().saturating_mul(512))),Err(e)if e.kind()==io::ErrorKind::NotFound=>total+=size,Err(e)=>return Err(io_error(e)),}}Ok(total)};
    let a=remaining(&app_pool,2*1024*1024)?;let d=remaining(&db_pool,DB_BYTES)?;
    let slots=roots.app.join(".nwui-auto/slots");let missing_slots=(0..16).filter(|n|!slots.join(n.to_string()).exists()).count()as u64;
    let a_inodes=8-u64::from(a_files)+missing_slots+u64::from(!slots.exists())+u64::from(!app_pool.exists());
    let d_inodes=8-u64::from(d_files)+u64::from(!db_pool.exists())+u64::from(!roots.db.join(".nwui-auto-work").exists());
    super::space::check(roots,locks,super::space::Requirement{bytes:d,inodes:d_inodes},super::space::Requirement{bytes:a,inodes:a_inodes}).map(|_|()).map_err(|e|if matches!(e.code,"insufficient_space"|"insufficient_inodes"){capacity()}else{e})
}
pub(crate) fn ensure(roots:&Roots,locks:&StorageLocks,j:&Journal)->Result<(),StepError> {
    check(roots,locks,j,true)?;
    if j.load().branch!=super::Branch::Upgrade{return Err(unsafe_pool());}
    // Validate both complete inventories before allocating even one inode.
    report(roots,locks,j)?;
    space(roots,locks,j)?;
    let result=(||{
        j.ensure_app_pool()?;
        let (uid,gid)=postgres::postgres_ids().map_err(|_|unsafe_pool())?;let db=db_root(roots)?;
        let parent=create_directory(&db,c".nwui-auto-work",uid,gid)?;let pool=create_directory(&parent,c"reserve",uid,gid)?;
        for n in 0..8 {
            let mut f=match leaf(&pool,n,true)? {Some(f)=>f,None=>{
                let name=CString::new(n.to_string()).unwrap();let fd=unsafe{libc::openat(pool.as_raw_fd(),name.as_ptr(),libc::O_RDWR|libc::O_CREAT|libc::O_EXCL|libc::O_NOFOLLOW|libc::O_CLOEXEC,0o600)};
                if fd<0{return Err(io_error(io::Error::last_os_error()));}let f=unsafe{File::from_raw_fd(fd)};
                if unsafe{libc::fchown(f.as_raw_fd(),uid,gid)}!=0{return Err(io_error(io::Error::last_os_error()));}f
            }};
            let m=f.metadata().map_err(io_error)?;if m.len()==DB_BYTES && m.blocks()*512>=DB_BYTES{continue;}
            f.set_len(0).map_err(io_error)?;let zeros=[0u8;65536];for _ in 0..DB_BYTES/65536{f.write_all(&zeros).map_err(io_error)?;}f.sync_all().map_err(io_error)?;
        }
        pool.sync_all().and_then(|_|parent.sync_all()).map_err(io_error)
    })();
    result.map_err(|e:StepError|if matches!(e.code,"insufficient_space"|"journal_write_failed"|"reserve_exhausted"){capacity()}else{e})
}
pub(crate) fn report(roots:&Roots,locks:&StorageLocks,j:&Journal)->Result<ReserveReport,StepError> {
    check(roots,locks,j,false)?;let (app_bytes,app_files)=j.app_reserves()?;
    let (db_bytes,db_files)=match db_pool(roots)?{Some((_,pool))=>inventory(&pool)?,None=>(0,0)};
    Ok(ReserveReport{app_bytes,app_files,db_bytes,db_files})
}
pub(crate) fn release_one_db(roots:&Roots,locks:&StorageLocks,j:&Journal)->Result<bool,StepError> {
    check(roots,locks,j,true)?;report(roots,locks,j)?;let Some((_,pool))=db_pool(roots)?else{return Ok(false)};inventory(&pool)?;
    for n in 0..8{if unlink(&pool,n)?{return Ok(true);}}Ok(false)
}
pub(crate) fn release_for_restore(roots:&Roots,locks:&StorageLocks,j:&Journal)->Result<u8,StepError> {
    check(roots,locks,j,true)?;report(roots,locks,j)?;let Some((_,pool))=db_pool(roots)?else{return Ok(0)};inventory(&pool)?;
    // Fixed half makes restarting R-restore idempotent rather than consuming the other half.
    let mut count=0;for n in 0..4{count+=u8::from(unlink(&pool,n)?);}Ok(count)
}
pub(crate) fn drain(roots:&Roots,locks:&StorageLocks,j:&Journal)->Result<(),StepError> {
    report(roots,locks,j)?;
    while j.release_app_reserve()?{}
    if let Some((parent,pool))=db_pool(roots)? {
        for n in 0..8{unlink(&pool,n)?;}
        if unsafe{libc::unlinkat(parent.as_raw_fd(),c"reserve".as_ptr(),libc::AT_REMOVEDIR)}!=0{return Err(io_error(io::Error::last_os_error()));}
        parent.sync_all().map_err(io_error)?;
    }Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{Branch,journal::Candidate};
    use std::os::unix::fs::{PermissionsExt,symlink};
    fn fixture()->(tempfile::TempDir,Roots,StorageLocks,Journal) {
        let t=tempfile::tempdir().unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
        for p in [&roots.db,&roots.app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let j=Journal::begin(&roots.app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();(t,roots,locks,j)
    }
    fn root_pool(roots:&Roots)->PathBuf {
        let p=roots.db.join(".nwui-auto-work/reserve");fs::create_dir_all(&p).unwrap();for p in [p.parent().unwrap(),&p]{fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}p
    }
    fn root_file(path:&Path){fs::write(path,b"reserve").unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o600)).unwrap();}
    #[test]
    fn w1_reserves_missing_pools_never_allocate_even_on_release_or_drain() {
        let (_t,r,l,j)=fixture();assert_eq!(report(&r,&l,&j).unwrap(),ReserveReport::default());
        assert!(!release_one_db(&r,&l,&j).unwrap());assert_eq!(release_for_restore(&r,&l,&j).unwrap(),0);drain(&r,&l,&j).unwrap();
        assert!(!r.db.join(".nwui-auto-work").exists());assert!(!r.app.join(".nwui-auto/reserve").exists());assert!(!r.app.join(".nwui-auto/slots").exists());
    }
    #[test]
    fn w1_reserves_partial_pool_resumes_real_allocated_bytes_and_postgres_ownership() {
        let (_t,r,l,j)=fixture();let p=root_pool(&r);root_file(&p.join("0"));j.ensure_app_pool().unwrap();
        fs::remove_file(r.app.join(".nwui-auto/reserve/0")).unwrap();ensure(&r,&l,&j).unwrap();
        assert_eq!(report(&r,&l,&j).unwrap(),ReserveReport{app_bytes:16*1024*1024,app_files:8,db_bytes:8*DB_BYTES,db_files:8});
        let ids=postgres::postgres_ids().unwrap();for n in 1..8{let m=fs::metadata(p.join(n.to_string())).unwrap();assert_eq!((m.uid(),m.gid()),ids);assert_eq!(m.mode()&0o7777,0o600);assert!(m.blocks()*512>=DB_BYTES);}
        let before=fs::metadata(p.join("1")).unwrap().ino();ensure(&r,&l,&j).unwrap();assert_eq!(fs::metadata(p.join("1")).unwrap().ino(),before);
    }
    #[test]
    fn w1_reserves_release_half_is_idempotent_and_single_release_does_not_refill() {
        let (_t,r,l,j)=fixture();let p=root_pool(&r);for n in 0..8{root_file(&p.join(n.to_string()));}
        assert_eq!(release_for_restore(&r,&l,&j).unwrap(),4);assert_eq!(release_for_restore(&r,&l,&j).unwrap(),0);
        assert_eq!(report(&r,&l,&j).unwrap().db_files,4);assert!(release_one_db(&r,&l,&j).unwrap());assert_eq!(report(&r,&l,&j).unwrap().db_files,3);assert!(!r.app.join(".nwui-auto/reserve").exists());
    }
    #[test]
    fn w1_reserves_a_only_drain_keeps_journal_slots_and_unrelated_work() {
        let (_t,r,mut l,j)=fixture();j.ensure_app_pool().unwrap();let p=root_pool(&r);root_file(&p.join("0"));root_file(&p.parent().unwrap().join("keep"));
        let count=fs::read_dir(r.app.join(".nwui-auto/slots")).unwrap().count();l.release_d();drain(&r,&l,&j).unwrap();
        assert!(!p.exists());assert!(p.parent().unwrap().join("keep").is_file());assert_eq!(fs::read_dir(r.app.join(".nwui-auto/slots")).unwrap().count(),count);assert_eq!(report(&r,&l,&j).unwrap(),ReserveReport::default());drain(&r,&l,&j).unwrap();
        assert!(ensure(&r,&l,&j).is_err());assert!(release_one_db(&r,&l,&j).is_err());
    }
    #[test]
    fn w1_reserves_refuses_foreign_link_hardlink_fifo_oversize_and_writable_before_any_release() {
        for side in ["app","db"]{for case in ["foreign","symlink","hardlink","fifo","oversize","writable","owner","parent-link"] {
            let (t,r,l,j)=fixture();let p=if side=="db"{root_pool(&r)}else{let p=r.app.join(".nwui-auto/reserve");fs::create_dir(&p).unwrap();fs::set_permissions(&p,fs::Permissions::from_mode(0o700)).unwrap();p};
            root_file(&p.join("0"));let target=p.join("1");match case {
                "foreign"=>root_file(&p.join("08")),
                "symlink"=>symlink(p.join("0"),&target).unwrap(),
                "hardlink"=>fs::hard_link(p.join("0"),&target).unwrap(),
                "fifo"=>{let c=CString::new(target.as_os_str().as_encoded_bytes()).unwrap();assert_eq!(unsafe{libc::mkfifo(c.as_ptr(),0o600)},0);},
                "oversize"=>{root_file(&target);File::options().write(true).open(&target).unwrap().set_len(if side=="db"{DB_BYTES+1}else{2*1024*1024+1}).unwrap();},
                "writable"=>fs::set_permissions(p.join("0"),fs::Permissions::from_mode(0o622)).unwrap(),
                "owner"=>{let f=File::open(p.join("0")).unwrap();assert_eq!(unsafe{libc::fchown(f.as_raw_fd(),10001,10001)},0);},
                "parent-link"=>{let moved=t.path().join("outside");fs::rename(&p,&moved).unwrap();symlink(moved,&p).unwrap();},_=>unreachable!(),
            }
            assert!(report(&r,&l,&j).is_err(),"{side} {case}");assert!(ensure(&r,&l,&j).is_err(),"{side} {case}");assert!(drain(&r,&l,&j).is_err(),"{side} {case}");
            assert!(p.join("0").exists(),"preserve {side} {case}");assert!(r.app.join(".nwui-auto/journal-v1.json").exists());
        }}
    }
    #[test]
    fn w1_reserves_stale_journal_and_foreign_roots_refuse_without_allocation() {
        let (_t,r,l,mut j)=fixture();let stale=Journal::open(&r.app,&l).unwrap();assert_eq!(report(&r,&l,&stale).unwrap(),ReserveReport::default());j.record(super::super::Step::P0b,super::super::StepState::Done,None).unwrap();
        // A.31 publishes P0b through init; only explicit ensure allocates pools/slots.
        assert!(!r.app.join(".nwui-auto/slots").exists());assert!(!r.app.join(".nwui-auto/reserve").exists());assert!(report(&r,&l,&stale).is_err());j.ensure_app_pool().unwrap();j.record(super::super::Step::P0b,super::super::StepState::Done,None).unwrap();assert!(ensure(&r,&l,&stale).is_err());
        let foreign=Roots{db:r.db.clone(),app:r.db.clone(),architecture:r.architecture.clone()};assert!(report(&foreign,&l,&j).is_err());assert!(!r.db.join(".nwui-auto-work").exists());
    }
    #[test]
    #[ignore="isolated Docker tmpfs: byte-limited or inode-limited /capacity"]
    fn w1_reserves_real_capacity_limit_refuses_before_pool_or_original_mutation() {
        if std::env::var_os("NWUI_RESERVE_CAPACITY_KIND").is_none(){return;}
        let t=tempfile::tempdir_in("/capacity").unwrap();let r=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
        for p in [&r.db,&r.app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        root_file(&r.db.join("original"));let l=StorageLocks::acquire(&r.db,&r.app).unwrap();let j=Journal::begin(&r.app,&l,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();
        let pressure=Path::new("/capacity/pressure");
        match std::env::var("NWUI_RESERVE_CAPACITY_KIND").unwrap().as_str() {
            "bytes"=>{let mut f=File::create(pressure).unwrap();let block=[0u8;65536];let e=loop{if let Err(e)=f.write_all(&block){break e;}};assert_eq!(e.raw_os_error(),Some(libc::ENOSPC));},
            "inodes"=>{fs::create_dir(pressure).unwrap();let mut n=0;let e=loop{match File::create(pressure.join(n.to_string())){Ok(_)=>n+=1,Err(e)=>break e}};assert_eq!(e.raw_os_error(),Some(libc::ENOSPC));},_=>panic!("capacity fixture kind"),
        }
        let before=files::snapshot(t.path()).unwrap();let e=ensure(&r,&l,&j).unwrap_err();assert_eq!(e.code,"insufficient_space");assert!(e.detail.unwrap().contains("dbBytes=268435456"));assert_eq!(files::snapshot(t.path()).unwrap(),before);
    }

}
