//! Lightweight node-owned DNS availability. Authentication is shared with node reads;
//! the handler does not load configuration rows, resolve targets or execute nginx.
use crate::{
    auth::{principal::require_gateway, rbac::Action},
    error::ApiError,
    request_context::RequestContext,
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{Request, State},
    routing::get,
};
use serde_json::{Value, json};

pub(super) fn router() -> Router<AppState> {
    Router::new().route("/api/v1/runtime-dns", get(status))
}

async fn status(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    require_gateway(&state, request.headers(), &ctx, &[Action::ConfigRead]).await?;
    let status = crate::dns_guard::status(&state.config);
    Ok(Json(
        json!({"available": status.available, "reason": status.reason}),
    ))
}
