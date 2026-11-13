//! User administration and approval queue (`apps/web/app/admin/users/actions.ts`,
//! `admin/pending/actions.ts`, the list/detail/pending pages).
//!
//! Same rules and messages as TS, with these fixes (no data loss):
//! - **Last active admin**: TS checked the admin count and then acted, so two concurrent
//!   demotions/disables/deletes of the last two admins could leave none. Here the target row and
//!   every approved, enabled admin row are locked in id order (`FOR UPDATE`) in the same transaction as the
//!   change, so the guard and the act are one step.
//! - `session_version = session_version + 1` in SQL (TS wrote `before + 1` from a stale read).
//! - Approve is one conditional `UPDATE … WHERE NOT approved` (TS read, then wrote).
//! - Reject only applies to accounts still awaiting approval. TS disabled any id it was given —
//!   including an active admin or the caller — without revoking sessions or the last-admin guard.
//!
//! Views never contain the password hash; login methods are `hasPassword` + provider names.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};
use tokio_postgres::error::SqlState;

use super::{AdminError, AdminResult, ApiErrorCode, field, unavailable};
use crate::auth::audit::{self, AuditEntry};
use crate::auth::password::validate_password;
use crate::auth::principal::SessionPrincipal;
use crate::auth::rbac::{parse_role, role_str};
use crate::auth::setup::is_valid_email;
use crate::contracts::Role;
use crate::crypto::js_trim;
use crate::request_context::RequestContext;
use crate::state::AppState;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserListItem {
    pub id: String,
    /// The account email, else the first verified linked identity's email (OAuth-only accounts).
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub role: Role,
    pub enabled: bool,
    pub approved: bool,
    pub has_password: bool,
    /// Linked OAuth provider names, in link order.
    pub providers: Vec<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDetail {
    pub id: String,
    pub email: Option<String>,
    pub role: Role,
    pub enabled: bool,
    pub approved: bool,
    pub has_password: bool,
    pub totp_enabled: bool,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingUser {
    pub id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub provider: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Copy, Debug)]
pub struct NewUser<'a> {
    pub email: &'a str,
    pub password: &'a str,
    pub confirm_password: &'a str,
    pub role: &'a str,
}

