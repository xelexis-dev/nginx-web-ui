//! 2FA enforcement policy and connected gateways (`apps/web/app/admin/security/**`).
//!
//! Policy changes end the sessions of the affected group (`session_version + 1`) so the new
//! `requires2fa` claim applies on their next request: admins for the admin switch, everyone else
//! for the user switch, OAuth-linked accounts for the OAuth switch. TS saved the policy and bumped
//! the versions as separate statements (a failure in between left sessions with a stale claim)
//! and read "before" without a lock; here it is one transaction holding the settings row.
//!
//! Gateway revocation stamps `revoked_at` (the row stays: who connected when is this node's
//! security record) with one conditional UPDATE, so a double submit writes one audit row.

use serde::Serialize;
use serde_json::{Value, json};

use super::{AdminError, AdminResult, ApiErrorCode, unavailable};
use crate::auth::audit::{self, AuditEntry};
use crate::auth::principal::SessionPrincipal;
use crate::auth::rbac::parse_role;
use crate::contracts::Role;
use crate::crypto::js_trim;
use crate::request_context::RequestContext;
use crate::state::AppState;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TwoFactorPolicy {
    pub require2fa_admin: bool,
    pub require2fa_user: bool,
    pub require2fa_oauth: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectedGateway {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub last_used_ip: Option<String>,
    pub revoked_at: Option<String>,
    pub legacy_orphaned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityView {
    pub policy: TwoFactorPolicy,
    /// Newest first; never the token hash.
    pub gateways: Vec<ConnectedGateway>,
}

type Utc = chrono::DateTime<chrono::Utc>;

fn iso(at: Utc) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn policy_json(policy: &TwoFactorPolicy) -> Value {
    json!({"require2faAdmin": policy.require2fa_admin, "require2faUser": policy.require2fa_user, "require2faOauth": policy.require2fa_oauth})
}

pub async fn view(state: &AppState) -> AdminResult<SecurityView> {
    let client = state.db.get().await.map_err(unavailable)?;
    let policy = client
        .query_opt("SELECT require_2fa_admin, require_2fa_user, require_2fa_oauth FROM app_settings WHERE id = 'current'", &[])
        .await
        .map_err(unavailable)?
        .map(|r| TwoFactorPolicy { require2fa_admin: r.get(0), require2fa_user: r.get(1), require2fa_oauth: r.get(2) })
        .unwrap_or(TwoFactorPolicy { require2fa_admin: false, require2fa_user: false, require2fa_oauth: false });
    let rows = client
        .query(
            "SELECT id, name, role::text AS role, created_at, last_used_at, last_used_ip, revoked_at, \
             legacy_issuer AND NOT EXISTS (SELECT 1 FROM users WHERE id = gateway_tokens.created_by_user_id \
             AND enabled AND approved AND role = 'admin') AS legacy_orphaned \
             FROM gateway_tokens ORDER BY created_at DESC, id",
            &[],
        )
        .await
        .map_err(unavailable)?;
    let gateways = rows
        .iter()
        .map(|r| {
            Ok(ConnectedGateway {
                id: r.get("id"),
                name: r.get("name"),
                role: parse_role(r.get("role")).ok_or(AdminError::Api(ApiErrorCode::Internal))?,
                created_at: iso(r.get("created_at")),
                last_used_at: r.get::<_, Option<Utc>>("last_used_at").map(iso),
                last_used_ip: r.get("last_used_ip"),
                revoked_at: r.get::<_, Option<Utc>>("revoked_at").map(iso),
                legacy_orphaned: r.get("legacy_orphaned"),
            })
        })
        .collect::<AdminResult<Vec<_>>>()?;
    Ok(SecurityView { policy, gateways })
}

pub async fn update_policy(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, next: TwoFactorPolicy) -> AdminResult<()> {
    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    // The singleton row, created with the defaults if absent (= TS "no row: nothing enforced"),
    // then held so a concurrent change cannot read the same "before".
    tx.execute("INSERT INTO app_settings (id) VALUES ('current') ON CONFLICT (id) DO NOTHING", &[])
        .await
        .map_err(unavailable)?;
    let row = tx
        .query_one("SELECT require_2fa_admin, require_2fa_user, require_2fa_oauth FROM app_settings WHERE id = 'current' FOR UPDATE", &[])
        .await
        .map_err(unavailable)?;
    let before = TwoFactorPolicy { require2fa_admin: row.get(0), require2fa_user: row.get(1), require2fa_oauth: row.get(2) };
    tx.execute(
        "UPDATE app_settings SET require_2fa_admin = $1, require_2fa_user = $2, require_2fa_oauth = $3, actor_id = $4, updated_at = now() \
         WHERE id = 'current'",
        &[&next.require2fa_admin, &next.require2fa_user, &next.require2fa_oauth, &actor.user_id],
    )
    .await
    .map_err(unavailable)?;
    // Only the groups whose rule changed are signed out; overlapping bumps are harmless
    // (the version only has to change).
    let bumps = [
        (next.require2fa_admin != before.require2fa_admin, "UPDATE users SET session_version = session_version + 1 WHERE role = 'admin'"),
        (next.require2fa_user != before.require2fa_user, "UPDATE users SET session_version = session_version + 1 WHERE role <> 'admin'"),
        (
            next.require2fa_oauth != before.require2fa_oauth,
            "UPDATE users SET session_version = session_version + 1 WHERE id IN (SELECT DISTINCT user_id FROM oauth_accounts)",
        ),
    ];
    for (changed, sql) in bumps {
        if changed {
            tx.execute(sql, &[]).await.map_err(unavailable)?;
        }
    }
    let entry = AuditEntry {
        actor_id: Some(actor.user_id.clone()),
        action: "settings:2fa-policy".into(),
        before: Some(policy_json(&before)),
        after: Some(policy_json(&next)),
        ip: ctx.ip.clone(),
        ..AuditEntry::default()
    };
    audit::write(&*tx, &entry).await.map_err(unavailable)?;
    tx.commit().await.map_err(unavailable)
}

/// Already revoked → nothing to do (no second audit row).
pub async fn revoke_gateway(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, id: &str) -> AdminResult<()> {
    let id = js_trim(id);
    if id.is_empty() {
        return Err(AdminError::Refused("잘못된 요청입니다.".to_owned()));
    }
    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let revoked = tx
        .query_opt(
            "UPDATE gateway_tokens SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL RETURNING name, role::text AS role",
            &[&id],
        )
        .await
        .map_err(unavailable)?;
    let Some(row) = revoked else {
        let exists = tx.query_opt("SELECT 1 FROM gateway_tokens WHERE id = $1", &[&id]).await.map_err(unavailable)?.is_some();
        return if exists { Ok(()) } else { Err(AdminError::NotFound) };
    };
    let entry = AuditEntry {
        actor_id: Some(actor.user_id.clone()),
        action: "node:gateway:revoke".into(),
        target: Some(id.to_owned()),
        before: Some(json!({"name": row.get::<_, String>("name"), "role": row.get::<_, String>("role")})),
        ip: ctx.ip.clone(),
        ..AuditEntry::default()
    };
    audit::write(&*tx, &entry).await.map_err(unavailable)?;
    tx.commit().await.map_err(unavailable)
}
