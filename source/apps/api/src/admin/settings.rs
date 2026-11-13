//! General settings (`apps/web/app/admin/settings/general/**`): the app's public base URL (absolute
//! links in notifications — background sweeps have no request to learn the host from) and the
//! metrics retention / excluded hosts.
//!
//! Change vs TS (stricter): `metricsRetentionDays` accepts decimal integers only (TS `Number()`
//! also took `1e2`).

use serde::Serialize;
use serde_json::{Value, json};

use super::{AdminResult, field, unavailable};
use crate::auth::audit::{self, AuditEntry};
use crate::auth::principal::SessionPrincipal;
use crate::contracts::FormInput;
use crate::crypto::js_trim;
use crate::request_context::RequestContext;
use crate::state::AppState;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralSettings {
    pub public_base_url: Option<String>,
    pub metrics_retention_days: Option<i32>,
    /// Newline-separated domains.
    pub metrics_excluded_hosts: Option<String>,
}

/// TS `parseWebhookUrl`: WHATWG URL, http or https only.
pub fn parse_http_url(raw: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(raw).map_err(|_| "URL 형식이 올바르지 않습니다.".to_owned())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("http:// 또는 https:// 주소만 사용할 수 있습니다.".to_owned());
    }
    Ok(url)
}

pub async fn view(state: &AppState) -> AdminResult<GeneralSettings> {
    let row = state
        .db
        .get()
        .await
        .map_err(unavailable)?
        .query_opt("SELECT public_base_url, metrics_retention_days, metrics_excluded_hosts FROM app_settings WHERE id = 'current'", &[])
        .await
        .map_err(unavailable)?;
    Ok(match row {
        Some(r) => GeneralSettings { public_base_url: r.get(0), metrics_retention_days: r.get(1), metrics_excluded_hosts: r.get(2) },
        None => GeneralSettings { public_base_url: None, metrics_retention_days: None, metrics_excluded_hosts: None },
    })
}

/// Upserts the singleton row's columns and audits, in one transaction.
async fn save(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, sql: &str, params: &[&(dyn tokio_postgres::types::ToSql + Sync)], action: &str, after: Value) -> AdminResult<()> {
    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    tx.execute(sql, params).await.map_err(unavailable)?;
    let entry = AuditEntry { actor_id: Some(actor.user_id.clone()), action: action.to_owned(), after: Some(after), ip: ctx.ip.clone(), ..AuditEntry::default() };
    audit::write(&*tx, &entry).await.map_err(unavailable)?;
    tx.commit().await.map_err(unavailable)
}

/// Returns the screen message.
pub async fn update_public_base_url(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, form: &FormInput) -> AdminResult<String> {
    let raw = js_trim(form.get("publicBaseUrl").unwrap_or(""));
    if !raw.is_empty() {
        // Same rule as webhook URLs: this too is an address a browser will open.
        parse_http_url(raw).map_err(|message| field("publicBaseUrl", message))?;
    }
    let value = (!raw.is_empty()).then(|| raw.trim_end_matches('/').to_owned());
    save(
        state, actor, ctx,
        "INSERT INTO app_settings (id, public_base_url, updated_at, actor_id) VALUES ('current', $1, now(), $2) \
         ON CONFLICT (id) DO UPDATE SET public_base_url = EXCLUDED.public_base_url, updated_at = now(), actor_id = EXCLUDED.actor_id",
        &[&value, &actor.user_id],
        "settings:public-base-url",
        json!({"publicBaseUrl": value}),
    )
    .await?;
    Ok(if value.is_some() { "앱 주소를 저장했습니다." } else { "앱 주소를 지웠습니다." }.to_owned())
}

/// Returns the screen message.
pub async fn update_metrics(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, form: &FormInput) -> AdminResult<String> {
    let raw_days = js_trim(form.get("metricsRetentionDays").unwrap_or(""));
    let retention_days: Option<i32> = if raw_days.is_empty() {
        None
    } else {
        let days = raw_days.bytes().all(|b| b.is_ascii_digit()).then(|| raw_days.parse::<i32>().ok()).flatten().filter(|d| (1..=3650).contains(d));
        Some(days.ok_or_else(|| field("metricsRetentionDays", "1 이상 3650 이하의 정수여야 합니다(비우면 무제한)."))?)
    };
    let raw_hosts = js_trim(form.get("metricsExcludedHosts").unwrap_or(""));
    let hosts: Vec<String> = raw_hosts
        .split(['\n', ','])
        .map(|s| js_trim(s).to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if let Some(bad) = hosts.iter().find(|h| h.len() > 253 || !h.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() || matches!(b, b'.' | b'_' | b'-'))) {
        return Err(field("metricsExcludedHosts", format!("도메인 형식이 아닙니다: {bad}")));
    }
    let excluded = (!hosts.is_empty()).then(|| hosts.join("\n"));
    save(
        state, actor, ctx,
        "INSERT INTO app_settings (id, metrics_retention_days, metrics_excluded_hosts, updated_at, actor_id) VALUES ('current', $1, $2, now(), $3) \
         ON CONFLICT (id) DO UPDATE SET metrics_retention_days = EXCLUDED.metrics_retention_days, \
         metrics_excluded_hosts = EXCLUDED.metrics_excluded_hosts, updated_at = now(), actor_id = EXCLUDED.actor_id",
        &[&retention_days, &excluded, &actor.user_id],
        "settings:metrics",
        json!({"metricsRetentionDays": retention_days, "metricsExcludedHosts": excluded}),
    )
    .await?;
    Ok(match retention_days {
        None => "보존 기간을 무제한으로 설정했습니다.".to_owned(),
        Some(days) => format!("보존 기간을 {days}일로 설정했습니다."),
    })
}
