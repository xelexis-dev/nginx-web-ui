use crate::{error::ApiError, http_policy::check_browser_origin, request_context::RequestContext};
use axum::{
    Router,
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use std::net::SocketAddr;

/// Applied inside `finish_router` so rejected requests receive the same HTTP response policy.
/// The listener must install ConnectInfo; never infer a trusted peer from request headers.
pub fn with_request_context(routes: Router, external_origin: Option<String>) -> Router {
    routes.layer(middleware::from_fn_with_state(external_origin, request_context))
}

async fn request_context(
    State(external_origin): State<Option<String>>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(ConnectInfo(peer)) = request.extensions().get::<ConnectInfo<SocketAddr>>().copied() else {
        return ApiError::internal().into_response();
    };
    let context = match RequestContext::resolve(request.headers(), peer, external_origin.as_deref()) {
        Ok(context) => context,
        Err(error) => return error.into_response(),
    };
    let path = request.uri().path();
    let browser = path == "/api/ui/v1" || path.starts_with("/api/ui/v1/");
    if browser && !matches!(*request.method(), Method::GET | Method::HEAD | Method::OPTIONS)
        && let Err(error) = check_browser_origin(request.headers(), &context) {
        return error.into_response();
    }
    request.extensions_mut().insert(context);
    next.run(request).await
}

pub fn finish_router(routes: Router, version: &str) -> Router {
    routes
        .fallback(|| async { ApiError::not_found() })
        .method_not_allowed_fallback(|| async {
            ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "이 경로에서 지원하지 않는 요청 방식입니다.",
            )
        })
        .layer(middleware::from_fn_with_state(
            version.to_owned(),
            response_policy,
        ))
}

async fn response_policy(State(version): State<String>, request: Request, next: Next) -> Response {
    let node_api =
        request.uri().path() == "/api/v1" || request.uri().path().starts_with("/api/v1/");
    let head = request.method() == Method::HEAD;
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if node_api && response.status() != StatusCode::UNAUTHORIZED {
        // Product version is read from the same build input as Next; never from a caller header.
        match HeaderValue::from_str(&version) {
            Ok(value) => {
                response.headers_mut().insert("x-node-version", value);
            }
            Err(_) => {
                return ApiError::internal().into_response();
            }
        }
    }
    // Fallbacks and errors follow RFC 9110 §9.3.2 too, not just GET routes.
    if head {
        *response.body_mut() = Body::empty();
    }
    response
}
