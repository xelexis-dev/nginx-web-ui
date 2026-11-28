//! Explicit remote operations. No request can supply an arbitrary method or API path.
use super::{
    client::{
        APPLY_TIMEOUT, CONFIG_APPLY_TIMEOUT, DEFAULT_TIMEOUT, FailureKind, LOG_STREAM_HANDSHAKE,
        NodeResult, RENEW_TIMEOUT,
    },
    service, views,
};
use crate::{
    auth::{
        principal::{self, TwoFactorGate},
        rbac::Action,
    },
    contracts::{FormInput, FormResult},
    error::ApiError,
    http_policy::read_form,
    request_context::RequestContext,
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    body::{Body, Bytes},
    extract::{Path, Query, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use reqwest::Method;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

pub const FORM_IDS: &[&str] = &[
    "node.createNodeUpstream",
    "node.updateNodeUpstream",
    "node.deleteNodeUpstream",
    "node.createNodeServer",
    "node.updateNodeServer",
    "node.deleteNodeServer",
    "node.toggleNodeServer",
    "node.createNodeStream",
    "node.updateNodeStream",
    "node.deleteNodeStream",
    "node.toggleNodeStream",
    "node.applyNodeConfig",
    "node.rollbackNodeSnapshot",
    "node.createNodeCertificate",
    "node.updateNodeCertificate",
    "node.deleteNodeCertificate",
    "node.retryNodeCertificate",
    "node.renewNodeCertificate",
    "node.renewDnsNodeCertificate",
    "node.submitNodeDnsChallenge",
    "node.importNodeCertificate",
    "node.replaceNodeCertificate",
];
pub fn router() -> Router<AppState> {
    let mut router = Router::new()
        .route("/api/nodes/{id}/metrics", get(metrics))
        .route("/api/nodes/{id}/logs/stream", get(logs))
        .route("/api/nodes/{id}/certificates/{cid}/cancel", post(cancel));
    for id in FORM_IDS {
        router = router.route(&format!("/api/ui/v1/forms/{id}"), post(form));
    }
    router
}
pub fn failure_status(kind: &FailureKind) -> StatusCode {
    match kind {
        FailureKind::Unauthorized => StatusCode::UNAUTHORIZED,
        FailureKind::Forbidden => StatusCode::FORBIDDEN,
        FailureKind::NotFound => StatusCode::NOT_FOUND,
        FailureKind::Validation => StatusCode::BAD_REQUEST,
        FailureKind::DomainFailure => StatusCode::UNPROCESSABLE_ENTITY,
        FailureKind::Conflict => StatusCode::CONFLICT,
        FailureKind::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::BAD_GATEWAY,
    }
}
fn failure_response(result: NodeResult, cancel: bool) -> Response {
    match result {
        NodeResult::Failure { kind, message, .. } => {
            let status = failure_status(&kind);
            (
                status,
                Json(if cancel {
                    json!({"ok":false,"error":message,"code":kind})
                } else {
                    json!({"error":{"code":kind,"message":message}})
                }),
            )
                .into_response()
        }
        _ => ApiError::internal().into_response(),
    }
}
async fn relay_access(
    state: &AppState,
    ctx: &RequestContext,
    headers: &axum::http::HeaderMap,
    id: &str,
    action: Action,
    audit_target: &str,
) -> Result<(service::NodeAccess, crate::contracts::Actor), ApiError> {
    let session =
        principal::require_user(state, headers, ctx, None, TwoFactorGate::Enforce, crate::auth::principal::Use::SessionOrToken).await?;
    if action != Action::ConfigRead { session.check_origin(headers, ctx)?; }
    let actor = session.actor(ctx);
    match service::resolve(state, &session, id, action).await {
        Ok(a) => Ok((a, actor)),
        Err(e) => {
            if e.status == StatusCode::FORBIDDEN {
                if let Ok(client) = state.db.get().await {
                    let _ = crate::auth::audit::write(
                        &**client,
                        &crate::auth::audit::AuditEntry {
                            actor_id: actor.id,
                            via: actor.via,
                            action: "rbac:denied".into(),
                            target: Some(audit_target.into()),
                            before: Some(
                                json!({"role":session.role_label(),"nodeId":id,"requiredAction":action}),
                            ),
                            ip: actor.ip,
                            ..Default::default()
                        },
                    )
                    .await;
                }
            }
            Err(e)
        }
    }
}
async fn metrics(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<String>,
    Query(query): Query<BTreeMap<String, String>>,
    request: Request,
) -> Result<Response, ApiError> {
    let (mut access, _) = relay_access(
        &state,
        &ctx,
        request.headers(),
        &id,
        Action::ConfigRead,
        "node:metrics:read",
    )
    .await?;
    if let Err(failure @ NodeResult::Failure { kind: FailureKind::Unreachable, .. }) = service::prepare_access(&state, &mut access, false).await {
        return Ok(failure_response(failure, false));
    }
    let path = if let Some(range) = query.get("range") {
        format!("/api/v1/metrics?range={}", views::segment(range))
    } else {
        "/api/v1/metrics".into()
    };
    match state
        .node_http
        .call(
            &state.secrets,
            &access.target,
            Method::GET,
            &path,
            None,
            DEFAULT_TIMEOUT,
        )
        .await
    {
        NodeResult::Success { value, .. } => Ok(Json(value).into_response()),
        failure => Ok(failure_response(failure, false)),
    }
}
/// Receiver drop is cancellation: the task owns and drops the upstream response.
struct ByteStream(tokio::sync::mpsc::Receiver<Result<Bytes, std::io::Error>>);
impl futures_core::Stream for ByteStream {
    type Item = Result<Bytes, std::io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}
pub fn stream_body(
    mut upstream: reqwest::Response,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Body {
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    tokio::spawn(async move {
        // Cancel the entire relay, including a send blocked by a slow consumer.
        // Dropping this read-only response closes the remote streaming request.
        let relay = async {
            // A buffering front may wait for body bytes before forwarding headers.
            if tx.send(Ok(Bytes::from_static(b": keepalive\n\n"))).await.is_err() {
                return;
            }
            loop {
                match upstream.chunk().await {
                    Ok(Some(bytes)) => {
                        if tx.send(Ok(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(_) => {
                        let _ = tx
                            .send(Err(std::io::Error::other("node stream disconnected")))
                            .await;
                        break;
                    }
                }
            }
        };
        tokio::select! {
            biased;
            _ = shutdown.wait_for(|value| *value) => {},
            _ = tx.closed() => {},
            _ = relay => {},
        }
    });
    Body::from_stream(ByteStream(rx))
}
async fn logs(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<String>,
    Query(query): Query<BTreeMap<String, String>>,
    request: Request,
) -> Result<Response, ApiError> {
    let (mut access, _) = relay_access(
        &state,
        &ctx,
        request.headers(),
        &id,
        Action::ConfigRead,
        "node:logs:stream",
    )
    .await?;
    if let Err(failure @ NodeResult::Failure { kind: FailureKind::Unreachable, .. }) = service::prepare_access(&state, &mut access, false).await {
        return Ok(failure_response(failure, false));
    }
    let path = format!(
        "/api/v1/logs/stream?file={}",
        views::segment(query.get("file").map(String::as_str).unwrap_or("access"))
    );
    let checked_state = state.clone();
    let checked_headers = request.headers().clone();
    let checked_ctx = ctx.clone();
    let mut shutdown = crate::auth::stream_guard::checked_shutdown(
        &state, crate::auth::stream_guard::REVALIDATE_EVERY, move || {
            let state = checked_state.clone();
            let headers = checked_headers.clone();
            let ctx = checked_ctx.clone();
            let id = id.clone();
            async move {
                let checked = async {
                    let session = principal::require_user(&state, &headers, &ctx, Some(Action::ConfigRead), TwoFactorGate::Enforce, crate::auth::principal::Use::SessionOrToken).await?;
                    service::resolve(&state, &session, &id, Action::ConfigRead).await.map(|_| ())
                }.await;
                // Token streams fail closed on node lookup errors too; session outage behaviour stays intact.
                if crate::auth::user_token::is_user_token_request(&headers) { checked.map_err(|_| ApiError::forbidden()) } else { checked }
            }
        },
    );
    let opened = tokio::select! {
        biased;
        _ = shutdown.wait_for(|stopping| *stopping) => {
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE, "not_ready", "API 서버가 준비되지 않았습니다.",
            ));
        }
        result = state.node_http.open_stream(&state.secrets, &access.target, &path, LOG_STREAM_HANDSHAKE) => result,
    };
    match opened {
        Err(failure) => Ok(failure_response(failure, false)),
        Ok(upstream) => Ok(Response::builder()
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache, no-transform")
            .header("x-accel-buffering", "no")
            // Ends with the browser session that opened it (see `stream_guard`).
            .body(stream_body(
                upstream,
                shutdown,
            ))
            .unwrap()),
    }
}
async fn cancel(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path((id, cid)): Path<(String, String)>,
    request: Request,
) -> Result<Response, ApiError> {
    let (mut access, actor) = relay_access(
        &state,
        &ctx,
        request.headers(),
        &id,
        Action::ConfigWrite,
        "node:cert:cancel",
    )
    .await?;
    if matches!(cid.as_str(), "." | "..") {
        return Err(ApiError::not_found());
    }
    let started = std::time::Instant::now();
    if let Err(failure) = service::prepare_access(&state, &mut access, true).await {
        return Ok(failure_response(failure, true));
    }
    let result = state
        .node_http
        .call(
            &state.secrets,
            &access.target,
            Method::POST,
            &format!("/api/v1/certificates/{}/cancel", views::segment(&cid)),
            None,
            DEFAULT_TIMEOUT.saturating_sub(started.elapsed()),
        )
        .await;
    bookkeeping(
        &state,
        &access,
        &actor,
        "cert:cancel",
        json!({"certificateId":cid}),
        &result,
        false,
    )
    .await;
    if result.is_ok() {
        Ok(Json(json!({"ok":true})).into_response())
    } else {
        Ok(failure_response(result, true))
    }
}
pub struct Command {
    pub method: Method,
    pub path: String,
    pub action: Action,
    pub audit: &'static str,
    pub detail: Value,
    pub timeout: Duration,
    pub body: Option<Value>,
    pub base: String,
    pub retry: String,
    pub id: String,
    pub envelope: Option<&'static str>,
    pub redirect: bool,
    pub title: Option<&'static str>,
}
fn bad() -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "validation_failed",
        "잘못된 요청입니다.",
    )
}
pub fn command(name: &str, node_id: &str, form: &FormInput) -> Result<Command, ApiError> {
    if !FORM_IDS.contains(&name) {
        return Err(ApiError::not_found());
    }
    if form.get("nodeVersionFamily").is_some_and(|family| !matches!(family, "1.19" | "2.0")) { return Err(bad()); }
    let operation = name.strip_prefix("node.").unwrap();
    let root = format!("/nodes/{}", views::segment(node_id));
    let id = if operation == "rollbackNodeSnapshot" {
        form.args.get(1).map(String::as_str).unwrap_or("")
    } else {
        form.get("id").unwrap_or("").trim()
    };
    let resource = if operation.contains("Upstream") {
        "upstreams"
    } else if operation.contains("Server") {
        "servers"
    } else if operation.contains("Stream") {
        "streams"
    } else if operation.contains("Certificate") || operation == "submitNodeDnsChallenge" {
        "certificates"
    } else if operation == "rollbackNodeSnapshot" {
        "history"
    } else {
        "apply"
    };
    let suffix = match resource {
        "upstreams" => "reverse-proxy/upstreams",
        "servers" => "reverse-proxy",
        "streams" => "stream",
        r => r,
    };
    let mut c = Command {
        method: Method::POST,
        path: format!("/api/v1/{resource}"),
        action: Action::ConfigWrite,
        audit: "",
        detail: json!({}),
        timeout: DEFAULT_TIMEOUT,
        body: None,
        base: format!("{root}/{suffix}"),
        retry: String::new(),
        id: id.into(),
        envelope: None,
        redirect: false,
        title: None,
    };
    let creating = operation.starts_with("create") || operation == "importNodeCertificate";
    if !creating && resource != "apply" && (id.is_empty() || matches!(id, "." | "..")) {
        return Err(bad());
    }
    let singular = match resource {
        "upstreams" => "upstream",
        "servers" => "server",
        "streams" => "stream",
        "certificates" => "certificate",
        _ => "",
    };
    if !id.is_empty() {
        c.detail[format!("{singular}Id")] = json!(id);
    }
    let is_edit = operation.starts_with("update");
    if creating || is_edit {
        c.body = Some(match resource {
            "upstreams" => crate::control::resources::form_body(
                crate::control::resources::Resource::Upstream,
                form,
            )?,
            "servers" => crate::control::resources::form_body(
                crate::control::resources::Resource::Server,
                form,
            )?,
            "streams" => crate::control::resources::form_body(
                crate::control::resources::Resource::Stream,
                form,
            )?,
            _ => crate::certificates::service::form_input(form),
        });
        c.redirect = true;
        if creating {
            c.envelope = Some(singular);
        } else {
            c.method = Method::PATCH;
        }
    }
    if !creating && resource != "apply" {
        c.path.push('/');
        c.path.push_str(&views::segment(id));
    }
    c.retry = if creating {
        format!("{}/new", c.base)
    } else if is_edit {
        format!("{}/{}/edit", c.base, views::segment(id))
    } else {
        format!("{}/{}", c.base, views::segment(id))
    };
    c.audit = match operation {
        "createNodeUpstream" => "upstream:create",
        "updateNodeUpstream" => "upstream:update",
        "deleteNodeUpstream" => "upstream:delete",
        "createNodeServer" => "server:create",
        "updateNodeServer" => "server:update",
        "deleteNodeServer" => "server:delete",
        "toggleNodeServer" => "server:toggle",
        "createNodeStream" => "stream:create",
        "updateNodeStream" => "stream:update",
        "deleteNodeStream" => "stream:delete",
        "toggleNodeStream" => "stream:toggle",
        "createNodeCertificate" => "cert:create",
        "updateNodeCertificate" => "cert:update",
        "deleteNodeCertificate" => "cert:delete",
        "retryNodeCertificate" => "cert:retry",
        "renewNodeCertificate" => "cert:renew",
        "renewDnsNodeCertificate" => "cert:renew-dns",
        "submitNodeDnsChallenge" => "cert:dns-submit",
        "importNodeCertificate" => "cert:import",
        "replaceNodeCertificate" => "cert:replace",
        "applyNodeConfig" => "apply",
        "rollbackNodeSnapshot" => "rollback",
        _ => return Err(ApiError::not_found()),
    };
    if operation.starts_with("delete") {
        c.method = Method::DELETE;
        c.redirect = true;
    }
    if operation.starts_with("toggle") {
        c.path.push_str("/toggle");
    }
    match operation {
        "applyNodeConfig" => {
            c.action = Action::NginxReload;
            c.timeout = CONFIG_APPLY_TIMEOUT;
            let base = form.args.get(1).map(String::as_str).unwrap_or("");
            if ![format!("{root}/reverse-proxy"), format!("{root}/stream")]
                .contains(&base.to_string())
            {
                return Err(bad());
            }
            c.base = base.into();
            c.retry = base.into();
        }
        "rollbackNodeSnapshot" => {
            c.action = Action::NginxReload;
            c.timeout = CONFIG_APPLY_TIMEOUT;
            c.path.push_str("/rollback");
            c.detail = json!({"snapshotId":id});
            c.retry = c.base.clone();
        }
        "retryNodeCertificate" => c.path.push_str("/retry"),
        "renewNodeCertificate" => {
            c.path.push_str("/renew");
            c.action = Action::NginxReload;
            c.timeout = RENEW_TIMEOUT;
            c.title = Some("인증서 갱신 실패");
        }
        "renewDnsNodeCertificate" => {
            c.path.push_str("/renew-dns");
            c.action = Action::NginxReload;
            c.title = Some("인증서 갱신 시작 실패");
        }
        "submitNodeDnsChallenge" => {
            c.path.push_str("/dns-challenge");
            c.title = Some("TXT 검증 시작 실패");
        }
        "importNodeCertificate" => {
            c.path.push_str("/import");
            c.retry = format!("{}/import", c.base);
            c.body = Some(
                json!({"certPem":form.get("certPem").unwrap_or(""),"keyPem":form.get("keyPem").unwrap_or("")}),
            );
        }
        "replaceNodeCertificate" => {
            c.path.push_str("/replace");
            c.action = Action::NginxReload;
            c.timeout = APPLY_TIMEOUT;
            c.redirect = true;
            c.retry = format!("{}/{}/replace", c.base, views::segment(id));
            c.title = Some("인증서는 교체됐지만 적용에 실패했습니다");
            c.body = Some(
                json!({"certPem":form.get("certPem").unwrap_or(""),"keyPem":form.get("keyPem").unwrap_or("")}),
            );
        }
        _ => {}
    }
    if let Some(body) = c.body.as_mut() {
        // Browser number inputs become JSON numbers on the gateway wire, including old nodes.
        for key in ["listen"] {
            if let Some(raw) = body[key].as_str() {
                body[key] = number(raw);
            }
        }
        if resource == "servers" {
            if body["targetMode"] != "locations" {
                body["targetMode"] = json!("single");
            }
            if let Some(raw) = body["tls"]["tlsPort"].as_str() {
                body["tls"]["tlsPort"] = number(raw);
            }
            if body["certificateId"]
                .as_str()
                .is_none_or(|s| s.trim().is_empty())
            {
                if let Some(value) = body["tls"]["tlsOptions"].get("http2") {
                    if !value.is_boolean() {
                        return Err(crate::control::resources::field_error("http2", "HTTP/2 설정은 true 또는 false여야 합니다."));
                    }
                    if value == true {
                        return Err(crate::control::resources::field_error("http2", "HTTP/2를 사용하려면 인증서를 선택하세요."));
                    }
                }
                body["certificateId"] = Value::Null;
                body["tls"] = Value::Null;
            }
        }
        for key in [
            "name",
            "serverName",
            "proxyPass",
            "upstreamId",
            "certificateId",
        ] {
            if let Some(s) = body[key].as_str() {
                body[key] = json!(s.trim());
            }
        }
        for key in ["servers", "domains"] {
            if let Some(values) = body[key].as_array() {
                body[key] = json!(
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                );
            }
        }
    }
    if let Some(body) = &c.body {
        let keys: &[&str] = match operation {
            "createNodeUpstream" | "updateNodeUpstream" => &["name"],
            "createNodeServer" => &["serverName", "listen"],
            "updateNodeServer" => &["serverName"],
            "createNodeStream" | "updateNodeStream" => &["listen", "protocol"],
            "createNodeCertificate" => &["domains", "staging", "challengeType"],
            "updateNodeCertificate" => &["domains", "challengeType"],
            _ => &[],
        };
        for key in keys {
            c.detail[*key] = body[*key].clone();
        }
    }
    Ok(c)
}
fn number(raw: &str) -> Value {
    let raw = raw.trim();
    if raw.is_empty() {
        json!(0)
    } else {
        raw.parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    }
}
async fn bookkeeping(
    state: &AppState,
    access: &service::NodeAccess,
    actor: &crate::contracts::Actor,
    audit: &str,
    mut detail: Value,
    result: &NodeResult,
    skip_validation: bool,
) {
    if service::record_probe(state, &access.node.id, result)
        .await
        .is_err()
    {
        eprintln!("[nodes] probe write failed");
    }
    if skip_validation
        && matches!(
            result,
            NodeResult::Failure {
                kind: FailureKind::Validation,
                ..
            }
        )
    {
        return;
    }
    detail["ok"] = json!(result.is_ok());
    if service::audit_node(state, &access.node, audit, actor, detail)
        .await
        .is_err()
    {
        eprintln!("[nodes] audit write failed");
    }
}
fn form_failure(node: &str, c: &Command, failure: NodeResult) -> Value {
    if let NodeResult::Failure {
        kind,
        message,
        field_errors,
        ..
    } = &failure
        && (matches!(kind, FailureKind::Validation) || field_errors.is_some())
    {
        if c.audit == "apply" {
            return form_failure(
                node,
                c,
                NodeResult::failure(FailureKind::NodeError, message, None),
            );
        }
        return json!({"ok":false,"error":message,"fieldErrors":field_errors});
    }
    let error = match &failure {
        NodeResult::Failure { message, .. } => message.as_str(),
        _ => "노드가 예상과 다른 응답을 보냈습니다.",
    };
    let mut result = views::failed(node, &c.retry, &c.base, failure.clone(), None);
    result["error"] = json!(error);
    if let Some(title) = c.title {
        result["nodeError"]["domainFailureTitle"] = json!(title);
    }
    result
}
pub fn check_form_version(name: &str, input: &FormInput, previous: Option<&str>, current: Option<&str>) -> Result<(), NodeResult> {
    let replacement = matches!(name, "node.updateNodeServer" | "node.updateNodeStream" | "node.updateNodeUpstream");
    let legacy = |version: Option<&str>| version.and_then(crate::auth::node_version::parse_semver)
        .is_some_and(|v| (v.major, v.minor) == (1, 19));
    let modern = current.and_then(crate::auth::node_version::parse_semver).is_some_and(|v| v.major >= 2);
    if replacement && modern && (input.get("nodeVersionFamily") == Some("1.19")
        || (input.get("nodeVersionFamily").is_none() && legacy(previous))) {
        return Err(NodeResult::failure(FailureKind::Conflict,
            "노드가 2.0으로 업데이트되었습니다. 화면을 새로 고친 뒤 다시 저장하세요.", current.map(str::to_owned)));
    }
    Ok(())
}

pub fn supports_http2(version: Option<&str>) -> bool {
    version.and_then(crate::auth::node_version::parse_semver)
        .is_some_and(|v| v.major >= 2 && (v.major > 2 || v.minor >= 1))
}
fn http2_requested(name: &str, body: Option<&Value>) -> bool {
    matches!(name, "node.createNodeServer" | "node.updateNodeServer")
        && body.is_some_and(|b| b["tls"]["tlsOptions"]["http2"] == true)
}
pub fn check_http2_capability(name: &str, body: Option<&mut Value>, version: Option<&str>) -> Result<(), NodeResult> {
    if !matches!(name, "node.createNodeServer" | "node.updateNodeServer") { return Ok(()); }
    let Some(body) = body else { return Ok(()); };
    if body["tls"]["tlsOptions"].get("http2").is_some_and(|value| !value.is_boolean()) {
        let message = "HTTP/2 설정은 true 또는 false여야 합니다.";
        return Err(NodeResult::Failure { ok:false, kind:FailureKind::Validation, message:message.into(), node_version:version.map(str::to_owned), field_errors:Some(BTreeMap::from([("http2".into(), message.into())])) });
    }
    if !supports_http2(version) {
        if body["tls"]["tlsOptions"]["http2"] == true {
            let message = "이 노드는 HTTP/2 설정을 지원하지 않습니다. 노드를 2.1 이상으로 업데이트하세요.";
            return Err(NodeResult::Failure { ok:false, kind:FailureKind::Conflict, message:message.into(), node_version:version.map(str::to_owned), field_errors:Some(BTreeMap::from([("http2".into(), message.into())])) });
        }
    }
    // Canonical off also stays safe if a cached node version is stale.
    if body["tls"]["tlsOptions"]["http2"] == false {
        if let Some(options) = body["tls"].get_mut("tlsOptions").and_then(Value::as_object_mut) { options.remove("http2"); }
    }
    Ok(())
}

async fn form(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<FormResult>, ApiError> {
    let session = principal::require_user(
        &state,
        request.headers(),
        &ctx,
        None,
        TwoFactorGate::Enforce, crate::auth::principal::Use::SessionOrToken)
    .await?;
    session.check_origin(request.headers(), &ctx)?;
    let name = request
        .uri()
        .path()
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_owned();
    let input = read_form(request).await?;
    let arity = if matches!(
        name.as_str(),
        "node.applyNodeConfig" | "node.rollbackNodeSnapshot"
    ) {
        2
    } else {
        1
    };
    if input.args.len() != arity {
        return Err(bad());
    }
    let node = &input.args[0];
    if node.is_empty() {
        return Err(bad());
    }
    let mut c = match command(&name, node, &input) {
        Ok(c) => c,
        Err(e) if e.status == StatusCode::BAD_REQUEST => {
            return Ok(Json(FormResult {
                result: Some(json!({"ok":false,"error":e.message,"fieldErrors":e.field_errors})),
                ..Default::default()
            }));
        }
        Err(e) => return Err(e),
    };
    let mut access = match service::resolve(&state, &session, node, c.action).await {
        Ok(a) => a,
        Err(e) if e.status == StatusCode::FORBIDDEN && !session.is_token() => {
            let failure = NodeResult::failure(
                FailureKind::Forbidden,
                "이 작업을 수행할 권한이 없습니다.",
                None,
            );
            return Ok(Json(FormResult {
                result: Some(if name == "node.rollbackNodeSnapshot" {
                    serde_json::to_value(failure).unwrap()
                } else {
                    form_failure(node, &c, failure)
                }),
                ..Default::default()
            }));
        }
        Err(e) => return Err(e),
    };
    if matches!(name.as_str(), "node.rollbackNodeSnapshot" | "node.replaceNodeCertificate") { service::resolve(&state, &session, node, Action::ConfigWrite).await?; }
    let started = std::time::Instant::now();
    let previous_version = access.target.node_version.clone();
    let requested_http2 = http2_requested(&name, c.body.as_ref());
    let prepared = if requested_http2 { service::probe_access(&state, &mut access).await }
        else { service::prepare_access(&state, &mut access, true).await };
    let prepared = prepared.map_err(|mut failure| {
        if requested_http2 {
            if let NodeResult::Failure { kind, node_version, field_errors, message, .. } = &mut failure {
                if matches!(kind, FailureKind::NodeError | FailureKind::TooOld | FailureKind::VersionMismatch)
                    && !supports_http2(node_version.as_deref()) {
                    *field_errors = Some(BTreeMap::from([("http2".into(), message.clone())]));
                }
            }
        }
        failure
    });
    let prepared = prepared.and_then(|_| check_form_version(&name, &input, previous_version.as_deref(), access.target.node_version.as_deref()))
        .and_then(|_| check_http2_capability(&name, c.body.as_mut(), access.target.node_version.as_deref()));
    let sent = prepared.is_ok();
    let mut result = match prepared {
        Err(failure) => failure,
        Ok(()) => state.node_http.call(
            &state.secrets, &access.target, c.method.clone(), &c.path, c.body.as_ref(),
            c.timeout.saturating_sub(started.elapsed()),
        ).await,
    };
    if requested_http2 {
        if let NodeResult::Success { value, node_version, .. } = &result {
            if !supports_http2(node_version.as_deref()) || value["server"]["config"]["tlsOptions"]["http2"] != true {
                result = NodeResult::failure(FailureKind::Conflict,
                    "노드가 저장 중 변경되었거나 HTTP/2 설정을 확인하지 못했습니다. 다른 설정이 저장되었을 수 있으니 화면을 새로 고친 뒤 확인하세요.", node_version.clone());
            }
        }
    }
    if sent {
        bookkeeping(
            &state,
            &access,
            &session.actor(&ctx),
            c.audit,
            c.detail.clone(),
            &result,
            true,
        )
        .await;
    }
    if name == "node.rollbackNodeSnapshot" {
        return Ok(Json(FormResult {
            result: Some(serde_json::to_value(result).unwrap()),
            refresh: Some(true),
            ..Default::default()
        }));
    }
    let mut output = FormResult {
        refresh: if result.is_ok() { Some(true) } else { None },
        ..Default::default()
    };
    let NodeResult::Success { value, .. } = result else {
        output.result = Some(form_failure(node, &c, result));
        return Ok(Json(output));
    };
    let unexpected = || {
        form_failure(
            node,
            &c,
            NodeResult::failure(
                FailureKind::NodeError,
                "노드가 예상과 다른 응답을 보냈습니다.",
                None,
            ),
        )
    };
    if let Some(envelope) = c.envelope {
        if let Some(id) = value[envelope]["id"]
            .as_str()
            .filter(|v| !v.is_empty() && !matches!(*v, "." | ".."))
        {
            output.redirect_to = Some(format!("{}/{}", c.base, views::segment(id)));
        } else {
            output.result = Some(unexpected());
        }
    } else if name == "node.applyNodeConfig" {
        output.result = Some(if let Some(text) = value["output"].as_str() {
            json!({"ok":true,"output":text})
        } else {
            unexpected()
        });
    } else if name == "node.submitNodeDnsChallenge" {
        output.result = Some(if value["submitted"] == true {
            json!({"ok":true})
        } else if value["notPropagated"] == true
            && value["pending"]
                .as_array()
                .is_some_and(|a| a.iter().all(Value::is_string))
        {
            let names = value["pending"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect::<Vec<_>>()
                .join(", ");
            json!({"ok":false,"notPropagated":true,"pending":value["pending"],"error":format!("아직 TXT 레코드가 보이지 않습니다: {names} — 전파를 기다린 뒤 다시 확인하세요.")})
        } else {
            unexpected()
        });
    } else if name == "node.replaceNodeCertificate" {
        if let Some(names) = value["uncovered"]
            .as_array()
            .filter(|a| a.iter().all(Value::is_string))
        {
            let names = names
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect::<Vec<_>>()
                .join(",");
            output.redirect_to = Some(format!(
                "{}/{}{}",
                c.base,
                views::segment(&c.id),
                if names.is_empty() {
                    String::new()
                } else {
                    format!("?uncovered={}", views::segment(&names))
                }
            ));
        } else {
            output.result = Some(unexpected());
        }
    } else if c.redirect {
        output.redirect_to = Some(if c.method == Method::DELETE {
            c.base.clone()
        } else {
            format!("{}/{}", c.base, views::segment(&c.id))
        });
    } else {
        output.result = Some(json!({"ok":true}));
    }
    Ok(Json(output))
}
