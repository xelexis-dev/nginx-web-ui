//! Finite PG17 branch: K/agreement, private A.11 steps, then immutable ready authority.
use super::{Journal,KeyBinding,KeySource,PrivatePg,StepError,StorageLocks,StorageSelection,boot,generation,selector::{self,Origin},journal::Candidate};
use crate::{config::RuntimeConfig,db_upgrade::{files,key_binding,layout::{self,Roots,Decision},pair::Pair,postgres}};
use std::{path::Path,sync::Arc};
fn map(e:crate::db_upgrade::error::Error)->StepError {StepError{code:e.code,message:e.message,detail:e.detail.map(str::to_owned)}}
fn mismatch()->StepError {StepError{code:"pair_mismatch",message:"PG17 데이터와 준비 기록이 일치하지 않습니다. 기존 데이터를 보존했습니다.",detail:None}}
fn identifier(path:&Path)->crate::db_upgrade::error::Result<String> {let c=postgres::read_control(17,path)?;Ok(c.system_identifier)}
fn present(path:&Path)->Result<bool,StepError> {match std::fs::symlink_metadata(path){Ok(_)=>Ok(true),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>Ok(false),Err(_)=>Err(mismatch())}}
pub(super) fn link_window(roots:&Roots,j:&Journal)->Result<Option<(std::path::PathBuf,std::collections::BTreeSet<String>)>,StepError>{
    if j.load().branch!=super::Branch::Upgrade{return Ok(None);}
    let linked=j.load().records.iter().rev().find(|r|r.step==super::Step::P3Convert&&r.state==super::StepState::Intent).and_then(|r|r.data.as_ref()).and_then(|v|v.get("mayBeLinked")).and_then(|v|v.as_bool())==Some(true);
    let committed=j.load().records.iter().rev().find(|r|r.step==super::Step::P5Commit).is_some_and(|r|r.state==super::StepState::Done);
    if !linked||!committed{return Ok(None);}
    let(l,pending)=super::cleanup::pending_entries(roots,j)?;Ok((!pending.is_empty()).then_some((l,pending)))
}
/// Repair only after trusted pair/selector admission, then read and cross-check physical control.
pub(super) fn repair_for_start(roots:&Roots,locks:&StorageLocks,j:&Journal)->Result<(Pair,String),StepError>{
    use crate::db_upgrade::{key_file,paths};
    if !locks.holds_d()||!locks.matches_roots(&roots.db,&roots.app)||!j.belongs_to(&roots.app){return Err(mismatch());}
    let saved=Journal::open(&roots.app,locks)?;if serde_json::to_value(saved.load()).map_err(|_|mismatch())?!=serde_json::to_value(j.load()).map_err(|_|mismatch())?{return Err(mismatch());}
    let pair:Pair=files::read_json(&roots.app.join(".nwui/pair-v1.json")).map_err(|_|mismatch())?;pair.validate().map_err(map)?;
    if !pair.ready||pair.architecture!=roots.architecture{return Err(mismatch());}
    let active=present(&roots.app.join(".nwui-auto/active-v1.json"))?;
    let authority=if active{
        let selected=selector::read_selector(&roots.app)?;let source=j.key_source()?;
        let canonical=paths::startup_data_key_file(&roots.app).map_err(map)?;let path=source.path(&roots.app,Some(canonical.as_os_str()));
        let key=key_file::read_key_file(&path,key_file::policy_for(&source,&roots.app,&canonical)).map_err(|_|mismatch())?;
        let selected=selector::load_authority(&roots.db,&roots.app,&roots.architecture,selected,&key.bytes)?;Some((selected,key))
    }else{
        let Decision::Ready(saved)=layout::classify_metadata_locked(locks,roots).map_err(map)?else{return Err(mismatch())};if saved!=pair{return Err(mismatch());}None
    };
    let linked=link_window(roots,j)?;
    let window=linked.as_ref().map(|(l,pending)|postgres::LinkWindow{l_root:l.as_path(),pending});
    postgres::repair_cluster_ownership(&roots.db.join("17"),window).map_err(map)?;
    let id=layout::control(roots,&identifier).map_err(map)?;
    if pair.system_identifier.as_deref()!=Some(id.as_str()){return Err(mismatch());}
    if let Some((before,key))=authority{
        let selected=selector::read_selector(&roots.app)?;if selected!=before.selector{return Err(mismatch());}
        let after=selector::load_generation(&roots.db,&roots.app,&roots.architecture,selected,&id,&key.bytes)?;
        let live:Pair=files::read_json(&roots.app.join(".nwui/pair-v1.json")).map_err(|_|mismatch())?;
        if after.manifest!=before.manifest||after.db_pair!=before.db_pair||after.app_pair!=before.app_pair||live!=pair{return Err(mismatch());}
    }else{let Decision::Ready(saved)=layout::classify_locked(locks,roots,&identifier).map_err(map)?else{return Err(mismatch())};if saved!=pair{return Err(mismatch());}}
    Ok((pair,id))
}
pub(crate) struct Prepared {pub journal:Journal,pub selection:StorageSelection,pub privileges:crate::nonroot::Prepared}
/// Phase-1 dispatch into F1 or Y1, before RuntimeConfig requires the persisted secrets.
/// The caller must finish normal upgrade resume/roll-forward before entering this branch.
pub(crate) async fn enter(roots:Roots,locks:Arc<StorageLocks>,journal:Option<Journal>,current:Candidate,inputs:crate::db_upgrade::fresh::Inputs,envdir:std::path::PathBuf)->Result<Journal,StepError> {
    let r=roots.clone();let l=locks.clone();let candidate=current.clone();
    let journal=tokio::task::spawn_blocking(move|| {
        if !l.holds_d() || !l.matches_roots(&r.db,&r.app){return Err(mismatch());}
        if let Some(j)=journal {
            if !j.belongs_to(&r.app) || serde_json::to_value(Journal::open(&r.app,&l)?.load()).map_err(|_|mismatch())?!=serde_json::to_value(j.load()).map_err(|_|mismatch())?
                || j.load().branch==super::Branch::Upgrade && !j.load().records.iter().any(|record|record.step==super::Step::P5Commit && record.state==super::StepState::Done && record.data.is_none()) {return Err(mismatch());}
            Ok(Some(j))
        }else{
            match layout::classify_metadata_locked(&l,&r).map_err(map)? {
                Decision::Empty|Decision::Fresh(_)=>Ok(None),
                Decision::Ready(_)=>Journal::begin(&r.app,&l,super::Branch::Ready,candidate).map(Some),
            }
        }
    }).await.map_err(|_|mismatch())??;
    let journal=match journal {
        Some(j)=>j,
        None=>super::fresh::provision(roots.clone(),locks.clone(),current,inputs,envdir.clone()).await?.0,
    };
    tokio::task::spawn_blocking(move|| {
        crate::db_upgrade::fresh::ready_environment_locked(&locks,&roots,&envdir).map_err(map)?;
        Ok(journal)
    }).await.map_err(|_|mismatch())?
}
/// config was read from root startup settings after the existing secret environment publication.
/// Caller awaits this whole finite operation under custody; no gated process starts here.
pub(crate) async fn prepare(roots:Roots,locks:Arc<StorageLocks>,journal:Journal,current:Candidate,config:RuntimeConfig,inputs:boot::Inputs)->Result<Prepared,StepError> {
    if !journal.belongs_to(&roots.app) || journal.load().branch==super::Branch::Upgrade && !journal.load().records.iter().any(|r|r.step==super::Step::P5Commit && r.state==super::StepState::Done && r.data.is_none()) {return Err(mismatch());}
    let r=roots.clone();let l=locks.clone();
    let (journal,pair,id)=tokio::task::spawn_blocking(move|| {
        if !l.holds_d() || !l.matches_roots(&r.db,&r.app){return Err(mismatch());}
        let(pair,id)=repair_for_start(&r,&l,&journal)?;Ok((journal,pair,id))
    }).await.map_err(|_|mismatch())??;
    if config.data_key_file!=crate::db_upgrade::paths::startup_data_key_file(&roots.app).map_err(map)?{return Err(mismatch());}
    let ensured=super::key_write::ensure_key_locked(roots.clone(),locks.clone(),journal,pair.encryption_key_required).await?;
    let runtime=tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move|| {
        let mut j=ensured.journal;let key=ensured.key;
        if ensured.source!=KeySource::Canonical{return Err(mismatch());}
        let active=roots.app.join(".nwui-auto/active-v1.json");
        let selected=if present(&active)? {
            let selected=selector::read_selector(&roots.app)?;
            let checked=selector::load_generation(&roots.db,&roots.app,&roots.architecture,selected.clone(),&id,&key.bytes)?;
            if checked.app_pair.pair_id!=pair.pair_id{return Err(mismatch());}
            generation::mark_key_required(&roots,&locks,&id,&key)?;
            Some(selected)
        }else{
            let Decision::Ready(saved)=layout::classify_locked(&locks,&roots,&identifier).map_err(map)? else{return Err(mismatch())};
            if saved!=pair{return Err(mismatch());}
            key_binding::verify_key_locked(&locks,&roots,&identifier,&key.secrets).map_err(map)?;
            key_binding::mark_key_required_locked(&locks,&roots,&identifier,&key.secrets).map_err(map)?;
            None
        };
        let agreement=KeyBinding::Verified{pair_id:pair.pair_id.clone()};
        let epoch=j.load().records.last().map(|r|r.seq).unwrap_or(0).checked_add(1).ok_or_else(mismatch)?;
        let selection=StorageSelection{version:1,epoch,outcome:super::Outcome::Pg17,branch:j.load().branch,pgdata:roots.db.join("17/data"),backup_id:selected.as_ref().and_then(|s|s.backup_id.clone()),schema_profile:selected.as_ref().and_then(|s|s.profile_id.clone()).unwrap_or_else(||"none".into()),key_binding:agreement.clone()};
        let work=std::path::Path::new("/run").join(format!("nwui-ready-{}",uuid::Uuid::new_v4()));postgres::directory(&work).map_err(map)?;
        let server=postgres::Private::start_locked_ready(&locks,&roots,&work).map_err(map)?;
        let url:tokio_postgres::Config=config.database_url.parse().map_err(|_|mismatch())?;
        let pg=PrivatePg{socket_dir:server.socket.clone(),port:50432,dbname:url.get_dbname().ok_or_else(mismatch)?.into(),role:url.get_user().ok_or_else(mismatch)?.into(),major:17};
        // This is already-ready PG17, not a converted target; there is no V1/V2 boundary.
        let boot_result=runtime.block_on(async {
            boot::migrate(&selection,&current,&config,&key.secrets,&pg,&locks,&mut j).await?;
            let notices=boot::notices(&roots,&j)?;
            boot::finish(&selection,&config,&key.secrets,&pg,&locks,&mut j,inputs,&notices,crate::publisher::boot::BootMode::Ready).await
        });
        // Reap the private postmaster on every failure as well as success, before any handoff.
        let stopped=server.stop().map_err(map);
        let privileges=match boot_result{Err(e)=>{stopped?;return Err(e)},Ok(p)=>{stopped?;p}};
        if selected.is_none() {
            let live:Pair=files::read_json(&roots.app.join(".nwui/pair-v1.json")).map_err(|_|mismatch())?;
            let origin=match pair.mode.as_str(){"fresh"=>Origin::Fresh,"migration"=>Origin::Manual,_=>return Err(mismatch())};
            let generation=uuid::Uuid::new_v4().to_string();
            let selected=generation::prepare(&roots,&locks,&mut j,generation::GenerationDraft{generation:&generation,origin,source_profile:None,actual_id:&id,db_pair:&live,app_pair:&live,agreement:&agreement,key_source:&KeySource::Canonical,key:&key})?;
            generation::commit(&roots,&locks,&mut j,&selected,&id,&key)?;
        }
        Ok(Prepared{journal:j,selection,privileges})
    }).await.map_err(|_|StepError{code:"boot_step_failed",message:"PG17 시작 준비를 완료하지 못했습니다. 기존 데이터를 보존했습니다.",detail:None})?
}

