//! OAuth (OIDC) provider administration (`apps/web/app/admin/oauth/**`,
//! `packages/auth/src/oauth-config.ts`).
//!
//! The client secret is sealed (`enc:v1:`) and never leaves: not in views, not in audit rows. On
//! update the form sends `__keep__` to leave it unchanged (empty would otherwise erase it and kill
//! the provider — same convention as the webhook/SMTP forms).
//!
//! Changes vs TS (stricter, no data loss): create is one INSERT whose primary-key conflict becomes
//! the field error (TS checked, then inserted); `sortOrder` accepts decimal integers within the
//! column's range only (TS `Number()` also took `1e2`/`0x10`, and out-of-range values became a
//! database 500).

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};
use tokio_postgres::error::SqlState;

use super::{AdminError, AdminResult, ApiErrorCode, field, unavailable};
use crate::auth::audit::{self, AuditEntry};
use crate::auth::principal::SessionPrincipal;
use crate::contracts::FormInput;
use crate::crypto::{is_ecmascript_whitespace, js_trim};
use crate::request_context::RequestContext;
use crate::state::AppState;

pub const DEFAULT_SCOPE: &str = "openid email profile";
/// Sent by the edit form in place of the secret: "keep the stored one".
pub const KEEP_SECRET: &str = "__keep__";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    pub id: String,
    pub name: String,
    pub issuer: String,
    pub client_id: String,
    pub scope: String,
    pub enabled: bool,
    pub sort_order: i32,
    pub linked_users: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDetail {
    #[serde(flatten)]
    pub provider: ProviderView,
    /// What the operator registers at the provider (absolute with the public base URL).
    pub callback_url: String,
}

/// `/^[a-z0-9][a-z0-9-]{0,63}$/`, not reserved. The id becomes the callback path.
pub fn validate_provider_id(id: &str) -> Result<(), String> {
    let bytes = id.as_bytes();
    let shape = (1..=64).contains(&bytes.len())
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes.iter().all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !shape {
        return Err(format!("공급자 ID 는 영소문자로 시작하고 소문자·숫자·하이픈만 쓸 수 있습니다(최대 64자): \"{id}\""));
    }
    // Auth.js's own provider id: taking it would remove password sign-in.
    if id == "credentials" {
        return Err(format!("\"{id}\" 는 예약된 ID 라 쓸 수 없습니다."));
    }
    Ok(())
}

/// WHATWG URL parse, https only (secrets and id_tokens must not travel in clear).
pub fn validate_issuer(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|_| format!("로그인 서버 주소가 올바르지 않습니다: \"{url}\""))?;
    if parsed.scheme() != "https" {
        return Err("로그인 서버 주소는 https 여야 합니다.".to_owned());
    }
    Ok(())
}

/// One space between parts, duplicates removed, `openid` guaranteed first when missing; empty →
/// the default (without `openid` there is no id_token and no `sub`).
pub fn normalize_scope(raw: &str) -> String {
    let parts: Vec<&str> = raw.split(is_ecmascript_whitespace).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return DEFAULT_SCOPE.to_owned();
    }
    let mut out: Vec<&str> = Vec::new();
    if !parts.contains(&"openid") {
        out.push("openid");
    }
    for part in parts {
        if !out.contains(&part) {
            out.push(part);
        }
    }
    out.join(" ")
}

/// `/api/auth/callback/<id>`, absolute when the public base URL is known.
pub fn callback_url(id: &str, public_base_url: Option<&str>) -> String {
    let path = format!("/api/auth/callback/{id}");
    match public_base_url.filter(|base| !base.is_empty()) {
        Some(base) => format!("{}{path}", base.trim_end_matches('/')),
        None => path,
    }
}

type Fields = BTreeMap<String, String>;

struct Parsed {
    name: String,
    issuer: String,
    client_id: String,
    scope: String,
    enabled: bool,
    sort_order: i32,
    /// `None` = keep the stored secret.
    client_secret: Option<String>,
}

/// TS `parseForm`: trimmed strings, `enabled === "true"`, `sortOrder` default 0.
fn parse_form(form: &FormInput) -> (Parsed, Fields) {
    let mut errors = Fields::new();
    let text = |key: &str| js_trim(form.get(key).unwrap_or("")).to_owned();
    let name = text("name");
    if name.is_empty() {
        errors.insert("name".into(), "표시 이름을 입력하세요.".into());
    }
    let issuer = text("issuer");
    if issuer.is_empty() {
        errors.insert("issuer".into(), "로그인 서버 주소를 입력하세요.".into());
    } else if let Err(message) = validate_issuer(&issuer) {
        errors.insert("issuer".into(), message);
    }
    let client_id = text("clientId");
    if client_id.is_empty() {
        errors.insert("clientId".into(), "클라이언트 ID 를 입력하세요.".into());
    }
    let raw_secret = text("clientSecret");
    let client_secret = if raw_secret == KEEP_SECRET {
        None
    } else {
        if raw_secret.is_empty() {
            errors.insert("clientSecret".into(), "클라이언트 비밀키를 입력하세요.".into());
        }
        Some(raw_secret)
    };
    let raw_order = text("sortOrder");
    let sort_order = if raw_order.is_empty() {
        0
    } else {
        let decimal = raw_order.strip_prefix(['+', '-']).unwrap_or(&raw_order);
        match (!decimal.is_empty() && decimal.bytes().all(|b| b.is_ascii_digit())).then(|| raw_order.parse::<i32>().ok()).flatten() {
            Some(n) => n,
            None => {
                errors.insert("sortOrder".into(), "정렬 순서는 정수여야 합니다.".into());
                0
            }
        }
    };
    let parsed = Parsed {
        name,
        issuer,
        client_id,
        scope: normalize_scope(form.get("scope").unwrap_or("")),
        enabled: text("enabled") == "true",
        sort_order,
        client_secret,
    };
    (parsed, errors)
}

