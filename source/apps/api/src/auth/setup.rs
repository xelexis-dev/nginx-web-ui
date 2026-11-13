//! First-run setup: create the very first admin (`apps/web/app/setup/actions.ts`).
//!
//! Authorization is the first-run state itself — permitted only while `users` is empty. TS
//! checked emptiness and inserted in two steps, so two concurrent submits with different emails
//! could both create an admin. Here the check and the insert run in one transaction serialized by
//! a transaction-scoped advisory lock, so exactly one first admin can ever be created — by the web
//! form or by the boot-time env seed ([`seed_admin`]), which takes the same lock.

use std::collections::BTreeMap;

use crate::error::ApiError;
use axum::http::StatusCode;
use crate::request_context::RequestContext;
use crate::state::AppState;

#[derive(Clone, Copy, Debug)]
pub struct SetupInput<'a> {
    pub email: &'a str,
    pub password: &'a str,
    pub confirm_password: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupOutcome {
    Created { user_id: String },
    AlreadyInitialized,
    Invalid(BTreeMap<String, String>),
}

pub const ALREADY_INITIALIZED: &str = "이미 관리자 계정이 존재합니다. 로그인해 주세요.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedOutcome {
    Seeded,
    /// A user already exists, or the operator did not give both values.
    Skipped,
}

/// Boot-time seed from `ADMIN_EMAIL`/`ADMIN_PASSWORD` (`packages/auth/src/seed.ts`): only when
/// both are non-empty and no user exists; idempotent on every boot. Canonicalise the email like
/// setup/create; preserve the password as given without applying policy. The caller passes the values —
/// no environment is read here.
pub async fn seed_admin(state: &AppState, email: Option<&str>, password: Option<&str>) -> Result<SeedOutcome, ApiError> {
    seed_admin_with(&state.db, &state.passwords, email, password).await
}
pub(crate) async fn seed_admin_with(pool: &deadpool_postgres::Pool, passwords: &crate::auth::password::PasswordHasher, email: Option<&str>, password: Option<&str>) -> Result<SeedOutcome, ApiError> {
    if !is_first_run_with_pool(pool).await? {
        return Ok(SeedOutcome::Skipped);
    }
    let (Some(email), Some(password)) = (email.filter(|e| !e.is_empty()), password.filter(|p| !p.is_empty())) else {
        return Ok(SeedOutcome::Skipped);
    };
    let email = crate::crypto::js_trim(email).to_lowercase();
    if email.is_empty() {
        return Ok(SeedOutcome::Skipped);
    }
    // bcrypt before the lock (and without a connection); the decisive check is under the lock.
    let password_hash = passwords.hash(password.to_owned()).await.map_err(|_| ApiError::internal())?;
    let mut client = pool.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(|_| db_unavailable())?;
    tx.execute("SELECT pg_advisory_xact_lock(hashtext($1))", &[&SETUP_LOCK_KEY]).await.map_err(|_| db_unavailable())?;
    let first: bool = tx
        .query_one("SELECT NOT EXISTS (SELECT 1 FROM users)", &[])
        .await
        .map_err(|_| db_unavailable())?
        .get(0);
    if !first {
        return Ok(SeedOutcome::Skipped);
    }
    tx.execute(
        "INSERT INTO users (id, email, password_hash, role) VALUES ($1, $2, $3, 'admin')",
        &[&uuid::Uuid::new_v4().to_string(), &email, &password_hash],
    )
    .await
    .map_err(|_| db_unavailable())?;
    tx.commit().await.map_err(|_| db_unavailable())?;
    Ok(SeedOutcome::Seeded)
}

/// Serializes first-admin creation across processes (transaction-scoped; released on commit).
const SETUP_LOCK_KEY: &str = "nginx-web-ui:first-admin-setup";

fn db_unavailable() -> ApiError {
    super::principal::db_unavailable()
}

pub async fn is_first_run(state: &AppState) -> Result<bool, ApiError> {
    is_first_run_with_pool(&state.db).await
}
async fn is_first_run_with_pool(pool: &deadpool_postgres::Pool) -> Result<bool, ApiError> {
    let client = pool.get().await.map_err(|_| db_unavailable())?;
    let row = client
        .query_one("SELECT NOT EXISTS (SELECT 1 FROM users)", &[])
        .await
        .map_err(|_| db_unavailable())?;
    Ok(row.get(0))
}

