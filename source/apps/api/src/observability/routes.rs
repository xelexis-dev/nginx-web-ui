use super::{collector, logs, query};
use crate::{
    auth::{
        principal::{
            UserPrincipal, TwoFactorGate, db_unavailable, require_gateway, require_user,
        },
        rbac::Action,
    },
    error::ApiError,
    request_context::RequestContext,
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{Request, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/metrics", get(read))
        .route("/api/v1/logs/stream", get(read))
        .route("/api/v1/audit", get(read))
        .route("/api/v1/audit/{id}", get(read))
        .route("/api/ui/v1/metrics", get(read))
        .route("/api/ui/v1/logs/stream", get(read))
}
async fn read(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Response, ApiError> {
    let node = request.uri().path().starts_with("/api/v1/");
    if node {
        require_gateway(&state, request.headers(), &ctx, &[Action::ConfigRead]).await?;
    } else {
        require_user(
            &state,
            request.headers(),
            &ctx,
            Some(Action::ConfigRead),
            TwoFactorGate::Enforce, crate::auth::principal::Use::SessionOrToken)
        .await?;
    }
    let path = request
        .uri()
        .path()
        .trim_start_matches(if node { "/api/v1" } else { "/api/ui/v1" });
    let params: BTreeMap<String, String> =
        url::form_urlencoded::parse(request.uri().query().unwrap_or("").as_bytes())
            .into_owned()
            .collect();
    let mut response = if path == "/logs/stream" {
        let path = match params.get("file").map(String::as_str).unwrap_or("access") {
            "access" => state.config.nginx_access_log.clone(),
            "error" => state.config.nginx_error_log.clone(),
            _ => {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "bad_request",
                    "알 수 없는 로그 파일입니다.",
                ));
            }
        };
        // A browser session's stream ends when the session does (see `stream_guard`).
        let shutdown = if node {
            let checked_state = state.clone();
            let headers = request.headers().clone();
            let checked_ctx = ctx.clone();
            crate::auth::stream_guard::checked_shutdown(
                &state, crate::auth::stream_guard::REVALIDATE_EVERY,
                move || {
                    let state = checked_state.clone();
                    let headers = headers.clone();
                    let ctx = checked_ctx.clone();
                    async move { require_gateway(&state, &headers, &ctx, &[Action::ConfigRead]).await.map(|_| ()) }
                },
            )
        } else {
            crate::auth::stream_guard::session_bound_shutdown(
                &state,
                request.headers().clone(),
                ctx.clone(),
                Some(Action::ConfigRead),
                crate::auth::stream_guard::REVALIDATE_EVERY,
            )
        };
        let mut r = logs::response(path, shutdown).into_response();
        r.headers_mut().insert(
            "cache-control",
            HeaderValue::from_static("no-cache, no-transform"),
        );
        r.headers_mut()
            .insert("x-accel-buffering", HeaderValue::from_static("no"));
        r
    } else {
        let data = if path == "/metrics" {
            query::metrics(&state, params.get("range").map(String::as_str)).await?
        } else if path == "/audit" {
            query::audit_list(&state, &params).await?
        } else if let Some(id) = path.strip_prefix("/audit/") {
            query::audit_detail(&state, id).await?
        } else {
            return Err(ApiError::not_found());
        };
        Json(data).into_response()
    };
    if node {
        response.headers_mut().insert(
            "x-node-version",
            HeaderValue::from_str(&state.config.version).map_err(|_| ApiError::internal())?,
        );
    }
    Ok(response)
}

pub async fn view(
    state: &AppState,
    session: &UserPrincipal,
    path: &str,
    params: &BTreeMap<String, String>,
) -> Result<Option<Value>, ApiError> {
    if path != "/"
        && path != "/metrics"
        && path != "/logs"
        && path != "/audit"
        && !path.starts_with("/audit/")
    {
        return Ok(None);
    }
    if !session.can(Action::ConfigRead) {
        return Err(ApiError::forbidden());
    }
    let data = match path {
        "/metrics" => {
            json!({"initialRange":query::range(params.get("range").map(String::as_str)).0})
        }
        "/logs" => json!({"files":["access","error"]}),
        "/audit" => query::audit_list(state, params).await?,
        "/" => dashboard(state, session).await?,
        _ => {
            let id = path.trim_start_matches("/audit/");
            if id.is_empty() || id.contains('/') {
                return Ok(None);
            }
            query::audit_detail(state, id).await?
        }
    };
    Ok(Some(data))
}
async fn dashboard(state: &AppState, session: &UserPrincipal) -> Result<Value, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let since = chrono::Utc::now() - chrono::Duration::hours(24);
    let row=client.query_one("SELECT (SELECT count(*) FROM nginx_servers) AS servers,(SELECT count(*) FROM nginx_servers WHERE enabled) AS active_servers,(SELECT count(*) FROM nginx_streams) AS streams,(SELECT count(*) FROM nginx_streams WHERE enabled) AS active_streams,(SELECT count(*) FROM audit_logs WHERE created_at >= $1) AS audits",&[&since]).await.map_err(|_|db_unavailable())?;
    drop(client);
    let metrics = collector::read_nginx_metrics(state).await;
    Ok(
        json!({"email":session.email,"serverTotal":row.get::<_,i64>("servers"),"serverActive":row.get::<_,i64>("active_servers"),"streamTotal":row.get::<_,i64>("streams"),"streamActive":row.get::<_,i64>("active_streams"),"audit24h":row.get::<_,i64>("audits"),"metrics":metrics}),
    )
}
