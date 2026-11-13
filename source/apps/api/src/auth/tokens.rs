//! Access tokens: issue, list, revoke (`apps/web/lib/access-token{,-service}.ts`, the issue form
//! parser in `app/(app)/account/tokens/tokens.ts`).
//!
//! - `user` tokens belong to a person (any role, their own only); `server` tokens belong to the
//!   instance and carry `node:pair` — **admin only** to issue, list or revoke, checked here, not
//!   only by the screen.
//! - The scope (kind + owner) always comes from code/session, never from the form; it is the SQL
//!   WHERE of list and revoke.
//! - The plaintext exists only in `issue`'s return value: stored are SHA-256 hex and the last 4
//!   characters. Server-token issue audits carry `{kind}` only (the audit view is readable by
//!   viewers; the token detail page is the source of name/scopes/expiry).
//! - Revoke does not distinguish missing / someone else's / already revoked (no existence oracle)
//!   and never overwrites the first revocation time.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;
use serde_json::{Value, json};
use tokio_postgres::error::SqlState;

use super::audit::{self, AuditEntry};
use super::principal::SessionPrincipal;
use super::rbac::{Action, can};
use crate::contracts::{FormInput, FormResult, Role};
use crate::crypto::js_trim;
use crate::error::ApiError;
use axum::http::StatusCode;
use crate::request_context::RequestContext;
use crate::state::AppState;

