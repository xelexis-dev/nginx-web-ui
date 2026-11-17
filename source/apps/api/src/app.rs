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
use crate::{state::AppState, auth::{principal::{self, Use}, route_policy, user_token}};

#[derive(Clone)]
struct ContextState { origin: Option<String>, app: Option<AppState> }

/// Applied inside `finish_router` so rejected requests receive the same HTTP response policy.
/// The listener must install ConnectInfo; never infer a trusted peer from request headers.
pub fn with_request_context(routes: Router, external_origin: Option<String>) -> Router {
    routes.layer(middleware::from_fn_with_state(ContextState { origin: external_origin, app: None }, request_context))
}

pub fn with_user_request_context(routes: Router, state: AppState) -> Router {
    routes.layer(middleware::from_fn_with_state(ContextState { origin: state.config.auth_url.clone(), app: Some(state) }, request_context))
}

async fn request_context(
    State(boundary): State<ContextState>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(ConnectInfo(peer)) = request.extensions().get::<ConnectInfo<SocketAddr>>().copied() else {
        return ApiError::internal().into_response();
    };
    let context = match RequestContext::resolve(request.headers(), peer, boundary.origin.as_deref()) {
        Ok(context) => context,
        Err(error) => return error.into_response(),
    };
    let path = request.uri().path();
    let browser = path == "/api/ui/v1" || path.starts_with("/api/ui/v1/");
    let user_token_request = user_token::is_user_token_request(request.headers());
    let class = route_policy::route(request.method().as_str(), path);
    let node_api = path == "/api/v1" || path.starts_with("/api/v1/");
    let user_surface = !node_api && class.is_none_or(|r|r.user_surface);
    if user_token_request && user_surface && let Some(state) = boundary.app {
        let result = async {
            let usage = class.map_or(Use::SessionOnly, |r|r.usage);
            let user = user_token::authenticate(&state, request.headers(), &context).await?;
            let stream = class.is_some_and(|r|r.stream);
            let permit = state.runtime.user_token_limits.admit(user.token_id().ok_or_else(ApiError::internal)?, &user.user_id,
                !matches!(*request.method(), Method::GET|Method::HEAD|Method::OPTIONS), stream)?;
            // Even policy refusals consume the authenticated token/owner budget.
            user_token::check_fetch_metadata(request.headers())?;
            if usage == Use::SessionOnly { return Err(principal::session_only()); }
            let actions = if class.is_some_and(|r|r.operation_id=="readBrowserView") {
                route_policy::view_actions(request.uri().query())?
            } else { class.map_or(&[][..], |r|r.actions) };
            for &action in actions { principal::authorize(&state, &user, &context, action).await?; }
            let stopped = permit.as_ref().map(|_| crate::auth::stream_guard::token_lifetime(&state, request.headers().clone(), context.clone(), actions.to_vec(), user.expires_ms()));
            request.extensions_mut().insert(context);
            let mut response = next.run(request).await;
            if response.status().is_success() { user_token::record_use(&state, &user).await; }
            response.headers_mut().remove(header::SET_COOKIE);
            if let (Some(permit),Some(stopped))=(permit,stopped) {
                let (parts,body)=response.into_parts();
                response=Response::from_parts(parts, Body::from_stream(crate::auth::token_limits::LimitedStream {body:body.into_data_stream(),_permit:permit,stopped:Some(stopped)}));
            }
            Ok::<_,ApiError>(response)
        }.await;
        return result.unwrap_or_else(IntoResponse::into_response);
    }
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
    let user_token_request = user_token::is_user_token_request(request.headers());
    let head = request.method() == Method::HEAD;
    let mut response = next.run(request).await;
    if user_token_request { response.headers_mut().remove(header::SET_COOKIE); }
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
