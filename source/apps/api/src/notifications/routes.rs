use crate::{
    auth::{
        principal::{TwoFactorGate, db_unavailable, require_session},
        rbac::Action,
    },
    contracts::FormResult,
    error::ApiError,
    http_policy::{check_browser_origin, read_form},
    request_context::RequestContext,
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{Request, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/ui/v1/notifications", get(feed))
        .route("/api/ui/v1/notifications/seen", post(seen))
        .route("/api/ui/v1/forms/notifications.markSeen", post(mark_form))
}
async fn feed(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let session = require_session(
        &state,
        request.headers(),
        &ctx,
        Some(Action::ConfigRead),
        TwoFactorGate::Enforce,
    )
    .await?;
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let rows=client.query("SELECT id,type,severity::text,title,body,link,created_at FROM notifications ORDER BY created_at DESC,id DESC LIMIT 20",&[]).await.map_err(|_|db_unavailable())?;
    let count:i64=client.query_one("SELECT count(*) FROM notifications WHERE created_at>coalesce((SELECT notifications_seen_at FROM users WHERE id=$1),'-infinity'::timestamptz)",&[&session.user_id]).await.map_err(|_|db_unavailable())?.get(0);
    let items:Vec<Value>=rows.iter().map(|r|json!({"id":r.get::<_,String>("id"),"type":r.get::<_,String>("type"),"severity":r.get::<_,String>("severity"),"title":r.get::<_,String>("title"),"body":r.get::<_,Option<String>>("body"),"link":r.get::<_,Option<String>>("link"),"createdAt":r.get::<_,DateTime<Utc>>("created_at").to_rfc3339_opts(chrono::SecondsFormat::Millis,true)})).collect();
    Ok(Json(json!({"items":items,"unreadCount":count})))
}
async fn seen(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    check_browser_origin(request.headers(), &ctx)?;
    let session = require_session(
        &state,
        request.headers(),
        &ctx,
        Some(Action::ConfigRead),
        TwoFactorGate::Enforce,
    )
    .await?;
    mark_seen(&state, &session.user_id).await?;
    Ok(Json(json!({"ok":true})))
}
async fn mark_form(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<FormResult>, ApiError> {
    check_browser_origin(request.headers(), &ctx)?;
    let session = require_session(
        &state,
        request.headers(),
        &ctx,
        Some(Action::ConfigRead),
        TwoFactorGate::Enforce,
    )
    .await?;
    let form = read_form(request).await?;
    if !form.args.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "이 작업은 인수를 받지 않습니다.",
        ));
    }
    mark_seen(&state, &session.user_id).await?;
    Ok(Json(FormResult {
        result: Some(json!({"ok":true})),
        redirect_to: None,
        refresh: Some(true),
    }))
}
async fn mark_seen(state: &AppState, user_id: &str) -> Result<(), ApiError> {
    state
        .db
        .get()
        .await
        .map_err(|_| db_unavailable())?
        .execute(
            "UPDATE users SET notifications_seen_at=now() WHERE id=$1",
            &[&user_id],
        )
        .await
        .map_err(|_| db_unavailable())?;
    Ok(())
}
