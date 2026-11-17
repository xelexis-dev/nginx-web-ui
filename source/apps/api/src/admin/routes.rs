//! Admin HTTP surface: live `user:manage`, 2FA enforced; token admission follows the explicit
//! route census. Cookie writes retain the trusted same-origin boundary.
//!
//! | Route | TS source |
//! | --- | --- |
//! | `GET /api/ui/v1/admin/users` | `admin/users/page.tsx` |
//! | `GET /api/ui/v1/admin/users/{id}` | `admin/users/[id]/page.tsx`, `…/edit/page.tsx` (404 if missing) |
//! | `GET /api/ui/v1/admin/pending` | `admin/pending/page.tsx` |
//! | `POST /api/ui/v1/forms/admin.users.create` | `createUser` → `/admin/users/<id>` |
//! | `POST /api/ui/v1/forms/admin.users.updateRole` | `updateUserRole` → `/admin/users/<id>` |
//! | `POST /api/ui/v1/forms/admin.users.resetPassword` | `resetPassword` → `/admin/users/<id>` |
//! | `POST /api/ui/v1/forms/admin.users.delete` | `deleteUser` → `/admin/users` |
//! | `POST /api/ui/v1/forms/admin.users.toggle` | `toggleUser` → `{refresh: true}` (stays on the page) |
//! | `POST /api/ui/v1/forms/admin.pending.approve` | `approveUser` → `/admin/pending` |
//! | `POST /api/ui/v1/forms/admin.pending.reject` | `rejectUser` → `/admin/pending` |
//! | `GET /api/ui/v1/admin/security` | `admin/security/page.tsx` + `connected-gateways.tsx` |
//! | `POST /api/ui/v1/forms/admin.security.updateTwoFactorPolicy` | `updateTwoFactorPolicy` → `{refresh: true}` |
//! | `POST /api/ui/v1/forms/admin.security.revokeGateway` | `revokeGatewayToken` → `{refresh: true}` |
//! | `GET /api/ui/v1/admin/tokens`, `/admin/tokens/{id}` | server tokens `{tokens, issuableScopes}` / one (404) |
//! | `POST /api/ui/v1/forms/admin.tokens.issue` | `issueServerTokenAction` → `{result: {token, id, name}}` |
//! | `POST /api/ui/v1/forms/admin.tokens.revoke` | `revokeServerTokenAction` → `{refresh: true}` |
//! | `GET /api/ui/v1/admin/oauth`, `/admin/oauth/{id}` | `admin/oauth/page.tsx`, `[id]/page.tsx` (+`callbackUrl`) |
//! | `POST /api/ui/v1/forms/admin.oauth.{create,update}` | `createProvider`/`updateProvider` → `/admin/oauth/<id>` |
//! | `POST /api/ui/v1/forms/admin.oauth.{delete,toggle}` | `deleteProvider`/`toggleProvider` → `/admin/oauth` |
//!
//! Refusals: 400 `validation_failed` + `fieldErrors`, 409 `conflict` + the TS message, 404.

use axum::extract::{Path, Request, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde_json::Value;

use super::oauth;
use super::security::{self, TwoFactorPolicy};
use super::settings;
use super::users::{self, NewUser};
use crate::auth::principal::{UserPrincipal, TwoFactorGate, require_user, Use};
use crate::auth::rbac::Action;
use crate::auth::tokens::{self, TokenKind, TokenScope};
use crate::contracts::{FormInput, FormResult};
use crate::error::ApiError;
use crate::http_policy::read_form;
use crate::request_context::RequestContext;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/ui/v1/admin/users", get(list_users))
        .route("/api/ui/v1/admin/users/{id}", get(user_detail))
        .route("/api/ui/v1/admin/pending", get(pending_users))
        .route("/api/ui/v1/forms/admin.users.create", post(create_user))
        .route("/api/ui/v1/forms/admin.users.updateRole", post(update_role))
        .route("/api/ui/v1/forms/admin.users.resetPassword", post(reset_password))
        .route("/api/ui/v1/forms/admin.users.delete", post(delete_user))
        .route("/api/ui/v1/forms/admin.users.toggle", post(toggle_user))
        .route("/api/ui/v1/forms/admin.pending.approve", post(approve_user))
        .route("/api/ui/v1/forms/admin.pending.reject", post(reject_user))
        .route("/api/ui/v1/admin/security", get(security_view))
        .route("/api/ui/v1/forms/admin.security.updateTwoFactorPolicy", post(update_policy))
        .route("/api/ui/v1/forms/admin.security.revokeGateway", post(revoke_gateway))
        .route("/api/ui/v1/admin/tokens", get(list_server_tokens))
        .route("/api/ui/v1/admin/tokens/{id}", get(server_token_detail))
        .route("/api/ui/v1/forms/admin.tokens.issue", post(issue_server_token))
        .route("/api/ui/v1/forms/admin.tokens.revoke", post(revoke_server_token))
        .route("/api/ui/v1/admin/oauth", get(list_providers))
        .route("/api/ui/v1/admin/oauth/{id}", get(provider_detail))
        .route("/api/ui/v1/forms/admin.oauth.create", post(create_provider))
        .route("/api/ui/v1/forms/admin.oauth.update", post(update_provider))
        .route("/api/ui/v1/forms/admin.oauth.delete", post(delete_provider))
        .route("/api/ui/v1/forms/admin.oauth.toggle", post(toggle_provider))
        .route("/api/ui/v1/admin/settings/general", get(general_settings))
        .route("/api/ui/v1/forms/admin.settings.updatePublicBaseUrl", post(update_public_base_url))
        .route("/api/ui/v1/forms/admin.settings.updateMetrics", post(update_metrics))
        .route("/api/ui/v1/admin/secrets", get(secrets_health))
        .merge(super::webhooks::router())
        .merge(super::mail_recipients::router())
        .merge(super::smtp::router())
}

