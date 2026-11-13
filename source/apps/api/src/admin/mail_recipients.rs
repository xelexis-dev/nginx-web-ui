//! Mail recipient administration (`apps/web/app/admin/notifications/mail/**`) — who gets which
//! events by mail. How mail is sent (SMTP) is `admin::smtp` and the notification owner's mailer.
//!
//! | Route | TS source |
//! | --- | --- |
//! | `GET /api/ui/v1/admin/notifications/mail` | `mail/page.tsx` (by address) |
//! | `GET /api/ui/v1/admin/notifications/mail/{id}` | `[id]/page.tsx`, `[id]/edit/page.tsx` (404 if missing) |
//! | `POST /api/ui/v1/forms/admin.mailRecipients.create` | `createMailRecipient` → `/admin/notifications/mail/<id>` |
//! | `POST /api/ui/v1/forms/admin.mailRecipients.update` | `updateMailRecipient` → `/admin/notifications/mail/<id>` |
//! | `POST /api/ui/v1/forms/admin.mailRecipients.delete` | `deleteMailRecipient` → `/admin/notifications/mail` |
//! | `POST /api/ui/v1/forms/admin.mailRecipients.test` | `testMailRecipient` → `{result: {ok, message \| error}}` |
//!
//! Changes vs TS: the duplicate-address check and the write run under one transaction-scoped
//! advisory lock (TS checked, then wrote: two saves could both pass and the address would get
//! every mail twice); update locks the row and a vanished row is 404; database failures are 503;
//! repeated events are stored once; deleting nothing audits nothing.

use std::collections::BTreeMap;

