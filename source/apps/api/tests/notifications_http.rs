mod auth_support;
use auth_support::*;
use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use std::net::SocketAddr;
use tower::ServiceExt;
fn app(db: &TestDb) -> Router {
    nwui_api::app::finish_router(
        nwui_api::app::with_request_context(
            nwui_api::notifications::router().with_state(db.state.clone()),
            None,
        ),
        &db.state.config.version,
    )
}
fn cookie() -> String {
    let claims = json!({"id":"u1","sv":3});
    let token = nwui_api::auth::session::encode(
        claims.as_object().unwrap(),
        SECRET,
        nwui_api::auth::session::COOKIE_NAME_PLAIN,
        nwui_api::auth::principal::now_unix_ms() / 1000,
        3600,
    )
    .unwrap();
    format!("authjs.session-token={token}")
}
fn request(method: &str, path: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "admin.example.test")
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))))
}
async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let r = app.clone().oneshot(req).await.unwrap();
    let status = r.status();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn viewer_feed_is_safe_bounded_and_seen_is_session_scoped() {
    let db = test_db().await;
    let app = app(&db);
    insert_user(
        &db.client,
        UserRow {
            role: "viewer",
            ..Default::default()
        },
        None,
    )
    .await;
    insert_user(
        &db.client,
        UserRow {
            id: "u2",
            email: "u2@example.test",
            role: "viewer",
            ..Default::default()
        },
        None,
    )
    .await;
    db.client.batch_execute("INSERT INTO notifications(id,type,severity,title,data) SELECT n::text,'cert.issued','success','issued','{\"internal\":\"not-in-feed\"}' FROM generate_series(1,25) n").await.unwrap();
    let get = || {
        request("GET", "/api/ui/v1/notifications")
            .header("cookie", cookie())
            .body(Body::empty())
            .unwrap()
    };
    let (status, body) = send(&app, get()).await;
    assert_eq!(status, 200);
    assert_eq!(body["items"].as_array().unwrap().len(), 20);
    assert_eq!(body["unreadCount"], 25);
    assert!(!body.to_string().contains("not-in-feed"));
    assert_eq!(
        send(
            &app,
            request("POST", "/api/ui/v1/notifications/seen")
                .header("cookie", cookie())
                .body(Body::empty())
                .unwrap()
        )
        .await
        .0,
        403
    );
    let (status, _) = send(
        &app,
        request("POST", "/api/ui/v1/notifications/seen")
            .header("cookie", cookie())
            .header("origin", "http://admin.example.test")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(send(&app, get()).await.1["unreadCount"], 0);
    assert!(
        db.client
            .query_one("SELECT notifications_seen_at FROM users WHERE id='u2'", &[])
            .await
            .unwrap()
            .get::<_, Option<chrono::DateTime<chrono::Utc>>>(0)
            .is_none()
    );
    db.client
        .execute("UPDATE users SET session_version=4 WHERE id='u1'", &[])
        .await
        .unwrap();
    assert_eq!(send(&app, get()).await.0, 401);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn gateway_bearer_cannot_read_or_mark_browser_notifications() {
    let db = test_db().await;
    let app = app(&db);
    let token = "synthetic-notification-gateway";
    auth_support::gateway_issuer(&db.client).await;
    db.client
        .execute(
            "INSERT INTO gateway_tokens(id,name,role,token_hash,created_by_user_id) VALUES ('g','g','admin',$1,'q3-gateway-issuer')",
            &[&nwui_api::auth::token_digest(token)],
        )
        .await
        .unwrap();
    for (method, path) in [
        ("GET", "/api/ui/v1/notifications"),
        ("POST", "/api/ui/v1/notifications/seen"),
    ] {
        let req = request(method, path)
            .header("authorization", format!("Bearer {token}"))
            .header("origin", "http://admin.example.test")
            .body(Body::empty())
            .unwrap();
        assert_eq!(send(&app, req).await.0, 401);
    }
}
