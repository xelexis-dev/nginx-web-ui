//! SMTP settings (`apps/web/app/admin/settings/smtp/**`). Transport, verification and sending are
//! the notification owner's (`crate::notifications::smtp`); this is the screen and its rules.
//!
//! | Route | TS source |
//! | --- | --- |
//! | `GET /api/ui/v1/admin/settings/smtp` | `smtp/page.tsx` + `getMailerState` (readiness for the mail screens) |
//! | `POST /api/ui/v1/forms/admin.smtp.update` | `updateSmtpSettings` → `{result: {ok, message}, refresh}` |
//! | `POST /api/ui/v1/forms/admin.smtp.test` | `testSmtp` → `{result: {ok, message \| error}}` |
//!
//! The password is sealed and never leaves: the view says only whether one is stored. The form
//! sends `__keep__` (or leaves the field out) to keep it and an empty value to remove it.
//!
//! Changes vs TS: an absent password field keeps the password (TS read it as "" and removed it);
//! a kept password is used only with the stored host — to use another server, enter it again
//! (TS verified, and the test button connected, with the stored password to any typed host: a way
//! to read a secret the screens never show); keeping a password now also works when the stored
//! settings are disabled (TS resolved it through `getMailerState`, which returns nothing then, and
//! verified without a password); a stored password that cannot be opened is said, not replaced by
//! none; the port accepts decimal digits only (TS `Number()` also took `0x19`/`1e2`); the save is
//! one transaction with its audit row; database failures are 503.

use std::collections::BTreeMap;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde_json::{Value, json};

use super::routes::{manager, manager_form, to_json};
use super::{AdminError, AdminResult, ApiErrorCode, TestOutcome, field, unavailable};
use crate::auth::audit::{self, AuditEntry};
use crate::auth::principal::SessionPrincipal;
use crate::auth::setup::is_valid_email;
use crate::contracts::{FormInput, FormResult};
use crate::crypto::js_trim;
use crate::error::ApiError;
use crate::notifications::smtp::{MailInput, MailerConfig, send_with_config, verify_config};
use crate::request_context::RequestContext;
use crate::state::AppState;

pub const KEEP_PASSWORD: &str = "__keep__";
pub const NOT_CONFIGURED: &str = "SMTP가 설정되지 않았습니다. 관리자 패널 → 사이트 설정 → SMTP 설정에서 설정하세요.";
pub const DISABLED: &str = "SMTP가 꺼져 있습니다. 관리자 패널 → 사이트 설정 → SMTP 설정에서 켜세요.";
pub const PASSWORD_UNREADABLE: &str = "저장된 SMTP 비밀번호를 복호화할 수 없습니다(암호화 키 유실). 비밀번호를 다시 입력하세요.";
pub const PASSWORD_OTHER_SERVER: &str = "다른 서버를 쓰려면 비밀번호를 다시 입력하세요. 저장된 비밀번호는 저장된 서버로만 보냅니다.";
pub const TEST_NOT_CONFIGURED: &str = "SMTP가 설정되어 있지 않습니다. SMTP 호스트와 발신 주소를 먼저 입력한 뒤 다시 시도하세요.";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmtpConfigView {
    pub enabled: bool,
    pub host: String,
    pub port: i32,
    pub secure: bool,
    pub username: String,
    pub from_email: String,
    pub from_name: String,
    pub reject_unauthorized: bool,
}

/// `getMailerState`: whether mail can go out right now, and why not.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Readiness {
    pub ok: bool,
    /// `not-configured` | `disabled` | `password-unreadable`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmtpView {
    /// The form defaults: the stored values, or TS's defaults when nothing is stored.
    pub config: SmtpConfigView,
    pub has_stored_password: bool,
    pub readiness: Readiness,
}

struct Stored {
    enabled: bool,
    host: String,
    port: i32,
    from_email: String,
    password_enc: Option<String>,
}

enum PasswordInput {
    Keep,
    Remove,
    Set(String),
}

struct Parsed {
    enabled: bool,
    host: String,
    port: u16,
    secure: bool,
    username: Option<String>,
    from_email: String,
    from_name: Option<String>,
    reject_unauthorized: bool,
    password: PasswordInput,
}

type Fields = BTreeMap<String, String>;

