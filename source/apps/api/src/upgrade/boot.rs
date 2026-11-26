//! Finite A.11 runner steps over private PG17, after K/agreement and (on upgrade) V1.
use super::{Branch,Journal,KeyBinding,PrivatePg,Step,StepError,StepState,StorageLocks,StorageSelection};
use crate::{config::RuntimeConfig,crypto::SecretBox,runtime_boot::{self,BootCtx}};

// No Debug/Serialize: startup admin credentials stay in memory only.
pub(crate) struct Inputs {pub admin_email:Option<String>,pub admin_password:Option<String>,pub backfill:Option<String>}
impl Inputs {
    pub(crate) fn startup()->Self {Self{admin_email:crate::nonroot::startup_value("ADMIN_EMAIL"),admin_password:crate::nonroot::envdir_value("ADMIN_PASSWORD"),backfill:crate::nonroot::startup_value("ENCRYPT_BACKFILL")}}
}
#[cfg(test)]
pub(crate) struct Prepared {pub privileges:crate::nonroot::Prepared,pub migrations_applied:usize}
fn invalid()->StepError {StepError{code:"key_unverified",message:"키와 데이터의 연결을 검증한 뒤 시작 준비를 진행해야 합니다.",detail:None}}
async fn step<T>(j:&mut Journal,id:Step,name:&str,work:impl std::future::Future<Output=Result<T,StepError>>)->Result<T,StepError> {
    let data=serde_json::json!({"bootStep":name});j.record(id,StepState::Intent,Some(data.clone()))?;
    match work.await {
        Ok(value)=>{j.record(id,StepState::Done,Some(data))?;Ok(value)},
        Err(error)=>{j.record_retry_failure(id,error.code,chrono::Utc::now())?;Err(error)},
    }
}
fn verified(selection:&StorageSelection,locks:&StorageLocks,j:&Journal)->Result<(),StepError> {
    if !locks.holds_a() || !locks.holds_d() || selection.branch!=j.load().branch
        || !matches!(&selection.key_binding,KeyBinding::Verified{pair_id} if super::types::canonical_generation(pair_id)) {return Err(invalid());}
    Ok(())
}
/// V2 runs after this returns and before finish creates or backfills application rows.
pub(crate) async fn migrate(selection:&StorageSelection,current_candidate:&super::journal::Candidate,config:&RuntimeConfig,secrets:&SecretBox,pg:&PrivatePg,locks:&StorageLocks,j:&mut Journal)->Result<usize,StepError> {
    verified(selection,locks,j)?;
    let pool=pg.pool(config)?;let ctx=BootCtx{config,secrets,pool:&pool};
    let id=if j.load().branch==Branch::Upgrade{Step::P4Target}else{Step::Y2};
    // A new boot may apply newer migrations while the journal retains its original attempt identity.
    let expected_migrations=&current_candidate.migrations_sha256;
    let migrations_applied=step(j,id,"run_migrations",async {
        let migrations=crate::migrations::read_migrations(&config.migrations_dir).await.map_err(|_|StepError{code:"migration_failed",message:"적용할 데이터 변경 이력을 읽지 못했습니다.",detail:Some("run_migrations: read migration journal".into())})?;
        if super::journal::migrations_sha256(&migrations)!=*expected_migrations {return Err(StepError{code:"verification_failed",message:"후보와 적용할 데이터 변경 이력이 다릅니다. 적용을 중단했습니다.",detail:None});}
        runtime_boot::run_migrations(&ctx).await
    }).await?;
    pool.close();
    Ok(migrations_applied)
}
/// Call only after the candidate catalog and data have passed V2 on the upgrade branch.
pub(super) async fn finish(selection:&StorageSelection,config:&RuntimeConfig,secrets:&SecretBox,pg:&PrivatePg,locks:&StorageLocks,j:&mut Journal,inputs:Inputs,notices:&Notices,mode:crate::publisher::boot::BootMode)->Result<crate::nonroot::Prepared,StepError> {
    verified(selection,locks,j)?;
    let pool=pg.pool(config)?;let ctx=BootCtx{config,secrets,pool:&pool};
    let id=if j.load().branch==Branch::Upgrade{Step::P4Target}else{Step::Y2};
    step(j,id,"preflight_api_port_rows",runtime_boot::preflight_api_port_rows(&ctx)).await?;
    step(j,id,"ensure_instance",runtime_boot::ensure_instance(&ctx)).await?;
    step(j,id,"seed_admin",runtime_boot::seed_admin(&ctx,inputs.admin_email.as_deref(),inputs.admin_password.as_deref())).await?;
    step(j,id,"dns_preflight",runtime_boot::dns_preflight(&ctx)).await?;
    step(j,id,"backfill",runtime_boot::backfill(&ctx,inputs.backfill.as_deref())).await?;
    record_notices(&pool,j,id,notices).await?;
    storage_notice(&pool).await;
    pool.close();
    let event=serde_json::json!({"bootStep":"prepare_privileges"});j.record(Step::P5Publish,StepState::Intent,Some(event.clone()))?;
    let privileges=match super::runner::prepare_privileges(selection,locks,pg,j,mode).await {
        Ok(prepared)=>{j.record(Step::P5Publish,StepState::Done,Some(event))?;prepared},
        Err(error)=>{j.record_retry_failure(Step::P5Publish,error.code,chrono::Utc::now())?;return Err(error)},
    };
    Ok(privileges)
}

