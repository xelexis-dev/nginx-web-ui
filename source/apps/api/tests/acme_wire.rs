use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use nwui_api::certificates::{
    CertificateError,
    protocol::{AcmeClient, Cancellation, DnsBeginResult, HttpChallengeStore},
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

struct Ca {
    base: String,
    token: String,
    status: String,
    auth_valid: bool,
    cert: String,
    payloads: Vec<(String, Value, Value)>,
    stall: bool,
    fail_challenge: bool,
}
async fn ca(
    State(shared): State<Arc<Mutex<Ca>>>,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    if shared.lock().unwrap().stall {
        std::future::pending::<()>().await;
    }
    let mut s = shared.lock().unwrap();
    let mut status = StatusCode::OK;
    let mut headers = HeaderMap::new();
    headers.insert("replay-nonce", "dGVzdC1ub25jZQ".parse().unwrap());
    let (payload, protected) = if method == Method::POST {
        let jws: Value = serde_json::from_slice(&body).unwrap();
        let payload = URL_SAFE_NO_PAD
            .decode(jws["payload"].as_str().unwrap())
            .unwrap();
        let protected: Value = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(jws["protected"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(protected["url"], format!("{}{}", s.base, uri.path()));
        assert_eq!(protected["alg"], "ES256");
        (
            serde_json::from_slice(&payload).unwrap_or(Value::Null),
            protected,
        )
    } else {
        (Value::Null, Value::Null)
    };
    s.payloads
        .push((uri.path().into(), payload.clone(), protected));
    let reply = match uri.path() {
        "/directory" => {
            json!({"newNonce":format!("{}/nonce",s.base),"newAccount":format!("{}/account",s.base),"newOrder":format!("{}/new-order",s.base)})
        }
        "/nonce" => json!({}),
        "/account" => {
            headers.insert("location", format!("{}/account/1", s.base).parse().unwrap());
            json!({"status":"valid","orders":format!("{}/orders",s.base)})
        }
        "/account/1" => json!({"status":"valid","contact":payload["contact"]}),
        "/new-order" => {
            headers.insert("location", format!("{}/order/1", s.base).parse().unwrap());
            order(&s)
        }
        "/order/1" => order(&s),
        "/auth/1" => {
            json!({"identifier":{"type":"dns","value":"example.com"},"status":if s.auth_valid {"valid"}else{"pending"},"wildcard":false,"challenges":[{"type":"dns-01","url":format!("{}/challenge/dns",s.base),"status":"pending","token":s.token},{"type":"http-01","url":format!("{}/challenge/http",s.base),"status":"pending","token":s.token}]})
        }
        "/challenge/dns" | "/challenge/http" => {
            if s.fail_challenge {
                status = StatusCode::BAD_REQUEST;
                json!({"type":"urn:ietf:params:acme:error:unauthorized","detail":"challenge denied","status":400})
            } else {
                s.status = "ready".into();
                json!({"type":"dns-01","url":format!("{}{}",s.base,uri.path()),"status":"pending","token":s.token})
            }
        }
        "/finalize" => {
            s.status = "valid".into();
            order(&s)
        }
        "/cert" => return (status, headers, s.cert.clone()).into_response(),
        _ => {
            status = StatusCode::NOT_FOUND;
            json!({"type":"urn:ietf:params:acme:error:malformed","detail":"unexpected request","status":404})
        }
    };
    (status, headers, axum::Json(reply)).into_response()
}
fn order(s: &Ca) -> Value {
    json!({"status":s.status,"expires":"2030-01-02T03:04:05Z","identifiers":[{"type":"dns","value":"example.com"}],"authorizations":[format!("{}/auth/1",s.base)],"finalize":format!("{}/finalize",s.base),"certificate":if s.status=="valid"{Some(format!("{}/cert",s.base))}else{None}})
}
async fn server(token: &str) -> (AcmeClient, Arc<Mutex<Ca>>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let cert = rcgen::generate_simple_self_signed(vec!["example.com".into()])
        .unwrap()
        .cert
        .pem();
    let state = Arc::new(Mutex::new(Ca {
        base: base.clone(),
        token: token.into(),
        status: "pending".into(),
        auth_valid: false,
        cert,
        payloads: vec![],
        stall: false,
        fail_challenge: false,
    }));
    let app = Router::new().fallback(ca).with_state(state.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        AcmeClient::new(&format!("{base}/directory"), Duration::from_secs(5)).unwrap(),
        state,
        task,
    )
}
fn fixtures() -> Value {
    serde_json::from_slice(&std::fs::read(std::path::Path::new(&std::env::var("NWUI_CONTROL_FIXTURES").expect("set NWUI_CONTROL_FIXTURES")).join("acme/vectors.json")).expect("ACME fixture"))
    .unwrap()
}

#[tokio::test]
#[ignore = "golden vectors: set NWUI_CONTROL_FIXTURES"]
async fn legacy_account_keys_produce_golden_txt_and_serializable_resumable_orders() {
    let vectors = fixtures();
    for account in vectors["accounts"].as_array().unwrap() {
        for case in account["cases"].as_array().unwrap() {
            let (client, state, task) = server(case["token"].as_str().unwrap()).await;
            let result = client
                .begin_dns(
                    account["accountKeyPkcs8Pem"].as_str().unwrap(),
                    "admin@example.com",
                    &["example.com".into()],
                    &Cancellation::default(),
                )
                .await
                .unwrap();
            assert_eq!(result.records[0].record_value, case["dns01TxtValue"]);
            assert_eq!(result.records[0].record_name, "_acme-challenge.example.com");
            let persisted = serde_json::to_value(&result).unwrap();
            assert_eq!(persisted["order"]["expires"], "2030-01-02T03:04:05Z");
            assert_eq!(
                persisted["order"]["url"],
                format!("{}/order/1", state.lock().unwrap().base)
            );
            let restored: DnsBeginResult = serde_json::from_value(persisted).unwrap();
            assert!(restored.key_pem.contains("PRIVATE KEY"));
            let s = state.lock().unwrap();
            let registration = s.payloads.iter().find(|(p, _, _)| p == "/account").unwrap();
            for member in ["crv", "kty", "x", "y"] {
                assert_eq!(registration.2["jwk"][member], account["jwk"][member]);
            }
            assert!(s.payloads.iter().any(|(p, b, _)| p == "/account/1"
                && b["contact"] == json!(["mailto:admin@example.com"])));
            task.abort();
        }
    }
}

#[tokio::test]
#[ignore = "golden vectors: set NWUI_CONTROL_FIXTURES"]
async fn manual_dns_resume_fetches_fresh_order_and_finalizes_the_persisted_csr() {
    let (client, state, task) = server("token").await;
    let vectors = fixtures();
    let key = vectors["accounts"][0]["accountKeyPkcs8Pem"]
        .as_str()
        .unwrap();
    let saved = json!({"url":format!("{}/order/1",state.lock().unwrap().base),"status":"pending","authorizations":[]});
    let csr = vectors["csr"]["cases"][0]["csrPem"].as_str().unwrap();
    let result = client
        .complete_dns(key, &saved, csr, &Cancellation::default())
        .await
        .unwrap();
    assert_eq!(result.cert_pem, state.lock().unwrap().cert);
    let s = state.lock().unwrap();
    assert_eq!(
        s.payloads
            .iter()
            .find(|(p, _, _)| p == "/account")
            .unwrap()
            .1["onlyReturnExisting"],
        true
    );
    assert!(s.payloads.iter().any(|(p, _, _)| p == "/challenge/dns"));
    let finalize = s
        .payloads
        .iter()
        .find(|(p, _, _)| p == "/finalize")
        .unwrap();
    let (_, pem) = x509_parser::pem::parse_x509_pem(csr.as_bytes()).unwrap();
    assert_eq!(
        URL_SAFE_NO_PAD
            .decode(finalize.1["csr"].as_str().unwrap())
            .unwrap(),
        pem.contents
    );
    task.abort();
}

#[tokio::test]
#[ignore = "golden vectors: set NWUI_CONTROL_FIXTURES"]
async fn already_valid_order_is_downloaded_without_resubmitting_challenges_or_csr() {
    let (client, state, task) = server("token").await;
    state.lock().unwrap().status = "valid".into();
    let vectors = fixtures();
    let saved = json!({"url":format!("{}/order/1",state.lock().unwrap().base)});
    client
        .complete_dns(
            vectors["accounts"][0]["accountKeyPkcs8Pem"]
                .as_str()
                .unwrap(),
            &saved,
            vectors["csr"]["cases"][0]["csrPem"].as_str().unwrap(),
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert!(
        state
            .lock()
            .unwrap()
            .payloads
            .iter()
            .all(|(p, _, _)| !p.starts_with("/challenge/") && p != "/finalize")
    );
    task.abort();
}

#[tokio::test]
#[ignore = "golden vectors: set NWUI_CONTROL_FIXTURES"]
async fn cancellation_prevents_network_and_timeout_bounds_an_unresponsive_ca() {
    let (client, state, task) = server("token").await;
    let cancel = Cancellation::default();
    cancel.cancel();
    let vectors = fixtures();
    let key = vectors["accounts"][0]["accountKeyPkcs8Pem"]
        .as_str()
        .unwrap();
    assert!(matches!(
        client
            .begin_dns(key, "admin@example.com", &["example.com".into()], &cancel)
            .await,
        Err(CertificateError::Aborted)
    ));
    assert!(state.lock().unwrap().payloads.is_empty());
    state.lock().unwrap().stall = true;
    let client = AcmeClient::new(
        &format!("{}/directory", state.lock().unwrap().base),
        Duration::from_millis(30),
    )
    .unwrap();
    assert!(matches!(
        client
            .begin_dns(
                key,
                "admin@example.com",
                &["example.com".into()],
                &Cancellation::default()
            )
            .await,
        Err(CertificateError::Timeout)
    ));
    task.abort();
}

#[derive(Default)]
struct Store(Mutex<Vec<(String, String)>>);
impl HttpChallengeStore for Store {
    async fn set(&self, token: &str, value: &str) -> nwui_api::certificates::Result<()> {
        self.0.lock().unwrap().push((token.into(), value.into()));
        Ok(())
    }
    async fn remove(&self, token: &str) -> nwui_api::certificates::Result<()> {
        self.0.lock().unwrap().retain(|(t, _)| t != token);
        Ok(())
    }
}
#[tokio::test]
#[ignore = "golden vectors: set NWUI_CONTROL_FIXTURES"]
async fn failed_http_challenge_is_removed_before_returning_ca_error() {
    let (client, state, task) = server("token").await;
    state.lock().unwrap().fail_challenge = true;
    let store = Store::default();
    let vectors = fixtures();
    let error = client
        .issue_http(
            vectors["accounts"][0]["accountKeyPkcs8Pem"]
                .as_str()
                .unwrap(),
            "admin@example.com",
            &["example.com".into()],
            &store,
            &Cancellation::default(),
        )
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("challenge denied"));
    assert!(store.0.lock().unwrap().is_empty());
    assert!(
        state
            .lock()
            .unwrap()
            .payloads
            .iter()
            .any(|(p, _, _)| p == "/challenge/http")
    );
    task.abort();
}

struct FailingCleanup;
impl HttpChallengeStore for FailingCleanup {
    async fn set(&self, _: &str, _: &str) -> nwui_api::certificates::Result<()> {
        Ok(())
    }
    async fn remove(&self, _: &str) -> nwui_api::certificates::Result<()> {
        Err(CertificateError::Callback("database unavailable".into()))
    }
}
#[tokio::test]
#[ignore = "golden vectors: set NWUI_CONTROL_FIXTURES"]
async fn successful_http_issuance_retains_certificate_and_reports_cleanup_failure() {
    let (client, state, task) = server("token").await;
    let vectors = fixtures();
    let result = client
        .issue_http(
            vectors["accounts"][0]["accountKeyPkcs8Pem"]
                .as_str()
                .unwrap(),
            "admin@example.com",
            &["example.com".into()],
            &FailingCleanup,
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.cert_pem, state.lock().unwrap().cert);
    assert_eq!(result.cleanup_failures.len(), 1);
    assert_eq!(result.cleanup_failures[0].token, "token");
    assert!(
        result.cleanup_failures[0]
            .error
            .contains("database unavailable")
    );
    task.abort();
}

struct CancellingStore {
    cancel: Cancellation,
    values: Arc<Mutex<Vec<String>>>,
    events: Arc<Mutex<Vec<&'static str>>>,
}
impl HttpChallengeStore for CancellingStore {
    async fn set(&self, token: &str, value: &str) -> nwui_api::certificates::Result<()> {
        assert_eq!(value, "token.UTjg4HDK2fhiL6V9Ql-zPB-VvMnwUkH225DRXiU9O_4");
        self.cancel.cancel();
        self.events.lock().unwrap().push("cancel");
        let token = token.to_owned();
        let values = self.values.clone();
        let events = self.events.clone();
        // Model a DB request already sent: dropping the await does not cancel
        // the backend commit, which happens after the cancellation signal.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            values.lock().unwrap().push(token);
            events.lock().unwrap().push("commit");
        })
        .await
        .unwrap();
        Ok(())
    }
    async fn remove(&self, token: &str) -> nwui_api::certificates::Result<()> {
        self.events.lock().unwrap().push("remove");
        self.values.lock().unwrap().retain(|t| t != token);
        Ok(())
    }
}
#[tokio::test]
#[ignore = "golden vectors: set NWUI_CONTROL_FIXTURES"]
async fn cancellation_awaits_deferred_backend_commit_before_removing_http_challenge() {
    let (client, state, task) = server("token").await;
    let vectors = fixtures();
    let cancel = Cancellation::default();
    let store = CancellingStore {
        cancel: cancel.clone(),
        values: Arc::new(Mutex::new(vec![])),
        events: Arc::new(Mutex::new(vec![])),
    };
    let result = client
        .issue_http(
            vectors["accounts"][0]["accountKeyPkcs8Pem"]
                .as_str()
                .unwrap(),
            "admin@example.com",
            &["example.com".into()],
            &store,
            &cancel,
        )
        .await;
    assert!(matches!(result, Err(CertificateError::Aborted)));
    assert_eq!(
        *store.events.lock().unwrap(),
        vec!["cancel", "commit", "remove"]
    );
    assert!(store.values.lock().unwrap().is_empty());
    assert!(
        state
            .lock()
            .unwrap()
            .payloads
            .iter()
            .all(|(p, _, _)| p != "/challenge/http")
    );
    task.abort();
}

#[tokio::test]
#[ignore = "golden vectors: set NWUI_CONTROL_FIXTURES"]
async fn already_valid_authorization_needs_no_dns_record() {
    let (client, state, task) = server("token").await;
    let vectors = fixtures();
    state.lock().unwrap().auth_valid = true;
    let result = client
        .begin_dns(
            vectors["accounts"][0]["accountKeyPkcs8Pem"]
                .as_str()
                .unwrap(),
            "admin@example.com",
            &["example.com".into()],
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert!(result.records.is_empty());
    task.abort();
}
