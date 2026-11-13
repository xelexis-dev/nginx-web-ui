//! P4: V1 before DDL, V2 before startup row changes, then the A.11/S4 owner.
use super::{Branch,Journal,Step,StepState,StepError,StorageLocks,StorageSelection,PrivatePg,boot,profile,verification,journal::Candidate};
use crate::{config::RuntimeConfig,crypto::SecretBox,db_upgrade::{cluster,inspect,layout::Roots}};
fn failed()->StepError {StepError{code:"verification_failed",message:"변환 대상과 검증 기록이 일치하지 않습니다. 기존 데이터를 보존했습니다.",detail:None}}
fn map(e:crate::db_upgrade::error::Error)->StepError {StepError{code:e.code,message:e.message,detail:e.detail.map(str::to_owned)}}
/// The caller owns/reaps the private PG17 and awaits this whole step under A+D custody.
pub(crate) async fn prepare(roots:&Roots,locks:&StorageLocks,j:&mut Journal,source:&profile::Admitted,current:&Candidate,selection:&StorageSelection,pg:&PrivatePg,config:&RuntimeConfig,key:&SecretBox,inputs:boot::Inputs)->Result<crate::nonroot::Prepared,StepError> {
    profile::require_complete(source)?;
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app) || j.load().branch!=Branch::Upgrade || selection.branch!=Branch::Upgrade || selection.pgdata!=roots.db.join(".nwui-auto-work").join(&j.load().attempt_id).join("data") || pg.major!=17 || pg.role!="nginx" || pg.dbname!="nginx_web_ui" || *current!=j.load().candidate || source.candidate!=*current {return Err(failed());}
    let proof=profile::proof(j)?;let(id,sha)=source.source.identity();if proof.profile_id!=id || proof.profile_sha256!=sha || selection.schema_profile!=id{return Err(failed());}
    let credentials:tokio_postgres::Config=config.database_url.parse().map_err(|_|failed())?;let password=std::str::from_utf8(credentials.get_password().ok_or_else(failed)?).map_err(|_|failed())?;
    if credentials.get_user()!=Some("nginx") || credentials.get_dbname()!=Some("nginx_web_ui"){return Err(failed());}
    let packaged=crate::migrations::read_migrations(&config.migrations_dir).await.map_err(|_|failed())?;
    if super::journal::migrations_sha256(&packaged)!=current.migrations_sha256{return Err(failed());}
    let packaged=packaged.iter().map(|m|(m.hash.clone(),m.timestamp)).collect::<Vec<_>>();
    let root=inspect::connect(&pg.socket_dir,"postgres","postgres",None).await.map_err(map)?;
    let actual=root.client.query_one("SELECT current_setting('data_directory'),current_setting('server_version_num')::int",&[]).await.map_err(|_|failed())?;
    let directory:String=actual.get(0);let version:i32=actual.get(1);if std::path::Path::new(&directory)!=selection.pgdata || !(170000..180000).contains(&version){return Err(failed());}drop(root);
    j.record(Step::P4Target,StepState::Intent,Some(serde_json::json!({"verificationEvent":"v1"})))?;
    let result=async {
        let collations=cluster::collect_auto_collations(&pg.socket_dir,false).await.map_err(map)?;
        super::collation::validate(&collations).map_err(|mut error|{error.code="verification_failed";error})?;
        let mut target=cluster::inspect_cluster(&pg.socket_dir,password,Some(key),false).await.map_err(map)?;target.collations=collations.collations;
        let app=inspect::connect(&pg.socket_dir,&target.application().map_err(map)?,"postgres",None).await.map_err(map)?;
        profile::require_postgres(&app.client).await?;
        let baseline=verification::v1(&source.snapshot,&target,&app.client,&source.source,&source.target,Some(key)).await?;
        drop(app);
        // No app inspection connection may retain the legacy database during its rename.
        cluster::rename_legacy_application(&pg.socket_dir,&source.snapshot).await.map_err(map)?;
        boot::migrate(selection,current,config,key,pg,locks,j).await?;
        let collations=cluster::collect_auto_collations(&pg.socket_dir,false).await.map_err(map)?;
        super::collation::validate(&collations).map_err(|mut error|{error.code="verification_failed";error})?;
        let others=cluster::inspect_non_application(&pg.socket_dir,password,Some(key)).await.map_err(map)?;
        // A fresh postgres connection avoids a SET ROLE carried by DDL in an application pool.
        let app=inspect::connect(&pg.socket_dir,&pg.dbname,"postgres",None).await.map_err(map)?;
        app.client.batch_execute("RESET ROLE").await.map_err(|_|failed())?;profile::require_postgres(&app.client).await?;
        verification::v2(&app.client,&others,&baseline,&source.source,&source.target,&packaged,Some(key)).await?;
        drop(app);j.record(Step::P4Target,StepState::Done,None)?;
        let notices=boot::notices(roots,j)?;
        boot::finish(selection,config,key,pg,locks,j,inputs,&notices,crate::publisher::boot::BootMode::Conversion).await
    }.await;
    if let Err(error)=&result {if !j.load().records.last().is_some_and(|r|matches!(&r.state,StepState::Failed{code}if code==error.code)){j.record_retry_failure(Step::P4Target,error.code,chrono::Utc::now())?;}}result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    #[ignore="native PG17: source-shaped private target; A22/P4 integration, not binary conversion"]
    async fn w1_a22_native_p2_admission_p4_v1_ddl_v2_and_legacy_rename_use_private_postgres() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        use crate::db_upgrade::{files,fresh,postgres};use super::super::{Outcome,KeyBinding,profile};
        use std::{fs,path::Path,os::unix::fs::PermissionsExt};
        if crate::nonroot::PRIVILEGE_SPLIT{return;}
        for legacy in [false,true] {
            let t=tempfile::tempdir().unwrap();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o711)).unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:crate::db_upgrade::engine::architecture().unwrap().into()};for p in [&roots.db,&roots.app]{files::private_dir(p).unwrap();}let env=t.path().join("env");files::private_dir(&env).unwrap();let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();
            fresh::secrets_locked(&locks,&roots,&fresh::Inputs{auth_secret:Some("fixture-only".into()),admin_password:None},&env).unwrap();fresh::provision_locked(&locks,&roots).await.unwrap();
            let mut config=RuntimeConfig::read(|name|match name{"AUTH_SECRET"=>Some("fixture-only".into()),"DATABASE_URL"=>Some(fs::read_to_string(env.join("DATABASE_URL")).unwrap()),_=>None}).unwrap();config.data_key_file=roots.app.join("secrets/data_key");config.target_dns.descriptor_path=t.path().join("target-dns.json");config.migrations_dir=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");let migrations=crate::migrations::read_migrations(&config.migrations_dir).await.unwrap();let current=Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:super::super::journal::migrations_sha256(&migrations)};let mut j={let fixture_journal=Journal::begin(&roots.app,&locks,Branch::Upgrade,current.clone()).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};
            let parent=roots.db.join(".nwui-auto-work");postgres::directory(&parent).unwrap();let work=parent.join(&j.load().attempt_id);postgres::directory(&work).unwrap();let data=work.join("data");fs::rename(roots.db.join("17/data"),&data).unwrap();let logs=t.path().join("logs");postgres::directory(&logs).unwrap();let server=postgres::Private::start(17,&data,&logs).unwrap();
            let root=inspect::connect(&server.socket,"postgres","postgres",None).await.unwrap();root.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();if legacy{root.client.batch_execute("ALTER DATABASE nginx_web_ui RENAME TO nginx_control").await.unwrap();}drop(root);
            let name=if legacy{"nginx_control"}else{"nginx_web_ui"};let app=inspect::connect(&server.socket,name,"postgres",None).await.unwrap();app.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();app.client.batch_execute("SET ROLE nginx; SET search_path=public").await.unwrap();
            for m in migrations.iter().take(28){for sql in &m.statements{app.client.batch_execute(sql).await.unwrap();}}
            app.client.batch_execute("CREATE SCHEMA drizzle; CREATE TABLE drizzle.__drizzle_migrations(id serial primary key,hash text NOT NULL,created_at bigint)").await.unwrap();for m in migrations.iter().take(28){app.client.execute("INSERT INTO drizzle.__drizzle_migrations(hash,created_at) VALUES($1,$2)",&[&m.hash,&m.timestamp]).await.unwrap();}
            let key=SecretBox::from_key([7;32]);let sealed=key.seal("fixture-sealed-only").unwrap();app.client.execute("INSERT INTO users(id,email,totp_secret) VALUES('preserved','fixture@example.test',$1)",&[&sealed]).await.unwrap();app.client.batch_execute("RESET ROLE; SET search_path=pg_catalog; SET default_transaction_read_only=on").await.unwrap();
            let password=fs::read_to_string(roots.app.join("secrets/db_password")).unwrap();let source=cluster::inspect_source(&server.socket,&password,Some(&key)).await.unwrap();let rows=migrations.iter().take(28).map(|m|(m.hash.clone(),m.timestamp)).collect::<Vec<_>>();let admitted=profile::admit(Ok(source),&app.client,&rows,&current,&migrations).await.unwrap();
            // One extra real source object must survive the P2 error boundary as a named diagnostic.
            app.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();
            app.client.batch_execute("BEGIN READ WRITE; CREATE INDEX p2_extra_index ON public.users(email)").await.unwrap();
            let mut extra:cluster::Snapshot=serde_json::from_value(serde_json::to_value(&admitted.snapshot).unwrap()).unwrap();
            extra.data.insert(name.into(),inspect::snapshot_database(&app.client,Some(&key),true).await.unwrap());
            let error=profile::admit(Ok(extra),&app.client,&rows,&current,&migrations).await.err().unwrap();assert_eq!(error.code,"object_unsupported");let detail=error.detail.unwrap();assert_eq!(detail,"relation public p2_extra_index");
            app.client.batch_execute("ROLLBACK; SET default_transaction_read_only=on").await.unwrap();drop(app);
            // P2 rehearsal/pg_upgrade check are separate gates; this P4 fixture supplies their attestation.
            let proof=profile::packaged_identity().unwrap();j.record(Step::P2Preflight,StepState::Done,Some(super::super::profile::fixture_done(crate::db_upgrade::migration::UpgradeMode::Copy))).unwrap();let selection=StorageSelection{version:1,epoch:7,outcome:Outcome::Upgrading,branch:Branch::Upgrade,pgdata:data.clone(),backup_id:Some(j.load().attempt_id.clone()),schema_profile:proof.profile_id,key_binding:KeyBinding::Verified{pair_id:uuid::Uuid::new_v4().to_string()}};let pg=PrivatePg{socket_dir:server.socket.clone(),port:50432,dbname:"nginx_web_ui".into(),role:"nginx".into(),major:17};
            let prepared=prepare(&roots,&locks,&mut j,&admitted,&current,&selection,&pg,&config,&key,boot::Inputs{admin_email:None,admin_password:None,backfill:None}).await.unwrap();assert!(prepared.payload.is_none() && prepared.key_receipt.is_none());
            let app=inspect::connect(&server.socket,"nginx_web_ui","postgres",None).await.unwrap();let row=app.client.query_one("SELECT email,totp_secret FROM public.users WHERE id='preserved'",&[]).await.unwrap();assert_eq!(row.get::<_,String>(0),"fixture@example.test");assert_eq!(row.get::<_,String>(1),sealed);assert_eq!(app.client.query_one("SELECT count(*)::bigint FROM drizzle.__drizzle_migrations",&[]).await.unwrap().get::<_,i64>(0),migrations.len()as i64);assert!(j.load().records.iter().any(|r|r.step==Step::P4Target && r.state==StepState::Done && r.data.is_none()));app.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();app.client.batch_execute("CREATE COLLATION public.p4_icu(provider=icu,locale='en-US'); CREATE TABLE public.p4_unindexed(label text COLLATE public.p4_icu)").await.unwrap();
            let error=prepare(&roots,&locks,&mut j,&admitted,&current,&selection,&pg,&config,&key,boot::Inputs{admin_email:None,admin_password:None,backfill:None}).await.err().unwrap();assert_eq!(error.code,"verification_failed");assert!(error.message.contains("정렬"));app.client.batch_execute("DROP TABLE public.p4_unindexed; DROP COLLATION public.p4_icu").await.unwrap();
            drop(app);server.stop().unwrap();
        }
    }
}
