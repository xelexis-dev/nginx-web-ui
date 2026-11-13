//! Who is calling, re-validated against the database on every request.
//!
//! Route handlers first decide which kind of principal they accept, then check role/resource
//! permission. The kinds never bridge: a gateway Bearer token is never a browser session, and
//! the browser/admin surface accepts sessions only.
//!
//! - **Session** — the Auth.js-compatible cookie (see [`super::session`]). The cookie only proves
//!   we issued it; the user row must still exist, be enabled and approved, and carry the same
//!   `sessionVersion` (bumped on role/password/policy change, disable, deletion). `role` and
//!   `totpEnabled` are taken from the row. `requires2fa` is the login-time claim: every policy
//!   change bumps the affected users' `sessionVersion`, so a matching version means it is current.
//!   (`apps/web/lib/guard.ts` `currentUser`, `apps/web/proxy.ts`.)
//! - **Gateway** — `Authorization: Bearer` node token for `/api/v1` (`apps/web/lib/node-api.ts`
//!   `authenticateNode`): version gate first, then the token, then RBAC on the token's role.
//! - **Pairing** — a `node:pair` server access token presented to `POST /api/v1/pair`
//!   (`apps/web/app/api/v1/pair/route.ts`).
//!
//! RFC 9110 §11.1: the auth-scheme is case-insensitive (TS accepted only `Bearer `; intentional
//! change). RFC 6750 §3: 401 carries `WWW-Authenticate: Bearer` ([`ApiError::new`]'s default).

use axum::http::{HeaderMap, StatusCode};
use serde_json::{Value, json};

use super::audit::{self, AuditEntry};
use super::node_version::{describe_mismatch, is_compatible};
use super::rbac::{Action, can, parse_role, role_str};
use super::session;
use super::throttle::LoginThrottle;
use crate::contracts::{Actor, Role};
use crate::error::ApiError;
use crate::request_context::RequestContext;
use crate::state::AppState;

pub const GATEWAY_VERSION_HEADER: &str = "x-gateway-version";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPrincipal {
    pub user_id: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub role: Role,
    pub session_version: i32,
    pub totp_enabled: bool,
    pub requires_2fa: bool,
    /// This session proved this app's second factor (`mfaVerified` claim): a sign-in or an
    /// enrollment that accepted a code, or `account.verifyTwoFactor`. An OAuth sign-in never sets
    /// it — the provider's login is not this app's factor.
    pub mfa_verified: bool,
    /// The cookie's `exp` (unix seconds).
    pub expires_at: i64,
}

impl SessionPrincipal {
    /// 2FA is required by policy but not enrolled (fresh DB value): only enrollment and sign-out.
    pub fn must_enroll_2fa(&self) -> bool {
        self.requires_2fa && !self.totp_enabled
    }

    /// 2FA is required and enrolled, but this session has not proved it (an OAuth sign-in, or a
    /// cookie from before the claim existed — asked once): only verification and sign-out.
    pub fn must_verify_2fa(&self) -> bool {
        self.requires_2fa && self.totp_enabled && !self.mfa_verified
    }

