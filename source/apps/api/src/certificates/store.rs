//! Durable certificate/job storage; no network side effect occurs in a transaction.
use super::{
    protocol::{create_account_key, resolve_directory_url},
    service::CertificateInput,
};
use crate::{
    auth::{
        audit::{self, AuditEntry},
        principal::db_unavailable,
    },
    contracts::Actor,
    error::ApiError,
    state::AppState,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_postgres::GenericClient;

#[derive(Deserialize)]
pub struct Certificate {
    pub id: String,
    pub domains: Value,
    pub alias: Option<String>,
    pub status: String,
    pub source: String,
    pub staging: bool,
    pub challenge_type: String,
    pub dns_provider: Option<String>,
    pub dns_provider_token_enc: Option<String>,
    pub cert_pem: Option<String>,
    pub key_pem: Option<String>,
    pub dns_order: Option<Value>,
    pub issued_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub failure_count: i32,
    pub issue_log: Value,
    pub created_at: DateTime<Utc>,
}
impl Certificate {
    pub fn names(&self) -> Vec<String> {
        self.domains
            .as_array()
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }
}
pub async fn get(db: &impl GenericClient, id: &str, lock: bool) -> Result<Certificate, ApiError> {
    let sql = if lock {
        "SELECT to_jsonb(c) FROM certificates c WHERE id=$1 FOR UPDATE"
    } else {
        "SELECT to_jsonb(c) FROM certificates c WHERE id=$1"
    };
    let row = db
        .query_opt(sql, &[&id])
        .await
        .map_err(|_| db_unavailable())?
        .ok_or_else(ApiError::not_found)?;
    serde_json::from_value(row.get(0)).map_err(|_| ApiError::internal())
}
pub async fn active(db: &impl GenericClient, id: &str) -> Result<bool, ApiError> {
    Ok(db.query_opt("SELECT id FROM certificate_jobs WHERE certificate_id=$1 AND state IN ('queued','active','cleanup')",&[&id]).await.map_err(|_|db_unavailable())?.is_some())
}
pub async fn audit(
    db: &impl GenericClient,
    actor: &Actor,
    action: &str,
    id: &str,
    after: Value,
) -> Result<(), ApiError> {
    // A queued job can outlive its requesting account. Keep its attribution as
    // a label rather than failing the certificate commit on the audit FK.
    let actor_id = if let Some(id) = &actor.id {
        db.query_opt("SELECT id FROM users WHERE id=$1 FOR KEY SHARE", &[id])
            .await
            .map_err(|_| db_unavailable())?
            .map(|row| row.get::<_, String>(0))
    } else {
        None
    };
    let actor_label = actor.label.clone().or_else(|| {
        if actor_id.is_none() {
            actor.id.as_ref().map(|id| format!("deleted-user:{id}"))
        } else {
            None
        }
    });
    audit::write(
        db,
        &AuditEntry {
            actor_id,
            actor_label,
            ip: actor.ip.clone(),
            action: action.into(),
            target: Some(id.into()),
            after: Some(after),
            ..Default::default()
        },
    )
    .await
    .map_err(|_| db_unavailable())
}
pub async fn account(
    db: &impl GenericClient,
    state: &AppState,
    staging: bool,
    email: &str,
) -> Result<(String, String), ApiError> {
    let directory = resolve_directory_url(staging, state.config.acme_directory_url.as_deref());
    let key = create_account_key().map_err(|_| ApiError::internal())?;
    let sealed = state.secrets.seal(&key).map_err(|_| ApiError::internal())?;
    let row=db.query_one("INSERT INTO acme_accounts(id,directory_url,email,account_key_pem) VALUES($1,$2,$3,$4) ON CONFLICT(directory_url) DO UPDATE SET email=EXCLUDED.email RETURNING account_key_pem",&[&uuid::Uuid::new_v4().to_string(),&directory,&email,&sealed]).await.map_err(|_|db_unavailable())?;
    let stored: String = row.get(0);
    let key = state
        .secrets
        .open_maybe_plain(Some(&stored))
        .ok_or_else(ApiError::internal)?;
    Ok((directory, key))
}
/// Plaintext exists only in memory; the entire snapshot is sealed in payload_enc.
#[derive(Serialize, Deserialize)]
pub struct JobPayload {
    pub directory: String,
    pub account_key: String,
    pub email: String,
    pub domains: Vec<String>,
    pub dns_token: Option<String>,
    pub dns_order: Option<Value>,
}
pub async fn payload(
    db: &impl GenericClient,
    state: &AppState,
    cert: &Certificate,
) -> Result<JobPayload, ApiError> {
    let directory = resolve_directory_url(cert.staging, state.config.acme_directory_url.as_deref());
    let row = db
        .query_opt(
            "SELECT email,account_key_pem FROM acme_accounts WHERE directory_url=$1",
            &[&directory],
        )
        .await
        .map_err(|_| db_unavailable())?
        .ok_or_else(|| super::service::conflict("ACME 계정을 찾을 수 없습니다."))?;
    let stored: String = row.get(1);
    Ok(JobPayload {
        directory,
        account_key: state
            .secrets
            .open_maybe_plain(Some(&stored))
            .ok_or_else(ApiError::internal)?,
        email: row.get(0),
        domains: cert.names(),
        dns_token: state.secrets.open(cert.dns_provider_token_enc.as_deref()),
        dns_order: cert.dns_order.clone(),
    })
}
pub fn job_kind(input: &CertificateInput) -> &'static str {
    if input.challenge_type == "http-01" {
        "http"
    } else if input.dns_provider.as_deref() == Some("cloudflare") {
        "dns_auto"
    } else {
        "dns_begin"
    }
}
pub fn cert_kind(cert: &Certificate) -> &'static str {
    if cert.challenge_type == "http-01" {
        "http"
    } else if cert.dns_provider.as_deref() == Some("cloudflare") {
        "dns_auto"
    } else {
        "dns_begin"
    }
}
pub async fn enqueue(
    db: &impl GenericClient,
    state: &AppState,
    cert: &Certificate,
    kind: &str,
    mode: &str,
    actor: &Actor,
) -> Result<String, ApiError> {
    if active(db, &cert.id).await? {
        return Err(super::service::conflict("발급이 진행 중입니다."));
    }
    let payload = payload(db, state, cert).await?;
    let sealed = state
        .secrets
        .seal(&serde_json::to_string(&payload).map_err(|_| ApiError::internal())?)
        .map_err(|_| ApiError::internal())?;
    let id = uuid::Uuid::new_v4().to_string();
    db.execute("INSERT INTO certificate_jobs(id,certificate_id,kind,mode,payload_enc,actor) VALUES($1,$2,$3,$4,$5,$6)",&[&id,&cert.id,&kind,&mode,&sealed,&json!(actor)]).await.map_err(|e|if e.code()==Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION){super::service::conflict("발급이 진행 중입니다.")}else{db_unavailable()})?;
    Ok(id)
}