pub const NAME_MAX: usize = 100;
/// Every scope a token can carry: the RBAC actions plus the purpose scope `node:pair`.
pub const ALL_SCOPES: [&str; 5] = ["config:read", "config:write", "nginx:reload", "user:manage", "node:pair"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenKind {
    User,
    Server,
}

/// Which rows a list/revoke may touch. A user scope always names its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenScope {
    User { owner_user_id: String },
    Server,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueFormValue {
    pub name: String,
    pub scopes: Vec<String>,
    /// Unix milliseconds; `None` = never expires.
    pub expires_at_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedToken {
    pub id: String,
    /// The plaintext, returned exactly once.
    pub token: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenStatus {
    Active,
    Expired,
    Revoked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenListRow {
    pub id: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
    /// `nwu_u_…abcd` / `nwu_s_…abcd`.
    pub masked: String,
    pub status: TokenStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenError {
    /// The form's field errors (`name`, `scopes`, `expiresAt`).
    Invalid(BTreeMap<String, String>),
    /// TS `AccessTokenError`: a refusal with a message the screen shows.
    Rejected(String),
    /// Missing, not in this scope, or already revoked — deliberately one answer.
    NothingToRevoke,
    Unavailable,
}

// ─────────────────────────────── HTTP (shared by /account/tokens and /admin/tokens) ───────────────────────────────

/// Issue refusals: form errors as they are; a scope/kind refusal on the `scopes` field (the
/// only thing the user can change — the kind comes from the route).
pub fn issue_error(error: TokenError) -> ApiError {
    match error {
        TokenError::Invalid(fields) => validation(fields),
        TokenError::Rejected(message) => validation(BTreeMap::from([("scopes".to_owned(), message)])),
        TokenError::NothingToRevoke => ApiError::internal(),
        TokenError::Unavailable => super::principal::db_unavailable(),
    }
}

pub fn revoke_error(error: TokenError) -> ApiError {
    match error {
        TokenError::NothingToRevoke => ApiError::new(StatusCode::CONFLICT, "conflict", "해지할 항목이 없습니다 — 이미 해지되었거나 없는 토큰입니다."),
        TokenError::Rejected(message) => ApiError::new(StatusCode::FORBIDDEN, "forbidden", message),
        TokenError::Invalid(fields) => validation(fields),
        TokenError::Unavailable => super::principal::db_unavailable(),
    }
}

pub fn list_error(error: TokenError) -> ApiError {
    match error {
        TokenError::Rejected(message) => ApiError::new(StatusCode::FORBIDDEN, "forbidden", message),
        TokenError::Unavailable => super::principal::db_unavailable(),
        _ => ApiError::internal(),
    }
}

fn validation(fields: BTreeMap<String, String>) -> ApiError {
    let mut error = ApiError::new(StatusCode::BAD_REQUEST, "validation_failed", "입력값이 올바르지 않습니다.");
    error.field_errors = Some(fields);
    error
}

/// `{tokens, issuableScopes}` — the list and what the issue form may offer this caller.
pub async fn list_view(state: &AppState, actor: &SessionPrincipal, scope: &TokenScope) -> Result<Value, ApiError> {
    let rows = list(state, actor, scope, super::principal::now_unix_ms()).await.map_err(list_error)?;
    let kind = if *scope == TokenScope::Server { TokenKind::Server } else { TokenKind::User };
    Ok(json!({"tokens": rows, "issuableScopes": issuable_scopes(kind, actor.role)}))
}

/// One row of the scope, or 404 — a missing id and one outside the scope look the same.
pub async fn detail_view(state: &AppState, actor: &SessionPrincipal, scope: &TokenScope, id: &str) -> Result<Value, ApiError> {
    let rows = list(state, actor, scope, super::principal::now_unix_ms()).await.map_err(list_error)?;
    let row = rows.into_iter().find(|r| r.id == id).ok_or_else(ApiError::not_found)?;
    serde_json::to_value(row).map_err(|_| ApiError::internal())
}

/// Issue from the form; the plaintext is returned in `result` and nowhere else (no redirect:
/// that would carry it through a URL, history or logs).
pub async fn issue_from_form(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, kind: TokenKind, form: &FormInput) -> Result<FormResult, ApiError> {
    let value = parse_issue_form(form, super::principal::now_unix_ms()).map_err(|fields| issue_error(TokenError::Invalid(fields)))?;
    let name = value.name.clone();
    let issued = issue(state, actor, ctx, kind, value).await.map_err(issue_error)?;
    // The TS `IssueTokenState` shape.
    Ok(FormResult { result: Some(json!({"ok": true, "token": issued.token, "id": issued.id, "name": name})), ..FormResult::default() })
}

pub async fn revoke_from_form(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, scope: &TokenScope, form: &FormInput) -> Result<FormResult, ApiError> {
    let id = js_trim(form.get("id").unwrap_or(""));
    if id.is_empty() {
        return Err(ApiError::new(StatusCode::CONFLICT, "conflict", "잘못된 요청입니다."));
    }
    revoke(state, actor, ctx, scope, id).await.map_err(revoke_error)?;
    // `result.ok` clears a previous error in the browser action state (TS `RevokeTokenState`).
    Ok(FormResult { result: Some(json!({"ok": true})), refresh: Some(true), ..FormResult::default() })
}

/// The scopes a token of `kind` may be given by `role` (TS `issuableScopes`): a server token's
/// scopes belong to the instance (role not consulted); a user token never exceeds its owner.
pub fn issuable_scopes(kind: TokenKind, role: Role) -> Vec<&'static str> {
    match kind {
        TokenKind::Server => ALL_SCOPES.to_vec(),
        // `node:pair` has no RBAC action: it never passes through the role filter.
        TokenKind::User => ALL_SCOPES
            .iter()
            .copied()
            .filter(|scope| action_of(scope).is_some_and(|action| can(role, action)))
            .collect(),
    }
}

fn action_of(scope: &str) -> Option<Action> {
    Action::ALL.into_iter().find(|a| a.as_str() == scope)
}

const PREFIX_USER: &str = "nwu_u_";
const PREFIX_SERVER: &str = "nwu_s_";
const SUFFIX_LEN: usize = 4;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn prefix(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::User => PREFIX_USER,
        TokenKind::Server => PREFIX_SERVER,
    }
}

/// TS `parseIssueForm`: `name`, repeated `scopes`, `expiry` ∈ {30d, 90d, 1y, custom, never},
/// `expiresOn` (YYYY-MM-DD, the **end** of that UTC day) for custom. A past date is refused here.
pub fn parse_issue_form(form: &FormInput, now_ms: i64) -> Result<IssueFormValue, BTreeMap<String, String>> {
    let mut errors = BTreeMap::new();
    let mut fail = |field: &str, message: String| {
        errors.insert(field.to_owned(), message);
    };
    let name = js_trim(form.get("name").unwrap_or("")).to_owned();
    if name.is_empty() {
        fail("name", "이름을 입력하세요.".into());
    } else if name.encode_utf16().count() > NAME_MAX {
        fail("name", format!("이름은 {NAME_MAX}자 이하여야 합니다."));
    }
    let scopes = form.all("scopes").to_vec();
    if scopes.is_empty() {
        fail("scopes", "권한을 하나 이상 선택하세요.".into());
    }
    let mut expires_at_ms = None;
    match form.get("expiry").unwrap_or("") {
        "never" => {}
        "30d" => expires_at_ms = Some(now_ms + 30 * DAY_MS),
        "90d" => expires_at_ms = Some(now_ms + 90 * DAY_MS),
        "1y" => expires_at_ms = Some(now_ms + 365 * DAY_MS),
        "custom" => {
            let raw = js_trim(form.get("expiresOn").unwrap_or(""));
            if raw.is_empty() {
                fail("expiresAt", "만료 날짜를 입력하세요.".into());
            } else {
                // `YYYY-MM-DD` means through the end of that day (UTC): parsing it as midnight
                // would always err short, possibly into the past.
                let end = (raw.len() == 10)
                    .then(|| chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok())
                    .flatten()
                    .and_then(|date| date.and_hms_milli_opt(23, 59, 59, 999))
                    .map(|at| at.and_utc().timestamp_millis());
                match end {
                    None => fail("expiresAt", "만료 날짜 형식이 올바르지 않습니다.".into()),
                    Some(at) if at <= now_ms => fail("expiresAt", "지난 날짜는 고를 수 없습니다. 더 뒤의 날짜를 고르세요.".into()),
                    Some(at) => expires_at_ms = Some(at),
                }
            }
        }
        _ => fail("expiresAt", "만료 기간을 선택하세요.".into()),
    }
    if errors.is_empty() { Ok(IssueFormValue { name, scopes, expires_at_ms }) } else { Err(errors) }
}

fn iso(at: chrono::DateTime<chrono::Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn from_ms(ms: i64) -> Result<chrono::DateTime<chrono::Utc>, TokenError> {
    chrono::DateTime::from_timestamp_millis(ms).ok_or_else(|| TokenError::Rejected("만료 날짜 형식이 올바르지 않습니다.".into()))
}

fn unavailable<E>(_: E) -> TokenError {
    TokenError::Unavailable
}

pub async fn issue(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, kind: TokenKind, value: IssueFormValue) -> Result<IssuedToken, TokenError> {
    // Who may issue: server tokens (they carry `node:pair`) are admin-only, checked here.
    if kind == TokenKind::Server && actor.role != Role::Admin {
        return Err(TokenError::Rejected("서버 토큰은 관리자만 발급할 수 있습니다.".into()));
    }
    // What may be given: the vocabulary, then the kind/role bound (distinct messages).
    let allowed = issuable_scopes(kind, actor.role);
    for scope in &value.scopes {
        if !ALL_SCOPES.contains(&scope.as_str()) {
            return Err(TokenError::Rejected(format!("알 수 없는 권한입니다: {scope}")));
        }
        if !allowed.contains(&scope.as_str()) {
            return Err(TokenError::Rejected(format!("이 토큰에 줄 수 없는 권한입니다: {scope}")));
        }
    }
    if kind == TokenKind::Server && value.expires_at_ms.is_none() {
        return Err(TokenError::Invalid(BTreeMap::from([("expiresAt".into(), "만료 기간을 선택하세요.".into())])));
    }
    let owner = (kind == TokenKind::User).then(|| actor.user_id.clone());
    let expires_at = value.expires_at_ms.map(from_ms).transpose()?;

    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|_| TokenError::Unavailable)?;
    let token = format!("{}{}", prefix(kind), URL_SAFE_NO_PAD.encode(raw));
    let id = uuid::Uuid::new_v4().to_string();
    let scopes = json!(value.scopes);

    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    let inserted = tx
        .execute(
            "INSERT INTO access_tokens (id, kind, owner_user_id, name, token_hash, token_suffix, scopes, expires_at, created_by_user_id) \
             VALUES ($1, $2::text::token_kind, $3, $4, $5, $6, $7, $8, $9)",
            &[
                &id,
                &kind_str(kind),
                &owner,
                &value.name,
                &super::token_digest(&token),
                &&token[token.len() - SUFFIX_LEN..],
                &scopes,
                &expires_at,
                &actor.user_id,
            ],
        )
        .await;
    if let Err(error) = inserted {
        if error.code() == Some(&SqlState::CHECK_VIOLATION) {
            return Err(TokenError::Rejected("토큰을 저장할 수 없습니다 — 종류·주인·권한 목록이 서로 맞지 않습니다.".into()));
        }
        return Err(TokenError::Unavailable);
    }
    // Never the plaintext. Server tokens: kind only (viewers read the audit log).
    let after = match kind {
        TokenKind::Server => json!({"kind": "server"}),
        TokenKind::User => json!({"kind": "user", "name": value.name, "scopes": scopes, "expiresAt": expires_at.map(iso)}),
    };
    let entry = AuditEntry {
        actor_id: Some(actor.user_id.clone()),
        actor_label: actor.email.clone(),
        action: "token:issue".into(),
        target: Some(id.clone()),
        after: Some(after),
        ip: ctx.ip.clone(),
        ..AuditEntry::default()
    };
    audit::write(&*tx, &entry).await.map_err(unavailable)?;
    tx.commit().await.map_err(unavailable)?;
    Ok(IssuedToken { id, token })
}

fn kind_str(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::User => "user",
        TokenKind::Server => "server",
    }
}

/// `(kind, owner)` for the WHERE clause; `None` owner = the server scope.
fn scope_parts(scope: &TokenScope) -> (&'static str, Option<&str>) {
    match scope {
        TokenScope::User { owner_user_id } => ("user", Some(owner_user_id.as_str())),
        TokenScope::Server => ("server", None),
    }
}

/// Newest first, revoked ones included; never the hash.
pub async fn list(state: &AppState, actor: &SessionPrincipal, scope: &TokenScope, now_ms: i64) -> Result<Vec<TokenListRow>, TokenError> {
    // Refused before the database is touched.
    if *scope == TokenScope::Server && actor.role != Role::Admin {
        return Err(TokenError::Rejected("서버 토큰 목록은 관리자만 볼 수 있습니다.".into()));
    }
    let (kind, owner) = scope_parts(scope);
    let rows = state
        .db
        .get()
        .await
        .map_err(unavailable)?
        .query(
            "SELECT id, name, token_suffix, scopes, expires_at, created_at, last_used_at, revoked_at FROM access_tokens \
             WHERE kind = $1::text::token_kind AND owner_user_id IS NOT DISTINCT FROM $2 ORDER BY created_at DESC, id",
            &[&kind, &owner],
        )
        .await
        .map_err(unavailable)?;
    let kind = if kind == "user" { TokenKind::User } else { TokenKind::Server };
    Ok(rows
        .iter()
        .map(|r| {
            type Utc = chrono::DateTime<chrono::Utc>;
            let expires_at: Option<Utc> = r.get("expires_at");
            let revoked_at: Option<Utc> = r.get("revoked_at");
            let status = if revoked_at.is_some() {
                TokenStatus::Revoked
            } else if expires_at.is_some_and(|at| at.timestamp_millis() <= now_ms) {
                TokenStatus::Expired
            } else {
                TokenStatus::Active
            };
            let scopes: Value = r.get("scopes");
            TokenListRow {
                id: r.get("id"),
                name: r.get("name"),
                scopes: scopes.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
                expires_at: expires_at.map(iso),
                created_at: iso(r.get("created_at")),
                last_used_at: r.get::<_, Option<Utc>>("last_used_at").map(iso),
                revoked_at: revoked_at.map(iso),
                masked: format!("{}…{}", prefix(kind), r.get::<_, &str>("token_suffix")),
                status,
            }
        })
        .collect())
}

pub async fn revoke(state: &AppState, actor: &SessionPrincipal, ctx: &RequestContext, scope: &TokenScope, id: &str) -> Result<(), TokenError> {
    // About the caller, not the row: the same answer for any id, so nothing leaks.
    if *scope == TokenScope::Server && actor.role != Role::Admin {
        return Err(TokenError::Rejected("서버 토큰은 관리자만 해지할 수 있습니다.".into()));
    }
    let (kind, owner) = scope_parts(scope);
    let mut client = state.db.get().await.map_err(unavailable)?;
    let tx = client.transaction().await.map_err(unavailable)?;
    // The scope narrows in this one statement; `revoked_at IS NULL` keeps the first revocation.
    let revoked = tx
        .query_opt(
            "UPDATE access_tokens SET revoked_at = now() \
             WHERE id = $1 AND kind = $2::text::token_kind AND owner_user_id IS NOT DISTINCT FROM $3 AND revoked_at IS NULL \
             RETURNING revoked_at",
            &[&id, &kind, &owner],
        )
        .await
        .map_err(unavailable)?
        .ok_or(TokenError::NothingToRevoke)?;
    tx.execute("UPDATE gateway_tokens SET revoked_at = now() WHERE access_token_id = $1 AND revoked_at IS NULL", &[&id]).await.map_err(unavailable)?;
    let entry = AuditEntry {
        actor_id: Some(actor.user_id.clone()),
        actor_label: actor.email.clone(),
        action: "token:revoke".into(),
        target: Some(id.to_owned()),
        after: Some(json!({"revokedAt": iso(revoked.get(0))})),
        ip: ctx.ip.clone(),
        ..AuditEntry::default()
    };
    audit::write(&*tx, &entry).await.map_err(unavailable)?;
    tx.commit().await.map_err(unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::Role;

    const NOW: i64 = 1_800_000_000_000; // 2027-01-15T08:00:00Z
    const DAY: i64 = 24 * 60 * 60 * 1000;

    fn form(pairs: &[(&str, &str)]) -> FormInput {
        let mut input = FormInput::default();
        for (k, v) in pairs {
            input.fields.entry((*k).to_owned()).or_default().push((*v).to_owned());
        }
        input
    }

    #[test]
    fn issuable_scopes_follow_kind_then_role() {
        assert_eq!(issuable_scopes(TokenKind::Server, Role::Viewer), ALL_SCOPES.to_vec(), "server tokens ignore the role");
        assert_eq!(issuable_scopes(TokenKind::User, Role::Admin), ["config:read", "config:write", "nginx:reload", "user:manage"]);
        assert_eq!(issuable_scopes(TokenKind::User, Role::Editor), ["config:read", "config:write", "nginx:reload"]);
        assert_eq!(issuable_scopes(TokenKind::User, Role::Viewer), ["config:read"]);
    }

    #[test]
    fn presets_resolve_against_now() {
        for (preset, days) in [("30d", Some(30)), ("90d", Some(90)), ("1y", Some(365)), ("never", None)] {
            let got = parse_issue_form(&form(&[("name", " CI 배포용 "), ("scopes", "config:read"), ("scopes", "config:write"), ("expiry", preset)]), NOW).unwrap();
            assert_eq!(got, IssueFormValue { name: "CI 배포용".into(), scopes: vec!["config:read".into(), "config:write".into()], expires_at_ms: days.map(|d| NOW + d * DAY) });
        }
    }

    #[test]
    fn custom_dates_mean_the_end_of_that_utc_day_and_never_the_past() {
        let custom = |on: &str| parse_issue_form(&form(&[("name", "n"), ("scopes", "config:read"), ("expiry", "custom"), ("expiresOn", on)]), NOW);
        let end = chrono::DateTime::parse_from_rfc3339("2027-01-16T23:59:59.999Z").unwrap().timestamp_millis();
        assert_eq!(custom(" 2027-01-16 ").unwrap().expires_at_ms, Some(end));
        // "Today" (UTC) is still in the future until 23:59:59.999.
        assert!(custom("2027-01-15").is_ok());
        let err = |on: &str| custom(on).unwrap_err()["expiresAt"].clone();
        assert_eq!(err("2027-01-14"), "지난 날짜는 고를 수 없습니다. 더 뒤의 날짜를 고르세요.");
        assert_eq!(err(""), "만료 날짜를 입력하세요.");
        assert_eq!(err("2027-02-30"), "만료 날짜 형식이 올바르지 않습니다.");
        assert_eq!(err("next week"), "만료 날짜 형식이 올바르지 않습니다.");
    }

    #[test]
    fn field_errors_use_the_ts_texts() {
        let errors = parse_issue_form(&form(&[("name", "   "), ("expiry", "forever")]), NOW).unwrap_err();
        assert_eq!(errors["name"], "이름을 입력하세요.");
        assert_eq!(errors["scopes"], "권한을 하나 이상 선택하세요.");
        assert_eq!(errors["expiresAt"], "만료 기간을 선택하세요.");
        // 100 UTF-16 units is the limit (an emoji counts two).
        let long = "가".repeat(100);
        assert!(parse_issue_form(&form(&[("name", &long), ("scopes", "x"), ("expiry", "never")]), NOW).is_ok());
        let too_long = format!("{}🔐", "가".repeat(99));
        assert_eq!(parse_issue_form(&form(&[("name", &too_long), ("scopes", "x"), ("expiry", "never")]), NOW).unwrap_err()["name"], "이름은 100자 이하여야 합니다.");
    }
}
