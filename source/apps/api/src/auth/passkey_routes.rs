//! Browser passkey operations. Credential material stays server-side; user, then credential,
//! is the lock order shared by sign-in, enrollment, removal and 2FA disable.
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use tokio_postgres::{Row, Transaction};

use super::audit::{self, AuditEntry};
use super::passkey::{
    self, Availability, Expected, PasskeyAlg, StoredCredential, UnavailableReason,
};
use super::passkey_ceremony::{self as ceremony, Ceremony, Registration};
use super::principal::{self, SessionPrincipal, TwoFactorGate, db_unavailable, now_unix_ms};
use super::{account, auth_log, login, pending_login, rbac, session};
use crate::contracts::{FormInput, FormResult, Role};
use crate::error::ApiError;
use crate::http_policy::read_form;
use crate::request_context::RequestContext;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/ui/v1/forms/login.passkeyStart", post(signin_start))
        .route("/api/ui/v1/forms/login.passkeyFinish", post(signin_finish))
        .route("/api/ui/v1/account/passkeys", get(account_list))
        .route("/api/ui/v1/account/passkeys/{id}", get(account_detail))
        .route(
            "/api/ui/v1/forms/account.passkeys.start",
            post(registration_start),
        )
        .route(
            "/api/ui/v1/forms/account.passkeys.finish",
            post(registration_finish),
        )
        .route("/api/ui/v1/forms/account.passkeys.rename", post(rename))
        .route("/api/ui/v1/forms/account.passkeys.remove", post(remove_own))
        .route("/api/ui/v1/admin/users/{id}/passkeys", get(admin_list))
        .route(
            "/api/ui/v1/forms/admin.users.passkeys.remove",
            post(remove_admin),
        )
}

fn refused(message: &str) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, "conflict", message)
}
fn changed() -> ApiError {
    refused("인증 정보가 바뀌었거나 요청이 만료되었습니다. 다시 시작하세요.")
}
fn limited() -> ApiError {
    ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limited",
        "요청이 잦습니다. 잠시 후 다시 시도하세요.",
    )
}
fn failed<E>(_: E) -> ApiError {
    db_unavailable()
}
fn result(value: Value) -> Response {
    Json(FormResult {
        result: Some(value),
        ..Default::default()
    })
    .into_response()
}
fn redirect(path: &str) -> Response {
    Json(FormResult {
        redirect_to: Some(path.into()),
        ..Default::default()
    })
    .into_response()
}
fn with_cookies(mut response: Response, cookies: Vec<String>) -> Result<Response, ApiError> {
    for cookie in cookies {
        response.headers_mut().append(
            header::SET_COOKIE,
            HeaderValue::from_str(&cookie).map_err(|_| ApiError::internal())?,
        );
    }
    Ok(response)
}

