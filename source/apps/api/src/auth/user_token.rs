//! User API tokens never become sessions and never fall back to cookies.
use super::{
    principal::{self, UserPrincipal},
    rbac::{Action, parse_role},
};
use crate::{error::ApiError, request_context::RequestContext, state::AppState};
use axum::http::{HeaderMap, StatusCode, header};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

// Select the user-token namespace before strictly validating the complete credential.
pub fn is_user_token_request(headers: &HeaderMap) -> bool {
    headers.get_all(header::AUTHORIZATION).iter().any(|value| {
        let bytes=value.as_bytes();
        bytes.get(..7).is_some_and(|scheme|scheme.eq_ignore_ascii_case(b"bearer ")) && bytes[7..].starts_with(b"nwu_u_")
    })
}

pub fn parse_header(headers: &HeaderMap) -> Result<&str, ApiError> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let (Some(value), None) = (values.next(), values.next()) else {
        return Err(ApiError::invalid_user_token());
    };
    let (scheme, token) = value
        .to_str()
        .ok()
        .and_then(|v| v.split_once(' '))
        .ok_or_else(ApiError::invalid_user_token)?;
    let encoded = token
        .strip_prefix("nwu_u_")
        .ok_or_else(ApiError::invalid_user_token)?;
    if !scheme.eq_ignore_ascii_case("bearer")
        || encoded.len() != 43
        || !URL_SAFE_NO_PAD.decode(encoded).is_ok_and(|v| v.len() == 32)
    {
        return Err(ApiError::invalid_user_token());
    }
    Ok(token)
}
pub fn check_fetch_metadata(headers: &HeaderMap) -> Result<(), ApiError> {
    if headers
        .get_all("sec-fetch-site")
        .iter()
        .any(|v| !matches!(v.to_str(), Ok("same-origin" | "none")))
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "invalid_origin",
            "요청 출처를 확인할 수 없습니다.",
        ));
    }
    Ok(())
}
pub async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
    ctx: &RequestContext,
) -> Result<UserPrincipal, ApiError> {
    if !is_user_token_request(headers) { return Err(ApiError::invalid_user_token()); }
    let now = principal::now_unix_ms();
    principal::check_pairing_ip_lock(state, ctx, now).map_err(|e| e.with_retry_after(900))?;
    let failed = || {
        if let Some(ip) = &ctx.ip {
            state.runtime.login_throttle.record_ip_failure(ip, now);
        }
        ApiError::invalid_user_token()
    };
    let token = parse_header(headers).map_err(|_| failed())?;
    let client = state
        .db
        .get()
        .await
        .map_err(|_| principal::db_unavailable())?;
    let row = client.query_opt(
        "SELECT t.id,t.name,t.scopes,t.revoked_at IS NOT NULL AS revoked, \
         t.expires_at IS NOT NULL AND t.expires_at <= now() AS expired, \
         (extract(epoch FROM t.expires_at)*1000)::bigint AS expires_ms, \
         u.id AS user_id,u.email,u.role::text AS role,u.enabled,u.approved,u.totp_enabled \
         FROM access_tokens t JOIN users u ON u.id=t.owner_user_id WHERE t.token_hash=$1 AND t.kind='user'",
        &[&super::token_digest(token)],
    ).await.map_err(|_|principal::db_unavailable())?;
    let row = row
        .filter(|r| {
            !r.get::<_, bool>("revoked")
                && !r.get::<_, bool>("expired")
                && r.get::<_, bool>("enabled")
                && r.get::<_, bool>("approved")
        })
        .ok_or_else(failed)?;
    if !row.get::<_, bool>("totp_enabled") {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "two_factor_required",
            "토큰 주인이 2단계 인증을 등록해야 토큰을 쓸 수 있습니다.",
        ));
    }
    let scopes: serde_json::Value = row.get("scopes");
    let scopes = scopes
        .as_array()
        .ok_or_else(failed)?
        .iter()
        .filter_map(|v| {
            Action::ALL
                .into_iter()
                .find(|a| v.as_str() == Some(a.as_str()))
        })
        .collect();
    Ok(UserPrincipal::token(
        row.get("user_id"),
        row.get("email"),
        parse_role(row.get("role")).ok_or_else(ApiError::internal)?,
        row.get("id"),
        row.get("name"),
        scopes,
        row.get("expires_ms"),
    ))
}
pub async fn record_use(state: &AppState, user: &UserPrincipal) {
    if let Some(id) = user.token_id()
        && let Ok(client) = state.db.get().await
    {
        let _=client.execute("UPDATE access_tokens SET last_used_at=now() WHERE id=$1 AND (last_used_at IS NULL OR last_used_at <= now()-interval '60 seconds')",&[&id]).await;
    }
}