async fn storage_notice(pool:&deadpool_postgres::Pool){
    for warning in super::status::storage_warnings(){
        let result=async{
            let mut client=pool.get().await.map_err(|_|notice_failed())?;
            let tx=client.transaction().await.map_err(|_|notice_failed())?;
            tx.batch_execute("SET LOCAL statement_timeout='2s'; SET LOCAL lock_timeout='1s'").await.map_err(|_|notice_failed())?;
            super::status::insert_notification(&*tx,&super::status::storage_notification(&warning)).await?;
            tx.commit().await.map_err(|_|notice_failed())
        }.await;
        storage_notice_result(result);
    }
}
fn storage_notice_result(result:Result<(),StepError>){if result.is_err(){eprintln!("[storage] desktop_shared_folder_notice_pending");}}
fn notice_failed()->StepError{StepError{code:"io_error",message:"업그레이드 알림을 저장하지 못했습니다.",detail:None}}
pub(super) struct Notices {
    pub(super) rows: Vec<super::status::UpgradeNotification>,
    retention_known: bool,
}

const STALE_BACKUP_NOTICES: &str = "FROM notifications WHERE type = $1
    AND ($2::text IS NULL OR data->>'backupId' IS DISTINCT FROM $2 OR dedupe_key IS DISTINCT FROM $3)";

pub(super) async fn record_notices(pool: &deadpool_postgres::Pool, j: &mut Journal, id: Step, notices: &Notices) -> Result<(), StepError> {
    if notices.rows.is_empty() {
        if !notices.retention_known {
            return Ok(());
        }
        let stale = async {
            let client = pool.get().await.ok()?;
            // With no notice to publish, both retained-backup parameters are NULL.
            let empty: Option<&str> = None;
            let query = format!("SELECT EXISTS (SELECT 1 {STALE_BACKUP_NOTICES})");
            let row = client.query_one(query.as_str(), &[&super::status::BACKUP_KEPT_KIND, &empty, &empty]).await.ok()?;
            row.try_get::<_, bool>(0).ok()
        }.await;
        match stale {
            Some(true) => {}
            Some(false) => return Ok(()),
            None => {
                eprintln!("[upgrade] retained backup notice check unavailable");
                return Ok(());
            }
        }
    }
    step(j, id, "upgrade_notifications", async {
        let mut client = pool.get().await.map_err(|_| notice_failed())?;
        let transaction = client.transaction().await.map_err(|_| notice_failed())?;
        publish_notices(&*transaction, notices).await?;
        transaction.commit().await.map_err(|_| notice_failed())
    }).await
}

async fn publish_notices(client: &impl tokio_postgres::GenericClient, notices: &Notices) -> Result<(), StepError> {
    for notice in &notices.rows {
        super::status::insert_notification(client, notice).await?;
    }
    if notices.retention_known {
        let backup = notices.rows.iter().find(|notice| notice.kind == super::status::BACKUP_KEPT_KIND);
        let id = backup.map(|notice| notice.data["backupId"].as_str().ok_or_else(notice_failed)).transpose()?;
        let key = backup.map(|notice| notice.dedupe_key.as_str());
        let query = format!("DELETE {STALE_BACKUP_NOTICES}");
        client.execute(
            query.as_str(),
            &[&super::status::BACKUP_KEPT_KIND, &id, &key],
        ).await.map_err(|_| notice_failed())?;
    }
    Ok(())
}