pub fn availability_view(state: &AppState, ctx: &RequestContext) -> Value {
    match passkey::availability(ctx, state.config.auth_url.as_deref()) {
        Availability::Available { .. } => json!({"available":true,"reason":null}),
        Availability::Unavailable(reason) => json!({"available":false,"reason": match reason {
            UnavailableReason::InsecureOrigin => "insecure_origin", UnavailableReason::IpHost => "ip_host", UnavailableReason::AuthUrlHttp => "auth_url_http",
        }}),
    }
}
fn available(state: &AppState, ctx: &RequestContext) -> Result<(String, String), ApiError> {
    match passkey::availability(ctx, state.config.auth_url.as_deref()) {
        Availability::Available { rp_id, origin } => Ok((rp_id, origin)),
        Availability::Unavailable(_) => Err(refused(
            "패스키는 HTTPS 도메인 또는 http://localhost에서 사용할 수 있습니다.",
        )),
    }
}
fn current_origin(state: &AppState, ctx: &RequestContext) -> Option<String> {
    available(state, ctx).ok().map(|(_, origin)| origin)
}
fn expected(entry: &Ceremony) -> Expected<'_> {
    Expected {
        rp_id: &entry.rp_id,
        origin: &entry.origin,
        challenge: &entry.challenge,
    }
}
fn current_ceremony(
    state: &AppState,
    ctx: &RequestContext,
    entry: &Ceremony,
) -> Result<(), ApiError> {
    let (rp, origin) = available(state, ctx)?;
    if entry.rp_id != rp || entry.origin != origin || now_unix_ms() >= entry.expires_ms {
        return Err(changed());
    }
    Ok(())
}
fn replace_ceremony(state: &AppState, headers: &HeaderMap, ctx: &RequestContext) {
    for (_, id) in session::request_cookies(headers)
        .iter()
        .filter(|(name, _)| name == ceremony::cookie_name(ctx.secure))
    {
        state.runtime.passkey_ceremonies.remove(id);
    }
}
fn take_ceremony(
    state: &AppState,
    headers: &HeaderMap,
    ctx: &RequestContext,
    registration: bool,
) -> Result<Ceremony, ApiError> {
    let id = ceremony::read_cookie(headers, ctx.secure).ok_or_else(changed)?;
    state
        .runtime
        .passkey_ceremonies
        .take(&id, registration, now_unix_ms())
        .ok_or_else(changed)
}
fn start_budget(state: &AppState, ctx: &RequestContext) -> Result<(), ApiError> {
    if state
        .runtime
        .login_throttle
        .begin_passkey_start(ctx.ip.as_deref().unwrap_or("unknown"), now_unix_ms())
    {
        Ok(())
    } else {
        Err(limited())
    }
}
fn passkey_name(form: &FormInput) -> Result<String, ApiError> {
    let name = crate::crypto::js_trim(form.get("name").unwrap_or(""));
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(refused("이름을 1~64자로 입력하세요."));
    }
    Ok(name.into())
}

async fn signin_start(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    start_budget(&state, &ctx)?;
    let (rp_id, origin) = available(&state, &ctx)?;
    replace_ceremony(&state, &headers, &ctx);
    let (id, challenge) = state
        .runtime
        .passkey_ceremonies
        .create(rp_id.clone(), origin, None, now_unix_ms())
        .ok_or_else(ApiError::internal)?;
    with_cookies(
        result(
            json!({"publicKey":{"challenge":URL_SAFE_NO_PAD.encode(challenge),"rpId":rp_id,"userVerification":"required","allowCredentials":[],"timeout":300000}}),
        ),
        vec![ceremony::set_cookie(ctx.secure, &id)],
    )
}

