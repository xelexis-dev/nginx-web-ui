mod outbound_support;
mod auth_support;
use auth_support::*;
use chrono::Utc;
use nwui_api::notifications::queue::*;
use std::sync::{Arc, Mutex};
async fn receiver() -> (
    String,
    Arc<Mutex<Vec<axum::http::HeaderMap>>>,
    tokio::task::JoinHandle<()>,
) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let records = seen.clone();
    let app = axum::Router::new().route(
        "/hook",
        axum::routing::post(move |headers: axum::http::HeaderMap| {
            let records = records.clone();
            async move {
                records.lock().unwrap().push(headers);
                axum::http::StatusCode::NO_CONTENT
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, seen, task)
}
async fn target(db: &TestDb, url: &str) {
    let sealed = db.state.secrets.seal(url).unwrap();
    db.client.execute("INSERT INTO webhook_targets(id,name,url_enc,events) VALUES ('w','test',$1,'[\"cert.issued\",\"cert.renewal_due\"]')",&[&sealed]).await.unwrap();
}
async fn event(db: &TestDb) {
    db.client.execute("INSERT INTO notifications(id,type,severity,title) VALUES ('n','cert.issued','success','issued')",&[]).await.unwrap();
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn planner_and_workers_deliver_each_target_once_under_concurrency() {
    let db = test_db().await;
    let (url, seen, server) = receiver().await;
    target(&db, &url).await;
    event(&db).await;
    let now = Utc::now();
    let (a, b) = tokio::join!(plan(&db.state, now), plan(&db.state, now));
    assert_eq!(a.unwrap() + b.unwrap(), 1);
    let (a, b) = tokio::join!(dispatch_once(&db.state, now), dispatch_once(&db.state, now));
    assert_eq!(a.unwrap() + b.unwrap(), 1);
    assert_eq!(dispatch_once(&db.state, now).await.unwrap(), 0);
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(seen.lock().unwrap()[0].contains_key("idempotency-key"));
    assert_eq!(
        db.client
            .query_one("SELECT status FROM notification_deliveries", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "delivered"
    );
    server.abort();
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn durable_expired_lease_recovery_and_failed_channel_never_resends_success() {
    let db = test_db().await;
    let (url, seen, server) = receiver().await;
    target(&db, &url).await;
    event(&db).await;
    db.client.execute("INSERT INTO mail_recipients(id,address,events) VALUES ('m','to@example.test','[\"cert.issued\"]')",&[]).await.unwrap();
    let now = Utc::now();
    assert_eq!(plan(&db.state, now).await.unwrap(), 2);
    db.client.execute("UPDATE notification_deliveries SET status='delivering',lease_owner='dead-worker',lease_until=$1",&[&(now-chrono::Duration::seconds(1))]).await.unwrap();
    assert_eq!(dispatch_once(&db.state, now).await.unwrap(), 2);
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(
        dispatch_once(&db.state, now + chrono::Duration::hours(1))
            .await
            .unwrap(),
        1
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    let rows = db
        .client
        .query(
            "SELECT channel,status,last_error FROM notification_deliveries ORDER BY channel",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows[0].get::<_, String>("status"), "pending");
    assert_eq!(rows[1].get::<_, String>("status"), "delivered");
    assert!(
        db.client
            .query_one("SELECT last_error FROM mail_recipients WHERE id='m'", &[])
            .await
            .unwrap()
            .get::<_, Option<String>>(0)
            .is_none(),
        "global SMTP configuration failure is not a recipient failure"
    );
    server.abort();
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn manual_dns_due_sweep_deduplicates_and_marks_only_actual_delivery() {
    let db = test_db().await;
    let now = Utc::now();
    let expires = now + chrono::Duration::days(12);
    db.client.execute("INSERT INTO certificates(id,domains,status,source,challenge_type,expires_at) VALUES ('c','[\"example.test\"]','issued','acme','dns-01',$1)",&[&expires]).await.unwrap();
    let (a, b) = tokio::join!(sweep_due(&db.state, now), sweep_due(&db.state, now));
    assert_eq!(a.unwrap() + b.unwrap(), 1);
    assert_eq!(plan(&db.state, now).await.unwrap(), 0);
    assert!(
        db.client
            .query_one(
                "SELECT renewal_notice_sent_at FROM certificates WHERE id='c'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, Option<chrono::DateTime<Utc>>>(0)
            .is_none()
    );
    let (url, seen, server) = receiver().await;
    target(&db, &url).await;
    assert_eq!(plan(&db.state, now).await.unwrap(), 1);
    assert_eq!(dispatch_once(&db.state, now).await.unwrap(), 1);
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(
        sweep_due(&db.state, now + chrono::Duration::hours(12))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sweep_due(&db.state, now + chrono::Duration::days(8))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.client
            .query_one("SELECT count(*) FROM notifications", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        2
    );
    server.abort();
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn renewal_cancels_stale_unsent_due_deliveries() {
    let db = test_db().await;
    let now = Utc::now();
    let expires = now + chrono::Duration::days(5);
    let (url, seen, server) = receiver().await;
    target(&db, &url).await;
    db.client.execute("INSERT INTO certificates(id,domains,status,source,challenge_type,expires_at) VALUES ('c','[\"example.test\"]','issued','acme','dns-01',$1)",&[&expires]).await.unwrap();
    sweep_due(&db.state, now).await.unwrap();
    plan(&db.state, now).await.unwrap();
    db.client
        .execute(
            "UPDATE certificates SET expires_at=$1 WHERE id='c'",
            &[&(now + chrono::Duration::days(90))],
        )
        .await
        .unwrap();
    dispatch_once(&db.state, now).await.unwrap();
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(
        db.client
            .query_one("SELECT status FROM notification_deliveries", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "cancelled"
    );
    server.abort();
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn planning_failure_rolls_back_receipts_and_keeps_event_discoverable() {
    let db = test_db().await;
    target(&db, "http://127.0.0.1:1/hook").await;
    event(&db).await;
    db.client.batch_execute("CREATE FUNCTION reject_plan() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test'; END $$; CREATE TRIGGER reject_plan BEFORE UPDATE ON notifications FOR EACH ROW EXECUTE FUNCTION reject_plan();").await.unwrap();
    assert!(plan(&db.state, Utc::now()).await.is_err());
    assert_eq!(
        db.client
            .query_one("SELECT count(*) FROM notification_deliveries", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert!(
        db.client
            .query_one("SELECT delivery_planned_at FROM notifications", &[])
            .await
            .unwrap()
            .get::<_, Option<chrono::DateTime<Utc>>>(0)
            .is_none()
    );
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn webhook_retry_uses_stable_id_and_never_persists_response_secrets() {
    let db = test_db().await;
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed = seen.clone();
    let app = axum::Router::new().route(
        "/secret-path",
        axum::routing::post(move |headers: axum::http::HeaderMap| {
            let observed = observed.clone();
            async move {
                let mut log = observed.lock().unwrap();
                log.push(headers["idempotency-key"].to_str().unwrap().to_owned());
                (
                    if log.len() == 1 {
                        axum::http::StatusCode::SERVICE_UNAVAILABLE
                    } else {
                        axum::http::StatusCode::NO_CONTENT
                    },
                    "response-secret-value",
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let url = format!("http://{}/secret-path", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    target(&db, &url).await;
    event(&db).await;
    let now = Utc::now();
    plan(&db.state, now).await.unwrap();
    dispatch_once(&db.state, now).await.unwrap();
    let error: String = db
        .client
        .query_one("SELECT last_error FROM notification_deliveries", &[])
        .await
        .unwrap()
        .get(0);
    assert!(error.contains("503"));
    assert!(!error.contains("secret"));
    dispatch_once(&db.state, now + chrono::Duration::minutes(1))
        .await
        .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 2);
    {
        let records = seen.lock().unwrap();
        assert_eq!(records[0], records[1]);
    }
    server.abort();
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn shutdown_drains_delivery_and_commits_receipt() {
    let db = test_db().await;
    let began = Arc::new(tokio::sync::Notify::new());
    let signal = began.clone();
    let app = axum::Router::new().route(
        "/hook",
        axum::routing::post(move || {
            let signal = signal.clone();
            async move {
                signal.notify_one();
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                axum::http::StatusCode::NO_CONTENT
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    target(&db, &url).await;
    event(&db).await;
    let handle = nwui_api::notifications::start(db.state.clone());
    tokio::time::timeout(std::time::Duration::from_secs(3), began.notified())
        .await
        .unwrap();
    let mut stopped = Box::pin(handle.shutdown());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut stopped)
            .await
            .is_err()
    );
    tokio::time::timeout(std::time::Duration::from_secs(3), stopped)
        .await
        .unwrap();
    assert_eq!(
        db.client
            .query_one("SELECT status FROM notification_deliveries", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "delivered"
    );
    server.abort();
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn retention_is_bounded_and_preserves_inflight_receipts() {
    let db = test_db().await;
    let now = Utc::now();
    let old = now - chrono::Duration::days(91);
    db.client.execute("INSERT INTO notifications(id,type,severity,title,created_at) SELECT 'old'||n,'cert.issued','success','old',$1 FROM generate_series(1,20501) n",&[&old]).await.unwrap();
    event(&db).await;
    db.client.execute("INSERT INTO notification_deliveries(id,notification_id,channel,target_id,status,lease_owner,lease_until) VALUES ('active','old1','webhook','w','delivering','worker',$1)",&[&(now+chrono::Duration::minutes(1))]).await.unwrap();
    assert_eq!(prune(&db.state, now).await.unwrap(), 20000);
    assert_eq!(prune(&db.state, now).await.unwrap(), 500);
    assert_eq!(
        db.client
            .query_one("SELECT count(*) FROM notifications", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        2
    );
    db.client
        .execute(
            "UPDATE notification_deliveries SET status='delivered' WHERE id='active'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(prune(&db.state, now).await.unwrap(), 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn unresolved_due_reminder_discovers_new_channel_without_duplicate_feed_rows() {
    let db = test_db().await;
    let now = Utc::now();
    let expires = now + chrono::Duration::days(12);
    db.client.execute("INSERT INTO certificates(id,domains,status,source,challenge_type,expires_at) VALUES ('c','[\"example.test\"]','issued','acme','dns-01',$1)",&[&expires]).await.unwrap();
    db.client.execute("INSERT INTO mail_recipients(id,address,events) VALUES ('m','to@example.test','[\"cert.renewal_due\"]')",&[]).await.unwrap();
    sweep_due(&db.state, now).await.unwrap();
    plan(&db.state, now).await.unwrap();
    dispatch_once(&db.state, now).await.unwrap();
    let (url, seen, server) = receiver().await;
    target(&db, &url).await;
    let later = now + chrono::Duration::hours(12);
    assert_eq!(sweep_due(&db.state, later).await.unwrap(), 0);
    assert_eq!(plan(&db.state, later).await.unwrap(), 1);
    dispatch_once(&db.state, later).await.unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(
        db.client
            .query_one("SELECT count(*) FROM notifications", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    server.abort();
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_concurrency_queued_webhooks_reuse_the_same_keep_alive_connection() {
    use axum::extract::ConnectInfo;
    use std::net::SocketAddr;
    let db = test_db().await;
    let peers = Arc::new(Mutex::new(Vec::new()));
    let seen = peers.clone();
    let app = axum::Router::new().route("/hook", axum::routing::post(move |ConnectInfo(peer): ConnectInfo<SocketAddr>| {
        let seen = seen.clone();
        async move { seen.lock().unwrap().push(peer); axum::http::StatusCode::NO_CONTENT }
    }));
    let listener = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    target(&db, &format!("http://{}/hook", listener.local_addr().unwrap())).await;
    let server = tokio::spawn(async move { axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap() });
    for id in ["first", "second"] {
        db.client.execute("INSERT INTO notifications(id,type,severity,title) VALUES ($1,'cert.issued','success','issued')", &[&id]).await.unwrap();
        let now = Utc::now();
        assert_eq!(plan(&db.state, now).await.unwrap(), 1);
        assert_eq!(dispatch_once(&db.state, now).await.unwrap(), 1);
    }
    let connections = peers.lock().unwrap().clone();
    server.abort();
    assert_eq!(connections.len(), 2);
    assert_eq!(connections[0], connections[1], "each delivery rebuilt its client and discarded the keep-alive pool");
}