fn iso(at: chrono::DateTime<chrono::Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn role_of(row: &tokio_postgres::Row) -> AdminResult<Role> {
    parse_role(row.get("role")).ok_or(AdminError::Api(ApiErrorCode::Internal))
}

async fn client(state: &AppState) -> AdminResult<deadpool_postgres::Object> {
    state.db.get().await.map_err(unavailable)
}

pub async fn list(state: &AppState) -> AdminResult<Vec<UserListItem>> {
    let client = client(state).await?;
    let users = client
        .query(
            "SELECT id, email, role::text AS role, enabled, approved, password_hash IS NOT NULL AS has_password, created_at \
             FROM users ORDER BY created_at, id",
            &[],
        )
        .await
        .map_err(unavailable)?;
    let links = client
        .query(
            "SELECT a.user_id, COALESCE(p.name, a.provider_id) AS provider, a.display_name, CASE WHEN a.email_verified THEN a.email END AS email \
             FROM oauth_accounts a LEFT JOIN oauth_providers p ON p.id = a.provider_id \
             ORDER BY a.linked_at, a.provider_id, a.subject",
            &[],
        )
        .await
        .map_err(unavailable)?;
    users
        .iter()
        .map(|u| {
            let id: String = u.get("id");
            let mine: Vec<&tokio_postgres::Row> = links.iter().filter(|l| l.get::<_, &str>("user_id") == id).collect();
            // TS `??=` over the links: the first non-null value wins.
            let first = |column: &str| mine.iter().find_map(|l| l.get::<_, Option<String>>(column));
            Ok(UserListItem {
                email: u.get::<_, Option<String>>("email").or_else(|| first("email")),
                display_name: first("display_name"),
                role: role_of(u)?,
                enabled: u.get("enabled"),
                approved: u.get("approved"),
                has_password: u.get("has_password"),
                providers: mine.iter().map(|l| l.get("provider")).collect(),
                created_at: iso(u.get("created_at")),
                id,
            })
        })
        .collect()
}

pub async fn detail(state: &AppState, id: &str) -> AdminResult<UserDetail> {
    let row = client(state)
        .await?
        .query_opt(
            "SELECT id, email, role::text AS role, enabled, approved, password_hash IS NOT NULL AS has_password, totp_enabled, created_at \
             FROM users WHERE id = $1",
            &[&id],
        )
        .await
        .map_err(unavailable)?
        .ok_or(AdminError::NotFound)?;
    Ok(UserDetail {
        id: row.get("id"),
        email: row.get("email"),
        role: role_of(&row)?,
        enabled: row.get("enabled"),
        approved: row.get("approved"),
        has_password: row.get("has_password"),
        totp_enabled: row.get("totp_enabled"),
        created_at: iso(row.get("created_at")),
    })
}

/// Awaiting approval and not rejected (`approved = false AND enabled`), oldest first.
pub async fn pending(state: &AppState) -> AdminResult<Vec<PendingUser>> {
    let rows = client(state)
        .await?
        .query(
            "SELECT u.id, u.email, u.created_at, l.provider, l.display_name, l.email AS link_email \
             FROM users u LEFT JOIN LATERAL ( \
                 SELECT COALESCE(p.name, a.provider_id) AS provider, a.display_name, CASE WHEN a.email_verified THEN a.email END AS email \
                 FROM oauth_accounts a LEFT JOIN oauth_providers p ON p.id = a.provider_id \
                 WHERE a.user_id = u.id ORDER BY a.linked_at, a.provider_id, a.subject LIMIT 1 \
             ) l ON true \
             WHERE NOT u.approved AND u.enabled ORDER BY u.created_at, u.id",
            &[],
        )
        .await
        .map_err(unavailable)?;
    Ok(rows
        .iter()
        .map(|r| PendingUser {
            id: r.get("id"),
            email: r.get::<_, Option<String>>("email").or_else(|| r.get("link_email")),
            display_name: r.get("display_name"),
            provider: r.get("provider"),
            created_at: iso(r.get("created_at")),
        })
        .collect())
}

fn password_errors(password: &str, confirm_password: &str) -> BTreeMap<String, String> {
    let mut errors = BTreeMap::new();
    if let Some(message) = validate_password(password) {
        errors.insert("password".to_owned(), message);
    }
    if password != confirm_password {
        errors.insert("confirmPassword".to_owned(), "비밀번호가 일치하지 않습니다.".to_owned());
    }
    errors
}

async fn write_audit(tx: &tokio_postgres::Transaction<'_>, actor: &SessionPrincipal, ctx: &RequestContext, action: &str, target: &str, before: Option<Value>, after: Option<Value>) -> AdminResult<()> {
    let entry = AuditEntry {
        actor_id: Some(actor.user_id.clone()),
        action: action.to_owned(),
        target: Some(target.to_owned()),
        before,
        after,
        ip: ctx.ip.clone(),
        ..AuditEntry::default()
    };
    audit::write(tx, &entry).await.map_err(unavailable)
}

/// Returns the new user's id.
pub async fn create(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, input: NewUser<'_>) -> AdminResult<String> {
    let email = js_trim(input.email).to_lowercase();
    let mut errors = password_errors(input.password, input.confirm_password);
    if !is_valid_email(&email) {
        errors.insert("email".to_owned(), "올바른 이메일 형식을 입력하세요.".to_owned());
    }
    let role = parse_role(js_trim(input.role));
    if role.is_none() {
        errors.insert("role".to_owned(), "역할을 선택하세요.".to_owned());
    }
    let Some(role) = role.filter(|_| errors.is_empty()) else { return Err(AdminError::Invalid(errors)) };

    // bcrypt before borrowing a connection.
    let password_hash = state.passwords.hash(input.password.to_owned()).await.map_err(|_| AdminError::Api(ApiErrorCode::Internal))?;
    let id = uuid::Uuid::new_v4().to_string();
    let mut client = client(state).await?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let inserted = tx
        .execute(
            "INSERT INTO users (id, email, password_hash, role) VALUES ($1, $2, $3, $4::text::role)",
            &[&id, &email, &password_hash, &role_str(role)],
        )
        .await;
    if let Err(error) = inserted {
        if error.code() == Some(&SqlState::UNIQUE_VIOLATION) {
            return Err(field("email", format!("이미 사용 중인 이메일입니다: {email}")));
        }
        return Err(unavailable(error));
    }
    // Never password material in the audit row.
    write_audit(&tx, actor, ctx, "user:create", &id, None, Some(json!({"email": email, "role": role_str(role)}))).await?;
    tx.commit().await.map_err(unavailable)?;
    Ok(id)
}

struct Target {
    email: Option<String>,
    role: Role,
    enabled: bool,
    approved: bool,
}

/// Locks the target row and every approved, enabled admin row, in id order (no deadlock between two
/// such transactions), and returns the target plus the enabled-admin count seen under the lock.
async fn lock_with_admins(tx: &tokio_postgres::Transaction<'_>, id: &str) -> AdminResult<(Option<Target>, usize)> {
    let rows = tx
        .query(
            "SELECT id, email, role::text AS role, enabled, approved FROM users \
             WHERE id = $1 OR (role = 'admin' AND enabled AND approved) ORDER BY id FOR UPDATE",
            &[&id],
        )
        .await
        .map_err(unavailable)?;
    let mut target = None;
    let mut admins = 0;
    for row in &rows {
        let role = role_of(row)?;
        let enabled: bool = row.get("enabled");
        let approved: bool = row.get("approved");
        if role == Role::Admin && enabled && approved {
            admins += 1;
        }
        if row.get::<_, &str>("id") == id {
            target = Some(Target { email: row.get("email"), role, enabled, approved });
        }
    }
    Ok((target, admins))
}

fn bad_request() -> AdminError {
    AdminError::Refused("잘못된 요청입니다.".to_owned())
}

pub async fn update_role(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, id: &str, role: &str) -> AdminResult<()> {
    let id = js_trim(id);
    if id.is_empty() {
        return Err(bad_request());
    }
    let role = parse_role(js_trim(role)).ok_or_else(|| field("role", "역할을 선택하세요."))?;
    // Changing one's own role mid-session is refused (complements the self-delete guard).
    if id == actor.user_id {
        return Err(AdminError::Refused("본인 계정의 역할은 변경할 수 없습니다.".to_owned()));
    }
    let mut client = client(state).await?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let (target, admins) = lock_with_admins(&tx, id).await?;
    let before = target.ok_or(AdminError::NotFound)?;
    if before.role == Role::Admin && before.enabled && before.approved && role != Role::Admin && admins <= 1 {
        return Err(AdminError::Refused("마지막 남은 활성 admin의 역할은 변경할 수 없습니다.".to_owned()));
    }
    // New sessionVersion: the target's existing sessions end on their next request.
    tx.execute(
        "UPDATE users SET role = $2::text::role, session_version = session_version + 1 WHERE id = $1",
        &[&id, &role_str(role)],
    )
    .await
    .map_err(unavailable)?;
    write_audit(&tx, actor, ctx, "user:update", id, Some(json!({"role": role_str(before.role)})), Some(json!({"role": role_str(role)}))).await?;
    tx.commit().await.map_err(unavailable)
}

pub async fn reset_password(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, id: &str, password: &str, confirm_password: &str, authenticated_at: Option<i64>) -> AdminResult<()> {
    let id = js_trim(id);
    if id.is_empty() {
        return Err(bad_request());
    }
    if id == actor.user_id && !authenticated_at.is_some_and(|time| {
        let age = (crate::auth::principal::now_unix_ms() / 1000).saturating_sub(time);
        (-15..=600).contains(&age)
    }) {
        return Err(AdminError::Refused("최근 인증이 필요합니다. 다시 로그인해 인증한 뒤 시도하세요.".into()));
    }
    let errors = password_errors(password, confirm_password);
    if !errors.is_empty() {
        return Err(AdminError::Invalid(errors));
    }
    let has_password: bool = client(state)
        .await?
        .query_opt("SELECT password_hash IS NOT NULL FROM users WHERE id = $1", &[&id])
        .await
        .map_err(unavailable)?
        .ok_or(AdminError::NotFound)?
        .get(0);
    // Never give an OAuth-only account a password: that would bypass the provider's control.
    let oauth_only = || AdminError::Refused("이 계정은 OAuth 로그인만 사용합니다 — 비밀번호를 설정할 수 없습니다.".to_owned());
    if !has_password {
        return Err(oauth_only());
    }
    let password_hash = state.passwords.hash(password.to_owned()).await.map_err(|_| AdminError::Api(ApiErrorCode::Internal))?;
    let mut client = client(state).await?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let changed = tx
        .execute(
            "UPDATE users SET password_hash = $2, session_version = session_version + 1 WHERE id = $1 AND password_hash IS NOT NULL",
            &[&id, &password_hash],
        )
        .await
        .map_err(unavailable)?;
    if changed == 0 {
        // Deleted, or became OAuth-only, while bcrypt ran.
        return Err(AdminError::NotFound);
    }
    write_audit(&tx, actor, ctx, "user:reset-password", id, None, Some(json!({"passwordReset": true}))).await?;
    tx.commit().await.map_err(unavailable)
}

pub async fn delete(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, id: &str) -> AdminResult<()> {
    let id = js_trim(id);
    if id.is_empty() {
        return Err(bad_request());
    }
    if id == actor.user_id {
        return Err(AdminError::Refused("본인 계정은 삭제할 수 없습니다.".to_owned()));
    }
    let mut client = client(state).await?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let (target, admins) = lock_with_admins(&tx, id).await?;
    let before = target.ok_or(AdminError::NotFound)?;
    if before.role == Role::Admin && before.enabled && before.approved && admins <= 1 {
        return Err(AdminError::Refused("마지막 남은 활성 admin은 삭제할 수 없습니다.".to_owned()));
    }
    // History keeps its rows (actor_id ON DELETE SET NULL); a future FK without a rule is refused.
    if let Err(error) = tx.execute("DELETE FROM users WHERE id = $1", &[&id]).await {
        if error.code() == Some(&SqlState::FOREIGN_KEY_VIOLATION) {
            return Err(AdminError::Refused("이 사용자와 연결된 데이터가 있어 삭제할 수 없습니다.".to_owned()));
        }
        return Err(unavailable(error));
    }
    write_audit(&tx, actor, ctx, "user:delete", id, Some(json!({"email": before.email, "role": role_str(before.role)})), None).await?;
    tx.commit().await.map_err(unavailable)
}

/// Flips `enabled`; returns the new value. Disabling revokes sessions.
pub async fn toggle(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, id: &str) -> AdminResult<bool> {
    let id = js_trim(id);
    if id.is_empty() {
        return Err(bad_request());
    }
    let mut client = client(state).await?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let (target, admins) = lock_with_admins(&tx, id).await?;
    let before = target.ok_or(AdminError::NotFound)?;
    let enabled = !before.enabled;
    if !enabled {
        if id == actor.user_id {
            return Err(AdminError::Refused("본인 계정은 비활성화할 수 없습니다.".to_owned()));
        }
        if before.role == Role::Admin && before.approved && admins <= 1 {
            return Err(AdminError::Refused("마지막 남은 활성 admin은 비활성화할 수 없습니다.".to_owned()));
        }
    }
    // Enabling keeps the version: tokens from before the disable were already revoked by it.
    let sql = if enabled {
        "UPDATE users SET enabled = true WHERE id = $1"
    } else {
        "UPDATE users SET enabled = false, session_version = session_version + 1 WHERE id = $1"
    };
    tx.execute(sql, &[&id]).await.map_err(unavailable)?;
    let action = if enabled { "user:enable" } else { "user:disable" };
    write_audit(&tx, actor, ctx, action, id, Some(json!({"enabled": before.enabled})), Some(json!({"enabled": enabled}))).await?;
    tx.commit().await.map_err(unavailable)?;
    Ok(enabled)
}

/// Approves a pending account with `role` (anything else becomes viewer, as TS). An account that
/// is already approved or gone is left alone — the approval screen is not a role editor.
/// No sessionVersion bump: an unapproved account never had a session.
pub async fn approve(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, id: &str, role: &str) -> AdminResult<()> {
    let id = js_trim(id);
    if id.is_empty() {
        return Ok(());
    }
    let role = parse_role(js_trim(role)).unwrap_or(Role::Viewer);
    let mut client = client(state).await?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let Some(row) = tx
        .query_opt("SELECT role::text AS role FROM users WHERE id = $1 AND NOT approved FOR UPDATE", &[&id])
        .await
        .map_err(unavailable)?
    else {
        return Ok(());
    };
    let before = role_of(&row)?;
    tx.execute("UPDATE users SET approved = true, role = $2::text::role WHERE id = $1", &[&id, &role_str(role)])
        .await
        .map_err(unavailable)?;
    write_audit(
        &tx, actor, ctx, "user:approve", id,
        Some(json!({"approved": false, "role": role_str(before)})),
        Some(json!({"approved": true, "role": role_str(role)})),
    )
    .await?;
    tx.commit().await.map_err(unavailable)
}

/// Rejects a pending account by disabling it (kept, not deleted, so a returning person does not
/// get a fresh account). Anything not awaiting approval is left alone.
pub async fn reject(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, id: &str) -> AdminResult<()> {
    let id = js_trim(id);
    if id.is_empty() {
        return Ok(());
    }
    let mut client = client(state).await?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let Some(row) = tx
        .query_opt("SELECT enabled FROM users WHERE id = $1 AND NOT approved FOR UPDATE", &[&id])
        .await
        .map_err(unavailable)?
    else {
        return Ok(());
    };
    let before: bool = row.get(0);
    tx.execute("UPDATE users SET enabled = false WHERE id = $1", &[&id]).await.map_err(unavailable)?;
    write_audit(&tx, actor, ctx, "user:reject", id, Some(json!({"enabled": before})), Some(json!({"enabled": false}))).await?;
    tx.commit().await.map_err(unavailable)
}
