use super::service;
use crate::{
    auth::{
        audit::{self, AuditEntry},
        principal::{self, TwoFactorGate},
        rbac::{Action, role_str},
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
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};

pub const FORM_IDS: &[&str] = &[
    "nodes.pairNode",
    "nodes.updateNode",
    "nodes.unpairNode",
    "nodes.refreshStatuses",
];
pub fn router() -> Router<AppState> {
    let mut router = Router::new()
        .merge(super::runtime_dns::router())
        .route("/api/v1/info", get(info))
        .route("/api/v1/identity-challenge", post(super::identity::challenge))
        .route("/api/v1/pair", post(pair).delete(unpair));
    for id in FORM_IDS {
        router = router.route(&format!("/api/ui/v1/forms/{id}"), post(form));
    }
    router
}
async fn info(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let gateway = principal::require_gateway(&state, request.headers(), &ctx, &[]).await?;
    let mut info = service::info(&state).await?;
    info["gatewayTokenId"] = json!(gateway.token_id);
    Ok(Json(info))
}
async fn pair(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    principal::check_gateway_version(request.headers(), &state.config.version)?;
    let now = principal::now_unix_ms();
    principal::check_pairing_ip_lock(&state, &ctx, now)?;
    let bytes = axum::body::to_bytes(request.into_body(), 64 * 1024)
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "validation_failed",
                "JSON 객체가 필요합니다.",
            )
        })?;
    let body: Value = serde_json::from_slice(&bytes)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "validation_failed",
                "JSON 객체가 필요합니다.",
            )
        })?;
    let parsed = service::parse_pair_body(&body).map_err(|fields| {
        let mut error = ApiError::new(
            StatusCode::BAD_REQUEST,
            "validation_failed",
            "입력값이 올바르지 않습니다.",
        );
        error.field_errors = Some(fields);
        error
    })?;
    let grant = principal::authenticate_pairing(&state, &parsed.token, &ctx, now).await?;
    if !grant.legacy_pairing {
        service::require_pairing_role(parsed.role, &grant.scopes)?;
    }
    let mut info = service::info(&state).await?;
    let mut entropy = [0u8; 32];
    getrandom::fill(&mut entropy).map_err(|_| ApiError::internal())?;
    let token = URL_SAFE_NO_PAD.encode(entropy);
    let id = uuid::Uuid::new_v4().to_string();
    let mut client = state
        .db
        .get()
        .await
        .map_err(|_| principal::db_unavailable())?;
    let tx = client
        .transaction()
        .await
        .map_err(|_| principal::db_unavailable())?;
    let live = tx.query_opt(
        "SELECT id FROM access_tokens WHERE id = $1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now()) FOR UPDATE",
        &[&grant.access_token_id],
    ).await.map_err(|_| principal::db_unavailable())?;
    if live.is_none() {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", "토큰이 유효하지 않거나 해지되었습니다."));
    }
    tx.execute("INSERT INTO gateway_tokens(id,name,token_hash,role,created_by_user_id,access_token_id) VALUES ($1,$2,$3,$4::text::role,$5,$6)",&[&id,&parsed.gateway_name,&crate::auth::token_digest(&token),&role_str(parsed.role),&grant.created_by_user_id,&grant.access_token_id]).await.map_err(|_|principal::db_unavailable())?;
    audit::write(
        &*tx,
        &AuditEntry {
            actor_label: Some(format!("token:{}", grant.access_token_id)),
            action: "node:pair".into(),
            target: Some(id.clone()),
            after: Some(json!({"gatewayName":parsed.gateway_name,"role":parsed.role})),
            ip: ctx.ip.clone(),
            ..Default::default()
        },
    )
    .await
    .map_err(|_| principal::db_unavailable())?;
    tx.commit().await.map_err(|_| principal::db_unavailable())?;
    info.as_object_mut().unwrap().remove("sha");
    info["gatewayTokenId"] = json!(id);
    info["token"] = json!(token);
    info["role"] = json!(parsed.role);
    Ok(Json(info))
}
async fn unpair(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let gateway = principal::require_gateway(&state, request.headers(), &ctx, &[]).await?;
    let mut client = state
        .db
        .get()
        .await
        .map_err(|_| principal::db_unavailable())?;
    let tx = client
        .transaction()
        .await
        .map_err(|_| principal::db_unavailable())?;
    tx.execute(
        "UPDATE gateway_tokens SET revoked_at=now() WHERE id=$1",
        &[&gateway.token_id],
    )
    .await
    .map_err(|_| principal::db_unavailable())?;
    audit::write(
        &*tx,
        &AuditEntry {
            actor_label: Some(format!("gateway:{}", gateway.name)),
            action: "node:unpair".into(),
            target: Some(gateway.token_id),
            ip: ctx.ip,
            ..Default::default()
        },
    )
    .await
    .map_err(|_| principal::db_unavailable())?;
    tx.commit().await.map_err(|_| principal::db_unavailable())?;
    Ok(Json(json!({"revoked":true})))
}
async fn form(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<FormResult>, ApiError> {
    check_browser_origin(request.headers(), &ctx)?;
    let session = principal::require_session(
        &state,
        request.headers(),
        &ctx,
        Some(Action::UserManage),
        TwoFactorGate::Enforce,
    )
    .await?;
    let command = request
        .uri()
        .path()
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_owned();
    let input = read_form(request).await?;
    if !input.args.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_arguments",
            "잘못된 요청입니다.",
        ));
    }
    let actor = session.actor(&ctx);
    let mut result = FormResult::default();
    match command.as_str() {
        "nodes.pairNode" => {
            // The browser bridge carries its prior attempt explicitly; no token is echoed.
            let attempt = input
                .get("_attempt")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0)
                .saturating_add(1);
            match service::parse_form(&input, true) {
                Err(errors) => {
                    result.result = Some(json!({"ok":false,"fieldErrors":errors,"attempt":attempt}))
                }
                Ok(parsed) => match service::pair_node(&state, parsed, &actor).await? {
                    Ok(id) => {
                        result.redirect_to = Some(format!("/nodes/{id}"));
                        result.refresh = Some(true);
                    }
                    Err(mut failure) => {
                        failure["attempt"] = json!(attempt);
                        result.result = Some(failure);
                    }
                },
            }
        }
        "nodes.updateNode" | "nodes.unpairNode" => {
            let id = input.get("id").unwrap_or("").trim();
            if id.is_empty() {
                result.result = Some(json!({"ok":false,"error":"잘못된 요청입니다."}));
                return Ok(Json(result));
            }
            if command == "nodes.updateNode" {
                match service::parse_form(&input, false) {
                    Err(errors) => result.result = Some(json!({"ok":false,"fieldErrors":errors})),
                    Ok(parsed) => {
                        service::update_node(&state, id, parsed, &actor).await?;
                        result.redirect_to = Some(format!("/nodes/{id}"));
                        result.refresh = Some(true);
                    }
                }
            } else {
                let failed = service::unpair_node(&state, id, &actor).await?;
                result.redirect_to = Some(
                    if failed {
                        "/nodes?unpairWarning=revoke_failed"
                    } else {
                        "/nodes"
                    }
                    .into(),
                );
                result.refresh = Some(true);
            }
        }
        "nodes.refreshStatuses" => {
            service::refresh_statuses(&state).await?;
            result.refresh = Some(true);
        }
        _ => return Err(ApiError::not_found()),
    }
    Ok(Json(result))
}
