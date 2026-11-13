#[path = "auth_support/mod.rs"]
mod auth_support;

use nwui_api::migrations::read_migrations;
use std::fs;

fn fixture(entries: serde_json::Value) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("meta")).unwrap();
    fs::write(dir.path().join("meta/_journal.json"), serde_json::to_vec(&serde_json::json!({
        "version":"7", "dialect":"postgresql", "entries": entries
    })).unwrap()).unwrap();
    dir
}

#[tokio::test]
async fn preserves_exact_drizzle_hash_and_statement_boundaries() {
    let dir = fixture(serde_json::json!([{"idx":0,"when":1234,"tag":"0000_test","version":"7","breakpoints":true}]));
    fs::write(dir.path().join("0000_test.sql"), "select 1;\r\n--> statement-breakpoint\r\nselect 2;\r\n").unwrap();
    let entries = read_migrations(dir.path()).await.unwrap();
    use sha2::Digest;
    assert_eq!(entries[0].hash, hex::encode(sha2::Sha256::digest(fs::read(dir.path().join("0000_test.sql")).unwrap())));
    assert_eq!(entries[0].statements, ["select 1;\r\n", "\r\nselect 2;\r\n"]);
    assert_eq!(entries[0].timestamp, 1234);
}

#[tokio::test]
async fn refuses_ambiguous_or_escaping_journals_before_any_database_work() {
    for entries in [
        serde_json::json!([{"idx":0,"when":1,"tag":"../outside"}]),
        serde_json::json!([{"idx":0,"when":2,"tag":"a"},{"idx":1,"when":1,"tag":"b"}]),
        serde_json::json!([{"idx":0,"when":1,"tag":"a"},{"idx":1,"when":1,"tag":"b"}]),
        serde_json::json!([{"idx":0,"when":1,"tag":"a"},{"idx":1,"when":2,"tag":"a"}]),
    ] {
        let dir = fixture(entries);
        fs::write(dir.path().join("a.sql"), "select 1;").unwrap();
        fs::write(dir.path().join("b.sql"), "select 1;").unwrap();
        assert!(read_migrations(dir.path()).await.is_err());
    }
}

#[tokio::test]
async fn invalid_utf8_is_not_silently_replaced_and_hashed() {
    let dir = fixture(serde_json::json!([{"idx":0,"when":1,"tag":"a"}]));
    fs::write(dir.path().join("a.sql"), [0xff, 0xfe]).unwrap();
    assert!(read_migrations(dir.path()).await.is_err());
}

#[tokio::test]
async fn actual_product_journal_is_accepted() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");
    let entries = read_migrations(&directory).await.unwrap();
    assert!(entries.iter().any(|entry| entry.tag == "0025_totp_replay_guard"));
}

