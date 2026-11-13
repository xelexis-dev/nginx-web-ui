//! Browser-facing auth endpoints. The root boundary (`app::with_request_context`) resolves the
//! [`RequestContext`] and enforces Origin on unsafe `/api/ui/v1` methods before these run.
//!
//! | Route | Behaviour (TS source) |
//! | --- | --- |
//! | `GET /api/ui/v1/session` | `SessionView`, DB-validated; 401 Session challenge; 503 on DB outage |
//! | `POST /api/ui/v1/session/refresh` | same view; re-issues the cookie when < 4 h remain (sliding) |
//! | `GET /api/ui/v1/auth/options` | `{firstRun, authenticated, providers:[{id,name}]}` for login/setup |
//! | `POST /api/ui/v1/forms/setup.createFirstAdmin` | `app/setup/actions.ts` `createFirstAdmin` |
//! | `POST /api/ui/v1/forms/login.loginAction` | `app/login/actions.ts` `loginAction`; an account with its own TOTP gets a pending sign-in (`/login/2fa`), not a session |
//! | `POST /api/ui/v1/forms/login.verifyTwoFactor` | new: completes a pending sign-in with a TOTP or backup code |
//! | `POST /api/ui/v1/forms/login.cancelTwoFactor` | new: drops this browser's pending sign-in |
//! | `POST /api/ui/v1/forms/session.signOut` | `lib/session-actions.ts` → Auth.js signout |
//!
//! Set-Cookie is only sent on browser-called POST responses; `GET /session` (read by SSR) never
//! sets cookies. Renewal is the browser session client's `POST /session/refresh` on
//! navigation/activity — the replacement for Auth.js re-issuing the cookie on every request that
//! passed `apps/web/proxy.ts` (next-auth `lib/index.js` `handleAuth` appends the session
//! action's Set-Cookie to every middleware response).

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde_json::{Value, json};

use super::auth_log::{self, FailureReason};
use super::audit::{self, AuditEntry};
use super::login::{self, Credentials, FactorKind, LoginOutcome, StoredFactor};
use super::pending_login::{self, Pending};
use super::principal::{self, db_unavailable, now_unix_ms};
use super::rbac::role_str;
use super::session;
use super::setup::{self, SetupInput, SetupOutcome};
use crate::contracts::{FormInput, FormResult};
use crate::error::ApiError;
use crate::http_policy::read_form;
use crate::request_context::RequestContext;
use crate::state::AppState;

/// Every failed sign-in answers alike — wrong password, throttled account or throttled address.
pub const LOGIN_FAILURE_REDIRECT: &str = "/login?error=CredentialsSignin";
/// The factor screen of a pending sign-in.
pub const TWO_FACTOR_PAGE: &str = "/login/2fa";
pub const TWO_FACTOR_INVALID_REDIRECT: &str = "/login/2fa?error=invalid";
/// A wrong code sent from the backup-code screen goes back there.
pub const TWO_FACTOR_BACKUP_INVALID_REDIRECT: &str = "/login/2fa/backup?error=invalid";
/// No pending sign-in can complete here (none, expired, spent, cancelled, or the account changed).
pub const TWO_FACTOR_EXPIRED_REDIRECT: &str = "/login?error=TwoFactorExpired";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/ui/v1/session", get(read_session))
        .route("/api/ui/v1/session/refresh", post(refresh_session))
        .route("/api/ui/v1/auth/options", get(auth_options))
        .route("/api/ui/v1/forms/setup.createFirstAdmin", post(create_first_admin))
        .route("/api/ui/v1/forms/login.loginAction", post(login_action))
        .route("/api/ui/v1/forms/login.verifyTwoFactor", post(verify_two_factor))
        .route("/api/ui/v1/forms/login.cancelTwoFactor", post(cancel_two_factor))
        .route("/api/ui/v1/forms/session.signOut", post(sign_out))
        .merge(super::account::router())
        .merge(super::passkey_routes::router())
        .merge(super::oidc::router(super::oidc::default_transport()))
}

