//! P2 check transport: diagnostic retention and scratch retirement precede profile Done.
use super::{Journal,Step,StepState,StepError,StorageLocks,profile::Admitted};
use crate::db_upgrade::{layout::Roots,control::Control,migration::{self,UpgradeMode},postgres};
use std::time::Duration;
fn mismatch()->StepError{StepError{code:"verification_failed",message:"사전 검사 대상과 검증된 원본 기록이 다릅니다. 기존 데이터를 보존했습니다.",detail:None}}
fn map(e:crate::db_upgrade::error::Error)->StepError{StepError{code:e.code,message:e.message,detail:e.detail.map(str::to_owned)}}
/// P2 owns one diagnostic directory before either source inspection or target initialization.
pub(crate) fn begin_locked(roots:&Roots,locks:&StorageLocks,j:&mut Journal,control:&Control)->Result<String,StepError>{
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app) || j.load().branch!=super::Branch::Upgrade {return Err(mismatch());}
    let current=Journal::open(&roots.app,locks)?;
    if serde_json::to_value(current.load()).map_err(|_|mismatch())?!=serde_json::to_value(j.load()).map_err(|_|mismatch())?{return Err(mismatch());}
    super::replacement::precommit(&roots.app,j)?;
    let header=super::backup::verify_backup(roots,j)?;super::source::control(control)?;
    if control.system_identifier!=header.system_identifier || j.load().records.iter().any(|r|matches!(r.step,Step::P2Preflight|Step::P3Convert)){return Err(mismatch());}
    let work=roots.db.join(".nwui-auto-work").join(&j.load().attempt_id);
    for path in [work.join("data"),work.join("target")] {match std::fs::symlink_metadata(path){Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},_=>return Err(mismatch())}}
    let name=format!("check-{}",uuid::Uuid::new_v4());j.record(Step::P2Preflight,StepState::Intent,Some(serde_json::json!({"logName":name})))?;
    super::diagnostics::create(roots,locks,j,&name,true)?;Ok(name)
}
/// P2 first admits all collation dependencies, then inspects the released application profile.
/// Both private postmasters are stopped and reaped before returning, including error paths.
pub(crate) async fn inspect_source(roots:Roots,locks:std::sync::Arc<StorageLocks>,j:&Journal,name:&str,password:&str,key:Option<std::sync::Arc<crate::crypto::SecretBox>>,packaged:&[crate::migrations::Migration])->Result<(Admitted,std::fs::File),StepError>{
    use crate::db_upgrade::{cluster,inspect};
    let r=roots.clone();let l=locks.clone();let frame=j.load().clone();let name=name.to_owned();
    let (logs,header)=tokio::task::spawn_blocking(move||{
        if !l.holds_d()||!l.matches_roots(&r.db,&r.app){return Err(mismatch());}
        let current=Journal::open(&r.app,&l)?;
        if serde_json::to_value(current.load()).map_err(|_|mismatch())?!=serde_json::to_value(frame).map_err(|_|mismatch())?{return Err(mismatch());}
        Ok((super::diagnostics::owned_path(&r.db,&current,&name,true)?,super::backup::open_verified_manifest(&r,&current)?.header))
    }).await.map_err(|_|mismatch())??;
    let data=match header.source_location{super::backup_format::SourceLocation::Root=>roots.db.clone(),super::backup_format::SourceLocation::Pgdata=>roots.db.join("pgdata")};
    let start=|label:&str|{
        let roots=roots.clone();let locks=locks.clone();let data=data.clone();let log=logs.join(label);
        tokio::task::spawn_blocking(move||{postgres::directory(&log).map_err(map)?;postgres::Private::start_locked_legacy(&locks,&roots,&data,&log).map_err(map)})
    };
    let server=start("catalog").await.map_err(|_|mismatch())??;
    let collected=cluster::collect_auto_collations(&server.socket,true).await.map_err(map);
    tokio::task::spawn_blocking(move||server.stop().map_err(map)).await.map_err(|_|mismatch())??;
    let mut collations=collected?;
    let r=roots.clone();let l=locks.clone();let k=key.clone();let log=logs.clone();let location=header.source_location;
    let (template0,used,bytes)=tokio::task::spawn_blocking(move||super::template0::inspect(&r,&l,location,&log,k.as_deref())).await.map_err(|_|mismatch())??;
    collations.collations.insert("template0".into(),used);
    super::collation::validate(&collations)?;
    let server=start("profile").await.map_err(|_|mismatch())??;
    let result=async{
        let app_name=collations.application().map_err(map)?;
        let app=inspect::connect(&server.socket,&app_name,"postgres",None).await.map_err(map)?;
        inspect::require_legacy_ledger(&app.client).await.map_err(|e|{let mut error=map(e);if let Some(guidance)=super::status::source_guidance(error.code=="source_too_old"){error.detail=Some(guidance.into());}error})?;
        let rows=app.client.query("SELECT DISTINCT hash,created_at FROM ONLY drizzle.__drizzle_migrations ORDER BY created_at,hash",&[]).await.map_err(|_|mismatch())?;
        let ledger=rows.iter().map(|row|Ok((row.try_get::<_,String>(0).map_err(|_|mismatch())?,row.try_get::<_,i64>(1).map_err(|_|mismatch())?))).collect::<Result<Vec<_>,StepError>>()?;
        let snapshot=cluster::inspect_source_without_template0(&server.socket,password,key.as_deref()).await.map(|mut snapshot|{
            snapshot.data.insert("template0".into(),template0);snapshot.collations=collations.collations;snapshot.largest_index_bytes=snapshot.largest_index_bytes.max(bytes);snapshot
        });
        let admitted=super::profile::admit(snapshot,&app.client,&ledger,&j.load().candidate,packaged).await?;
        super::profile::require_complete(&admitted)?;
        let socket=server.socket.clone();let output=logs.clone();
        let schema=tokio::task::spawn_blocking(move||{
            let mut schema=diagnostic_file(&output.join("schema.sql"))?;let error=diagnostic_file(&output.join("schema-dump.stderr"))?;
            super::rehearsal_io::dump(&socket,&app_name,&mut schema,&error)?;Ok(schema)
        }).await.map_err(|_|mismatch())??;
        Ok((admitted,schema))
    }.await;
    tokio::task::spawn_blocking(move||server.stop().map_err(map)).await.map_err(|_|mismatch())??;
    result
}
fn diagnostic_file(path:&std::path::Path)->Result<std::fs::File,StepError>{
    use std::os::unix::fs::OpenOptionsExt;
    let file=std::fs::OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path).map_err(super::backup_format::io_error)?;
    postgres::own(path).map_err(map)?;Ok(file)
}
/// Complete the existing P2 Intent: schema-only rehearsal -> stop/retire -> real check -> Done.
pub(crate) async fn finish(roots:Roots,locks:std::sync::Arc<StorageLocks>,j:Journal,source:Admitted,mut schema:std::fs::File,config:&crate::config::RuntimeConfig,name:&str)->Result<(Journal,Admitted),StepError>{
    let r=roots.clone();let l=locks.clone();let log_name=name.to_owned();
    let (mut j,source,control,server)=tokio::task::spawn_blocking(move||{
        let mut j=j;
        if !l.holds_d()||!l.matches_roots(&r.db,&r.app)||!j.belongs_to(&r.app)||j.load().candidate!=source.candidate||j.load().records.iter().any(|record|record.step==Step::P3Convert){return Err(mismatch());}
        let current=Journal::open(&r.app,&l)?;if serde_json::to_value(current.load()).map_err(|_|mismatch())?!=serde_json::to_value(j.load()).map_err(|_|mismatch())?{return Err(mismatch());}
        super::replacement::precommit(&r.app,&j)?;
        super::profile::require_complete(&source)?;
        let logs=super::diagnostics::owned_path(&r.db,&j,&log_name,true)?;
        let reader=super::backup::open_verified_manifest(&r,&j)?;let header=&reader.header;
        let(app_bytes,app_files)=super::backup::app_usage(&r,&l)?;
        let usage=super::space::Usage{db_bytes:reader.summary.data_bytes,app_bytes,largest_index_bytes:source.snapshot.largest_index_bytes,manifest_bytes:reader.summary.bytes,db_files:reader.summary.entries.checked_add(1).ok_or_else(super::backup_format::too_large)?,app_files};
        let assessment=super::space::assess(&r,&l,&j,usage,super::space::Phase::P2{backup_bytes:reader.summary.data_bytes})?;
        j.record(Step::P2Preflight,StepState::Intent,Some(serde_json::json!({"logName":log_name,"space":assessment.space})))?;
        super::runner::publish_step(&r,&j,Step::P2Preflight,0,usage.db_bytes,Some(assessment.space.clone()),false);assessment.require()?;
        let data=match header.source_location{super::backup_format::SourceLocation::Root=>r.db.clone(),super::backup_format::SourceLocation::Pgdata=>r.db.join("pgdata")};
        super::source::require_idle(&r,&l,&data)?;let control=postgres::read_control(15,&data).map_err(map)?;control.require_cold().map_err(map)?;
        if control.system_identifier!=header.system_identifier{return Err(mismatch());}
        let work=r.db.join(".nwui-auto-work").join(&j.load().attempt_id);match std::fs::symlink_metadata(&work){Err(e)if e.kind()==std::io::ErrorKind::NotFound=>postgres::directory(&work).map_err(map)?,Ok(_)=>{super::target::directory(&work)?;},Err(e)=>return Err(super::backup_format::io_error(e))}
        let data=work.join("data");let logs=logs.join("rehearsal");postgres::directory(&logs).map_err(map)?;
        postgres::directory(&data).map_err(map)?;let command=migration::init_target_command(&data,&control,&source.snapshot).map_err(map)?;postgres::capture(command,&logs,Duration::from_secs(300)).map_err(map)?;
        let server=postgres::Private::start(17,&data,&logs).map_err(map)?;Ok((j,source,control,server))
    }).await.map_err(|_|mismatch())??;
    let result=async{
        let app_name=source.snapshot.application().map_err(map)?;
        let credentials:tokio_postgres::Config=config.database_url.parse().map_err(|_|mismatch())?;
        let password=std::str::from_utf8(credentials.get_password().ok_or_else(mismatch)?).map_err(|_|mismatch())?;
        if credentials.get_user()!=Some("nginx"){return Err(mismatch());}
        let root=crate::db_upgrade::inspect::connect(&server.socket,"postgres","postgres",None).await.map_err(map)?;
        root.client.batch_execute("SET default_transaction_read_only=off").await.map_err(|_|mismatch())?;
        root.client.batch_execute("SET standard_conforming_strings=on").await.map_err(|_|mismatch())?;
        root.client.batch_execute(&format!("CREATE ROLE nginx LOGIN PASSWORD '{}'",password.replace('\'',"''"))).await.map_err(|_|mismatch())?;
        root.client.batch_execute(&format!("CREATE DATABASE \"{}\" OWNER nginx",app_name)).await.map_err(|_|mismatch())?;drop(root);
        let socket=server.socket.clone();let db=app_name.clone();let logs=super::diagnostics::owned_path(&roots.db,&j,name,true)?;
        tokio::task::spawn_blocking(move||{let error=diagnostic_file(&logs.join("schema-restore.stderr"))?;super::rehearsal_io::restore(&socket,&db,&mut schema,&error)}).await.map_err(|_|mismatch())??;
        let pg=super::PrivatePg{socket_dir:server.socket.clone(),port:50432,dbname:app_name.clone(),role:"nginx".into(),major:17};
        let(_,ledger)=super::rehearsal::seed_and_migrate(&pg,config,&j.load().candidate,&source.ledger).await?;
        let app=crate::db_upgrade::inspect::connect(&server.socket,&app_name,"postgres",None).await.map_err(map)?;
        super::profile::verify_rehearsal(&j,&source,&app.client,&ledger).await
    }.await;
    tokio::task::spawn_blocking(move||server.stop().map_err(map)).await.map_err(|_|mismatch())??;
    let checked=result?;let r=roots.clone();let l=locks.clone();let log_name=name.to_owned();
    tokio::task::spawn_blocking(move||{
        super::diagnostics::retire_check(&r,&l,&j,&log_name)?;
        let reader=super::backup::open_verified_manifest(&r,&j)?;let original=match reader.header.source_location{super::backup_format::SourceLocation::Root=>r.db.clone(),super::backup_format::SourceLocation::Pgdata=>r.db.join("pgdata")};
        let work=r.db.join(".nwui-auto-work").join(&j.load().attempt_id);let logs=super::diagnostics::owned_path(&r.db,&j,&log_name,true)?;
        let probe=super::linkprobe::probe(&original,&work,&logs)?;
        if let Some(reason)=probe.copy_reason{eprintln!("[upgrade] hard-link probe selected copy: {reason}");}
        run_started(&r,&l,&mut j,&source,&control,probe.mode,&log_name)?;
        super::profile::finish_rehearsal(&r,&l,&mut j,&source,checked,probe.mode)?;
        super::runner::publish_step(&r,&j,Step::P2Preflight,reader.summary.data_bytes,reader.summary.data_bytes,None,false);Ok((j,source))
    }).await.map_err(|_|mismatch())?
}
/// Await on a blocking thread under custody, after complete source inspection and its private PG15 stop.
pub(crate) fn run_started(roots:&Roots,locks:&StorageLocks,j:&mut Journal,source:&Admitted,control:&Control,mode:UpgradeMode,name:&str)->Result<(),StepError>{
    super::profile::require_complete(source)?;
    if source.candidate!=j.load().candidate || !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app){return Err(mismatch());}
    let current=Journal::open(&roots.app,locks)?;
    if serde_json::to_value(current.load()).map_err(|_|mismatch())?!=serde_json::to_value(j.load()).map_err(|_|mismatch())?{return Err(mismatch());}
    super::replacement::precommit(&roots.app,j)?;
    let header=super::backup::verify_backup(roots,j)?;control.require_cold().map_err(map)?;
    if control.system_identifier!=header.system_identifier || j.load().records.iter().any(|r|r.step==Step::P3Convert){return Err(mismatch());}
    let logs=super::diagnostics::owned_path(&roots.db,j,name,true)?;
    let work=roots.db.join(".nwui-auto-work").join(&j.load().attempt_id);let data=work.join("data");
    for path in [&data,&work.join("target")] {match std::fs::symlink_metadata(path){Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},_=>return Err(mismatch())}}
    let result=(||{
        match std::fs::symlink_metadata(&work){Err(e)if e.kind()==std::io::ErrorKind::NotFound=>postgres::directory(&work).map_err(map)?,Ok(_)=>{super::target::directory(&work)?;},Err(_)=>return Err(mismatch())}
        let command=migration::init_target_command(&data,control,&source.snapshot).map_err(map)?;
        postgres::directory(&data).map_err(map)?;postgres::capture(command,&logs,Duration::from_secs(300)).map_err(map)?;
        migration::upgrade_auto_at(roots,locks,j,name,mode,true).map_err(map)?;
        super::diagnostics::retire_check(roots,locks,j,name)
    })();
    if let Err(error)=&result{j.record_retry_failure(Step::P2Preflight,error.code,chrono::Utc::now())?;}result
}
#[cfg(test)]
fn run_locked(roots:&Roots,locks:&StorageLocks,j:&mut Journal,source:&Admitted,control:&Control,mode:UpgradeMode)->Result<(),StepError>{
    super::profile::require_complete(source)?;let name=begin_locked(roots,locks,j,control)?;run_started(roots,locks,j,source,control,mode,&name)
}

