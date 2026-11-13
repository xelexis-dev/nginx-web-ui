use axum::{
    http::{StatusCode, header},
    response::IntoResponse,
};
use http_body_util::BodyExt;
use nwui_api::error::ApiError;

#[tokio::test]
async fn missing_credentials_advertise_authentication_and_are_not_cacheable() {
    let response = ApiError::unauthorized().into_response();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers().get(header::WWW_AUTHENTICATE).unwrap(),
        "Bearer realm=\"nginx-web-ui\""
    );
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn internal_failures_return_a_safe_json_envelope() {
    let response = ApiError::internal().into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body,
        serde_json::json!({"error":{"code":"internal_error","message":"요청을 처리하지 못했습니다."}})
    );
}