fn view_of(row: &tokio_postgres::Row) -> ProviderView {
    ProviderView {
        id: row.get("id"),
        name: row.get("name"),
        issuer: row.get("issuer"),
        client_id: row.get("client_id"),
        scope: row.get("scope"),
        enabled: row.get("enabled"),
        sort_order: row.get("sort_order"),
        linked_users: row.get("linked_users"),
    }
}

const VIEW_SQL: &str = "SELECT p.id, p.name, p.issuer, p.client_id, p.scope, p.enabled, p.sort_order, \
    (SELECT count(*) FROM oauth_accounts a WHERE a.provider_id = p.id) AS linked_users FROM oauth_providers p";

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

pub async fn list(state: &AppState) -> AdminResult<Vec<ProviderView>> {
    let rows = state
        .db
        .get()
        .await
        .map_err(unavailable)?
        .query(&format!("{VIEW_SQL} ORDER BY p.sort_order, p.name, p.id"), &[])
        .await
        .map_err(unavailable)?;
    Ok(rows.iter().map(view_of).collect())
}

pub async fn detail(state: &AppState, id: &str) -> AdminResult<ProviderDetail> {
    let client = state.db.get().await.map_err(unavailable)?;
    let row = client
        .query_opt(&format!("{VIEW_SQL} WHERE p.id = $1"), &[&id])
        .await
        .map_err(unavailable)?
        .ok_or(AdminError::NotFound)?;
    let base: Option<String> = client
        .query_opt("SELECT public_base_url FROM app_settings WHERE id = 'current'", &[])
        .await
        .map_err(unavailable)?
        .and_then(|r| r.get(0));
    let provider = view_of(&row);
    let callback_url = callback_url(&provider.id, base.as_deref());
    Ok(ProviderDetail { provider, callback_url })
}

pub async fn create(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, form: &FormInput) -> AdminResult<String> {
    let id = js_trim(form.get("id").unwrap_or("")).to_owned();
    let (parsed, mut errors) = parse_form(form);
    if let Err(message) = validate_provider_id(&id) {
        errors.insert("id".into(), message);
    }
    if !errors.is_empty() {
        return Err(AdminError::Invalid(errors));
    }
    let secret = parsed.client_secret.as_deref().unwrap_or_default();
    let sealed = state.secrets.seal(secret).map_err(|_| AdminError::Api(ApiErrorCode::Internal))?;
    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let inserted = tx
        .execute(
            "INSERT INTO oauth_providers (id, name, issuer, client_id, client_secret_enc, scope, enabled, sort_order, actor_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            &[&id, &parsed.name, &parsed.issuer, &parsed.client_id, &sealed, &parsed.scope, &parsed.enabled, &parsed.sort_order, &actor.user_id],
        )
        .await;
    if let Err(error) = inserted {
        if error.code() == Some(&SqlState::UNIQUE_VIOLATION) {
            return Err(field("id", "이미 같은 ID 의 공급자가 있습니다."));
        }
        return Err(unavailable(error));
    }
    // Never the secret in the audit row.
    let after = json!({"name": parsed.name, "issuer": parsed.issuer, "enabled": parsed.enabled});
    audit_in(&tx, actor, ctx, "oauth:provider-create", &id, None, Some(after)).await?;
    tx.commit().await.map_err(unavailable)?;
    Ok(id)
}