/// TS `parseForm`: trimmed strings, blank optional values are none, switches by literal `"true"`
/// except `rejectUnauthorized`, which only `"false"` turns off. The password is taken as sent.
fn parse_form(form: &FormInput) -> (Parsed, Fields) {
    let mut errors = Fields::new();
    let text = |key: &str| js_trim(form.get(key).unwrap_or("")).to_owned();
    let optional = |key: &str| Some(text(key)).filter(|v| !v.is_empty());
    let host = text("host");
    if host.is_empty() {
        errors.insert("host".into(), "SMTP 호스트를 입력하세요.".into());
    }
    let raw_port = text("port");
    let port = (!raw_port.is_empty() && raw_port.bytes().all(|b| b.is_ascii_digit())).then(|| raw_port.parse::<u16>().ok()).flatten().filter(|p| *p >= 1);
    if port.is_none() {
        errors.insert("port".into(), "1-65535 사이의 포트를 입력하세요.".into());
    }
    let from_email = text("fromEmail");
    if from_email.is_empty() {
        errors.insert("fromEmail".into(), "발신 주소를 입력하세요.".into());
    } else if !is_valid_email(&from_email) {
        errors.insert("fromEmail".into(), "발신 주소 형식이 올바르지 않습니다.".into());
    }
    let password = match form.get("password") {
        None | Some(KEEP_PASSWORD) => PasswordInput::Keep,
        Some("") => PasswordInput::Remove,
        Some(value) => PasswordInput::Set(value.to_owned()),
    };
    let parsed = Parsed {
        enabled: text("enabled") == "true",
        host,
        port: port.unwrap_or(0),
        secure: text("secure") == "true",
        username: optional("username"),
        from_email,
        from_name: optional("fromName"),
        reject_unauthorized: text("rejectUnauthorized") != "false",
        password,
    };
    (parsed, errors)
}

async fn load(client: &impl tokio_postgres::GenericClient, lock: bool) -> AdminResult<Option<Stored>> {
    let sql = format!("SELECT enabled, host, port, from_email, password_enc FROM smtp_settings WHERE id = 'current'{}", if lock { " FOR UPDATE" } else { "" });
    let row = client.query_opt(&sql, &[]).await.map_err(unavailable)?;
    Ok(row.map(|row| Stored {
        enabled: row.get("enabled"),
        host: row.get("host"),
        port: row.get("port"),
        from_email: row.get("from_email"),
        password_enc: row.get::<_, Option<String>>("password_enc").filter(|p| !p.is_empty()),
    }))
}

/// The password to connect with. A kept one only goes to the server it was stored for.
fn resolve_password(state: &AppState, parsed: &Parsed, stored: Option<&Stored>) -> AdminResult<Option<String>> {
    match &parsed.password {
        PasswordInput::Set(value) => Ok(Some(value.clone())),
        PasswordInput::Remove => Ok(None),
        PasswordInput::Keep => {
            let Some(stored) = stored.filter(|s| s.password_enc.is_some()) else { return Ok(None) };
            if !stored.host.eq_ignore_ascii_case(&parsed.host) {
                return Err(field("password", PASSWORD_OTHER_SERVER));
            }
            state.secrets.open(stored.password_enc.as_deref()).map(Some).ok_or_else(|| AdminError::Refused(PASSWORD_UNREADABLE.to_owned()))
        }
    }
}

fn mailer(parsed: &Parsed, password: Option<String>) -> MailerConfig {
    MailerConfig {
        host: parsed.host.clone(),
        port: parsed.port,
        secure: parsed.secure,
        username: parsed.username.clone(),
        password,
        from_email: parsed.from_email.clone(),
        from_name: parsed.from_name.clone(),
        reject_unauthorized: parsed.reject_unauthorized,
    }
}

