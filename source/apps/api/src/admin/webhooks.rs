//! Webhook target administration (`apps/web/app/admin/notifications/webhooks/**`).
//!
//! | Route | TS source |
//! | --- | --- |
//! | `GET /api/ui/v1/admin/notifications/webhooks` | `webhooks/page.tsx` (by name) |
//! | `GET /api/ui/v1/admin/notifications/webhooks/{id}` | `[id]/page.tsx`, `[id]/edit/page.tsx` (404 if missing) |
//! | `POST /api/ui/v1/forms/admin.webhooks.create` | `createWebhookTarget` → `/admin/notifications/webhooks/<id>` |
//! | `POST /api/ui/v1/forms/admin.webhooks.update` | `updateWebhookTarget` → `/admin/notifications/webhooks/<id>` |
//! | `POST /api/ui/v1/forms/admin.webhooks.delete` | `deleteWebhookTarget` → `/admin/notifications/webhooks` |
//! | `POST /api/ui/v1/forms/admin.webhooks.test` | `testWebhookTarget` → `{result: {ok, message \| error}}` |
//!
//! The URL (often the credential itself) and the headers are sealed and never leave: views carry a
//! masked URL and header names, audit rows carry the host. The edit form sends `__keep__` for the
//! URL and a blank header box to keep what is stored; `clearHeaders=true` removes the headers.
//!
//! Changes vs TS: update runs in one transaction on the locked row and a vanished row is 404 (TS
//! updated nothing and redirected); database failures are 503 rather than the driver message in
//! the banner; repeated events are stored once; the test button refuses a malformed URL or header
//! box instead of silently testing the stored values, and sends the stored headers only to the
//! stored URL's origin (TS forwarded them to any unsaved URL — a way to read a secret the screens
//! never show); test sends are audited before they go out. A failed delivery is an outcome
//! (`result.ok: false`), not an API error.

use std::collections::BTreeMap;

