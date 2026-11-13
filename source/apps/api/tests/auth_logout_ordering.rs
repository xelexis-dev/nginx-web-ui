//! Regression for a delayed browser refresh response arriving after successful logout.
//! The server processes refresh first; delivery of its Set-Cookie is deliberately delayed.
mod auth_support;

use axum::{body::Body, extract::ConnectInfo, http::{Request, StatusCode}, Router};
use nwui_api::auth::session;
use serde_json::json;
use std::net::SocketAddr;
use tower::ServiceExt;

async fn request(app: &Router, method: &str, path: &str, cookie: &str) -> axum::response::Response {
    app.clone().oneshot(Request::builder().method(method).uri(path)
        .header("host", "admin.example.test")
        .header("origin", "http://admin.example.test")
        .header("cookie", cookie)
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))))
        .body(Body::empty()).unwrap()).await.unwrap()
}

fn renewed_cookie(response: &axum::response::Response) -> String {
    response.headers().get_all("set-cookie").iter()
        .map(|v|v.to_str().unwrap())
        .find(|v|!v.contains("Max-Age=0"))
        .expect("short-lived session must be renewed")
        .split(';').next().unwrap().to_owned()
}

#[tokio::test]
#[ignore = "real PostgreSQL; regression requires logout revocation decision"]
async fn delayed_refresh_cookie_cannot_restore_a_signed_out_session() {
    let db = auth_support::test_db().await;
    auth_support::insert_user(&db.client, auth_support::UserRow::default(), None).await;
    let app = nwui_api::server::router(db.state.clone());
    let now = nwui_api::auth::principal::now_unix_ms() / 1000;
    let claims = json!({"id":"u1", "sub":"u1", "sv":3, "requires2fa":false});
    let token = session::encode(claims.as_object().unwrap(), auth_support::SECRET,
        session::COOKIE_NAME_PLAIN, now, 3600).unwrap();
    let original_cookie = format!("{}={token}", session::COOKIE_NAME_PLAIN);

    // The activity listener starts this on pointerdown, before the logout form submits.
    let refresh = request(&app, "POST", "/api/ui/v1/session/refresh", &original_cookie).await;
    assert_eq!(refresh.status(), StatusCode::OK);
    let delayed_cookie = renewed_cookie(&refresh);

    // Logout reaches the browser first and clears its current cookie.
    let logout = request(&app, "POST", "/api/ui/v1/forms/session.signOut", &original_cookie).await;
    assert_eq!(logout.status(), StatusCode::OK);
    assert!(logout.headers().get_all("set-cookie").iter()
        .any(|v|v.to_str().unwrap().contains("Max-Age=0")));

    // An in-flight response then installs the refreshed cookie. It must not authenticate.
    let after = request(&app, "GET", "/api/ui/v1/session", &delayed_cookie).await;
    assert_eq!(after.status(), StatusCode::UNAUTHORIZED,
        "a delayed Set-Cookie response restored the session after successful logout");
}
