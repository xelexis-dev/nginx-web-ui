#[path = "auth_support/mod.rs"]
mod auth_support;

use nwui_api::{migrations::{self, NewerDatabase}, runtime_boot::{self, BootCtx}};
use std::{fs, path::{Path, PathBuf}};

const PACKAGED_MIGRATIONS: usize = 39;

fn full() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle")
}
fn prefix(count: usize) -> tempfile::TempDir {
    let root=full();
    let mut journal:serde_json::Value=serde_json::from_slice(&fs::read(root.join("meta/_journal.json")).unwrap()).unwrap();
    journal["entries"].as_array_mut().unwrap().truncate(count);
    let temp=tempfile::tempdir().unwrap();fs::create_dir(temp.path().join("meta")).unwrap();
    for entry in journal["entries"].as_array().unwrap() {
        let sql=format!("{}.sql",entry["tag"].as_str().unwrap());fs::copy(root.join(&sql),temp.path().join(sql)).unwrap();
    }
    fs::write(temp.path().join("meta/_journal.json"),serde_json::to_vec(&journal).unwrap()).unwrap();temp
}
async fn snapshot(db:&auth_support::TestDb)->(i64,i64) {
    (db.client.query_one("SELECT count(*) FROM drizzle.__drizzle_migrations",&[]).await.unwrap().get(0),
     db.client.query_one("SELECT count(*) FROM pg_class",&[]).await.unwrap().get(0))
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn g1_fresh_then_restart() {
    let empty=prefix(0);let db=auth_support::test_db_with_migrations(&[],empty.path()).await;
    assert_eq!(migrations::run(&db.state.db,&full()).await.unwrap(),PACKAGED_MIGRATIONS);
    assert_eq!(migrations::run(&db.state.db,&full()).await.unwrap(),0);
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn g2_legacy_28_upgrades_to_current() {
    let empty=prefix(0);let old=prefix(28);let db=auth_support::test_db_with_migrations(&[],empty.path()).await;
    assert_eq!(migrations::run(&db.state.db,old.path()).await.unwrap(),28);
    assert_eq!(migrations::run(&db.state.db,&full()).await.unwrap(),PACKAGED_MIGRATIONS - 28);
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn g3_downgrade_and_empty_package_preserve_database() {
    let db=auth_support::test_db().await;let before=snapshot(&db).await;
    let latest=migrations::read_migrations(&full()).await.unwrap().last().unwrap().timestamp;
    for count in [28,0] {
        let older=prefix(count);let packaged=migrations::read_migrations(older.path()).await.unwrap().last().map(|m|m.timestamp);
        let error=migrations::run(&db.state.db,older.path()).await.unwrap_err();
        let newer=error.downcast_ref::<NewerDatabase>().expect("typed downgrade refusal");
        assert_eq!(newer.applied,latest);assert_eq!(newer.packaged,packaged);assert_eq!(snapshot(&db).await,before);
    }
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn g4_duplicates_preserved_but_duplicate_future_timestamps_refuse() {
    let db=auth_support::test_db().await;
    db.client.batch_execute("INSERT INTO drizzle.__drizzle_migrations(hash,created_at) SELECT hash,created_at FROM drizzle.__drizzle_migrations ORDER BY created_at LIMIT 1").await.unwrap();
    assert_eq!(migrations::run(&db.state.db,&full()).await.unwrap(),0);
    assert_eq!(snapshot(&db).await.0, i64::try_from(PACKAGED_MIGRATIONS + 1).unwrap());
    let future=migrations::read_migrations(&full()).await.unwrap().last().unwrap().timestamp+1;
    for _ in 0..2 {db.client.execute("INSERT INTO drizzle.__drizzle_migrations(hash,created_at) VALUES('future',$1)",&[&future]).await.unwrap();}
    let before=snapshot(&db).await;
    let error=migrations::run(&db.state.db,&full()).await.unwrap_err();assert_eq!(error.downcast_ref::<NewerDatabase>().unwrap().applied,future);
    assert_eq!(snapshot(&db).await,before);
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn g5_boot_maps_only_typed_newer_database() {
    let db=auth_support::test_db().await;let old=prefix(28);let mut config=(*db.state.config).clone();config.migrations_dir=old.path().into();
    let ctx=BootCtx{config:&config,secrets:&db.state.secrets,pool:&db.state.db};
    let failure=runtime_boot::run_migrations(&ctx).await.unwrap_err();assert_eq!(failure.code,"database_newer");
    assert_eq!(failure.message,"이 DB는 더 새 버전의 이미지가 사용했습니다. 이 이미지로는 시작하지 않습니다.");
    assert!(failure.detail.unwrap().contains("newer image"));
    // The same words in an untyped SQL error must keep the generic classification.
    let pending=prefix(PACKAGED_MIGRATIONS);
    let mut journal:serde_json::Value=serde_json::from_slice(&fs::read(pending.path().join("meta/_journal.json")).unwrap()).unwrap();
    let next=journal["entries"].as_array().unwrap().last().unwrap()["when"].as_i64().unwrap()+1;
    let tag=format!("{PACKAGED_MIGRATIONS:04}_broken");
    journal["entries"].as_array_mut().unwrap().push(serde_json::json!({"idx":PACKAGED_MIGRATIONS,"when":next,"tag":tag}));
    fs::write(pending.path().join("meta/_journal.json"),serde_json::to_vec(&journal).unwrap()).unwrap();
    fs::write(pending.path().join(format!("{tag}.sql")),"DO $$ BEGIN RAISE EXCEPTION 'newer image'; END $$;").unwrap();
    config.migrations_dir=pending.path().into();let ctx=BootCtx{config:&config,secrets:&db.state.secrets,pool:&db.state.db};
    let failure=runtime_boot::run_migrations(&ctx).await.unwrap_err();assert_eq!(failure.code,"migration_failed");
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn newer_database_refusal_never_executes_pending_sql() {
    let old=prefix(28);let db=auth_support::test_db_with_migrations(&[],old.path()).await;
    let future=migrations::read_migrations(&full()).await.unwrap().last().unwrap().timestamp+1;
    db.client.execute("INSERT INTO drizzle.__drizzle_migrations(hash,created_at) VALUES('future',$1)",&[&future]).await.unwrap();
    // Sequence increments survive transaction rollback, so this detects attempted SQL too.
    db.client.batch_execute("CREATE SEQUENCE public.guard_sentinel").await.unwrap();
    let pending=prefix(PACKAGED_MIGRATIONS);
    let entry=migrations::read_migrations(pending.path()).await.unwrap().remove(28);
    fs::write(pending.path().join(format!("{}.sql",entry.tag)),"SELECT nextval('public.guard_sentinel');").unwrap();
    let before=snapshot(&db).await;
    let error=migrations::run(&db.state.db,pending.path()).await.unwrap_err();assert!(error.downcast_ref::<NewerDatabase>().is_some());
    assert!(!db.client.query_one("SELECT is_called FROM public.guard_sentinel",&[]).await.unwrap().get::<_,bool>(0));
    assert_eq!(snapshot(&db).await,before);
}
