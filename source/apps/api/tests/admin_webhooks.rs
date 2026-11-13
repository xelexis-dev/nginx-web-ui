//! HTTP tests for the admin webhook targets through the root boundary, on a real PostgreSQL
//! database with local receivers.
//!
//!     NWUI_TEST_DATABASE_URL=postgres://user:pass@host:5432/postgres cargo test --test admin_webhooks -- --include-ignored

mod outbound_support;
mod auth_support;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use auth_support::{TestDb, UserRow, audit_rows, insert_user, test_db};
use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request, StatusCode};
use nwui_api::auth::session;
use serde_json::{Value, json};
use tower::ServiceExt as _;

const ORIGIN: &str = "http://admin.example.test";
const BASE: &str = "/api/ui/v1/admin/notifications/webhooks";

fn app(db: &TestDb) -> Router {
    nwui_api::app::with_request_context(nwui_api::admin::router().with_state(db.state.clone()), None)
}

async fn boss(db: &TestDb) -> String {
    insert_user(&db.client, UserRow { id: "boss", email: "boss@example.test", role: "admin", totp_enabled: true, ..Default::default() }, Some("x")).await;
    let now = nwui_api::auth::principal::now_unix_ms() / 1000;
    let claims = json!({"sub": "boss", "id": "boss", "sv": 3, "requires2fa": true, "mfaVerified": true});
    let token = session::encode(claims.as_object().unwrap(), auth_support::SECRET, session::COOKIE_NAME_PLAIN, now, 3600).unwrap();
    format!("authjs.session-token={token}")
}

async fn call(app: &Router, method: &str, path: &str, cookie: &str, form: Option<&str>) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "admin.example.test")
        .header("cookie", cookie)
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));
    if form.is_some() {
        builder = builder.header("origin", ORIGIN).header("content-type", "application/x-www-form-urlencoded");
    }
    let response = app.clone().oneshot(builder.body(Body::from(form.unwrap_or("").to_owned())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() })
}

