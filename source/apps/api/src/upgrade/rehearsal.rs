//! Ledger-seeded schema rehearsal; A.22's exact schema/rule verification follows this core.
use super::{PrivatePg,StepError,journal::{Candidate,migrations_sha256}};
use crate::{config::RuntimeConfig,db_upgrade::profiles};
fn failed(detail:&str)->StepError {StepError{code:"rehearsal_failed",message:"스키마 사전 검증 이력이 예상한 변경과 일치하지 않습니다. 원본은 보존했습니다.",detail:Some(detail.into())}}
fn database_error(error:&tokio_postgres::Error)->StepError {
    if error.as_db_error().is_some_and(|e|e.code()==&tokio_postgres::error::SqlState::DISK_FULL){return StepError{code:"insufficient_space",message:"스키마 검증용 저장 공간이 부족합니다. 원본을 보존했습니다.",detail:None};}failed("rehearsal_sql")
}
/// Only the empty schema-only private PG17 app: roles/schema already restored, no business rows.
/// Return the real post-DDL ledger for A.22; never publish P2 Done before that verification.
pub(crate) async fn seed_and_migrate(pg:&PrivatePg,config:&RuntimeConfig,candidate:&Candidate,source_ledger:&[(String,i64)])->Result<(usize,Vec<(String,i64)>),StepError> {
    if pg.major!=17 || pg.role!="nginx" || !matches!(pg.dbname.as_str(),"nginx_control"|"nginx_web_ui"){return Err(failed("private_target"));}
    let profile=profiles::select_source(source_ledger)?;
    let migrations=crate::migrations::read_migrations(&config.migrations_dir).await.map_err(|_|failed("packaged_migrations"))?;
    if migrations_sha256(&migrations)!=candidate.migrations_sha256{return Err(failed("candidate_migrations"));}
    let packaged=migrations.iter().map(|m|(m.hash.clone(),m.timestamp)).collect::<Vec<_>>();profile.check_packaged_prefix(&packaged)?;
    let mut seeds=source_ledger.to_vec();seeds.sort_by_key(|(_,time)|*time);seeds.dedup();
    let pool=pg.pool(config)?;
    let result=async {
        {
            let mut client=pool.get().await.map_err(|_|failed("private_target"))?;
            let version:i32=client.query_one("SELECT current_setting('server_version_num')::int",&[]).await.map_err(|e|database_error(&e))?.get(0);
            if !(170000..180000).contains(&version){return Err(failed("target_major"));}
            let tx=client.transaction().await.map_err(|e|database_error(&e))?;
            tx.batch_execute("SET LOCAL lock_timeout='30s'; LOCK TABLE ONLY drizzle.__drizzle_migrations IN EXCLUSIVE MODE").await.map_err(|e|database_error(&e))?;
            if tx.query_opt("SELECT 1 FROM ONLY drizzle.__drizzle_migrations LIMIT 1",&[]).await.map_err(|e|database_error(&e))?.is_some(){return Err(failed("ledger_not_empty"));}
            for (hash,time) in &seeds {tx.execute("INSERT INTO drizzle.__drizzle_migrations(hash,created_at) VALUES($1,$2)",&[hash,time]).await.map_err(|e|database_error(&e))?;}
            tx.commit().await.map_err(|e|database_error(&e))?;
        }
        // Release the seed client before common migrations acquire their own transaction/client.
        let applied=crate::migrations::run(&pool,&config.migrations_dir).await.map_err(|e|e.chain().find_map(|e|e.downcast_ref::<tokio_postgres::Error>()).map(database_error).unwrap_or_else(||failed("run_migrations")))?;
        let client=pool.get().await.map_err(|_|failed("private_target"))?;
        let rows=client.query("SELECT hash,created_at FROM ONLY drizzle.__drizzle_migrations ORDER BY id",&[]).await.map_err(|e|database_error(&e))?;
        let ledger=rows.iter().map(|r|Ok((r.try_get::<_,String>(0).map_err(|_|failed("rehearsal_ledger"))?,r.try_get::<_,i64>(1).map_err(|_|failed("rehearsal_ledger"))?))).collect::<Result<Vec<_>,StepError>>()?;
        if ledger!=packaged || applied!=packaged.len()-profile.signatures.len(){return Err(failed("rehearsal_ledger"));}Ok((applied,ledger))
    }.await;
    pool.close();result
}
#[cfg(test)]
mod tests {
    use super::*;use std::{fs,os::unix::fs::PermissionsExt};
    #[tokio::test]
    async fn w1_rehearsal_rejects_unknown_source_or_candidate_before_connecting_to_any_database() {
        let t=tempfile::tempdir().unwrap();let mut config=RuntimeConfig::read(|name|match name{"AUTH_SECRET"=>Some("fixture".into()),"DATABASE_URL"=>Some("postgres://nginx:fixture@must-not-contact.invalid/nginx_web_ui".into()),_=>None}).unwrap();config.migrations_dir=t.path().join("missing");
        let pg=PrivatePg{socket_dir:t.path().join("absent-socket"),port:50432,dbname:"nginx_web_ui".into(),role:"nginx".into(),major:17};let candidate=Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)};
        assert_eq!(seed_and_migrate(&pg,&config,&candidate,&[]).await.unwrap_err().code,"ledger_unknown");let rows=profiles::released().unwrap().signatures.into_iter().map(|s|(s.hash,s.created_at)).collect::<Vec<_>>();assert_eq!(seed_and_migrate(&pg,&config,&candidate,&rows).await.unwrap_err().detail.as_deref(),Some("packaged_migrations"));assert!(!pg.socket_dir.exists());
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    #[ignore="native PG17: real legacy schema, DISTINCT source ledger, packaged suffix only"]
    async fn w1_rehearsal_native_seeds_exact_source_then_runs_only_suffix_and_preserves_nonempty_retry() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        use crate::db_upgrade::{fresh,files,layout::Roots,postgres};
        let t=tempfile::tempdir().unwrap();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o711)).unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:crate::db_upgrade::engine::architecture().unwrap().into()};for path in [&roots.db,&roots.app]{files::private_dir(path).unwrap();}let env=t.path().join("env");files::private_dir(&env).unwrap();let locks=super::super::StorageLocks::acquire(&roots.db,&roots.app).unwrap();fresh::secrets_locked(&locks,&roots,&fresh::Inputs{auth_secret:Some("fixture".into()),admin_password:None},&env).unwrap();fresh::provision_locked(&locks,&roots).await.unwrap();let work=t.path().join("rehearsal");postgres::directory(&work).unwrap();let server=postgres::Private::start(17,&roots.db.join("17/data"),&work).unwrap();
        let mut config=RuntimeConfig::read(|name|match name{"AUTH_SECRET"=>Some("fixture".into()),"DATABASE_URL"=>Some(fs::read_to_string(env.join("DATABASE_URL")).unwrap()),_=>None}).unwrap();config.migrations_dir=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");let migrations=crate::migrations::read_migrations(&config.migrations_dir).await.unwrap();let candidate=Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:migrations_sha256(&migrations)};let pg=PrivatePg{socket_dir:server.socket.clone(),port:50432,dbname:"nginx_web_ui".into(),role:"nginx".into(),major:17};let pool=pg.pool(&config).unwrap();
        {let client=pool.get().await.unwrap();for migration in migrations.iter().take(28){for sql in &migration.statements{client.batch_execute(sql).await.unwrap();}}client.batch_execute("CREATE SCHEMA drizzle; CREATE TABLE drizzle.__drizzle_migrations(id serial primary key,hash text NOT NULL,created_at bigint)").await.unwrap();}pool.close();
        let mut source=profiles::released().unwrap().signatures.into_iter().map(|s|(s.hash,s.created_at)).collect::<Vec<_>>();source.reverse();source.push(source[0].clone());let (applied,ledger)=seed_and_migrate(&pg,&config,&candidate,&source).await.unwrap();assert_eq!(applied,migrations.len()-28);assert_eq!(ledger,migrations.iter().map(|m|(m.hash.clone(),m.timestamp)).collect::<Vec<_>>());
        assert_eq!(seed_and_migrate(&pg,&config,&candidate,&source).await.unwrap_err().detail.as_deref(),Some("ledger_not_empty"));let check_pool=pg.pool(&config).unwrap();let client=check_pool.get().await.unwrap();assert_eq!(client.query_one("SELECT count(*)::bigint FROM drizzle.__drizzle_migrations",&[]).await.unwrap().get::<_,i64>(0),migrations.len()as i64);drop(client);check_pool.close();server.stop().unwrap();
    }
}
