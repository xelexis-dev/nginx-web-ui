#[path = "../../tests/auth_support/mod.rs"]
mod fixtures;

use super::*;
use crate::{
    admin::settings,
    auth::principal::SessionPrincipal,
    contracts::{FormInput, Role},
};
use fixtures::TestDb;

fn admin() -> SessionPrincipal {
    SessionPrincipal {
        user_id: "u1".into(),
        email: Some("u1@example.test".into()),
        name: None,
        role: Role::Admin,
        session_version: 3,
        totp_enabled: false,
        requires_2fa: false,
        mfa_verified: false,
        expires_at: i64::MAX,
    }
}

async fn settings(db: &TestDb, audit: &str, history: &str) -> Value {
    let mut form = FormInput::default();
    form.fields.insert("auditRetentionDays".into(), vec![audit.into()]);
    form.fields.insert("historyRetentionDays".into(), vec![history.into()]);
    let row = db.client.query_one(
        "SELECT (SELECT audit_retention_days::text FROM app_settings WHERE id='current'),
            (SELECT history_retention_days::text FROM app_settings WHERE id='current')", &[],
    ).await.unwrap();
    for (i, field) in ["expectedAuditRetentionDays", "expectedHistoryRetentionDays"].into_iter().enumerate() {
        form.fields.insert(field.into(), vec![row.get::<_, Option<String>>(i).unwrap_or_default()]);
    }
    settings::update_retention(&db.state, &admin(), &fixtures::ctx(), &form).await.unwrap()
}

async fn database() -> TestDb {
    let db = fixtures::test_db().await;
    fixtures::insert_user(&db.client, fixtures::UserRow { role: "admin", ..Default::default() }, None).await;
    db
}

async fn due(db: &TestDb, collection: Collection) {
    db.client.execute(
        "UPDATE retention_state SET next_run_at=now()-interval '1 second' WHERE collection=$1",
        &[&collection.key()],
    ).await.unwrap();
}

async fn check(db: &TestDb, collection: Collection) {
    run(&db.state, collection, &db.state.runtime.shutdown.subscribe()).await;
}

async fn snapshot_count(db: &TestDb) -> i64 {
    db.client.query_one("SELECT count(*) FROM config_snapshots", &[]).await.unwrap().get(0)
}

