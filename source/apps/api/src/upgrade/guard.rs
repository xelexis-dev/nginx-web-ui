//! R5 K9: block an old image before the selector commit; A owns all authority.
use super::{Branch,Journal,Step,StepState,StepError,StorageLocks,backup_format::{Kind,SourceLocation},selector::{Origin,Selector}};
use crate::db_upgrade::{layout::Roots,postgres};
use std::{fs::File,io::{Read,Seek,SeekFrom,Write},os::{fd::{AsRawFd,FromRawFd},unix::fs::{MetadataExt,PermissionsExt}},path::Path};
const VERSION:&std::ffi::CStr=c"PG_VERSION";
const SAVED:&std::ffi::CStr=c"PG_VERSION.nwui-pre17";
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
fn marker(parent:&File,bytes:&[u8])->Result<(),StepError> {
    let mut fd=unsafe{libc::openat(parent.as_raw_fd(),MARKER.as_ptr(),libc::O_RDWR|libc::O_CREAT|libc::O_EXCL|libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC,0o600)};
    if fd<0 && std::io::Error::last_os_error().raw_os_error()==Some(libc::EEXIST){fd=unsafe{libc::openat(parent.as_raw_fd(),MARKER.as_ptr(),libc::O_RDWR|libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC)};}
    if fd<0{return Err(io_error(std::io::Error::last_os_error()));}let mut file=unsafe{File::from_raw_fd(fd)};let m=file.metadata().map_err(io_error)?;let s=stat(parent,MARKER)?.ok_or_else(mismatch)?;
    if !regular(&s) || m.dev()!=s.st_dev || m.ino()!=s.st_ino || m.len()>bytes.len()as u64{return Err(mismatch());}
    let mut before=Vec::new();(&mut file).take(bytes.len()as u64+1).read_to_end(&mut before).map_err(io_error)?;if !bytes.starts_with(&before){return Err(mismatch());}
    file.set_permissions(std::fs::Permissions::from_mode(0o600)).map_err(io_error)?;file.seek(SeekFrom::Start(0)).map_err(io_error)?;file.write_all(bytes).and_then(|_|file.sync_all()).map_err(io_error)?;parent.sync_all().map_err(io_error)
}
#[cfg(test)]fn checkpoint(point:&str){if std::env::var("NWUI_OLD_GUARD_KILL_POINT").as_deref()==Ok(point){unsafe{libc::kill(libc::getpid(),libc::SIGKILL);}}}
#[cfg(not(test))]fn checkpoint(_point:&str){}
fn content(backup:&str,generation:&str)->String {
    format!("NWUI: PostgreSQL 17 upgrade. Do not start an older image on these volumes.\nBackup: {backup}\nGeneration: {generation}\nStop the container and follow the documented PG15 restore procedure. Keep both volumes and the backup.\n")
}
/// Read-only reuse for P6/operator cleanup; parent is the caller's validated L capability.
pub(crate) fn verify_marker(parent:&File,backup:&str,generation:&str)->Result<(),StepError> {
    if !super::types::canonical_generation(backup) || !super::types::canonical_generation(generation){return Err(mismatch());}
    let m=parent.metadata().map_err(io_error)?;
    if !m.is_dir() || m.mode()&0o022!=0 || !(m.uid()==0 || postgres::postgres_ids().is_ok_and(|(uid,_)|uid==m.uid())){return Err(mismatch());}
    let expected=content(backup,generation);let (_,bytes)=read(parent,MARKER,expected.len()as u64)?;
    if bytes!=expected.as_bytes(){return Err(mismatch());}Ok(())
}
/// Only after manifest verification of every restored original entry; never on a committed selector.
pub(crate) fn revert(roots:&Roots,locks:&StorageLocks,j:&mut Journal)->Result<(),StepError> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app) || j.load().branch!=Branch::Upgrade {return Err(mismatch());}
    super::replacement::precommit(&roots.app,j)?;
    for path in [roots.db.join("17/data")] {
        match std::fs::symlink_metadata(path){Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},_=>return Err(mismatch())}
    }
    super::backup::verify_restored(roots,j)?;
    let manifest=super::backup::open_verified_manifest(roots,j)?;
    let l=if manifest.header.source_location==SourceLocation::Root{roots.db.clone()}else{roots.db.join("pgdata")};let parent=super::target::directory(&l)?;
    super::source::require_idle(roots,locks,&l)?;
    if stat(&parent,SAVED)?.is_some(){return Err(mismatch());}
    let intent=j.load().records.iter().rev().find(|r|r.step==Step::P5Guard && r.state==StepState::Intent);
    let Some(intent)=intent else {if stat(&parent,MARKER)?.is_some(){return Err(mismatch());}return Ok(())};
    let data=intent.data.as_ref().ok_or_else(mismatch)?;let generation=data.get("generation").and_then(|v|v.as_str()).filter(|g|super::types::canonical_generation(g)).ok_or_else(mismatch)?;
    if data.get("backupId").and_then(|v|v.as_str())!=Some(j.load().attempt_id.as_str()){return Err(mismatch());}
    let completed=j.load().records.iter().any(|r|r.step==Step::P5Guard && r.state==StepState::Done && r.data.as_ref().and_then(|v|v.get("guardEvent")).and_then(|v|v.as_str())==Some("reverted"));
    if stat(&parent,MARKER)?.is_some() {
        if completed{return Err(mismatch());}
        let expected=content(&j.load().attempt_id,generation);let (_,bytes)=read(&parent,MARKER,expected.len()as u64)?;
        if bytes!=expected.as_bytes(){return Err(mismatch());}
        if unsafe{libc::unlinkat(parent.as_raw_fd(),MARKER.as_ptr(),0)}!=0{return Err(io_error(std::io::Error::last_os_error()));}
    }
    checkpoint("revert-unlink");parent.sync_all().map_err(io_error)?;checkpoint("revert-fsync");
    if !completed {let event=serde_json::json!({"guardEvent":"reverted","backupId":j.load().attempt_id,"generation":generation});j.record(Step::P5Guard,StepState::Done,Some(event))?;}
    Ok(())
}
pub(crate) fn install(roots:&Roots,locks:&StorageLocks,j:&mut Journal,selected:&Selector)->Result<(),StepError> {
    install_with(roots,locks,j,selected,&|path|{let control=postgres::read_control(17,path).map_err(|e|StepError{code:e.code,message:e.message,detail:None})?;control.require_cold().map_err(|e|StepError{code:e.code,message:e.message,detail:None})?;Ok(control.system_identifier)})
}
fn install_with(roots:&Roots,locks:&StorageLocks,j:&mut Journal,selected:&Selector,read_target:&dyn Fn(&Path)->Result<String,StepError>)->Result<(),StepError> {
    selected.validate()?;
    if j.load().records.iter().any(|r|r.step==Step::P5Guard && r.data.as_ref().and_then(|v|v.get("guardEvent")).and_then(|v|v.as_str())==Some("reverted")){return Err(mismatch());}
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app) || j.load().branch!=Branch::Upgrade
        || selected.origin!=Origin::Auto || selected.backup_id.as_deref()!=Some(j.load().attempt_id.as_str()) || !j.load().records.iter().any(|r|r.step==Step::P4Target && r.state==StepState::Done && r.data.is_none()) {return Err(mismatch());}
    super::replacement::precommit(&roots.app,j)?;
    if read_target(&roots.db.join("17/data"))?!=selected.system_identifier{return Err(mismatch());}
    let mut manifest=super::backup::open_verified_manifest(roots,j)?;let l=if manifest.header.source_location==SourceLocation::Root{roots.db.clone()}else{roots.db.join("pgdata")};let parent=super::target::directory(&l)?;
    if stat(&parent,c"postmaster.pid")?.is_some(){return Err(StepError{code:"storage_busy",message:"이전 PostgreSQL이 실행 중일 수 있어 구동 차단을 보류했습니다.",detail:None});}
    let mut version_hash=None;manifest.for_each(|e|{if e.path=="PG_VERSION" {if e.kind!=Kind::File || e.bytes>16{return Err(mismatch());}version_hash=e.sha256.clone();}Ok(())})?;
    let expected=version_hash.ok_or_else(mismatch)?;let data=serde_json::json!({"generation":selected.generation,"backupId":j.load().attempt_id,"systemIdentifier":selected.system_identifier,"versionSha256":expected});
    let intent=j.load().records.iter().rev().find(|r|r.step==Step::P5Guard && r.state==StepState::Intent);
    if intent.is_some_and(|r|r.data.as_ref()!=Some(&data)){return Err(mismatch());}
    let src=stat(&parent,VERSION)?.is_some();let dst=stat(&parent,SAVED)?.is_some();if src==dst || intent.is_none() && (!src || stat(&parent,MARKER)?.is_some()){return Err(mismatch());}
    let completed=j.load().records.iter().rev().find(|r|r.step==Step::P5Guard).is_some_and(|r|r.state==StepState::Done && r.data.is_none());if completed && src{return Err(mismatch());}
    let (_,bytes)=read(&parent,if src{VERSION}else{SAVED},16)?;if super::selector::digest(&bytes)!=expected || std::str::from_utf8(&bytes).ok().map(str::trim)!=Some("15"){return Err(mismatch());}
    if intent.is_none(){j.record(Step::P5Guard,StepState::Intent,Some(data))?;}checkpoint("intent");
    let content=content(&j.load().attempt_id,selected.generation.as_deref().ok_or_else(mismatch)?);
    marker(&parent,content.as_bytes())?;checkpoint("marker");
    if src {
        if stat(&parent,SAVED)?.is_some(){return Err(mismatch());}
        if unsafe{libc::renameat(parent.as_raw_fd(),VERSION.as_ptr(),parent.as_raw_fd(),SAVED.as_ptr())}!=0{return Err(io_error(std::io::Error::last_os_error()));}
    }
    checkpoint("rename");parent.sync_all().map_err(io_error)?;checkpoint("dir-fsync");
    if stat(&parent,VERSION)?.is_some() || stat(&parent,SAVED)?.is_none(){return Err(mismatch());}
    if !completed {j.record(Step::P5Guard,StepState::Done,None)?;}checkpoint("done");Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;use std::{fs,path::PathBuf,os::unix::process::ExitStatusExt};use crate::db_upgrade::control::{Control,COMPATIBILITY_FIELDS};
    #[test]
    fn w1_guard_read_only_marker_verification_reuses_exact_bytes_and_rejects_tampering() {
        let t=tempfile::tempdir().unwrap();let parent=super::super::target::directory(t.path()).unwrap();let backup=uuid::Uuid::new_v4().to_string();let generation=uuid::Uuid::new_v4().to_string();
        marker(&parent,content(&backup,&generation).as_bytes()).unwrap();let before=crate::db_upgrade::files::snapshot(t.path()).unwrap();verify_marker(&parent,&backup,&generation).unwrap();assert_eq!(crate::db_upgrade::files::snapshot(t.path()).unwrap(),before);
        assert!(verify_marker(&parent,&backup,&uuid::Uuid::new_v4().to_string()).is_err());assert!(verify_marker(&parent,"../escape",&generation).is_err());assert_eq!(crate::db_upgrade::files::snapshot(t.path()).unwrap(),before);
        fs::write(t.path().join("UPGRADED-TO-PG17.txt"),b"foreign").unwrap();assert!(verify_marker(&parent,&backup,&generation).is_err());assert_eq!(fs::read(t.path().join("UPGRADED-TO-PG17.txt")).unwrap(),b"foreign");
    }
    fn fixture(location:SourceLocation)->(tempfile::TempDir,Roots,StorageLocks,Journal,Selector,PathBuf) {
        let t=tempfile::tempdir().unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
        for p in [&roots.db,&roots.app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let l=if location==SourceLocation::Root{roots.db.clone()}else{let p=roots.db.join("pgdata");fs::create_dir(&p).unwrap();fs::set_permissions(&p,fs::Permissions::from_mode(0o700)).unwrap();p};fs::write(l.join("PG_VERSION"),b"15\n").unwrap();fs::create_dir(l.join("global")).unwrap();fs::write(l.join("global/pg_control"),b"cold control fixture").unwrap();
        let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j={let fixture_journal=Journal::begin(&roots.app,&locks,Branch::Upgrade,super::super::journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};
        let text=format!("Database system identifier: 123456\nDatabase cluster state: shut down\n{}",COMPATIBILITY_FIELDS.iter().map(|f|format!("{f}: 1\n")).collect::<String>());let control=Control::parse(&text,"").unwrap();
        let names=std::collections::BTreeSet::from(["PG_VERSION".into(),"global".into()]);let backup=super::super::backup::prepare(&roots,&locks,&j.load().attempt_id,location,&names,&control,"2026-10-28T00:00:00Z").unwrap();super::super::backup::create_verified(&roots,&locks,&mut j,&backup).unwrap();j.record(Step::P4Target,StepState::Done,None).unwrap();
        let selected=Selector{version:1,active_major:17,generation:Some(uuid::Uuid::new_v4().to_string()),system_identifier:"987654".into(),origin:Origin::Auto,backup_id:Some(j.load().attempt_id.clone()),profile_id:Some("legacy-28".into()),profile_sha256:Some("1".repeat(64)),manifest_sha256:Some("2".repeat(64)),operator_reason:None};(t,roots,locks,j,selected,l)
    }
    fn install_test(roots:&Roots,locks:&StorageLocks,j:&mut Journal,selected:&Selector)->Result<(),StepError>{install_with(roots,locks,j,selected,&|path|{assert_eq!(path,roots.db.join("17/data"));Ok("987654".into())})}
    #[test]
    fn w1_old_guard_reverts_only_after_full_restore_and_preserves_foreign_or_committed_material() {
        for location in [SourceLocation::Root,SourceLocation::Pgdata] {
            let (_t,roots,locks,mut j,selected,l)=fixture(location);install_test(&roots,&locks,&mut j,&selected).unwrap();
            assert!(revert(&roots,&locks,&mut j).is_err());assert!(l.join("UPGRADED-TO-PG17.txt").exists());
            super::super::restore::restore_entries(&roots,&locks,&mut j).unwrap();
            let marker=l.join("UPGRADED-TO-PG17.txt");let original=fs::read(&marker).unwrap();fs::write(&marker,b"foreign").unwrap();
            assert!(revert(&roots,&locks,&mut j).is_err());assert_eq!(fs::read(&marker).unwrap(),b"foreign");fs::write(&marker,&original).unwrap();
            let path=roots.app.join(".nwui-auto/active-v1.json");fs::write(&path,b"committed").unwrap();assert!(revert(&roots,&locks,&mut j).is_err());assert_eq!(fs::read(&marker).unwrap(),original);fs::remove_file(path).unwrap();
            revert(&roots,&locks,&mut j).unwrap();let count=j.load().records.len();revert(&roots,&locks,&mut j).unwrap();assert_eq!(count,j.load().records.len());
            assert!(!marker.exists());assert_eq!(fs::read(l.join("PG_VERSION")).unwrap(),b"15\n");assert!(install_test(&roots,&locks,&mut j,&selected).is_err());
            assert_eq!(j.load().outcome,super::super::Outcome::Upgrading);
        }
    }
    #[test]
    fn w1_old_guard_is_idempotent_and_never_rewrites_the_verified_version_bytes() {
        for location in [SourceLocation::Root,SourceLocation::Pgdata]{let (_t,roots,locks,mut j,selected,l)=fixture(location);let version=fs::metadata(l.join("PG_VERSION")).unwrap().ino();install_test(&roots,&locks,&mut j,&selected).unwrap();
            assert!(!l.join("PG_VERSION").exists());assert_eq!(fs::read(l.join("PG_VERSION.nwui-pre17")).unwrap(),b"15\n");assert_eq!(fs::metadata(l.join("PG_VERSION.nwui-pre17")).unwrap().ino(),version);assert!(l.join("UPGRADED-TO-PG17.txt").is_file());
            let count=j.load().records.len();install_test(&roots,&locks,&mut j,&selected).unwrap();assert_eq!(j.load().records.len(),count);assert!(!roots.app.join(".nwui-auto/active-v1.json").exists());}
    }
    #[test]
    fn w1_old_guard_kill_child(){let Some(root)=std::env::var_os("NWUI_OLD_GUARD_TEST_ROOT")else{return};let roots=Roots{db:PathBuf::from(&root).join("db"),app:PathBuf::from(root).join("app"),architecture:"arm64".into()};let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j=Journal::open(&roots.app,&locks).unwrap();let selected:Selector=serde_json::from_str(&std::env::var("NWUI_OLD_GUARD_TEST_SELECTOR").unwrap()).unwrap();if std::env::var_os("NWUI_OLD_GUARD_REVERT").is_some(){revert(&roots,&locks,&mut j).unwrap();}else{install_test(&roots,&locks,&mut j,&selected).unwrap();}}
    #[test]
    fn w1_old_guard_real_kills_resume_reversal_after_unlink_and_parent_fsync() {
        for location in [SourceLocation::Root,SourceLocation::Pgdata] {for point in ["revert-unlink","revert-fsync"] {
            let (t,roots,locks,mut j,selected,l)=fixture(location);install_test(&roots,&locks,&mut j,&selected).unwrap();super::super::restore::restore_entries(&roots,&locks,&mut j).unwrap();drop(j);drop(locks);
            let result=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::guard::tests::w1_old_guard_kill_child","--nocapture"]).env("NWUI_OLD_GUARD_TEST_ROOT",t.path()).env("NWUI_OLD_GUARD_TEST_SELECTOR",serde_json::to_string(&selected).unwrap()).env("NWUI_OLD_GUARD_REVERT","1").env("NWUI_OLD_GUARD_KILL_POINT",point).status().unwrap();assert_eq!(result.signal(),Some(libc::SIGKILL));
            let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j=Journal::open(&roots.app,&locks).unwrap();revert(&roots,&locks,&mut j).unwrap();assert!(!l.join("UPGRADED-TO-PG17.txt").exists());assert_eq!(fs::read(l.join("PG_VERSION")).unwrap(),b"15\n");super::super::backup::verify_restored(&roots,&j).unwrap();
        }}
    }
    #[test]
    fn w1_old_guard_real_kills_resume_marker_and_version_rename_in_both_layouts(){
        for location in [SourceLocation::Root,SourceLocation::Pgdata]{for point in ["intent","marker","rename","dir-fsync","done"]{
            let (t,roots,locks,j,selected,l)=fixture(location);drop(j);drop(locks);
            let result=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::guard::tests::w1_old_guard_kill_child","--nocapture"]).env("NWUI_OLD_GUARD_TEST_ROOT",t.path()).env("NWUI_OLD_GUARD_TEST_SELECTOR",serde_json::to_string(&selected).unwrap()).env("NWUI_OLD_GUARD_KILL_POINT",point).status().unwrap();assert_eq!(result.signal(),Some(libc::SIGKILL));
            let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j=Journal::open(&roots.app,&locks).unwrap();install_test(&roots,&locks,&mut j,&selected).unwrap();assert!(!l.join("PG_VERSION").exists());assert_eq!(fs::read(l.join("PG_VERSION.nwui-pre17")).unwrap(),b"15\n");assert_eq!(j.load().records.iter().rev().find(|r|r.step==Step::P5Guard).unwrap().state,StepState::Done);
        }}
    }
    #[test]
    fn w1_old_guard_never_adopts_a_foreign_marker_ambiguous_version_or_live_pid(){for case in ["marker","both","neither","version","pid","selector"]{
        let (_t,roots,locks,mut j,selected,l)=fixture(SourceLocation::Root);match case{"marker"=>fs::write(l.join("UPGRADED-TO-PG17.txt"),b"foreign").unwrap(),"both"=>fs::write(l.join("PG_VERSION.nwui-pre17"),b"15\n").unwrap(),"neither"=>fs::remove_file(l.join("PG_VERSION")).unwrap(),"version"=>fs::write(l.join("PG_VERSION"),b"17\n").unwrap(),"pid"=>fs::write(l.join("postmaster.pid"),b"1\n").unwrap(),"selector"=>fs::write(roots.app.join(".nwui-auto/active-v1.json"),b"foreign").unwrap(),_=>unreachable!()}
        let before=crate::db_upgrade::files::snapshot(&roots.db).unwrap();let count=j.load().records.len();assert!(install_test(&roots,&locks,&mut j,&selected).is_err(),"{case}");assert_eq!(crate::db_upgrade::files::snapshot(&roots.db).unwrap(),before);assert_eq!(j.load().records.len(),count);
    }}
}