async fn secrets_health(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    manager(&state, &headers, &ctx).await?;
    to_json(super::secrets::view(&state).await?)
}

fn message(text: String) -> Json<FormResult> {
    Json(FormResult { result: Some(serde_json::json!({"ok": true, "message": text})), refresh: Some(true), ..FormResult::default() })
}

async fn general_settings(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    manager(&state, &headers, &ctx).await?;
    to_json(settings::view(&state).await?)
}

async fn update_public_base_url(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    Ok(message(settings::update_public_base_url(&state, actor.session()?, &ctx, &form).await?))
}

async fn update_metrics(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    Ok(message(settings::update_metrics(&state, &actor, &ctx, &form).await?))
}

async fn list_providers(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    manager(&state, &headers, &ctx).await?;
    to_json(oauth::list(&state).await?)
}

async fn provider_detail(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    manager(&state, &headers, &ctx).await?;
    to_json(oauth::detail(&state, &id).await?)
}

async fn create_provider(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    let id = oauth::create(&state, actor.session()?, &ctx, &form).await?;
    Ok(redirect(format!("/admin/oauth/{id}")))
}

async fn update_provider(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    let id = oauth::update(&state, actor.session()?, &ctx, &form).await?;
    Ok(redirect(format!("/admin/oauth/{id}")))
}

async fn delete_provider(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    oauth::delete(&state, actor.session()?, &ctx, form.get("id").unwrap_or("")).await?;
    Ok(redirect("/admin/oauth"))
}

async fn toggle_provider(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    oauth::toggle(&state, actor.session()?, &ctx, form.get("id").unwrap_or("")).await?;
    Ok(redirect("/admin/oauth"))
}

// Server tokens: the scope is `Server` in code only — never from the form or URL.

async fn list_server_tokens(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let actor = require_user(&state, &headers, &ctx, Some(Action::UserManage), TwoFactorGate::Enforce, Use::SessionOnly).await?;
    tokens::list_view(&state, actor.session()?, &TokenScope::Server).await.map(Json)
}

async fn server_token_detail(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let actor = require_user(&state, &headers, &ctx, Some(Action::UserManage), TwoFactorGate::Enforce, Use::SessionOnly).await?;
    tokens::detail_view(&state, actor.session()?, &TokenScope::Server, &id).await.map(Json)
}

async fn issue_server_token(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    tokens::issue_from_form(&state, actor.session()?, &ctx, TokenKind::Server, &form).await.map(Json)
}

async fn revoke_server_token(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    tokens::revoke_from_form(&state, actor.session()?, &ctx, &TokenScope::Server, &form).await.map(Json)
}

/// Stay on the page. `result.ok` lets the browser action state clear a previous error (TS
/// `{ ok: true }`); a refresh alone would leave the stale state in place.
fn refresh() -> Json<FormResult> {
    Json(FormResult { result: Some(serde_json::json!({"ok": true})), refresh: Some(true), ..FormResult::default() })
}

async fn security_view(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    manager(&state, &headers, &ctx).await?;
    to_json(security::view(&state).await?)
}

