//! The signed-in user's own area (`apps/web/app/(app)/account/**`). Any role. The scope is always
//! the session's own user — never the form. Unlike TS (which used `auth()`, the cookie alone),
//! every entry point re-validates the session against the database.
//!
//! | Route | Gate | TS source |
//! | --- | --- | --- |
//! | `GET /api/ui/v1/account` | 2FA Enforce | `account/page.tsx` |
//! | `POST /api/ui/v1/forms/account.changePassword` | Enforce | `changeOwnPassword` → sign-out, `/login` |
//! | `GET /api/ui/v1/account/2fa` | AllowEnrollment | `account/2fa/page.tsx` (fresh pending secret) |
//! | `POST /api/ui/v1/forms/account.enableTwoFactor` | AllowEnrollment | `enableTwoFactor` → `{result:{backupCodes}}` |
//! | `POST /api/ui/v1/forms/account.finishTwoFactorSetup` | AllowEnrollment | `finishTwoFactorSetup` |
//! | `POST /api/ui/v1/forms/account.disableTwoFactor` | Enforce (a verified session) | clears 2FA and all own passkeys → `/account/2fa` |
//! | `POST /api/ui/v1/forms/account.regenerateBackupCodes` | Enforce + a proved factor | new: fresh backup codes → `{result:{backupCodes}}` |
//! | `POST /api/ui/v1/forms/account.verifyTwoFactor` | AllowEnrollment | new: this session's local second factor → `/` |
//! | `GET /api/ui/v1/account/tokens[/{id}]` | Enforce | `account/tokens/page.tsx`, `[id]/page.tsx` |
//! | `POST /api/ui/v1/forms/account.tokens.{issue,revoke}` | Enforce | `issueTokenAction`, `revokeTokenAction` |
//!
//! 2FA enrollment stays stateless as in TS (the page mints a secret, the form returns it), with two
//! hardenings: the returned secret must be at least 128 bits (RFC 4226 R6 — a self-chosen short
//! secret is refused), and the confirming code's time step is recorded (`totp_last_step`) so it
//! cannot sign in again (RFC 6238 §5.2). Enable is one conditional UPDATE tied to the admitted session;
//! disable atomically clears the secret, backup codes, step and all own passkeys. Where TS called `unstable_update`, the cookie
//! is re-issued with the fresh `totpEnabled` claim.
//!
//! Local proof (`mfaVerified` claim, see `SessionPrincipal::mfa_verified`): set only where a code
//! was accepted — a password sign-in, `enableTwoFactor`, `verifyTwoFactor`. `finishTwoFactorSetup`
//! mirrors the DB's `totpEnabled` but never grants the proof (enrollment finished on another
//! device proves nothing about this session); disabling needs a verified session (else an
//! unverified one could remove the factor and enroll its own).

use std::collections::BTreeMap;

use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde_json::{Value, json};

use super::audit::{self, AuditEntry};
use super::principal::{SessionPrincipal, TwoFactorGate, db_unavailable, now_unix_ms, require_session};
use super::tokens::{self, TokenKind, TokenScope};
use super::{password, session, totp};
use crate::contracts::{FormInput, FormResult};
use crate::crypto::js_trim;
use crate::error::ApiError;
use crate::http_policy::read_form;
use crate::request_context::RequestContext;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/ui/v1/account", get(account_view))
        .route("/api/ui/v1/forms/account.changePassword", post(change_password))
        .route("/api/ui/v1/account/2fa", get(two_factor_view))
        .route("/api/ui/v1/forms/account.enableTwoFactor", post(enable_two_factor))
        .route("/api/ui/v1/forms/account.finishTwoFactorSetup", post(finish_two_factor_setup))
        .route("/api/ui/v1/forms/account.disableTwoFactor", post(disable_two_factor))
        .route("/api/ui/v1/forms/admin.users.resetTwoFactor", post(reset_two_factor))
        .route("/api/ui/v1/forms/account.regenerateBackupCodes", post(regenerate_backup_codes))
        .route("/api/ui/v1/forms/account.verifyTwoFactor", post(verify_two_factor))
        .route("/api/ui/v1/account/tokens", get(list_tokens))
        .route("/api/ui/v1/account/tokens/{id}", get(token_detail))
        .route("/api/ui/v1/forms/account.tokens.issue", post(issue_token))
        .route("/api/ui/v1/forms/account.tokens.revoke", post(revoke_token))
}

