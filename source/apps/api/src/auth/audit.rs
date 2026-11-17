//! `audit_logs` writer, the shape of `apps/web/lib/audit.ts` `writeAudit`.
//!
//! Lives under `auth` until the root decides where the shared writer goes; other modules may
//! import it. Never put password material, plaintext tokens or secrets in `before`/`after`.

use serde_json::Value;

#[derive(Clone, Debug, Default)]
pub struct AuditEntry {
    pub actor_id: Option<String>,
    /// Stored local actor email, or an external label: `gateway:<name>`, `token:<id>`.
    pub actor_label: Option<String>,
    pub action: String,
    pub target: Option<String>,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub ip: Option<String>,
    pub via: Option<crate::contracts::Via>,
}

pub async fn write(client: &impl tokio_postgres::GenericClient, entry: &AuditEntry) -> Result<(), tokio_postgres::Error> {
    client
        .execute(
            "INSERT INTO audit_logs (id, actor_id, actor_label, action, target, before, after, ip, via) \
             VALUES ($1, $2, COALESCE($3, (SELECT email FROM users WHERE id=$2)), $4, $5, $6, $7, $8, $9)",
            &[
                &uuid::Uuid::new_v4().to_string(),
                &entry.actor_id,
                &entry.actor_label,
                &entry.action,
                &entry.target,
                &entry.before,
                &entry.after,
                &entry.ip,
                &entry.via.as_ref().map(|via| serde_json::json!(via)),
            ],
        )
        .await
        .map(|_| ())
}