use axum::extract::{Path, Request, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde_json::{Value, json};

use super::routes::{manager, manager_form, redirect, to_json};
use super::{AdminError, AdminResult, ApiErrorCode, TestOutcome, field, unavailable};
use crate::auth::audit::{self, AuditEntry};
use crate::auth::principal::SessionPrincipal;
use crate::contracts::{FormInput, FormResult};
use crate::crypto::js_trim;
use crate::error::ApiError;
use crate::notify::send::SendOptions;
use crate::notify::target::{DeliveryConfig, deliver_with_config, open_target};
use crate::notify::{self, RenderContext, WebhookFormat};
use crate::request_context::RequestContext;
use crate::state::AppState;

pub use crate::notify::target::{HEADERS_DAMAGED, HEADERS_UNREADABLE, URL_UNREADABLE};

/// Sent by the edit form in place of the URL: "keep the stored one".
pub const KEEP_SECRET: &str = "__keep__";
pub const URL_REQUIRED: &str = "webhook URL 을 입력하세요.";
pub const TEST_URL_REQUIRED: &str = "먼저 webhook URL 을 입력한 뒤 다시 시도하세요.";
pub const STORED_HEADERS_OTHER_ORIGIN: &str = "다른 주소로 시험하려면 헤더를 다시 입력하세요. 저장된 헤더는 저장된 주소로만 보냅니다.";

const LIST_PATH: &str = "/admin/notifications/webhooks";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookRow {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub format: String,
    pub events: Vec<String>,
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_status: Option<i32>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookDetail {
    #[serde(flatten)]
    pub target: WebhookRow,
    pub body_template: Option<String>,
    /// `<origin>/<first segment>/...`; `None` when the stored URL cannot be opened.
    pub masked_url: Option<String>,
    pub header_names: Vec<String>,
    /// Why the stored values cannot be opened (lost data key, damage) — shown as a banner.
    pub open_error: Option<String>,
}


type Fields = BTreeMap<String, String>;

enum UrlInput {
    Keep,
    Blank,
    Given(String),
}

struct Parsed {
    name: String,
    enabled: bool,
    format: WebhookFormat,
    events: Vec<String>,
    body_template: Option<String>,
    url: Result<UrlInput, String>,
    /// `None` = keep the stored headers; `Some(empty)` = remove them.
    headers: Option<Vec<(String, String)>>,
}

/// TS `parseForm`. The URL stays undecided: saving and testing read a blank one differently.
fn parse_form(form: &FormInput) -> (Parsed, Fields) {
    let mut errors = Fields::new();
    let text = |key: &str| js_trim(form.get(key).unwrap_or("")).to_owned();

    let name = text("name");
    if name.is_empty() {
        errors.insert("name".into(), "이름을 입력하세요.".into());
    }
    let format = WebhookFormat::parse(&text("format"));

    let mut events: Vec<String> = Vec::new();
    for event in form.all("events") {
        if !events.contains(event) {
            events.push(event.clone());
        }
    }
    let unknown: Vec<&str> = events.iter().map(String::as_str).filter(|e| !notify::is_known_event_type(e)).collect();
    if events.is_empty() {
        errors.insert("events".into(), "보낼 사건을 하나 이상 고르세요.".into());
    } else if !unknown.is_empty() {
        errors.insert("events".into(), format!("알 수 없는 사건: {}", unknown.join(", ")));
    }

    let raw_url = text("url");
    let url = if raw_url == KEEP_SECRET {
        Ok(UrlInput::Keep)
    } else if raw_url.is_empty() {
        Ok(UrlInput::Blank)
    } else {
        notify::parse_webhook_url(&raw_url).map(|_| UrlInput::Given(raw_url))
    };

    // Same rule as the SMTP password: blank keeps, the checkbox removes.
    let raw_headers = form.get("headers").unwrap_or("");
    let headers = if text("clearHeaders") == "true" {
        Some(Vec::new())
    } else if js_trim(raw_headers).is_empty() {
        None
    } else {
        match notify::parse_header_lines(raw_headers) {
            Ok(headers) => Some(headers),
            Err(message) => {
                errors.insert("headers".into(), message);
                None
            }
        }
    };

    let body_template = (format == WebhookFormat::Custom).then(|| form.get("bodyTemplate").unwrap_or("").to_owned());
    if let Some(template) = &body_template {
        if js_trim(template).is_empty() {
            errors.insert("bodyTemplate".into(), "본문 템플릿을 입력하세요.".into());
        } else if let Err(message) = notify::validate_template(template) {
            errors.insert("bodyTemplate".into(), message);
        }
    }

    let parsed = Parsed { name, enabled: text("enabled") == "true", format, events, body_template, url, headers };
    (parsed, errors)
}

/// For saving: `None` = keep; a blank URL is an error, not "keep".
fn url_for_save(parsed: &Parsed, errors: &mut Fields) -> Option<String> {
    match &parsed.url {
        Ok(UrlInput::Keep) => None,
        Ok(UrlInput::Blank) => {
            errors.insert("url".into(), URL_REQUIRED.into());
            None
        }
        Ok(UrlInput::Given(url)) => Some(url.clone()),
        Err(message) => {
            errors.insert("url".into(), message.clone());
            None
        }
    }
}

/// `JSON.stringify(headers)` — entry order kept, which a `serde_json::Map` would sort.
fn headers_json(headers: &[(String, String)]) -> String {
    let entries: Vec<String> = headers.iter().map(|(name, value)| format!("{}:{}", Value::from(name.as_str()), Value::from(value.as_str()))).collect();
    format!("{{{}}}", entries.join(","))
}

/// `new URL(url).host` — never the path or query (a Discord path is the credential).
fn host_of(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
}

fn seal(state: &AppState, plain: &str) -> AdminResult<String> {
    state.secrets.seal(plain).map_err(|_| AdminError::Api(ApiErrorCode::Internal))
}

fn events_of(value: Option<Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items.into_iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
        _ => Vec::new(),
    }
}

fn iso(at: chrono::DateTime<chrono::Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn row_of(row: &tokio_postgres::Row) -> WebhookRow {
    type Utc = chrono::DateTime<chrono::Utc>;
    WebhookRow {
        id: row.get("id"),
        name: row.get("name"),
        enabled: row.get("enabled"),
        format: row.get("format"),
        events: events_of(row.get("events")),
        last_attempt_at: row.get::<_, Option<Utc>>("last_attempt_at").map(iso),
        last_success_at: row.get::<_, Option<Utc>>("last_success_at").map(iso),
        last_status: row.get("last_status"),
        last_error: row.get("last_error"),
    }
}

const ROW_SQL: &str = "SELECT id, name, enabled, format::text AS format, events, last_attempt_at, last_success_at, last_status, last_error, \
    url_enc, headers_enc, body_template FROM webhook_targets";

async fn audit_with(client: &impl tokio_postgres::GenericClient, actor: &crate::contracts::Actor, ctx: &RequestContext, action: &str, target: Option<&str>, before: Option<Value>, after: Option<Value>) -> AdminResult<()> {
    let entry = AuditEntry {
        actor_id: actor.id.clone(),
        action: action.to_owned(),
        target: target.map(str::to_owned),
        before,
        after,
        ip: ctx.ip.clone(), via: actor.via.clone(),
        ..AuditEntry::default()
    };
    audit::write(client, &entry).await.map_err(unavailable)
}

pub async fn list(state: &AppState) -> AdminResult<Vec<WebhookRow>> {
    let client = state.db.get().await.map_err(unavailable)?;
    let rows = client.query(&format!("{ROW_SQL} ORDER BY name, id"), &[]).await.map_err(unavailable)?;
    Ok(rows.iter().map(row_of).collect())
}

pub async fn detail(state: &AppState, id: &str) -> AdminResult<WebhookDetail> {
    let client = state.db.get().await.map_err(unavailable)?;
    let row = client.query_opt(&format!("{ROW_SQL} WHERE id = $1"), &[&id]).await.map_err(unavailable)?.ok_or(AdminError::NotFound)?;
    let target = row_of(&row);
    let body_template: Option<String> = row.get("body_template");
    let opened = open_target(&state.secrets, row.get("url_enc"), row.get("headers_enc"), &target.format, body_template.clone());
    let (masked_url, header_names, open_error) = match opened {
        Ok(config) => (Some(notify::mask_webhook_url(&config.url)), config.headers.into_iter().map(|(name, _)| name).collect(), None),
        Err(error) => (None, Vec::new(), Some(error)),
    };
    Ok(WebhookDetail { target, body_template, masked_url, header_names, open_error })
}

pub async fn create(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, form: &FormInput) -> AdminResult<String> {
    let (parsed, mut errors) = parse_form(form);
    let url = url_for_save(&parsed, &mut errors);
    let Some(url) = url.filter(|_| errors.is_empty()) else {
        if !errors.contains_key("url") && matches!(parsed.url, Ok(UrlInput::Keep)) {
            errors.insert("url".into(), URL_REQUIRED.into());
        }
        return Err(AdminError::Invalid(errors));
    };
    let headers = parsed.headers.clone().unwrap_or_default();
    let url_enc = seal(state, &url)?;
    let headers_enc = (!headers.is_empty()).then(|| seal(state, &headers_json(&headers))).transpose()?;
    let id = uuid::Uuid::new_v4().to_string();
    let events = Value::from(parsed.events.clone());

    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    tx.execute(
        "INSERT INTO webhook_targets (id, name, enabled, format, url_enc, headers_enc, body_template, events, actor_id) \
         VALUES ($1, $2, $3, $4::text::webhook_format, $5, $6, $7, $8, $9)",
        &[&id, &parsed.name, &parsed.enabled, &parsed.format.as_str(), &url_enc, &headers_enc, &parsed.body_template, &events, &actor.user_id],
    )
    .await
    .map_err(unavailable)?;
    // Neither the URL nor header values: the host and format say what was added.
    let after = json!({
        "name": parsed.name, "enabled": parsed.enabled, "format": parsed.format.as_str(),
        "host": host_of(&url), "events": parsed.events, "headerCount": headers.len(),
    });
    audit_with(&*tx, &actor.actor(ctx), ctx, "settings:webhook-create", Some(&id), None, Some(after)).await?;
    tx.commit().await.map_err(unavailable)?;
    Ok(id)
}

pub async fn update(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, form: &FormInput) -> AdminResult<String> {
    let id = js_trim(form.get("id").unwrap_or("")).to_owned();
    if id.is_empty() {
        return Err(AdminError::NotFound);
    }
    let (parsed, mut errors) = parse_form(form);
    let url = url_for_save(&parsed, &mut errors);
    if !errors.is_empty() {
        return Err(AdminError::Invalid(errors));
    }
    let url_enc = url.as_deref().map(|url| seal(state, url)).transpose()?;
    let headers_enc = match &parsed.headers {
        None => None,
        Some(headers) if headers.is_empty() => Some(None),
        Some(headers) => Some(Some(seal(state, &headers_json(headers))?)),
    };
    let events = Value::from(parsed.events.clone());

    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let before = tx
        .query_opt("SELECT name, enabled, format::text AS format, events FROM webhook_targets WHERE id = $1 FOR UPDATE", &[&id])
        .await
        .map_err(unavailable)?
        .ok_or(AdminError::NotFound)?;
    tx.execute(
        "UPDATE webhook_targets SET name = $2, enabled = $3, format = $4::text::webhook_format, body_template = $5, events = $6, \
         updated_at = now(), actor_id = $7, url_enc = COALESCE($8, url_enc), \
         headers_enc = CASE WHEN $9 THEN $10 ELSE headers_enc END WHERE id = $1",
        &[&id, &parsed.name, &parsed.enabled, &parsed.format.as_str(), &parsed.body_template, &events, &actor.user_id, &url_enc, &headers_enc.is_some(), &headers_enc.clone().flatten()],
    )
    .await
    .map_err(unavailable)?;
    let before = json!({
        "name": before.get::<_, String>("name"), "enabled": before.get::<_, bool>("enabled"),
        "format": before.get::<_, String>("format"), "events": before.get::<_, Value>("events"),
    });
    let after = json!({
        "name": parsed.name, "enabled": parsed.enabled, "format": parsed.format.as_str(), "events": parsed.events,
        "urlChanged": url_enc.is_some(), "headersChanged": headers_enc.is_some(),
    });
    audit_with(&*tx, &actor.actor(ctx), ctx, "settings:webhook-update", Some(&id), Some(before), Some(after)).await?;
    tx.commit().await.map_err(unavailable)?;
    Ok(id)
}

/// Missing id → nothing.
pub async fn delete(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, id: &str) -> AdminResult<()> {
    let id = js_trim(id);
    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let Some(before) = tx
        .query_opt("DELETE FROM webhook_targets WHERE id = $1 RETURNING name, format::text AS format, events", &[&id])
        .await
        .map_err(unavailable)?
    else {
        return Ok(());
    };
    let before = json!({"name": before.get::<_, String>("name"), "format": before.get::<_, String>("format"), "events": before.get::<_, Value>("events")});
    audit_with(&*tx, &actor.actor(ctx), ctx, "settings:webhook-delete", Some(id), Some(before), None).await?;
    tx.commit().await.map_err(unavailable)
}

fn same_origin(a: &str, b: &str) -> bool {
    match (url::Url::parse(a), url::Url::parse(b)) {
        (Ok(a), Ok(b)) => a.origin().is_tuple() && a.origin() == b.origin(),
        _ => false,
    }
}

/// Sends the sample event once with the form's values, filling what the form did not resend
/// (`__keep__`, blank URL or headers) from the stored target. Not recorded as the target's last
/// attempt (that is for real notifications), but audited before it goes out.
pub async fn test_send(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, form: &FormInput, options: &SendOptions) -> AdminResult<TestOutcome> {
    let id = js_trim(form.get("id").unwrap_or("")).to_owned();
    let (parsed, errors) = parse_form(form);
    // Only what a send needs; name and events do not matter for a test.
    let mut fields: Fields = errors.into_iter().filter(|(key, _)| key == "headers" || key == "bodyTemplate").collect();
    if let Err(message) = &parsed.url {
        fields.insert("url".into(), message.clone());
    }
    if !fields.is_empty() {
        return Err(AdminError::Invalid(fields));
    }
    let typed_url = match &parsed.url {
        Ok(UrlInput::Given(url)) => Some(url.clone()),
        _ => None,
    };

    let client = state.db.get().await.map_err(unavailable)?;
    let stored = if id.is_empty() {
        None
    } else {
        let row = client
            .query_opt("SELECT url_enc, headers_enc, format::text AS format FROM webhook_targets WHERE id = $1", &[&id])
            .await
            .map_err(unavailable)?
            .ok_or(AdminError::NotFound)?;
        let needed = typed_url.is_none() || parsed.headers.is_none();
        needed.then(|| open_target(&state.secrets, row.get("url_enc"), row.get("headers_enc"), row.get("format"), None)).transpose().map_err(AdminError::Refused)?
    };

    let (url, stored_url) = match (typed_url, &stored) {
        (Some(url), _) => (url, false),
        (None, Some(stored)) => (stored.url.clone(), true),
        (None, None) => return Err(field("url", TEST_URL_REQUIRED)),
    };
    let (headers, stored_headers) = match (&parsed.headers, &stored) {
        (Some(headers), _) => (headers.clone(), false),
        (None, Some(stored)) if stored.headers.is_empty() => (Vec::new(), false),
        (None, Some(stored)) if same_origin(&stored.url, &url) => (stored.headers.clone(), true),
        (None, Some(_)) => return Err(field("headers", STORED_HEADERS_OTHER_ORIGIN)),
        (None, None) => (Vec::new(), false),
    };

    let base_url: Option<String> = client
        .query_opt("SELECT public_base_url FROM app_settings WHERE id = 'current'", &[])
        .await
        .map_err(unavailable)?
        .and_then(|row| row.get(0));
    let after = json!({"host": host_of(&url), "format": parsed.format.as_str(), "storedUrl": stored_url, "storedHeaders": stored_headers});
    audit_with(&**client, &actor.actor(ctx), ctx, "settings:webhook-test", (!id.is_empty()).then_some(id.as_str()), None, Some(after)).await?;
    drop(client);

    let config = DeliveryConfig { url, headers, format: parsed.format, body_template: parsed.body_template };
    let event = notify::sample_event(crate::auth::principal::now_unix_ms());
    let result = deliver_with_config(&state.webhook_http, &config, &event, RenderContext { base_url: base_url.as_deref() }, options).await;
    if result.ok {
        let attempts = if result.attempts > 1 { format!(", {}회 시도", result.attempts) } else { String::new() };
        let status = result.status.map(|s| s.to_string()).unwrap_or_else(|| "?".into());
        return Ok(TestOutcome::Sent(format!("테스트 알림을 보냈습니다 (HTTP {status}{attempts}).")));
    }
    Ok(TestOutcome::Failed(result.error.unwrap_or_else(|| "발송하지 못했습니다.".into())))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/ui/v1/admin/notifications/webhooks", get(list_route))
        .route("/api/ui/v1/admin/notifications/webhooks/{id}", get(detail_route))
        .route("/api/ui/v1/forms/admin.webhooks.create", post(create_route))
        .route("/api/ui/v1/forms/admin.webhooks.update", post(update_route))
        .route("/api/ui/v1/forms/admin.webhooks.delete", post(delete_route))
        .route("/api/ui/v1/forms/admin.webhooks.test", post(test_route))
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
    let id = create(&state, actor.session()?, &ctx, &form).await?;
    Ok(redirect(format!("{LIST_PATH}/{id}")))
}

async fn update_route(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    let id = update(&state, actor.session()?, &ctx, &form).await?;
    Ok(redirect(format!("{LIST_PATH}/{id}")))
}

async fn delete_route(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    delete(&state, actor.session()?, &ctx, form.get("id").unwrap_or("")).await?;
    Ok(redirect(LIST_PATH))
}

/// The command ran either way; whether the receiver took it is the result.
async fn test_route(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    Ok(Json(test_send(&state, actor.session()?, &ctx, &form, &SendOptions::default()).await?.into_result()))
}