fn enc(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

type Seen = Arc<Mutex<Vec<(HeaderMap, String)>>>;

/// A receiver answering `status` + `body` at `/hook`; returns its origin.
async fn receiver(status: u16, body: &'static str) -> (String, Seen) {
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let app = axum::Router::new().route(
        "/hook",
        axum::routing::post(move |headers: HeaderMap, text: String| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push((headers, text));
                (StatusCode::from_u16(status).unwrap(), body)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), seen)
}

async fn stored(db: &TestDb, id: &str) -> (String, Option<String>) {
    let row = db.client.query_one("SELECT url_enc, headers_enc FROM webhook_targets WHERE id = $1", &[&id]).await.unwrap();
    (row.get(0), row.get(1))
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn webhook_targets_are_validated_sealed_and_masked() {
    let db = test_db().await;
    let app = app(&db);
    let boss = boss(&db).await;
    let create = format!("{BASE_FORMS}.create", BASE_FORMS = "/api/ui/v1/forms/admin.webhooks");

    let (status, body) = call(&app, "POST", &create, &boss, Some("name=+&url=ftp%3A%2F%2Fx&format=generic")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["error"]["fieldErrors"],
        json!({"name": "이름을 입력하세요.", "events": "보낼 사건을 하나 이상 고르세요.", "url": "http:// 또는 https:// 주소만 사용할 수 있습니다."})
    );
    let (_, body) = call(&app, "POST", &create, &boss, Some("name=a&url=&events=cert.issued&events=x.y")).await;
    assert_eq!(body["error"]["fieldErrors"], json!({"events": "알 수 없는 사건: x.y", "url": "webhook URL 을 입력하세요."}));
    let (_, body) = call(&app, "POST", &create, &boss, Some(&format!("name=a&url={}&events=cert.issued&format=custom&bodyTemplate={}", enc("https://h.test/x"), enc("{\"t\":\"{{nope}}\"}")))).await;
    assert_eq!(body["error"]["fieldErrors"]["bodyTemplate"], "알 수 없는 변수: {{nope}}");
    let (_, body) = call(&app, "POST", &create, &boss, Some(&format!("name=a&url={}&events=cert.issued&headers={}", enc("https://h.test/x"), enc("Host: x")))).await;
    assert_eq!(body["error"]["fieldErrors"]["headers"], "1번째 줄: \"Host\" 은(는) 직접 지정할 수 없는 헤더입니다.");

    let url = "https://discord.test/api/webhooks/123/SECRET-PATH";
    let form = format!(
        "name=%EC%9A%B4%EC%98%81&enabled=true&format=discord&events=cert.issued&events=cert.failed&events=cert.issued&url={}&headers={}",
        enc(url),
        enc("Authorization: Bearer SECRET-HEADER\nX-B: 2")
    );
    let (status, body) = call(&app, "POST", &create, &boss, Some(&form)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body["redirectTo"].as_str().unwrap().strip_prefix("/admin/notifications/webhooks/").unwrap().to_owned();
    let (url_enc, headers_enc) = stored(&db, &id).await;
    assert!(url_enc.starts_with("enc:v1:") && headers_enc.as_deref().unwrap().starts_with("enc:v1:"));
    assert_eq!(db.state.secrets.open(Some(&url_enc)).as_deref(), Some(url));
    assert_eq!(db.state.secrets.open(headers_enc.as_deref()).as_deref(), Some("{\"Authorization\":\"Bearer SECRET-HEADER\",\"X-B\":\"2\"}"), "JSON in entry order, like JSON.stringify");
    let audit = audit_rows(&db.client, "settings:webhook-create").await;
    let after: Value = audit[0].get("after");
    assert_eq!(after, json!({"name": "운영", "enabled": true, "format": "discord", "host": "discord.test", "events": ["cert.issued", "cert.failed"], "headerCount": 2}));

    let (status, list) = call(&app, "GET", BASE, &boss, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!((list[0]["id"].as_str(), list[0]["format"].as_str(), &list[0]["events"]), (Some(id.as_str()), Some("discord"), &json!(["cert.issued", "cert.failed"])));
    assert_eq!((&list[0]["lastAttemptAt"], &list[0]["lastError"]), (&Value::Null, &Value::Null));
    let (status, detail) = call(&app, "GET", &format!("{BASE}/{id}"), &boss, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!((detail["maskedUrl"].as_str(), &detail["headerNames"], &detail["openError"]), (Some("https://discord.test/api/..."), &json!(["Authorization", "X-B"]), &Value::Null));
    for text in [list.to_string(), detail.to_string()] {
        assert!(!text.contains("SECRET"), "no secret in views: {text}");
    }
    assert_eq!(call(&app, "GET", &format!("{BASE}/missing"), &boss, None).await.0, StatusCode::NOT_FOUND);

    // Keep the URL and headers.
    let update = "/api/ui/v1/forms/admin.webhooks.update";
    let (status, body) = call(&app, "POST", update, &boss, Some(&format!("id={id}&name=b&format=generic&events=cert.failed&url=__keep__&headers="))).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"redirectTo": format!("/admin/notifications/webhooks/{id}")})));
    assert_eq!(stored(&db, &id).await, (url_enc.clone(), headers_enc.clone()));
    let after: Value = audit_rows(&db.client, "settings:webhook-update").await[0].get("after");
    assert_eq!((&after["urlChanged"], &after["headersChanged"], &after["format"]), (&json!(false), &json!(false), &json!("generic")));
    // A blank URL is not "keep".
    let (_, body) = call(&app, "POST", update, &boss, Some(&format!("id={id}&name=b&events=cert.failed&url="))).await;
    assert_eq!(body["error"]["fieldErrors"]["url"], "webhook URL 을 입력하세요.");
    // Clear the headers.
    call(&app, "POST", update, &boss, Some(&format!("id={id}&name=b&events=cert.failed&url=__keep__&headers=X%3A+1&clearHeaders=true"))).await;
    assert_eq!(stored(&db, &id).await, (url_enc.clone(), None));
    let (status, _) = call(&app, "POST", update, &boss, Some("id=missing&name=b&events=cert.failed&url=__keep__")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let delete = "/api/ui/v1/forms/admin.webhooks.delete";
    let (status, body) = call(&app, "POST", delete, &boss, Some(&format!("id={id}"))).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"redirectTo": "/admin/notifications/webhooks"})));
    assert_eq!(db.client.query_one("SELECT count(*) FROM webhook_targets", &[]).await.unwrap().get::<_, i64>(0), 0);
    let before: Value = audit_rows(&db.client, "settings:webhook-delete").await[0].get("before");
    assert_eq!(before, json!({"name": "b", "format": "generic", "events": ["cert.failed"]}));
    call(&app, "POST", delete, &boss, Some(&format!("id={id}"))).await;
    assert_eq!(audit_rows(&db.client, "settings:webhook-delete").await.len(), 1, "deleting nothing audits nothing");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn test_send_fills_stored_values_but_sends_stored_headers_only_to_their_origin() {
    let db = test_db().await;
    let app = app(&db);
    let boss = boss(&db).await;
    let (a, seen_a) = receiver(204, "").await;
    let (b, seen_b) = receiver(204, "").await;
    let (bad, _) = receiver(404, "nope").await;
    let test = "/api/ui/v1/forms/admin.webhooks.test";

    let (_, body) = call(&app, "POST", "/api/ui/v1/forms/admin.webhooks.create", &boss, Some(&format!("name=a&events=cert.issued&url={}&headers={}", enc(&format!("{a}/hook")), enc("X-Token: stored")))).await;
    let id = body["redirectTo"].as_str().unwrap().rsplit('/').next().unwrap().to_owned();

    let (status, body) = call(&app, "POST", test, &boss, Some(&format!("id={id}&url=__keep__&format=generic"))).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"result": {"ok": true, "message": "테스트 알림을 보냈습니다 (HTTP 204)."}})));
    assert_eq!(seen_a.lock().unwrap()[0].0["x-token"], "stored");
    let (status, _) = call(&app, "POST", test, &boss, Some(&format!("id={id}&url={}", enc(&format!("{a}/hook?other=1"))))).await;
    assert_eq!(status, StatusCode::OK, "same origin keeps the stored headers");
    assert_eq!(seen_a.lock().unwrap()[1].0["x-token"], "stored");

    let (status, body) = call(&app, "POST", test, &boss, Some(&format!("id={id}&url={}", enc(&format!("{b}/hook"))))).await;
    assert_eq!((status, body["error"]["fieldErrors"]["headers"].as_str()), (StatusCode::BAD_REQUEST, Some("다른 주소로 시험하려면 헤더를 다시 입력하세요. 저장된 헤더는 저장된 주소로만 보냅니다.")));
    assert!(seen_b.lock().unwrap().is_empty());
    let (status, _) = call(&app, "POST", test, &boss, Some(&format!("id={id}&url={}&headers={}", enc(&format!("{b}/hook")), enc("X-Token: typed")))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(seen_b.lock().unwrap()[0].0["x-token"], "typed");
    let (status, _) = call(&app, "POST", test, &boss, Some(&format!("id={id}&url={}&clearHeaders=true", enc(&format!("{b}/hook"))))).await;
    assert_eq!(status, StatusCode::OK, "no headers at all is fine");
    assert!(seen_b.lock().unwrap()[1].0.get("x-token").is_none());

    let (status, body) = call(&app, "POST", test, &boss, Some("url=")).await;
    assert_eq!((status, body["error"]["fieldErrors"]["url"].as_str()), (StatusCode::BAD_REQUEST, Some("먼저 webhook URL 을 입력한 뒤 다시 시도하세요.")));
    let (_, body) = call(&app, "POST", test, &boss, Some(&format!("id={id}&url=not-a-url"))).await;
    assert_eq!(body["error"]["fieldErrors"]["url"], "URL 형식이 올바르지 않습니다.", "a typo is refused, not silently replaced by the stored URL");
    let (_, body) = call(&app, "POST", test, &boss, Some(&format!("id={id}&url=__keep__&headers={}", enc("bad line")))).await;
    assert_eq!(body["error"]["fieldErrors"]["headers"], "1번째 줄: \"이름: 값\" 형식이어야 합니다.");
    let (_, body) = call(&app, "POST", test, &boss, Some(&format!("id={id}&url=__keep__&format=custom&bodyTemplate="))).await;
    assert_eq!(body["error"]["fieldErrors"]["bodyTemplate"], "본문 템플릿을 입력하세요.");
    let (status, _) = call(&app, "POST", test, &boss, Some("id=missing&url=__keep__")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = call(&app, "POST", test, &boss, Some(&format!("url={}", enc(&format!("{bad}/hook"))))).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"result": {"ok": false, "error": "HTTP 404 Not Found"}})));

    // Tests are not the target's last attempt, but they are audited (host only) before sending.
    let row = db.client.query_one("SELECT last_attempt_at IS NULL FROM webhook_targets WHERE id = $1", &[&id]).await.unwrap();
    assert!(row.get::<_, bool>(0));
    let tests = db.client.query("SELECT target, after FROM audit_logs WHERE action = 'settings:webhook-test' ORDER BY created_at", &[]).await.unwrap();
    assert_eq!(tests.len(), 5, "only sends that ran are audited");
    let host = a.strip_prefix("http://").unwrap();
    assert_eq!((tests[0].get::<_, Option<String>>(0), tests[0].get::<_, Value>(1)), (Some(id.clone()), json!({"host": host, "format": "generic", "storedUrl": true, "storedHeaders": true})));
    assert_eq!(tests[2].get::<_, Value>(1)["storedHeaders"], false, "typed headers");
    assert_eq!(tests[4].get::<_, Option<String>>(0), None, "an unsaved target");

    // A lost key is said, not skipped.
    let foreign = nwui_api::crypto::SecretBox::from_key([9u8; 32]).seal("https://x.test/").unwrap();
    db.client.execute("UPDATE webhook_targets SET url_enc = $2 WHERE id = $1", &[&id, &foreign]).await.unwrap();
    let (status, body) = call(&app, "POST", test, &boss, Some(&format!("id={id}&url=__keep__"))).await;
    assert_eq!((status, body["error"]["message"].as_str()), (StatusCode::CONFLICT, Some(nwui_api::admin::webhooks::URL_UNREADABLE)));
    let (_, detail) = call(&app, "GET", &format!("{BASE}/{id}"), &boss, None).await;
    assert_eq!((&detail["maskedUrl"], detail["openError"].as_str()), (&Value::Null, Some(nwui_api::admin::webhooks::URL_UNREADABLE)));
}