    pub fn actor(&self, ctx: &RequestContext) -> Actor {
        Actor { id: Some(self.user_id.clone()), label: None, ip: ctx.ip.clone() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayPrincipal {
    pub token_id: String,
    pub name: String,
    pub role: Role,
}

impl GatewayPrincipal {
    pub fn actor(&self, ctx: &RequestContext) -> Actor {
        Actor { id: None, label: Some(format!("gateway:{}", self.name)), ip: ctx.ip.clone() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingGrant {
    pub access_token_id: String,
    pub scopes: Value,
    pub legacy_pairing: bool,
    /// The admin who issued the server token (becomes `gateway_tokens.created_by_user_id`).
    pub created_by_user_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Principal {
    Anonymous,
    Session(SessionPrincipal),
    Gateway(GatewayPrincipal),
    Pairing(PairingGrant),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TwoFactorGate {
    /// Refuse sessions that must still enroll or verify 2FA.
    Enforce,
    /// The 2FA screens (enroll, verify) and sign-out.
    AllowEnrollment,
}

pub fn db_unavailable() -> ApiError {
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "service_unavailable", "데이터베이스를 사용할 수 없습니다. 잠시 후 다시 시도하세요.")
}

pub fn two_factor_required() -> ApiError {
    ApiError::new(StatusCode::FORBIDDEN, "two_factor_required", "2단계 인증을 먼저 등록해야 합니다.")
}

/// Same code as enrollment (the screens route both to `/account/2fa`), its own words.
pub fn two_factor_verification_required() -> ApiError {
    ApiError::new(StatusCode::FORBIDDEN, "two_factor_required", "2단계 인증 코드를 먼저 확인해야 합니다.")
}

pub fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ───────────────────────────── session ─────────────────────────────

/// The request's session claims, or `None` when there is no usable cookie (absent, ambiguous
/// chunks, bad token, expired). Only proves the cookie is ours — nothing about the account.
pub fn session_claims(state: &AppState, headers: &HeaderMap, ctx: &RequestContext, now: i64) -> Option<session::Claims> {
    let cookie_name = session::cookie_name(ctx.secure);
    let cookies = session::request_cookies(headers);
    let token = session::read_cookie(cookies.iter().map(|(n, v)| (n.as_str(), v.as_str())), cookie_name).ok()??;
    let secrets: Vec<&str> = state.config.auth_secrets.iter().map(String::as_str).collect();
    session::decode(&token, &secrets, cookie_name, now).ok()
}

/// Cookie + DB re-validation. `Ok(None)` = not signed in (or revoked). `Err` only for DB outage.
pub async fn authenticate_session(state: &AppState, headers: &HeaderMap, ctx: &RequestContext, now: i64) -> Result<Option<SessionPrincipal>, ApiError> {
    let Some(claims) = session_claims(state, headers, ctx, now) else { return Ok(None) };
    let Some(user_id) = claims.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    // TS `(sv ?? 0) !== user.sessionVersion`: absent is 0; anything but a number never matches.
    let claimed_sv = match claims.get("sv") {
        None | Some(Value::Null) => Some(0),
        Some(value) => value.as_i64(),
    };
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let row = client
        .query_opt(
            "SELECT email, role::text AS role, session_version, totp_enabled, enabled, approved FROM users WHERE id = $1",
            &[&user_id],
        )
        .await
        .map_err(|_| db_unavailable())?;
    let Some(row) = row else { return Ok(None) };
    let session_version: i32 = row.get("session_version");
    if claimed_sv != Some(i64::from(session_version)) || !row.get::<_, bool>("enabled") || !row.get::<_, bool>("approved") {
        return Ok(None);
    }
    let role = parse_role(row.get("role")).ok_or_else(ApiError::internal)?;
    let totp_enabled: bool = row.get("totp_enabled");
    let policy = claims.get("requires2fa").and_then(Value::as_bool).unwrap_or(false);
    // An external provider's login is not this app's factor: a user who enrolled a TOTP proves
    // it. Read from the row, so disabling 2FA later does not trap the session in enrollment.
    let external = claims.get("externalLogin").and_then(Value::as_bool).unwrap_or(false);
    Ok(Some(SessionPrincipal {
        user_id: user_id.to_owned(),
        email: row.get("email"),
        name: claims.get("name").and_then(Value::as_str).map(str::to_owned),
        role,
        session_version,
        totp_enabled,
        requires_2fa: policy || (external && totp_enabled),
        // Absent (older cookies) is unproven: under a requiring policy such a session is asked once.
        mfa_verified: claims.get("mfaVerified").and_then(Value::as_bool).unwrap_or(false),
        expires_at: claims.get("exp").and_then(Value::as_f64).map(|e| e as i64).unwrap_or(now),
    }))
}

/// The session guard for browser routes.
///
/// No/invalid/revoked session → [`ApiError::session_required`]. 2FA not enrolled while required
/// and `gate` is `Enforce` → 403 `two_factor_required`. Role lacks `action` → best-effort
/// `rbac:denied` audit, then [`ApiError::forbidden`].
pub async fn require_session(
    state: &AppState,
    headers: &HeaderMap,
    ctx: &RequestContext,
    action: Option<Action>,
    gate: TwoFactorGate,
) -> Result<SessionPrincipal, ApiError> {
    let principal = authenticate_session(state, headers, ctx, now_unix_ms() / 1000)
        .await?
        .ok_or_else(ApiError::session_required)?;
    if gate == TwoFactorGate::Enforce && principal.must_enroll_2fa() {
        return Err(two_factor_required());
    }
    if gate == TwoFactorGate::Enforce && principal.must_verify_2fa() {
        return Err(two_factor_verification_required());
    }
    if let Some(action) = action
        && !can(principal.role, action)
    {
        // A failed audit write must never turn the denial into something else.
        if let Ok(client) = state.db.get().await {
            let entry = AuditEntry {
                actor_id: Some(principal.user_id.clone()),
                action: "rbac:denied".into(),
                target: Some(action.as_str().into()),
                before: Some(json!({"role": role_str(principal.role), "requiredAction": action.as_str()})),
                ip: ctx.ip.clone(),
                ..AuditEntry::default()
            };
            let _ = audit::write(&**client, &entry).await;
        }
        return Err(ApiError::forbidden());
    }
    Ok(principal)
}

// ───────────────────────────── gateway ─────────────────────────────

/// The Bearer credential: exactly one `Authorization` header, scheme matched case-insensitively,
/// token = the rest after one space, ECMAScript-trimmed (TS `slice(7).trim()`); empty → `None`.
pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let mut values = headers.get_all(axum::http::header::AUTHORIZATION).iter();
    let (Some(value), None) = (values.next(), values.next()) else { return None };
    let (scheme, rest) = value.to_str().ok()?.split_once(' ')?;
    let token = crate::crypto::js_trim(rest);
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then(|| token.to_owned())
}

/// Accept a same-major gateway (≥ 2) whose minor is at least this node’s, or 2.0 ↔ 1.19.
/// Major 1 keeps exact major.minor; missing or unreadable → 409.
pub fn check_gateway_version(headers: &HeaderMap, node_version: &str) -> Result<(), ApiError> {
    // Fetch joins repeated headers with ", " — that is what TS compared (and it never parses).
    let gateway = headers
        .get_all(GATEWAY_VERSION_HEADER)
        .iter()
        .map(|v| v.to_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(", ");
    if is_compatible(&gateway, node_version) {
        return Ok(());
    }
    Err(ApiError::new(StatusCode::CONFLICT, "version_mismatch", describe_mismatch(&gateway, node_version)))
}

fn invalid_token() -> ApiError {
    ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", "토큰이 유효하지 않거나 해지되었습니다.")
}

/// Gateway guard for `/api/v1`: version → Bearer → token lookup (unknown and revoked give the
/// byte-identical 401) → best-effort `last_used_*` (before RBAC, so a 403 still records use) →
/// each of `actions` in order, stopping at the first denial. Empty `actions` = authenticate only.
pub async fn require_gateway(state: &AppState, headers: &HeaderMap, ctx: &RequestContext, actions: &[Action]) -> Result<GatewayPrincipal, ApiError> {
    check_gateway_version(headers, &state.config.version)?;
    let token = bearer_token(headers)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", "인증 토큰이 없습니다."))?;
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    // The compatibility marker preserves existing gateways; new rows follow their issuer.
    let row = client
        .query_opt(
            "SELECT id, name, role::text AS role, revoked_at IS NOT NULL AS revoked FROM gateway_tokens WHERE token_hash = $1 AND (legacy_issuer OR EXISTS (SELECT 1 FROM users WHERE id = gateway_tokens.created_by_user_id AND enabled AND approved AND role = 'admin'))",
            &[&super::token_digest(&token)],
        )
        .await
        .map_err(|_| db_unavailable())?;
    let Some(row) = row.filter(|r| !r.get::<_, bool>("revoked")) else { return Err(invalid_token()) };
    let gateway = GatewayPrincipal {
        token_id: row.get("id"),
        name: row.get("name"),
        role: parse_role(row.get("role")).ok_or_else(ApiError::internal)?,
    };
    // Best effort: failing to leave a trace must not decide access.
    let _ = client
        .execute("UPDATE gateway_tokens SET last_used_at = now(), last_used_ip = $2 WHERE id = $1", &[&gateway.token_id, &ctx.ip])
        .await;
    if actions.iter().any(|&action| !can(gateway.role, action)) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            format!("이 토큰의 등급({})으로는 할 수 없는 동작입니다.", role_str(gateway.role)),
        ));
    }
    Ok(gateway)
}

// ───────────────────────────── pairing ─────────────────────────────

/// Runs before the pairing body is read, so a locked address gets 429 even with a bad body.
/// An unknown client IP is never locked (and never counted). Uses the process-wide throttle
/// shared with sign-in (`state.runtime.login_throttle`).
pub fn check_pairing_ip_lock(state: &AppState, ctx: &RequestContext, now_ms: i64) -> Result<(), ApiError> {
    ip_lock(&state.runtime.login_throttle, ctx, now_ms)
}

fn ip_lock(throttle: &LoginThrottle, ctx: &RequestContext, now_ms: i64) -> Result<(), ApiError> {
    match &ctx.ip {
        Some(ip) if throttle.is_ip_locked(ip, now_ms) => Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "실패가 잦아 잠시 차단되었습니다. 15분 뒤에 다시 시도하세요.",
        )),
        _ => Ok(()),
    }
}

/// Validates a non-empty presented server token (the caller rejects an empty one with 400 first).
///
/// not_found / revoked / expired → one identical 401 and one IP failure (distinguishing them
/// would confirm a token exists). wrong_kind / missing_scope → distinct 403s, not counted (the
/// caller already holds a valid token; it is a configuration mistake, not a guess). Order:
/// revoked → expired → kind → scope (TS `checkToken`). `last_used_*` is written and a failure is
/// an error: pairing is a one-off human action and must not proceed untracked.
pub async fn authenticate_pairing(state: &AppState, presented: &str, ctx: &RequestContext, now_ms: i64) -> Result<PairingGrant, ApiError> {
    let throttle = &state.runtime.login_throttle;
    let guess_failed = || {
        if let Some(ip) = &ctx.ip {
            throttle.record_ip_failure(ip, now_ms);
        }
        invalid_token()
    };
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    // Looked up by hash only: a unique-index lookup on SHA-256 needs no constant-time compare.
    let row = client
        .query_opt(
            "SELECT id, kind::text AS kind, scopes, revoked_at IS NOT NULL AS revoked, \
                    (extract(epoch FROM expires_at) * 1000)::bigint AS expires_ms, created_by_user_id, legacy_pairing \
             FROM access_tokens WHERE token_hash = $1 LIMIT 1",
            &[&super::token_digest(presented)],
        )
        .await
        .map_err(|_| db_unavailable())?;
    let Some(row) = row else { return Err(guess_failed()) };
    if row.get::<_, bool>("revoked") || row.get::<_, Option<i64>>("expires_ms").is_some_and(|expires| expires <= now_ms) {
        return Err(guess_failed());
    }
    if row.get::<_, String>("kind") != "server" {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "사용자 토큰으로는 노드를 연결할 수 없습니다 — 이 노드의 관리자 화면에서 발급한 서버 토큰이 필요합니다.",
        ));
    }
    // Only an array grants: a scalar string must never become a substring match (TS `scopeSatisfies`).
    let scopes: Value = row.get("scopes");
    if !scopes.as_array().is_some_and(|granted| granted.iter().any(|s| s == "node:pair")) {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "forbidden", "이 토큰에는 노드 연결(node:pair) 권한이 없습니다."));
    }
    // The token delegates its issuer's authority to connect gateways: it ends with that authority
    // (issuer deleted — the FK sets NULL —, disabled, unapproved or no longer an admin). Not a
    // guess: the caller holds a real token, so this is not counted against the address.
    let created_by_user_id: Option<String> = row.get("created_by_user_id");
    let issuer_active = match &created_by_user_id {
        Some(issuer) => client
            .query_opt("SELECT 1 FROM users WHERE id = $1 AND enabled AND approved AND role = 'admin'", &[issuer])
            .await
            .map_err(|_| db_unavailable())?
            .is_some(),
        None => false,
    };
    if !issuer_active {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "이 토큰을 발급한 관리자 계정이 삭제·비활성화되었거나 관리자가 아닙니다 — 이 노드에서 서버 토큰을 새로 발급하세요.",
        ));
    }
    let grant = PairingGrant { access_token_id: row.get("id"), scopes, legacy_pairing: row.get("legacy_pairing"), created_by_user_id };
    client
        .execute("UPDATE access_tokens SET last_used_at = now(), last_used_ip = $2 WHERE id = $1", &[&grant.access_token_id, &ctx.ip])
        .await
        .map_err(|_| db_unavailable())?;
    Ok(grant)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (name, value) in pairs {
            h.append(axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(), value.parse().unwrap());
        }
        h
    }

    #[test]
    fn bearer_scheme_is_case_insensitive_and_token_trimmed() {
        assert_eq!(bearer_token(&headers(&[("authorization", "Bearer abc")])).as_deref(), Some("abc"));
        assert_eq!(bearer_token(&headers(&[("authorization", "bearer  abc ")])).as_deref(), Some("abc"));
        assert_eq!(bearer_token(&headers(&[("authorization", "BEARER abc")])).as_deref(), Some("abc"));
        for bad in ["Bearer", "Bearer ", "Bearer    ", "Basic abc", "Bearerabc", "abc"] {
            assert_eq!(bearer_token(&headers(&[("authorization", bad)])), None, "{bad:?}");
        }
        assert_eq!(bearer_token(&headers(&[])), None);
        assert_eq!(bearer_token(&headers(&[("authorization", "Bearer a"), ("authorization", "Bearer b")])), None, "ambiguous");
    }

    #[test]
    fn gateway_version_gate_uses_major_minor() {
        assert!(check_gateway_version(&headers(&[("x-gateway-version", "1.17.9")]), "1.17.2").is_ok());
        assert!(check_gateway_version(&headers(&[("x-gateway-version", "1.17.0-rc.1")]), "1.17.2").is_ok());
        for gateway in [None, Some("1.18.0"), Some("junk")] {
            let h = gateway.map(|g| headers(&[("x-gateway-version", g)])).unwrap_or_default();
            let err = check_gateway_version(&h, "1.17.2").unwrap_err();
            assert_eq!((err.status, err.code), (StatusCode::CONFLICT, "version_mismatch"), "{gateway:?}");
            assert_eq!(err.message, describe_mismatch(gateway.unwrap_or(""), "1.17.2"));
        }
    }

    #[test]
    fn gateway_first_at_the_node() {
        for gateway in ["2.0.0", "2.0.9", "2.1.0", "2.9.3", "1.19.0", "1.19.11"] {
            assert!(check_gateway_version(&headers(&[("x-gateway-version", gateway)]), "2.0.0").is_ok(), "{gateway}");
        }
        for gateway in [None, Some("3.0.0"), Some("1.18.9"), Some("1.20.0"), Some(""), Some("junk")] {
            let h = gateway.map(|g| headers(&[("x-gateway-version", g)])).unwrap_or_default();
            let err = check_gateway_version(&h, "2.0.0").unwrap_err();
            assert_eq!((err.status, err.code), (StatusCode::CONFLICT, "version_mismatch"));
            assert_eq!(err.message, describe_mismatch(gateway.unwrap_or(""), "2.0.0"));
        }
        for gateway in ["2.0.9", "1.19.11"] {
            let err = check_gateway_version(&headers(&[("x-gateway-version", gateway)]), "2.1.0").unwrap_err();
            assert_eq!((err.status, err.code), (StatusCode::CONFLICT, "version_mismatch"));
            assert!(err.message.contains("게이트웨이를 올리세요"));
        }
    }

    #[test]
    fn pairing_lock_only_applies_to_a_known_ip() {
        let throttle = LoginThrottle::default();
        let ctx = |ip: Option<&str>| RequestContext { ip: ip.map(str::to_owned), external_origin: "http://x".into(), secure: false, origin_configured: false };
        for _ in 0..super::super::throttle::MAX_IP_FAILURES {
            throttle.record_ip_failure("203.0.113.9", 1_000);
        }
        let err = ip_lock(&throttle, &ctx(Some("203.0.113.9")), 2_000).unwrap_err();
        assert_eq!((err.status, err.code), (StatusCode::TOO_MANY_REQUESTS, "rate_limited"));
        assert_eq!(err.message, "실패가 잦아 잠시 차단되었습니다. 15분 뒤에 다시 시도하세요.");
        assert!(ip_lock(&throttle, &ctx(Some("203.0.113.10")), 2_000).is_ok());
        assert!(ip_lock(&throttle, &ctx(None), 2_000).is_ok());
    }

    #[test]
    fn two_factor_enrollment_is_required_only_when_policy_says_so_and_not_enrolled() {
        let p = |requires_2fa, totp_enabled| SessionPrincipal {
            user_id: "u".into(), email: None, name: None, role: Role::Viewer, session_version: 0,
            totp_enabled, requires_2fa, mfa_verified: false, expires_at: 0,
        };
        assert!(p(true, false).must_enroll_2fa());
        assert!(!p(true, true).must_enroll_2fa());
        assert!(!p(false, false).must_enroll_2fa());
    }

    #[test]
    fn verification_is_required_only_when_policy_says_so_enrolled_and_unproven() {
        let p = |requires_2fa, totp_enabled, mfa_verified| SessionPrincipal {
            user_id: "u".into(), email: None, name: None, role: Role::Viewer, session_version: 0,
            totp_enabled, requires_2fa, mfa_verified, expires_at: 0,
        };
        assert!(p(true, true, false).must_verify_2fa());
        assert!(!p(true, true, true).must_verify_2fa());
        assert!(!p(true, false, false).must_verify_2fa(), "enrollment first");
        assert!(!p(false, true, false).must_verify_2fa(), "not required by policy");
    }
}
