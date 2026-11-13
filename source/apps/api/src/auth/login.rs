//! Credentials sign-in, a port of `packages/auth/src/authorize.ts`.
//!
//! Same order and same enumeration defences: account throttle → user lookup → constant-work
//! bcrypt against a dummy hash for unknown or password-less (OAuth-only) accounts → password →
//! disabled → not approved → role-based 2FA policy. Disabled / pending are only reported after the
//! password matched, so they reveal nothing to a guesser.
//!
//! **Two steps for an account with its own TOTP.** The password alone never signs such an account
//! in: [`authorize`] answers [`LoginOutcome::SecondFactor`] and the caller records a pending sign-in
//! (`pending_login`) that `/login/2fa` completes with [`accept_second_factor`]. The password step
//! takes no code at all (one sent anyway is ignored, not consumed) and does not clear the account
//! throttle — factor failures keep counting against the same limit however often the password is
//! entered again.
//!
//! Two deliberate changes to the factor check (both stricter, no data loss):
//! - RFC 6238 §5.2 — a TOTP code is accepted once: the matched time step is recorded with
//!   `UPDATE … SET totp_last_step = $step WHERE … (totp_last_step IS NULL OR totp_last_step < $step)`;
//!   a code for the same or an earlier step is refused.
//! - A backup code is removed with a compare-and-swap on the stored array, so two concurrent
//!   sign-ins with the same code cannot both succeed (TS read-then-write allowed that).

use serde_json::{Value, json};

use super::rbac::{parse_role, role_str};
use super::session::Claims;
use super::password::PasswordError;
use super::totp;
use crate::contracts::Role;
use crate::error::ApiError;
use crate::state::AppState;

/// A valid cost-12 hash of a throwaway string (same value as TS): unknown accounts still cost one
/// bcrypt, so timing does not reveal whether an email exists.
pub const DUMMY_PASSWORD_HASH: &str = "$2b$12$9HsPVSamNL.9VAeawoj9M..5Xq4bc9wZYDwcGROCJWG23P69G5j6.";

pub const MAX_EMAIL_BYTES: usize = 320;

#[derive(Clone, Copy, Debug)]
pub struct Credentials<'a> {
    pub email: &'a str,
    pub password: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizedUser {
    pub id: String,
    pub email: Option<String>,
    pub role: Role,
    pub session_version: i32,
    pub totp_enabled: bool,
    pub requires_2fa: bool,
    /// This sign-in accepted this app's second factor (see `SessionPrincipal::mfa_verified`).
    pub mfa_verified: bool,
    /// Signed in through an external identity provider, whose login is not this app's factor.
    pub external_login: bool,
}

impl AuthorizedUser {
    /// The JWT claims Auth.js writes for a credentials sign-in (its default token plus the
    /// `jwt` callback of `packages/auth/src/auth.config.ts`). `iat`/`exp`/`jti` come from encode.
    pub fn session_claims(&self) -> Claims {
        let mut claims = Claims::new();
        claims.insert("authTime".into(), (super::principal::now_unix_ms() / 1000).into());
        claims.insert("sub".into(), self.id.clone().into());
        if let Some(email) = &self.email {
            claims.insert("email".into(), email.clone().into());
        }
        claims.insert("id".into(), self.id.clone().into());
        claims.insert("role".into(), role_str(self.role).into());
        claims.insert("sv".into(), self.session_version.into());
        claims.insert("totpEnabled".into(), self.totp_enabled.into());
        claims.insert("requires2fa".into(), self.requires_2fa.into());
        claims.insert("mfaVerified".into(), self.mfa_verified.into());
        if self.external_login {
            claims.insert("externalLogin".into(), true.into());
        }
        claims
    }
}

/// The password matched an account with its own TOTP: what a pending sign-in must still find.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingFactor {
    pub user_id: String,
    pub session_version: i32,
    /// `pending_login::factor_digest` of the stored second factor.
    pub factor: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginOutcome {
    Success(AuthorizedUser),
    /// No session yet: the second factor comes on its own screen.
    SecondFactor(PendingFactor),
    /// Wrong/missing anything, unknown account, locked account — one indistinguishable outcome.
    Rejected,
    Disabled,
    Pending,
    /// The password check queue is full; nothing was checked and nothing is counted.
    Busy,
}