pub async fn view(state: &AppState) -> AdminResult<SmtpView> {
    let client = state.db.get().await.map_err(unavailable)?;
    let row = client
        .query_opt("SELECT enabled, host, port, secure, username, password_enc, from_email, from_name, reject_unauthorized FROM smtp_settings WHERE id = 'current'", &[])
        .await
        .map_err(unavailable)?;
    let not_ready = |reason, message| Readiness { ok: false, reason: Some(reason), message: Some(message) };
    let Some(row) = row else {
        let config = SmtpConfigView { enabled: false, host: String::new(), port: 587, secure: false, username: String::new(), from_email: String::new(), from_name: String::new(), reject_unauthorized: true };
        return Ok(SmtpView { config, has_stored_password: false, readiness: not_ready("not-configured", NOT_CONFIGURED) });
    };
    let sealed: Option<String> = row.get::<_, Option<String>>("password_enc").filter(|p| !p.is_empty());
    let enabled: bool = row.get("enabled");
    let readiness = if !enabled {
        not_ready("disabled", DISABLED)
    } else if sealed.is_some() && state.secrets.open(sealed.as_deref()).is_none() {
        not_ready("password-unreadable", PASSWORD_UNREADABLE)
    } else {
        Readiness { ok: true, reason: None, message: None }
    };
    let config = SmtpConfigView {
        enabled,
        host: row.get("host"),
        port: row.get("port"),
        secure: row.get("secure"),
        username: row.get::<_, Option<String>>("username").unwrap_or_default(),
        from_email: row.get("from_email"),
        from_name: row.get::<_, Option<String>>("from_name").unwrap_or_default(),
        reject_unauthorized: row.get("reject_unauthorized"),
    };
    Ok(SmtpView { config, has_stored_password: sealed.is_some(), readiness })
}

/// Saves the settings. Enabled settings must connect and authenticate first — "saved" would
/// otherwise read as "mail goes out" while expiry notices fail quietly. Disabled ones are kept
/// unchecked (half-filled settings may be parked).
pub async fn update(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, form: &FormInput) -> AdminResult<()> {
    let (parsed, errors) = parse_form(form);
    if !errors.is_empty() {
        return Err(AdminError::Invalid(errors));
    }
    let stored = {
        let client = state.db.get().await.map_err(unavailable)?;
        load(&**client, false).await?
    };
    if parsed.enabled {
        let password = resolve_password(state, &parsed, stored.as_ref())?;
        let check = verify_config(&mailer(&parsed, password),&state.config).await;
        if !check.ok {
            let error = check.error.unwrap_or_default();
            return Err(AdminError::Refused(format!("{}:{} 연결·인증에 실패해 저장하지 않았습니다 — {error}", parsed.host, parsed.port)));
        }
    } else if matches!(parsed.password, PasswordInput::Keep) && stored.as_ref().is_some_and(|s| s.password_enc.is_some() && !s.host.eq_ignore_ascii_case(&parsed.host)) {
        // Parking disabled settings for another server must not carry the password along either.
        return Err(field("password", PASSWORD_OTHER_SERVER));
    }
    let (password_changed, sealed) = match &parsed.password {
        PasswordInput::Keep => (false, None),
        PasswordInput::Remove => (true, None),
        PasswordInput::Set(value) => (true, Some(state.secrets.seal(value).map_err(|_| AdminError::Api(ApiErrorCode::Internal))?)),
    };

    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    // A missing singleton row cannot be serialized with FOR UPDATE alone.
    tx.execute("SELECT pg_advisory_xact_lock(hashtext('nginx-web-ui:smtp-settings'))", &[]).await.map_err(unavailable)?;
    let before = load(&*tx, true).await?;
    if matches!(parsed.password, PasswordInput::Keep)
        && stored.as_ref().map(|s| (&s.host, &s.password_enc)) != before.as_ref().map(|s| (&s.host, &s.password_enc))
    {
        return Err(AdminError::Refused("SMTP 설정이 다른 요청에서 변경되었습니다. 새로고침한 뒤 다시 저장하세요.".into()));
    }
    tx.execute(
        "INSERT INTO smtp_settings (id, enabled, host, port, secure, username, password_enc, from_email, from_name, reject_unauthorized, updated_at, actor_id) \
         VALUES ('current', $1, $2, $3, $4, $5, $6, $7, $8, $9, now(), $10) \
         ON CONFLICT (id) DO UPDATE SET enabled = EXCLUDED.enabled, host = EXCLUDED.host, port = EXCLUDED.port, secure = EXCLUDED.secure, \
         username = EXCLUDED.username, from_email = EXCLUDED.from_email, from_name = EXCLUDED.from_name, \
         reject_unauthorized = EXCLUDED.reject_unauthorized, updated_at = now(), actor_id = EXCLUDED.actor_id, \
         password_enc = CASE WHEN $11 THEN EXCLUDED.password_enc ELSE smtp_settings.password_enc END",
        &[&parsed.enabled, &parsed.host, &i32::from(parsed.port), &parsed.secure, &parsed.username, &sealed, &parsed.from_email, &parsed.from_name, &parsed.reject_unauthorized, &actor.user_id, &password_changed],
    )
    .await
    .map_err(unavailable)?;
    // Never the password, in either direction — only whether it changed.
    let before = before.map(|b| json!({"enabled": b.enabled, "host": b.host, "port": b.port, "fromEmail": b.from_email}));
    let after = json!({"enabled": parsed.enabled, "host": parsed.host, "port": parsed.port, "fromEmail": parsed.from_email, "passwordChanged": password_changed});
    let entry = AuditEntry { actor_id: Some(actor.user_id.clone()), action: "settings:smtp".into(), before, after: Some(after), ip: ctx.ip.clone(), ..AuditEntry::default() };
    audit::write(&*tx, &entry).await.map_err(unavailable)?;
    tx.commit().await.map_err(unavailable)
}