/// Capture durable notice inputs before this boot's row steps; insert only in the target BootCtx.
pub(super) fn notices(roots:&crate::db_upgrade::layout::Roots,j:&Journal)->Result<Notices,StepError>{
    let mut rows=Vec::new();let attempt=&j.load().attempt_id;
    let consumed=matches!(j.load().outcome,super::Outcome::Restored|super::Outcome::RestoreIncomplete)||j.load().records.iter().any(|r|matches!(r.step,Step::CmdRemoveBackup|Step::CmdRestorePg15)&&r.state==StepState::Done&&r.data.as_ref().and_then(|d|d.get("backupId")).and_then(|v|v.as_str())==Some(attempt.as_str()));
    let retained=(|| ->Result<(),StepError>{
    if !consumed{if let Some(record)=j.load().records.iter().rev().find(|r|r.step==Step::P1Backup&&r.state==StepState::Done){
        let location=roots.db.join(".nwui-pg15-backup").join(attempt);
        match std::fs::symlink_metadata(&location){
            Ok(_)=>{super::target::directory(&location)?;let bytes=record.data.as_ref().and_then(|d|d.get("sourceBytes")).and_then(|v|v.as_u64()).ok_or_else(notice_failed)?;
                let backup=super::types::RetainedCopy{id:attempt.clone(),bytes,remove_command:format!("nwui-db-upgrade remove-pg15-backup --confirm {attempt}")};rows.push(super::status::backup_kept_notification(&backup,&location.display().to_string()));},
            Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},Err(_)=>return Err(notice_failed()),
        }
    }}
    Ok(())})();
    // A failed inspection cannot prove the backup is gone.
    let retention_known=retained.is_ok();
    if let Err(error)=retained{eprintln!("[upgrade] retained backup notice unavailable: {}",error.code);}
    if j.load().outcome==super::Outcome::ActivationFailed{let failure=super::journal::last_attempt_failure(&j.load().records).ok_or_else(notice_failed)?;rows.push(super::status::activation_failed_notification(attempt,&failure.code));}
    Ok(Notices { rows, retention_known })
}