pub async fn authorize(state: &AppState, credentials: Credentials<'_>, now_ms: i64) -> Result<LoginOutcome, ApiError> {
    authorize_from_ip(state, credentials, None, now_ms).await
}

pub async fn authorize_from_ip(state: &AppState, credentials: Credentials<'_>, ip: Option<&str>, now_ms: i64) -> Result<LoginOutcome, ApiError> {
    let Credentials { email, password } = credentials;
    if email.len() > MAX_EMAIL_BYTES || password.is_empty() {
        return Ok(LoginOutcome::Rejected);
    }
    let trimmed_email = crate::crypto::js_trim(email);
    let normalized_email = trimmed_email.to_lowercase();
    let email = normalized_email.as_str();
    if email.is_empty() || email.len() > MAX_EMAIL_BYTES {
        return Ok(LoginOutcome::Rejected);
    }
    let throttle = &state.runtime.login_throttle;
    // Counted on admission. A failure keeps it; any other exit releases it on drop.
    let account_key = super::throttle::login_key(&super::throttle::email_key(email), ip);
    let Some(attempt) = throttle.begin_account(&account_key, now_ms) else {
        return Ok(LoginOutcome::Rejected);
    };
    let rejected = |attempt: crate::auth::throttle::Attempt<'_>| {
        attempt.keep();
        Ok(LoginOutcome::Rejected)
    };

    // Pool connections are borrowed per statement and never held across bcrypt: sign-ins queue
    // for a bcrypt slot, and a queued sign-in holding a connection would starve every other route.
    let rows = {
        let client = state.db.get().await.map_err(|_| db_unavailable())?;
        client.query(
            "SELECT id, email, password_hash, role::text AS role, session_version, totp_enabled, totp_secret, \
                    backup_codes, enabled, approved \
             FROM users WHERE lower(email) = $1 ORDER BY (email = $2) DESC, id LIMIT 2",
            &[&email, &trimmed_email],
        ).await.map_err(|_| db_unavailable())?
    };
    // Legacy emails were case-sensitive and may collide after folding. Never guess an identity.
    let row = if rows.len() == 1 || rows.first().is_some_and(|row| row.get::<_, Option<String>>("email").as_deref() == Some(trimmed_email)) {
        rows.into_iter().next()
    } else {
        None
    };
    let password_hash = row.as_ref().and_then(|r| r.get::<_, Option<String>>("password_hash"));
    let (Some(row), Some(password_hash)) = (row, password_hash) else {
        // Unknown, ambiguous or password-less (OAuth-only) account: the same bcrypt work, then a failure.
        return match state.passwords.verify_or_busy(password.to_owned(), DUMMY_PASSWORD_HASH.to_owned()).await {
            Err(PasswordError::Busy) => Ok(LoginOutcome::Busy),
            _ => rejected(attempt),
        };
    };
    match state.passwords.verify_or_busy(password.to_owned(), password_hash).await {
        Ok(true) => {}
        Err(PasswordError::Busy) => return Ok(LoginOutcome::Busy),
        _ => return rejected(attempt),
    }
    // An administrator's decision, not a guess: not counted.
    if !row.get::<_, bool>("enabled") {
        return Ok(LoginOutcome::Disabled);
    }
    if !row.get::<_, bool>("approved") {
        return Ok(LoginOutcome::Pending);
    }

    let id: String = row.get("id");
    let totp_enabled: bool = row.get("totp_enabled");
    if totp_enabled {
        // The right password is not a failure (the attempt is released on drop), but it is not a
        // sign-in either: the account throttle is left as it is until the factor succeeds.
        drop(attempt);
        let secret: Option<String> = row.get("totp_secret");
        return Ok(LoginOutcome::SecondFactor(PendingFactor {
            user_id: id,
            session_version: row.get("session_version"),
            factor: super::pending_login::factor_digest(secret.as_deref()),
        }));
    }

    let role = parse_role(row.get("role")).ok_or_else(ApiError::internal)?;
    let policy = query_opt(state, "SELECT require_2fa_admin, require_2fa_user FROM app_settings LIMIT 1", &[]).await?;
    let (require_admin, require_user) = policy.map(|p| (p.get::<_, bool>(0), p.get::<_, bool>(1))).unwrap_or((false, false));
    throttle.clear_account(&account_key);
    Ok(LoginOutcome::Success(AuthorizedUser {
        id,
        email: row.get("email"),
        role,
        session_version: row.get("session_version"),
        totp_enabled,
        requires_2fa: if role == Role::Admin { require_admin } else { require_user },
        mfa_verified: false,
        external_login: false,
    }))
}

