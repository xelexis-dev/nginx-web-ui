//! Read-only, authenticated presentation dispatcher. Paths select a static view;
//! they are never file names, SQL, or arbitrary remote destinations.
use crate::{auth::principal::{self, TwoFactorGate}, error::ApiError, request_context::RequestContext, state::AppState};
use axum::{Extension, Json, Router, extract::{Query, State}, http::{HeaderMap, StatusCode}, routing::get};
use serde_json::Value;
use std::collections::BTreeMap;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/ui/v1/views", get(read))
}

async fn read(
    State(state): State<AppState>, Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap, Query(mut query): Query<BTreeMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let session = principal::require_session(&state, &headers, &ctx, None, TwoFactorGate::Enforce).await?;
    let path = query.remove("path").filter(|p| p.starts_with('/') && !p.starts_with("//")
        && !p.contains(['\\', '?', '#']) && !p.chars().any(char::is_control))
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid_view", "올바른 화면 경로가 필요합니다."))?;
    if let Some(mut view) = crate::control::views::view(&state, &session, &path, &query).await? {
        if path == "/stream" || path == "/stream/new" || (path.starts_with("/stream/") && path.ends_with("/edit")) {
            let client = state.db.get().await.map_err(|_| principal::db_unavailable())?;
            view["reservedPorts"] = serde_json::to_value(crate::control::resources::reserved_ports(&**client, &state).await?).map_err(|_| ApiError::internal())?;
        }
        if path == "/stream" || path == "/reverse-proxy" {
            view["pending"] = serde_json::to_value(crate::control::pending::status(&state).await?).map_err(|_| ApiError::internal())?;
        }
        return Ok(Json(view));
    }
    if let Some(view) = crate::control::operations::view(&state, &session, &path, &query).await? {
        return Ok(Json(view));
    }
    if let Some(view) = crate::nodes::views::view(&state, &session, &path, &query).await? {
        return Ok(Json(view));
    }
    if let Some(view) = crate::observability::view(&state, &session, &path, &query).await? {
        return Ok(Json(view));
    }
    if let Some(view) = crate::certificates::views::view(&state, &session, &path, &query).await? {
        return Ok(Json(view));
    }
    Err(ApiError::not_found())
}
