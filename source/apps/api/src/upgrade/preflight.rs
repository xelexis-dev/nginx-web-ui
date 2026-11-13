//! P0b reads only port settings; AUTH_SECRET and the fresh DB password do not exist yet.
use super::StepError;

#[derive(Debug,PartialEq,Eq)]
pub(crate) struct StartupPorts {pub api:u16,pub web:u16,pub db:u16}
fn invalid(detail:impl ToString)->StepError {
    StepError {code:"api_port_conflict",message:"API 포트 환경 설정을 확인하세요.",detail:Some(detail.to_string().chars().take(512).collect())}
}
/// The runner passes these values to N2 preflight_ports, which owns listener-conflict checks.
pub(crate) fn startup_ports()->Result<StartupPorts,StepError> {
    checked_ports(crate::nonroot::startup_value)
}
fn checked_ports(get:impl Fn(&str)->Option<String>)->Result<StartupPorts,StepError> {
    let ports=read_ports(get)?;
    crate::runtime_boot::preflight_ports(ports.api,ports.web,ports.db)?;
    Ok(ports)
}
fn read_ports(get:impl Fn(&str)->Option<String>)->Result<StartupPorts,StepError> {
    let port=|name,default:&str| get(name).as_deref().unwrap_or(default).parse::<u16>().map_err(invalid);
    let api=port("API_PORT","3001")?;let web=port("PORT","3000")?;
    if web==0 {return Err(invalid("PORT must be nonzero"));}
    if api==0 || api==web {return Err(invalid("API_PORT must be nonzero and separate from the web port"));}
    let db=match get("DATABASE_URL") {
        Some(value)=>url::Url::parse(&value).map_err(invalid)?.port().unwrap_or(5432),
        None=>5432,
    };
    Ok(StartupPorts {api,web,db})
}