pub async fn update(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, form: &FormInput) -> AdminResult<String> {
    let id = js_trim(form.get("id").unwrap_or("")).to_owned();
    if id.is_empty() {
        return Err(AdminError::Refused("잘못된 요청입니다.".to_owned()));
    }
    let (parsed, errors) = parse_form(form);
    if !errors.is_empty() {
        return Err(AdminError::Invalid(errors));
    }
    let sealed = parsed
        .client_secret
        .as_deref()
        .map(|secret| state.secrets.seal(secret))
        .transpose()
        .map_err(|_| AdminError::Api(ApiErrorCode::Internal))?;
    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let before = tx
        .query_opt("SELECT name, issuer, enabled FROM oauth_providers WHERE id = $1 FOR UPDATE", &[&id])
        .await
        .map_err(unavailable)?
        .ok_or(AdminError::NotFound)?;
    if before.get::<_, String>("issuer") != parsed.issuer
        && tx.query_one("SELECT EXISTS(SELECT 1 FROM oauth_accounts WHERE provider_id=$1)", &[&id])
            .await.map_err(unavailable)?.get::<_, bool>(0)
    {
        return Err(field("issuer", "연결된 계정이 있는 로그인 서버의 주소는 바꿀 수 없습니다. 새 로그인 서버를 추가해 다시 연결하세요."));
    }
    tx.execute(
        "UPDATE oauth_providers SET name = $2, issuer = $3, client_id = $4, scope = $5, enabled = $6, sort_order = $7, \
         client_secret_enc = COALESCE($8, client_secret_enc), updated_at = now(), actor_id = $9 WHERE id = $1",
        &[&id, &parsed.name, &parsed.issuer, &parsed.client_id, &parsed.scope, &parsed.enabled, &parsed.sort_order, &sealed, &actor.user_id],
    )
    .await
    .map_err(unavailable)?;
    let before = json!({"name": before.get::<_, String>("name"), "issuer": before.get::<_, String>("issuer"), "enabled": before.get::<_, bool>("enabled")});
    let after = json!({"name": parsed.name, "issuer": parsed.issuer, "enabled": parsed.enabled, "secretChanged": sealed.is_some()});
    audit_in(&tx, actor, ctx, "oauth:provider-update", &id, Some(before), Some(after)).await?;
    tx.commit().await.map_err(unavailable)?;
    Ok(id)
}

/// Deletes the provider; its links cascade, the users stay (those who only had this provider
/// can no longer sign in — the count goes into the audit row). Missing id → nothing.
pub async fn delete(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, id: &str) -> AdminResult<()> {
    let id = js_trim(id);
    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let Some(before) = tx
        .query_opt(
            "SELECT name, issuer, (SELECT count(*) FROM oauth_accounts WHERE provider_id = $1) AS linked FROM oauth_providers WHERE id = $1 FOR UPDATE",
            &[&id],
        )
        .await
        .map_err(unavailable)?
    else {
        return Ok(());
    };
    tx.execute("DELETE FROM oauth_providers WHERE id = $1", &[&id]).await.map_err(unavailable)?;
    let before = json!({"name": before.get::<_, String>("name"), "issuer": before.get::<_, String>("issuer"), "linkedUsers": before.get::<_, i64>("linked")});
    audit_in(&tx, actor, ctx, "oauth:provider-delete", id, Some(before), None).await?;
    tx.commit().await.map_err(unavailable)
}

/// Missing id → nothing.
pub async fn toggle(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, id: &str) -> AdminResult<()> {
    let id = js_trim(id);
    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let Some(row) = tx
        .query_opt(
            "UPDATE oauth_providers SET enabled = NOT enabled, updated_at = now(), actor_id = $2 WHERE id = $1 RETURNING enabled",
            &[&id, &actor.user_id],
        )
        .await
        .map_err(unavailable)?
    else {
        return Ok(());
    };
    let enabled: bool = row.get(0);
    audit_in(&tx, actor, ctx, "oauth:provider-toggle", id, Some(json!({"enabled": !enabled})), Some(json!({"enabled": enabled}))).await?;
    tx.commit().await.map_err(unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_ids_follow_the_ts_rule() {
        for ok in ["keycloak", "k", "0idp", "my-idp-2", &"a".repeat(64)] {
            assert_eq!(validate_provider_id(ok), Ok(()), "{ok}");
        }
        for bad in ["", "-idp", "KeyCloak", "a_b", "a.b", "a/b", "a b", &"a".repeat(65)] {
            assert_eq!(validate_provider_id(bad).unwrap_err(), format!("공급자 ID 는 영소문자로 시작하고 소문자·숫자·하이픈만 쓸 수 있습니다(최대 64자): \"{bad}\""));
        }
        assert_eq!(validate_provider_id("credentials").unwrap_err(), "\"credentials\" 는 예약된 ID 라 쓸 수 없습니다.");
    }

    #[test]
    fn issuers_must_be_https_urls() {
        assert_eq!(validate_issuer("https://idp.example.test/realms/main"), Ok(()));
        assert_eq!(validate_issuer("http://idp.example.test").unwrap_err(), "로그인 서버 주소는 https 여야 합니다.");
        assert_eq!(validate_issuer("idp.example.test").unwrap_err(), "로그인 서버 주소가 올바르지 않습니다: \"idp.example.test\"");
    }

    #[test]
    fn scopes_are_normalized_with_openid_first() {
        assert_eq!(normalize_scope(""), DEFAULT_SCOPE);
        assert_eq!(normalize_scope("  \u{3000} "), DEFAULT_SCOPE);
        assert_eq!(normalize_scope("email  profile email"), "openid email profile");
        assert_eq!(normalize_scope("profile openid\temail"), "profile openid email");
        assert_eq!(normalize_scope("openid openid"), "openid");
    }

    #[test]
    fn callback_urls_are_absolute_only_with_a_public_base() {
        assert_eq!(callback_url("kc", None), "/api/auth/callback/kc");
        assert_eq!(callback_url("kc", Some("")), "/api/auth/callback/kc");
        assert_eq!(callback_url("kc", Some("https://nginx.example.test//")), "https://nginx.example.test/api/auth/callback/kc");
    }
}