/// The stored second factor of a user, as read with the rest of its row.
pub struct StoredFactor {
    pub totp_secret: Option<String>,
    pub backup_codes: Option<Value>,
}

/// Which kind of second factor a submission may be. The sign-in factor screens take one kind each
/// (`/login/2fa` the authenticator, `/login/2fa/backup` a backup code); `Either` is the single
/// field that takes both (`/account/2fa`, and a sign-in page from before the split).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactorKind {
    Either,
    Authenticator,
    Backup,
}

/// One use of the second factor, the same for a password sign-in and `account.verifyTwoFactor`:
/// a 6-digit TOTP code whose time step is recorded (RFC 6238 §5.2 — the same or an earlier step
/// is a replay), or a backup code removed by compare-and-swap (a concurrent use of it loses).
/// A code of a kind that `kind` excludes is refused before it is looked at: never recorded, never
/// spent. `code` is already ECMAScript-trimmed.
pub async fn accept_second_factor(state: &AppState, user_id: &str, factor: StoredFactor, kind: FactorKind, code: &str, now_ms: i64) -> Result<bool, ApiError> {
    if kind != FactorKind::Backup && code.len() == totp::DIGITS && code.bytes().all(|b| b.is_ascii_digit()) {
        let secret = state.secrets.open_maybe_plain(factor.totp_secret.as_deref());
        let Some(matched) = secret.and_then(|s| totp::verify(&s, code, now_ms.max(0) as u64)) else { return Ok(false) };
        let recorded = execute(
            state,
            "UPDATE users SET totp_last_step = $2 WHERE id = $1 AND (totp_last_step IS NULL OR totp_last_step < $2)",
            &[&user_id, &(matched.step as i64)],
        )
        .await?;
        return Ok(recorded == 1);
    }
    if kind != FactorKind::Authenticator && totp::looks_like_backup_code(code) {
        return consume_backup_code(state, user_id, factor.backup_codes, code).await;
    }
    Ok(false)
}

/// Finds the code among the stored hashes (bcrypt, on the blocking pool, holding no connection)
/// and removes it only if the stored array is still exactly what was read — a concurrent use of
/// the same code loses.
async fn consume_backup_code(state: &AppState, user_id: &str, stored: Option<Value>, code: &str) -> Result<bool, ApiError> {
    let Some(Value::Array(items)) = stored else { return Ok(false) };
    let hashes: Vec<String> = items.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
    let code = code.to_owned();
    let search = hashes.clone();
    let found = state
        .passwords
        .run(move || totp::find_backup_code(&code, &search))
        .await
        .map_err(|_| ApiError::internal())?;
    let Some(index) = found else { return Ok(false) };
    let remaining: Vec<&String> = hashes.iter().enumerate().filter(|(i, _)| *i != index).map(|(_, h)| h).collect();
    let changed = execute(
        state,
        "UPDATE users SET backup_codes = $2 WHERE id = $1 AND backup_codes = $3",
        &[&user_id, &json!(remaining), &Value::Array(items)],
    )
    .await?;
    Ok(changed == 1)
}

type Params<'a> = &'a [&'a (dyn tokio_postgres::types::ToSql + Sync)];

/// One statement on a freshly borrowed connection, returned to the pool before this resolves.
async fn query_opt(state: &AppState, sql: &str, params: Params<'_>) -> Result<Option<tokio_postgres::Row>, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    client.query_opt(sql, params).await.map_err(|_| db_unavailable())
}

async fn execute(state: &AppState, sql: &str, params: Params<'_>) -> Result<u64, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    client.execute(sql, params).await.map_err(|_| db_unavailable())
}

fn db_unavailable() -> ApiError {
    super::principal::db_unavailable()
}
