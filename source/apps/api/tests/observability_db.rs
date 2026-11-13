mod auth_support;
use auth_support::*;
use chrono::{DateTime, Utc};
use nwui_api::observability::{collector::*, query};
use tokio::io::AsyncWriteExt;

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-09-22T12:10:00Z")
        .unwrap()
        .with_timezone(&Utc)
}
fn line(host: &str) -> String {
    format!(
        "127.0.0.1 - - [22/Sep/2026:12:00:00 +0000] \"GET / HTTP/1.1\" 200 42 \"-\" \"agent\" \"{host}\"\n"
    )
}
async fn append(path: &std::path::Path, s: &str) {
    let mut f = tokio::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .await
        .unwrap();
    f.write_all(s.as_bytes()).await.unwrap();
    f.flush().await.unwrap();
}
async fn count(db: &TestDb) -> i64 {
    db.client
        .query_one(
            "SELECT coalesce(sum(requests),0)::int8 FROM domain_metric_minutes",
            &[],
        )
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn restart_partial_append_small_fingerprint_inode_rotation_and_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.log");
    tokio::fs::write(&path, line("one.test")).await.unwrap();
    let db = test_db_with(&[("NGINX_ACCESS_LOG", path.to_str().unwrap())]).await;
    let first = parse_pass(&db.state, now()).await.unwrap().unwrap();
    assert_eq!(first.requests, 1);
    assert_eq!(
        parse_pass(&db.state, now())
            .await
            .unwrap()
            .unwrap()
            .requests,
        0
    );
    let second = line("two.test");
    append(&path, second.trim_end()).await;
    assert_eq!(
        parse_pass(&db.state, now()).await.unwrap().unwrap().offset,
        first.offset
    );
    append(&path, "\n").await;
    assert_eq!(
        parse_pass(&db.state, now())
            .await
            .unwrap()
            .unwrap()
            .requests,
        1
    );
    assert_eq!(count(&db).await, 2);
    tokio::fs::rename(&path, dir.path().join("old.log"))
        .await
        .unwrap();
    tokio::fs::write(&path, line("three.test").repeat(4))
        .await
        .unwrap();
    assert_eq!(
        parse_pass(&db.state, now())
            .await
            .unwrap()
            .unwrap()
            .requests,
        4
    );
    tokio::fs::write(&path, line("four.test")).await.unwrap();
    assert_eq!(
        parse_pass(&db.state, now())
            .await
            .unwrap()
            .unwrap()
            .requests,
        1
    );
    assert_eq!(count(&db).await, 7);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn checkpoint_and_increments_rollback_together_and_parallel_passes_do_not_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.log");
    tokio::fs::write(&path, line("atomic.test")).await.unwrap();
    let db = test_db_with(&[("NGINX_ACCESS_LOG", path.to_str().unwrap())]).await;
    db.client.batch_execute("CREATE FUNCTION reject_checkpoint() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test'; END $$; CREATE TRIGGER fail_checkpoint BEFORE INSERT ON metric_parse_state FOR EACH ROW EXECUTE FUNCTION reject_checkpoint();").await.unwrap();
    assert!(parse_pass(&db.state, now()).await.is_err());
    assert_eq!(count(&db).await, 0);
    assert_eq!(
        db.client
            .query_one("SELECT count(*) FROM metric_hosts", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    db.client
        .batch_execute("DROP TRIGGER fail_checkpoint ON metric_parse_state")
        .await
        .unwrap();
    let (a, b) = tokio::join!(parse_pass(&db.state, now()), parse_pass(&db.state, now()));
    assert_eq!(
        a.unwrap().unwrap().requests + b.unwrap().unwrap().requests,
        1
    );
    assert_eq!(count(&db).await, 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn exclusion_host_budget_oversized_line_and_health_are_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.log");
    let mut data = (0..1005)
        .map(|i| line(&format!("h{i}.test")))
        .collect::<String>();
    data.push_str(&"x".repeat(200 * 1024));
    data.push('\n');
    data.push_str(&line("excluded.test"));
    tokio::fs::write(&path, data).await.unwrap();
    let db = test_db_with(&[("NGINX_ACCESS_LOG", path.to_str().unwrap())]).await;
    db.client.batch_execute("INSERT INTO nginx_servers(id,server_name,listen) SELECT 'budget-'||n,'h'||n||'.test',19090 FROM generate_series(0,1004) n").await.unwrap();
    db.client.execute("INSERT INTO app_settings(id,metrics_excluded_hosts) VALUES ('singleton','EXCLUDED.TEST')",&[]).await.unwrap();
    let pass = parse_pass(&db.state, now()).await.unwrap().unwrap();
    assert_eq!(pass.requests, 1005);
    assert_eq!(pass.unparsed, 1);
    assert_eq!(
        db.client
            .query_one("SELECT count(*) FROM metric_hosts", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1001
    );
    assert_eq!(db.client.query_one("SELECT m.requests FROM domain_metric_minutes m JOIN metric_hosts h ON h.id=m.host_id WHERE h.host='__other__'",&[]).await.unwrap().get::<_,i32>(0),5);
    let value = query::metrics_at(&db.state, "24h", 86_400_000, now())
        .await
        .unwrap();
    assert_eq!(value["health"]["unparsedCount"], 1);
    assert_eq!(value["domains"]["series"].as_array().unwrap().len(), 5);
    assert_eq!(value["domains"]["source"], "minutes");
    assert_eq!(value["health"]["parseRatio"], serde_json::Value::Null);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn utc_rollup_is_idempotent_and_retention_keeps_hourly_history() {
    let db = test_db().await;
    {
        let connection = db.state.db.get().await.unwrap();
        connection
            .batch_execute("SET TIME ZONE 'Asia/Kathmandu'")
            .await
            .unwrap();
    }
    db.client.batch_execute("INSERT INTO metric_hosts(host) VALUES ('one.test'); INSERT INTO domain_metric_minutes(bucket,host_id,requests,bytes_sent,s2xx,s3xx,s4xx,s5xx) SELECT x,1,2,42,2,0,0,0 FROM unnest(ARRAY['2026-08-20T00:00:00Z'::timestamptz,'2026-09-22T10:02:00Z'::timestamptz,'2026-09-22T10:59:00Z'::timestamptz]) x").await.unwrap();
    rollup_and_prune(&db.state, now()).await.unwrap();
    rollup_and_prune(&db.state, now()).await.unwrap();
    assert_eq!(count(&db).await, 4);
    let n: i64 = db
        .client
        .query_one("SELECT sum(requests)::int8 FROM domain_metric_hours", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 6);
    let value = query::metrics_at(&db.state, "90d", 90 * 86_400_000, now())
        .await
        .unwrap();
    assert_eq!(value["domains"]["source"], "hours");
    assert_eq!(value["domains"]["series"][0]["total"], 6.0);
    db.client
        .execute(
            "INSERT INTO app_settings(id,metrics_retention_days) VALUES ('singleton',30)",
            &[],
        )
        .await
        .unwrap();
    rollup_and_prune(&db.state, now() + chrono::Duration::hours(2))
        .await
        .unwrap();
    assert_eq!(
        db.client
            .query_one("SELECT sum(requests)::int8 FROM domain_metric_hours", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        4
    );
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn gauge_fake_stub_status_no_synthetic_zero_and_singleton_shutdown() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/status", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener,axum::Router::new().route("/status",axum::routing::get(||async{"Active connections: 2\nserver accepts handled requests\n 100 99 500\nReading: 0 Writing: 1 Waiting: 1\n"}))).await.unwrap()
    });
    let dir = tempfile::tempdir().unwrap();
    let db = test_db_with(&[
        ("NGINX_STATUS_URL", &url),
        (
            "NGINX_ACCESS_LOG",
            dir.path().join("absent").to_str().unwrap(),
        ),
    ])
    .await;
    assert!(collect_gauge(&db.state, now()).await.unwrap());
    assert!(collect_gauge(&db.state, now()).await.unwrap());
    assert_eq!(
        db.client
            .query_one("SELECT count(*) FROM nginx_metric_samples", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    let first = start(db.state.clone());
    let second = start(db.state.clone());
    second.shutdown().await;
    first.shutdown().await;
    server.abort();
    assert!(
        !collect_gauge(&db.state, now() + chrono::Duration::minutes(1))
            .await
            .unwrap()
    );
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn counter_reset_and_gaps_have_null_rates_and_top_host_zero_fill() {
    let db = test_db().await;
    db.client.batch_execute("INSERT INTO nginx_metric_samples(collected_at,active,reading,writing,waiting,accepts,handled,requests) VALUES ('2026-09-22T12:00Z',2,0,1,1,100,100,100),('2026-09-22T12:01Z',2,0,1,1,160,159,220),('2026-09-22T12:02Z',2,0,1,1,1,1,1),('2026-09-22T12:06Z',2,0,1,1,20,20,50)").await.unwrap();
    let value = query::metrics_at(&db.state, "1h", 3_600_000, now())
        .await
        .unwrap();
    let p = value["global"].as_array().unwrap();
    assert_eq!(p.len(), 4);
    assert_eq!(p[1]["requestsPerSec"], 2.0);
    assert!(p[2]["requestsPerSec"].is_null());
    assert!(p[3]["requestsPerSec"].is_null());
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn late_backfill_updates_an_already_rolled_hour_before_minute_pruning() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.log");
    let data = line("late.test").replace("22/Sep/2026:12:00:00", "20/Aug/2026:12:00:00");
    tokio::fs::write(&path, data).await.unwrap();
    let db = test_db_with(&[("NGINX_ACCESS_LOG", path.to_str().unwrap())]).await;
    rollup_and_prune(&db.state, now()).await.unwrap();
    parse_pass(&db.state, now()).await.unwrap();
    rollup_and_prune(&db.state, now() + chrono::Duration::hours(2))
        .await
        .unwrap();
    assert_eq!(count(&db).await, 0);
    assert_eq!(
        db.client
            .query_one(
                "SELECT coalesce(sum(requests),0)::int8 FROM domain_metric_hours",
                &[]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    append(
        &path,
        &line("late.test").replace("22/Sep/2026:12:00:00", "20/Aug/2026:12:00:00"),
    )
    .await;
    parse_pass(&db.state, now() + chrono::Duration::hours(3))
        .await
        .unwrap();
    assert_eq!(
        db.client
            .query_one("SELECT sum(requests)::int8 FROM domain_metric_hours", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        2
    );
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn shutdown_waits_for_inflight_checkpoint_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.log");
    let data = line("shutdown.test").replace(
        "22/Sep/2026:12:00:00",
        &(Utc::now() - chrono::Duration::minutes(5))
            .format("%d/%b/%Y:%H:%M:%S")
            .to_string(),
    );
    tokio::fs::write(&path, data).await.unwrap();
    let db = test_db_with(&[
        ("NGINX_ACCESS_LOG", path.to_str().unwrap()),
        ("NGINX_STATUS_URL", "http://127.0.0.1:1/status"),
    ])
    .await;
    db.client
        .execute("SELECT pg_advisory_lock(683291012)", &[])
        .await
        .unwrap();
    let worker = start(db.state.clone());
    let mut waiting = false;
    for _ in 0..200 {
        waiting=db.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event='advisory')",&[]).await.unwrap().get(0);
        if waiting {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(waiting, "parser must enter its transaction before shutdown");
    let mut stopped = Box::pin(worker.shutdown());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut stopped)
            .await
            .is_err()
    );
    db.client
        .execute("SELECT pg_advisory_unlock(683291012)", &[])
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), stopped)
        .await
        .unwrap();
    assert_eq!(count(&db).await, 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn oversized_line_skip_survives_pass_limit_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.log");
    let mut file = tokio::fs::File::create(&path).await.unwrap();
    let chunk = vec![b'x'; 1024 * 1024];
    for _ in 0..65 {
        file.write_all(&chunk).await.unwrap();
    }
    file.write_all(format!("\n{}", line("after.test")).as_bytes())
        .await
        .unwrap();
    file.flush().await.unwrap();
    drop(file);
    let db = test_db_with(&[("NGINX_ACCESS_LOG", path.to_str().unwrap())]).await;
    let first = parse_pass(&db.state, now()).await.unwrap().unwrap();
    assert_eq!(first.offset, 64 * 1024 * 1024);
    assert_eq!(first.unparsed, 1);
    let second = parse_pass(&db.state, now()).await.unwrap().unwrap();
    assert_eq!(second.requests, 1);
    assert_eq!(second.unparsed, 1);
    assert_eq!(count(&db).await, 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_e6_s17_unknown_hosts_cannot_consume_configured_host_budget() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("access.log");
    let mut data=(0..1005).map(|i|line(&format!("junk-{i}.invalid"))).collect::<String>();data.push_str(&line("legit.test"));tokio::fs::write(&path,data).await.unwrap();
    let db=test_db_with(&[("NGINX_ACCESS_LOG",path.to_str().unwrap())]).await;
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen) VALUES ('s17','legit.test',19090)",&[]).await.unwrap();
    parse_pass(&db.state,now()).await.unwrap().unwrap();
    let unknown:i64=db.client.query_one("SELECT count(*) FROM metric_hosts WHERE host LIKE 'junk-%'",&[]).await.unwrap().get(0);
    assert_eq!(unknown,0,"default-server Host noise must not create metric-host rows");
    let requests:i64=db.client.query_one("SELECT coalesce(sum(m.requests),0)::int8 FROM domain_metric_minutes m JOIN metric_hosts h ON h.id=m.host_id WHERE h.host='legit.test'",&[]).await.unwrap().get(0);assert_eq!(requests,1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_e6_s17_legacy_unknown_rows_do_not_starve_a_new_configured_host() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("access.log");tokio::fs::write(&path,line("new.test")).await.unwrap();
    let db=test_db_with(&[("NGINX_ACCESS_LOG",path.to_str().unwrap())]).await;
    db.client.batch_execute("INSERT INTO metric_hosts(host) SELECT 'old-junk-'||n||'.invalid' FROM generate_series(1,1000) n; INSERT INTO nginx_servers(id,server_name,listen) VALUES ('s17','new.test',19090);").await.unwrap();
    parse_pass(&db.state,now()).await.unwrap().unwrap();
    let requests:i64=db.client.query_one("SELECT coalesce(sum(m.requests),0)::int8 FROM domain_metric_minutes m JOIN metric_hosts h ON h.id=m.host_id WHERE h.host='new.test'",&[]).await.unwrap().get(0);assert_eq!(requests,1,"historical unknown rows must not consume active admission budget");
    assert_eq!(db.client.query_one("SELECT count(*) FROM metric_hosts WHERE host LIKE 'old-junk-%'",&[]).await.unwrap().get::<_,i64>(0),1000,"do not delete historical data to fix admission");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_e6_i20_cached_range_is_available_while_another_range_waits_on_sql() {
    use std::time::Duration;
    let mut db=test_db_with(&[("API_DB_POOL_SIZE","1")]).await;
    let warm=query::metrics(&db.state,Some("24h")).await.unwrap();
    let tx=db.client.transaction().await.unwrap();tx.batch_execute("LOCK TABLE nginx_metric_samples IN ACCESS EXCLUSIVE MODE").await.unwrap();
    let state=db.state.clone();let waiting=tokio::spawn(async move{query::metrics(&state,Some("1h")).await});
    let reached=tokio::time::timeout(Duration::from_secs(3),async{loop{tx.execute("SELECT pg_stat_clear_snapshot()",&[]).await.unwrap();let blocked:bool=tx.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid() AND wait_event_type='Lock')",&[]).await.unwrap().get(0);if blocked{break;}tokio::time::sleep(Duration::from_millis(10)).await;}}).await.is_ok();
    let cached=tokio::time::timeout(Duration::from_millis(300),query::metrics(&db.state,Some("24h"))).await;
    tx.rollback().await.unwrap();waiting.await.unwrap().unwrap();
    assert!(reached,"cold range must reach the real SQL lock before probing the hot range");
    assert_eq!(cached.expect("cache lookup must not wait behind another range's SQL").unwrap(),warm);
}

#[test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
fn audit_e6_i20_log_scan_does_not_hold_the_only_pool_connection() {
    use std::{future::{Future, poll_fn}, task::Poll, time::Duration};
    tokio::runtime::Builder::new_current_thread().enable_all().max_blocking_threads(1).build().unwrap().block_on(async {
        let dir=tempfile::tempdir().unwrap(); let path=dir.path().join("access.log");
        tokio::fs::write(&path,line("scan.test")).await.unwrap();
        let db=test_db_with(&[("API_DB_POOL_SIZE","1"),("NGINX_ACCESS_LOG",path.to_str().unwrap())]).await;
        assert_eq!(parse_pass(&db.state,now()).await.unwrap().unwrap().requests,1);
        let additional = 100_000;
        append(&path,&line("scan.test").repeat(additional)).await;
        assert!(tokio::fs::metadata(&path).await.unwrap().len() > 8 * 1024 * 1024);
        let (release,parked)=std::sync::mpsc::channel::<()>();
        let (started,ready)=tokio::sync::oneshot::channel();
        let blocker=tokio::task::spawn_blocking(move || {let _=started.send(());let _=parked.recv();});
        ready.await.unwrap();
        let held=db.state.db.get().await.unwrap();
        let (yielded,first_yield)=tokio::sync::oneshot::channel();
        let state=db.state.clone();
        let scan=tokio::spawn(async move {
            let mut pass=Box::pin(parse_pass(&state,now()));let mut yielded=Some(yielded);
            poll_fn(|cx| {let result=pass.as_mut().poll(cx);if matches!(result,Poll::Pending){if let Some(tx)=yielded.take(){let _=tx.send(());}}result}).await
        });
        first_yield.await.unwrap();drop(held);
        let available=tokio::time::timeout(Duration::from_millis(300),db.state.db.get()).await;
        let pool_free=matches!(&available,Ok(Ok(_)));
        if let Ok(Ok(client))=&available {
            client.batch_execute("SET TIME ZONE 'Asia/Kathmandu'").await.unwrap();
        }
        drop(available);
        let scanning=!scan.is_finished();drop(release);blocker.await.unwrap();
        assert_eq!(scan.await.unwrap().unwrap().unwrap().requests,additional as i64);
        assert_eq!(count(&db).await,additional as i64 + 1,"checkpoint CAS must be invariant to DB session time zone changes");
        assert!(scanning,"the scan must still be waiting for the blocked filesystem worker");
        assert!(pool_free,"log filesystem I/O must release the only pooled connection");
    });
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_e6_s17_only_enabled_configured_names_are_admitted() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("access.log");
    let data=["EXACT.test:80","a.wild.test","both.test","a.both.test","mail.example","disabled.test","wild.test","notboth.test"].into_iter().map(line).collect::<String>();
    tokio::fs::write(&path,data).await.unwrap();
    let db=test_db_with(&[("NGINX_ACCESS_LOG",path.to_str().unwrap())]).await;
    db.client.batch_execute("INSERT INTO nginx_servers(id,server_name,listen,enabled) VALUES ('on','exact.test *.wild.test .both.test mail.*',19090,true),('off','disabled.test',19091,false)").await.unwrap();
    assert_eq!(parse_pass(&db.state,now()).await.unwrap().unwrap().requests,8);
    let rows=db.client.query("SELECT host FROM metric_hosts WHERE host<>'__other__' ORDER BY host",&[]).await.unwrap();
    let hosts=rows.iter().map(|row|row.get::<_,String>(0)).collect::<Vec<_>>();
    assert_eq!(hosts,["a.both.test","a.wild.test","both.test","exact.test","mail.example"]);
}

#[test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
fn audit_e6_i20_moved_checkpoint_discards_a_stale_scan() {
    use std::{future::{Future, poll_fn}, task::Poll, time::Duration};
    tokio::runtime::Builder::new_current_thread().enable_all().max_blocking_threads(1).build().unwrap().block_on(async {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("access.log");
        tokio::fs::write(&path,line("cas.test")).await.unwrap();
        let mut db=test_db_with(&[("API_DB_POOL_SIZE","1"),("NGINX_ACCESS_LOG",path.to_str().unwrap())]).await;
        let (release,parked)=std::sync::mpsc::channel::<()>();
        let (started,ready)=tokio::sync::oneshot::channel();
        let blocker=tokio::task::spawn_blocking(move || {let _=started.send(());let _=parked.recv();});ready.await.unwrap();
        let held=db.state.db.get().await.unwrap();
        let (yielded,first_yield)=tokio::sync::oneshot::channel();let state=db.state.clone();
        let scan=tokio::spawn(async move {
            let mut pass=Box::pin(parse_pass(&state,now()));let mut yielded=Some(yielded);
            poll_fn(|cx| {let result=pass.as_mut().poll(cx);if matches!(result,Poll::Pending){if let Some(tx)=yielded.take(){let _=tx.send(());}}result}).await
        });first_yield.await.unwrap();drop(held);
        // Fair pool handoff proves the collector's short snapshot has released its client.
        let probe=tokio::time::timeout(Duration::from_secs(3),db.state.db.get()).await.unwrap().unwrap();drop(probe);
        let tx=db.client.transaction().await.unwrap();
        tx.execute("SELECT pg_advisory_xact_lock(683291012)",&[]).await.unwrap();
        tx.execute("INSERT INTO metric_parse_state(id,\"offset\",unparsed_count) VALUES ('current',123,7)",&[]).await.unwrap();
        tx.commit().await.unwrap();drop(release);blocker.await.unwrap();
        assert!(scan.await.unwrap().unwrap().is_none(),"a different process's checkpoint must win");
        assert_eq!(count(&db).await,0,"a discarded scan writes no counters");
        let row=db.client.query_one("SELECT \"offset\",unparsed_count FROM metric_parse_state WHERE id='current'",&[]).await.unwrap();
        assert_eq!(row.get::<_,i64>(0),123);assert_eq!(row.get::<_,i64>(1),7);
    });
}

async fn checkpoint_tuple(db: &TestDb) -> (String, String, DateTime<Utc>) {
    let row = db.client.query_one("SELECT xmin::text,xmax::text,updated_at FROM metric_parse_state WHERE id='current'",&[]).await.unwrap();
    (row.get(0), row.get(1), row.get(2))
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn idle_pass_leaves_the_checkpoint_row_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.log");
    tokio::fs::write(&path,line("idle.test")).await.unwrap();
    let db = test_db_with(&[("NGINX_ACCESS_LOG",path.to_str().unwrap())]).await;
    assert_eq!(parse_pass(&db.state,now()).await.unwrap().unwrap().requests,1);
    let before = checkpoint_tuple(&db).await;
    assert_eq!(parse_pass(&db.state,now()+chrono::Duration::seconds(10)).await.unwrap().unwrap().requests,0);
    assert_eq!(checkpoint_tuple(&db).await,before,"idle pass must neither update nor lock the checkpoint row");
    append(&path,&line("idle.test")).await;
    assert_eq!(parse_pass(&db.state,now()+chrono::Duration::seconds(20)).await.unwrap().unwrap().requests,1);
    assert_ne!(checkpoint_tuple(&db).await.0,before.0);
    assert_eq!(count(&db).await,2);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn grace_held_line_is_counted_after_grace_without_appending_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.log");
    let data = line("grace.test").replace("22/Sep/2026:12:00:00","22/Sep/2026:12:10:00");
    tokio::fs::write(&path,&data).await.unwrap();
    let db = test_db_with(&[("NGINX_ACCESS_LOG",path.to_str().unwrap())]).await;
    let held = parse_pass(&db.state,now()).await.unwrap().unwrap();
    assert_eq!((held.offset,held.requests),(0,0));
    let row = db.client.query_one("SELECT \"offset\",size FROM metric_parse_state WHERE id='current'",&[]).await.unwrap();
    assert_eq!((row.get::<_,i64>(0),row.get::<_,i64>(1)),(0,data.len() as i64),"persist the held line's size without consuming it");
    let before = checkpoint_tuple(&db).await;
    let held = parse_pass(&db.state,now()+chrono::Duration::seconds(30)).await.unwrap().unwrap();
    assert_eq!((held.offset,held.requests),(0,0));
    assert_eq!(checkpoint_tuple(&db).await,before,"inside grace, an unchanged held line writes nothing");
    assert_eq!(count(&db).await,0);
    let consumed = parse_pass(&db.state,now()+chrono::Duration::seconds(150)).await.unwrap().unwrap();
    assert_eq!((consumed.offset,consumed.requests),(data.len() as u64,1));
    assert_ne!(checkpoint_tuple(&db).await.0,before.0);
    assert_eq!(parse_pass(&db.state,now()+chrono::Duration::seconds(180)).await.unwrap().unwrap().requests,0);
    assert_eq!(count(&db).await,1);
}
