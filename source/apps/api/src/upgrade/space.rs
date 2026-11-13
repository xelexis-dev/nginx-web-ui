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
/// A.31 measured inputs; manifest_bytes includes the actual header, placeholder entries and trailer.
/// The caller uses the manifest writer itself; this helper rounds its raw total on A.
#[derive(Clone,Copy,Debug)]
pub(crate) struct Usage {pub db_bytes:u64,pub app_bytes:u64,pub largest_index_bytes:u64,pub manifest_bytes:u64,pub db_files:u64,pub app_files:u64}
#[derive(Clone,Copy,Debug)]
pub(crate) enum Phase {P0b,P2{backup_bytes:u64}}
#[derive(Debug)]
pub(crate) struct SpaceAssessment {pub space_sufficient:bool,pub space:super::types::StatusSpace}
impl SpaceAssessment {pub(crate) fn require(&self)->Result<(),StepError>{if self.space_sufficient{Ok(())}else{Err(error("insufficient_space"))}}}
/// Publish the returned space first; require() then refuses a shortfall with its status retained.
pub(crate) fn assess(roots:&Roots,locks:&StorageLocks,j:&super::Journal,usage:Usage,phase:Phase)->Result<SpaceAssessment,StepError>{
    let pools=super::reserves::report(roots,locks,j)?;let have=read_with(roots,locks,stats)?;assess_available(usage,phase,&pools,have)
}
fn read_with(roots:&Roots,locks:&StorageLocks,probe:impl Fn(BorrowedFd<'_>)->Result<(u64,Available,u64),StepError>)->Result<Checked,StepError>{
    if !locks.holds_d()||!locks.matches_roots(&roots.db,&roots.app){return Err(error("storage_busy"));}
    let(d,db,_)=probe(locks.d_fd().ok_or_else(||error("storage_busy"))?)?;let(a,app,app_block_bytes)=probe(locks.a_fd())?;
    Ok(Checked{shared_filesystem:d==a,db,app,app_block_bytes})
}
/// P0b metadata walk: reuse the actual P1 header and the writer's bounded line encoder.
pub(crate) fn manifest_bytes(header:&super::backup_format::Header,entries:impl IntoIterator<Item=Result<super::backup_format::Entry,StepError>>)->Result<u64,StepError>{
    let mut writer=super::backup_format::StreamWriter::new(std::io::sink(),header)?;
    for entry in entries{
        let mut entry=entry?;if entry.kind==super::backup_format::Kind::File{entry.sha256=Some("0".repeat(64));}
        writer.entry(&entry)?;
    }
    Ok(writer.finish()?.1.bytes)
}
/// Round the one complete manifest file, not every JSON line separately.
fn manifest_allocation(bytes:u64,block_bytes:u64)->Result<u64,StepError>{
    if bytes>super::backup_format::MAX_FILE{return Err(super::backup_format::too_large());}
    if block_bytes==0{return Err(error("unsupported_layout"));}
    let padding=(block_bytes-bytes%block_bytes)%block_bytes;
    bytes.checked_add(padding).ok_or_else(||error("unsupported_layout"))
}
fn assess_available(usage:Usage,phase:Phase,pools:&super::reserves::ReserveReport,have:Checked)->Result<SpaceAssessment,StepError>{
    let invalid=||error("unsupported_layout");
    let floor=crate::db_upgrade::engine::required_bytes(usage.db_bytes,0,usage.largest_index_bytes).map_err(|e|StepError{code:e.code,message:e.message,detail:None})?;
    let (db_need,app_need)=match phase{
        Phase::P0b=>{
            // A.26 fixed pools; only still-missing files consume additional space.
            let db_pool=(256u64*1024*1024).checked_sub(pools.db_bytes).ok_or_else(invalid)?;
            let app_pool=(16u64*1024*1024).checked_sub(pools.app_bytes).ok_or_else(invalid)?;
            let db_missing=8u64.checked_sub(pools.db_files.into()).ok_or_else(invalid)?;
            let app_missing=8u64.checked_sub(pools.app_files.into()).ok_or_else(invalid)?;
            (Requirement{bytes:floor.checked_add(db_pool).ok_or_else(invalid)?,inodes:usage.db_files.checked_mul(2).and_then(|v|v.checked_add(1024)).and_then(|v|v.checked_add(db_missing)).ok_or_else(invalid)?},
             Requirement{bytes:usage.app_bytes.checked_add(manifest_allocation(usage.manifest_bytes,have.app_block_bytes)?).and_then(|v|v.checked_add(app_pool)).ok_or_else(invalid)?,inodes:usage.app_files.checked_add(1024).and_then(|v|v.checked_add(app_missing)).ok_or_else(invalid)?})
        },
        Phase::P2{backup_bytes}=>(Requirement{bytes:floor.checked_sub(backup_bytes).ok_or_else(invalid)?,inodes:usage.db_files.checked_add(1024).ok_or_else(invalid)?},Requirement{bytes:usage.app_bytes,inodes:usage.app_files.checked_add(1024).ok_or_else(invalid)?}),
    };
    let devices=if have.shared_filesystem{
        let need=Requirement{bytes:db_need.bytes.checked_add(app_need.bytes).ok_or_else(invalid)?,inodes:db_need.inodes.checked_add(app_need.inodes).ok_or_else(invalid)?};
        let available=Available{bytes:have.db.bytes.min(have.app.bytes),inodes:match(have.db.inodes,have.app.inodes){(Some(a),Some(b))=>Some(a.min(b)),(known,None)|(None,known)=>known}};
        vec![(need,available)]
    }else{vec![(db_need,have.db),(app_need,have.app)]};
    let mut space=super::types::StatusSpace{required_bytes:0,available_bytes:0,inodes_required:0,inodes_available:0};let mut space_sufficient=true;
    for (need,available)in devices{
        space.required_bytes=space.required_bytes.checked_add(need.bytes).ok_or_else(invalid)?;space.available_bytes=space.available_bytes.checked_add(available.bytes).ok_or_else(invalid)?;
        space_sufficient&=available.bytes>=need.bytes;
        if let Some(inodes)=available.inodes{
            space.inodes_required=space.inodes_required.checked_add(need.inodes).ok_or_else(invalid)?;space.inodes_available=space.inodes_available.checked_add(inodes).ok_or_else(invalid)?;
            space_sufficient&=inodes>=need.inodes;
        }
    }Ok(SpaceAssessment{space_sufficient,space})
}
#[cfg(test)]
fn assess_with_probe(roots:&Roots,locks:&StorageLocks,usage:Usage,phase:Phase,pools:&super::reserves::ReserveReport,probe:impl Fn(BorrowedFd<'_>)->Result<(u64,Available,u64),StepError>)->Result<SpaceAssessment,StepError>{assess_available(usage,phase,pools,read_with(roots,locks,probe)?)}

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

#[cfg(test)]
mod assessment_tests{
    use super::*;
    use super::super::{Journal,Branch,journal::Candidate,reserves::ReserveReport};
    use std::{fs,os::unix::fs::PermissionsExt};
    fn usage()->Usage{Usage{db_bytes:1000,app_bytes:100,largest_index_bytes:200,manifest_bytes:1536,db_files:10,app_files:20}}
    fn full()->ReserveReport{ReserveReport{app_bytes:16*1024*1024,db_bytes:256*1024*1024,app_files:8,db_files:8}}
    fn roots()->(tempfile::TempDir,Roots,StorageLocks){let t=tempfile::tempdir().unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for p in [&roots.db,&roots.app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();(t,roots,locks)}
    fn injected(shared:bool,db:Available,app:Available,phase:Phase,pools:ReserveReport)->SpaceAssessment{
        let(_t,r,l)=roots();let result=assess_with_probe(&r,&l,usage(),phase,&pools,|fd|Ok(if fd.as_raw_fd()==l.d_fd().unwrap().as_raw_fd(){(1,db,1)}else{(if shared{1}else{2},app,1)})).unwrap();result
    }
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
        let mut u=usage();u.manifest_bytes=sized;
        let have=Available{bytes:10_000_000_000,inodes:Some(10000)};let got=assess_available(u,Phase::P0b,&full(),Checked{shared_filesystem:true,db:have,app:have,app_block_bytes:4096}).unwrap();
        let floor=crate::db_upgrade::engine::required_bytes(u.db_bytes,0,u.largest_index_bytes).unwrap();assert_eq!(got.space.required_bytes,floor+u.app_bytes+manifest_allocation(sized,4096).unwrap());
        let short=assess_available(u,Phase::P0b,&full(),Checked{shared_filesystem:false,db:Available{bytes:floor,..have},app:Available{bytes:u.app_bytes+entries.len()as u64*512,..have},app_block_bytes:4096}).unwrap();assert_eq!(short.require().unwrap_err().code,"insufficient_space");
    }
    #[test]
    fn exact_manifest_rounding_uses_the_a_device_and_is_not_readded_at_p2(){
        for (bytes,expected)in [(0,0),(1,4096),(4096,4096),(4097,8192)]{assert_eq!(manifest_allocation(bytes,4096).unwrap(),expected);}
        assert_eq!(manifest_allocation(1,0).unwrap_err().code,"unsupported_layout");
        let(_t,r,l)=roots();let u=usage();let have=Available{bytes:10_000_000_000,inodes:Some(10000)};
        let p0=assess_with_probe(&r,&l,u,Phase::P0b,&full(),|fd|Ok(if fd.as_raw_fd()==l.d_fd().unwrap().as_raw_fd(){(1,have,512)}else{(2,have,4096)})).unwrap();
        let floor=crate::db_upgrade::engine::required_bytes(u.db_bytes,0,u.largest_index_bytes).unwrap();assert_eq!(p0.space.required_bytes,floor+u.app_bytes+4096);
        let mut ignored=u;ignored.manifest_bytes=u64::MAX;
        let p2=assess_with_probe(&r,&l,ignored,Phase::P2{backup_bytes:u.db_bytes},&full(),|_|Ok((1,have,4096))).unwrap();assert_eq!(p2.space.required_bytes,floor-u.db_bytes+u.app_bytes);
    }
    #[test]
    fn both_modes_use_the_same_four_d_floor_and_frozen_manifest_allowance(){
        let u=usage();let floor=4*u.db_bytes+2*u.largest_index_bytes+1_073_741_824;
        assert_eq!(crate::db_upgrade::engine::required_bytes(u.db_bytes,0,u.largest_index_bytes).unwrap(),floor,"A.31 forbids a 3D floor in either mode");
        // assess has no mode switch: both callers receive this one frozen floor.
        let s=injected(true,Available{bytes:u64::MAX,inodes:Some(10000)},Available{bytes:u64::MAX,inodes:Some(10000)},Phase::P0b,full());assert_eq!(s.space.required_bytes,floor+u.app_bytes+u.manifest_bytes);assert_eq!(s.space.inodes_required,2*u.db_files+1024+u.app_files+1024);
    }
    #[test]
    fn coalesced_and_split_devices_enforce_each_budget_without_double_counting_free_space(){
        let u=usage();let d=crate::db_upgrade::engine::required_bytes(u.db_bytes,0,u.largest_index_bytes).unwrap();let a=u.app_bytes+u.manifest_bytes;
        let db=Available{bytes:d,inodes:Some(10000)};let app=Available{bytes:a,inodes:Some(10000)};let split=injected(false,db,app,Phase::P0b,full());assert!(split.space_sufficient);assert_eq!(split.space.available_bytes,d+a);
        let shared=injected(true,db,app,Phase::P0b,full());assert!(!shared.space_sufficient);assert_eq!(shared.space.available_bytes,a);assert_eq!(shared.require().unwrap_err().code,"insufficient_space");
        let s=injected(false,Available{bytes:d-1,..db},Available{bytes:10_000_000_000,..app},Phase::P0b,full());assert!(!s.space_sufficient,"abundant A must not hide a D shortfall");
    }
    #[test]
    fn missing_pools_add_only_missing_bytes_and_files_at_p0b(){
        let have=Available{bytes:10_000_000_000,inodes:Some(10000)};let base=injected(false,have,have,Phase::P0b,full());let absent=injected(false,have,have,Phase::P0b,ReserveReport::default());assert_eq!(absent.space.required_bytes-base.space.required_bytes,272*1024*1024);assert_eq!(absent.space.inodes_required-base.space.inodes_required,16);
        let partial=injected(false,have,have,Phase::P0b,ReserveReport{app_bytes:8*1024*1024,app_files:4,db_bytes:128*1024*1024,db_files:4});assert_eq!(partial.space.required_bytes-base.space.required_bytes,136*1024*1024);assert_eq!(partial.space.inodes_required-base.space.inodes_required,8);
    }
    #[test]
    fn p2_subtracts_verified_backup_and_does_not_readd_manifest_or_reserves(){
        let u=usage();let d=crate::db_upgrade::engine::required_bytes(u.db_bytes,0,u.largest_index_bytes).unwrap()-u.db_bytes;
        let have=Available{bytes:d+u.app_bytes,inodes:Some(10000)};let s=injected(true,have,have,Phase::P2{backup_bytes:u.db_bytes},ReserveReport::default());assert!(s.space_sufficient);assert_eq!(s.space.required_bytes,d+u.app_bytes);assert_eq!(s.space.inodes_required,u.db_files+1024+u.app_files+1024);
        assert!(!injected(true,Available{bytes:d+u.app_bytes-1,..have},have,Phase::P2{backup_bytes:u.db_bytes},full()).space_sufficient);
    }
    #[test]
    fn unknown_inode_devices_are_omitted_but_byte_checks_still_apply(){
        let unknown=Available{bytes:10_000_000_000,inodes:None};let known=Available{bytes:10_000_000_000,inodes:Some(9000)};
        let s=injected(false,unknown,known,Phase::P0b,full());assert!(s.space_sufficient);assert_eq!(s.space.inodes_required,usage().app_files+1024);assert_eq!(s.space.inodes_available,9000);
        let s=injected(true,unknown,unknown,Phase::P0b,full());assert_eq!((s.space.inodes_required,s.space.inodes_available),(0,0));assert!(s.space_sufficient);
        let s=injected(false,Available{bytes:0,..unknown},known,Phase::P0b,full());assert!(!s.space_sufficient);assert_eq!(s.require().unwrap_err().code,"insufficient_space");
        let s=injected(false,unknown,Available{inodes:Some(0),..known},Phase::P0b,full());assert!(!s.space_sufficient);assert_eq!(s.require().unwrap_err().code,"insufficient_space");
    }
    #[test]
    fn a_shortfall_has_status_and_writes_no_backup_byte(){
        let(_t,r,l)=roots();let result=assess_with_probe(&r,&l,usage(),Phase::P0b,&full(),|_|Ok((1,Available{bytes:0,inodes:Some(0)},1))).unwrap();let backup=r.db.join("must-not-exist");
        let attempt=(||{result.require()?;fs::write(&backup,b"backup bytes").unwrap();Ok::<(),StepError>(())})();assert_eq!(attempt.unwrap_err().code,"insufficient_space");assert!(result.space.required_bytes>0);assert!(!backup.exists());assert!(fs::read_dir(&r.db).unwrap().next().is_none());assert!(fs::read_dir(&r.app).unwrap().next().is_none());
    }
    #[test]
    fn arithmetic_and_probe_failures_never_wrap_or_create_files(){
        let(_t,r,l)=roots();let have=Available{bytes:10_000_000_000,inodes:Some(10000)};
        for bad in ["db-bytes","app-bytes","db-files","app-files","manifest","backup","pool"]{
            let mut u=usage();let mut phase=Phase::P0b;let mut pool=full();match bad{"db-bytes"=>u.db_bytes=u64::MAX,"app-bytes"=>u.app_bytes=u64::MAX,"db-files"=>u.db_files=u64::MAX,"app-files"=>u.app_files=u64::MAX,"manifest"=>u.manifest_bytes=super::super::backup_format::MAX_FILE+1,"backup"=>phase=Phase::P2{backup_bytes:u64::MAX},_=>pool.db_bytes=u64::MAX};
            assert!(assess_with_probe(&r,&l,u,phase,&pool,|_|Ok((1,have,1))).is_err(),"{bad}");
        }
        assert_eq!(assess_with_probe(&r,&l,usage(),Phase::P0b,&full(),|_|Err(error("io_error"))).unwrap_err().code,"io_error");
        assert!(fs::read_dir(&r.db).unwrap().next().is_none());assert!(fs::read_dir(&r.app).unwrap().next().is_none());
    }
    #[test]
    #[ignore="owned 8MiB tmpfs; NWUI_TEST_SPACE_FILL=1"]
    fn native_tmpfs_fill_is_observed_by_assess_without_backup_or_journal_mutation(){
        if std::env::var("NWUI_TEST_SPACE_FILL").as_deref()!=Ok("1"){eprintln!("NWUI_TEST_SPACE_FILL not enabled");return;}
        use std::io::Write;
        let t=tempfile::tempdir_in("/nwui-space-native").unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for p in [&roots.db,&roots.app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();
        let j=Journal::begin(&roots.app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();let frame=serde_json::to_vec(j.load()).unwrap();
        let before=assess(&roots,&locks,&j,usage(),Phase::P0b).unwrap();let mut ballast=fs::File::create(roots.db.join("owned-ballast")).unwrap();let mut left=before.space.available_bytes.saturating_sub(8192);let chunk=[0u8;4096];
        while left>0{let n=left.min(chunk.len()as u64)as usize;match ballast.write(&chunk[..n]){Ok(0)=>panic!("zero ballast write"),Ok(written)=>left-=written as u64,Err(e)if e.raw_os_error()==Some(libc::ENOSPC)=>break,Err(e)=>panic!("{e}")}}ballast.sync_all().unwrap();
        let tree=(crate::db_upgrade::files::snapshot(&roots.db).unwrap(),crate::db_upgrade::files::snapshot(&roots.app).unwrap());let after=assess(&roots,&locks,&j,usage(),Phase::P0b).unwrap();assert!(after.space.available_bytes<before.space.available_bytes);assert_eq!(after.require().unwrap_err().code,"insufficient_space");assert_eq!(serde_json::to_vec(j.load()).unwrap(),frame);assert!(!roots.db.join("backup").exists());assert_eq!(tree,(crate::db_upgrade::files::snapshot(&roots.db).unwrap(),crate::db_upgrade::files::snapshot(&roots.app).unwrap()));
    }

}