use axum::extract::{Path, Request, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde_json::{Value, json};

use super::routes::{manager, manager_form, redirect, to_json};
use super::{AdminError, AdminResult, TestOutcome, field, unavailable};
use crate::auth::audit::{self, AuditEntry};
use crate::auth::principal::SessionPrincipal;
use crate::auth::setup::is_valid_email;
use crate::contracts::{FormInput, FormResult};
use crate::crypto::js_trim;
use crate::error::ApiError;
use crate::notify;
use crate::request_context::RequestContext;
use crate::state::AppState;

pub const DUPLICATE_ADDRESS: &str = "이미 등록된 주소입니다.";
const LIST_PATH: &str = "/admin/notifications/mail";
/// Serializes the duplicate check with the write, across every writer of the table.
const LOCK_SQL: &str = "SELECT pg_advisory_xact_lock(hashtext('nginx-web-ui:mail-recipients'))";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailRecipientRow {
    pub id: String,
    pub label: Option<String>,
    pub address: String,
    pub enabled: bool,
    pub events: Vec<String>,
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_error: Option<String>,
}

type Fields = BTreeMap<String, String>;

struct Parsed {
    label: Option<String>,
    address: String,
    enabled: bool,
    events: Vec<String>,
}

/// TS `parseForm`: trimmed strings, a blank label is none, `enabled === "true"`.
fn parse_form(form: &FormInput) -> AdminResult<Parsed> {
    let mut errors = Fields::new();
    let text = |key: &str| js_trim(form.get(key).unwrap_or("")).to_owned();
    let address = text("address");
    if address.is_empty() {
        errors.insert("address".into(), "수신 주소를 입력하세요.".into());
    } else if !is_valid_email(&address) {
        errors.insert("address".into(), "주소 형식이 올바르지 않습니다.".into());
    }
    let mut events: Vec<String> = Vec::new();
    for event in form.all("events") {
        if !events.contains(event) {
            events.push(event.clone());
        }
    }
    let unknown: Vec<&str> = events.iter().map(String::as_str).filter(|e| !notify::is_known_event_type(e)).collect();
    if events.is_empty() {
        errors.insert("events".into(), "받을 사건을 하나 이상 고르세요.".into());
    } else if !unknown.is_empty() {
        errors.insert("events".into(), format!("알 수 없는 사건: {}", unknown.join(", ")));
    }
    if !errors.is_empty() {
        return Err(AdminError::Invalid(errors));
    }
    let label = Some(text("label")).filter(|l| !l.is_empty());
    Ok(Parsed { label, address, enabled: text("enabled") == "true", events })
}

fn iso(at: chrono::DateTime<chrono::Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn row_of(row: &tokio_postgres::Row) -> MailRecipientRow {
    type Utc = chrono::DateTime<chrono::Utc>;
    let events = match row.get::<_, Option<Value>>("events") {
        Some(Value::Array(items)) => items.into_iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
        _ => Vec::new(),
    };
    MailRecipientRow {
        id: row.get("id"),
        label: row.get("label"),
        address: row.get("address"),
        enabled: row.get("enabled"),
        events,
        last_attempt_at: row.get::<_, Option<Utc>>("last_attempt_at").map(iso),
        last_success_at: row.get::<_, Option<Utc>>("last_success_at").map(iso),
        last_error: row.get("last_error"),
    }
}

const ROW_SQL: &str = "SELECT id, label, address, enabled, events, last_attempt_at, last_success_at, last_error FROM mail_recipients";

/// The same address twice would get every mail twice; addresses differing only in case are the
/// same (JS `toLowerCase`, compared here rather than with the database collation's `lower`).
async fn is_duplicate(tx: &tokio_postgres::Transaction<'_>, address: &str, except: Option<&str>) -> AdminResult<bool> {
    let wanted = address.to_lowercase();
    let rows = tx.query("SELECT id, address FROM mail_recipients", &[]).await.map_err(unavailable)?;
    Ok(rows.iter().any(|r| Some(r.get::<_, &str>(0)) != except && r.get::<_, &str>(1).to_lowercase() == wanted))
}

async fn audit_in(tx: &tokio_postgres::Transaction<'_>, actor: &SessionPrincipal, ctx: &RequestContext, action: &str, target: &str, before: Option<Value>, after: Option<Value>) -> AdminResult<()> {
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

fn snapshot(parsed: &Parsed) -> Value {
    json!({"label": parsed.label, "address": parsed.address, "enabled": parsed.enabled, "events": parsed.events})
}

pub async fn list(state: &AppState) -> AdminResult<Vec<MailRecipientRow>> {
    let client = state.db.get().await.map_err(unavailable)?;
    let rows = client.query(&format!("{ROW_SQL} ORDER BY address, id"), &[]).await.map_err(unavailable)?;
    Ok(rows.iter().map(row_of).collect())
}

pub async fn detail(state: &AppState, id: &str) -> AdminResult<MailRecipientRow> {
    let client = state.db.get().await.map_err(unavailable)?;
    let row = client.query_opt(&format!("{ROW_SQL} WHERE id = $1"), &[&id]).await.map_err(unavailable)?.ok_or(AdminError::NotFound)?;
    Ok(row_of(&row))
}

pub async fn create(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, form: &FormInput) -> AdminResult<String> {
    let parsed = parse_form(form)?;
    let id = uuid::Uuid::new_v4().to_string();
    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    tx.execute(LOCK_SQL, &[]).await.map_err(unavailable)?;
    if is_duplicate(&tx, &parsed.address, None).await? {
        return Err(field("address", DUPLICATE_ADDRESS));
    }
    tx.execute(
        "INSERT INTO mail_recipients (id, label, address, enabled, events, actor_id) VALUES ($1, $2, $3, $4, $5, $6)",
        &[&id, &parsed.label, &parsed.address, &parsed.enabled, &Value::from(parsed.events.clone()), &actor.user_id],
    )
    .await
    .map_err(unavailable)?;
    audit_in(&tx, actor, ctx, "settings:mail-recipient-create", &id, None, Some(snapshot(&parsed))).await?;
    tx.commit().await.map_err(unavailable)?;
    Ok(id)
}

pub async fn update(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, form: &FormInput) -> AdminResult<String> {
    let id = js_trim(form.get("id").unwrap_or("")).to_owned();
    if id.is_empty() {
        return Err(AdminError::NotFound);
    }
    let parsed = parse_form(form)?;
    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    tx.execute(LOCK_SQL, &[]).await.map_err(unavailable)?;
    let before = tx
        .query_opt("SELECT label, address, enabled, events FROM mail_recipients WHERE id = $1 FOR UPDATE", &[&id])
        .await
        .map_err(unavailable)?
        .ok_or(AdminError::NotFound)?;
    if is_duplicate(&tx, &parsed.address, Some(&id)).await? {
        return Err(field("address", DUPLICATE_ADDRESS));
    }
    tx.execute(
        "UPDATE mail_recipients SET label = $2, address = $3, enabled = $4, events = $5, updated_at = now(), actor_id = $6 WHERE id = $1",
        &[&id, &parsed.label, &parsed.address, &parsed.enabled, &Value::from(parsed.events.clone()), &actor.user_id],
    )
    .await
    .map_err(unavailable)?;
    let before = json!({
        "label": before.get::<_, Option<String>>("label"), "address": before.get::<_, String>("address"),
        "enabled": before.get::<_, bool>("enabled"), "events": before.get::<_, Value>("events"),
    });
    audit_in(&tx, actor, ctx, "settings:mail-recipient-update", &id, Some(before), Some(snapshot(&parsed))).await?;
    tx.commit().await.map_err(unavailable)?;
    Ok(id)
}

/// Missing id → nothing.
pub async fn delete(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, id: &str) -> AdminResult<()> {
    let id = js_trim(id);
    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let Some(before) = tx.query_opt("DELETE FROM mail_recipients WHERE id = $1 RETURNING address, events", &[&id]).await.map_err(unavailable)? else {
        return Ok(());
    };
    let before = json!({"address": before.get::<_, String>("address"), "events": before.get::<_, Value>("events")});
    audit_in(&tx, actor, ctx, "settings:mail-recipient-delete", id, Some(before), None).await?;
    tx.commit().await.map_err(unavailable)
}

pub const TEST_ADDRESS_REQUIRED: &str = "먼저 올바른 수신 주소를 입력한 뒤 다시 시도하세요.";
pub const TEST_MAILER_UNAVAILABLE: &str = "메일을 보낼 수 없습니다 — 관리자 패널 → 사이트 설정 → SMTP 설정을 먼저 마치세요.";

/// `testMailRecipient`: the sample event, as the real notification mail, to the form's address
/// (possibly unsaved) through the saved SMTP settings. Not recorded as the recipient's last
/// attempt — that column shows whether real notifications are going out.
pub async fn test_send(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, form: &FormInput) -> AdminResult<TestOutcome> {
    let address = js_trim(form.get("address").unwrap_or(""));
    if address.is_empty() || !is_valid_email(address) {
        return Err(field("address", TEST_ADDRESS_REQUIRED));
    }
    let config = crate::notifications::smtp::mailer_config(state).await.map_err(|_| AdminError::Refused(TEST_MAILER_UNAVAILABLE.to_owned()))?;
    let base_url: Option<String> = {
        let client = state.db.get().await.map_err(unavailable)?;
        let base_url = client.query_opt("SELECT public_base_url FROM app_settings WHERE id = 'current'", &[]).await.map_err(unavailable)?.and_then(|row| row.get(0));
        let entry = AuditEntry {
            actor_id: Some(actor.user_id.clone()),
            action: "settings:mail-recipient-test".into(),
            target: form.get("id").map(js_trim).filter(|id| !id.is_empty()).map(str::to_owned),
            after: Some(json!({"address": address})),
            ip: ctx.ip.clone(),
            ..AuditEntry::default()
        };
        audit::write(&**client, &entry).await.map_err(unavailable)?;
        base_url
    };
    let event = notify::sample_event(crate::auth::principal::now_unix_ms());
    let content = crate::notifications::mail::mail_content(&event, notify::RenderContext { base_url: base_url.as_deref() });
    let input = crate::notifications::smtp::MailInput { to: address.to_owned(), subject: content.subject, text: content.text, html: Some(content.html), message_id: None };
    let sent = crate::notifications::smtp::send_with_config(&config, &input,&state.config).await;
    Ok(if sent.ok {
        TestOutcome::Sent(format!("{address} 로 테스트 메일을 보냈습니다."))
    } else {
        TestOutcome::Failed(sent.error.unwrap_or_else(|| "발송하지 못했습니다.".into()))
    })
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/ui/v1/admin/notifications/mail", get(list_route))
        .route("/api/ui/v1/admin/notifications/mail/{id}", get(detail_route))
        .route("/api/ui/v1/forms/admin.mailRecipients.create", post(create_route))
        .route("/api/ui/v1/forms/admin.mailRecipients.update", post(update_route))
        .route("/api/ui/v1/forms/admin.mailRecipients.delete", post(delete_route))
        .route("/api/ui/v1/forms/admin.mailRecipients.test", post(test_route))
}

async fn test_route(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    Ok(Json(test_send(&state, &actor, &ctx, &form).await?.into_result()))
}

async fn list_route(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    manager(&state, &headers, &ctx).await?;
    to_json(list(&state).await?)
}

async fn detail_route(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    manager(&state, &headers, &ctx).await?;
    to_json(detail(&state, &id).await?)
}

async fn create_route(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    let id = create(&state, &actor, &ctx, &form).await?;
    Ok(redirect(format!("{LIST_PATH}/{id}")))
}

async fn update_route(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    let id = update(&state, &actor, &ctx, &form).await?;
    Ok(redirect(format!("{LIST_PATH}/{id}")))
}

async fn delete_route(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    delete(&state, &actor, &ctx, form.get("id").unwrap_or("")).await?;
    Ok(redirect(LIST_PATH))
}