/// `/^[^\s@]+@[^\s@]+\.[^\s@]+$/` with the ECMAScript `\s` set.
pub fn is_valid_email(email: &str) -> bool {
    if email.chars().any(crate::crypto::is_ecmascript_whitespace) {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else { return false };
    // The domain holds a '.' with at least one character on each side (the regex backtracks
    // to any such dot).
    !local.is_empty()
        && !domain.contains('@')
        && domain.char_indices().any(|(i, c)| c == '.' && i > 0 && i + 1 < domain.len())
}

pub async fn create_first_admin(state: &AppState, input: SetupInput<'_>, ctx: &RequestContext) -> Result<SetupOutcome, ApiError> {
    // Cheap early exit before any bcrypt work, as TS; the decisive check is under the lock below.
    if !is_first_run(state).await? {
        return Ok(SetupOutcome::AlreadyInitialized);
    }
    // A DNS name chosen by the request must not authorize first-admin setup via rebinding.
    let allowed_host = ctx.origin_configured || url::Url::parse(&ctx.external_origin).is_ok_and(|url| match url.host() {
        Some(url::Host::Ipv4(_) | url::Host::Ipv6(_)) => true,
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        None => false,
    });
    if !allowed_host {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "setup_host_forbidden", "최초 관리자 설정은 localhost 또는 IP 주소로 접속하거나 AUTH_URL에 관리 화면 주소를 지정한 뒤 진행하세요."));
    }
    let email = crate::crypto::js_trim(input.email).to_lowercase();
    let mut errors = BTreeMap::new();
    if !is_valid_email(&email) {
        errors.insert("email".to_owned(), "올바른 이메일 형식을 입력하세요.".to_owned());
    }
    if let Some(message) = super::password::validate_password(input.password) {
        errors.insert("password".to_owned(), message);
    }
    if input.password != input.confirm_password {
        errors.insert("confirmPassword".to_owned(), "비밀번호가 일치하지 않습니다.".to_owned());
    }
    if !errors.is_empty() {
        return Ok(SetupOutcome::Invalid(errors));
    }

    let password_hash = state.passwords.hash(input.password.to_owned()).await.map_err(|_| ApiError::internal())?;
    let id = uuid::Uuid::new_v4().to_string();
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(|_| db_unavailable())?;
    tx.execute("SELECT pg_advisory_xact_lock(hashtext($1))", &[&SETUP_LOCK_KEY]).await.map_err(|_| db_unavailable())?;
    let first: bool = tx
        .query_one("SELECT NOT EXISTS (SELECT 1 FROM users)", &[])
        .await
        .map_err(|_| db_unavailable())?
        .get(0);
    if !first {
        return Ok(SetupOutcome::AlreadyInitialized);
    }
    let inserted = tx
        .execute(
            "INSERT INTO users (id, email, password_hash, role) VALUES ($1, $2, $3, 'admin')",
            &[&id, &email, &password_hash],
        )
        .await;
    if let Err(error) = inserted {
        // The optional env seed does not take the lock; its row can still collide on email.
        if error.code() == Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION) {
            let mut errors = BTreeMap::new();
            errors.insert("email".to_owned(), format!("이미 사용 중인 이메일입니다: {email}"));
            return Ok(SetupOutcome::Invalid(errors));
        }
        return Err(db_unavailable());
    }
    // Never password material. The new admin is both subject and actor.
    let entry = super::audit::AuditEntry {
        actor_id: Some(id.clone()),
        action: "admin:first-setup".into(),
        target: Some(id.clone()),
        after: Some(serde_json::json!({"email": email, "role": "admin"})),
        ip: ctx.ip.clone(),
        ..Default::default()
    };
    super::audit::write(&*tx, &entry).await.map_err(|_| db_unavailable())?;
    tx.commit().await.map_err(|_| db_unavailable())?;
    Ok(SetupOutcome::Created { user_id: id })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_shape_matches_the_ts_pattern() {
        for ok in ["a@b.c", "admin@example.test", "a.b@c.d.e", "a@b.c.", "한글@예시.한국"] {
            assert!(is_valid_email(ok), "{ok}");
        }
        for bad in ["", "a", "a@b", "a@.b", "a@b.", "@b.c", "a@@b.c", "a b@c.d", "a@b\u{3000}.c", "a@b.c\u{feff}"] {
            assert!(!is_valid_email(bad), "{bad:?}");
        }
    }
}
