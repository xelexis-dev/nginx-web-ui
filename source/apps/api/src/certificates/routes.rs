//! Certificate HTTP commands share the durable issuance service with browser forms.
use super::{service, views};
use crate::{
    auth::{
        principal::{self, TwoFactorGate},
        rbac::Action,
    },
    contracts::{Actor, FormResult},
    error::ApiError,
    http_policy::{check_browser_origin, read_form},
    request_context::RequestContext,
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};

pub const FORM_IDS: &[&str] = &[
    "certificates.createCertificate",
    "certificates.updateCertificate",
    "certificates.retryCertificate",
    "certificates.submitDnsChallengeAction",
    "certificates.renewDnsCertificate",
    "certificates.renewCertificate",
    "certificates.deleteCertificate",
    "certificates.importCertificate",
    "certificates.replaceCertificate",
];
pub fn router() -> Router<AppState> {
    let mut router = Router::new()
        .route("/api/acme-challenge/{token}", get(public_challenge))
        .route("/.well-known/acme-challenge/{token}", get(public_challenge))
        .route("/api/v1/certificates", get(node_list).post(node_create))
        .route("/api/v1/certificates/import", post(node_import))
        .route(
            "/api/v1/certificates/{id}",
            get(node_detail).patch(node_update).delete(node_delete),
        );
    for operation in [
        "retry",
        "renew",
        "renew-dns",
        "dns-challenge",
        "cancel",
        "replace",
    ] {
        router = router.route(
            &format!("/api/v1/certificates/{{id}}/{operation}"),
            post(node_operation),
        );
    }
    for command in FORM_IDS {
        router = router.route(&format!("/api/ui/v1/forms/{command}"), post(form));
    }
    router
        .route("/api/certificates/{id}/cancel", post(browser_cancel))
        .route("/api/ui/v1/certificates/{id}/cancel", post(browser_cancel))
}
async fn public_challenge(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Response, ApiError> {
    let mut status = StatusCode::NOT_FOUND;
    let mut value = "not found".to_owned();
    if !token.is_empty()
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        let client = state
            .db
            .get()
            .await
            .map_err(|_| principal::db_unavailable())?;
        if let Some(row) = client
            .query_opt(
                "SELECT key_authorization FROM acme_challenges WHERE token=$1",
                &[&token],
            )
            .await
            .map_err(|_| principal::db_unavailable())?
        {
            status = StatusCode::OK;
            value = row.get(0);
        }
    }
    Ok((
        status,
        [
            ("content-type", "text/plain"),
            ("cache-control", "no-store"),
        ],
        value,
    )
        .into_response())
}
async fn json_body(request: Request) -> Result<Value, ApiError> {
    let bytes = axum::body::to_bytes(request.into_body(), 2 * 1024 * 1024)
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "JSON 본문을 읽을 수 없습니다.",
            )
        })?;
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "JSON 객체가 필요합니다.",
            )
        })
}
async fn node_list(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    principal::require_gateway(&state, request.headers(), &ctx, &[Action::ConfigRead]).await?;
    Ok(Json(json!({"certificates":views::list(&state).await?})))
}
async fn node_detail(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<String>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    principal::require_gateway(&state, request.headers(), &ctx, &[Action::ConfigRead]).await?;
    Ok(Json(
        json!({"certificate":views::detail(&state,&id).await?}),
    ))
}
async fn node_create(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Response, ApiError> {
    let actor = principal::require_gateway(&state, request.headers(), &ctx, &[Action::ConfigWrite])
        .await?
        .actor(&ctx);
    let input = service::parse_input(&json_body(request).await?)?;
    let id = service::create(&state, &input, &actor).await?;
    Ok((StatusCode::CREATED, Json(json!({"certificate":{"id":id}}))).into_response())
}
async fn node_import(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Response, ApiError> {
    let actor = principal::require_gateway(&state, request.headers(), &ctx, &[Action::ConfigWrite])
        .await?
        .actor(&ctx);
    let input = service::parse_pem_input(&json_body(request).await?)?;
    let id = service::import(&state, &input, &actor).await?;
    Ok((StatusCode::CREATED, Json(json!({"certificate":{"id":id}}))).into_response())
}
async fn node_update(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<String>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let actor = principal::require_gateway(&state, request.headers(), &ctx, &[Action::ConfigWrite])
        .await?
        .actor(&ctx);
    let input = service::parse_input(&json_body(request).await?)?;
    service::update(&state, &id, &input, &actor).await?;
    Ok(Json(json!({"certificate":{"id":id}})))
}
async fn node_delete(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<String>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let actor = principal::require_gateway(&state, request.headers(), &ctx, &[Action::ConfigWrite])
        .await?
        .actor(&ctx);
    service::remove(&state, &id, &actor).await?;
    Ok(Json(json!({"deleted":true})))
}
fn action(operation: &str) -> Action {
    if matches!(operation, "renew" | "renew-dns" | "replace") {
        Action::NginxReload
    } else {
        Action::ConfigWrite
    }
}
async fn operate(
    state: &AppState,
    id: &str,
    operation: &str,
    actor: &Actor,
) -> Result<Value, ApiError> {
    match operation {
        "retry" => {
            service::retry(state, id, actor).await?;
            Ok(json!({"retrying":true}))
        }
        "renew" => service::renew(state, id, actor).await,
        "renew-dns" => {
            service::renew_dns(state, id, actor).await?;
            Ok(json!({"started":true}))
        }
        "dns-challenge" => service::submit_dns(state, id, actor).await,
        "cancel" => {
            service::cancel(state, id, actor).await?;
            Ok(json!({"cancelled":true}))
        }
        _ => Err(ApiError::not_found()),
    }
}
pub fn replace_response(result: Value) -> Result<Value, ApiError> {
    if let Some(error) = result["applyError"].as_str().filter(|s| !s.is_empty()) {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "apply_failed",
            error,
        ));
    }
    Ok(json!({"certificate":{"id":result["id"]},"uncovered":result["uncovered"]}))
}
async fn node_operation(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<String>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let operation = request
        .uri()
        .path()
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_owned();
    let actor = principal::require_gateway(&state, request.headers(), &ctx, &[action(&operation)])
        .await?
        .actor(&ctx);
    let result = if operation == "replace" {
        let input = service::parse_pem_input(&json_body(request).await?)?;
        replace_response(service::replace(&state, &id, &input, &actor).await?)?
    } else {
        operate(&state, &id, &operation, &actor).await?
    };
    Ok(Json(result))
}
async fn browser_cancel(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    check_browser_origin(request.headers(), &ctx)?;
    let session = principal::require_session(
        &state,
        request.headers(),
        &ctx,
        Some(Action::ConfigWrite),
        TwoFactorGate::Enforce,
    )
    .await?;
    match service::cancel(&state, &id, &session.actor(&ctx)).await {
        Ok(()) => Ok(Json(json!({"ok":true})).into_response()),
        Err(error) if error.status.is_client_error() => Ok((
            error.status,
            Json(json!({"ok":false,"error":error.message})),
        )
            .into_response()),
        Err(error) => Err(error),
    }
}
async fn form(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<FormResult>, ApiError> {
    check_browser_origin(request.headers(), &ctx)?;
    let name = request
        .uri()
        .path()
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_owned();
    let operation = match name.as_str() {
        "certificates.createCertificate" => "create",
        "certificates.updateCertificate" => "update",
        "certificates.retryCertificate" => "retry",
        "certificates.submitDnsChallengeAction" => "dns-challenge",
        "certificates.renewDnsCertificate" => "renew-dns",
        "certificates.renewCertificate" => "renew",
        "certificates.deleteCertificate" => "delete",
        "certificates.importCertificate" => "import",
        "certificates.replaceCertificate" => "replace",
        _ => return Err(ApiError::not_found()),
    };
    let session = principal::require_session(
        &state,
        request.headers(),
        &ctx,
        Some(action(operation)),
        TwoFactorGate::Enforce,
    )
    .await?;
    let input = read_form(request).await?;
    if !input.args.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_arguments",
            "잘못된 요청입니다.",
        ));
    }
    let id = input.get("id").unwrap_or("").trim();
    if !matches!(operation, "create" | "import") && id.is_empty() {
        return Ok(Json(FormResult {
            result: Some(json!({"ok":false,"error":"잘못된 요청입니다."})),
            ..Default::default()
        }));
    }
    let actor = session.actor(&ctx);
    let mutation=async{
        let mut output=FormResult{refresh:Some(true),..Default::default()};
        match operation{
            "create"|"update"=>{
                let parsed=service::parse_input(&service::form_input(&input))?;
                let id=if operation=="create"{service::create(&state,&parsed,&actor).await?}else{service::update(&state,id,&parsed,&actor).await?;id.to_owned()};
                output.redirect_to=Some(format!("/certificates/{}",crate::nodes::views::segment(&id)));
            },
            "import"|"replace"=>{
                let parsed=service::parse_pem_input(&json!({"certPem":input.get("certPem").unwrap_or(""),"keyPem":input.get("keyPem").unwrap_or("")}))?;
                if operation=="import"{let id=service::import(&state,&parsed,&actor).await?;output.redirect_to=Some(format!("/certificates/{}",crate::nodes::views::segment(&id)));}
                else{
                    let result=service::replace(&state,id,&parsed,&actor).await?;
                    if let Some(error)=result["applyError"].as_str().filter(|s|!s.is_empty()){output.result=Some(json!({"ok":false,"error":error}));}
                    else{
                        let names=result["uncovered"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(",");
                        output.redirect_to=Some(format!("/certificates/{}{}",crate::nodes::views::segment(id),if names.is_empty(){String::new()}else{format!("?uncovered={}",crate::nodes::views::segment(&names))}));
                    }
                }
            },
            "delete"=>{service::remove(&state,id,&actor).await?;output.redirect_to=Some("/certificates".into());},
            _=>{
                let result=operate(&state,id,operation,&actor).await?;
                output.result=Some(if result["notPropagated"]==true{
                    let names=result["pending"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", ");
                    json!({"ok":false,"notPropagated":true,"pending":result["pending"],"error":format!("아직 TXT 레코드가 보이지 않습니다: {names} — 전파를 기다린 뒤 다시 확인하세요.")})
                }else{json!({"ok":true})});
            },
        }
        Ok::<_,ApiError>(output)
    }.await;
    match mutation {
        Ok(output) => Ok(Json(output)),
        Err(error) if error.status.is_client_error() => Ok(Json(FormResult {
            result: Some(
                json!({"ok":false,"error":error.message,"fieldErrors":error.field_errors}),
            ),
            refresh: if matches!(operation, "renew" | "dns-challenge") {
                Some(true)
            } else {
                None
            },
            ..Default::default()
        })),
        Err(error) => Err(error),
    }
}