fn iso(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn form_response(redirect_to: &str, cookies: Vec<String>) -> Result<Response, ApiError> {
    let body = FormResult { redirect_to: Some(redirect_to.to_owned()), ..FormResult::default() };
    with_cookies(Json(body).into_response(), cookies)
}

/// `SessionView` (`packages/contracts/openapi.json`). Allowed while 2FA enrollment is pending —
/// SSR needs `requires2fa`/`totpEnabled` to send the user to enrollment.
async fn read_session(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let principal = principal::authenticate_session(&state, &headers, &ctx, now_unix_ms() / 1000)
        .await?
        .ok_or_else(ApiError::session_required)?;
    Ok(Json(session_view(&principal, principal.expires_at)))
}

fn session_view(principal: &principal::SessionPrincipal, expires_at: i64) -> Value {
    json!({
        "user": {
            "id": principal.user_id,
            "email": principal.email,
            "name": principal.name,
            "image": null,
            "role": role_str(principal.role),
            "sv": principal.session_version,
            "totpEnabled": principal.totp_enabled,
            "requires2fa": principal.requires_2fa,
            // Enrolled and required, but this session has not passed the code: `/account/2fa`.
            "verificationRequired": principal.must_verify_2fa(),
        },
        "expires": iso(expires_at),
    })
}

fn with_cookies(mut response: Response, cookies: Vec<String>) -> Result<Response, ApiError> {
    for cookie in cookies {
        let value = HeaderValue::from_str(&cookie).map_err(|_| ApiError::internal())?;
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    Ok(response)
}

/// Sliding renewal, called by the browser session client on navigation/activity (never on an
/// idle timer). DB-validated like `GET /session`; re-issues the cookie for a full
/// [`session::SESSION_MAX_AGE_SECS`] when less than [`session::RENEW_BELOW_SECS`] remain.
/// A legacy cookie is also renewed once to establish its post-upgrade authentication time.
async fn refresh_session(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Response, ApiError> {
    let now = now_unix_ms() / 1000;
    let present = session::request_cookies(&headers);
    let cookie_name = session::cookie_name(ctx.secure);
    let Some(current) = principal::authenticate_session(&state, &headers, &ctx, now).await? else {
        // A browser response: expire the dead cookie instead of re-sending it on every request.
        let response = ApiError::session_required().into_response();
        return with_cookies(response, session::clear_cookies(cookie_name, &present, ctx.secure));
    };
    let mut claims = principal::session_claims(&state, &headers, &ctx, now).ok_or_else(ApiError::session_required)?;
    let first_refresh = !claims.contains_key("authTime");
    if first_refresh { claims.insert("authTime".into(), now.into()); }
    let auth_time = session::authentication_time(&claims);
    if !auth_time.is_some_and(|time| time <= now + session::CLOCK_TOLERANCE_SECS && now.saturating_sub(time) < session::AUTH_MAX_AGE_SECS) {
        return with_cookies(ApiError::session_required().into_response(), session::clear_cookies(cookie_name, &present, ctx.secure));
    }
    if !first_refresh && current.expires_at - now >= session::RENEW_BELOW_SECS {
        return Ok(Json(session_view(&current, current.expires_at)).into_response());
    }
    // Auth.js keeps the token's claims and only rewrites iat/exp/jti; re-encoding also moves a
    // cookie from a rotated-out secret to the writing secret (`auth_secrets[0]`).
    let secret = state.config.auth_secrets.first().ok_or_else(ApiError::internal)?;
    let remaining = session::AUTH_MAX_AGE_SECS - now.saturating_sub(auth_time.unwrap());
    let max_age = session::SESSION_MAX_AGE_SECS.min(remaining);
    let token = session::encode(&claims, secret, cookie_name, now, max_age).map_err(|_| ApiError::internal())?;
    let expires_at = now + max_age;
    let cookies = session::issue_cookies(cookie_name, &token, &present, ctx.secure, expires_at);
    with_cookies(Json(session_view(&current, expires_at)).into_response(), cookies)
}

/// What the login and setup screens need before anyone is signed in. A provider whose client
/// secret cannot be opened is left out, as the TS sign-in does (usually a lost data key).
async fn auth_options(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let first_run = setup::is_first_run(&state).await?;
    let authenticated = principal::authenticate_session(&state, &headers, &ctx, now_unix_ms() / 1000).await?.is_some();
    // The connection goes back to the pool before the pending check borrows one: holding it would
    // make this request wait for itself on a one-connection pool, and starve larger ones.
    let rows = {
        let client = state.db.get().await.map_err(|_| db_unavailable())?;
        client
            .query("SELECT id, name, client_secret_enc FROM oauth_providers WHERE enabled ORDER BY sort_order, name", &[])
            .await
            .map_err(|_| db_unavailable())?
    };
    let providers: Vec<Value> = rows
        .iter()
        .filter(|row| state.secrets.open(Some(row.get::<_, &str>("client_secret_enc"))).is_some())
        .map(|row| json!({"id": row.get::<_, String>("id"), "name": row.get::<_, String>("name")}))
        .collect();
    let two_factor_pending = pending_login::is_pending(&state, &headers, &ctx, now_unix_ms()).await?;
    Ok(Json(json!({"firstRun": first_run, "authenticated": authenticated, "providers": providers, "twoFactorPending": two_factor_pending, "passkey": super::passkey_routes::availability_view(&state,&ctx)})))
}

async fn create_first_admin(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Response, ApiError> {
    let form = read_form(request).await?;
    let input = SetupInput {
        email: form.get("email").unwrap_or(""),
        password: form.get("password").unwrap_or(""),
        confirm_password: form.get("confirmPassword").unwrap_or(""),
    };
    match setup::create_first_admin(&state, input, &ctx).await? {
        SetupOutcome::Created { .. } => form_response("/login", Vec::new()),
        SetupOutcome::AlreadyInitialized => Err(ApiError::new(StatusCode::CONFLICT, "conflict", setup::ALREADY_INITIALIZED)),
        SetupOutcome::Invalid(errors) => {
            let mut error = ApiError::new(StatusCode::BAD_REQUEST, "validation_failed", "입력값이 올바르지 않습니다.");
            error.field_errors = Some(errors);
            Err(error)
        }
    }
}

/// Best effort: an audit problem never changes a sign-in outcome.
async fn audit_quietly(state: &AppState, entry: AuditEntry) {
    if let Ok(client) = state.db.get().await {
        let _ = audit::write(&**client, &entry).await;
    }
}

async fn login_action(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Response, ApiError> {
    let present = session::request_cookies(request.headers());
    // A new password submission ends whatever sign-in this browser had pending — for any account,
    // whatever this submission's outcome — and every response but a new pending sign-in expires its
    // cookie. The throttle counters are not touched.
    let pending_name = pending_login::cookie_name(ctx.secure);
    let had_pending = present.iter().any(|(name, _)| name == pending_name);
    for (_, id) in present.iter().filter(|(name, _)| name == pending_name) {
        state.runtime.pending_logins.remove(id);
    }
    let respond = |redirect_to: &str, mut cookies: Vec<String>| {
        if had_pending {
            cookies.push(pending_login::clear_cookie(ctx.secure));
        }
        form_response(redirect_to, cookies)
    };
    let form = read_form(request).await?;
    let email = form.get("email").unwrap_or("");
    let now_ms = now_unix_ms();
    let throttle = &state.runtime.login_throttle;
    let log_file = state.config.auth_log_file.as_path();
    let ip = ctx.ip.as_deref();

    // Per-address ceiling first, so a throttled address costs no bcrypt work. The attempt is
    // counted on admission and settled below: a success releases it, anything else keeps it.
    let ip_attempt = match ip {
        Some(address) => match throttle.begin_ip(address, now_ms) {
            Some(attempt) => Some(attempt),
            None => {
                auth_log::append(log_file, ip, FailureReason::IpThrottled, now_ms);
                return respond(LOGIN_FAILURE_REDIRECT, Vec::new());
            }
        },
        None => None,
    };

    // A `totp` field is ignored: the second factor has its own step.
    let credentials = Credentials { email, password: form.get("password").unwrap_or("") };
    let (reason, redirect) = match login::authorize_from_ip(&state, credentials, ctx.ip.as_deref(), now_ms).await? {
        LoginOutcome::Success(user) => {
            let cookie_name = session::cookie_name(ctx.secure);
            let now = now_ms / 1000;
            let secret = state.config.auth_secrets.first().ok_or_else(ApiError::internal)?;
            let token = session::encode(&user.session_claims(), secret, cookie_name, now, session::SESSION_MAX_AGE_SECS)
                .map_err(|_| ApiError::internal())?;
            let cookies = session::issue_cookies(cookie_name, &token, &present, ctx.secure, now + session::SESSION_MAX_AGE_SECS);
            // Auth.js `events.signIn`: the login row is part of signing in.
            let client = state.db.get().await.map_err(|_| db_unavailable())?;
            let entry = AuditEntry { actor_id: Some(user.id.clone()), action: "login".into(), ip: ctx.ip.clone(), ..AuditEntry::default() };
            audit::write(&**client, &entry).await.map_err(|_| db_unavailable())?;
            return respond("/", cookies);
        }
        // The right password for an account with its own TOTP: no session, no login audit — a
        // pending sign-in for `/login/2fa`. A session this browser still holds is expired, so it
        // cannot stand in for the one being established. Not a failure: the address attempt is
        // released.
        LoginOutcome::SecondFactor(factor) => {
            let Some(id) = state.runtime.pending_logins.create(&factor.user_id, &super::throttle::email_key(email), factor.session_version, factor.factor, now_ms) else {
                // No random id, or the store is full of live sign-ins: a generic failure, uncounted.
                return respond(LOGIN_FAILURE_REDIRECT, Vec::new());
            };
            let mut cookies = session::clear_cookies(session::cookie_name(ctx.secure), &present, ctx.secure);
            cookies.push(pending_login::set_cookie(ctx.secure, &id));
            // The new pending cookie replaces the old one: no expiry line for the same name.
            return form_response(TWO_FACTOR_PAGE, cookies);
        }
        LoginOutcome::Disabled => (FailureReason::AccountDisabled, "/login?error=disabled"),
        LoginOutcome::Pending => (FailureReason::AccountPending, "/login?error=pending"),
        LoginOutcome::Rejected => (FailureReason::BadCredentials, LOGIN_FAILURE_REDIRECT),
        // Refused before any password check: not a guess, so the attempt is released uncounted.
        LoginOutcome::Busy => return respond(LOGIN_FAILURE_REDIRECT, Vec::new()),
    };

    auth_log::append(log_file, ip, reason, now_ms);
    if let Some(attempt) = ip_attempt {
        attempt.keep();
    }
    let account_key = if email.len() <= login::MAX_EMAIL_BYTES { super::throttle::email_key(email) } else { String::new() };
    audit_lock_transitions(&state, &ctx, &account_key, now_ms).await;
    respond(redirect, Vec::new())
}

fn bounded_audit_target(account: &str) -> String {
    let mut end = account.len().min(login::MAX_EMAIL_BYTES);
    while !account.is_char_boundary(end) { end -= 1; }
    account[..end].to_owned()
}

/// A failure that just locked the address or the account is audited once (per transition).
pub(super) async fn audit_lock_transitions(state: &AppState, ctx: &RequestContext, account_key: &str, now_ms: i64) {
    let throttle = &state.runtime.login_throttle;
    if let Some(ip) = ctx.ip.as_deref() {
        if throttle.take_ip_lock_notice(ip, now_ms) {
            audit_quietly(state, AuditEntry { action: "auth:ip-locked".into(), target: Some(ip.to_owned()), ip: Some(ip.to_owned()), ..AuditEntry::default() }).await;
        }
    }
    if let Some(email) = account_key.strip_prefix("email:").filter(|email| !email.is_empty() && email.len() <= login::MAX_EMAIL_BYTES) {
        let key = super::throttle::login_key(account_key, ctx.ip.as_deref());
        if throttle.take_account_lock_notice(&key, now_ms) {
            audit_quietly(state, AuditEntry { action: "auth:login-locked".into(), target: Some(bounded_audit_target(email)), ip: ctx.ip.clone(), ..AuditEntry::default() }).await;
        }
    }
}

/// Completes a pending sign-in with a TOTP or backup code (form field `totp`).
///
/// Only a live pending sign-in of this browser can complete, once: its lock serialises concurrent
/// submissions, and the first success takes it out of the store before the session exists. The
/// address and the account are throttled like the password step — factor failures count against
/// the same account limit — and each pending sign-in dies after [`pending_login::MAX_ATTEMPTS`]
/// wrong codes. The account is read again with the code, and once more in the transaction that
/// writes the `login` audit, before any session is issued.
/// Which screen sent the code: `/login/2fa` (`method=totp`) or `/login/2fa/backup`
/// (`method=backup`). A page from before the split sends no method and takes either kind.
/// Anything else — empty, another value or case, the field twice — is `None`: a failed
/// submission whose code is never looked at.
fn factor_method(form: &FormInput) -> Option<FactorKind> {
    match form.all("method") {
        [] => Some(FactorKind::Either),
        [method] => match method.as_str() {
            "totp" => Some(FactorKind::Authenticator),
            "backup" => Some(FactorKind::Backup),
            _ => None,
        },
        _ => None,
    }
}

async fn verify_two_factor(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Response, ApiError> {
    let present = session::request_cookies(request.headers());
    let id = pending_login::read_cookie(request.headers(), ctx.secure);
    let form = read_form(request).await?;
    let method = factor_method(&form);
    let invalid = if method == Some(FactorKind::Backup) { TWO_FACTOR_BACKUP_INVALID_REDIRECT } else { TWO_FACTOR_INVALID_REDIRECT };
    let now_ms = now_unix_ms();
    let store = &state.runtime.pending_logins;
    let throttle = &state.runtime.login_throttle;
    let log_file = state.config.auth_log_file.as_path();
    let ip = ctx.ip.as_deref();
    let clear = || vec![pending_login::clear_cookie(ctx.secure)];
    let expired = || form_response(TWO_FACTOR_EXPIRED_REDIRECT, clear());

    let Some(id) = id else { return expired() };
    let Some(slot) = store.get(&id, now_ms) else { return expired() };
    let mut pending = slot.lock().await;
    let end = |pending: &mut Pending| {
        pending.finished = true;
        store.take_if_current(&id, &slot);
    };
    // Read after the wait for the lock: a verification ahead of this one may have completed or
    // spent it, a cancel or a new password step may have removed it, or it may have expired.
    if !pending.live(now_unix_ms()) || !store.is_current(&id, &slot) {
        end(&mut pending);
        return expired();
    }

    let ip_attempt = match ip {
        Some(address) => match throttle.begin_ip(address, now_ms) {
            Some(attempt) => Some(attempt),
            // A lock ends the pending sign-in without blaming the password, which was right.
            None => {
                auth_log::append(log_file, ip, FailureReason::IpThrottled, now_ms);
                end(&mut pending);
                return expired();
            }
        },
        None => None,
    };
    let login_key = super::throttle::login_key(&pending.throttle_key, ctx.ip.as_deref());
    let Some(account_attempt) = throttle.begin_account(&login_key, now_ms) else {
        auth_log::append(log_file, ip, FailureReason::BadCredentials, now_ms);
        end(&mut pending);
        return expired();
    };
    let Some(account) = pending_login::current_account(&state, &pending).await? else {
        end(&mut pending);
        return expired();
    };

    // Once more right before the code is used: a code spent on a sign-in that cannot complete is
    // lost to the user. (A cancel or a new password step that lands during the check itself, or a
    // failed final re-check, still spends it — the pending sign-in is gone either way.)
    if !pending.live(now_unix_ms()) || !store.is_current(&id, &slot) {
        end(&mut pending);
        return expired();
    }
    let code = crate::crypto::js_trim(form.get("totp").unwrap_or(""));
    let factor = StoredFactor { totp_secret: account.totp_secret, backup_codes: account.backup_codes };
    // The time the code is checked against is read now, after any wait, not at request start.
    let accepted = match method {
        Some(kind) => login::accept_second_factor(&state, &pending.user_id, factor, kind, code, now_unix_ms()).await?,
        None => false,
    };
    if !accepted {
        account_attempt.keep();
        if let Some(attempt) = ip_attempt {
            attempt.keep();
        }
        auth_log::append(log_file, ip, FailureReason::BadCredentials, now_ms);
        let key = pending.throttle_key.clone();
        pending.attempts += 1;
        let spent = pending.attempts >= pending_login::MAX_ATTEMPTS;
        if spent {
            end(&mut pending);
        }
        drop(pending);
        audit_lock_transitions(&state, &ctx, &key, now_ms).await;
        return if spent { expired() } else { form_response(invalid, Vec::new()) };
    }

    // Consumed here, exactly once: an expired (the code check can take a while), replaced or
    // cancelled pending sign-in does not complete.
    let live = pending.live(now_unix_ms());
    pending.finished = true;
    if !store.take_if_current(&id, &slot) || !live {
        return expired();
    }
    let Some(user) = pending_login::complete_login(&state, &pending, &ctx, &now_unix_ms).await? else { return expired() };
    throttle.clear_account(&login_key);
    drop(account_attempt);
    drop(ip_attempt);

    let cookie_name = session::cookie_name(ctx.secure);
    let now = now_ms / 1000;
    let secret = state.config.auth_secrets.first().ok_or_else(ApiError::internal)?;
    let token = session::encode(&user.session_claims(), secret, cookie_name, now, session::SESSION_MAX_AGE_SECS).map_err(|_| ApiError::internal())?;
    let mut cookies = session::issue_cookies(cookie_name, &token, &present, ctx.secure, now + session::SESSION_MAX_AGE_SECS);
    cookies.extend(clear());
    form_response("/", cookies)
}

/// Drops this browser's pending sign-in (the factor screen's "back to sign-in").
async fn cancel_two_factor(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Response, ApiError> {
    if let Some(id) = pending_login::read_cookie(&headers, ctx.secure) {
        state.runtime.pending_logins.remove(&id);
    }
    form_response("/login", vec![pending_login::clear_cookie(ctx.secure)])
}

/// Sign-out revokes on the server: the user's `session_version` moves on, so every cookie of that
/// user stops authenticating — this one, its copies, other devices, and a refresh that was in
/// flight and delivers its Set-Cookie after this response (Auth.js only expired the cookie in
/// this browser; a copy stayed valid, and slid on). The bump is conditional on the cookie's own
/// version, so a stale cookie revokes nothing and audits nothing. A database failure answers 503
/// — the session may still be alive, and saying "signed out" would be false — though this
/// browser's cookies are expired either way.
async fn sign_out(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Response, ApiError> {
    let present = session::request_cookies(&headers);
    let cookie_name = session::cookie_name(ctx.secure);
    let mut cleared = session::clear_cookies(cookie_name, &present, ctx.secure);
    // A pending two-step sign-in of this browser ends too, with or without a session: otherwise
    // signing out would lead straight back to its factor screen.
    let pending_name = pending_login::cookie_name(ctx.secure);
    if present.iter().any(|(name, _)| name == pending_name) {
        for (_, id) in present.iter().filter(|(name, _)| name == pending_name) {
            state.runtime.pending_logins.remove(id);
        }
        cleared.push(pending_login::clear_cookie(ctx.secure));
    }
    if let Some(claims) = principal::session_claims(&state, &headers, &ctx, now_unix_ms() / 1000) {
        let user_id = claims.get("id").and_then(Value::as_str).filter(|id| !id.is_empty());
        // TS `(sv ?? 0)`, as authentication reads it.
        let sv = match claims.get("sv") {
            None | Some(Value::Null) => Some(0),
            Some(value) => value.as_i64(),
        };
        if let (Some(user_id), Some(sv)) = (user_id, sv.and_then(|sv| i32::try_from(sv).ok()))
            && revoke_sessions(&state, user_id, sv, &ctx).await.is_err()
        {
            return with_cookies(db_unavailable().into_response(), cleared);
        }
    }
    form_response("/login", cleared)
}

/// `UPDATE … WHERE session_version = <the cookie's>` and the `logout` audit, together.
async fn revoke_sessions(state: &AppState, user_id: &str, sv: i32, ctx: &RequestContext) -> Result<(), ApiError> {
    let failed = |_| db_unavailable();
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(failed)?;
    let revoked = tx
        .execute("UPDATE users SET session_version = session_version + 1 WHERE id = $1 AND session_version = $2", &[&user_id, &sv])
        .await
        .map_err(failed)?;
    if revoked == 1 {
        let entry = AuditEntry { actor_id: Some(user_id.to_owned()), action: "logout".into(), ip: ctx.ip.clone(), ..AuditEntry::default() };
        audit::write(&*tx, &entry).await.map_err(failed)?;
    }
    tx.commit().await.map_err(failed)
}

#[cfg(test)]
mod audit_tests {
    #[test]
    fn audit_s4_audit_targets_are_byte_bounded_without_splitting_utf8() {
        for text in ["a".repeat(1000), "é".repeat(500), "x".repeat(319) + "é"] {
            let target = super::bounded_audit_target(&text);
            assert!(target.len() <= 320);
            assert!(text.starts_with(&target));
        }
    }
}