#[cfg(test)]
mod tests {
    use super::*;use std::{fs,os::unix::fs::{PermissionsExt,MetadataExt,DirBuilderExt}};
    #[tokio::test]
    #[ignore="native PG17: database_newer must stop the private postmaster"]
    async fn database_newer_ready_failure_stops_private_postmaster() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        let t=tempfile::tempdir().unwrap();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o711)).unwrap();
        let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:crate::db_upgrade::paths::architecture().unwrap().into()};
        for p in [&roots.db,&roots.app]{fs::DirBuilder::new().mode(0o700).create(p).unwrap();}
        let env=t.path().join("env");fs::DirBuilder::new().mode(0o700).create(&env).unwrap();
        let locks=Arc::new(StorageLocks::acquire(&roots.db,&roots.app).unwrap());
        let dir=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");
        let migrations=crate::migrations::read_migrations(&dir).await.unwrap();
        let future=migrations.last().unwrap().timestamp+1;
        let candidate=Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:super::super::journal::migrations_sha256(&migrations)};
        let j=enter(roots.clone(),locks.clone(),None,candidate.clone(),crate::db_upgrade::fresh::Inputs{auth_secret:Some("fixture-auth-only".into()),admin_password:None},env.clone()).await.unwrap();
        let work=t.path().join("future-ledger");postgres::directory(&work).unwrap();
        let server=postgres::Private::start_locked_ready(&locks,&roots,&work).unwrap();
        let app=crate::db_upgrade::inspect::connect(&server.socket,"nginx_web_ui","postgres",None).await.unwrap();
        app.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();
        app.client.batch_execute("SET ROLE nginx; CREATE SCHEMA drizzle; CREATE TABLE drizzle.__drizzle_migrations(id serial PRIMARY KEY,hash text NOT NULL,created_at bigint)").await.unwrap();
        app.client.execute("INSERT INTO drizzle.__drizzle_migrations(hash,created_at) VALUES('future',$1)",&[&future]).await.unwrap();
        drop(app);server.stop().unwrap();
        let mut config=RuntimeConfig::read(|name|match name {"AUTH_SECRET"|"DATABASE_URL"=>Some(fs::read_to_string(env.join(name)).unwrap()),_=>None}).unwrap();
        config.data_key_file=roots.app.join("secrets/data_key");config.migrations_dir=dir;
        let error=match prepare(roots.clone(),locks.clone(),j,candidate,config,boot::Inputs{admin_email:None,admin_password:None,backfill:None}).await{Err(error)=>error,Ok(_)=>panic!("newer database must refuse")};
        assert_eq!(error.code,"database_newer");assert!(!roots.db.join("17/data/postmaster.pid").exists());
        let work=t.path().join("verify-preserved");postgres::directory(&work).unwrap();
        let server=postgres::Private::start_locked_ready(&locks,&roots,&work).unwrap();
        let app=crate::db_upgrade::inspect::connect(&server.socket,"nginx_web_ui","postgres",None).await.unwrap();
        let row=app.client.query_one("SELECT count(*),max(created_at) FROM drizzle.__drizzle_migrations",&[]).await.unwrap();
        assert_eq!(row.get::<_,i64>(0),1);assert_eq!(row.get::<_,i64>(1),future);
        drop(app);server.stop().unwrap();
        eprintln!("native database_newer: private postmaster stopped, future ledger preserved");
    }
    #[tokio::test]
    async fn w1_ready_entry_never_routes_uncommitted_upgrade_or_foreign_journal_into_f1() {
        for case in ["upgrade","foreign","stale"] {
            let t=tempfile::tempdir().unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
            for p in [&roots.db,&roots.app]{fs::DirBuilder::new().mode(0o700).create(p).unwrap();}
            let locks=Arc::new(StorageLocks::acquire(&roots.db,&roots.app).unwrap());let candidate=Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)};
            let branch=if case=="upgrade"{super::super::Branch::Upgrade}else{super::super::Branch::Ready};let j={let fixture_journal=Journal::begin(&roots.app,&locks,branch,candidate.clone()).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};
            if case=="stale"{let mut other=Journal::open(&roots.app,&locks).unwrap();other.record(super::super::Step::Y2,super::super::StepState::Intent,None).unwrap();}
            let mut selected=roots.clone();if case=="foreign"{selected.app=t.path().join("foreign");fs::DirBuilder::new().mode(0o700).create(&selected.app).unwrap();}
            let env=t.path().join("env");fs::DirBuilder::new().mode(0o700).create(&env).unwrap();let before=(files::snapshot(&roots.app).unwrap(),files::snapshot(&roots.db).unwrap());
            assert_eq!(enter(selected,locks,Some(j),candidate,crate::db_upgrade::fresh::Inputs::default(),env.clone()).await.err().unwrap().code,"pair_mismatch");
            assert_eq!(before,(files::snapshot(&roots.app).unwrap(),files::snapshot(&roots.db).unwrap()));assert!(fs::read_dir(env).unwrap().next().is_none());
        }
    }
    #[tokio::test]
    async fn w1_ready_never_treats_an_uncommitted_upgrade_as_ready_pg17() {
        let t=tempfile::tempdir().unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
        for p in [&roots.db,&roots.app]{fs::DirBuilder::new().mode(0o700).create(p).unwrap();}
        let locks=Arc::new(StorageLocks::acquire(&roots.db,&roots.app).unwrap());let candidate=Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)};
        let j={let fixture_journal=Journal::begin(&roots.app,&locks,super::super::Branch::Upgrade,candidate.clone()).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};let before=files::snapshot(&roots.app).unwrap();
        let config=RuntimeConfig::read(|name|match name{"AUTH_SECRET"=>Some("fixture-only".into()),"DATABASE_URL"=>Some("postgres://nginx:fixture-only@127.0.0.1/nginx_web_ui".into()),_=>None}).unwrap();
        assert_eq!(prepare(roots.clone(),locks,j,candidate,config,boot::Inputs{admin_email:None,admin_password:None,backfill:None}).await.err().unwrap().code,"pair_mismatch");assert_eq!(files::snapshot(&roots.app).unwrap(),before);assert!(fs::read_dir(roots.db).unwrap().next().is_none());
    }
    #[tokio::test]
    async fn w1_ready_revalidates_the_pg17_tree_before_key_or_postmaster_work() {
        let t=tempfile::tempdir().unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
        for p in [&roots.db,&roots.app,&roots.db.join("17"),&roots.db.join("17/data"),&roots.db.join("17/data/global"),&roots.app.join(".nwui")] {fs::DirBuilder::new().mode(0o700).create(p).unwrap();}
        fs::write(roots.db.join("17/data/PG_VERSION"),b"17\n").unwrap();fs::write(roots.db.join("17/data/global/pg_control"),b"fixture").unwrap();std::os::unix::fs::symlink("/not-opened",roots.db.join("17/data/pg_wal")).unwrap();
        let mut pair=Pair::fresh("arm64");pair.ready=true;pair.fresh_step=None;pair.system_identifier=Some("123".into());files::atomic_json(&roots.app.join(".nwui/pair-v1.json"),&pair).unwrap();files::private_dir(&roots.db.join(".nwui")).unwrap();files::atomic_json(&roots.db.join(".nwui/pair-v1.json"),&pair).unwrap();
        let locks=Arc::new(StorageLocks::acquire(&roots.db,&roots.app).unwrap());let candidate=Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)};let j=Journal::begin(&roots.app,&locks,super::super::Branch::Ready,candidate.clone()).unwrap();
        let path=roots.app.join(".nwui-auto/journal-v1.json");let before=fs::read(&path).unwrap();let mut config=RuntimeConfig::read(|name|match name{"AUTH_SECRET"=>Some("fixture-only".into()),"DATABASE_URL"=>Some("postgres://nginx:fixture-only@127.0.0.1/nginx_web_ui".into()),_=>None}).unwrap();config.data_key_file=roots.app.join("secrets/data_key");
        assert_eq!(prepare(roots.clone(),locks,j,candidate,config,boot::Inputs{admin_email:None,admin_password:None,backfill:None}).await.err().unwrap().code,"unsupported_layout");assert_eq!(fs::read(path).unwrap(),before);assert!(!roots.app.join("secrets").exists());assert_eq!(fs::read_link(roots.db.join("17/data/pg_wal")).unwrap(),Path::new("/not-opened"));
    }
    #[tokio::test]
    #[ignore="native PG17: disposable fresh and repeated ready branch"]
    async fn w1_ready_native_private_boot_commits_once_and_repeats_without_replacing_authority() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        if crate::nonroot::PRIVILEGE_SPLIT{return;}
        let t=tempfile::tempdir().unwrap();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o711)).unwrap();
        let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:crate::db_upgrade::paths::architecture().unwrap().into()};
        for p in [&roots.db,&roots.app]{fs::DirBuilder::new().mode(0o700).create(p).unwrap();}
        let env=t.path().join("env");fs::DirBuilder::new().mode(0o700).create(&env).unwrap();
        let locks=Arc::new(StorageLocks::acquire(&roots.db,&roots.app).unwrap());
        let dir=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");
        let candidate=Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:super::super::journal::migrations_sha256(&crate::migrations::read_migrations(&dir).await.unwrap())};
        let j=enter(roots.clone(),locks.clone(),None,candidate.clone(),crate::db_upgrade::fresh::Inputs{auth_secret:Some("fixture-auth-only".into()),admin_password:None},env.clone()).await.unwrap();
        assert_eq!(j.load().branch,super::super::Branch::Fresh);assert!(!roots.app.join("secrets/data_key").exists());
        let mut config=RuntimeConfig::read(|name|match name {"AUTH_SECRET"|"DATABASE_URL"=>Some(fs::read_to_string(env.join(name)).unwrap()),_=>None}).unwrap();
        config.data_key_file=roots.app.join("secrets/data_key");config.target_dns.descriptor_path=t.path().join("target-dns.json");config.migrations_dir=dir;
        let inputs=||boot::Inputs{admin_email:None,admin_password:None,backfill:None};
        let mut prepared=prepare(roots.clone(),locks.clone(),j,candidate.clone(),config.clone(),inputs()).await.unwrap();prepared.journal=super::super::committed::resume(roots.clone(),locks.clone(),prepared.journal).await.unwrap();
        assert!(matches!(prepared.selection.key_binding,KeyBinding::Verified{..}));assert!(prepared.privileges.payload.is_none() && prepared.privileges.key_receipt.is_none());assert!(!roots.db.join("17/data/postmaster.pid").exists());
        let key=fs::read(&config.data_key_file).unwrap();assert_eq!(key.len(),64);assert_eq!(fs::metadata(&config.data_key_file).unwrap().mode()&0o7777,0o600);
        let path=roots.app.join(".nwui-auto/active-v1.json");let selected=fs::read(&path).unwrap();assert_eq!(selector::read_selector(&roots.app).unwrap().origin,Origin::Fresh);
        let saved:Pair=files::read_json(&roots.app.join(".nwui/pair-v1.json")).unwrap();assert!(saved.encryption_key_required);
        // A killed container leaves both an unclean control state and postmaster.pid.
        let work=t.path().join("crash-work");postgres::directory(&work).unwrap();
        let server=postgres::Private::start(17,&roots.db.join("17/data"),&work).unwrap();
        let pid: i32=fs::read_to_string(roots.db.join("17/data/postmaster.pid")).unwrap().lines().next().unwrap().parse().unwrap();
        assert!(pid>1);assert_eq!(unsafe{libc::kill(pid,libc::SIGKILL)},0);drop(server);
        assert!(roots.db.join("17/data/postmaster.pid").is_file());
        std::os::unix::fs::chown(roots.db.join("17"),Some(0),Some(0)).unwrap();std::os::unix::fs::chown(roots.db.join("17/data/global/pg_control"),Some(12345),Some(12345)).unwrap();
        let committed=super::super::committed::resume(roots.clone(),locks.clone(),prepared.journal).await.unwrap();
        let(uid,gid)=postgres::postgres_ids().unwrap();let m=fs::metadata(roots.db.join("17/data/global/pg_control")).unwrap();assert_eq!((m.uid(),m.gid()),(uid,gid));
        let resumed=enter(roots.clone(),locks.clone(),Some(committed),candidate.clone(),crate::db_upgrade::fresh::Inputs::default(),env.clone()).await.unwrap();
        std::os::unix::fs::chown(roots.db.join("17"),Some(0),Some(0)).unwrap();std::os::unix::fs::chown(roots.db.join("17/data/global/pg_control"),Some(12345),Some(12345)).unwrap();
        let mut repeated=prepare(roots.clone(),locks.clone(),resumed,candidate.clone(),config.clone(),inputs()).await.unwrap();
        let m=fs::metadata(roots.db.join("17/data/global/pg_control")).unwrap();assert_eq!((m.uid(),m.gid()),(uid,gid));
        let authority=files::snapshot(&roots.app).unwrap();let tree=files::snapshot(&roots.db.join("17")).unwrap();repair_for_start(&roots,&locks,&repeated.journal).unwrap();assert_eq!(files::snapshot(&roots.app).unwrap(),authority);assert_eq!(files::snapshot(&roots.db.join("17")).unwrap(),tree);
        assert_eq!(fs::read(path).unwrap(),selected);assert_eq!(fs::read(&config.data_key_file).unwrap(),key);assert!(!roots.db.join("17/data/postmaster.pid").exists());assert_eq!(repeated.selection.outcome,super::super::Outcome::Pg17);
        assert!(fs::File::open(&roots.app).unwrap().try_lock().is_err());assert!(fs::File::open(&roots.db).unwrap().try_lock().is_err());
        async fn notice_count(roots:&Roots,locks:&StorageLocks,log:&Path)->i64{
            postgres::directory(log).unwrap();let server=postgres::Private::start_locked_ready(locks,roots,log).unwrap();let app=crate::db_upgrade::inspect::connect(&server.socket,"nginx_web_ui","postgres",None).await.unwrap();let count=app.client.query_one("SELECT count(*)::bigint FROM public.notifications WHERE type='system.db_upgrade_activation_failed'",&[]).await.unwrap().get(0);drop(app);server.stop().unwrap();count
        }
        assert_eq!(notice_count(&roots,&locks,&t.path().join("notice-before")).await,0);
        repeated.journal.fail_attempt(super::super::Step::Phase2Monitor,&StepError{code:"activation_timeout",message:"fixture",detail:None},super::super::Outcome::ActivationFailed,chrono::Utc::now()).unwrap();
        assert_eq!(notice_count(&roots,&locks,&t.path().join("notice-not-yet")).await,0,"phase2 has no notification writer");
        repeated=prepare(roots.clone(),locks.clone(),repeated.journal,candidate.clone(),config.clone(),inputs()).await.unwrap();assert_eq!(notice_count(&roots,&locks,&t.path().join("notice-after")).await,1);
        repeated=prepare(roots.clone(),locks.clone(),repeated.journal,candidate.clone(),config.clone(),inputs()).await.unwrap();assert_eq!(notice_count(&roots,&locks,&t.path().join("notice-dedupe")).await,1);
        let mut broken=config.clone();broken.migrations_dir=t.path().join("missing-migrations");let error=match prepare(roots.clone(),locks.clone(),repeated.journal,candidate.clone(),broken,inputs()).await{Err(error)=>error,Ok(_)=>panic!("missing packaged migrations must refuse")};
        let mut refused=Journal::open(&roots.app,&locks).unwrap();assert!(super::super::replacement::is_committed(&roots).unwrap());let outcome=super::super::runner::committed_failure_outcome(refused.load(),super::super::Step::Y2);assert_eq!(outcome,super::super::Outcome::RefusedPrivilege);refused.fail_attempt_with_identity(super::super::Step::Y2,&error,outcome,chrono::Utc::now(),Some(&"b".repeat(64))).unwrap();assert!(super::super::boot::notices(&roots,&refused).unwrap().is_empty());let status=super::super::refusal::status_value(&roots,refused.load(),outcome,&candidate,None).unwrap();assert!(!serde_json::to_string(&status).unwrap().contains("restore-pg15"));assert_eq!(notice_count(&roots,&locks,&t.path().join("notice-after-y2-refusal")).await,1);repeated.journal=refused;
        // A different physical cluster must never inherit the trusted selector's identity.
        let authority=files::snapshot(&roots.app).unwrap();let retained=t.path().join("retained-original");fs::rename(roots.db.join("17/data"),&retained).unwrap();
        let data=roots.db.join("17/data");postgres::directory(&data).unwrap();let logs=t.path().join("different-control");postgres::directory(&logs).unwrap();let mut init=postgres::command(17,"initdb").unwrap();init.arg("-D").arg(&data).args(["--no-locale","--encoding=UTF8","--auth=trust"]);postgres::capture(init,&logs,std::time::Duration::from_secs(300)).unwrap();std::os::unix::fs::chown(data.join("global/pg_control"),Some(12345),Some(12345)).unwrap();
        let error=repair_for_start(&roots,&locks,&repeated.journal).unwrap_err();assert_eq!(error.code,"pair_mismatch");assert_eq!(files::snapshot(&roots.app).unwrap(),authority);assert!(retained.join("global/pg_control").is_file());let m=fs::metadata(data.join("global/pg_control")).unwrap();assert_eq!((m.uid(),m.gid()),(uid,gid));
    }
}