fn invalid_with(message: &str, fields: Fields) -> ApiError {
    let mut error = ApiError::new(StatusCode::BAD_REQUEST, "validation_failed", message);
    error.field_errors = Some(fields);
    error
}

/// Connects (DNS, TCP, TLS, authentication) with the form's values, possibly unsaved, and sends
/// one message when a recipient is given — only a real send shows whether the sender address is
/// accepted. Audits the attempt without saving SMTP settings.
pub async fn test(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, form: &FormInput) -> Result<TestOutcome, ApiError> {
    let (parsed, errors) = parse_form(form);
    if parsed.host.is_empty() && parsed.from_email.is_empty() {
        return Err(invalid_with(TEST_NOT_CONFIGURED, errors));
    }
    if !errors.is_empty() {
        return Err(invalid_with("입력값을 확인하세요.", errors));
    }
    let to = js_trim(form.get("testTo").unwrap_or(""));
    let config = {
        let client = state.db.get().await.map_err(|_| crate::auth::principal::db_unavailable())?;
        let stored = load(&**client, false).await?;
        let config = mailer(&parsed, resolve_password(state, &parsed, stored.as_ref())?);
        let entry = AuditEntry {
            actor_id: Some(actor.user_id.clone()),
            action: "settings:smtp-test".into(),
            after: Some(json!({"host": parsed.host, "port": parsed.port, "fromEmail": parsed.from_email, "to": (!to.is_empty()).then_some(to)})),
            ip: ctx.ip.clone(),
            ..AuditEntry::default()
        };
        audit::write(&**client, &entry).await.map_err(unavailable)?;
        config
    };
    let verified = verify_config(&config,&state.config).await;
    if !verified.ok {
        return Ok(TestOutcome::Failed(format!("{}:{} 에 연결하지 못했습니다 — {}", config.host, config.port, verified.error.unwrap_or_default())));
    }
    if to.is_empty() {
        return Ok(TestOutcome::Sent("연결·인증에 성공했습니다. (수신 주소를 넣으면 실제 발송까지 시험합니다)".into()));
    }
    let input = MailInput {
        to: to.to_owned(),
        subject: "[nginx-web-ui] SMTP 테스트".into(),
        text: "이 메일이 보이면 nginx-web-ui의 SMTP 설정이 정상입니다.".into(),
        html: None,
        message_id: None,
    };
    let sent = send_with_config(&config, &input,&state.config).await;
    if !sent.ok {
        return Ok(TestOutcome::Failed(format!("연결·인증은 되었으나 발송이 거부되었습니다 — {}", sent.error.unwrap_or_default())));
    }
    Ok(TestOutcome::Sent(format!("{to} 로 테스트 메일을 보냈습니다.")))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/ui/v1/admin/settings/smtp", get(view_route))
        .route("/api/ui/v1/forms/admin.smtp.update", post(update_route))
        .route("/api/ui/v1/forms/admin.smtp.test", post(test_route))
}

async fn view_route(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    manager(&state, &headers, &ctx).await?;
    to_json(view(&state).await?)
}

async fn update_route(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    update(&state, &actor, &ctx, &form).await?;
    Ok(Json(FormResult { result: Some(json!({"ok": true, "message": "저장했습니다."})), refresh: Some(true), ..FormResult::default() }))
}

async fn test_route(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    Ok(Json(test(&state, &actor, &ctx, &form).await?.into_result()))
}
