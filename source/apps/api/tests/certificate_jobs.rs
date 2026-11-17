mod auth_support;
use nwui_api::{
    certificates::{jobs, service},
    contracts::Actor,
};
use serde_json::json;
use std::time::Duration;
fn actor() -> Actor {
    Actor {
        via: None,
        id: None,
        label: Some("test".into()),
        ip: None,
    }
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn cancelled_queued_job_cleans_http_tokens_and_erases_snapshot() {
    let t = auth_support::test_db().await;
    let input =
        service::parse_input(&json!({"domains":["example.com"],"email":"admin@example.com"}))
            .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    t.client
        .execute(
            "UPDATE certificate_jobs SET http_tokens='[\"owned-token\"]' WHERE certificate_id=$1",
            &[&id],
        )
        .await
        .unwrap();
    t.client.execute("INSERT INTO acme_challenges(token,key_authorization) VALUES('owned-token','owned-value'),('other-token','other-value')",&[]).await.unwrap();
    service::cancel(&t.state, &id, &actor()).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    for _ in 0..100 {
        let status: String = t
            .client
            .query_one(
                "SELECT state FROM certificate_jobs WHERE certificate_id=$1",
                &[&id],
            )
            .await
            .unwrap()
            .get(0);
        if status == "cancelled" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    worker.shutdown().await.unwrap();
    let row = t
        .client
        .query_one(
            "SELECT state,payload_enc,http_tokens FROM certificate_jobs WHERE certificate_id=$1",
            &[&id],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "cancelled");
    assert_eq!(row.get::<_, String>(1), "");
    assert_eq!(row.get::<_, serde_json::Value>(2), json!([]));
    let remaining = t
        .client
        .query("SELECT token FROM acme_challenges", &[])
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].get::<_, String>(0), "other-token");
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn restart_preserves_unsubmitted_manual_order_and_recovers_submitted_order() {
    let t = auth_support::test_db().await;
    let input = service::parse_input(
        &json!({"domains":["example.com"],"email":"admin@example.com","challengeType":"dns-01"}),
    )
    .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    let order = json!({"order":{"url":"http://localhost/order/1"},"csrPem":"csr","keyPem":"enc:v1:sealed","records":[],"submittedAt":"2026-09-23T00:00:00Z"});
    t.client
        .execute(
            "UPDATE certificates SET dns_order=$2 WHERE id=$1",
            &[&id, &order],
        )
        .await
        .unwrap();
    t.client.execute("UPDATE certificate_jobs SET kind='dns_complete',state='active' WHERE certificate_id=$1",&[&id]).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    worker.shutdown().await.unwrap();
    let row = t
        .client
        .query_one(
            "SELECT status::text,dns_order,last_error,issue_log FROM certificates WHERE id=$1",
            &[&id],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "awaiting-dns");
    assert!(
        row.get::<_, serde_json::Value>(1)
            .get("submittedAt")
            .is_none()
    );
    assert!(row.get::<_, Option<String>>(2).unwrap().contains("재시작"));
    assert_eq!(row.get::<_, serde_json::Value>(3)[0]["level"], "info");
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn restart_preserves_failed_and_unsubmitted_manual_states() {
    let t = auth_support::test_db().await;
    let order = json!({"order":{"url":"https://ca.test/1"},"keyPem":"enc:v1:opaque"});
    t.client.execute("INSERT INTO certificates(id,domains,status,challenge_type,dns_order,last_attempt_at,last_error) VALUES('failed','[]','failed','dns-01',$1,now(),'original failure'),('waiting','[]','awaiting-dns','dns-01',$1,now(),NULL)",&[&order]).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    worker.shutdown().await.unwrap();
    let row=t.client.query_one("SELECT status::text,dns_order,last_error,issue_log FROM certificates WHERE id='failed'",&[]).await.unwrap();
    assert_eq!(row.get::<_, String>(0), "failed");
    assert_eq!(row.get::<_, serde_json::Value>(1), order);
    assert_eq!(row.get::<_, String>(2), "original failure");
    assert_eq!(row.get::<_, serde_json::Value>(3), json!([]));
    let row = t
        .client
        .query_one(
            "SELECT dns_order,last_error,issue_log FROM certificates WHERE id='waiting'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, serde_json::Value>(0), order);
    assert!(row.get::<_, Option<String>>(1).is_none());
    assert_eq!(row.get::<_, serde_json::Value>(2), json!([]));
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn restart_honors_ca_order_expiry_over_local_age() {
    let t = auth_support::test_db().await;
    let future = json!({"order":{"url":"https://ca.test/future","expires":(chrono::Utc::now()+chrono::Duration::days(1)).to_rfc3339()}});
    let expired = json!({"order":{"url":"https://ca.test/past","expires":(chrono::Utc::now()-chrono::Duration::minutes(1)).to_rfc3339()}});
    t.client.execute("INSERT INTO certificates(id,domains,status,challenge_type,dns_order,last_attempt_at,cert_pem,key_pem) VALUES('future','[]','awaiting-dns','dns-01',$1,now()-interval '8 days',NULL,NULL),('expired','[]','awaiting-dns','dns-01',$2,now(),NULL,NULL),('renew','[]','issued','dns-01',$2,now(),'live cert','live key')",&[&future,&expired]).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    worker.shutdown().await.unwrap();
    let row = t
        .client
        .query_one(
            "SELECT status::text,dns_order FROM certificates WHERE id='future'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "awaiting-dns");
    assert_eq!(row.get::<_, serde_json::Value>(1), future);
    let row=t.client.query_one("SELECT status::text,dns_order,last_error,issue_log FROM certificates WHERE id='expired'",&[]).await.unwrap();
    assert_eq!(row.get::<_, String>(0), "failed");
    assert!(row.get::<_, Option<serde_json::Value>>(1).is_none());
    assert!(row.get::<_, String>(2).contains("만료"));
    assert!(
        !row.get::<_, serde_json::Value>(3)
            .as_array()
            .unwrap()
            .is_empty()
    );
    let row = t
        .client
        .query_one(
            "SELECT status::text,dns_order,cert_pem,key_pem FROM certificates WHERE id='renew'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "issued");
    assert!(row.get::<_, Option<serde_json::Value>>(1).is_none());
    assert_eq!(row.get::<_, String>(2), "live cert");
    assert_eq!(row.get::<_, String>(3), "live key");
}

async fn wait_terminal(t: &auth_support::TestDb, id: &str) -> String {
    for _ in 0..250 {
        let status: String = t
            .client
            .query_one(
                "SELECT state FROM certificate_jobs WHERE certificate_id=$1",
                &[&id],
            )
            .await
            .unwrap()
            .get(0);
        if matches!(status.as_str(), "succeeded" | "failed" | "cancelled") {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("job failed to settle")
}
async fn mock_ca() -> (String, tokio::task::JoinHandle<()>) {
    let (base, task, _) = mock_ca_with(None).await;
    (base, task)
}
/// `slow` holds one CA path open for a while and signals when it is reached.
async fn mock_ca_with(
    slow: Option<(&'static str, Duration)>,
) -> (String, tokio::task::JoinHandle<()>, std::sync::Arc<tokio::sync::Notify>) {
    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, Uri},
    };
    use std::sync::Arc;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let reached = Arc::new(tokio::sync::Notify::new());
    type Ca = (String, Option<(&'static str, Duration)>, Arc<tokio::sync::Notify>);
    let app=Router::new().fallback(|State((base,slow,reached)):State<Ca>,uri:Uri|async move{
  if let Some((path,delay))=slow{if uri.path()==path{reached.notify_one();tokio::time::sleep(delay).await;}}
  let mut headers=HeaderMap::new();headers.insert("replay-nonce","dGVzdC1ub25jZQ".parse().unwrap());
  let result=match uri.path(){"/directory"=>json!({"newNonce":format!("{base}/nonce"),"newAccount":format!("{base}/account"),"newOrder":format!("{base}/new-order")}),"/account"=>{headers.insert("location",format!("{base}/account/1").parse().unwrap());json!({"status":"valid","orders":format!("{base}/orders")})},"/new-order"|"/order/1"=>{headers.insert("location",format!("{base}/order/1").parse().unwrap());json!({"status":"pending","identifiers":[{"type":"dns","value":"example.com"}],"authorizations":[format!("{base}/auth/1")],"finalize":format!("{base}/finalize")})},"/auth/1"=>json!({"identifier":{"type":"dns","value":"example.com"},"status":"pending","challenges":[{"type":"dns-01","url":format!("{base}/challenge/dns"),"status":"pending","token":"job-token"}]}),_=>json!({"status":"valid"})};(headers,Json(result))
 }).with_state((base.clone(),slow,reached.clone()));
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, task, reached)
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn worker_manual_dns_begin_persists_sealed_order_and_releases_queue() {
    let (base, ca) = mock_ca().await;
    let directory = format!("{base}/directory");
    let t = auth_support::test_db_with(&[("ACME_DIRECTORY_URL", &directory)]).await;
    let input = service::parse_input(
        &json!({"domains":["example.com"],"email":"admin@example.com","challengeType":"dns-01"}),
    )
    .unwrap();
    auth_support::insert_user(&t.client,auth_support::UserRow::default(),None).await;
    let mut token_actor=actor(); token_actor.id=Some("u1".into());
    token_actor.via=Some(nwui_api::contracts::Via::Token{id:"job-token-id".into(),name:"job-token".into()});
    let id = service::create(&t.state, &input, &token_actor).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    assert_eq!(wait_terminal(&t, &id).await, "succeeded");
    worker.shutdown().await.unwrap();
    let row = t
        .client
        .query_one(
            "SELECT status::text,dns_order,cert_pem,key_pem FROM certificates WHERE id=$1",
            &[&id],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "awaiting-dns");
    let order: serde_json::Value = row.get(1);
    assert_eq!(order["order"]["url"], format!("{base}/order/1"));
    assert!(order["keyPem"].as_str().unwrap().starts_with("enc:v1:"));
    assert!(row.get::<_, Option<String>>(2).is_none());
    assert!(row.get::<_, Option<String>>(3).is_none());
    assert_eq!(t.client.query_one("SELECT via FROM audit_logs WHERE action='cert:dns-begin' AND target=$1",&[&id]).await.unwrap().get::<_,serde_json::Value>(0),json!({"kind":"token","id":"job-token-id","name":"job-token"}));
    assert_eq!(t.client.query_one("SELECT actor_id FROM audit_logs WHERE action='cert:dns-begin' AND target=$1",&[&id]).await.unwrap().get::<_,String>(0),"u1");
    ca.abort();
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn cancelled_cloudflare_create_with_lost_response_recovers_only_owned_record() {
    use axum::{
        Json, Router,
        extract::State,
        http::{Method, Uri},
    };
    use std::sync::{Arc, Mutex};
    let (base, ca) = mock_ca().await;
    let directory = format!("{base}/directory");
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cf_base = format!("http://{}", socket.local_addr().unwrap());
    let data = Arc::new(Mutex::new((
        None::<serde_json::Value>,
        Vec::<String>::new(),
    )));
    let started = Arc::new(tokio::sync::Notify::new());
    let app=Router::new().fallback({let started=started.clone();move |State(data):State<Arc<Mutex<(Option<serde_json::Value>,Vec<String>)>>>,method:Method,uri:Uri,body:axum::body::Bytes|{let started=started.clone();async move{
  if uri.path()=="/zones"{return Json(json!({"success":true,"result":[{"id":"z","name":"example.com"}]}));}
  if method==Method::POST{let mut record:serde_json::Value=serde_json::from_slice(&body).unwrap();record["id"]=json!("owned-record");started.notify_one();tokio::time::sleep(Duration::from_millis(150)).await;data.lock().unwrap().0=Some(record);return Json(json!({"success":false,"errors":[{"message":"lost create response"}]}));}
  if method==Method::DELETE{data.lock().unwrap().1.push(uri.path().into());return Json(json!({"success":true,"result":{}}));}
  let own=data.lock().unwrap().0.clone().unwrap();let mut other=own.clone();other["id"]=json!("unrelated-record");other["comment"]=json!("another-job");Json(json!({"success":true,"result":[own,other],"result_info":{"total_pages":1}}))
 }}}).with_state(data.clone());
    let cf = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
    let t = auth_support::test_db_with(&[
        ("ACME_DIRECTORY_URL", &directory),
        ("CF_API_BASE", &cf_base),
    ])
    .await;
    let input=service::parse_input(&json!({"domains":["example.com"],"email":"admin@example.com","challengeType":"dns-01","dnsProvider":"cloudflare","dnsToken":"test-token"})).unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), started.notified())
        .await
        .unwrap();
    service::cancel(&t.state, &id, &actor()).await.unwrap();
    assert_eq!(wait_terminal(&t, &id).await, "cancelled");
    worker.shutdown().await.unwrap();
    assert_eq!(
        data.lock().unwrap().1,
        vec!["/zones/z/dns_records/owned-record"]
    );
    let row = t
        .client
        .query_one(
            "SELECT cf_records,payload_enc FROM certificate_jobs WHERE certificate_id=$1",
            &[&id],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, serde_json::Value>(0), json!([]));
    assert_eq!(row.get::<_, String>(1), "");
    cf.abort();
    ca.abort();
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn unknown_cloudflare_create_is_retained_when_record_is_not_yet_visible() {
    use axum::{Json, Router};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let gets = Arc::new(AtomicUsize::new(0));
    let copy = gets.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(move || {
        let gets = copy.clone();
        async move {
            gets.fetch_add(1, Ordering::SeqCst);
            Json(json!({"success":true,"result":[],"result_info":{"total_pages":1}}))
        }
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let t = auth_support::test_db_with(&[("CF_API_BASE", &base)]).await;
    let input =
        service::parse_input(&json!({"domains":["example.com"],"email":"admin@example.com"}))
            .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    let row = t
        .client
        .query_one(
            "SELECT payload_enc FROM certificate_jobs WHERE certificate_id=$1",
            &[&id],
        )
        .await
        .unwrap();
    let enc: String = row.get(0);
    let mut payload: serde_json::Value =
        serde_json::from_str(&t.state.secrets.open(Some(&enc)).unwrap()).unwrap();
    payload["dns_token"] = json!("private-test-token");
    let sealed = t.state.secrets.seal(&payload.to_string()).unwrap();
    t.client.execute("UPDATE certificate_jobs SET state='cleanup',updated_at=now()-interval '1 minute',payload_enc=$2,cf_records=$3 WHERE certificate_id=$1",&[&id,&sealed,&json!([{"zone_id":"z","name":"_acme-challenge.example.com","value":"value","owner":"nwui:unknown:intent"}])]).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    worker.shutdown().await.unwrap();
    let row = t
        .client
        .query_one(
            "SELECT state,payload_enc,cf_records FROM certificate_jobs WHERE certificate_id=$1",
            &[&id],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "cleanup");
    assert!(row.get::<_, String>(1).starts_with("enc:v1:"));
    assert_eq!(
        row.get::<_, serde_json::Value>(2).as_array().unwrap().len(),
        1
    );
    assert_eq!(
        gets.load(Ordering::SeqCst),
        1,
        "unknown outcome is retried with backoff, never busy-looped or cleared"
    );
    server.abort();
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn database_ownership_lock_prevents_two_api_workers() {
    let t = auth_support::test_db().await;
    let first = jobs::start(t.state.clone()).await.unwrap();
    let mut other = t.state.clone();
    other.runtime = std::sync::Arc::new(Default::default());
    assert!(jobs::start(other.clone()).await.is_err());
    first.shutdown().await.unwrap();
    jobs::start(other).await.unwrap().shutdown().await.unwrap();
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn http_cleanup_waits_for_prior_transaction_commit() {
    use nwui_api::certificates::protocol::HttpChallengeStore;
    let t = auth_support::test_db().await;
    let input =
        service::parse_input(&json!({"domains":["example.com"],"email":"admin@example.com"}))
            .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    let job: String = t
        .client
        .query_one(
            "SELECT id FROM certificate_jobs WHERE certificate_id=$1",
            &[&id],
        )
        .await
        .unwrap()
        .get(0);
    let mut connection = t.state.db.get().await.unwrap();
    let tx = connection.transaction().await.unwrap();
    tx.query_one(
        "SELECT id FROM certificate_jobs WHERE id=$1 FOR UPDATE",
        &[&job],
    )
    .await
    .unwrap();
    tx.execute(
        "INSERT INTO acme_challenges(token,key_authorization) VALUES('delayed','authorization')",
        &[],
    )
    .await
    .unwrap();
    tx.execute(
        "UPDATE certificate_jobs SET http_tokens='[\"delayed\"]' WHERE id=$1",
        &[&job],
    )
    .await
    .unwrap();
    let store = jobs::DatabaseChallengeStore::new(t.state.clone(), job);
    let cleanup = tokio::spawn(async move { store.remove("delayed").await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !cleanup.is_finished(),
        "cleanup cannot overtake a backend INSERT"
    );
    tx.commit().await.unwrap();
    cleanup.await.unwrap().unwrap();
    assert!(
        t.client
            .query_opt(
                "SELECT token FROM acme_challenges WHERE token='delayed'",
                &[]
            )
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn restart_discards_stale_dns_orders_without_erasing_live_certificate() {
    let t = auth_support::test_db().await;
    t.client.execute("INSERT INTO certificates(id,domains,status,challenge_type,dns_order,last_attempt_at,cert_pem,key_pem) VALUES('old-initial','[]','awaiting-dns','dns-01','{}',now()-interval '8 days',NULL,NULL),('old-renew','[]','issued','dns-01','{}',now()-interval '8 days','keep certificate','keep key')",&[]).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    worker.shutdown().await.unwrap();
    let initial = t
        .client
        .query_one(
            "SELECT status::text,dns_order FROM certificates WHERE id='old-initial'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(initial.get::<_, String>(0), "failed");
    assert!(initial.get::<_, Option<serde_json::Value>>(1).is_none());
    let renewed = t
        .client
        .query_one(
            "SELECT status::text,dns_order,cert_pem,key_pem FROM certificates WHERE id='old-renew'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(renewed.get::<_, String>(0), "issued");
    assert!(renewed.get::<_, Option<serde_json::Value>>(1).is_none());
    assert_eq!(renewed.get::<_, String>(2), "keep certificate");
    assert_eq!(renewed.get::<_, String>(3), "keep key");
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn worker_reports_lost_database_ownership_to_api_supervisor() {
    let t = auth_support::test_db().await;
    let worker = jobs::start(t.state.clone()).await.unwrap();
    let mut failed = worker.failure_receiver();
    let pids=t.client.query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=current_database() AND application_name='nwui-certificate-owner'",&[]).await.unwrap();
    assert_eq!(pids.len(), 1);
    tokio::time::timeout(Duration::from_secs(4), failed.changed())
        .await
        .unwrap()
        .unwrap();
    assert!(*failed.borrow());
    assert!(worker.shutdown().await.is_err());
}

async fn job_state(t: &auth_support::TestDb, id: &str) -> (String, Option<String>) {
    let row = t
        .client
        .query_one("SELECT state,error FROM certificate_jobs WHERE certificate_id=$1", &[&id])
        .await
        .unwrap();
    (row.get(0), row.get(1))
}
/// A TXT create that fails without an HTTP status leaves an intent whose record
/// may or may not exist. Once it has stayed invisible past the limit, cleanup
/// must treat it as never created: the job ends failed, the certificate shows
/// the create error again, and retry/renewal are no longer blocked.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn unconfirmed_cloudflare_create_ends_failed_after_limit_with_original_error() {
    use axum::{
        Json, Router,
        http::{Method, Uri},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let (base, ca) = mock_ca().await;
    let directory = format!("{base}/directory");
    let lookups = Arc::new(AtomicUsize::new(0));
    let release_lookup = Arc::new(tokio::sync::Notify::new());
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cf_base = format!("http://{}", socket.local_addr().unwrap());
    let app = Router::new().fallback({
        let lookups = lookups.clone();
        let release_lookup = release_lookup.clone();
        move |method: Method, uri: Uri| {
            let lookups = lookups.clone();
            let release_lookup = release_lookup.clone();
            async move {
                if uri.path() == "/zones" {
                    return Json(json!({"success":true,"result":[{"id":"z","name":"example.com"}]}));
                }
                if method == Method::POST {
                    // Outlives the client timeout and creates nothing.
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    return Json(json!({"success":false,"errors":[{"message":"late"}]}));
                }
                // Let the test move its recorded clock while this retry still owns an old snapshot.
                if lookups.fetch_add(1, Ordering::SeqCst) == 1 {
                    release_lookup.notified().await;
                }
                Json(json!({"success":true,"result":[],"result_info":{"total_pages":1}}))
            }
        }
    });
    let cf = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
    let t = auth_support::test_db_with(&[
        ("ACME_DIRECTORY_URL", &directory),
        ("CF_API_BASE", &cf_base),
        ("API_ACME_TIMEOUT_SECS", "1"),
    ])
    .await;
    let input=service::parse_input(&json!({"domains":["example.com"],"email":"admin@example.com","challengeType":"dns-01","dnsProvider":"cloudflare","dnsToken":"test-token"})).unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    // The create times out, the attempt fails, and the first cleanup finds nothing
    // (it has finished once the certificate shows the cleanup message).
    let mut original = None;
    for _ in 0..200 {
        let (state, error) = job_state(&t, &id).await;
        let shown: Option<String> = t
            .client
            .query_one("SELECT last_error FROM certificates WHERE id=$1", &[&id])
            .await
            .unwrap()
            .get(0);
        if state == "cleanup"
            && lookups.load(Ordering::SeqCst) >= 1
            && shown.is_some_and(|m| m.starts_with("발급 후 정리"))
        {
            original = error;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let original = original.expect("attempt failed with the create error and cleanup looked once");
    // A later lookup within the limit still keeps the intent.
    t.client
        .execute("UPDATE certificate_jobs SET updated_at=now()-interval '1 minute' WHERE certificate_id=$1", &[&id])
        .await
        .unwrap();
    for _ in 0..300 {
        if lookups.load(Ordering::SeqCst) >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(lookups.load(Ordering::SeqCst), 2);
    assert_eq!(job_state(&t, &id).await.0, "cleanup");
    // Move the time the create was first found missing back past the limit. Only a
    // recorded time moves; an intent that never recorded one stays as it is.
    t.client.execute("UPDATE certificate_jobs SET updated_at=now()-interval '1 minute',cf_records=(SELECT jsonb_agg(CASE WHEN item->>'unconfirmed_since' IS NULL THEN item ELSE jsonb_set(item,'{unconfirmed_since}',to_jsonb((item->>'unconfirmed_since')::timestamptz-interval '11 minutes')) END) FROM jsonb_array_elements(cf_records) item) WHERE certificate_id=$1",&[&id]).await.unwrap();
    release_lookup.notify_one();
    // An in-flight retry can re-date updated_at after our clock move: allow its
    // 30 s cleanup interval, 10 s poll, 5 s owner check and maximum DB backoff.
    let settled = tokio::time::timeout(jobs::MAX_RETRY_DELAY + Duration::from_secs(45), async {
        loop {
            let state = job_state(&t, &id).await.0;
            if state != "cleanup" {
                break state;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("cleanup must settle after the next scheduled retry and bounded DB backoff");
    worker.shutdown().await.unwrap();
    assert_eq!(settled, "failed");
    let job = t
        .client
        .query_one("SELECT cf_records,payload_enc FROM certificate_jobs WHERE certificate_id=$1", &[&id])
        .await
        .unwrap();
    assert_eq!(job.get::<_, serde_json::Value>(0), json!([]));
    assert_eq!(job.get::<_, String>(1), "");
    let cert = t
        .client
        .query_one("SELECT status::text,last_error,issue_log FROM certificates WHERE id=$1", &[&id])
        .await
        .unwrap();
    assert_eq!(cert.get::<_, String>(0), "failed");
    assert_eq!(cert.get::<_, Option<String>>(1).as_deref(), Some(original.as_str()));
    let log: serde_json::Value = cert.get(2);
    assert!(
        log.as_array().unwrap().iter().any(|e| e["msg"].as_str().is_some_and(|m| m.contains("Cloudflare") && m.contains("직접"))),
        "issue log tells the operator the unconfirmed record may need manual removal: {log}"
    );
    // The failure was announced once, with the create error.
    let sent = t
        .client
        .query("SELECT type,data->>'error' FROM notifications WHERE link=$1", &[&format!("/certificates/{id}")])
        .await
        .unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].get::<_, String>(0), "cert.failed");
    assert_eq!(sent[0].get::<_, Option<String>>(1).as_deref(), Some(original.as_str()));
    service::retry(&t.state, &id, &actor()).await.unwrap();
    cf.abort();
    ca.abort();
}
/// Ending cleanup restores only a cleanup progress message. A cancelled renewal
/// of a live certificate leaves its error field as the renewal found it.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn cancelled_renewal_leaves_live_certificate_error_untouched() {
    let t = auth_support::test_db().await;
    let input =
        service::parse_input(&json!({"domains":["example.com"],"email":"admin@example.com"}))
            .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    t.client
        .execute("UPDATE certificate_jobs SET mode='renew' WHERE certificate_id=$1", &[&id])
        .await
        .unwrap();
    t.client.execute("UPDATE certificates SET status='issued',cert_pem='live certificate',key_pem='live key',expires_at=now()+interval '20 days',last_error=NULL WHERE id=$1",&[&id]).await.unwrap();
    service::cancel(&t.state, &id, &actor()).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    assert_eq!(wait_terminal(&t, &id).await, "cancelled");
    worker.shutdown().await.unwrap();
    let row = t
        .client
        .query_one("SELECT status::text,last_error FROM certificates WHERE id=$1", &[&id])
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "issued");
    assert_eq!(row.get::<_, Option<String>>(1), None);
}

/// Starts a manual DNS-01 order whose CA holds `/new-order` open for `hold`.
async fn issuing_with_slow_ca(
    hold: Duration,
) -> (auth_support::TestDb, String, jobs::JobWorker, tokio::task::JoinHandle<()>) {
    let (base, ca, reached) = mock_ca_with(Some(("/new-order", hold))).await;
    let directory = format!("{base}/directory");
    let t = auth_support::test_db_with(&[("ACME_DIRECTORY_URL", &directory)]).await;
    let input = service::parse_input(
        &json!({"domains":["example.com"],"email":"admin@example.com","challengeType":"dns-01"}),
    )
    .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), reached.notified())
        .await
        .expect("the job reached the CA");
    (t, id, worker, ca)
}
async fn certificate_outcome(t: &auth_support::TestDb, id: &str, until: &[&str]) -> (String, Option<String>) {
    for _ in 0..750 {
        let row = t
            .client
            .query_one("SELECT status::text,last_error FROM certificates WHERE id=$1", &[&id])
            .await
            .unwrap();
        let status: String = row.get(0);
        if until.contains(&status.as_str()) {
            return (status, row.get(1));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("certificate did not reach {until:?}")
}

/// A pool that stays exhausted past its wait limit is a slow database, not a
/// cancel request: the running issuance continues once connections free up.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn transient_database_errors_do_not_cancel_a_running_issuance() {
    let (t, id, worker, ca) = issuing_with_slow_ca(Duration::from_secs(7)).await;
    let size = t.state.db.status().max_size;
    let mut held = Vec::new();
    for _ in 0..size {
        held.push(t.state.db.get().await.unwrap());
    }
    // Longer than the pool's 5 second wait limit.
    tokio::time::sleep(Duration::from_secs(6)).await;
    drop(held);
    let (status, error) = certificate_outcome(&t, &id, &["awaiting-dns", "failed"]).await;
    worker.shutdown().await.unwrap();
    assert_eq!((status.as_str(), error), ("awaiting-dns", None));
    ca.abort();
}

/// Losing the ownership connection stops the work, and says so instead of
/// reading like a user cancel.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn lost_ownership_stops_issuance_with_its_own_reason() {
    let (t, id, worker, ca) = issuing_with_slow_ca(Duration::from_secs(30)).await;
    t.client.execute("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=current_database() AND application_name='nwui-certificate-owner'",&[]).await.unwrap();
    let (_, error) = certificate_outcome(&t, &id, &["failed"]).await;
    let _ = worker.shutdown().await;
    assert_eq!(
        error.as_deref(),
        Some("데이터베이스 연결이 끊겨 발급이 중단되었습니다. 다시 시도하세요.")
    );
    ca.abort();
}

/// Stopping the API during an issuance reads as a restart, as recovery says it.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn shutdown_during_issuance_says_the_api_restarted() {
    let (t, id, worker, ca) = issuing_with_slow_ca(Duration::from_secs(30)).await;
    worker.shutdown().await.unwrap();
    let (_, error) = certificate_outcome(&t, &id, &["failed"]).await;
    assert_eq!(error.as_deref(), Some("API 재시작으로 발급이 중단되었습니다."));
    ca.abort();
}

/// Finished jobs are kept for 90 days, like the notification feed; jobs still
/// in progress are never pruned.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn finished_jobs_are_pruned_after_the_retention_period() {
    let t = auth_support::test_db().await;
    t.client.execute("INSERT INTO certificate_jobs(id,certificate_id,kind,mode,state,payload_enc,actor,updated_at) VALUES
      ('old-succeeded','c1','http','initial','succeeded','','{}',now()-interval '91 days'),
      ('old-failed','c2','http','initial','failed','','{}',now()-interval '91 days'),
      ('old-cancelled','c3','http','initial','cancelled','','{}',now()-interval '91 days'),
      ('recent-failed','c4','http','initial','failed','','{}',now()-interval '89 days'),
      ('old-cleanup','c5','http','initial','cleanup','','{}',now()-interval '91 days')",&[]).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    let mut remaining = Vec::new();
    for _ in 0..100 {
        remaining = t
            .client
            .query("SELECT id FROM certificate_jobs ORDER BY id", &[])
            .await
            .unwrap()
            .iter()
            .map(|r| r.get::<_, String>(0))
            .collect::<Vec<_>>();
        if remaining.len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    worker.shutdown().await.unwrap();
    assert_eq!(remaining, ["old-cleanup", "recent-failed"]);
}

/// A cancel the user already recorded stays a cancel when the API happens to
/// stop before the worker notices it.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn recorded_cancel_wins_over_a_concurrent_shutdown() {
    let (t, id, worker, ca) = issuing_with_slow_ca(Duration::from_secs(30)).await;
    t.client
        .execute("UPDATE certificate_jobs SET cancel_requested=true WHERE certificate_id=$1", &[&id])
        .await
        .unwrap();
    worker.shutdown().await.unwrap();
    let (_, error) = certificate_outcome(&t, &id, &["failed"]).await;
    assert_eq!(error.as_deref(), Some("요청이 중단되었습니다."));
    ca.abort();
}

// ---- Cleanup that cannot finish, and renewals that cannot be queued ----

const TOKEN: &str = "private-test-token";
/// Cloudflare stand-in for cleanup. `mode`: 0 deletes, 1 answers 503, 2 refuses with 403.
async fn cleanup_cloudflare(
    mode: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    deletes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
) -> (String, tokio::task::JoinHandle<()>) {
    use axum::{Json, Router, http::{Method, StatusCode}};
    use std::sync::atomic::Ordering;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(move |method: Method| {
        let (mode, deletes) = (mode.clone(), deletes.clone());
        async move {
            if method != Method::DELETE {
                return (StatusCode::OK, Json(json!({"success":true,"result":[],"result_info":{"total_pages":1}})));
            }
            deletes.fetch_add(1, Ordering::SeqCst);
            match mode.load(Ordering::SeqCst) {
                0 => (StatusCode::OK, Json(json!({"success":true,"result":{"id":"rec-1"}}))),
                1 => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"success":false,"errors":[{"code":10500,"message":"temporarily unavailable"}]}))),
                _ => (StatusCode::FORBIDDEN, Json(json!({"success":false,"errors":[{"code":10000,"message":"Authentication error"}]}))),
            }
        }
    });
    (base, tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }))
}
/// A certificate whose job reached cleanup with one Cloudflare TXT record still to remove.
/// `payload` None seals the real snapshot with the token; Some replaces it verbatim.
async fn in_cleanup(
    t: &auth_support::TestDb,
    status: &str,
    error: Option<&str>,
    intent: serde_json::Value,
    payload: Option<&str>,
) -> String {
    let input =
        service::parse_input(&json!({"domains":["example.com"],"email":"admin@example.com"}))
            .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    let enc: String = t
        .client
        .query_one("SELECT payload_enc FROM certificate_jobs WHERE certificate_id=$1", &[&id])
        .await
        .unwrap()
        .get(0);
    let sealed = match payload {
        Some(raw) => raw.to_owned(),
        None => {
            let mut open: serde_json::Value =
                serde_json::from_str(&t.state.secrets.open(Some(&enc)).unwrap()).unwrap();
            open["dns_token"] = json!(TOKEN);
            t.state.secrets.seal(&open.to_string()).unwrap()
        }
    };
    t.client.execute("UPDATE certificate_jobs SET state='cleanup',updated_at=now()-interval '1 minute',payload_enc=$2,cf_records=$3,error=$4 WHERE certificate_id=$1",&[&id,&sealed,&json!([intent]),&error]).await.unwrap();
    t.client.execute("UPDATE certificates SET status=$2::text::cert_status,last_error=$3 WHERE id=$1",&[&id,&status,&error]).await.unwrap();
    id
}
fn txt_intent() -> serde_json::Value {
    json!({"zone_id":"z","name":"_acme-challenge.example.com","value":"txt-value","owner":"nwui:job:1","record_id":"rec-1"})
}
/// One worker run: start, wait until it has worked on this job once, stop. The next run is a
/// restart, so everything a later attempt knows came from the database.
async fn cleanup_run(t: &auth_support::TestDb, id: &str) {
    let before: chrono::DateTime<chrono::Utc> = t.client.query_one("SELECT now()", &[]).await.unwrap().get(0);
    let worker = jobs::start(t.state.clone()).await.unwrap();
    for _ in 0..200 {
        let row = t
            .client
            .query_one("SELECT updated_at FROM certificate_jobs WHERE certificate_id=$1 ORDER BY created_at LIMIT 1", &[&id])
            .await
            .unwrap();
        if row.get::<_, chrono::DateTime<chrono::Utc>>(0) >= before {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    worker.shutdown().await.unwrap();
    t.client
        .execute("UPDATE certificate_jobs SET updated_at=now()-interval '1 minute' WHERE certificate_id=$1 AND state='cleanup'", &[&id])
        .await
        .unwrap();
}
async fn job_row(t: &auth_support::TestDb, id: &str) -> (String, String, serde_json::Value) {
    let row = t
        .client
        .query_one("SELECT state,payload_enc,cf_records FROM certificate_jobs WHERE certificate_id=$1 ORDER BY created_at LIMIT 1", &[&id])
        .await
        .unwrap();
    (row.get(0), row.get(1), row.get(2))
}
async fn backdate_failing_since(t: &auth_support::TestDb, id: &str, interval: &str) {
    t.client.execute(&format!("UPDATE certificate_jobs SET cf_records=(SELECT jsonb_agg(jsonb_set(item,'{{failing_since}}',to_jsonb(now()-interval '{interval}'))) FROM jsonb_array_elements(cf_records) item) WHERE certificate_id=$1"),&[&id]).await.unwrap();
}
async fn cleanup_notices(t: &auth_support::TestDb, id: &str) -> Vec<tokio_postgres::Row> {
    t.client
        .query("SELECT body,data FROM notifications WHERE type='cert.cleanup_failed' AND data->>'certificateId'=$1", &[&id])
        .await
        .unwrap()
}
/// Nothing a person reads about a given-up cleanup may carry the token or sealed data.
async fn assert_no_secrets(t: &auth_support::TestDb) {
    let text: String = t.client.query_one("SELECT concat_ws('|',(SELECT string_agg(concat_ws('|',title,body,data::text),'|') FROM notifications),(SELECT string_agg(\"after\"::text,'|') FROM audit_logs),(SELECT string_agg(concat_ws('|',last_error,issue_log::text),'|') FROM certificates),(SELECT string_agg(cf_records::text,'|') FROM certificate_jobs))",&[]).await.unwrap().get(0);
    assert!(!text.contains(TOKEN), "token disclosed: {text}");
    assert!(!text.contains("enc:v1:"), "sealed data disclosed: {text}");
}

/// An outage is retried without giving up, and the job ends normally once Cloudflare answers.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn unavailable_cloudflare_cleanup_retries_and_finishes_when_it_answers() {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    let (mode, deletes) = (Arc::new(AtomicUsize::new(1)), Arc::new(AtomicUsize::new(0)));
    let (cf, server) = cleanup_cloudflare(mode.clone(), deletes.clone()).await;
    let t = auth_support::test_db_with(&[("CF_API_BASE", &cf)]).await;
    let id = in_cleanup(&t, "issued", None, txt_intent(), None).await;
    cleanup_run(&t, &id).await;
    let (state, _, records) = job_row(&t, &id).await;
    assert_eq!(state, "cleanup");
    assert!(records[0]["failing_since"].is_string(), "the failure streak is recorded: {records}");
    assert!(records[0]["abandoned_at"].is_null());
    let shown: Option<String> = t.client.query_one("SELECT last_error FROM certificates WHERE id=$1", &[&id]).await.unwrap().get(0);
    assert!(shown.is_some_and(|m| m.starts_with("발급 후 정리가 완료되지 않았습니다")));
    mode.store(0, Ordering::SeqCst);
    cleanup_run(&t, &id).await;
    let (state, payload, records) = job_row(&t, &id).await;
    assert_eq!((state.as_str(), payload.as_str(), records), ("succeeded", "", json!([])));
    assert_eq!(deletes.load(Ordering::SeqCst), 2);
    let shown: Option<String> = t.client.query_one("SELECT last_error FROM certificates WHERE id=$1", &[&id]).await.unwrap().get(0);
    assert_eq!(shown, None);
    assert!(cleanup_notices(&t, &id).await.is_empty());
    server.abort();
}

/// A refusal (revoked token) never heals. After its limit the record is given up: the job ends,
/// the certificate is free again, and the record stays where an operator can find it.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn refused_cloudflare_cleanup_is_given_up_after_its_limit_and_keeps_the_record() {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    let (mode, deletes) = (Arc::new(AtomicUsize::new(2)), Arc::new(AtomicUsize::new(0)));
    let (cf, server) = cleanup_cloudflare(mode, deletes.clone()).await;
    let t = auth_support::test_db_with(&[("CF_API_BASE", &cf)]).await;
    let id = in_cleanup(&t, "failed", Some("원래 발급 오류"), txt_intent(), None).await;
    cleanup_run(&t, &id).await;
    let (_, _, records) = job_row(&t, &id).await;
    let first = records[0]["failing_since"].clone();
    assert!(first.is_string(), "{records}");
    // After a restart, within the limit: still retried, and the streak is not restarted.
    cleanup_run(&t, &id).await;
    let (state, _, records) = job_row(&t, &id).await;
    assert_eq!(state, "cleanup");
    assert_eq!(records[0]["failing_since"], first);
    assert_eq!(deletes.load(Ordering::SeqCst), 2);
    backdate_failing_since(&t, &id, "11 minutes").await;
    cleanup_run(&t, &id).await;
    let (state, payload, records) = job_row(&t, &id).await;
    assert_eq!((state.as_str(), payload.as_str()), ("failed", ""));
    assert_eq!(records.as_array().unwrap().len(), 1, "the given-up record is kept: {records}");
    assert_eq!(records[0]["record_id"], "rec-1");
    assert!(records[0]["abandoned_at"].is_string());
    assert!(records[0]["last_error"].as_str().is_some_and(|e| e.contains("Authentication error")), "{records}");
    let cert = t.client.query_one("SELECT status::text,last_error,issue_log::text FROM certificates WHERE id=$1", &[&id]).await.unwrap();
    assert_eq!(cert.get::<_, String>(0), "failed");
    assert_eq!(cert.get::<_, Option<String>>(1).as_deref(), Some("원래 발급 오류"), "the job's own error stays in front");
    let log: String = cert.get(2);
    assert!(log.contains("직접 지우세요") && log.contains("_acme-challenge.example.com") && log.contains("rec-1"), "{log}");
    assert!(!log.contains("삭제했습니다") && !log.contains("정리를 마쳤습니다"), "never claims the record is gone: {log}");
    let audit = auth_support::audit_rows(&t.client, "cert:cleanup-abandoned").await;
    assert_eq!(audit.len(), 1);
    let notices = cleanup_notices(&t, &id).await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].get::<_, serde_json::Value>(1)["records"][0]["recordId"], "rec-1");
    assert_no_secrets(&t).await;
    // The job slot is free: a retry is accepted.
    service::retry(&t.state, &id, &actor()).await.unwrap();
    server.abort();
}

/// An outage has a longer limit, but also one. A job that issued fine shows what to remove.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn unavailable_cloudflare_cleanup_is_given_up_after_a_day() {
    use std::sync::{Arc, atomic::AtomicUsize};
    let (cf, server) = cleanup_cloudflare(Arc::new(AtomicUsize::new(1)), Arc::new(AtomicUsize::new(0))).await;
    let t = auth_support::test_db_with(&[("CF_API_BASE", &cf)]).await;
    let mut intent = txt_intent();
    // Recorded by an earlier API process.
    intent["failing_since"] = json!(chrono::Utc::now() - chrono::Duration::hours(23));
    let id = in_cleanup(&t, "issued", None, intent, None).await;
    cleanup_run(&t, &id).await;
    assert_eq!(job_row(&t, &id).await.0, "cleanup", "a 503 is not given up after 23 hours");
    backdate_failing_since(&t, &id, "25 hours").await;
    cleanup_run(&t, &id).await;
    let (state, _, records) = job_row(&t, &id).await;
    assert_eq!(state, "succeeded");
    assert!(records[0]["abandoned_at"].is_string());
    let cert = t.client.query_one("SELECT status::text,last_error FROM certificates WHERE id=$1", &[&id]).await.unwrap();
    assert_eq!(cert.get::<_, String>(0), "issued");
    assert!(cert.get::<_, Option<String>>(1).is_some_and(|m| m.starts_with("Cloudflare TXT 레코드를 지우지 못해") && m.contains("rec-1")));
    assert_eq!(cleanup_notices(&t, &id).await.len(), 1);
    server.abort();
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn cancelled_job_whose_cleanup_is_refused_still_ends_cancelled() {
    use std::sync::{Arc, atomic::AtomicUsize};
    let (cf, server) = cleanup_cloudflare(Arc::new(AtomicUsize::new(2)), Arc::new(AtomicUsize::new(0))).await;
    let t = auth_support::test_db_with(&[("CF_API_BASE", &cf)]).await;
    let mut intent = txt_intent();
    intent["failing_since"] = json!(chrono::Utc::now() - chrono::Duration::minutes(11));
    let id = in_cleanup(&t, "failed", Some("요청이 중단되었습니다."), intent, None).await;
    t.client.execute("UPDATE certificate_jobs SET cancel_requested=true WHERE certificate_id=$1", &[&id]).await.unwrap();
    cleanup_run(&t, &id).await;
    let (state, _, records) = job_row(&t, &id).await;
    assert_eq!(state, "cancelled");
    assert_eq!(records.as_array().unwrap().len(), 1);
    assert_eq!(cleanup_notices(&t, &id).await.len(), 1, "a cancel does not hide records left behind");
    server.abort();
}

/// A snapshot that cannot be unsealed has no token. HTTP-01 tokens still go, the TXT record is
/// given up like any refusal, and no Cloudflare call is made without credentials.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn unopenable_snapshot_still_ends_cleanup_and_keeps_the_record() {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    let deletes = Arc::new(AtomicUsize::new(0));
    let (cf, server) = cleanup_cloudflare(Arc::new(AtomicUsize::new(0)), deletes.clone()).await;
    let t = auth_support::test_db_with(&[("CF_API_BASE", &cf)]).await;
    let id = in_cleanup(&t, "failed", None, txt_intent(), Some("enc:v1:not-a-sealed-payload")).await;
    t.client.execute("UPDATE certificate_jobs SET http_tokens='[\"tok-1\"]' WHERE certificate_id=$1", &[&id]).await.unwrap();
    t.client.execute("INSERT INTO acme_challenges(token,key_authorization) VALUES('tok-1','v')", &[]).await.unwrap();
    cleanup_run(&t, &id).await;
    let row = t.client.query_one("SELECT state,http_tokens,cf_records FROM certificate_jobs WHERE certificate_id=$1", &[&id]).await.unwrap();
    assert_eq!(row.get::<_, String>(0), "cleanup");
    assert_eq!(row.get::<_, serde_json::Value>(1), json!([]));
    assert!(row.get::<_, serde_json::Value>(2)[0]["failing_since"].is_string());
    let left: i64 = t.client.query_one("SELECT count(*) FROM acme_challenges", &[]).await.unwrap().get(0);
    assert_eq!(left, 0);
    backdate_failing_since(&t, &id, "11 minutes").await;
    cleanup_run(&t, &id).await;
    let (state, payload, records) = job_row(&t, &id).await;
    assert_eq!((state.as_str(), payload.as_str()), ("failed", ""));
    assert!(records[0]["abandoned_at"].is_string());
    assert_eq!(deletes.load(Ordering::SeqCst), 0);
    assert_eq!(cleanup_notices(&t, &id).await.len(), 1);
    assert_no_secrets(&t).await;
    service::retry(&t.state, &id, &actor()).await.unwrap();
    server.abort();
}

/// A create Cloudflare refused made no record. Without a token there is still nothing to
/// delete, so cleanup ends at once and announces no leftover record.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn refused_create_needs_no_token_to_finish_cleanup() {
    let t = auth_support::test_db().await;
    let intent = json!({"zone_id":"z","name":"_acme-challenge.example.com","value":"txt-value","owner":"nwui:job:1","rejected":true});
    let id = in_cleanup(&t, "failed", Some("원래 발급 오류"), intent, Some("enc:v1:not-a-sealed-payload")).await;
    cleanup_run(&t, &id).await;
    let (state, _, records) = job_row(&t, &id).await;
    assert_eq!((state.as_str(), records), ("failed", json!([])));
    assert!(cleanup_notices(&t, &id).await.is_empty());
    assert!(auth_support::audit_rows(&t.client, "cert:cleanup-abandoned").await.is_empty());
}

/// A queued job whose snapshot cannot be unsealed never runs: it fails its certificate instead
/// of holding the job slot forever.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn queued_job_with_unopenable_snapshot_fails_its_certificate() {
    let t = auth_support::test_db().await;
    let input =
        service::parse_input(&json!({"domains":["example.com"],"email":"admin@example.com"}))
            .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    t.client.execute("UPDATE certificate_jobs SET payload_enc='enc:v1:not-a-sealed-payload' WHERE certificate_id=$1", &[&id]).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    let settled = wait_terminal(&t, &id).await;
    worker.shutdown().await.unwrap();
    assert_eq!(settled, "failed");
    let cert = t.client.query_one("SELECT status::text,last_error FROM certificates WHERE id=$1", &[&id]).await.unwrap();
    assert_eq!(cert.get::<_, String>(0), "failed");
    assert!(cert.get::<_, Option<String>>(1).is_some_and(|m| m.contains("봉인 데이터")));
    service::retry(&t.state, &id, &actor()).await.unwrap();
}

/// Due certificates (soonest expiry first), all issued, none with a job in progress. The
/// directory is a closed local port so the renewals the sweep queues fail at once.
async fn due_certificates(t: &auth_support::TestDb, count: usize) -> Vec<String> {
    let mut ids = Vec::new();
    for i in 0..count {
        let input = service::parse_input(&json!({"domains":[format!("renew{i}.example.com")],"email":"admin@example.com"})).unwrap();
        let id = service::create(&t.state, &input, &actor()).await.unwrap();
        t.client.execute("UPDATE certificate_jobs SET state='succeeded',payload_enc='' WHERE certificate_id=$1", &[&id]).await.unwrap();
        t.client.execute(&format!("UPDATE certificates SET status='issued',expires_at=now()+interval '{} days',last_attempt_at=NULL WHERE id=$1", 10 + i), &[&id]).await.unwrap();
        ids.push(id);
    }
    ids
}
async fn renew_jobs(t: &auth_support::TestDb, id: &str) -> i64 {
    t.client.query_one("SELECT count(*) FROM certificate_jobs WHERE certificate_id=$1 AND mode='renew'", &[&id]).await.unwrap().get(0)
}
/// One sweep: a worker start runs it first thing. Waits for the other certificates' jobs.
async fn sweep(t: &auth_support::TestDb, queued: &[&str]) {
    let worker = jobs::start(t.state.clone()).await.unwrap();
    for _ in 0..200 {
        let mut all = true;
        for id in queued {
            all &= renew_jobs(t, id).await > 0;
        }
        if all {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    worker.shutdown().await.unwrap();
}

/// The first certificate cannot be queued. It is reported once, with its own context, and the
/// sweep still queues the others. The same failure 12 hours later is not announced again.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn renewal_sweep_reports_a_certificate_it_cannot_queue_and_goes_on() {
    let t = auth_support::test_db_with(&[("ACME_DIRECTORY_URL", "http://127.0.0.1:9/directory")]).await;
    let ids = due_certificates(&t, 3).await;
    let failing = &ids[0];
    t.client.batch_execute(&format!("CREATE FUNCTION refuse_renewal() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.certificate_id='{failing}' THEN RAISE EXCEPTION 'injected enqueue failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER refuse_renewal BEFORE INSERT ON certificate_jobs FOR EACH ROW EXECUTE FUNCTION refuse_renewal();")).await.unwrap();
    sweep(&t, &[&ids[1], &ids[2]]).await;
    assert_eq!(renew_jobs(&t, failing).await, 0);
    assert_eq!((renew_jobs(&t, &ids[1]).await, renew_jobs(&t, &ids[2]).await), (1, 1), "later certificates are still queued");
    let cert = t.client.query_one("SELECT last_error,last_attempt_at IS NOT NULL,jsonb_array_length(issue_log) FROM certificates WHERE id=$1", &[failing]).await.unwrap();
    assert!(cert.get::<_, Option<String>>(0).is_some());
    assert!(cert.get::<_, bool>(1));
    assert_eq!(cert.get::<_, i32>(2), 1);
    let notices = || async {
        t.client.query("SELECT type,severity::text,data FROM notifications WHERE link=$1", &[&format!("/certificates/{failing}")]).await.unwrap()
    };
    let sent = notices().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].get::<_, String>(0), "cert.renew_failed");
    let data: serde_json::Value = sent[0].get(2);
    assert_eq!(data["certificateId"], json!(failing));
    assert_eq!(data["domains"], json!(["renew0.example.com"]));
    assert!(data["error"].is_string());
    // A restart right away does not retry it: the attempt time spaces retries by 12 hours.
    sweep(&t, &[]).await;
    assert_eq!(t.client.query_one("SELECT jsonb_array_length(issue_log) FROM certificates WHERE id=$1", &[failing]).await.unwrap().get::<_, i32>(0), 1);
    // 12 hours later it is tried again and logged, but the same failure is not announced twice.
    t.client.execute("UPDATE certificates SET last_attempt_at=now()-interval '13 hours' WHERE id=$1", &[failing]).await.unwrap();
    sweep(&t, &[]).await;
    assert_eq!(t.client.query_one("SELECT jsonb_array_length(issue_log) FROM certificates WHERE id=$1", &[failing]).await.unwrap().get::<_, i32>(0), 2);
    assert_eq!(notices().await.len(), 1);
}

/// A manual renewal queued while the sweep's own attempt for the same certificate was failing
/// is the newer attempt: the sweep's failure is not recorded over it, and no failure is sent.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn a_renewal_queued_while_the_sweep_failed_is_not_overwritten() {
    let t = auth_support::test_db_with(&[("ACME_DIRECTORY_URL", "http://127.0.0.1:9/directory")]).await;
    let ids = due_certificates(&t, 1).await;
    let id = ids[0].clone();
    // The sweep's insert holds the certificate row (queue locks it), waits, then fails. A manual
    // renewal started meanwhile waits for that row lock and commits right after the failure.
    t.client.batch_execute("CREATE FUNCTION slow_refusal() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.actor->>'label'='system:certificate-renewal' THEN PERFORM pg_sleep(2); RAISE EXCEPTION 'injected enqueue failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER slow_refusal BEFORE INSERT ON certificate_jobs FOR EACH ROW EXECUTE FUNCTION slow_refusal();").await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    for _ in 0..300 {
        let sleeping: i64 = t.client.query_one("SELECT count(*) FROM pg_stat_activity WHERE wait_event='PgSleep'", &[]).await.unwrap().get(0);
        if sleeping > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // The manual renewal (it then runs, against the closed directory, and fails on its own).
    let manual = {
        let state = t.state.clone();
        let id = id.clone();
        tokio::spawn(async move { service::renew(&state, &id, &actor()).await })
    };
    let _ = manual.await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    worker.shutdown().await.unwrap();
    let recorded: Vec<String> = t.client.query("SELECT actor->>'label' FROM certificate_jobs WHERE certificate_id=$1 AND mode='renew'", &[&id]).await.unwrap().iter().map(|r| r.get(0)).collect();
    assert_eq!(recorded, ["test"], "only the manual renewal was queued");
    let log: serde_json::Value = t.client.query_one("SELECT issue_log FROM certificates WHERE id=$1", &[&id]).await.unwrap().get(0);
    assert!(!log.to_string().contains("자동 갱신을 시작하지 못했습니다"), "the sweep's stale failure landed on the newer attempt: {log}");
    let queue_failures: i64 = t.client.query_one("SELECT count(*) FROM notifications WHERE dedupe_key LIKE 'cert.renew_failed:queue:%'", &[]).await.unwrap().get(0);
    assert_eq!(queue_failures, 0);
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn idle_pool_timeout_does_not_kill_the_worker() {
    let t = auth_support::test_db_with(&[("API_DB_POOL_SIZE", "1")]).await;
    let worker = jobs::start(t.state.clone()).await.unwrap();
    let failed = worker.failure_receiver();
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let idle:bool=t.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND state='idle' AND query LIKE 'SELECT id,certificate_id,kind,mode,state,payload_enc,actor FROM certificate_jobs%')",&[]).await.unwrap().get(0);
            if idle {break;}tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("worker must finish its first idle queue read");
    let held = t.state.db.get().await.unwrap();
    // The next idle poll is at 10 s; hold through its 5 s pool wait limit.
    tokio::time::sleep(Duration::from_secs(16)).await;
    let unexpectedly_failed = *failed.borrow();
    drop(held);
    let outcome = worker.shutdown().await;
    assert!(
        !unexpectedly_failed,
        "transient idle pool timeout must not exit the worker: {outcome:?}"
    );
    outcome.unwrap();
}

#[tokio::test]
#[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
async fn deterministic_finish_error_exits_and_recovery_allows_the_next_job() {
    let (base, ca) = mock_ca().await;
    let directory = format!("{base}/directory");
    let t = auth_support::test_db_with(&[("ACME_DIRECTORY_URL", &directory)]).await;
    let input = service::parse_input(
        &json!({"domains":["example.com"],"email":"admin@example.com","challengeType":"dns-01"}),
    )
    .unwrap();
    let bad = service::create(&t.state, &input, &actor()).await.unwrap();
    let good = service::create(&t.state, &input, &actor()).await.unwrap();
    t.client.batch_execute(&format!("CREATE FUNCTION reject_pending() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.id='{bad}' AND NEW.status='awaiting-dns' THEN RAISE EXCEPTION 'deterministic finish constraint' USING ERRCODE='23514'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_pending BEFORE UPDATE ON certificates FOR EACH ROW EXECUTE FUNCTION reject_pending();")).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    let mut failed = worker.failure_receiver();
    let ended = tokio::time::timeout(Duration::from_secs(4), failed.wait_for(|v| *v)).await;
    if ended.is_err() {
        drop(worker);
        panic!("deterministic SQL errors must not retry forever");
    }
    drop(ended);
    assert!(worker.shutdown().await.is_err());
    let worker = jobs::start(t.state.clone()).await.unwrap();
    assert_eq!(wait_terminal(&t, &good).await, "succeeded");
    worker.shutdown().await.unwrap();
    ca.abort();
}
