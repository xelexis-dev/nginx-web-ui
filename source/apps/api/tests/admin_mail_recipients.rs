//! HTTP tests for the admin mail recipients through the root boundary, on a real PostgreSQL
//! database.
//!
//!     NWUI_TEST_DATABASE_URL=postgres://user:pass@host:5432/postgres cargo test --test admin_mail_recipients -- --include-ignored

mod auth_support;

use std::net::SocketAddr;

use auth_support::{TestDb, UserRow, audit_rows, insert_user, test_db};
use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use nwui_api::auth::session;
use serde_json::{Value, json};
use tower::ServiceExt as _;

const BASE: &str = "/api/ui/v1/admin/notifications/mail";
const FORMS: &str = "/api/ui/v1/forms/admin.mailRecipients";

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
        builder = builder.header("origin", "http://admin.example.test").header("content-type", "application/x-www-form-urlencoded");
    }
    let response = app.clone().oneshot(builder.body(Body::from(form.unwrap_or("").to_owned())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() })
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn mail_recipients_crud() {
    let db = test_db().await;
    let app = app(&db);
    let boss = boss(&db).await;
    let create = format!("{FORMS}.create");
    let update = format!("{FORMS}.update");

    let (status, body) = call(&app, "POST", &create, &boss, Some("address=+")).await;
    assert_eq!((status, &body["error"]["fieldErrors"]), (StatusCode::BAD_REQUEST, &json!({"address": "수신 주소를 입력하세요.", "events": "받을 사건을 하나 이상 고르세요."})));
    let (_, body) = call(&app, "POST", &create, &boss, Some("address=a%40b&events=cert.issued&events=x.y&events=z")).await;
    assert_eq!(body["error"]["fieldErrors"], json!({"address": "주소 형식이 올바르지 않습니다.", "events": "알 수 없는 사건: x.y, z"}));

    let (status, body) = call(&app, "POST", &create, &boss, Some("label=+%EC%9A%B4%EC%98%81+&address=+Ops%40Example.test+&enabled=true&events=cert.failed&events=cert.issued&events=cert.failed")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body["redirectTo"].as_str().unwrap().strip_prefix("/admin/notifications/mail/").unwrap().to_owned();
    let after: Value = audit_rows(&db.client, "settings:mail-recipient-create").await[0].get("after");
    assert_eq!(after, json!({"label": "운영", "address": "Ops@Example.test", "enabled": true, "events": ["cert.failed", "cert.issued"]}));

    // Same address in another case is a duplicate.
    let (status, body) = call(&app, "POST", &create, &boss, Some("address=ops%40example.TEST&events=cert.issued")).await;
    assert_eq!((status, body["error"]["fieldErrors"]["address"].as_str()), (StatusCode::BAD_REQUEST, Some("이미 등록된 주소입니다.")));
    let (_, body) = call(&app, "POST", &create, &boss, Some("address=second%40example.test&events=cert.issued")).await;
    let second = body["redirectTo"].as_str().unwrap().rsplit('/').next().unwrap().to_owned();

    let (status, list) = call(&app, "GET", BASE, &boss, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().iter().map(|r| r["address"].as_str().unwrap()).collect::<Vec<_>>(), ["Ops@Example.test", "second@example.test"]);
    assert_eq!((&list[1]["label"], &list[1]["enabled"], &list[1]["lastAttemptAt"]), (&Value::Null, &json!(false), &Value::Null));
    let (status, detail) = call(&app, "GET", &format!("{BASE}/{id}"), &boss, None).await;
    assert_eq!((status, detail["label"].as_str(), &detail["events"]), (StatusCode::OK, Some("운영"), &json!(["cert.failed", "cert.issued"])));
    assert_eq!(call(&app, "GET", &format!("{BASE}/missing"), &boss, None).await.0, StatusCode::NOT_FOUND);

    // Keeping its own address is not a duplicate; taking another one's is.
    let (status, body) = call(&app, "POST", &update, &boss, Some(&format!("id={id}&address=OPS%40example.test&events=cert.renewal_due&label="))).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"redirectTo": format!("/admin/notifications/mail/{id}")})));
    let audit = &audit_rows(&db.client, "settings:mail-recipient-update").await[0];
    assert_eq!((audit.get::<_, Value>("before")["address"].as_str(), audit.get::<_, Value>("after")), (Some("Ops@Example.test"), json!({"label": null, "address": "OPS@example.test", "enabled": false, "events": ["cert.renewal_due"]})));
    let (status, body) = call(&app, "POST", &update, &boss, Some(&format!("id={second}&address=ops%40EXAMPLE.test&events=cert.issued"))).await;
    assert_eq!((status, body["error"]["fieldErrors"]["address"].as_str()), (StatusCode::BAD_REQUEST, Some("이미 등록된 주소입니다.")));
    assert_eq!(call(&app, "POST", &update, &boss, Some("id=missing&address=n%40example.test&events=cert.issued")).await.0, StatusCode::NOT_FOUND);

    let (status, body) = call(&app, "POST", &format!("{FORMS}.delete"), &boss, Some(&format!("id={id}"))).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"redirectTo": "/admin/notifications/mail"})));
    let before: Value = audit_rows(&db.client, "settings:mail-recipient-delete").await[0].get("before");
    assert_eq!(before, json!({"address": "OPS@example.test", "events": ["cert.renewal_due"]}));
    call(&app, "POST", &format!("{FORMS}.delete"), &boss, Some(&format!("id={id}"))).await;
    assert_eq!(audit_rows(&db.client, "settings:mail-recipient-delete").await.len(), 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn concurrent_saves_cannot_both_take_one_address() {
    let db = test_db().await;
    let app = app(&db);
    let boss = boss(&db).await;
    let create = format!("{FORMS}.create");
    for round in 0..5 {
        let form = format!("address=race{round}%40example.test&events=cert.issued");
        let results = futures_join(call(&app, "POST", &create, &boss, Some(&form)), call(&app, "POST", &create, &boss, Some(&form.replace("race", "RACE")))).await;
        let statuses = [results.0.0, results.1.0];
        assert_eq!(statuses.iter().filter(|s| **s == StatusCode::OK).count(), 1, "round {round}: {statuses:?}");
    }
    assert_eq!(db.client.query_one("SELECT count(*) FROM mail_recipients", &[]).await.unwrap().get::<_, i64>(0), 5);
}

async fn futures_join<A: std::future::Future, B: std::future::Future>(a: A, b: B) -> (A::Output, B::Output) {
    tokio::join!(a, b)
}