#[cfg(test)]
mod tests{
    use super::*;
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    #[ignore="native PG15+PG17: admitted released schema, real check and scratch retirement"]
    async fn native_check_retires_only_scratch_before_profile_done_and_retains_report(){
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        use crate::db_upgrade::{files,inspect,cluster};use super::super::{Branch,backup,backup_format::SourceLocation,journal::Candidate};
        use std::{fs,path::Path,os::unix::fs::PermissionsExt};
        for (mode,bad_collation,bad_rehearsal) in [(UpgradeMode::Link,false,false),(UpgradeMode::Copy,false,false),(UpgradeMode::Copy,true,false),(UpgradeMode::Copy,false,true)]{
            let t=tempfile::tempdir().unwrap();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o711)).unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:crate::db_upgrade::engine::architecture().unwrap().into()};postgres::directory(&roots.db).unwrap();files::private_dir(&roots.app).unwrap();let logs=t.path().join("fixture");postgres::directory(&logs).unwrap();let mut init=postgres::command(15,"initdb").unwrap();init.arg("-D").arg(&roots.db).args(["--no-locale","--encoding=UTF8","--auth=trust"]);postgres::capture(init,&logs,Duration::from_secs(300)).unwrap();
            let app_name=if mode==UpgradeMode::Copy&&!bad_collation&&!bad_rehearsal{"nginx_control"}else{"nginx_web_ui"};
            let server=postgres::Private::start(15,&roots.db,&logs).unwrap();let root=inspect::connect(&server.socket,"postgres","postgres",None).await.unwrap();root.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();root.client.batch_execute("CREATE ROLE nginx LOGIN PASSWORD 'fixture-only'").await.unwrap();root.client.batch_execute(&format!("CREATE DATABASE {app_name} OWNER nginx")).await.unwrap();drop(root);
            let directory=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");let migrations=crate::migrations::read_migrations(&directory).await.unwrap();let candidate=Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:super::super::journal::migrations_sha256(&migrations)};
            let app=inspect::connect(&server.socket,app_name,"postgres",None).await.unwrap();app.client.batch_execute("SET default_transaction_read_only=off; SET ROLE nginx; SET search_path=public").await.unwrap();for m in migrations.iter().take(28){for sql in &m.statements{app.client.batch_execute(sql).await.unwrap();}}app.client.batch_execute("CREATE SCHEMA drizzle; CREATE TABLE drizzle.__drizzle_migrations(id serial primary key,hash text NOT NULL,created_at bigint)").await.unwrap();for m in migrations.iter().take(28){app.client.execute("INSERT INTO drizzle.__drizzle_migrations(hash,created_at) VALUES($1,$2)",&[&m.hash,&m.timestamp]).await.unwrap();}app.client.batch_execute("RESET ROLE; SET search_path=pg_catalog; SET default_transaction_read_only=on").await.unwrap();
            // Source construction uses the trusted fixture inspector before the cold P1 boundary.
            let partial=cluster::inspect_source_without_template0(&server.socket,"fixture-only",None).await.unwrap();assert!(!partial.data.contains_key("template0"));assert!(!partial.collations.contains_key("template0"));assert!(partial.databases.iter().any(|db|db.name=="template0"));assert!(!app.client.query_one("SELECT datallowconn FROM pg_catalog.pg_database WHERE datname='template0'",&[]).await.unwrap().get::<_,bool>(0));let mut snapshot=cluster::inspect_source(&server.socket,"fixture-only",None).await.unwrap();snapshot.collations=cluster::collect_auto_collations(&server.socket,false).await.unwrap().collations;for(name,data)in &partial.data{assert_eq!(snapshot.data.get(name),Some(data));}assert_eq!(partial.roles,snapshot.roles);assert_eq!(partial.databases,snapshot.databases);let ledger=migrations.iter().take(28).map(|m|(m.hash.clone(),m.timestamp)).collect::<Vec<_>>();let mut admitted=super::super::profile::admit(Ok(snapshot),&app.client,&ledger,&candidate,&migrations).await.unwrap();
            if bad_collation{app.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();app.client.batch_execute("CREATE COLLATION public.p2_icu(provider=icu,locale='en-US'); CREATE TABLE public.p2_unindexed(label text COLLATE public.p2_icu)").await.unwrap();}
            drop(app);server.stop().unwrap();
            files::private_dir(&roots.app.join("secrets")).unwrap();for field in ["auth_secret","db_password"]{fs::write(roots.app.join("secrets").join(field),b"fixture-only").unwrap();fs::set_permissions(roots.app.join("secrets").join(field),fs::Permissions::from_mode(0o600)).unwrap();}
            let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j=Journal::begin(&roots.app,&locks,Branch::Upgrade,candidate).unwrap();let control=postgres::read_control(15,&roots.db).unwrap();let names=fs::read_dir(&roots.db).unwrap().map(|e|e.unwrap().file_name().into_string().unwrap()).filter(|n|super::super::restore::set_aside_allowed(n,true)).collect();let legacy=super::super::preflight::Legacy15{location:SourceLocation::Root,control,original_names:names};let(prepared,assessment)=super::super::preflight::assess_backup(&roots,&locks,&j,&legacy).unwrap();assessment.require().unwrap();let control=&legacy.control;assert!(!roots.db.join(".nwui-pg15-backup").exists());super::super::reserves::ensure(&roots,&locks,&j).unwrap();backup::create_verified(&roots,&locks,&mut j,&prepared).unwrap();
            let template0=admitted.snapshot.data.remove("template0").unwrap();let before=serde_json::to_value(j.load()).unwrap();assert!(run_locked(&roots,&locks,&mut j,&admitted,&control,mode).is_err());assert_eq!(serde_json::to_value(j.load()).unwrap(),before);assert!(!roots.db.join(".nwui-auto-work/logs").exists());admitted.snapshot.data.insert("template0".into(),template0);let collations=admitted.snapshot.collations.remove("template0").unwrap();assert!(run_locked(&roots,&locks,&mut j,&admitted,&control,mode).is_err());assert_eq!(serde_json::to_value(j.load()).unwrap(),before);assert!(!roots.db.join(".nwui-auto-work/logs").exists());admitted.snapshot.collations.insert("template0".into(),collations);
            let expected=serde_json::to_value(&admitted.snapshot).unwrap();
            let locks=std::sync::Arc::new(locks);let name=begin_locked(&roots,&locks,&mut j,&control).unwrap();let frame=serde_json::to_value(j.load()).unwrap();
            let inspected=inspect_source(roots.clone(),locks.clone(),&j,&name,"fixture-only",None,&migrations).await;
            if bad_collation{let error=inspected.err().unwrap();assert_eq!(error.code,"collation_unsupported");assert_eq!(serde_json::to_value(j.load()).unwrap(),frame);let logs=super::super::diagnostics::owned_path(&roots.db,&j,&name,true).unwrap();assert!(!logs.join("profile").exists(),"collation refusal must precede strict schema inspection");super::super::backup::verify_backup(&roots,&j).unwrap();continue;}
            let (admitted,schema)=inspected.unwrap();assert_eq!(serde_json::to_value(&admitted.snapshot).unwrap(),expected);assert_eq!(serde_json::to_value(j.load()).unwrap(),frame);
            assert!(begin_locked(&roots,&locks,&mut j,&control).is_err());assert_eq!(serde_json::to_value(j.load()).unwrap(),frame);
            let mut config=crate::config::RuntimeConfig::read(|n|match n{"AUTH_SECRET"=>Some("fixture-only".into()),"DATABASE_URL"=>Some("postgres://nginx:fixture-only@must-not-contact.invalid/nginx_web_ui".into()),_=>None}).unwrap();config.migrations_dir=directory;
            if bad_rehearsal{config.migrations_dir=t.path().join("missing-packaged-migrations");}
            let finished=finish(roots.clone(),locks.clone(),j,admitted,schema,&config,&name).await;
            if bad_rehearsal{
                let error=finished.err().unwrap();assert_eq!(error.code,"rehearsal_failed");let j=Journal::open(&roots.app,&locks).unwrap();assert!(!j.load().records.iter().any(|r|r.step==Step::P2Preflight&&r.state==StepState::Done));
                let data=roots.db.join(".nwui-auto-work").join(&j.load().attempt_id).join("data");assert!(data.is_dir());assert!(!data.join("postmaster.pid").exists(),"failed rehearsal must reap PG17");
                let current=j.load().candidate.clone();let recovered=super::super::restore::recover_with_context(roots.clone(),locks.clone(),j,Step::P2Preflight,error.code,Some("a".repeat(64)),current,None).await.unwrap();
                assert_eq!(recovered.load().outcome,super::super::Outcome::Restored);assert!(!data.parent().unwrap().exists());let logs=super::super::diagnostics::path(&roots.db,&recovered.load().attempt_id,&name).unwrap();assert!(logs.join("schema.sql").is_file());assert!(logs.join("rehearsal").is_dir());super::super::backup::verify_restored(&roots,&recovered).unwrap();continue;
            }
            let (mut j,admitted)=finished.unwrap();super::super::profile::proof(&j).unwrap();assert_eq!(super::super::profile::mode(&j).unwrap(),UpgradeMode::Link);
            assert_eq!(j.load().records.last().unwrap().state,StepState::Done);let record=j.load().records.iter().rev().find(|r|r.step==Step::P2Preflight&&r.state==StepState::Intent).unwrap();let name=record.data.as_ref().unwrap().get("logName").unwrap().as_str().unwrap();let report=super::super::diagnostics::path(&roots.db,&j.load().attempt_id,name).unwrap();assert!(report.join("pg_upgrade_output.d").is_dir());let work=roots.db.join(".nwui-auto-work").join(&j.load().attempt_id);assert!(!work.join("data").exists());assert!(!work.join("init").exists());assert_eq!(fs::read_to_string(roots.db.join("PG_VERSION")).unwrap().trim(),"15");assert!(roots.db.join(".nwui-pg15-backup").join(&j.load().attempt_id).join("PG_VERSION").is_file());
            if mode==UpgradeMode::Link&&!crate::nonroot::PRIVILEGE_SPLIT{
                // Real pre-commit P4 insertion and R-restore, not an assembled-image boot claim.
                let converted=super::super::conversion::run_locked(&roots,&locks,&mut j,&control,&admitted.snapshot).unwrap();let current=j.load().candidate.clone();let ensured=super::super::key_write::ensure_key_locked(roots.clone(),locks.clone(),j,false).await.unwrap();j=ensured.journal;
                let selection=super::super::StorageSelection{version:1,epoch:j.load().records.last().unwrap().seq+1,outcome:super::super::Outcome::Upgrading,branch:Branch::Upgrade,pgdata:work.join("data"),backup_id:Some(j.load().attempt_id.clone()),schema_profile:super::super::profile::proof(&j).unwrap().profile_id,key_binding:super::super::KeyBinding::Verified{pair_id:uuid::Uuid::new_v4().to_string()}};
                let convert=j.load().records.iter().rev().find(|r|r.step==Step::P3Convert&&r.state==StepState::Done).unwrap().data.as_ref().unwrap().get("logName").unwrap().as_str().unwrap();let logs=super::super::diagnostics::path(&roots.db,&j.load().attempt_id,convert).unwrap().join("notice-boot");postgres::directory(&logs).unwrap();let target=postgres::Private::start(17,&selection.pgdata,&logs).unwrap();let pg=super::super::PrivatePg{socket_dir:target.socket.clone(),port:50432,dbname:"nginx_web_ui".into(),role:"nginx".into(),major:17};config.data_key_file=roots.app.join("secrets/data_key");config.target_dns.descriptor_path=t.path().join("notice-dns.json");
                super::super::target_boot::prepare(&roots,&locks,&mut j,&admitted,&current,&selection,&pg,&config,&ensured.key.secrets,super::super::boot::Inputs{admin_email:None,admin_password:None,backfill:None}).await.unwrap();
                let rows=super::super::boot::notices(&roots,&j).unwrap();super::super::boot::finish(&selection,&config,&ensured.key.secrets,&pg,&locks,&mut j,super::super::boot::Inputs{admin_email:None,admin_password:None,backfill:None},&rows,crate::publisher::boot::BootMode::Conversion).await.unwrap();
                let app=inspect::connect(&target.socket,"nginx_web_ui","postgres",None).await.unwrap();assert_eq!(app.client.query_one("SELECT count(*)::bigint FROM public.notifications WHERE type='system.db_upgrade_backup_kept'",&[]).await.unwrap().get::<_,i64>(0),1);drop(app);target.stop().unwrap();assert_eq!(postgres::read_control(17,&selection.pgdata).unwrap().system_identifier,converted.system_identifier);
                let recovered=super::super::restore::recover_with_context(roots.clone(),locks.clone(),j,Step::P5Publish,"interrupted",Some("a".repeat(64)),current,None).await.unwrap();assert_eq!(recovered.load().outcome,super::super::Outcome::Restored);super::super::backup::verify_restored(&roots,&recovered).unwrap();assert!(!roots.app.join("secrets/data_key").exists());assert!(super::super::boot::notices(&roots,&recovered).unwrap().is_empty());
                let logs=t.path().join("notice-restored");postgres::directory(&logs).unwrap();let restored=postgres::Private::start(15,&roots.db,&logs).unwrap();let app=inspect::connect(&restored.socket,app_name,"postgres",None).await.unwrap();assert_eq!(app.client.query_one("SELECT count(*)::bigint FROM public.notifications WHERE type='system.db_upgrade_backup_kept'",&[]).await.unwrap().get::<_,i64>(0),0);drop(app);restored.stop().unwrap();
            }
        }
    }
}
