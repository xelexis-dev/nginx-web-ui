use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    routing::get,
};
use http_body_util::BodyExt;
use nwui_api::app::finish_router;
use tower::ServiceExt;

fn routes() -> Router {
    finish_router(
        Router::new().route("/api/v1/probe", get(|| async { "ok" })),
        "1.18.0",
    )
}

#[tokio::test]
async fn unknown_node_paths_keep_machine_readable_versioned_errors() {
    let response = routes()
        .oneshot(
            Request::builder()
                .uri("/api/v1/missing")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response.headers()["x-node-version"], "1.18.0");
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["error"]["code"], "not_found");
}

#[tokio::test]
async fn method_rejection_advertises_allowed_methods_and_uses_the_error_contract() {
    let response = routes()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/probe")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    let allowed = response
        .headers()
        .get(header::ALLOW)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(allowed.split(',').any(|s| s.trim() == "GET"));
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["error"]["code"], "method_not_allowed");
}

#[tokio::test]
async fn head_errors_never_contain_a_body() {
    let response = routes()
        .oneshot(
            Request::builder()
                .method("HEAD")
                .uri("/api/v1/missing")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
}