/// Every failed finish consumes its ceremony and counts against the shared address ceiling,
/// including malformed forms. Success releases only this attempt, never prior IP failures.
async fn signin_finish(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Response, ApiError> {
    let headers = request.headers().clone();
    let entry = take_ceremony(&state, &headers, &ctx, false);
    let throttle = &state.runtime.login_throttle;
    let ip_attempt = ctx
        .ip
        .as_deref()
        .map(|ip| throttle.begin_ip(ip, now_unix_ms()));
    let ip_locked = matches!(ip_attempt, Some(None));
    let outcome = if ip_locked {
        Err(limited())
    } else {
        match entry {
            Ok(entry) => signin_finish_inner(&state, &ctx, &headers, request, entry).await,
            Err(e) => Err(e),
        }
    };
    let reason = if ip_locked {
        Some(auth_log::FailureReason::IpThrottled)
    } else {
        match &outcome {
            Ok((_, reason)) => *reason,
            Err(_) => Some(auth_log::FailureReason::BadCredentials),
        }
    };
    if let Some(reason) = reason {
        if let Some(Some(attempt)) = ip_attempt {
            attempt.keep();
        }
        auth_log::append(
            &state.config.auth_log_file,
            ctx.ip.as_deref(),
            reason,
            now_unix_ms(),
        );
        super::routes::audit_lock_transitions(&state, &ctx, "", now_unix_ms()).await;
    }
    let response = match outcome {
        Ok((response, _)) => response,
        Err(_) => redirect("/login?error=passkey"),
    };
    with_cookies(response, vec![ceremony::clear_cookie(ctx.secure)])
}

fn alg(value: i32) -> Result<PasskeyAlg, ApiError> {
    match value {
        -7 => Ok(PasskeyAlg::Es256),
        -257 => Ok(PasskeyAlg::Rs256),
        _ => Err(ApiError::internal()),
    }
}
fn verify_assertion(
    input: &passkey::AssertionInput,
    entry: &Ceremony,
    key: &Row,
    handle: &[u8],
) -> Result<passkey::VerifiedAssertion, ApiError> {
    let stored = StoredCredential {
        public_key_spki: key.get("public_key"),
        algorithm: alg(key.get("algorithm"))?,
        backup_eligible: key.get("backup_eligible"),
        rp_id: key.get("rp_id"),
        origin: key.get("origin"),
        user_handle: handle,
    };
    passkey::verify_assertion(input, &expected(entry), &stored).map_err(|_| changed())
}

async fn signin_finish_inner(
    state: &AppState,
    ctx: &RequestContext,
    headers: &HeaderMap,
    request: Request,
    entry: Ceremony,
) -> Result<(Response, Option<auth_log::FailureReason>), ApiError> {
    current_ceremony(state, ctx, &entry)?;
    let form = read_form(request).await?;
    let input =
        passkey::parse_assertion(form.get("credential").unwrap_or("")).map_err(|_| changed())?;
    let key = {
        let client = state.db.get().await.map_err(failed)?;
        client.query_opt("SELECT p.*, u.passkey_user_handle FROM user_passkeys p JOIN users u ON u.id=p.user_id WHERE credential_id=$1 AND rp_id=$2", &[&input.raw_id(),&entry.rp_id]).await.map_err(failed)?.ok_or_else(changed)?
    };
    let user_id: String = key.get("user_id");
    let account_key = format!("passkey:user:{user_id}");
    let attempt = state
        .runtime
        .login_throttle
        .begin_account(&account_key, now_unix_ms())
        .ok_or_else(limited)?;
    let outcome = async {
        let handle: Option<Vec<u8>> = key.get("passkey_user_handle");
        verify_assertion(&input,&entry,&key,handle.as_deref().ok_or_else(changed)?)?;
        let mut client = state.db.get().await.map_err(failed)?;
        let tx = client.transaction().await.map_err(failed)?;
        let user = tx.query_opt("SELECT email, role::text AS role, session_version, enabled, approved, password_hash, totp_enabled, passkey_user_handle FROM users WHERE id=$1 FOR UPDATE", &[&user_id]).await.map_err(failed)?.ok_or_else(changed)?;
        let key_id: String = key.get("id");
        let locked = tx.query_opt("SELECT * FROM user_passkeys WHERE id=$1 AND user_id=$2 AND rp_id=$3 FOR UPDATE", &[&key_id,&user_id,&entry.rp_id]).await.map_err(failed)?.ok_or_else(changed)?;
        let handle: Option<Vec<u8>> = user.get("passkey_user_handle");
        let verified = verify_assertion(&input,&entry,&locked,handle.as_deref().ok_or_else(changed)?)?;
        current_ceremony(state,ctx,&entry)?;
        if user.get::<_,Option<String>>("password_hash").is_none() { return Err(changed()); }
        if !user.get::<_,bool>("enabled") { return Ok((redirect("/login?error=disabled"),Some(auth_log::FailureReason::AccountDisabled))); }
        if !user.get::<_,bool>("approved") { return Ok((redirect("/login?error=pending"),Some(auth_log::FailureReason::AccountPending))); }
        if !passkey::counter_accepts(locked.get::<_,i64>("sign_count") as u32,verified.sign_count) {
            audit::write(&*tx,&AuditEntry { actor_id:Some(user_id.clone()),action:"auth:passkey-counter".into(),target:Some(key_id),ip:ctx.ip.clone(),..Default::default() }).await.map_err(failed)?;
            tx.commit().await.map_err(failed)?;
            return Err(changed());
        }
        let role = rbac::parse_role(user.get("role")).ok_or_else(ApiError::internal)?;
        let policy = tx.query_opt("SELECT require_2fa_admin, require_2fa_user FROM app_settings LIMIT 1",&[]).await.map_err(failed)?;
        let requires_2fa = policy.is_some_and(|p|p.get::<_,bool>(if role == Role::Admin {0} else {1}));
        let authorized = login::AuthorizedUser { id:user_id.clone(), email:user.get("email"),role,session_version:user.get("session_version"),totp_enabled:user.get("totp_enabled"),requires_2fa,mfa_verified:true,external_login:false };
        // Encode before commit: a failed cookie cannot leave a successful login audit behind.
        let now = now_unix_ms()/1000;
        let name = session::cookie_name(ctx.secure);
        let token = session::encode(&authorized.session_claims(),state.config.auth_secrets.first().ok_or_else(ApiError::internal)?,name,now,session::SESSION_MAX_AGE_SECS).map_err(|_|ApiError::internal())?;
        tx.execute("UPDATE user_passkeys SET sign_count=$2, backup_state=$3, last_used_at=now() WHERE id=$1", &[&key_id,&i64::from(verified.sign_count),&verified.backup_state]).await.map_err(failed)?;
        audit::write(&*tx,&AuditEntry { actor_id:Some(user_id.clone()),action:"login".into(),after:Some(json!({"method":"passkey","passkeyId":key_id})),ip:ctx.ip.clone(),..Default::default() }).await.map_err(failed)?;
        current_ceremony(state,ctx,&entry)?;
        tx.commit().await.map_err(failed)?;
        let mut cookies = session::issue_cookies(name,&token,&session::request_cookies(headers),ctx.secure,now+session::SESSION_MAX_AGE_SECS);
        for (_,id) in session::request_cookies(headers).iter().filter(|(n,_)|n == pending_login::cookie_name(ctx.secure)) { state.runtime.pending_logins.remove(id); }
        cookies.push(pending_login::clear_cookie(ctx.secure));
        Ok((with_cookies(redirect("/"),cookies)?,None))
    }.await;
    if matches!(&outcome, Ok((_, None))) {
        state.runtime.login_throttle.clear_account(&account_key);
    } else if outcome.is_err() {
        attempt.keep();
    }
    outcome
}

async fn own_principal(
    state: &AppState,
    headers: &HeaderMap,
    ctx: &RequestContext,
) -> Result<SessionPrincipal, ApiError> {
    principal::require_session(state, headers, ctx, None, TwoFactorGate::Enforce).await
}
async fn admin_principal(
    state: &AppState,
    headers: &HeaderMap,
    ctx: &RequestContext,
) -> Result<SessionPrincipal, ApiError> {
    principal::require_session(
        state,
        headers,
        ctx,
        Some(rbac::Action::UserManage),
        TwoFactorGate::Enforce,
    )
    .await
}
pub(super) fn ensure_verified(principal: &SessionPrincipal) -> Result<(), ApiError> {
    if principal.totp_enabled && !principal.mfa_verified {
        Err(principal::two_factor_verification_required())
    } else {
        Ok(())
    }
}
async fn password_proof(
    state: &AppState,
    principal: &SessionPrincipal,
    form: &FormInput,
) -> Result<String, ApiError> {
    let hash = {
        let client = state.db.get().await.map_err(failed)?;
        client
            .query_opt(
                "SELECT password_hash FROM users WHERE id=$1",
                &[&principal.user_id],
            )
            .await
            .map_err(failed)?
            .ok_or_else(ApiError::unauthorized)?
            .get::<_, Option<String>>(0)
            .ok_or_else(|| refused("이 계정은 비밀번호를 사용하지 않습니다."))?
    };
    account::check_password_against(
        state,
        &principal.user_id,
        form.get("currentPassword").unwrap_or(""),
        hash.clone(),
    )
    .await?;
    Ok(hash)
}
pub(super) async fn lock_user(tx: &Transaction<'_>, id: &str) -> Result<Row, ApiError> {
    tx.query_opt("SELECT session_version, password_hash, enabled, approved, totp_enabled, passkey_user_handle FROM users WHERE id=$1 FOR UPDATE",&[&id]).await.map_err(failed)?.ok_or_else(ApiError::unauthorized)
}
pub(super) fn same_account(row: &Row, version: i32, hash: &str, mfa_verified: bool) -> Result<(), ApiError> {
    if row.get::<_, i32>("session_version") != version
        || row.get::<_, Option<String>>("password_hash").as_deref() != Some(hash)
        || !row.get::<_, bool>("enabled")
        || !row.get::<_, bool>("approved")
        || (row.get::<_, bool>("totp_enabled") && !mfa_verified)
    {
        return Err(changed());
    }
    Ok(())
}
pub(super) async fn remove_all_passkeys(tx: &Transaction<'_>, actor: &str, user_id: &str, ctx: &RequestContext, reason: &str) -> Result<u64, ApiError> {
    let count = tx.execute("DELETE FROM user_passkeys WHERE user_id=$1", &[&user_id]).await.map_err(failed)?;
    if count > 0 {
        write_audit(tx, actor, user_id, ctx, "user:passkey-remove", json!({"passkeyId": null, "count": count, "reason": reason})).await?;
    }
    Ok(count)
}
async fn count_limit(tx: &Transaction<'_>, id: &str) -> Result<(), ApiError> {
    let count: i64 = tx
        .query_one(
            "SELECT count(*) FROM user_passkeys WHERE user_id=$1",
            &[&id],
        )
        .await
        .map_err(failed)?
        .get(0);
    if count >= 10 {
        Err(refused("패스키는 최대 10개까지 등록할 수 있습니다."))
    } else {
        Ok(())
    }
}
async fn bump(tx: &Transaction<'_>, id: &str) -> Result<i32, ApiError> {
    Ok(tx.query_one("UPDATE users SET session_version=session_version+1 WHERE id=$1 RETURNING session_version",&[&id]).await.map_err(failed)?.get(0))
}
async fn write_audit(
    tx: &Transaction<'_>,
    actor: &str,
    target: &str,
    ctx: &RequestContext,
    action: &str,
    after: Value,
) -> Result<(), ApiError> {
    audit::write(
        tx,
        &AuditEntry {
            actor_id: Some(actor.into()),
            target: Some(target.into()),
            action: action.into(),
            after: Some(after),
            ip: ctx.ip.clone(),
            ..Default::default()
        },
    )
    .await
    .map_err(failed)
}

async fn registration_start(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Response, ApiError> {
    let headers = request.headers().clone();
    let principal = own_principal(&state, &headers, &ctx).await?;
    ensure_verified(&principal)?;
    start_budget(&state, &ctx)?;
    let (rp_id, origin) = available(&state, &ctx)?;
    let form = read_form(request).await?;
    let name = passkey_name(&form)?;
    let hash = password_proof(&state, &principal, &form).await?;
    let (handle, exclude) = {
        let mut client = state.db.get().await.map_err(failed)?;
        let tx = client.transaction().await.map_err(failed)?;
        let user = lock_user(&tx, &principal.user_id).await?;
        same_account(
            &user,
            principal.session_version,
            &hash,
            principal.mfa_verified,
        )?;
        count_limit(&tx, &principal.user_id).await?;
        let mut random = vec![0; 32];
        getrandom::fill(&mut random).map_err(|_| ApiError::internal())?;
        let handle: Vec<u8> = tx.query_one("UPDATE users SET passkey_user_handle=COALESCE(passkey_user_handle,$2) WHERE id=$1 RETURNING passkey_user_handle",&[&principal.user_id,&random]).await.map_err(failed)?.get(0);
        let rows = tx
            .query(
                "SELECT credential_id, transports FROM user_passkeys WHERE user_id=$1",
                &[&principal.user_id],
            )
            .await
            .map_err(failed)?;
        let exclude: Vec<Value> = rows.iter().map(|r|json!({"type":"public-key","id":URL_SAFE_NO_PAD.encode(r.get::<_,&[u8]>(0)),"transports":r.get::<_,Option<Vec<String>>>(1).unwrap_or_default()})).collect();
        tx.commit().await.map_err(failed)?;
        (handle, exclude)
    };
    replace_ceremony(&state, &headers, &ctx);
    let registration = Registration {
        user_id: principal.user_id.clone(),
        session_version: principal.session_version,
        password_hash: hash,
        user_handle: handle.clone(),
        name,
        mfa_verified: principal.mfa_verified,
    };
    let (id, challenge) = state
        .runtime
        .passkey_ceremonies
        .create(rp_id.clone(), origin, Some(registration), now_unix_ms())
        .ok_or_else(ApiError::internal)?;
    let user_name = principal.email.as_deref().unwrap_or(&principal.user_id);
    let public_key = json!({"challenge":URL_SAFE_NO_PAD.encode(challenge),"rp":{"id":rp_id,"name":"nginx-web-ui"},"user":{"id":URL_SAFE_NO_PAD.encode(handle),"name":user_name,"displayName":user_name},"pubKeyCredParams":[{"type":"public-key","alg":-7},{"type":"public-key","alg":-257}],"authenticatorSelection":{"residentKey":"required","requireResidentKey":true,"userVerification":"required"},"attestation":"none","extensions":{"credProps":true},"excludeCredentials":exclude,"timeout":300000});
    with_cookies(
        result(json!({"publicKey":public_key})),
        vec![ceremony::set_cookie(ctx.secure, &id)],
    )
}

async fn registration_finish(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Response, ApiError> {
    let headers = request.headers().clone();
    let entry = take_ceremony(&state, &headers, &ctx, true);
    let response = async {
        let entry = entry?;
        let principal = own_principal(&state,&headers,&ctx).await?;
        ensure_verified(&principal)?;
        current_ceremony(&state,&ctx,&entry)?;
        let reg = entry.registration.as_ref().ok_or_else(changed)?;
        if reg.user_id != principal.user_id || reg.session_version != principal.session_version { return Err(changed()); }
        let form = read_form(request).await?;
        let input = passkey::parse_registration(form.get("credential").unwrap_or("")).map_err(|e|refused(&e.to_string()))?;
        let verified = passkey::verify_registration(&input,&expected(&entry)).map_err(|e|refused(&e.to_string()))?;
        let mut client = state.db.get().await.map_err(failed)?;
        let tx = client.transaction().await.map_err(failed)?;
        let user = lock_user(&tx,&reg.user_id).await?;
        same_account(&user,reg.session_version,&reg.password_hash,reg.mfa_verified && principal.mfa_verified)?;
        if user.get::<_,Option<Vec<u8>>>("passkey_user_handle").as_deref() != Some(reg.user_handle.as_slice()) { return Err(changed()); }
        current_ceremony(&state,&ctx,&entry)?;
        count_limit(&tx,&reg.user_id).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let algorithm: i32 = match verified.algorithm { PasskeyAlg::Es256 => -7, PasskeyAlg::Rs256 => -257 };
        let inserted = tx.execute("INSERT INTO user_passkeys (id,user_id,credential_id,public_key,algorithm,rp_id,origin,sign_count,backup_eligible,backup_state,transports,name) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT (credential_id) DO NOTHING",&[&id,&reg.user_id,&verified.credential_id,&verified.public_key_spki,&algorithm,&entry.rp_id,&entry.origin,&i64::from(verified.sign_count),&verified.backup_eligible,&verified.backup_state,&verified.transports,&reg.name]).await.map_err(failed)?;
        if inserted != 1 { return Err(refused("이미 등록된 패스키입니다.")); }
        let version = bump(&tx,&reg.user_id).await?;
        write_audit(&tx,&reg.user_id,&reg.user_id,&ctx,"user:passkey-add",json!({"passkeyId":id,"name":reg.name})).await?;
        current_ceremony(&state,&ctx,&entry)?;
        let cookies = account::reissue(&state,&headers,&ctx,&[("sv",version.into())])?;
        tx.commit().await.map_err(failed)?;
        with_cookies(redirect(&format!("/account/passkeys/{id}?added=1")),cookies)
    }.await;
    with_cookies(
        response.unwrap_or_else(IntoResponse::into_response),
        vec![ceremony::clear_cookie(ctx.secure)],
    )
}

fn row_view(row: &Row, origin: Option<&str>) -> Value {
    let stored: String = row.get("origin");
    json!({"id":row.get::<_,String>("id"),"name":row.get::<_,String>("name"),"rpId":row.get::<_,String>("rp_id"),"origin":stored,"createdAt":row.get::<_,chrono::DateTime<chrono::Utc>>("created_at").to_rfc3339(),"lastUsedAt":row.get::<_,Option<chrono::DateTime<chrono::Utc>>>("last_used_at").map(|at|at.to_rfc3339()),"backupEligible":row.get::<_,bool>("backup_eligible"),"backupState":row.get::<_,bool>("backup_state"),"unavailableHere":origin != Some(stored.as_str())})
}
async fn list_view(
    state: &AppState,
    ctx: &RequestContext,
    user_id: &str,
    actor_id: &str,
) -> Result<Value, ApiError> {
    let client = state.db.get().await.map_err(failed)?;
    let has_password: bool = client
        .query_opt(
            "SELECT password_hash IS NOT NULL FROM users WHERE id=$1",
            &[&user_id],
        )
        .await
        .map_err(failed)?
        .ok_or_else(ApiError::not_found)?
        .get(0);
    let actor_has_password: bool = client
        .query_one(
            "SELECT password_hash IS NOT NULL FROM users WHERE id=$1",
            &[&actor_id],
        )
        .await
        .map_err(failed)?
        .get(0);
    let rows = client.query("SELECT id,name,rp_id,origin,created_at,last_used_at,backup_eligible,backup_state FROM user_passkeys WHERE user_id=$1 ORDER BY created_at,id",&[&user_id]).await.map_err(failed)?;
    let origin = current_origin(state, ctx);
    let passkeys: Vec<Value> = rows
        .iter()
        .map(|row| row_view(row, origin.as_deref()))
        .collect();
    Ok(
        json!({"passkeys":passkeys,"availability":availability_view(state,ctx),"currentOrigin":origin,"hasPassword":has_password,"canRemove":actor_has_password}),
    )
}
async fn account_list(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let principal = own_principal(&state, &headers, &ctx).await?;
    list_view(&state, &ctx, &principal.user_id, &principal.user_id)
        .await
        .map(Json)
}
async fn admin_list(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let principal = admin_principal(&state, &headers, &ctx).await?;
    list_view(&state, &ctx, &id, &principal.user_id)
        .await
        .map(Json)
}
async fn account_detail(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let principal = own_principal(&state, &headers, &ctx).await?;
    let mut view = list_view(&state, &ctx, &principal.user_id, &principal.user_id).await?;
    let rows = view.as_object_mut().unwrap().remove("passkeys").unwrap();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .cloned()
        .ok_or_else(ApiError::not_found)?;
    view["passkey"] = row;
    Ok(Json(view))
}

async fn rename(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Response, ApiError> {
    let principal = own_principal(&state, request.headers(), &ctx).await?;
    let form = read_form(request).await?;
    let name = passkey_name(&form)?;
    let id = form.get("id").unwrap_or("");
    let mut client = state.db.get().await.map_err(failed)?;
    let tx = client.transaction().await.map_err(failed)?;
    let row = lock_user(&tx, &principal.user_id).await?;
    if row.get::<_, i32>("session_version") != principal.session_version
        || !row.get::<_, bool>("enabled")
        || !row.get::<_, bool>("approved")
    {
        return Err(changed());
    }
    let changed = tx
        .execute(
            "UPDATE user_passkeys SET name=$3 WHERE id=$1 AND user_id=$2",
            &[&id, &principal.user_id, &name],
        )
        .await
        .map_err(failed)?;
    if changed == 0 {
        return Err(ApiError::not_found());
    }
    write_audit(
        &tx,
        &principal.user_id,
        &principal.user_id,
        &ctx,
        "user:passkey-rename",
        json!({"passkeyId":id,"name":name}),
    )
    .await?;
    tx.commit().await.map_err(failed)?;
    Ok(redirect(&format!("/account/passkeys/{id}?renamed=1")))
}
async fn remove_own(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Response, ApiError> {
    remove(state, ctx, request, false).await
}
async fn remove_admin(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Response, ApiError> {
    remove(state, ctx, request, true).await
}
async fn remove(
    state: AppState,
    ctx: RequestContext,
    request: Request,
    admin: bool,
) -> Result<Response, ApiError> {
    let headers = request.headers().clone();
    let principal = if admin {
        admin_principal(&state, &headers, &ctx).await?
    } else {
        own_principal(&state, &headers, &ctx).await?
    };
    if !admin {
        ensure_verified(&principal)?;
    }
    let form = read_form(request).await?;
    let hash = password_proof(&state, &principal, &form).await?;
    let target = if admin {
        form.get("userId")
            .filter(|id| !id.is_empty())
            .ok_or_else(ApiError::not_found)?
    } else {
        &principal.user_id
    };
    let id = form.get("id").filter(|id| !id.is_empty());
    if !admin && id.is_none() {
        return Err(ApiError::not_found());
    }
    if admin && id.is_none() && !matches!(form.all("all"), [value] if value == "1") {
        return Err(refused("삭제할 패스키를 선택하거나 전체 삭제를 확인하세요."));
    }
    let mut client = state.db.get().await.map_err(failed)?;
    let tx = client.transaction().await.map_err(failed)?;
    // Admin and target can differ: use one deterministic user lock order to avoid reciprocal
    // removals deadlocking, then lock/delete credentials only after both users are held.
    let mut ids = vec![principal.user_id.as_str(), target];
    ids.sort_unstable();
    ids.dedup();
    for user_id in ids {
        let row = lock_user(&tx, user_id).await.map_err(|error| {
            if user_id != principal.user_id && error.status == StatusCode::UNAUTHORIZED {
                ApiError::not_found()
            } else {
                error
            }
        })?;
        if user_id == principal.user_id {
            same_account(
                &row,
                principal.session_version,
                &hash,
                admin || principal.mfa_verified,
            )?;
        }
    }
    let count = if let Some(id) = id {
        tx.execute(
            "DELETE FROM user_passkeys WHERE id=$1 AND user_id=$2",
            &[&id, &target],
        )
        .await
        .map_err(failed)?
    } else {
        tx.execute("DELETE FROM user_passkeys WHERE user_id=$1", &[&target])
            .await
            .map_err(failed)?
    };
    if count == 0 {
        return Err(ApiError::not_found());
    }
    let version = bump(&tx, target).await?;
    write_audit(
        &tx,
        &principal.user_id,
        target,
        &ctx,
        "user:passkey-remove",
        json!({"passkeyId":id,"count":count}),
    )
    .await?;
    let cookies = if target == principal.user_id {
        account::reissue(&state, &headers, &ctx, &[("sv", version.into())])?
    } else {
        Vec::new()
    };
    tx.commit().await.map_err(failed)?;
    let path = if admin {
        format!("/admin/users/{target}?passkeysRemoved=1")
    } else {
        "/account?removed=1".into()
    };
    with_cookies(redirect(&path), cookies)
}
