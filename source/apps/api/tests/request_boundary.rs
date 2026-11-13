use axum::{Extension, Router, body::Body, extract::ConnectInfo, http::{Request, StatusCode}, routing::{get, post}};
use http_body_util::BodyExt;
use nwui_api::{app::{finish_router, with_request_context}, request_context::RequestContext};
use tower::ServiceExt;

fn routes() -> Router {
    let routes = Router::new()
        .route("/api/ui/v1/probe", post(|Extension(ctx): Extension<RequestContext>| async move { ctx.ip.unwrap() }))
        .route("/api/v1/probe", get(|Extension(ctx): Extension<RequestContext>| async move { ctx.external_origin }));
    finish_router(with_request_context(routes, None), "1.18.0")
}

fn request(path: &str, method: &str, origin: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().uri(path).method(method)
        .header("host", "127.0.0.1:3001")
        .header("x-forwarded-host", "ui.example.test")
        .header("x-forwarded-proto", "https")
        .header("x-real-ip", "203.0.113.42");
    if let Some(origin) = origin { builder = builder.header("origin", origin); }
    let mut request = builder.body(Body::empty()).unwrap();
    request.extensions_mut().insert(ConnectInfo("127.0.0.1:45678".parse::<std::net::SocketAddr>().unwrap()));
    request
}

#[tokio::test]
async fn cookie_writes_enforce_origin_before_dispatch() {
    for origin in [None, Some("null"), Some("https://attacker.test")] {
        let response = routes().oneshot(request("/api/ui/v1/probe", "POST", origin)).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
    let response = routes().oneshot(request("/api/ui/v1/probe", "POST", Some("https://ui.example.test"))).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.into_body().collect().await.unwrap().to_bytes(), "203.0.113.42");
}

#[tokio::test]
async fn gateway_requests_do_not_require_browser_origin() {
    let response = routes().oneshot(request("/api/v1/probe", "GET", None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-node-version"], "1.18.0");
    assert_eq!(response.into_body().collect().await.unwrap().to_bytes(), "https://ui.example.test");
}

#[tokio::test]
async fn missing_peer_metadata_is_not_treated_as_a_trusted_proxy() {
    let mut request = request("/api/v1/probe", "GET", None);
    request.extensions_mut().remove::<ConnectInfo<std::net::SocketAddr>>();
    let response = routes().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