/// Checkboxes as TS: only the literal `"true"` enables a switch; an absent field is off.
async fn update_policy(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    let on = |name: &str| -> Result<bool,ApiError> {
        if form.all(name).iter().any(|value|!matches!(value.as_str(),"true"|"false")) {
            return Err(super::field(name,"true 또는 false로 입력하세요.").into());
        }
        Ok(form.get(name)==Some("true"))
    };
    let next = TwoFactorPolicy { require2fa_admin: on("require2faAdmin")?, require2fa_user: on("require2faUser")?, require2fa_oauth: on("require2faOauth")? };
    security::update_policy(&state, actor.session()?, &ctx, next).await?;
    Ok(refresh())
}

async fn revoke_gateway(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    security::revoke_gateway(&state, actor.session()?, &ctx, form.get("id").unwrap_or("")).await?;
    Ok(refresh())
}

pub(super) async fn manager(state: &AppState, headers: &HeaderMap, ctx: &RequestContext) -> Result<UserPrincipal, ApiError> {
    require_user(state, headers, ctx, Some(Action::UserManage), TwoFactorGate::Enforce, Use::SessionOrToken).await
}

/// Authorizes from the headers, then reads the form body.
pub(super) async fn manager_form(state: &AppState, ctx: &RequestContext, request: Request) -> Result<(UserPrincipal, FormInput), ApiError> {
    let usage = crate::auth::route_policy::route(request.method().as_str(), request.uri().path()).map_or(Use::SessionOnly, |r|r.usage);
    let actor = require_user(state, request.headers(), ctx, Some(Action::UserManage), TwoFactorGate::Enforce, usage).await?;
    actor.check_origin(request.headers(), ctx)?;
    Ok((actor, read_form(request).await?))
}

pub(super) fn redirect(to: impl Into<String>) -> Json<FormResult> {
    Json(FormResult { redirect_to: Some(to.into()), ..FormResult::default() })
}

pub(super) fn to_json<T: serde::Serialize>(value: T) -> Result<Json<Value>, ApiError> {
    serde_json::to_value(value).map(Json).map_err(|_| ApiError::internal())
}

async fn list_users(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    manager(&state, &headers, &ctx).await?;
    to_json(users::list(&state).await?)
}

async fn user_detail(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    manager(&state, &headers, &ctx).await?;
    to_json(users::detail(&state, &id).await?)
}

async fn pending_users(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    manager(&state, &headers, &ctx).await?;
    to_json(users::pending(&state).await?)
}

async fn create_user(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    let input = NewUser {
        email: form.get("email").unwrap_or(""),
        password: form.get("password").unwrap_or(""),
        confirm_password: form.get("confirmPassword").unwrap_or(""),
        role: form.get("role").unwrap_or(""),
    };
    let id = users::create(&state, actor.session()?, &ctx, input).await?;
    Ok(redirect(format!("/admin/users/{id}")))
}

async fn update_role(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    let id = form.get("id").unwrap_or("").trim().to_owned();
    users::update_role(&state, &actor, &ctx, &id, form.get("role").unwrap_or("")).await?;
    Ok(redirect(format!("/admin/users/{id}")))
}

async fn reset_password(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let authenticated_at = (!crate::auth::user_token::is_user_token_request(request.headers())).then(|| crate::auth::principal::session_claims(&state, request.headers(), &ctx, crate::auth::principal::now_unix_ms() / 1000)).flatten()
        .and_then(|claims| crate::auth::session::authentication_time(&claims));
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    let id = form.get("id").unwrap_or("").trim().to_owned();
    users::reset_password(&state, actor.session()?, &ctx, &id, form.get("password").unwrap_or(""), form.get("confirmPassword").unwrap_or(""), authenticated_at).await?;
    Ok(redirect(format!("/admin/users/{id}")))
}

async fn delete_user(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    users::delete(&state, actor.session()?, &ctx, form.get("id").unwrap_or("")).await?;
    Ok(redirect("/admin/users"))
}

async fn toggle_user(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    users::toggle(&state, &actor, &ctx, form.get("id").unwrap_or("")).await?;
    Ok(refresh())
}

async fn approve_user(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    users::approve(&state, &actor, &ctx, form.get("id").unwrap_or(""), form.get("role").unwrap_or("")).await?;
    Ok(redirect("/admin/pending"))
}

async fn reject_user(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let (actor, form) = manager_form(&state, &ctx, request).await?;
    users::reject(&state, &actor, &ctx, form.get("id").unwrap_or("")).await?;
    Ok(redirect("/admin/pending"))
}