#[tokio::test]
async fn audit_actor_snapshot_is_a_new_migration() {
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");
    let migrations=read_migrations(&root).await.unwrap();
    let snapshot=migrations.iter().find(|m|m.tag=="0034_audit_actor_snapshot").unwrap();
    assert_eq!(snapshot.timestamp,1793102402000);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_actor_snapshot_upgrades_the_5de48a33_prefix_and_a_fresh_database() {
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");
    let journal:serde_json::Value=serde_json::from_slice(&fs::read(root.join("meta/_journal.json")).unwrap()).unwrap();
    let previous=journal["entries"].as_array().unwrap().iter().filter(|e|e["idx"].as_u64().unwrap()<=33).cloned().collect::<Vec<_>>();
    assert_eq!(previous.last().unwrap()["tag"],"0033_node_identity_proof");
    assert_eq!(previous.last().unwrap()["when"],1793102401000i64);
    let baseline=fixture(serde_json::json!(previous));
    for entry in &previous {let name=format!("{}.sql",entry["tag"].as_str().unwrap());fs::copy(root.join(&name),baseline.path().join(name)).unwrap();}
    let db=auth_support::test_db_with_migrations(&[],baseline.path()).await;
    let latest:i64=db.client.query_one("SELECT max(created_at) FROM drizzle.__drizzle_migrations",&[]).await.unwrap().get(0);
    assert_eq!(latest,1793102401000);
    auth_support::insert_user(&db.client,auth_support::UserRow::default(),None).await;
    db.client.batch_execute("INSERT INTO audit_logs(id,actor_id,actor_label,action) VALUES('pre-upgrade','u1',NULL,'integration:renumber')").await.unwrap();
    assert_eq!(nwui_api::migrations::run(&db.state.db,&root).await.unwrap(),3);
    assert_eq!(db.client.query_one("SELECT actor_label FROM audit_logs WHERE id='pre-upgrade'",&[]).await.unwrap().get::<_,String>(0),"u1@example.test");
    assert_eq!(db.client.query_one("SELECT count(*) FROM drizzle.__drizzle_migrations WHERE created_at=1793102402000",&[]).await.unwrap().get::<_,i64>(0),1);
    assert_eq!(nwui_api::migrations::run(&db.state.db,&root).await.unwrap(),0);
    eprintln!("UPGRADED DB: exact 5de48a33 prefix (34 migrations, latest1793102401000) -> applied0034 through0036 once, backfill verified, repeat0");
    let fresh=auth_support::test_db().await;
    assert_eq!(fresh.client.query_one("SELECT count(*),max(created_at) FROM drizzle.__drizzle_migrations",&[]).await.unwrap().get::<_,i64>(0),37);
    assert_eq!(fresh.client.query_one("SELECT max(created_at) FROM drizzle.__drizzle_migrations",&[]).await.unwrap().get::<_,i64>(0),1793102404000);
    assert_eq!(nwui_api::migrations::run(&fresh.state.db,&root).await.unwrap(),0);
    eprintln!("FRESH DB: all37 migrations applied, latest1793102404000, repeat0");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_chronology_index_on_the_0034_prefix_and_fresh() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");
    let journal: serde_json::Value = serde_json::from_slice(&fs::read(root.join("meta/_journal.json")).unwrap()).unwrap();
    let previous = journal["entries"].as_array().unwrap().iter()
        .filter(|entry| entry["idx"].as_u64().unwrap() <= 34).cloned().collect::<Vec<_>>();
    assert_eq!(previous.len(), 35);
    assert_eq!(previous.last().unwrap()["tag"], "0034_audit_actor_snapshot");
    let baseline = fixture(serde_json::json!(previous));
    for entry in &previous {
        let name = format!("{}.sql", entry["tag"].as_str().unwrap());
        fs::copy(root.join(&name), baseline.path().join(name)).unwrap();
    }
    let upgraded = auth_support::test_db_with_migrations(&[], baseline.path()).await;
    assert_eq!(nwui_api::migrations::run(&upgraded.state.db, &root).await.unwrap(), 2);
    let fresh = auth_support::test_db().await;
    for (label, db) in [("35-entry prefix", &upgraded), ("fresh", &fresh)] {
        let definition: String = db.client.query_one(
            "SELECT indexdef FROM pg_indexes WHERE schemaname='public' AND indexname='audit_logs_created_at_id_idx'",
            &[],
        ).await.unwrap().get(0);
        assert_eq!(definition, "CREATE INDEX audit_logs_created_at_id_idx ON public.audit_logs USING btree (created_at DESC, id DESC)");
        db.client.batch_execute("SET enable_seqscan=off").await.unwrap();
        let plan: serde_json::Value = db.client.query_one(
            "EXPLAIN (FORMAT JSON) SELECT a.id,a.created_at,a.action,a.target,CASE WHEN u.email IS NOT NULL THEN COALESCE(a.actor_label,u.email) END AS email,a.actor_label,a.ip FROM audit_logs a LEFT JOIN users u ON u.id=a.actor_id WHERE ($1::text IS NULL OR a.action=$1) ORDER BY a.created_at DESC,a.id DESC LIMIT 50 OFFSET $2",
            &[&None::<&str>, &0i64],
        ).await.unwrap().get(0);
        assert!(plan.to_string().contains("audit_logs_created_at_id_idx"), "{label}: {plan}");
        assert_eq!(nwui_api::migrations::run(&db.state.db, &root).await.unwrap(), 0);
        eprintln!("{label}: chronology index definition and audit-list plan verified, repeat0");
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn passkey_migration_preserves_the_0035_users_and_starts_with_no_credentials() {
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");
    let journal:serde_json::Value=serde_json::from_slice(&fs::read(root.join("meta/_journal.json")).unwrap()).unwrap();
    let previous=journal["entries"].as_array().unwrap().iter().filter(|e|e["idx"].as_u64().unwrap()<=35).cloned().collect::<Vec<_>>();
    assert_eq!(previous.len(),36);
    assert_eq!(previous.last().unwrap()["tag"],"0035_audit_log_chronology");
    let baseline=fixture(serde_json::json!(previous));
    for entry in &previous { let name=format!("{}.sql",entry["tag"].as_str().unwrap()); fs::copy(root.join(&name),baseline.path().join(name)).unwrap(); }
    let db=auth_support::test_db_with_migrations(&[],baseline.path()).await;
    auth_support::insert_user(&db.client,auth_support::UserRow::default(),Some("unchanged-password-hash")).await;
    assert_eq!(nwui_api::migrations::run(&db.state.db,&root).await.unwrap(),1);
    let row=db.client.query_one("SELECT password_hash,passkey_user_handle FROM users WHERE id='u1'",&[]).await.unwrap();
    assert_eq!(row.get::<_,Option<String>>(0).as_deref(),Some("unchanged-password-hash"));
    assert!(row.get::<_,Option<Vec<u8>>>(1).is_none());
    assert_eq!(db.client.query_one("SELECT count(*) FROM user_passkeys",&[]).await.unwrap().get::<_,i64>(0),0);
    assert_eq!(nwui_api::migrations::run(&db.state.db,&root).await.unwrap(),0);
}