/// Fresh/ready have no conversion V2 boundary; upgrades call migrate and finish separately.
#[cfg(test)]
pub(crate) async fn prepare(selection:&StorageSelection,current_candidate:&super::journal::Candidate,config:&RuntimeConfig,secrets:&SecretBox,pg:&PrivatePg,locks:&StorageLocks,j:&mut Journal,inputs:Inputs)->Result<Prepared,StepError> {
    if selection.branch==Branch::Upgrade {return Err(StepError{code:"verification_failed",message:"변환한 데이터 검증을 완료한 뒤 시작 준비를 진행해야 합니다.",detail:Some("upgrade_requires_v2_boundary".into())});}
    let migrations_applied=migrate(selection,current_candidate,config,secrets,pg,locks,j).await?;
    let privileges=finish(selection,config,secrets,pg,locks,j,inputs,&Notices { rows: Vec::new(), retention_known: true },crate::publisher::boot::BootMode::Conversion).await?;
    Ok(Prepared{privileges,migrations_applied})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs,path::PathBuf,os::unix::fs::PermissionsExt};
    fn fixture()->(tempfile::TempDir,StorageLocks,Journal,RuntimeConfig,StorageSelection,PrivatePg) {
        let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");
        for p in [&db,&app] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let locks=StorageLocks::acquire(&db,&app).unwrap();let j=Journal::begin(&app,&locks,Branch::Ready,super::super::journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();
        let mut config=RuntimeConfig::read(|name|match name{"DATABASE_URL"=>Some("postgres://nginx:fixture-only@must-not-contact.invalid:6543/foreign".into()),"AUTH_SECRET"=>Some("fixture-only".into()),_=>None}).unwrap();
        config.migrations_dir=t.path().join("migrations");config.target_dns.descriptor_path=t.path().join("target-dns.json");
        let selection=StorageSelection{version:1,epoch:1,outcome:super::super::Outcome::Pg17,branch:Branch::Ready,pgdata:db.join("17/data"),backup_id:None,schema_profile:"none".into(),key_binding:KeyBinding::Verified{pair_id:uuid::Uuid::new_v4().to_string()}};
        let pg=PrivatePg{socket_dir:t.path().join("absent-socket"),port:50433,dbname:"nginx_web_ui".into(),role:"nginx".into(),major:17};
        (t,locks,j,config,selection,pg)
    }
    fn inputs()->Inputs {Inputs{admin_email:None,admin_password:None,backfill:None}}
    #[test]
    fn notice_inputs_use_journal_authority_and_never_resurrect_a_removed_backup(){
        let t=tempfile::tempdir().unwrap();let roots=crate::db_upgrade::layout::Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for path in [&roots.db,&roots.app]{fs::create_dir(path).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o700)).unwrap();}let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j=Journal::begin(&roots.app,&locks,Branch::Upgrade,super::super::journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();j.ensure_app_pool().unwrap();j.record(Step::P1Backup,StepState::Done,Some(serde_json::json!({"sourceBytes":123}))).unwrap();assert!(notices(&roots,&j).unwrap().rows.is_empty());
        let parent=roots.db.join(".nwui-pg15-backup");fs::create_dir(&parent).unwrap();fs::set_permissions(&parent,fs::Permissions::from_mode(0o700)).unwrap();let backup=parent.join(&j.load().attempt_id);fs::create_dir(&backup).unwrap();fs::set_permissions(&backup,fs::Permissions::from_mode(0o700)).unwrap();let rows=notices(&roots,&j).unwrap().rows;assert_eq!(rows.len(),1);assert_eq!(rows[0].data["bytes"],123);assert!(rows[0].body.contains(&j.load().attempt_id));
        let data=Some(serde_json::json!({"backupId":j.load().attempt_id}));j.record(Step::CmdRemoveBackup,StepState::Intent,data.clone()).unwrap();j.record(Step::CmdRemoveBackup,StepState::Done,data).unwrap();assert!(backup.exists());assert!(notices(&roots,&j).unwrap().rows.is_empty());j.fail_attempt(Step::S4Activate,&StepError{code:"activation_timeout",message:"fixture",detail:None},super::super::Outcome::ActivationFailed,chrono::Utc::now()).unwrap();let rows=notices(&roots,&j).unwrap().rows;assert_eq!(rows.len(),1);assert_eq!(rows[0].kind,"system.db_upgrade_activation_failed");
    }
    #[tokio::test]
    async fn w1_boot_upgrade_cannot_bypass_the_v2_boundary_with_the_composed_ready_path() {
        let (t,locks,mut j,config,mut selection,pg)=fixture();selection.branch=Branch::Upgrade;
        let before=fs::read(t.path().join("app/.nwui-auto/journal-v1.json")).unwrap();
        let error=prepare(&selection,&j.load().candidate.clone(),&config,&SecretBox::from_key([1;32]),&pg,&locks,&mut j,inputs()).await.err().unwrap();
        assert_eq!(error.detail.as_deref(),Some("upgrade_requires_v2_boundary"));assert_eq!(fs::read(t.path().join("app/.nwui-auto/journal-v1.json")).unwrap(),before);
    }
    #[tokio::test]
    async fn w1_boot_rejects_unverified_binding_before_journal_or_database_work() {
        let (t,locks,mut j,config,mut selection,pg)=fixture();let before=fs::read(t.path().join("app/.nwui-auto/journal-v1.json")).unwrap();selection.key_binding=KeyBinding::Unverified;
        assert_eq!(prepare(&selection,&j.load().candidate.clone(),&config,&SecretBox::from_key([1;32]),&pg,&locks,&mut j,inputs()).await.err().unwrap().code,"key_unverified");
        assert_eq!(fs::read(t.path().join("app/.nwui-auto/journal-v1.json")).unwrap(),before);assert!(!config.target_dns.descriptor_path.exists());
    }
    #[tokio::test]
    async fn w1_boot_missing_or_different_packaged_set_fails_before_database_steps() {
        for missing in [true,false] {let (t,locks,mut j,config,selection,pg)=fixture();
            if !missing {fs::create_dir_all(config.migrations_dir.join("meta")).unwrap();fs::write(config.migrations_dir.join("meta/_journal.json"),br#"{"version":"7","dialect":"postgresql","entries":[]}"#).unwrap();}
            let error=prepare(&selection,&j.load().candidate.clone(),&config,&SecretBox::from_key([1;32]),&pg,&locks,&mut j,inputs()).await.err().unwrap();assert_eq!(error.code,if missing{"migration_failed"}else{"verification_failed"});
            let saved=Journal::open(&t.path().join("app"),&locks).unwrap();let last=saved.load().records.last().unwrap();assert_eq!(last.step,Step::Y2);assert_eq!(last.state,StepState::Failed{code:error.code.into()});
            assert_eq!(saved.load().outcome,super::super::Outcome::Upgrading);assert!(!config.target_dns.descriptor_path.exists());assert_eq!(saved.load().records.len(),2);
        }
    }
    #[tokio::test]
    async fn w1_boot_new_candidate_migrations_do_not_reuse_the_old_attempt_hash() {
        let (_t,locks,mut j,config,selection,pg)=fixture();let old=j.load().candidate.clone();
        fs::create_dir_all(config.migrations_dir.join("meta")).unwrap();fs::write(config.migrations_dir.join("meta/_journal.json"),br#"{"version":"7","dialect":"postgresql","entries":[]}"#).unwrap();
        let mut current=old.clone();current.migrations_sha256=super::super::journal::migrations_sha256(&crate::migrations::read_migrations(&config.migrations_dir).await.unwrap());assert_ne!(current.migrations_sha256,old.migrations_sha256);
        let error=prepare(&selection,&current,&config,&SecretBox::from_key([1;32]),&pg,&locks,&mut j,inputs()).await.err().unwrap();
        assert_eq!(error.code,"migration_failed");assert!(error.detail.unwrap().contains("run_migrations"));assert_eq!(j.load().candidate,old);assert!(!config.target_dns.descriptor_path.exists());
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    #[ignore="native PG17: V2 non-application inspection boundary"]
    async fn w1_boot_native_v2_non_application_inspection_skips_app_objects_but_refuses_other_database_drift() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        use crate::db_upgrade::{fresh,files,layout::Roots,postgres,cluster,inspect};
        let t=tempfile::tempdir().unwrap();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o711)).unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:crate::db_upgrade::paths::architecture().unwrap().into()};for path in [&roots.db,&roots.app]{files::private_dir(path).unwrap();}let env=t.path().join("env");files::private_dir(&env).unwrap();let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();
        fresh::secrets_locked(&locks,&roots,&fresh::Inputs{auth_secret:Some("fixture-only".into()),admin_password:None},&env).unwrap();fresh::provision_locked(&locks,&roots).await.unwrap();let password=fs::read_to_string(roots.app.join("secrets/db_password")).unwrap();let work=t.path().join("observe");postgres::directory(&work).unwrap();let server=postgres::Private::start(17,&roots.db.join("17/data"),&work).unwrap();
        let app=inspect::connect(&server.socket,"nginx_web_ui","postgres",None).await.unwrap();app.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();app.client.batch_execute("CREATE FUNCTION public.profile_owned_probe() RETURNS integer LANGUAGE sql AS $$SELECT 1$$").await.unwrap();
        let snapshot=cluster::inspect_non_application(&server.socket,&password,None).await.unwrap();assert!(snapshot.databases.iter().any(|d|d.name=="nginx_web_ui"));assert!(!snapshot.data.contains_key("nginx_web_ui"));assert!(snapshot.data.contains_key("postgres") && snapshot.data.contains_key("template0") && snapshot.data.contains_key("template1"));
        assert_eq!(cluster::inspect_cluster(&server.socket,&password,None,false).await.unwrap_err().code,"unsupported_layout");
        let other=inspect::connect(&server.socket,"postgres","postgres",None).await.unwrap();other.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();other.client.batch_execute("CREATE TABLE public.unexpected(value integer)").await.unwrap();assert_eq!(cluster::inspect_non_application(&server.socket,&password,None).await.unwrap_err().code,"unsupported_layout");
        assert!(app.client.query_one("SELECT to_regprocedure('public.profile_owned_probe()') IS NOT NULL",&[]).await.unwrap().get::<_,bool>(0));drop(app);drop(other);server.stop().unwrap();
    }
    #[tokio::test]
    #[ignore="native PG17 I1: disposable NWUI_N2_PRIVATE_DATABASE_URL on a Unix socket"]
    async fn w1_boot_native_private_steps_are_ordered_idempotent_and_stop_before_admin_on_port_conflict() {
        if std::env::var_os("NWUI_N2_PRIVATE_DATABASE_URL").is_none(){return;}
        if crate::nonroot::PRIVILEGE_SPLIT {eprintln!("I1-only native boot check is not applicable after the privilege split");return;}
        let url=std::env::var("NWUI_N2_PRIVATE_DATABASE_URL").expect("native private fixture URL");let base:tokio_postgres::Config=url.parse().unwrap();
        let socket=match base.get_hosts(){[tokio_postgres::config::Host::Unix(path)]=>path.clone(),_=>panic!("private fixture must have exactly one Unix socket")};
        let (admin,driver)=base.connect(tokio_postgres::NoTls).await.unwrap();tokio::spawn(driver);let version:i32=admin.query_one("SELECT current_setting('server_version_num')::int",&[]).await.unwrap().get(0);assert!((170000..180000).contains(&version));
        let name=format!("w1_boot_{}",uuid::Uuid::new_v4().simple());admin.batch_execute(&format!("CREATE DATABASE {name}")).await.unwrap();
        assert_eq!(base.get_user(),Some("postgres"));assert!(base.get_password().is_none());
        let (t,locks,j,mut config,selection,mut pg)=fixture();config.database_url=format!("postgres://postgres@127.0.0.1:{}/postgres",base.get_ports().first().copied().unwrap_or(5432));config.migrations_dir=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");
        let state=j.load().clone();let mut current_candidate=state.candidate.clone();current_candidate.migrations_sha256=super::super::journal::migrations_sha256(&crate::migrations::read_migrations(&config.migrations_dir).await.unwrap());assert_ne!(state.candidate.migrations_sha256,current_candidate.migrations_sha256);
        fs::write(t.path().join("app/.nwui-auto/journal-v1.json"),serde_json::to_vec(&state).unwrap()).unwrap();let mut j=Journal::open(&t.path().join("app"),&locks).unwrap();
        pg.socket_dir=socket;pg.port=base.get_ports().first().copied().unwrap_or(5432);pg.dbname=name.clone();pg.role=base.get_user().unwrap_or("postgres").into();let secrets=SecretBox::from_key([7;32]);
        let applied=migrate(&selection,&current_candidate,&config,&secrets,&pg,&locks,&mut j).await.unwrap();assert!(applied>0);
        let mut selected=base.clone();selected.dbname(&name);let (boundary,driver)=selected.connect(tokio_postgres::NoTls).await.unwrap();tokio::spawn(driver);
        assert_eq!(boundary.query_one("SELECT count(*) FROM users",&[]).await.unwrap().get::<_,i64>(0),0);drop(boundary);
        assert!(!config.target_dns.descriptor_path.exists());assert!(!j.load().records.iter().any(|r|r.step==Step::P5Publish || r.step==Step::S4Prepare));
        let privileges=finish(&selection,&config,&secrets,&pg,&locks,&mut j,Inputs{admin_email:Some("i1@example.test".into()),admin_password:Some("fixture-password-only".into()),backfill:None},&Notices { rows: Vec::new(), retention_known: true },crate::publisher::boot::BootMode::Conversion).await.unwrap();assert!(privileges.payload.is_none() && privileges.key_receipt.is_none());
        let completed=j.load().records.iter().filter(|r|r.state==StepState::Done).filter_map(|r|r.data.as_ref()?.get("bootStep")?.as_str()).collect::<Vec<_>>();assert_eq!(completed,["run_migrations","preflight_api_port_rows","ensure_instance","seed_admin","dns_preflight","backfill","prepare_privileges"]);
        assert_eq!(prepare(&selection,&current_candidate,&config,&secrets,&pg,&locks,&mut j,inputs()).await.unwrap().migrations_applied,0);let descriptor=fs::read(&config.target_dns.descriptor_path).unwrap();
        let mut selected=base.clone();selected.dbname(&name);let (db,driver)=selected.connect(tokio_postgres::NoTls).await.unwrap();tokio::spawn(driver);
        db.batch_execute("DELETE FROM users").await.unwrap();db.execute("INSERT INTO nginx_servers(id,server_name,listen,enabled) VALUES('conflict','guard.test',$1,false)",&[&i32::from(config.listen.port())]).await.unwrap();
        let error=prepare(&selection,&current_candidate,&config,&secrets,&pg,&locks,&mut j,Inputs{admin_email:Some("must-not-be-created@example.test".into()),admin_password:Some("fixture-password-only".into()),backfill:None}).await.err().unwrap();assert_eq!(error.code,"api_port_conflict");
        assert_eq!(db.query_one("SELECT count(*) FROM users",&[]).await.unwrap().get::<_,i64>(0),0);assert_eq!(fs::read(&config.target_dns.descriptor_path).unwrap(),descriptor);assert!(matches!(j.load().records.last().unwrap().state,StepState::Failed{..}));drop(db);
        admin.batch_execute(&format!("DROP DATABASE {name} WITH (FORCE)")).await.unwrap();
    }
}

#[cfg(test)]mod desktop_shared_notice_tests{
 use super::*;
 #[test]fn desktop_shared_notice_failure_is_nonfatal(){storage_notice_result(Err(notice_failed()));storage_notice_result(Ok(()));}
}