// ─────────────────────────────── helpers ───────────────────────────────

fn refused(message: &str) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, "conflict", message)
}

fn invalid(fields: BTreeMap<String, String>) -> ApiError {
    let mut error = ApiError::new(StatusCode::BAD_REQUEST, "validation_failed", "입력값이 올바르지 않습니다.");
    error.field_errors = Some(fields);
    error
}

fn one_field(name: &str, message: &str) -> ApiError {
    invalid(BTreeMap::from([(name.to_owned(), message.to_owned())]))
}

fn iso(at: chrono::DateTime<chrono::Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn with_cookies(mut response: Response, cookies: Vec<String>) -> Result<Response, ApiError> {
    for cookie in cookies {
        let value = HeaderValue::from_str(&cookie).map_err(|_| ApiError::internal())?;
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    Ok(response)
}

fn redirect(to: &str) -> Response {
    Json(FormResult { redirect_to: Some(to.to_owned()), ..FormResult::default() }).into_response()
}

async fn gate(state: &AppState, headers: &HeaderMap, ctx: &RequestContext, gate: TwoFactorGate) -> Result<SessionPrincipal, ApiError> {
    require_session(state, headers, ctx, None, gate).await
}

async fn authorized_form(state: &AppState, ctx: &RequestContext, request: Request, two_factor: TwoFactorGate) -> Result<(SessionPrincipal, HeaderMap, FormInput), ApiError> {
    let headers = request.headers().clone();
    let principal = gate(state, &headers, ctx, two_factor).await?;
    Ok((principal, headers, read_form(request).await?))
}

async fn write_audit(state: &AppState, principal: &SessionPrincipal, ctx: &RequestContext, action: &str) -> Result<(), ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let entry = AuditEntry { actor_id: Some(principal.user_id.clone()), action: action.to_owned(), ip: ctx.ip.clone(), ..AuditEntry::default() };
    audit::write(&**client, &entry).await.map_err(|_| db_unavailable())
}

/// Re-issues this request's session cookie with `changes` applied (Auth.js `unstable_update`: the
/// other claims are kept, iat/exp/jti renewed for a full session).
pub(super) fn reissue(state: &AppState, headers: &HeaderMap, ctx: &RequestContext, changes: &[(&str, Value)]) -> Result<Vec<String>, ApiError> {
    let now = now_unix_ms() / 1000;
    let Some(mut claims) = super::principal::session_claims(state, headers, ctx, now) else { return Ok(Vec::new()) };
    for (key, value) in changes {
        claims.insert((*key).into(), value.clone());
    }
    let name = session::cookie_name(ctx.secure);
    let secret = state.config.auth_secrets.first().ok_or_else(ApiError::internal)?;
    let token = session::encode(&claims, secret, name, now, session::SESSION_MAX_AGE_SECS).map_err(|_| ApiError::internal())?;
    Ok(session::issue_cookies(name, &token, &session::request_cookies(headers), ctx.secure, now + session::SESSION_MAX_AGE_SECS))
}

/// The stored hash, or the TS refusal for an OAuth-only account (a password there would open a
/// second door the provider does not control).
async fn password_hash_of(state: &AppState, user_id: &str) -> Result<String, ApiError> {
    let row = state
        .db
        .get()
        .await
        .map_err(|_| db_unavailable())?
        .query_opt("SELECT password_hash FROM users WHERE id = $1", &[&user_id])
        .await
        .map_err(|_| db_unavailable())?
        .ok_or_else(|| refused("사용자를 찾을 수 없습니다."))?;
    row.get::<_, Option<String>>(0).ok_or_else(|| refused("이 계정은 비밀번호를 사용하지 않습니다."))
}

// ─────────────────────────────── account ───────────────────────────────

async fn account_view(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let principal = gate(&state, &headers, &ctx, TwoFactorGate::Enforce).await?;
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let me = client
        .query_one("SELECT email, password_hash IS NOT NULL AS has_password, totp_enabled FROM users WHERE id = $1", &[&principal.user_id])
        .await
        .map_err(|_| db_unavailable())?;
    // One line per enabled provider; the link state is this account's.
    let logins = client
        .query(
            "SELECT p.id, p.name, (SELECT min(a.linked_at) FROM oauth_accounts a WHERE a.provider_id = p.id AND a.user_id = $1) AS linked_at \
             FROM oauth_providers p WHERE p.enabled ORDER BY p.sort_order, p.name, p.id",
            &[&principal.user_id],
        )
        .await
        .map_err(|_| db_unavailable())?;
    let logins: Vec<Value> = logins
        .iter()
        .map(|r| json!({"providerId": r.get::<_, String>("id"), "name": r.get::<_, String>("name"), "linkedAt": r.get::<_, Option<chrono::DateTime<chrono::Utc>>>("linked_at").map(iso)}))
        .collect();
    Ok(Json(json!({
        "email": me.get::<_, Option<String>>("email"),
        "hasPassword": me.get::<_, bool>("has_password"),
        "totpEnabled": me.get::<_, bool>("totp_enabled"),
        "logins": logins,
    })))
}

async fn change_password(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Response, ApiError> {
    let (principal, headers, form) = authorized_form(&state, &ctx, request, TwoFactorGate::Enforce).await?;
    let current = form.get("currentPassword").unwrap_or("");
    let next = form.get("newPassword").unwrap_or("");
    let mut errors = BTreeMap::new();
    if current.is_empty() {
        errors.insert("currentPassword".to_owned(), "현재 비밀번호를 입력하세요.".to_owned());
    }
    if let Some(message) = password::validate_password(next) {
        errors.insert("newPassword".to_owned(), message);
    }
    if next != form.get("confirmPassword").unwrap_or("") {
        errors.insert("confirmPassword".to_owned(), "새 비밀번호가 일치하지 않습니다.".to_owned());
    }
    if !errors.is_empty() {
        return Err(invalid(errors));
    }
    // Re-authentication: a hijacked idle session cannot rotate the credentials.
    check_current_password(&state, &principal.user_id, current).await?;
    let hash = state.passwords.hash(next.to_owned()).await.map_err(|_| ApiError::internal())?;
    let changed = state
        .db
        .get()
        .await
        .map_err(|_| db_unavailable())?
        .execute(
            "UPDATE users SET password_hash = $2, session_version = session_version + 1 WHERE id = $1 AND password_hash IS NOT NULL",
            &[&principal.user_id, &hash],
        )
        .await
        .map_err(|_| db_unavailable())?;
    if changed == 0 {
        return Err(refused("사용자를 찾을 수 없습니다."));
    }
    write_audit(&state, &principal, &ctx, "user:change-own-password").await?;
    // Every session ended (sessionVersion); sign this device out too.
    let name = session::cookie_name(ctx.secure);
    with_cookies(redirect("/login"), session::clear_cookies(name, &session::request_cookies(&headers), ctx.secure))
}

// ─────────────────────────────── 2FA ───────────────────────────────

async fn two_factor_view(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let principal = gate(&state, &headers, &ctx, TwoFactorGate::AllowEnrollment).await?;
    if principal.totp_enabled {
        return Ok(Json(json!({"enabled": true, "pending": null, "verificationRequired": principal.must_verify_2fa()})));
    }
    // A fresh pending secret per view; persisted only once a code proves possession.
    let secret = totp::generate_secret();
    let uri = totp::otpauth_uri(&secret, principal.email.as_deref().unwrap_or("user"));
    Ok(Json(json!({"enabled": false, "pending": {"secret": secret, "otpauthUri": uri}, "verificationRequired": false})))
}

async fn enable_two_factor(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Response, ApiError> {
    let (principal, headers, form) = authorized_form(&state, &ctx, request, TwoFactorGate::AllowEnrollment).await?;
    let secret = js_trim(form.get("secret").unwrap_or("")).to_owned();
    let code = js_trim(form.get("code").unwrap_or(""));
    if secret.is_empty() {
        return Err(refused("설정 세션이 만료되었습니다. 페이지를 새로고침해 다시 시도하세요."));
    }
    // `verify` also refuses secrets under 128 bits.
    let matched = totp::verify(&secret, code, now_unix_ms().max(0) as u64)
        .ok_or_else(|| refused("코드가 올바르지 않습니다. 인증 앱의 6자리 코드를 다시 입력하세요."))?;
    let already = || refused("이미 2FA가 설정되어 있습니다.");
    if principal.totp_enabled {
        return Err(already());
    }
    let codes = totp::generate_backup_codes(totp::BACKUP_CODE_COUNT);
    let to_hash = codes.clone();
    let hashes = state
        .passwords
        .run(move || totp::hash_backup_codes(&to_hash))
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(|_| ApiError::internal())?;
    // Sealed at rest: reading the database alone must not defeat 2FA.
    let sealed = state.secrets.seal(&secret).map_err(|_| ApiError::internal())?;
    let enabled = state
        .db
        .get()
        .await
        .map_err(|_| db_unavailable())?
        .query_opt(
            "UPDATE users SET totp_secret = $2, totp_enabled = true, backup_codes = $3, totp_last_step = $4, session_version = session_version + 1 WHERE id = $1 AND NOT totp_enabled AND session_version = $5 AND enabled AND approved RETURNING session_version",
            &[&principal.user_id, &sealed, &json!(hashes), &(matched.step as i64), &principal.session_version],
        )
        .await
        .map_err(|_| db_unavailable())?;
    if enabled.is_none() {
        return Err(refused("인증 정보가 바뀌었거나 요청이 만료되었습니다. 다시 시작하세요."));
    }
    let version: i32 = enabled.unwrap().get(0);
    write_audit(&state, &principal, &ctx, "user:2fa-enable").await?;
    // The plaintext codes: this response only, never again. The accepted code is this session's
    // proof of the new factor.
    let body = Json(FormResult { result: Some(json!({"ok": true, "backupCodes": codes})), ..FormResult::default() }).into_response();
    with_cookies(body, reissue(&state, &headers, &ctx, &[("totpEnabled", true.into()), ("mfaVerified", true.into()), ("sv", version.into())])?)
}

/// After the user acknowledged the backup codes. Mirrors the DB's `totpEnabled`, never the
/// client's word — and never grants `mfaVerified` (the code was accepted, if at all, elsewhere).
async fn finish_two_factor_setup(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Response, ApiError> {
    let (principal, headers, _) = authorized_form(&state, &ctx, request, TwoFactorGate::AllowEnrollment).await?;
    if !principal.totp_enabled {
        return Ok(redirect("/account/2fa"));
    }
    with_cookies(redirect("/account"), reissue(&state, &headers, &ctx, &[("totpEnabled", true.into())])?)
}

/// Re-authentication with the current password, counted like a sign-in (10 per 15 minutes per
/// account, on admission) so a stolen session cannot guess the password without limit.
async fn check_current_password(state: &AppState, user_id: &str, current: &str) -> Result<(), ApiError> {
    let stored = password_hash_of(state, user_id).await?;
    check_password_against(state, user_id, current, stored).await
}

/// [`check_current_password`] against a hash the caller already read (and will hold the row to).
pub(super) async fn check_password_against(state: &AppState, user_id: &str, current: &str, stored: String) -> Result<(), ApiError> {
    let throttle = &state.runtime.login_throttle;
    let key = format!("reauth:user:{user_id}");
    let attempt = throttle.begin_account(&key, now_unix_ms()).ok_or_else(|| {
        ApiError::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "실패가 잦아 잠시 차단되었습니다. 15분 뒤에 다시 시도하세요.")
    })?;
    if !state.passwords.verify(current.to_owned(), stored).await {
        attempt.keep();
        return Err(one_field("currentPassword", "현재 비밀번호가 올바르지 않습니다."));
    }
    drop(attempt);
    throttle.clear_account(&key);
    Ok(())
}

/// A verified session only: an unverified one (an OAuth sign-in that has not passed the code)
/// could otherwise remove the factor and enroll its own.
async fn disable_two_factor(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Response, ApiError> {
    let (principal, headers, form) = authorized_form(&state, &ctx, request, TwoFactorGate::Enforce).await?;
    if !principal.totp_enabled {
        return Err(refused(NOT_ENROLLED));
    }
    if !principal.mfa_verified {
        return Err(super::principal::two_factor_verification_required());
    }
    let current = form.get("currentPassword").unwrap_or("");
    if current.is_empty() {
        return Err(one_field("currentPassword", "현재 비밀번호를 입력하세요."));
    }
    let hash = password_hash_of(&state, &principal.user_id).await?;
    check_password_against(&state, &principal.user_id, current, hash.clone()).await?;
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(|_| db_unavailable())?;
    let row = super::passkey_routes::lock_user(&tx, &principal.user_id).await?;
    super::passkey_routes::same_account(&row, principal.session_version, &hash, principal.mfa_verified)?;
    if !row.get::<_, bool>("totp_enabled") {
        return Err(refused(NOT_ENROLLED));
    }
    super::passkey_routes::remove_all_passkeys(&tx, &principal.user_id, &principal.user_id, &ctx, "2fa-disable").await?;
    let version = clear_two_factor(&tx, &principal.user_id).await?;
    audit::write(&*tx, &AuditEntry {
        actor_id: Some(principal.user_id.clone()),
        action: "user:2fa-disable".into(),
        ip: ctx.ip.clone(),
        ..Default::default()
    }).await.map_err(|_| db_unavailable())?;
    // If the policy requires 2FA, the gate now confines them to re-enrollment.
    let cookies = reissue(&state, &headers, &ctx, &[("totpEnabled", false.into()), ("mfaVerified", false.into()), ("sv", version.into())])?;
    if cookies.is_empty() {
        return Err(ApiError::unauthorized());
    }
    let response = with_cookies(redirect("/account/2fa"), cookies)?;
    tx.commit().await.map_err(|_| db_unavailable())?;
    Ok(response)
}

async fn clear_two_factor(tx: &tokio_postgres::Transaction<'_>, user_id: &str) -> Result<i32, ApiError> {
    let changed = tx
        .query_one(
            "UPDATE users SET totp_secret = NULL, totp_enabled = false, backup_codes = NULL, totp_last_step = NULL, session_version = session_version + 1 WHERE id = $1 RETURNING session_version",
            &[&user_id],
        )
        .await
        .map_err(|_| db_unavailable())?;
    Ok(changed.get(0))
}

async fn reset_two_factor(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Response, ApiError> {
    let principal = require_session(&state, request.headers(), &ctx, Some(super::rbac::Action::UserManage), TwoFactorGate::Enforce).await?;
    super::passkey_routes::ensure_verified(&principal)?;
    let form = read_form(request).await?;
    let target = form.get("id").filter(|id| !id.is_empty()).ok_or_else(ApiError::not_found)?;
    if target == principal.user_id {
        return Err(refused("본인 계정의 2FA는 계정 화면에서 관리하세요."));
    }
    let enrolled = state.db.get().await.map_err(|_| db_unavailable())?
        .query_opt("SELECT totp_enabled FROM users WHERE id = $1", &[&target]).await.map_err(|_| db_unavailable())?
        .ok_or_else(ApiError::not_found)?.get::<_, bool>(0);
    if !enrolled {
        return Err(refused(NOT_ENROLLED));
    }
    let current = form.get("currentPassword").unwrap_or("");
    if current.is_empty() {
        return Err(one_field("currentPassword", "현재 비밀번호를 입력하세요."));
    }
    let hash = password_hash_of(&state, &principal.user_id).await?;
    check_password_against(&state, &principal.user_id, current, hash.clone()).await?;
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(|_| db_unavailable())?;
    // Every user lock precedes credential locks, in the same order for reciprocal resets.
    let mut ids = [principal.user_id.as_str(), target];
    ids.sort_unstable();
    for id in ids {
        let row = super::passkey_routes::lock_user(&tx, id).await.map_err(|error| {
            if id == target && error.status == StatusCode::UNAUTHORIZED { ApiError::not_found() } else { error }
        })?;
        if id == principal.user_id {
            super::passkey_routes::same_account(&row, principal.session_version, &hash, principal.mfa_verified)?;
        } else if !row.get::<_, bool>("totp_enabled") {
            return Err(refused(NOT_ENROLLED));
        }
    }
    super::passkey_routes::remove_all_passkeys(&tx, &principal.user_id, target, &ctx, "2fa-reset").await?;
    clear_two_factor(&tx, target).await?;
    audit::write(&*tx, &AuditEntry {
        actor_id: Some(principal.user_id.clone()), action: "user:2fa-reset".into(),
        target: Some(target.into()), before: Some(json!({"totpEnabled": true})),
        after: Some(json!({"totpEnabled": false})), ip: ctx.ip.clone(), ..Default::default()
    }).await.map_err(|_| db_unavailable())?;
    tx.commit().await.map_err(|_| db_unavailable())?;
    Ok(redirect(&format!("/admin/users/{target}?twoFactorReset=1")))
}

const NOT_ENROLLED: &str = "2단계 인증이 설정되어 있지 않습니다.";
const REGENERATE_CODE_MISSING: &str = "인증 앱의 6자리 코드를 입력하세요.";
const REGENERATE_CODE_WRONG: &str = "코드가 올바르지 않습니다. 인증 앱의 6자리 코드를 다시 입력하세요.";
const REGENERATE_CHANGED: &str = "인증 정보가 방금 바뀌었습니다. 다시 시도하세요.";

/// New backup codes for an enrolled account, replacing every old one; the plaintext is in this
/// response only.
///
/// A session that proved the factor (`mfaVerified`), not just one `Enforce` lets through: a session
/// from before 2FA was enabled elsewhere, under a policy that does not require it, passes the gate
/// without ever having proved it. Then re-authentication, chosen by the stored row and never by the
/// form: the current password, or for an OAuth-only account (no password) a current authenticator
/// code, used once like a sign-in code. The values the proof was checked against are read first; the
/// codes are replaced under the row lock only if the session and account are still eligible and
/// those values (password hash, TOTP secret, backup codes) are unchanged. So codes that were never
/// stored are never returned, and no proof outlives the factor it proved. The audit row is part of
/// the same transaction.
async fn regenerate_backup_codes(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Response, ApiError> {
    let (principal, _, form) = authorized_form(&state, &ctx, request, TwoFactorGate::Enforce).await?;
    if !principal.totp_enabled {
        return Err(refused(NOT_ENROLLED));
    }
    if !principal.mfa_verified {
        return Err(super::principal::two_factor_verification_required());
    }
    let proved: [Option<Value>; 3] = {
        let client = state.db.get().await.map_err(|_| db_unavailable())?;
        let row = client
            .query_opt("SELECT to_jsonb(password_hash), to_jsonb(totp_secret), backup_codes FROM users WHERE id = $1", &[&principal.user_id])
            .await
            .map_err(|_| db_unavailable())?
            .ok_or_else(ApiError::unauthorized)?;
        [row.get(0), row.get(1), row.get(2)]
    };
    match proved[0].as_ref().and_then(Value::as_str) {
        Some(hash) => {
            let current = form.get("currentPassword").unwrap_or("");
            if current.is_empty() {
                return Err(one_field("currentPassword", "현재 비밀번호를 입력하세요."));
            }
            check_password_against(&state, &principal.user_id, current, hash.to_owned()).await?;
        }
        None => {
            let secret = proved[1].as_ref().and_then(Value::as_str).map(str::to_owned);
            prove_authenticator_code(&state, &ctx, &principal.user_id, secret, js_trim(form.get("code").unwrap_or(""))).await?;
        }
    }

    let codes = totp::generate_backup_codes(totp::BACKUP_CODE_COUNT);
    let to_hash = codes.clone();
    let hashes = state
        .passwords
        .run(move || totp::hash_backup_codes(&to_hash))
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(|_| ApiError::internal())?;

    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(|_| db_unavailable())?;
    let row = tx
        .query_opt(
            "SELECT session_version, enabled, approved, totp_enabled, to_jsonb(password_hash), to_jsonb(totp_secret), backup_codes \
             FROM users WHERE id = $1 FOR UPDATE",
            &[&principal.user_id],
        )
        .await
        .map_err(|_| db_unavailable())?
        .ok_or_else(ApiError::unauthorized)?;
    if row.get::<_, i32>(0) != principal.session_version || !row.get::<_, bool>(1) || !row.get::<_, bool>(2) {
        return Err(ApiError::unauthorized());
    }
    if !row.get::<_, bool>(3) {
        return Err(refused(NOT_ENROLLED));
    }
    let now: [Option<Value>; 3] = [row.get(4), row.get(5), row.get(6)];
    if now != proved {
        return Err(refused(REGENERATE_CHANGED));
    }
    tx.execute("UPDATE users SET backup_codes = $2 WHERE id = $1", &[&principal.user_id, &json!(hashes)])
        .await
        .map_err(|_| db_unavailable())?;
    let entry = AuditEntry { actor_id: Some(principal.user_id.clone()), action: "user:2fa-backup-regenerate".into(), ip: ctx.ip.clone(), ..AuditEntry::default() };
    audit::write(&*tx, &entry).await.map_err(|_| db_unavailable())?;
    tx.commit().await.map_err(|_| db_unavailable())?;
    Ok(Json(FormResult { result: Some(json!({"ok": true, "backupCodes": codes})), ..FormResult::default() }).into_response())
}

/// An OAuth-only account's re-authentication: a current authenticator code (never a backup code),
/// checked against `secret` and used once like a sign-in code, throttled on the same windows as
/// `account.verifyTwoFactor`.
async fn prove_authenticator_code(state: &AppState, ctx: &RequestContext, user_id: &str, secret: Option<String>, code: &str) -> Result<(), ApiError> {
    if code.is_empty() {
        return Err(one_field("code", REGENERATE_CODE_MISSING));
    }
    let now_ms = now_unix_ms();
    let throttle = &state.runtime.login_throttle;
    let limited = || ApiError::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "실패가 잦아 잠시 차단되었습니다. 15분 뒤에 다시 시도하세요.");
    let account_attempt = throttle.begin_account(&format!("user:{user_id}"), now_ms).ok_or_else(limited)?;
    let ip_attempt = match ctx.ip.as_deref() {
        Some(ip) => Some(throttle.begin_ip(ip, now_ms).ok_or_else(limited)?),
        None => None,
    };
    let factor = super::login::StoredFactor { totp_secret: secret, backup_codes: None };
    if !super::login::accept_second_factor(state, user_id, factor, super::login::FactorKind::Authenticator, code, now_ms).await? {
        account_attempt.keep();
        if let Some(attempt) = ip_attempt {
            attempt.keep();
        }
        return Err(one_field("code", REGENERATE_CODE_WRONG));
    }
    Ok(())
}

pub const VERIFY_CODE_WRONG: &str = "코드가 올바르지 않습니다. 인증 앱의 6자리 코드나 백업 코드를 입력하세요.";

/// This session's local second factor: the same one-use check as a password sign-in (TOTP step
/// recorded, backup code removed atomically), throttled on the same windows — the account's (by
/// user id: an OAuth-only account may have no email) and the address's. Success re-issues the
/// cookie with `mfaVerified`.
async fn verify_two_factor(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Response, ApiError> {
    let (principal, headers, form) = authorized_form(&state, &ctx, request, TwoFactorGate::AllowEnrollment).await?;
    if !principal.totp_enabled {
        return Err(refused(NOT_ENROLLED));
    }
    if principal.mfa_verified {
        return Ok(redirect("/"));
    }
    let now_ms = now_unix_ms();
    let throttle = &state.runtime.login_throttle;
    let account = format!("user:{}", principal.user_id);
    let limited = || ApiError::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "실패가 잦아 잠시 차단되었습니다. 15분 뒤에 다시 시도하세요.");
    // Counted on admission (a wrong code keeps both attempts, any other exit releases them), so a
    // concurrent burst cannot pass the check before the first failure is recorded.
    let account_attempt = throttle.begin_account(&account, now_ms).ok_or_else(limited)?;
    let ip_attempt = match ctx.ip.as_deref() {
        Some(ip) => Some(throttle.begin_ip(ip, now_ms).ok_or_else(limited)?),
        None => None,
    };
    let row = state
        .db
        .get()
        .await
        .map_err(|_| db_unavailable())?
        .query_one("SELECT totp_secret, backup_codes FROM users WHERE id = $1", &[&principal.user_id])
        .await
        .map_err(|_| db_unavailable())?;
    let factor = super::login::StoredFactor { totp_secret: row.get("totp_secret"), backup_codes: row.get("backup_codes") };
    let code = js_trim(form.get("code").unwrap_or(""));
    if !super::login::accept_second_factor(&state, &principal.user_id, factor, super::login::FactorKind::Either, code, now_ms).await? {
        account_attempt.keep();
        if let Some(attempt) = ip_attempt {
            attempt.keep();
        }
        return Err(one_field("code", VERIFY_CODE_WRONG));
    }
    drop(ip_attempt);
    drop(account_attempt);
    throttle.clear_account(&account);
    write_audit(&state, &principal, &ctx, "user:2fa-verify").await?;
    with_cookies(redirect("/"), reissue(&state, &headers, &ctx, &[("mfaVerified", true.into())])?)
}

// ─────────────────────────────── tokens ───────────────────────────────

async fn me(state: &AppState, headers: &HeaderMap, ctx: &RequestContext) -> Result<(SessionPrincipal, TokenScope), ApiError> {
    let principal = gate(state, headers, ctx, TwoFactorGate::Enforce).await?;
    let scope = TokenScope::User { owner_user_id: principal.user_id.clone() };
    Ok((principal, scope))
}

async fn list_tokens(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let (principal, scope) = me(&state, &headers, &ctx).await?;
    tokens::list_view(&state, &principal, &scope).await.map(Json)
}

async fn token_detail(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let (principal, scope) = me(&state, &headers, &ctx).await?;
    tokens::detail_view(&state, &principal, &scope, &id).await.map(Json)
}

async fn issue_token(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (principal, _) = me(&state, request.headers(), &ctx).await?;
    let form = read_form(request).await?;
    tokens::issue_from_form(&state, &principal, &ctx, TokenKind::User, &form).await.map(Json)
}

async fn revoke_token(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (principal, scope) = me(&state, request.headers(), &ctx).await?;
    let form = read_form(request).await?;
    tokens::revoke_from_form(&state, &principal, &ctx, &scope, &form).await.map(Json)
}