/// File-only legacy dispatch, before backup/reserve allocation or any private PG15 start.
pub(crate) struct Legacy15 {pub location:super::backup_format::SourceLocation,pub control:crate::db_upgrade::control::Control,pub original_names:std::collections::BTreeSet<String>}
/// P0b's exact metadata floor, before reserve allocation or a backup byte is written.
pub(crate) fn assess_backup(roots:&crate::db_upgrade::layout::Roots,locks:&super::StorageLocks,j:&super::Journal,legacy:&Legacy15)->Result<(super::backup::PreparedBackup,super::space::SpaceAssessment),StepError>{
    let prepared=super::backup::prepare(roots,locks,&j.load().attempt_id,legacy.location,&legacy.original_names,&legacy.control,&chrono::Utc::now().to_rfc3339())?;
    let(app_bytes,app_files)=super::backup::app_usage(roots,locks)?;
    let usage=super::space::Usage{db_bytes:prepared.source_bytes,app_bytes,largest_index_bytes:0,manifest_bytes:prepared.manifest_bytes,db_files:prepared.entries.checked_add(1).ok_or_else(super::backup_format::too_large)?,app_files};
    let assessment=super::space::assess(roots,locks,j,usage,super::space::Phase::P0b)?;
    super::runner::publish_step(roots,j,super::Step::P0b,0,prepared.source_bytes,Some(assessment.space.clone()),false);Ok((prepared,assessment))
}
fn layout_error()->StepError {StepError{code:"unsupported_layout",message:"이전 데이터의 경로 또는 파일 구성을 확인하지 못했습니다. 원본을 변경하지 않았습니다.",detail:None}}
fn present(path:&std::path::Path)->Result<bool,StepError> {match std::fs::symlink_metadata(path){Ok(_)=>Ok(true),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>Ok(false),Err(_)=>Err(layout_error())}}
fn entries(path:&std::path::Path)->Result<std::collections::BTreeSet<String>,StepError> {
    std::fs::read_dir(path).map_err(|_|layout_error())?.map(|e|e.map_err(|_|layout_error())?.file_name().into_string().map_err(|name|super::backup::invalid_name(&name))).collect()
}
fn empty_owned_directory(path:&std::path::Path,lost_found:bool)->Result<(),StepError> {
    use std::os::unix::fs::MetadataExt;
    crate::db_upgrade::files::real_path(path).map_err(|_|layout_error())?;let m=std::fs::symlink_metadata(path).map_err(|_|layout_error())?;
    if !m.is_dir() || m.mode()&0o022!=0 || !(m.uid()==0 || !lost_found && crate::db_upgrade::postgres::postgres_ids().is_ok_and(|(uid,_)|uid==m.uid())) || !entries(path)?.is_empty(){return Err(layout_error());}Ok(())
}
fn retained_discards(roots:&crate::db_upgrade::layout::Roots,journal:&super::Journal)->Result<(),StepError> {
    use std::os::unix::fs::MetadataExt;
    let expected=journal.load().previous.iter().filter_map(|p|p.discard_id.clone()).collect::<std::collections::BTreeSet<_>>();
    if expected.len()!=journal.load().previous.iter().filter(|p|p.discard_id.is_some()).count(){return Err(layout_error());}
    let parent=roots.db.join(".nwui-discard");if expected.is_empty(){if present(&parent)?{empty_owned_directory(&parent,false)?;}return Ok(());}
    let opened=super::target::directory(&parent)?;let parent_meta=opened.metadata().map_err(|_|layout_error())?;let db=std::fs::symlink_metadata(&roots.db).map_err(|_|layout_error())?;
    if parent_meta.mode()&0o7777!=0o700 || parent_meta.dev()!=db.dev() || entries(&parent)?!=expected{return Err(layout_error());}
    for id in expected {if !super::types::canonical_generation(&id){return Err(layout_error());}let child=super::target::directory(&parent.join(id))?;let m=child.metadata().map_err(|_|layout_error())?;if m.mode()&0o7777!=0o700 || m.dev()!=db.dev(){return Err(layout_error());}}
    Ok(())
}
pub(crate) fn legacy15(roots:&crate::db_upgrade::layout::Roots,locks:&super::StorageLocks)->Result<Option<Legacy15>,StepError> {
    legacy15_with(roots,locks,&|path|crate::db_upgrade::postgres::read_control(15,path).map_err(|e|StepError{code:e.code,message:e.message,detail:e.detail.map(str::to_owned)}))
}
pub(super) fn legacy15_with(roots:&crate::db_upgrade::layout::Roots,locks:&super::StorageLocks,read:&dyn Fn(&std::path::Path)->Result<crate::db_upgrade::control::Control,StepError>)->Result<Option<Legacy15>,StepError> {
    use std::{fs::OpenOptions,io::Read,os::unix::fs::{OpenOptionsExt,MetadataExt}};
    use crate::db_upgrade::{files,layout,mounts};use super::backup_format::SourceLocation;
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app){return Err(StepError{code:"storage_busy",message:"저장소 잠금이 유지되지 않았습니다.",detail:None});}
    // R4-1: keep the owner/mode detail (layout.rs) in the P0 refusal.
    layout::validate_roots(roots).map_err(|e|StepError{detail:e.detail.map(str::to_owned),..layout_error()})?;
    let at_root=present(&roots.db.join("PG_VERSION"))?;let nested=present(&roots.db.join("pgdata/PG_VERSION"))?;
    if at_root && nested{return Err(layout_error());}if !at_root && !nested{return Ok(None);}
    let location=if at_root{SourceLocation::Root}else{SourceLocation::Pgdata};let l=if at_root{roots.db.clone()}else{roots.db.join("pgdata")};
    files::real_path(&l).map_err(|_|layout_error())?;mounts::Table::read().and_then(|t|t.distinct(&[&l,&roots.app])).map_err(|_|layout_error())?;
    let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(l.join("PG_VERSION")).map_err(|_|layout_error())?;let m=file.metadata().map_err(|_|layout_error())?;
    if !m.is_file() || m.nlink()!=1 || m.len()>16 || m.mode()&0o022!=0 || !(m.uid()==0 || crate::db_upgrade::postgres::postgres_ids().is_ok_and(|(uid,_)|uid==m.uid())){return Err(layout_error());}
    let mut version=String::new();file.take(17).read_to_string(&mut version).map_err(|_|layout_error())?;if version.trim()!="15"{return Err(layout_error());}
    super::source::require_idle(roots,locks,&l)?;
    for path in [l.join("standby.signal"),l.join("recovery.signal"),roots.db.join("17"),roots.db.join(".nwui"),roots.app.join(".nwui")] {if present(&path)?{return Err(layout_error());}}
    let control_path=l.join("global/pg_control");files::real_path(&control_path).map_err(|_|layout_error())?;let checked=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(&control_path).map_err(|_|layout_error())?;let cm=checked.metadata().map_err(|_|layout_error())?;
    if !cm.is_file() || cm.nlink()!=1 || cm.len()==0 || cm.len()>65536 || cm.mode()&0o022!=0 || !(cm.uid()==0 || crate::db_upgrade::postgres::postgres_ids().is_ok_and(|(uid,_)|uid==cm.uid())){return Err(layout_error());}
    let journal=if present(&roots.app.join(".nwui-auto"))? {
        if present(&roots.app.join(".nwui-auto/journal-v1.json"))?{Some(super::Journal::open(&roots.app,locks)?)}else{super::journal::prepare_bootstrap(&roots.app,locks)?;None}
    }else{None};
    let selected=if present(&roots.app.join(".nwui-auto/active-v1.json"))? {
        let j=journal.as_ref().ok_or_else(layout_error)?;let selected=super::selector::read_selector(&roots.app)?;
        super::replacement::restored(j,&selected,&selected.system_identifier)?;Some(selected)
    }else{None};let restored=selected.is_some();
    if let Some(j)=&journal {if restored{retained_discards(roots,j)?;}else if j.load().previous.iter().any(|p|p.discard_id.is_some()){return Err(layout_error());}}
    let has_journal=journal.is_some();let mut names=entries(&l)?;
    for name in entries(&roots.db)? {
        if name=="lost+found" {empty_owned_directory(&roots.db.join(&name),true)?;if at_root{names.remove(&name);}continue;}
        if matches!(name.as_str(),".nwui-auto-work"|".nwui-pg15-backup"|".nwui-discard") && has_journal {
            if name==".nwui-auto-work" {
                let parent=roots.db.join(&name);super::target::directory(&parent)?;
                if entries(&parent)?.iter().any(|n|!matches!(n.as_str(),"reserve"|"logs")){return Err(layout_error());}
                super::diagnostics::validate(&roots.db)?;
                super::reserves::report(roots,locks,journal.as_ref().ok_or_else(layout_error)?)?;
            }else if name!=".nwui-discard" || !restored{empty_owned_directory(&roots.db.join(&name),false)?;}
            if at_root{names.remove(&name);}continue;
        }
        if !at_root && name!="pgdata" || at_root && !super::restore::set_aside_allowed(&name,true){return Err(layout_error());}
    }
    if !names.contains("PG_VERSION") || !names.contains("global"){return Err(layout_error());}
    files::real_path(&l.join("global/pg_control")).map_err(|_|layout_error())?;
    let mut count=1;for name in &names {layout::regular_tree(&l.join(name),1,&mut count).map_err(|e|StepError{code:e.code,message:e.message,detail:e.detail.map(str::to_owned)})?;}
    layout::legacy_material(roots).map_err(|e|StepError{code:e.code,message:e.message,detail:e.detail.map(str::to_owned)})?;
    // Reject names, foreign artifacts and unsafe source leaves before invoking a root tool.
    let control=read(&l)?;super::source::control(&control)?;
    if let Some(selected)=selected{super::replacement::restored(journal.as_ref().ok_or_else(layout_error)?,&selected,&control.system_identifier)?;}
    Ok(Some(Legacy15{location,control,original_names:names}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn legacy_fixture(location:super::super::backup_format::SourceLocation)->(tempfile::TempDir,crate::db_upgrade::layout::Roots,super::super::StorageLocks) {
        use std::{fs,os::unix::fs::PermissionsExt};let t=tempfile::tempdir().unwrap();let roots=crate::db_upgrade::layout::Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
        for p in [&roots.db,&roots.app,&roots.app.join("secrets"),&roots.app.join("tls")] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        for name in ["auth_secret","db_password"] {let p=roots.app.join("secrets").join(name);fs::write(&p,b"fixture-only").unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o644)).unwrap();}
        let key=rcgen::KeyPair::generate().unwrap();let cert=rcgen::CertificateParams::new(vec!["unconfigured.invalid".into()]).unwrap().self_signed(&key).unwrap();
        for (name,value,mode) in [("default.crt",cert.pem(),0o644),("default.key",key.serialize_pem(),0o600)] {let p=roots.app.join("tls").join(name);fs::write(&p,value).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(mode)).unwrap();}
        let l=if location==super::super::backup_format::SourceLocation::Root {roots.db.clone()}else{let p=roots.db.join("pgdata");fs::create_dir(&p).unwrap();fs::set_permissions(&p,fs::Permissions::from_mode(0o700)).unwrap();p};fs::create_dir(l.join("global")).unwrap();fs::write(l.join("global/pg_control"),b"fixture control").unwrap();fs::write(l.join("PG_VERSION"),b"15\n").unwrap();
        let locks=super::super::StorageLocks::acquire(&roots.db,&roots.app).unwrap();(t,roots,locks)
    }
    fn legacy_control()->crate::db_upgrade::control::Control {
        crate::db_upgrade::control::Control::parse(&format!("Database system identifier: 123\nDatabase cluster state: shut down\n{}",crate::db_upgrade::control::COMPATIBILITY_FIELDS.iter().map(|f|format!("{f}: 1\n")).collect::<String>()),"").unwrap()
    }
    #[test]
    fn w1_p0_legacy_layouts_are_read_only_and_keep_keyless_119_material() {
        use super::super::backup_format::SourceLocation;use crate::db_upgrade::files;
        for location in [SourceLocation::Root,SourceLocation::Pgdata] {let (_t,r,locks)=legacy_fixture(location);let before=(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap());
            let source=legacy15_with(&r,&locks,&|path|{assert_eq!(path,if location==SourceLocation::Root{r.db.clone()}else{r.db.join("pgdata")});Ok(legacy_control())}).unwrap().unwrap();assert_eq!(source.location,location);assert_eq!(source.control.system_identifier,"123");assert_eq!(source.original_names,std::collections::BTreeSet::from(["PG_VERSION".into(),"global".into()]));
            assert_eq!(before,(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap()));assert!(!r.app.join("secrets/data_key").exists());assert!(!r.app.join(".nwui-auto").exists());
        }
    }
    #[test]
    fn w1_legacy_environment_reuses_saved_119_secrets_without_touching_either_volume() {
        use std::{fs,os::unix::fs::{PermissionsExt,MetadataExt}};use crate::db_upgrade::{files,fresh};
        let (t,r,locks)=legacy_fixture(super::super::backup_format::SourceLocation::Root);let env=t.path().join("env");fs::create_dir(&env).unwrap();fs::set_permissions(&env,fs::Permissions::from_mode(0o700)).unwrap();let before=(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap());
        fresh::legacy_environment_locked(&locks,&r,&env).unwrap();assert_eq!(fs::read(env.join("AUTH_SECRET")).unwrap(),b"fixture-only");let url=url::Url::parse(&fs::read_to_string(env.join("DATABASE_URL")).unwrap()).unwrap();assert_eq!(url.path(),"/nginx_web_ui");assert_eq!(url.password(),Some("fixture-only"));assert_eq!(url.host_str(),Some("127.0.0.1"));
        for name in ["AUTH_SECRET","DATABASE_URL"]{assert_eq!(fs::metadata(env.join(name)).unwrap().mode()&0o7777,0o600);}
        assert_eq!(before,(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap()));assert!(!env.join("ADMIN_PASSWORD").exists());
    }
    #[test]
    fn w1_p0_non_utf8_source_name_uses_the_same_escaped_diagnostic_as_the_manifest() {
        use std::os::unix::ffi::OsStringExt;let (_t,r,locks)=legacy_fixture(super::super::backup_format::SourceLocation::Root);let name=std::ffi::OsString::from_vec(vec![0xff,b'x']);std::fs::write(r.db.join(&name),b"keep").unwrap();
        let error=legacy15_with(&r,&locks,&|_|panic!("non-UTF8 source reached pg_controldata")).err().unwrap();assert_eq!(error.code,"unsupported_layout");assert_eq!(error.detail,Some(format!("{name:?}")));assert_eq!(std::fs::read(r.db.join(name)).unwrap(),b"keep");
    }
    #[test]
    fn w1_p0_legacy_refuses_ambiguous_live_unsafe_or_foreign_layout_before_control_execution() {
        use std::fs;use super::super::backup_format::SourceLocation;
        for case in ["both","pid","version","wal-link","foreign","key"] {let (_t,r,locks)=legacy_fixture(SourceLocation::Root);
            match case {"both"=>{fs::create_dir(r.db.join("pgdata")).unwrap();fs::write(r.db.join("pgdata/PG_VERSION"),b"15\n").unwrap();},"pid"=>fs::write(r.db.join("postmaster.pid"),b"1\n").unwrap(),"version"=>fs::write(r.db.join("PG_VERSION"),b"17\n").unwrap(),"wal-link"=>std::os::unix::fs::symlink("/not-followed",r.db.join("pg_wal")).unwrap(),"foreign"=>fs::create_dir(r.db.join(".nwui-foreign")).unwrap(),_=>{fs::write(r.app.join("secrets/data_key"),b"invalid").unwrap();}}
            assert!(legacy15_with(&r,&locks,&|_|panic!("unsafe source reached pg_controldata")).is_err(),"{case}");assert!(!r.app.join(".nwui-auto").exists());assert!(!r.db.join(".nwui-auto-work").exists());
        }
    }
    #[test]
    fn w1_p0_admits_only_a_journal_bound_fixed_reserve_subset_before_control_execution() {
        use std::{fs,os::unix::fs::PermissionsExt};
        for valid in [true,false]{let (_t,r,l)=legacy_fixture(super::super::backup_format::SourceLocation::Root);
            let j=super::super::Journal::begin(&r.app,&l,super::super::Branch::Upgrade,super::super::journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();
            let parent=r.db.join(".nwui-auto-work");fs::create_dir(&parent).unwrap();fs::set_permissions(&parent,fs::Permissions::from_mode(0o700)).unwrap();let pool=parent.join("reserve");fs::create_dir(&pool).unwrap();fs::set_permissions(&pool,fs::Permissions::from_mode(0o700)).unwrap();
            let leaf=pool.join(if valid{"3"}else{"8"});fs::write(&leaf,b"partial").unwrap();fs::set_permissions(&leaf,fs::Permissions::from_mode(0o600)).unwrap();
            if valid{let found=legacy15_with(&r,&l,&|_|Ok(legacy_control())).unwrap().unwrap();assert!(!found.original_names.contains(".nwui-auto-work"));assert_eq!(super::super::reserves::report(&r,&l,&j).unwrap().db_files,1);}
            else{assert!(legacy15_with(&r,&l,&|_|panic!("foreign reserve reached control reader")).is_err());assert_eq!(fs::read(&leaf).unwrap(),b"partial");}
        }
    }
    #[test]
    fn w1_p0b_calls_the_shared_collision_core_before_runner_mutation() {
        checked_ports(|_|None).unwrap();
        for port in [80,443,8080,5432] {
            let error=checked_ports(|name|(name=="API_PORT").then(||port.to_string())).unwrap_err();
            assert_eq!(error.code,"api_port_conflict");
            assert_eq!(error.detail,crate::runtime_boot::preflight_ports(port,3000,5432).unwrap_err().detail);
        }
        assert!(checked_ports(|name|match name {"API_PORT"=>Some("5544".into()),"DATABASE_URL"=>Some("postgres://nginx@localhost:5544/fixture".into()),_=>None}).is_err());
    }
    #[test]
    fn w1_p0b_ports_need_no_auth_or_database_secrets() {
        let values=read_ports(|name|match name {"API_PORT"|"PORT"|"DATABASE_URL"=>None,_=>panic!("P0b read an unrelated setting")}).unwrap();
        assert_eq!(values,StartupPorts {api:3001,web:3000,db:5432});
    }
    #[test]
    fn w1_p0b_port_errors_match_runtime_config_before_mutation() {
        for (key,value) in [("API_PORT","0"),("API_PORT","3000"),("API_PORT",""),("API_PORT","bad"),("API_PORT","65536"),("API_PORT","-1"),("API_PORT"," 3001"),("PORT","0"),("PORT","bad"),("PORT","65536"),("PORT","3001")] {
            let get=|name:&str|if name==key {Some(value.to_owned())} else {None};
            let expected=crate::config::RuntimeConfig::read(get).err().expect("invalid ports must fail before missing secrets").to_string();
            let error=read_ports(get).unwrap_err();assert_eq!(error.code,"api_port_conflict");assert_eq!(error.detail.as_deref(),Some(expected.as_str()),"{key}");
        }
    }
    #[test]
    fn w1_p0b_database_url_is_parsed_only_when_set_and_never_logged() {
        let explicit=read_ports(|name|match name {"API_PORT"=>Some("+3011".into()),"PORT"=>Some("3010".into()),"DATABASE_URL"=>Some("postgres://nginx:fixture-secret@127.0.0.1:5440/fixture".into()),_=>None}).unwrap();
        assert_eq!(explicit,StartupPorts {api:3011,web:3010,db:5440});
        for value in ["", "not a URL with fixture-secret", "postgres://nginx:fixture-secret@[invalid"] {
            let error=read_ports(|name|(name=="DATABASE_URL").then(||value.to_owned())).unwrap_err();
            assert_eq!(error.code,"api_port_conflict");assert!(!format!("{error:?}").contains("fixture-secret"));
        }
    }

    #[test]
    fn w1_legacy_retained_discard_admission_requires_exact_previous_names_and_selector_identity() {
        use super::super::{Journal,Branch,selector::{Selector,Origin},journal::{Candidate,PreviousAttempt}};use crate::db_upgrade::files;use std::{fs,os::unix::fs::{PermissionsExt,symlink}};
        for case in ["valid","foreign","identity","missing"] {
            let (_t,r,locks)=legacy_fixture(super::super::backup_format::SourceLocation::Root);let candidate=Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)};let j=Journal::begin(&r.app,&locks,Branch::Upgrade,candidate.clone()).unwrap();let old=uuid::Uuid::new_v4().to_string();let g=uuid::Uuid::new_v4().to_string();let mut state=j.load().clone();state.previous.push(PreviousAttempt{attempt_id:old.clone(),outcome:super::super::Outcome::Restored,candidate,last_failure:None,generation:Some(g.clone()),discard_id:Some(g.clone()),ended_at:"2026-10-29T00:00:00Z".into()});files::atomic_json(&r.app.join(".nwui-auto/journal-v1.json"),&state).unwrap();
            let selected=Selector{version:1,active_major:15,generation:None,system_identifier:if case=="identity"{"999"}else{"123"}.into(),origin:Origin::Auto,backup_id:Some(old),profile_id:Some("legacy-28".into()),profile_sha256:Some("1".repeat(64)),manifest_sha256:None,operator_reason:Some("restore-pg15".into())};files::atomic_json(&r.app.join(".nwui-auto/active-v1.json"),&selected).unwrap();
            let parent=r.db.join(".nwui-discard");fs::create_dir(&parent).unwrap();fs::set_permissions(&parent,fs::Permissions::from_mode(0o700)).unwrap();if case!="missing"{let child=parent.join(&g);fs::create_dir(&child).unwrap();fs::set_permissions(&child,fs::Permissions::from_mode(0o700)).unwrap();symlink("/must-not-follow-retired-tree",child.join("foreign-link")).unwrap();}if case=="foreign"{fs::create_dir(parent.join(uuid::Uuid::new_v4().to_string())).unwrap();}
            if case=="valid"{let source=legacy15_with(&r,&locks,&|_|Ok(legacy_control())).unwrap().unwrap();assert!(!source.original_names.contains(".nwui-discard"));assert!(fs::symlink_metadata(parent.join(g).join("foreign-link")).unwrap().file_type().is_symlink());}else{assert!(legacy15_with(&r,&locks,&|_|Ok(legacy_control())).is_err(),"{case}");}
        }
    }
    #[test]
    fn w1_legacy_never_executes_a_control_reader_for_a_fifo() {
        let (_t,r,locks)=legacy_fixture(super::super::backup_format::SourceLocation::Root);let path=r.db.join("global/pg_control");std::fs::remove_file(&path).unwrap();let c=std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();assert_eq!(unsafe{libc::mkfifo(c.as_ptr(),0o600)},0);let called=std::cell::Cell::new(false);assert!(legacy15_with(&r,&locks,&|_|{called.set(true);Ok(legacy_control())}).is_err());assert!(!called.get());
    }

}
