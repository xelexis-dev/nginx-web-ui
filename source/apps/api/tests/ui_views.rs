mod auth_support;
use auth_support::*;
use axum::{body::Body, extract::ConnectInfo, http::{Request, StatusCode}};
use nwui_api::auth::{principal::now_unix_ms, session};
use serde_json::json;
use tower::ServiceExt;

async fn request(db: &TestDb, path: &str, claims: Option<serde_json::Value>) -> axum::response::Response {
    let mut req = Request::builder().uri(path).header("host", "admin.example.test")
        .extension(ConnectInfo("127.0.0.1:45000".parse::<std::net::SocketAddr>().unwrap()));
    if let Some(claims) = claims {
        let token = session::encode(claims.as_object().unwrap(), SECRET, session::COOKIE_NAME_PLAIN, now_unix_ms()/1000, 3600).unwrap();
        req = req.header("cookie", format!("{}={token}", session::COOKIE_NAME_PLAIN));
    }
    nwui_api::server::router(db.state.clone()).oneshot(req.body(Body::empty()).unwrap()).await.unwrap()
}

#[tokio::test]
#[ignore = "real DB: NWUI_TEST_DATABASE_URL"]
async fn views_require_fresh_session_and_mfa_before_dispatch() {
    let db = test_db().await;
    insert_user(&db.client, UserRow::default(), None).await;
    let path = "/api/ui/v1/views?path=%2Freverse-proxy%2Fupstreams";
    assert_eq!(request(&db,path,None).await.status(),StatusCode::UNAUTHORIZED);
    let claims = json!({"id":"u1","sv":3,"requires2fa":true});
    assert_eq!(request(&db,path,Some(claims)).await.status(),StatusCode::FORBIDDEN);
    let claims = json!({"id":"u1","sv":3});
    let reply = request(&db,path,Some(claims.clone())).await;
    assert_eq!(reply.status(),StatusCode::OK);
    assert!(!reply.headers().contains_key("set-cookie"));
    let body: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(reply.into_body(),4096).await.unwrap()).unwrap();
    assert_eq!(body["upstreams"],json!([]));
    assert_eq!(body["canWrite"],true);
    db.client.execute("UPDATE users SET session_version=4", &[]).await.unwrap();
    assert_eq!(request(&db,path,Some(claims)).await.status(),StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "real DB: NWUI_TEST_DATABASE_URL"]
async fn malformed_unknown_missing_and_read_only_views_are_distinct() {
    let db = test_db().await;
    insert_user(&db.client, UserRow{role:"viewer",..UserRow::default()}, None).await;
    let claims = json!({"id":"u1","sv":3});
    for (path, status) in [
        ("/api/ui/v1/views",400),
        ("/api/ui/v1/views?path=https%3A%2F%2Fevil.test",400),
        ("/api/ui/v1/views?path=%2Funknown",404),
        ("/api/ui/v1/views?path=%2Freverse-proxy%2Fupstreams%2Fmissing",404),
        ("/api/ui/v1/views?path=%2Freverse-proxy%2Fupstreams%2Fnew",403),
    ] { assert_eq!(request(&db,path,Some(claims.clone())).await.status().as_u16(),status,"{path}"); }
}
