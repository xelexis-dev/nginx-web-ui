//! Automatic conversion's fixed-path transport contract.
use super::{Journal,Step,StepState,StepError,StorageLocks,Branch};
use crate::db_upgrade::{layout::Roots,control::Control,cluster::Snapshot,migration::{self,UpgradeMode},postgres};
use std::{path::Path,time::Duration};
fn mismatch()->StepError {StepError{code:"verification_failed",message:"변환 준비 기록과 보관 사본을 확인하지 못했습니다. 원본과 작업 사본을 보존했습니다.",detail:None}}
fn map(e:crate::db_upgrade::error::Error)->StepError {StepError{code:e.code,message:e.message,detail:e.detail.map(str::to_owned)}}
fn present(path:&Path)->Result<bool,StepError>{match std::fs::symlink_metadata(path){Ok(_)=>Ok(true),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>Ok(false),Err(_)=>Err(mismatch())}}
/// P3 is attempted once per journal. A prior Intent resumes through R-restore, never re-links.
/// P2's schema rehearsal/check scratch must be retired before this real target is initialized.
/// Call on an awaited blocking thread under custody; no post-processing or DDL happens here.
pub(crate) fn run_locked(roots:&Roots,locks:&StorageLocks,j:&mut Journal,control:&Control,snapshot:&Snapshot)->Result<Control,StepError> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app) || j.load().branch!=Branch::Upgrade
        || j.load().records.iter().any(|r|r.step==Step::P3Convert) {return Err(mismatch());}
    super::replacement::precommit(&roots.app,j)?;
    let current=Journal::open(&roots.app,locks)?;if serde_json::to_value(current.load()).map_err(|_|mismatch())?!=serde_json::to_value(j.load()).map_err(|_|mismatch())?{return Err(mismatch());}
    let mode=super::profile::mode(j)?;
    let header=super::backup::verify_backup(roots,j)?;
    control.validate().and_then(|_|control.require_cold()).map_err(map)?;if header.system_identifier!=control.system_identifier{return Err(mismatch());}
    let old=match header.source_location{super::backup_format::SourceLocation::Root=>roots.db.clone(),super::backup_format::SourceLocation::Pgdata=>roots.db.join("pgdata")};let actual=postgres::read_control(15,&old).map_err(map)?;actual.require_cold().and_then(|_|control.compatible_with(&actual)).map_err(map)?;
    if actual.system_identifier!=control.system_identifier || present(&old.join("postmaster.pid"))? {return Err(mismatch());}
    let w=roots.db.join(".nwui-auto-work").join(&j.load().attempt_id);super::target::directory(&w)?;let data=w.join("data");if present(&data)? || present(&w.join("target"))? {return Err(mismatch());}
    let init=migration::init_target_command(&data,control,snapshot).map_err(map)?;let log_name=format!("convert-{}",uuid::Uuid::new_v4());let log=super::diagnostics::path(&roots.db,&j.load().attempt_id,&log_name)?;if present(&log)?{return Err(mismatch());}
    let mode_name=match mode{UpgradeMode::Link=>"link",UpgradeMode::Copy=>"copy"};
    j.record(Step::P3Convert,StepState::Intent,Some(serde_json::json!({"mayBeLinked":mode==UpgradeMode::Link,"mode":mode_name,"logName":log_name,"targetName":"data"})))?;
    let result:Result<Control,StepError>=(|| {
        super::diagnostics::create(roots,locks,j,&log_name,false)?;
        postgres::directory(&data).map_err(map)?;
        postgres::capture(init,&log,Duration::from_secs(300)).map_err(map)?;
        migration::upgrade_auto_at(roots,locks,j,&log_name,mode,false).map_err(map)?;
        let converted=postgres::read_control(17,&data).map_err(map)?;converted.validate().and_then(|_|converted.require_cold()).and_then(|_|control.compatible_with(&converted)).map_err(map)?;
        j.record(Step::P3Convert,StepState::Done,Some(serde_json::json!({"systemIdentifier":converted.system_identifier,"mode":mode_name,"logName":log_name})))?;Ok(converted)
    })();
    if let Err(error)=&result{j.record_retry_failure(Step::P3Convert,error.code,chrono::Utc::now())?;}result
}
#[cfg(test)]
mod tests {
    use crate::{upgrade::{Branch,Journal,Step,StepState,StorageLocks,Outcome,journal::Candidate},db_upgrade::{files,layout::Roots,migration::{self,UpgradeMode}}};
    use std::{fs,os::unix::fs::PermissionsExt};
    #[test]
    fn w1_auto_conversion_rejects_unowned_log_names_and_missing_or_stale_authority_without_tool_files() {
        for case in ["no-backup","log-traversal","log-alias","wrong-phase","selector","stale"] {
            let t=tempfile::tempdir().unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for p in [&roots.db,&roots.app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
            let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j={let fixture_journal=Journal::begin(&roots.app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};
            if case!="no-backup"{j.record(Step::P1Backup,StepState::Done,Some(serde_json::json!({"header":{"backupId":j.load().attempt_id,"sourceLocation":"root","systemIdentifier":"123"}}))).unwrap();}
            let mut name=format!("convert-{}",uuid::Uuid::new_v4());match case {
                "log-traversal"=>name="convert-../foreign".into(),"log-alias"=>name=format!("convert-{}",uuid::Uuid::new_v4().simple()),"wrong-phase"=>name=format!("check-{}",uuid::Uuid::new_v4()),"selector"=>fs::write(roots.app.join(".nwui-auto/active-v1.json"),b"committed").unwrap(),"stale"=>{let mut current=j.load().clone();current.candidate.revision="a".repeat(40);fs::write(roots.app.join(".nwui-auto/journal-v1.json"),serde_json::to_vec(&current).unwrap()).unwrap();},_=>{},
            }
            let before=(files::snapshot(&roots.db).unwrap(),files::snapshot(&roots.app).unwrap());assert!(migration::upgrade_auto_at(&roots,&locks,&j,&name,UpgradeMode::Link,false).is_err(),"{case}");assert_eq!(before,(files::snapshot(&roots.db).unwrap(),files::snapshot(&roots.app).unwrap()));assert!(!roots.db.join(".nwui-auto-work").exists());
        }
    }
    #[test]
    fn w1_p3_refuses_missing_profile_prior_conversion_or_stale_authority_before_tools_or_target_files() {
        use crate::db_upgrade::{control::{Control,COMPATIBILITY_FIELDS},cluster::Snapshot};
        for case in ["profile","prior","selector","stale"] {
            let t=tempfile::tempdir().unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for p in [&roots.db,&roots.app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j={let fixture_journal=Journal::begin(&roots.app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};
            match case{"prior"=>j.record(Step::P3Convert,StepState::Intent,None).unwrap(),"selector"=>fs::write(roots.app.join(".nwui-auto/active-v1.json"),b"committed").unwrap(),"stale"=>{let mut other=Journal::open(&roots.app,&locks).unwrap();other.record(Step::P0b,StepState::Intent,None).unwrap();},_=>{}}
            let control=Control::parse(&format!("Database system identifier: 123
Database cluster state: shut down
{}",COMPATIBILITY_FIELDS.iter().map(|f|format!("{f}: 1
")).collect::<String>()),"").unwrap();let snapshot=Snapshot{databases:vec![],roles:vec![],data:Default::default(),collations:Default::default(),largest_index_bytes:0};let before=(files::snapshot(&roots.db).unwrap(),files::snapshot(&roots.app).unwrap());let count=j.load().records.len();assert!(super::run_locked(&roots,&locks,&mut j,&control,&snapshot).is_err(),"{case}");assert_eq!(count,j.load().records.len());assert_eq!(before,(files::snapshot(&roots.db).unwrap(),files::snapshot(&roots.app).unwrap()));
        }
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    #[ignore="native PG15+PG17: fixed nested auto-root transport, not A22 qualification"]
    async fn w1_auto_conversion_native_root_layout_links_or_copies_only_fixed_work_paths() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        use crate::db_upgrade::{postgres,inspect,cluster::Snapshot};use std::time::Duration;
        for (coordinated,recover_target) in [(false,false),(true,false),(true,true)] {for mode in [UpgradeMode::Link,UpgradeMode::Copy] {
            let t=tempfile::tempdir().unwrap();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o711)).unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:crate::db_upgrade::engine::architecture().unwrap().into()};postgres::directory(&roots.db).unwrap();files::private_dir(&roots.app).unwrap();
            let logs=t.path().join("logs");postgres::directory(&logs).unwrap();let source_log=logs.join("source");postgres::directory(&source_log).unwrap();
            let mut init=postgres::command(15,"initdb").unwrap();init.arg("-D").arg(&roots.db).args(["--no-locale","--encoding=UTF8","--auth=trust"]);postgres::capture(init,&logs,Duration::from_secs(300)).unwrap();
            let source=postgres::Private::start(15,&roots.db,&source_log).unwrap();let mut config=tokio_postgres::Config::new();config.host_path(&source.socket).port(50432).user("postgres").dbname("postgres");let (client,driver)=config.connect(tokio_postgres::NoTls).await.unwrap();let driver=tokio::spawn(driver);
            client.batch_execute("CREATE TABLE public.transport_probe(id integer PRIMARY KEY,value text); INSERT INTO public.transport_probe VALUES(1,'nested root')").await.unwrap();let source_file:String=client.query_one("SELECT pg_relation_filepath('public.transport_probe')",&[]).await.unwrap().get(0);let (databases,roles)=inspect::read_catalog(&client).await.unwrap();drop(client);driver.await.unwrap().unwrap();source.stop().unwrap();
            let control=postgres::read_control(15,&roots.db).unwrap();let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j={let fixture_journal=Journal::begin(&roots.app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};
            if coordinated {
                let names=fs::read_dir(&roots.db).unwrap().map(|e|e.unwrap().file_name().into_string().unwrap()).collect();let prepared=super::super::backup::prepare(&roots,&locks,&j.load().attempt_id,super::super::backup_format::SourceLocation::Root,&names,&control,"2026-10-28T00:00:00Z").unwrap();super::super::backup::create_verified(&roots,&locks,&mut j,&prepared).unwrap();
                // Real P1 backup, synthetic P2 profile only: this is P3 transport, NOT A22 qualification.
                j.record(Step::P2Preflight,StepState::Done,Some(super::super::profile::fixture_done(mode))).unwrap();
            }else{
                // Direct helper mapping proof only; no P1 attestation claimed on this branch.
                j.record(Step::P1Backup,StepState::Done,Some(serde_json::json!({"header":{"backupId":j.load().attempt_id,"sourceLocation":"root","systemIdentifier":control.system_identifier}}))).unwrap();
            }
            let work=roots.db.join(".nwui-auto-work").join(&j.load().attempt_id);postgres::directory(&roots.db.join(".nwui-auto-work")).unwrap();postgres::directory(&work).unwrap();let new=work.join("data");let snapshot=Snapshot{databases,roles,data:Default::default(),collations:Default::default(),largest_index_bytes:0};
            if coordinated{super::run_locked(&roots,&locks,&mut j,&control,&snapshot).unwrap();assert!(j.load().records.iter().any(|r|r.step==Step::P3Convert && r.state==StepState::Intent));assert!(j.load().records.iter().any(|r|r.step==Step::P3Convert && r.state==StepState::Done));}
            else{postgres::directory(&new).unwrap();let check_name=format!("check-{}",uuid::Uuid::new_v4());j.record(Step::P2Preflight,StepState::Intent,Some(serde_json::json!({"logName":check_name}))).unwrap();let init_log=super::super::diagnostics::create(&roots,&locks,&j,&check_name,true).unwrap();postgres::capture(migration::init_target_command(&new,&control,&snapshot).unwrap(),&init_log,Duration::from_secs(300)).unwrap();for check in [true,false] {let name=if check{check_name.clone()}else{let name=format!("convert-{}",uuid::Uuid::new_v4());j.record(Step::P3Convert,StepState::Intent,Some(serde_json::json!({"logName":name}))).unwrap();super::super::diagnostics::create(&roots,&locks,&j,&name,false).unwrap();name};migration::upgrade_auto_at(&roots,&locks,&j,&name,mode,check).unwrap();}}
            let log_name=j.load().records.iter().rev().find(|r|r.step==Step::P3Convert).unwrap().data.as_ref().unwrap().get("logName").unwrap().as_str().unwrap();let observe=super::super::diagnostics::path(&roots.db,&j.load().attempt_id,log_name).unwrap().join("observe");postgres::directory(&observe).unwrap();let target=postgres::Private::start(17,&new,&observe).unwrap();let mut config=tokio_postgres::Config::new();config.host_path(&target.socket).port(50432).user("postgres").dbname("postgres");let (client,driver)=config.connect(tokio_postgres::NoTls).await.unwrap();let driver=tokio::spawn(driver);
            assert_eq!(client.query_one("SELECT value FROM public.transport_probe WHERE id=1",&[]).await.unwrap().get::<_,String>(0),"nested root");let target_file:String=client.query_one("SELECT pg_relation_filepath('public.transport_probe')",&[]).await.unwrap().get(0);use std::os::unix::fs::MetadataExt;assert_eq!(fs::metadata(roots.db.join(source_file)).unwrap().ino()==fs::metadata(new.join(target_file)).unwrap().ino(),mode==UpgradeMode::Link);
            drop(client);driver.await.unwrap().unwrap();
            // P2/P4 are fixture attestations: exercise real physical P5, not A22 or G1.
            let publication=if coordinated && !crate::nonroot::PRIVILEGE_SPLIT {
                use super::super::{key_write::{self,Purpose,Mode},KeySource,KeyBinding,StorageSelection,Outcome};
                files::private_dir(&roots.app.join("secrets")).unwrap();let generation=uuid::Uuid::new_v4().to_string();
                key_write::write_locked(&locks,&roots,&mut j,Purpose::Staged{generation:generation.clone()},Mode::Publish).unwrap();
                key_write::write_locked(&locks,&roots,&mut j,Purpose::Publication{generation:generation.clone()},Mode::Prepare).unwrap();
                let source=KeySource::Staged{generation:generation.clone()};let key=crate::db_upgrade::key_file::read_key_file(&source.path(&roots.app,None),crate::db_upgrade::key_file::KeyPolicy::Strict).unwrap();
                let id=postgres::read_control(17,&new).unwrap().system_identifier;let mut pair=crate::db_upgrade::pair::Pair::fresh(&roots.architecture);pair.mode="migration".into();pair.ready=true;pair.fresh_step=None;pair.system_identifier=Some(id.clone());
                let selection=StorageSelection{version:1,epoch:7,outcome:Outcome::Upgrading,branch:Branch::Upgrade,pgdata:new.clone(),backup_id:Some(j.load().attempt_id.clone()),schema_profile:"legacy-28".into(),key_binding:KeyBinding::Verified{pair_id:pair.pair_id.clone()}};
                j.record(Step::P4Target,StepState::Done,None).unwrap();
                let pg=super::super::PrivatePg{socket_dir:target.socket.clone(),port:50432,dbname:"postgres".into(),role:"postgres".into(),major:17};super::super::runner::prepare_privileges(&selection,&locks,&pg,&mut j,crate::publisher::boot::BootMode::Conversion).await.unwrap();
                Some((generation,source,key,id,pair,selection))
            }else{None};
            target.stop().unwrap();
            if coordinated{let records=j.load().records.len();let before=fs::read(new.join("global/pg_control")).unwrap();assert!(super::run_locked(&roots,&locks,&mut j,&control,&snapshot).is_err());assert_eq!(j.load().records.len(),records);assert_eq!(fs::read(new.join("global/pg_control")).unwrap(),before);}
            if let Some((generation,source,key,id,pair,selection))=publication {
                let proof=super::super::profile::proof(&j).unwrap();let draft=super::super::generation::GenerationDraft{generation:&generation,origin:super::super::selector::Origin::Auto,source_profile:Some((&proof.profile_id,&proof.profile_sha256)),actual_id:&id,db_pair:&pair,app_pair:&pair,agreement:&selection.key_binding,key_source:&source,key:&key};
                if recover_target {
                    let _selected=super::super::generation::prepare(&roots,&locks,&mut j,draft).unwrap();super::super::target::publish(&roots,&locks,&mut j,&id).unwrap();assert!(roots.db.join("17/data").is_dir());assert!(!roots.app.join(".nwui-auto/active-v1.json").exists());
                    let j=super::super::restore::recover(roots.clone(),std::sync::Arc::new(locks),j,Step::P5Publish,"interrupted",super::super::boot_identity::current().ok()).await.unwrap();assert_eq!(j.load().outcome,super::super::Outcome::Restored);super::super::backup::verify_restored(&roots,&j).unwrap();assert!(!roots.db.join("17/data").exists());assert!(!roots.app.join("secrets/data_key").exists());assert!(!roots.app.join(".nwui-auto/gen").join(&generation).exists());assert!(observe.parent().unwrap().join("pg_upgrade_output.d").is_dir());assert!(!work.exists());
                    let restored_log=logs.join("restored");postgres::directory(&restored_log).unwrap();let restored=postgres::Private::start(15,&roots.db,&restored_log).unwrap();let mut config=tokio_postgres::Config::new();config.host_path(&restored.socket).port(50432).user("postgres").dbname("postgres");let (client,driver)=config.connect(tokio_postgres::NoTls).await.unwrap();let driver=tokio::spawn(driver);assert_eq!(client.query_one("SELECT value FROM public.transport_probe WHERE id=1",&[]).await.unwrap().get::<_,String>(0),"nested root");drop(client);driver.await.unwrap().unwrap();restored.stop().unwrap();continue;
                }
                let selected=super::super::publication::publish_upgrade_locked(&roots,&locks,&mut j,&selection,draft,&super::super::runner::Phase1Step::new(Step::P5Publish)).unwrap();assert_eq!(selected.pgdata,roots.db.join("17/data"));assert_eq!(fs::metadata(roots.db.join("17")).unwrap().mode()&0o7777,0o755);assert_eq!(fs::metadata(roots.db.join("17/data")).unwrap().mode()&0o7777,0o700);assert_eq!(selected.epoch,selection.epoch);assert!(!new.exists());assert!(!roots.app.join("secrets/data_key").exists());
                let locks=std::sync::Arc::new(locks);let mut j=super::super::committed::resume(roots.clone(),locks,j).await.unwrap();assert!(j.load().records.iter().any(|r|r.step==Step::P5Commit && r.state==StepState::Done && r.data.is_none()));
                let canonical=crate::db_upgrade::key_file::read_key_file(&roots.app.join("secrets/data_key"),crate::db_upgrade::key_file::KeyPolicy::Strict).unwrap();assert_eq!(canonical.bytes,key.bytes);
                let (live,_):(crate::db_upgrade::pair::Pair,_)=super::super::selector::read_record(&roots.app.join(".nwui/pair-v1.json")).unwrap();assert!(live.encryption_key_required);assert_eq!(live.system_identifier.as_deref(),Some(id.as_str()));assert!(!roots.db.join(".nwui").exists());assert!(!roots.db.join("PG_VERSION").exists());assert!(roots.db.join("PG_VERSION.nwui-pre17").exists());assert!(roots.db.join("UPGRADED-TO-PG17.txt").exists());
                // Transport/cleanup proof only: the image gate separately proves service activation.
                let identity=super::super::boot_identity::current().unwrap();j.set_outcome(Outcome::Pg17).unwrap();j.handoff(super::super::journal::Handoff::Intent,&identity).unwrap();j.handoff(super::super::journal::Handoff::Done,&identity).unwrap();j.record(Step::Phase2Monitor,StepState::Done,Some(serde_json::json!({"bootIdentity":identity,"generation":generation,"observedAt":"2026-10-29T00:00:00Z"}))).unwrap();
                let mut cleanup_locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();cleanup_locks.release_d();let selected=super::super::selector::read_selector(&roots.app).unwrap();let selected=super::super::selector::load_generation(&roots.db,&roots.app,&roots.architecture,selected,&id,&canonical.bytes).unwrap();super::super::cleanup::cleanup(&roots,&cleanup_locks,&mut j,&selected).unwrap();assert!(!work.exists());assert!(observe.parent().unwrap().join("pg_upgrade_output.d").is_dir());assert!(roots.db.join(".nwui-pg15-backup").join(&j.load().attempt_id).join("PG_VERSION").is_file());assert_eq!(j.load().outcome,Outcome::Pg17);
                let after_log=observe.parent().unwrap().join("post-cleanup");postgres::directory(&after_log).unwrap();let after=postgres::Private::start(17,&roots.db.join("17/data"),&after_log).unwrap();let connection=crate::db_upgrade::inspect::connect(&after.socket,"postgres","postgres",None).await.unwrap();assert_eq!(connection.client.query_one("SELECT value FROM public.transport_probe WHERE id=1",&[]).await.unwrap().get::<_,String>(0),"nested root");drop(connection);after.stop().unwrap();
            }
        }}
    }

}