async fn seed_history(db: &TestDb, count: i32) {
    db.client.execute(
        "INSERT INTO config_snapshots (id, created_at, data)
         SELECT 's'||lpad(n::text, 5, '0'), now()-interval '100 days', '{}'::jsonb FROM generate_series(1,$1) n",
        &[&count],
    ).await.unwrap();
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn startup_save_window_off_and_restoring_a_longer_period() {
    let db = database().await;
    seed_history(&db, 150).await;
    let view = settings::retention_view(&db.state).await.unwrap();
    assert_eq!(view["history"]["status"], "off");
    assert_eq!(view["history"]["days"], Value::Null);
    assert_eq!(view["history"]["count"], 150);
    check(&db, Collection::History).await;
    assert_eq!(snapshot_count(&db).await, 150);

    assert_eq!(settings(&db, "", "30").await["schedulesCleanup"], true);
    let next: DateTime<Utc> = db.client.query_one(
        "SELECT next_run_at FROM retention_state WHERE collection='history'
         AND next_run_at BETWEEN now()+interval '9 minutes 58 seconds' AND now()+interval '10 minutes'", &[],
    ).await.unwrap().get(0);
    assert_eq!(settings::retention_view(&db.state).await.unwrap()["history"]["status"], "waiting");
    check(&db, Collection::History).await;
    assert_eq!(snapshot_count(&db).await, 150);
    due(&db, Collection::History).await;
    let worker = start(db.state.clone());
    tokio::time::sleep(Duration::from_millis(30)).await;
    db.state.runtime.shutdown.send_replace(true);
    worker.await.unwrap();
    assert_eq!(snapshot_count(&db).await, 150);

    settings(&db, "", "").await;
    assert!(db.client.query_one("SELECT next_run_at IS NULL FROM retention_state WHERE collection='history'", &[]).await.unwrap().get::<_, bool>(0));
    check(&db, Collection::History).await;
    assert_eq!(snapshot_count(&db).await, 150);
    db.state.runtime.shutdown.send_replace(false);

    settings(&db, "", "365").await;
    settings(&db, "", "30").await;
    let shortened_at: DateTime<Utc> = db.client.query_one("SELECT next_run_at FROM retention_state WHERE collection='history'", &[]).await.unwrap().get(0);
    assert!(shortened_at > next);
    assert_eq!(settings(&db, "", "365").await["schedulesCleanup"], false);
    assert_eq!(db.client.query_one("SELECT next_run_at FROM retention_state WHERE collection='history'", &[]).await.unwrap().get::<_, DateTime<Utc>>(0), shortened_at);
    due(&db, Collection::History).await;
    check(&db, Collection::History).await;
    assert_eq!(snapshot_count(&db).await, 150);

    let audit = db.client.query_one(
        "SELECT actor_id, before, after FROM audit_logs WHERE action='settings:retention' ORDER BY created_at DESC, id DESC LIMIT 1", &[],
    ).await.unwrap();
    assert_eq!(audit.get::<_, String>(0), "u1");
    assert_eq!(audit.get::<_, Value>(1)["historyRetentionDays"], 30);
    assert_eq!(audit.get::<_, Value>(2)["historyRetentionDays"], 365);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn history_floor_estimate_and_equal_timestamp_boundary() {
    let db = database().await;
    seed_history(&db, 150).await;
    for (id, name) in [("s00051", "older.example.test"), ("s00052", "newer.example.test")] {
        let data = json!({"servers":[{"id":"server","serverName":name,"listen":80}],"streams":[],"upstreams":[]});
        db.client.execute("UPDATE config_snapshots SET data=$2 WHERE id=$1", &[&id, &data]).await.unwrap();
    }
    let query = std::collections::BTreeMap::new();
    let principal = admin().into();
    let read = |path| crate::control::operations::view(&db.state, &principal, path, &query);
    let before = read("/history/s00001").await.unwrap().unwrap();
    assert_eq!(before["isFirst"], true);
    assert_eq!(before["previousPruned"], false);
    settings(&db, "", "30").await;
    due(&db, Collection::History).await;
    assert_eq!(estimate(&db.client, Collection::History, 30, false).await.unwrap()["deleted"], 50);
    check(&db, Collection::History).await;
    assert_eq!(snapshot_count(&db).await, 100);
    let list = read("/history").await.unwrap().unwrap();
    assert_eq!(list["data"]["lastCleanup"]["deleted"], 50);
    let oldest = read("/history/s00051").await.unwrap().unwrap();
    assert_eq!(oldest["isFirst"], false);
    assert_eq!(oldest["previousPruned"], true);
    let tied = read("/history/s00052").await.unwrap().unwrap();
    assert_eq!(tied["isFirst"], false);
    assert_eq!(tied["previousPruned"], false);
    assert_eq!(tied["diff"]["hasChanges"], true);
    assert_eq!(tied["diff"]["servers"][0]["changes"][0]["field"], "serverName");
    assert_eq!(read("/history/s00050").await.unwrap_err().status, axum::http::StatusCode::NOT_FOUND);

    db.client.batch_execute("DELETE FROM config_snapshots WHERE id > 's00100'").await.unwrap();
    due(&db, Collection::History).await;
    check(&db, Collection::History).await;
    assert_eq!(snapshot_count(&db).await, 50);
    assert_eq!(db.client.query_one("SELECT count(*) FROM audit_logs WHERE action='retention:prune'", &[]).await.unwrap().get::<_, i64>(0), 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_floor_preserves_latest_receipts_and_real_events() {
    let db = database().await;
    settings(&db, "90", "").await;
    db.client.batch_execute(
        "INSERT INTO audit_logs (id, action, created_at)
         SELECT 'audit-'||lpad(n::text, 5, '0'), 'test:event', now()-interval '120 days'
         FROM generate_series(1, 10050) n;
         INSERT INTO audit_logs (id, action, created_at, after) VALUES
         ('audit-old-receipt','retention:prune',now()-interval '120 days','{\"collection\":\"audit\"}'),
         ('audit-new-receipt','retention:prune',now()-interval '100 days','{\"collection\":\"audit\"}'),
         ('history-receipt','retention:prune',now()-interval '100 days','{\"collection\":\"history\"}');",
    ).await.unwrap();
    // This is deliberately inserted later with a clock value earlier than every other event.
    db.client.batch_execute("INSERT INTO audit_logs(id, action, created_at) VALUES('late-old','test:event',now()-interval '200 days')").await.unwrap();
    due(&db, Collection::Audit).await;
    let cutoff = db.client.query_one("SELECT now()-interval '90 days'", &[]).await.unwrap().get(0);
    let estimated = Collection::Audit.count(&db.client, cutoff).await.unwrap();
    assert_eq!(estimated, 53);
    check(&db, Collection::Audit).await;
    let row = db.client.query_one(
        "SELECT count(*), min(id) FROM audit_logs WHERE action='test:event'", &[],
    ).await.unwrap();
    assert_eq!(row.get::<_, i64>(0), 9_999); // The real admin settings event also counts toward the floor.
    assert_eq!(row.get::<_, String>(1), "audit-00052");
    let view = crate::observability::query::audit_list_view(
        &db.state, &std::collections::BTreeMap::from([("action".into(), "retention:prune".into())]), true,
    ).await.unwrap();
    assert_eq!(view["total"], 3);
    assert_eq!(view["lastCleanup"]["deleted"], estimated);
    assert_eq!(view["items"][0]["actorLabel"], "system");
    assert_eq!(view["items"][0]["target"], "감사 로그 · 53건 삭제");
    assert!(view["actions"].as_array().unwrap().contains(&json!("retention:prune")));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn failure_keeps_pending_counts_and_retries_receipt_before_deleting() {
    let db = database().await;
    seed_history(&db, 625).await;
    settings(&db, "", "30").await;
    due(&db, Collection::History).await;
    db.client.batch_execute("ALTER TABLE audit_logs ADD CONSTRAINT reject_receipt CHECK(action <> 'retention:prune')").await.unwrap();
    check(&db, Collection::History).await;
    assert_eq!(snapshot_count(&db).await, 125);
    let view = settings::retention_view(&db.state).await.unwrap();
    assert_eq!(view["history"]["status"], "failed");
    assert_eq!(view["history"]["lastFailure"]["deleted"], 500);
    assert_eq!(view["history"]["remaining"], 25);
    assert_eq!(view["history"]["lastCleanup"]["deleted"], 500);
    due(&db, Collection::History).await;
    check(&db, Collection::History).await;
    assert_eq!(snapshot_count(&db).await, 125);
    db.client.batch_execute("ALTER TABLE audit_logs DROP CONSTRAINT reject_receipt; UPDATE retention_state SET failure_at=now()-interval '1 hour' WHERE collection='history'").await.unwrap();
    due(&db, Collection::History).await;
    check(&db, Collection::History).await;
    assert_eq!(snapshot_count(&db).await, 100);
    let receipts: Vec<Value> = db.client.query("SELECT after FROM audit_logs WHERE action='retention:prune' ORDER BY created_at", &[])
        .await.unwrap().iter().map(|r| r.get(0)).collect();
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0]["deleted"], 500);
    assert_eq!(receipts[1]["deleted"], 25);
    let outcome = db.client.query_one("SELECT unreceipted_deleted, total_deleted FROM retention_state WHERE collection='history'", &[]).await.unwrap();
    assert_eq!(outcome.get::<_, i64>(0), 0);
    assert_eq!(outcome.get::<_, i64>(1), 525);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn shortening_after_a_receipt_failure_runs_at_the_new_deadline() {
    let db = database().await;
    seed_history(&db, 625).await;
    settings(&db, "", "60").await;
    due(&db, Collection::History).await;
    db.client.batch_execute("ALTER TABLE audit_logs ADD CONSTRAINT reject_receipt CHECK(action <> 'retention:prune')").await.unwrap();
    check(&db, Collection::History).await;
    assert_eq!(snapshot_count(&db).await, 125);
    let failed = settings::retention_view(&db.state).await.unwrap();
    assert_eq!(failed["history"]["status"], "failed");
    assert_eq!(failed["history"]["lastFailure"]["deleted"], 500);
    db.client.batch_execute("ALTER TABLE audit_logs DROP CONSTRAINT reject_receipt").await.unwrap();

    assert_eq!(settings(&db, "", "30").await["schedulesCleanup"], true);
    check(&db, Collection::History).await;
    assert_eq!(snapshot_count(&db).await, 125);
    assert_eq!(settings::retention_view(&db.state).await.unwrap()["history"]["status"], "waiting");

    due(&db, Collection::History).await;
    check(&db, Collection::History).await;
    assert_eq!(snapshot_count(&db).await, 100);
    assert_eq!(settings::retention_view(&db.state).await.unwrap()["history"]["status"], "done");
    let receipts: Vec<Value> = db.client.query(
        "SELECT after FROM audit_logs WHERE action='retention:prune' ORDER BY created_at, id", &[],
    ).await.unwrap().iter().map(|row| row.get(0)).collect();
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0]["deleted"], 500);
    assert_eq!(receipts[0]["retentionDays"], 60);
    assert_eq!(receipts[1]["deleted"], 25);
    assert_eq!(receipts[1]["retentionDays"], 30);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_hourly_cap_and_backlog_across_restarts() {
    let db = database().await;
    settings(&db, "90", "").await;
    db.client.batch_execute(
        "INSERT INTO audit_logs (id, action, created_at)
         SELECT 'seed-'||lpad(n::text, 6, '0'), 'test:event', now()-interval '100 days'
         FROM generate_series(1, 309999) n",
    ).await.unwrap();
    for runs in 1..=15 {
        due(&db, Collection::Audit).await;
        check(&db, Collection::Audit).await;
        let rows = db.client.query_one("SELECT total_deleted, unreceipted_deleted FROM retention_state WHERE collection='audit'", &[]).await.unwrap();
        assert_eq!(rows.get::<_, i64>(0), i64::from(runs)*20_000, "{runs} completed runs");
        assert_eq!(rows.get::<_, i64>(1), 0);
    }
    let total = db.client.query_one("SELECT count(*) FROM audit_logs WHERE action<>'retention:prune'", &[]).await.unwrap().get::<_, i64>(0);
    assert_eq!(total, 10_000);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn measure_history_selection_at_ten_thousand_rows() {
    let db = database().await;
    seed_history(&db, 10_000).await;
    db.client.batch_execute("ANALYZE config_snapshots").await.unwrap();
    let cutoff = Utc::now() - chrono::Days::new(30);
    let sql = format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) SELECT id FROM ({}) eligible ORDER BY created_at,id LIMIT 25", Collection::History.candidates(false));
    let plan: Value = db.client.query_one(&sql, &[&cutoff]).await.unwrap().get(0);
    eprintln!("10k snapshot candidate selection, 25 ids: execution {} ms, shared hit blocks {}", plan[0]["Execution Time"], plan[0]["Plan"]["Shared Hit Blocks"]);
    assert_eq!(plan[0]["Plan"]["Actual Rows"], 25);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn expired_run_recovers_committed_batch_after_restart() {
    let db = database().await;
    seed_history(&db, 150).await;
    settings(&db, "", "30").await;
    due(&db, Collection::History).await;
    db.client.batch_execute(
        "UPDATE retention_state SET run_started_at=now()-interval '11 minutes' WHERE collection='history'",
    ).await.unwrap();

    let mut connection = db.state.db.get().await.unwrap();
    let tx = connection.transaction().await.unwrap();
    assert!(lock(&tx).await.unwrap());
    let cutoff = tx.query_one("SELECT now()-interval '30 days'", &[]).await.unwrap().get(0);
    assert_eq!(delete_batch(&tx, Collection::History, 30, cutoff).await.unwrap(), 25);
    tx.commit().await.unwrap();
    drop(connection);
    assert_eq!(snapshot_count(&db).await, 125);

    let restarted = AppState::new((*db.state.config).clone()).unwrap();
    run(&restarted, Collection::History, &restarted.runtime.shutdown.subscribe()).await;
    assert_eq!(snapshot_count(&db).await, 100);
    let receipts = db.client.query_one(
        "SELECT count(*), sum((after->>'deleted')::integer) FROM audit_logs WHERE action='retention:prune'", &[],
    ).await.unwrap();
    assert_eq!(receipts.get::<_, i64>(0), 2);
    assert_eq!(receipts.get::<_, i64>(1), 50);
}


#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn lost_finalization_lock_does_not_release_the_hourly_budget() {
    let db = database().await;
    let collection = Collection::History;
    seed_history(&db, 601).await;
    settings(&db, "", "30").await;
    due(&db, collection).await;

    let mut client = db.state.db.get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    let (started, admitted_for_deletion) = claim_run(&tx, collection).await.unwrap().unwrap();
    assert!(admitted_for_deletion);
    tx.commit().await.unwrap();
    let cutoff = db.client.query_one("SELECT now()-interval '30 days'", &[]).await.unwrap().get(0);
    for _ in 0..20 {
        let tx = client.transaction().await.unwrap();
        assert!(lock(&tx).await.unwrap());
        assert_eq!(delete_batch(&tx, collection, 30, cutoff).await.unwrap(), 25);
        tx.commit().await.unwrap();
    }

    let mut second = db.state.db.get().await.unwrap();
    let held = second.transaction().await.unwrap();
    assert!(lock(&held).await.unwrap());
    let tx = client.transaction().await.unwrap();
    finalize_run(&tx, collection, started, Some(cutoff)).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(db.client.query_one("SELECT unreceipted_deleted FROM retention_state WHERE collection='history'", &[]).await.unwrap().get::<_, i64>(0), 500);
    held.commit().await.unwrap();
    drop(second);
    drop(client);

    db.client.batch_execute("UPDATE retention_state SET run_started_at=now()-interval '6 minutes' WHERE collection='history'").await.unwrap();
    for _ in 0..2 {
        check(&db, collection).await;
    }
    assert_eq!(snapshot_count(&db).await, 101);
    assert!(db.client.query_opt("SELECT id FROM config_snapshots WHERE id='s00501'", &[]).await.unwrap().is_some());
    let received = db.client.query_one("SELECT after FROM audit_logs WHERE action='retention:prune'", &[]).await.unwrap().get::<_, Value>(0);
    assert_eq!(received["deleted"], 500);
    assert!(db.client.query_one("SELECT next_run_at > now() AND last_run_count=500 FROM retention_state WHERE collection='history'", &[]).await.unwrap().get::<_, bool>(0));

    due(&db, collection).await;
    check(&db, collection).await;
    assert_eq!(snapshot_count(&db).await, 100);
    assert!(db.client.query_opt("SELECT id FROM config_snapshots WHERE id='s00501'", &[]).await.unwrap().is_none());
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_confirmation_counts_the_settings_event_at_the_floor() {
    let db = database().await;
    db.client.batch_execute(
        "INSERT INTO audit_logs(id, action, created_at)
         SELECT 'existing-'||n, 'test:event', now() - CASE WHEN n=1 THEN interval '100 days' ELSE interval '1 day' END
         FROM generate_series(1, 10000) n",
    ).await.unwrap();
    let query = std::collections::BTreeMap::from([("auditRetentionDays".into(), "90".into())]);
    let proposed = settings::estimate_retention(&db.state, &query).await.unwrap();
    assert_eq!(proposed["audit"]["deleted"], 1);
    settings(&db, "90", "").await;
    due(&db, Collection::Audit).await;
    check(&db, Collection::Audit).await;

    assert!(db.client.query_opt("SELECT id FROM audit_logs WHERE id='existing-1'", &[]).await.unwrap().is_none());
    let receipt = db.client.query_one("SELECT after FROM audit_logs WHERE action='retention:prune'", &[]).await.unwrap().get::<_, Value>(0);
    assert_eq!(receipt["deleted"], proposed["audit"]["deleted"]);
}


#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn unchanged_audit_proposal_at_the_floor_matches_the_worker() {
    let db = database().await;
    settings(&db, "90", "365").await;
    db.client.batch_execute(
        "INSERT INTO audit_logs(id, action, created_at)
         SELECT 'existing-'||n, 'test:event', now() - CASE WHEN n=1 THEN interval '100 days' ELSE interval '1 day' END
         FROM generate_series(1, 9999) n",
    ).await.unwrap();
    let mut query = std::collections::BTreeMap::from([("auditRetentionDays".into(), "90".into())]);
    assert_eq!(settings::estimate_retention(&db.state, &query).await.unwrap()["audit"]["deleted"], 0);

    query.insert("historyRetentionDays".into(), "365".into());
    let unchanged = settings::estimate_retention(&db.state, &query).await.unwrap();
    assert_eq!(unchanged["audit"]["deleted"], 0);
    query.insert("historyRetentionDays".into(), "".into());
    assert_eq!(settings::estimate_retention(&db.state, &query).await.unwrap()["audit"]["deleted"], 1);

    assert_eq!(settings(&db, "90", "365").await["schedulesCleanup"], false);
    due(&db, Collection::Audit).await;
    check(&db, Collection::Audit).await;
    let deleted: i64 = db.client.query_one("SELECT total_deleted FROM retention_state WHERE collection='audit'", &[]).await.unwrap().get(0);
    assert_eq!(unchanged["audit"]["deleted"], deleted);
    assert!(db.client.query_opt("SELECT id FROM audit_logs WHERE id='existing-1'", &[]).await.unwrap().is_some());
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_receipt_remaining_includes_the_previously_protected_receipt() {
    let db = database().await;
    settings(&db, "90", "").await;
    db.client.batch_execute(
        r#"INSERT INTO audit_logs(id,action,created_at)
             SELECT 'event-'||n, 'test:event', now() - CASE WHEN n=1 THEN interval '100 days' ELSE interval '1 day' END
             FROM generate_series(1, 10000) n;
           INSERT INTO audit_logs(id, action, created_at, after)
             VALUES('old-receipt', 'retention:prune', now()-interval '100 days', '{"collection":"audit"}');"#,
    ).await.unwrap();
    due(&db, Collection::Audit).await;
    check(&db, Collection::Audit).await;

    let receipt = db.client.query_one("SELECT after FROM audit_logs WHERE action='retention:prune' AND id <> 'old-receipt'", &[]).await.unwrap().get::<_, Value>(0);
    assert_eq!(receipt["deleted"], 1);
    assert_eq!(receipt["remaining"], 1);
    assert!(db.client.query_opt("SELECT id FROM audit_logs WHERE id='old-receipt'", &[]).await.unwrap().is_some());
    assert_eq!(Collection::Audit.count(&db.client, Utc::now() - chrono::Days::new(90)).await.unwrap(), 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn restoring_the_previous_policy_cancels_only_the_extra_deletions() {
    let db = database().await;
    settings(&db, "", "365").await;
    seed_history(&db, 102).await;
    db.client.batch_execute(
        "UPDATE config_snapshots SET created_at=CASE
            WHEN id='s00001' THEN now()-interval '500 days'
            WHEN id='s00002' THEN now()-interval '100 days'
            ELSE now()-interval '1 day' END",
    ).await.unwrap();
    assert_eq!(estimate(&db.client, Collection::History, 30, false).await.unwrap()["deleted"], 2);
    assert_eq!(estimate(&db.client, Collection::History, 365, false).await.unwrap()["deleted"], 1);

    assert_eq!(settings(&db, "", "30").await["schedulesCleanup"], true);
    let next: DateTime<Utc> = db.client.query_one("SELECT next_run_at FROM retention_state WHERE collection='history'", &[]).await.unwrap().get(0);
    assert_eq!(settings(&db, "", "365").await["schedulesCleanup"], false);
    assert_eq!(db.client.query_one("SELECT next_run_at FROM retention_state WHERE collection='history'", &[]).await.unwrap().get::<_, DateTime<Utc>>(0), next);
    check(&db, Collection::History).await;
    assert_eq!(snapshot_count(&db).await, 102);

    due(&db, Collection::History).await;
    check(&db, Collection::History).await;
    assert_eq!(snapshot_count(&db).await, 101);
    assert!(db.client.query_opt("SELECT id FROM config_snapshots WHERE id='s00001'", &[]).await.unwrap().is_none());
    assert!(db.client.query_opt("SELECT id FROM config_snapshots WHERE id='s00002'", &[]).await.unwrap().is_some());
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn filtered_local_audit_falls_back_to_the_last_page_after_pruning() {
    let db = database().await;
    settings(&db, "90", "").await;
    db.client.batch_execute(
        "INSERT INTO audit_logs(id, action, created_at)
           SELECT 'match-'||n, 'test:filtered', now() - CASE WHEN n=1 THEN interval '200 days' ELSE interval '100 days' END
           FROM generate_series(1,51) n;
         INSERT INTO audit_logs(id, action, created_at)
           SELECT 'other-'||n, 'test:other', now()-interval '1 day' FROM generate_series(1,9949) n;",
    ).await.unwrap();
    let query = std::collections::BTreeMap::from([("action".into(), "test:filtered".into()), ("page".into(), "2".into())]);
    let local = || crate::observability::query::audit_list_view(&db.state, &query, true);
    let before = local().await.unwrap();
    assert_eq!(before["page"], 2);
    assert_eq!(before["total"], 51);
    assert_eq!(before["items"][0]["id"], "match-1");

    due(&db, Collection::Audit).await;
    check(&db, Collection::Audit).await;
    let after = local().await.unwrap();
    assert_eq!(after["page"], 1);
    assert_eq!(after["total"], 50);
    assert_eq!(after["totalPages"], 1);
    assert_eq!(after["items"].as_array().unwrap().len(), 50);
    assert_eq!(after["lastCleanup"]["deleted"], 1);

    let node = crate::observability::query::audit_list(&db.state, &query).await.unwrap();
    assert_eq!(node["page"], 2);
    assert_eq!(node["totalPages"], 1);
    assert_eq!(node["items"], json!([]));
    assert!(node.get("lastCleanup").is_none());
}
