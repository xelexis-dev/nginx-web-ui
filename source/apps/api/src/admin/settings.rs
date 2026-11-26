//! General settings (`apps/web/app/admin/settings/general/**`): the app's public base URL (absolute
//! links in notifications — background sweeps have no request to learn the host from) and the
//! metrics retention / excluded hosts.
//!
//! Change vs TS (stricter): `metricsRetentionDays` accepts decimal integers only (TS `Number()`
//! also took `1e2`).

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};

use super::{AdminError, AdminResult, field, unavailable};
use crate::auth::audit::{self, AuditEntry};
use crate::auth::principal::{SessionPrincipal, UserPrincipal};
use crate::contracts::FormInput;
use crate::crypto::js_trim;
use crate::request_context::RequestContext;
use crate::retention::{self, Collection};
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
async fn save(state: &AppState, actor: &crate::contracts::Actor, ctx: &RequestContext, sql: &str, params: &[&(dyn tokio_postgres::types::ToSql + Sync)], action: &str, after: Value) -> AdminResult<()> {
    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    tx.execute(sql, params).await.map_err(unavailable)?;
    let entry = AuditEntry { actor_id: actor.id.clone(), action: action.to_owned(), after: Some(after), ip: ctx.ip.clone(), via: actor.via.clone(), ..AuditEntry::default() };
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
        state, &actor.actor(ctx), ctx,
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
pub async fn update_metrics(state: &AppState, actor: &UserPrincipal, ctx: &RequestContext, form: &FormInput) -> AdminResult<String> {
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
        state, &actor.actor(ctx), ctx,
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


fn retention_days(collection: Collection, raw: &str) -> AdminResult<Option<i32>> {
    let raw = js_trim(raw);
    if raw.is_empty() {
        return Ok(None);
    }
    let days = raw.bytes().all(|b| b.is_ascii_digit())
        .then(|| raw.parse::<i32>().ok()).flatten()
        .filter(|d| (collection.minimum()..=3650).contains(d));
    days.map(Some).ok_or_else(|| field(collection.field(), format!(
        "{} 보존 기간은 {} 이상 3650 이하의 정수여야 합니다(비우면 무제한).",
        collection.label(), collection.minimum(),
    )))
}

pub async fn retention_view(state: &AppState) -> AdminResult<Value> {
    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.build_transaction()
        .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
        .read_only(true).start().await.map_err(unavailable)?;
    let mut view = json!({});
    for collection in Collection::ALL {
        let table = collection.table();
        let policy = collection.policy();
        let sql = format!(
            "SELECT s.{policy}, (SELECT count(*) FROM {table}), (SELECT min(created_at) FROM {table}),
                 r.last_cleanup_at, r.last_cleanup_count, r.next_run_at, r.failure_at, r.failure_message, r.failure_count,
                 r.first_run_pending, r.run_started_at > now()-interval '5 minutes', r.last_run_at, now()-make_interval(days=>s.{policy})
             FROM retention_state r LEFT JOIN app_settings s ON s.id='current' WHERE r.collection=$1"
        );
        let row = tx.query_one(&sql, &[&collection.key()]).await.map_err(unavailable)?;
        let days: Option<i32> = row.get(0);
        let cutoff: Option<DateTime<Utc>> = row.get(12);
        let remaining = match cutoff {
            Some(at) => collection.count(&*tx, at).await.map_err(unavailable)?,
            None => 0,
        };
        let failure_at: Option<DateTime<Utc>> = row.get(6);
        let last_run_at: Option<DateTime<Utc>> = row.get(11);
        let status = if days.is_none() { "off" }
            else if row.get::<_, bool>(9) { "waiting" }
            else if row.get::<_, Option<bool>>(10) == Some(true) { "running" }
            else if failure_at > last_run_at { "failed" }
            else if remaining > 0 && last_run_at.is_some() { "running" }
            else { "done" };
        view[collection.key()] = json!({
            "days": days, "count": row.get::<_, i64>(1), "oldestAt": row.get::<_, Option<DateTime<Utc>>>(2),
            "lastCleanup": row.get::<_, Option<DateTime<Utc>>>(3).map(|at| retention::Cleanup { at, deleted: row.get(4) }),
            "nextRunAt": row.get::<_, Option<DateTime<Utc>>>(5), "remaining": remaining,
            "lastFailure": failure_at.map(|at| json!({"at": at, "message": row.get::<_, String>(7), "deleted": row.get::<_, i64>(8)})),
            "status": status,
        });
    }
    Ok(view)
}

async fn retention_policies(client: &impl tokio_postgres::GenericClient) -> AdminResult<[Option<i32>; 2]> {
    Ok(client.query_opt(
        "SELECT audit_retention_days, history_retention_days FROM app_settings WHERE id='current'",
        &[],
    ).await.map_err(unavailable)?.map_or([None, None], |row| [row.get(0), row.get(1)]))
}

pub async fn estimate_retention(state: &AppState, query: &std::collections::BTreeMap<String, String>) -> AdminResult<Value> {
    let days = Collection::ALL.map(|c| retention_days(c, query.get(c.field()).map_or("", String::as_str)));
    let [audit_days, history_days] = days;
    let proposed = [audit_days?, history_days?];
    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.build_transaction()
        .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
        .read_only(true).start().await.map_err(unavailable)?;
    let saved = retention_policies(&*tx).await?;
    let settings_change = Collection::ALL.into_iter().zip(proposed).zip(saved)
        .any(|((collection, proposed), saved)| query.contains_key(collection.field()) && proposed != saved);
    let mut estimates = json!({});
    for (collection, days) in Collection::ALL.into_iter().zip(proposed) {
        estimates[collection.key()] = match days {
            Some(days) => retention::estimate(&*tx, collection, days, settings_change).await.map_err(unavailable)?,
            None => Value::Null,
        };
    }
    Ok(estimates)
}

pub async fn update_retention(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, form: &FormInput) -> AdminResult<Value> {
    let parse = |collection: Collection| {
        retention_days(collection, form.get(collection.field())
            .ok_or_else(|| field(collection.field(), "보존 기간 값을 입력하세요(비우면 무제한)."))?)
    };
    let days = [parse(Collection::Audit)?, parse(Collection::History)?];
    let expected = |collection, name| {
        retention_days(collection, form.get(name)
            .ok_or_else(|| field(name, "저장하기 전에 현재 보존 기간을 확인하세요."))?)
    };
    let expected = [
        expected(Collection::Audit, "expectedAuditRetentionDays")?,
        expected(Collection::History, "expectedHistoryRetentionDays")?,
    ];
    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    tx.batch_execute("SET LOCAL statement_timeout = '5s'").await.map_err(unavailable)?;
    tx.query_one(&format!("SELECT pg_advisory_xact_lock({})", retention::LOCK), &[]).await.map_err(unavailable)?;
    let prior = retention_policies(&*tx).await?;
    if expected != prior {
        return Err(AdminError::Refused("보존 기간이 변경되었습니다. 화면을 새로 고친 뒤 다시 저장하세요.".into()));
    }
    let mut schedules_cleanup = false;
    if prior != days {
        tx.execute(
            "INSERT INTO app_settings (id, audit_retention_days, history_retention_days, updated_at, actor_id)
             VALUES ('current', $1, $2, now(), $3)
             ON CONFLICT (id) DO UPDATE SET audit_retention_days=EXCLUDED.audit_retention_days,
                 history_retention_days=EXCLUDED.history_retention_days, updated_at=now(), actor_id=EXCLUDED.actor_id",
            &[&days[0], &days[1], &actor.user_id],
        ).await.map_err(unavailable)?;
        for ((collection, old), new) in Collection::ALL.into_iter().zip(prior).zip(days) {
            if old == new { continue; }
            let schedule = new.is_some_and(|d| old.is_none_or(|old| d < old));
            schedules_cleanup |= schedule;
            tx.execute(
                "UPDATE retention_state SET
                    next_run_at=CASE WHEN $2 THEN NULL WHEN $3 THEN now()+interval '10 minutes' ELSE next_run_at END,
                    first_run_pending=CASE WHEN $2 THEN false WHEN $3 THEN true ELSE first_run_pending END,
                    run_started_at=CASE WHEN $2 OR $3 THEN NULL ELSE run_started_at END
                 WHERE collection=$1",
                &[&collection.key(), &new.is_none(), &schedule],
            ).await.map_err(unavailable)?;
        }
        let actor = actor.actor(ctx);
        let values = |days: [Option<i32>; 2]| json!({"auditRetentionDays":days[0],"historyRetentionDays":days[1]});
        audit::write(&*tx, &AuditEntry {
            actor_id: actor.id,
            action: "settings:retention".into(),
            before: Some(values(prior)),
            after: Some(values(days)),
            ip: ctx.ip.clone(),
            via: actor.via,
            ..AuditEntry::default()
        }).await.map_err(unavailable)?;
    }
    tx.commit().await.map_err(unavailable)?;
    Ok(json!({"schedulesCleanup": schedules_cleanup}))
}
