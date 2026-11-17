mod auth_support;
mod g13_support;
use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use nwui_api::{
    certificates::{jobs, pem, service, store},
    contracts::Actor,
    state::AppState,
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
struct Ca {
    base: String,
    status: String,
    cert: String,
    fail: bool,
    orders: usize,
    state: AppState,
}
struct CsrPublic(Vec<u8>);
impl rcgen::PublicKeyData for CsrPublic {
    fn der_bytes(&self) -> &[u8] {
        &self.0
    }
    fn algorithm(&self) -> &'static rcgen::SignatureAlgorithm {
        &rcgen::PKCS_RSA_SHA256
    }
}
async fn ca(
    State(shared): State<Arc<tokio::sync::Mutex<Ca>>>,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    let mut s = shared.lock().await;
    let mut headers = HeaderMap::new();
    headers.insert("replay-nonce", "dGVzdC1ub25jZQ".parse().unwrap());
    let payload = if method == Method::POST {
        let jws: Value = serde_json::from_slice(&body).unwrap();
        serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(jws["payload"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let mut status = StatusCode::OK;
    let reply = match uri.path() {
        "/directory" => {
            json!({"newNonce":format!("{}/nonce",s.base),"newAccount":format!("{}/account",s.base),"newOrder":format!("{}/new-order",s.base)})
        }
        "/account" => {
            headers.insert("location", format!("{}/account/1", s.base).parse().unwrap());
            json!({"status":"valid","orders":format!("{}/orders",s.base)})
        }
        "/new-order" => {
            s.orders += 1;
            headers.insert("location", format!("{}/order/1", s.base).parse().unwrap());
            order(&s)
        }
        "/order/1" => order(&s),
        "/auth/1" => {
            json!({"identifier":{"type":"dns","value":"example.com"},"status":"pending","challenges":[{"type":"http-01","url":format!("{}/challenge/http",s.base),"status":"pending","token":"worker-token"},{"type":"dns-01","url":format!("{}/challenge/dns",s.base),"status":"pending","token":"worker-token"}]})
        }
        "/challenge/http" | "/challenge/dns" => {
            if uri.path().ends_with("http") {
                let db = s.state.db.get().await.unwrap();
                assert!(
                    db.query_opt(
                        "SELECT token FROM acme_challenges WHERE token='worker-token'",
                        &[]
                    )
                    .await
                    .unwrap()
                    .is_some(),
                    "challenge must be committed before CA validation"
                );
            }
            if s.fail {
                status = StatusCode::BAD_REQUEST;
                json!({"type":"urn:ietf:params:acme:error:unauthorized","detail":"mock denied","status":400})
            } else {
                s.status = "ready".into();
                json!({"type":"http-01","url":format!("{}{}",s.base,uri.path()),"status":"pending","token":"worker-token"})
            }
        }
        "/finalize" => {
            use x509_parser::prelude::FromDer;
            let der = URL_SAFE_NO_PAD
                .decode(payload["csr"].as_str().unwrap())
                .unwrap();
            let (_, csr) =
                x509_parser::certification_request::X509CertificationRequest::from_der(&der)
                    .unwrap();
            let public = CsrPublic(
                csr.certification_request_info
                    .subject_pki
                    .subject_public_key
                    .data
                    .to_vec(),
            );
            let issuer = rcgen::Issuer::new(
                rcgen::CertificateParams::default(),
                rcgen::KeyPair::generate().unwrap(),
            );
            s.cert = rcgen::CertificateParams::new(vec!["example.com".into()])
                .unwrap()
                .signed_by(&public, &issuer)
                .unwrap()
                .pem();
            s.status = "valid".into();
            order(&s)
        }
        "/cert" => return (status, headers, s.cert.clone()).into_response(),
        _ => json!({"status":"valid"}),
    };
    (status, headers, axum::Json(reply)).into_response()
}
fn order(s: &Ca) -> Value {
    json!({"status":s.status,"identifiers":[{"type":"dns","value":"example.com"}],"authorizations":[format!("{}/auth/1",s.base)],"finalize":format!("{}/finalize",s.base),"certificate":if s.status=="valid"{Some(format!("{}/cert",s.base))}else{None}})
}
fn actor() -> Actor {
    Actor {
        via: None,
        id: None,
        label: Some("test:issuance".into()),
        ip: None,
    }
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn monitor_skips_a_busy_single_connection_pool_and_checks_ownership_later() {
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", socket.local_addr().unwrap());
    let directory = format!("{base}/directory");
    let t = auth_support::test_db_with(&[
        ("API_DB_POOL_SIZE", "1"),
        ("ACME_DIRECTORY_URL", &directory),
        ("API_ACME_TIMEOUT_SECS", "10"),
    ])
    .await;
    let shared = Arc::new(tokio::sync::Mutex::new(Ca {
        base,
        status: "pending".into(),
        cert: String::new(),
        fail: false,
        orders: 0,
        state: t.state.clone(),
    }));
    let app = Router::new().fallback(ca).with_state(shared.clone());
    let server = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
    t.client.batch_execute("CREATE FUNCTION monitor_gate() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(7142046); RETURN NEW; END $$; CREATE TRIGGER monitor_gate BEFORE INSERT ON acme_challenges FOR EACH ROW EXECUTE FUNCTION monitor_gate(); SELECT pg_advisory_lock(7142046)").await.unwrap();
    let input = service::parse_input(&json!({"domains":["example.com"],"email":"ops@example.com"}))
        .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    let end = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        let held:bool=t.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event='advisory' AND query LIKE 'INSERT INTO acme_challenges%')",&[]).await.unwrap().get(0);
        if held {
            break;
        }
        assert!(
            tokio::time::Instant::now() < end,
            "work never reached the held INSERT"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(350)).await;
    t.client
        .simple_query("SELECT pg_advisory_unlock(7142046)")
        .await
        .unwrap();
    let end = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let row = t
            .client
            .query_one("SELECT status::text FROM certificates WHERE id=$1", &[&id])
            .await
            .unwrap();
        if row.get::<_, String>(0) == "issued" {
            break;
        }
        assert!(
            tokio::time::Instant::now() < end,
            "monitor must not starve work after the only connection frees"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(shared.lock().await.orders, 1);
    let certificate = store::get(&t.client, &id, false).await.unwrap();
    let key = t
        .state
        .secrets
        .open(certificate.key_pem.as_deref())
        .unwrap();
    pem::parse_certificate_pem(certificate.cert_pem.as_deref().unwrap(), &key).unwrap();
    let mut failed = worker.failure_receiver();
    let terminated:bool=t.client.query_one("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=current_database() AND application_name='nwui-certificate-owner'",&[]).await.unwrap().get(0);
    assert!(terminated);
    tokio::time::timeout(Duration::from_secs(2), failed.wait_for(|failed| *failed))
        .await
        .expect("ownership checks resume after the skipped busy tick")
        .unwrap();
    assert!(worker.shutdown().await.is_err());
    server.abort();
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn apply_releases_the_single_connection_before_skipped_notification() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("nginx");
    std::fs::write(&bin, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    let conf = temp.path().join("conf");
    let stream = temp.path().join("stream");
    let certs = temp.path().join("certs");
    let mut t = auth_support::test_db_with(&[
        ("API_DB_POOL_SIZE", "1"),
        ("NGINX_BIN", bin.to_str().unwrap()),
        ("NGINX_CONF_D", conf.to_str().unwrap()),
        ("NGINX_STREAM_D", stream.to_str().unwrap()),
        ("CERTS_DIR", certs.to_str().unwrap()),
    ])
    .await;
    g13_support::witness(&mut t.state);
    t.client.batch_execute("INSERT INTO nginx_servers(id,server_name,config) VALUES('invalid','broken.test','{\"proxyPass\":\"not-a-url\"}')").await.unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        nwui_api::publisher_client::apply_saved(&t.state, &actor(), None),
    )
    .await
    .expect("skipped notice must not wait on apply's own pooled connection")
    .unwrap();
    assert!(result.ok);
    assert_eq!(
        t.client
            .query_one(
                "SELECT count(*) FROM notifications WHERE type='config.apply-skipped'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn notification_sweep_releases_the_single_connection_before_pruning() {
    let t = auth_support::test_db_with(&[("API_DB_POOL_SIZE", "1")]).await;
    t.client.batch_execute("INSERT INTO notifications(id,type,severity,title,created_at) VALUES('old','cert.issued','success','old',now()-interval '91 days'),('recent','cert.issued','success','recent',now())").await.unwrap();
    let count = tokio::time::timeout(
        Duration::from_secs(2),
        nwui_api::notifications::queue::sweep_due(&t.state, chrono::Utc::now()),
    )
    .await
    .expect("sweep must return its connection before prune checks out again")
    .unwrap();
    assert_eq!(count, 0);
    let ids = t
        .client
        .query("SELECT id FROM notifications ORDER BY id", &[])
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.get::<_, String>(0))
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["recent"]);
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn rollback_releases_the_single_connection_before_skipped_notification() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("nginx");
    std::fs::write(&bin, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    let conf = temp.path().join("conf");
    let stream = temp.path().join("stream");
    let certs = temp.path().join("certs");
    let mut t = auth_support::test_db_with(&[
        ("API_DB_POOL_SIZE", "1"),
        ("NGINX_BIN", bin.to_str().unwrap()),
        ("NGINX_CONF_D", conf.to_str().unwrap()),
        ("NGINX_STREAM_D", stream.to_str().unwrap()),
        ("CERTS_DIR", certs.to_str().unwrap()),
    ]).await;
    let data = json!({"upstreams":[],"streams":[],"servers":[{"id":"invalid","serverName":"broken.test","listen":8081,"enabled":true,"config":{"proxyPass":"not-a-url"}}]});
    g13_support::witness(&mut t.state);
    t.client.execute("INSERT INTO config_snapshots(id,data) VALUES('77777777-7777-4777-8777-777777777777',$1)", &[&data]).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), nwui_api::publisher_client::rollback_snapshot(&t.state, "77777777-7777-4777-8777-777777777777", &actor())).await.expect("rollback must not wait on its own pooled connection").unwrap();
    assert!(result.result.ok, "{}", result.result.output);
    assert_eq!(t.client.query_one("SELECT count(*) FROM notifications WHERE type='config.apply-skipped' AND severity='warning'", &[]).await.unwrap().get::<_, i64>(0), 1);
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn dashboard_releases_the_single_connection_while_status_is_stalled() {
    use nwui_api::{auth::principal::SessionPrincipal, contracts::Role};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/status", listener.local_addr().unwrap());
    let t = auth_support::test_db_with(&[("API_DB_POOL_SIZE", "1"), ("NGINX_STATUS_URL", &url)]).await;
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        entered.send(()).unwrap();
        let _ = resume.await;
    });
    let state = t.state.clone();
    let dashboard = tokio::spawn(async move {
        let session = SessionPrincipal { user_id:"test".into(), email:None, name:None, role:Role::Admin, session_version:1, totp_enabled:true, requires_2fa:true, mfa_verified:true, expires_at:i64::MAX };
        nwui_api::observability::view(&state, &(session).clone().into(), "/", &Default::default()).await
    });
    tokio::time::timeout(Duration::from_secs(2), waiting).await.unwrap().unwrap();
    let query = async { t.state.db.get().await.unwrap().query_one("SELECT 1", &[]).await.unwrap() };
    tokio::time::timeout(Duration::from_millis(500), query).await.expect("unrelated DB work must finish while status HTTP is stalled");
    assert!(!dashboard.is_finished());
    release.send(()).unwrap();
    assert!(dashboard.await.unwrap().unwrap().is_some());
    server.await.unwrap();
}
