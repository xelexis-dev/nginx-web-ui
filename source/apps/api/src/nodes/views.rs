use super::service;
use crate::{
    auth::{
        principal::{UserPrincipal, db_unavailable},
        rbac::Action,
    },
    error::ApiError,
    state::AppState,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn visible_node(mut node: service::NodeView, session: &UserPrincipal) -> service::NodeView {
    if !session.can(Action::ConfigWrite) && !session.can(Action::UserManage) {
        node.base_url.clear();
        node.last_error = None;
    }
    node
}

pub async fn view(
    state: &AppState,
    session: &UserPrincipal,
    path: &str,
    query: &BTreeMap<String, String>,
) -> Result<Option<Value>, ApiError> {
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    if parts.first() != Some(&"nodes") {
        return Ok(None);
    }
    if !session.can(Action::ConfigRead) {
        return Err(ApiError::forbidden());
    }
    let can_write = session.can(Action::UserManage);
    if parts.len() == 1 {
        let warning = if query.get("unpairWarning").map(String::as_str) == Some("revoke_failed") {
            Some(
                "노드에 연결 해제를 알리지 못했습니다. 노드의 /admin/security 에서 직접 해지하세요.",
            )
        } else {
            None
        };
        return Ok(Some(
            json!({"nodes":service::list(state).await?.into_iter().map(|node| visible_node(node, session)).collect::<Vec<_>>(),"canWrite":can_write,"unpairWarningText":warning}),
        ));
    }
    if parts == ["nodes", "new"] {
        if !can_write {
            return Err(ApiError::forbidden());
        }
        let client = state.db.get().await.map_err(|_| db_unavailable())?;
        let base = client
            .query_opt(
                "SELECT public_base_url FROM app_settings WHERE id='current'",
                &[],
            )
            .await
            .map_err(|_| db_unavailable())?
            .and_then(|r| r.get::<_, Option<String>>(0));
        let host = base
            .and_then(|b| url::Url::parse(&b).ok())
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        return Ok(Some(json!({"defaultGatewayName":host})));
    }
    if parts.len() == 3 && parts[2] == "edit" {
        if !can_write {
            return Err(ApiError::forbidden());
        }
        return Ok(Some(json!({"node":visible_node(service::get(state,parts[1]).await?, session)})));
    }
    // Layout reads never probe; remote content is dispatched separately below.
    if parts.len() == 3 && parts[2] == "layout" {
        return Ok(Some(json!({"node":visible_node(service::get(state,parts[1]).await?, session)})));
    }
    let Some(plan) = remote_plan(&parts[2..], query) else {
        return Ok(None);
    };
    let node_id = parts[1];
    let base = format!("/nodes/{node_id}");
    let list_href = if parts.len() == 2 {
        "/nodes".to_owned()
    } else {
        format!("{base}{}", plan.list_suffix)
    };
    let mut access = match service::resolve(state, session, node_id, plan.action).await {
        Ok(access) => access,
        Err(e) if e.status == axum::http::StatusCode::FORBIDDEN && !session.is_token() => {
            return Ok(Some(failed(
                node_id,
                path,
                &list_href,
                super::client::NodeResult::failure(
                    super::client::FailureKind::Forbidden,
                    "이 작업을 수행할 권한이 없습니다.",
                    None,
                ),
                None,
            )));
        }
        Err(e) => return Err(e),
    };
    if let Err(failure @ super::client::NodeResult::Failure { kind: super::client::FailureKind::Unreachable, .. }) = service::prepare_access(state, &mut access, true).await {
        let mut error = failed(node_id, path, &list_href, failure, None);
        error["node"] = json!(visible_node(access.node, session));
        error["canManage"] = json!(can_write);
        return Ok(Some(error));
    }
    access.node = visible_node(access.node, session);
    let resource_reads = async {
        // Resource fanout is at most three; the optional DNS read runs alongside it.
        let mut tasks = tokio::task::JoinSet::new();
        for (index, request) in plan.requests.iter().enumerate() {
            let state = state.clone();
            let path = request.path.clone();
            let target = super::client::NodeTarget {
                base_url: access.target.base_url.clone(),
                node_version: access.target.node_version.clone(),
                token_enc: access.target.token_enc.clone(),
                allow_insecure_tls: access.target.allow_insecure_tls,
            };
            tasks.spawn(async move {
                (
                    index,
                    state
                        .node_http
                        .call(
                            &state.secrets,
                            &target,
                            reqwest::Method::GET,
                            &path,
                            None,
                            super::client::DEFAULT_TIMEOUT,
                        )
                        .await,
                )
            });
        }
        let mut results = vec![None; plan.requests.len()];
        while let Some(result) = tasks.join_next().await {
            let (index, result) = result.map_err(|_| ApiError::internal())?;
            results[index] = Some(result);
        }
        Ok::<_, ApiError>(results)
    };
    let (results, runtime_dns) = tokio::join!(
        resource_reads,
        read_runtime_dns(state, &access.target, plan.runtime_dns),
    );
    let results = results?;
    let observed_version = results.iter().rev().flatten().find_map(|result| match result {
        super::client::NodeResult::Success { node_version: Some(version), .. } if !version.is_empty() => Some(version.clone()),
        _ => None,
    });
    let mut data = serde_json::Map::new();
    let mut single = Value::Null;
    for (request, result) in plan.requests.iter().zip(results) {
        match result.unwrap() {
            super::client::NodeResult::Success { value, .. } => {
                if plan.aggregate {
                    data.insert(request.key.into(), value);
                } else {
                    single = value;
                }
            }
            failure => {
                let mut error = failed(
                    node_id,
                    path,
                    &list_href,
                    failure,
                    if plan.aggregate {
                        Some(request.subject)
                    } else {
                        None
                    },
                );
                error["node"] = json!(access.node);
                error["canManage"] = json!(can_write);
                return Ok(Some(error));
            }
        }
    }
    if let Some(version) = observed_version {
        // A page response carries version/status evidence, never pairing authority.
        let observation = super::client::NodeResult::Success { ok: true, value: Value::Null, node_version: Some(version.clone()) };
        // Metadata refresh must not fail an otherwise successful node read.
        let _ = service::record_probe(state, node_id, &observation).await;
        access.node.last_version = Some(version);
    }
    let mut result = json!({"ok":true,"node":access.node,"effectiveRole":access.effective_role,"data":if plan.aggregate{Value::Object(data)}else{single},"canManage":can_write,"canWrite":session.can(Action::ConfigWrite)&&crate::auth::rbac::can(access.node.token_role,Action::ConfigWrite),"canApply":session.can(Action::NginxReload)&&crate::auth::rbac::can(access.node.token_role,Action::NginxReload)});
    if let Some(status) = runtime_dns {
        result["runtimeDns"] = status;
    }
    Ok(Some(result))
}

/// Best effort and target-only: older nodes, transport errors and invalid payloads
/// leave availability unknown. Never substitute the gateway's local descriptor.
async fn read_runtime_dns(
    state: &AppState,
    target: &super::client::NodeTarget,
    enabled: bool,
) -> Option<Value> {
    if !enabled {
        return None;
    }
    if target.node_version.as_deref().and_then(crate::auth::node_version::parse_semver)
        .is_some_and(|v| (v.major, v.minor) == (1, 19)) {
        return Some(json!({"available":false,"reason":"1.19 노드에서는 실행 중 이름 확인을 사용할 수 없습니다. 노드를 2.0으로 업데이트하세요."}));
    }
    let super::client::NodeResult::Success {
        value,
        node_version,
        ..
    } = state
        .node_http
        .call(
            &state.secrets,
            target,
            reqwest::Method::GET,
            "/api/v1/runtime-dns",
            None,
            super::client::DEFAULT_TIMEOUT,
        )
        .await
    else {
        return None;
    };
    let version = crate::auth::node_version::parse_semver(node_version.as_deref()?)?;
    if (version.major, version.minor) < (1, 20) {
        return None;
    }
    let available = value.get("available")?.as_bool()?;
    let reason = value.get("reason")?;
    if !reason.is_null() && !reason.is_string() {
        return None;
    }
    Some(json!({"available": available, "reason": reason}))
}

pub fn failed(
    node_id: &str,
    retry: &str,
    list: &str,
    failure: super::client::NodeResult,
    subject: Option<&str>,
) -> Value {
    let mut context = json!({"failure":failure,"nodeId":node_id,"retryHref":retry,"listHref":list});
    if let Some(subject) = subject {
        context["subject"] = json!(subject);
    }
    json!({"ok":false,"nodeError":context})
}
pub fn segment(id: &str) -> String {
    const SEGMENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~');
    percent_encoding::utf8_percent_encode(id, SEGMENT).to_string()
}
pub struct ReadRequest {
    pub key: &'static str,
    pub path: String,
    pub subject: &'static str,
}
pub struct ReadPlan {
    pub action: Action,
    pub requests: Vec<ReadRequest>,
    pub aggregate: bool,
    pub runtime_dns: bool,
    pub list_suffix: &'static str,
}
/// Exact browser page registry; neither query nor route params choose a remote method/path.
pub fn remote_plan(parts: &[&str], query: &BTreeMap<String, String>) -> Option<ReadPlan> {
    if parts
        .iter()
        .any(|p| p.is_empty() || matches!(*p, "." | ".."))
    {
        return None;
    }
    let mut plan = ReadPlan {
        action: Action::ConfigRead,
        requests: vec![],
        aggregate: false,
        runtime_dns: false,
        list_suffix: "",
    };
    let request = |key, path: String, subject| ReadRequest { key, path, subject };
    let upstreams = || request("upstreams", "/api/v1/upstreams".into(), "Upstream 목록을");
    let servers = || request("servers", "/api/v1/servers".into(), "서버 목록을");
    let certificates = || {
        request(
            "certificates",
            "/api/v1/certificates".into(),
            "인증서 목록을",
        )
    };
    let pending = || request("pending", "/api/v1/pending".into(), "미적용 변경을");
    match parts {
        [] => plan
            .requests
            .push(request("status", "/api/v1/status".into(), "노드 상태를")),
        ["metrics"] | ["logs"] => {}
        ["history"] => {
            plan.list_suffix = "/history";
            plan.requests
                .push(request("history", "/api/v1/history".into(), "히스토리를"));
        }
        ["audit"] => {
            plan.list_suffix = "/audit";
            let mut params = url::form_urlencoded::Serializer::new(String::new());
            for key in ["page", "action", "target", "actor", "from", "to"] {
                if let Some(value) = query.get(key) {
                    params.append_pair(key, value);
                }
            }
            let qs = params.finish();
            plan.requests.push(request(
                "audit",
                format!(
                    "/api/v1/audit{}",
                    if qs.is_empty() {
                        String::new()
                    } else {
                        format!("?{qs}")
                    }
                ),
                "감사 기록을",
            ));
        }
        ["audit", id] => {
            plan.list_suffix = "/audit";
            plan.requests.push(request(
                "audit",
                format!("/api/v1/audit/{}", segment(id)),
                "감사 기록을",
            ));
        }
        ["reverse-proxy"] => {
            plan.list_suffix = "/reverse-proxy";
            plan.aggregate = true;
            plan.requests = vec![upstreams(), servers(), pending()];
        }
        ["reverse-proxy", "new"] => {
            plan.list_suffix = "/reverse-proxy";
            plan.action = Action::ConfigWrite;
            plan.aggregate = true;
            plan.requests = vec![upstreams(), certificates()];
        }
        ["reverse-proxy", "upstreams"] => {
            plan.list_suffix = "/reverse-proxy/upstreams";
            plan.requests.push(upstreams());
        }
        ["reverse-proxy", "upstreams", "new"] => {
            plan.list_suffix = "/reverse-proxy/upstreams";
            plan.action = Action::ConfigWrite;
        }
        ["reverse-proxy", "upstreams", id] | ["reverse-proxy", "upstreams", id, "edit"] => {
            plan.list_suffix = "/reverse-proxy/upstreams";
            if parts.len() == 4 {
                plan.action = Action::ConfigWrite;
            }
            plan.requests.push(request(
                "upstream",
                format!("/api/v1/upstreams/{}", segment(id)),
                "그 Upstream을",
            ));
        }
        ["reverse-proxy", id] | ["reverse-proxy", id, "edit"] => {
            plan.list_suffix = "/reverse-proxy";
            plan.aggregate = true;
            if parts.len() == 3 {
                plan.action = Action::ConfigWrite;
            }
            plan.requests = vec![
                request(
                    "server",
                    format!("/api/v1/servers/{}", segment(id)),
                    "그 서버를",
                ),
                upstreams(),
                certificates(),
            ];
        }
        ["stream"] => {
            plan.list_suffix = "/stream";
            plan.aggregate = true;
            plan.requests = vec![
                request("streams", "/api/v1/streams".into(), "Stream 목록을"),
                pending(),
            ];
        }
        ["stream", "new"] => {
            plan.list_suffix = "/stream";
            plan.action = Action::ConfigWrite;
        }
        ["stream", id] | ["stream", id, "edit"] => {
            plan.list_suffix = "/stream";
            if parts.len() == 3 {
                plan.action = Action::ConfigWrite;
            }
            plan.requests.push(request(
                "stream",
                format!("/api/v1/streams/{}", segment(id)),
                "그 Stream을",
            ));
        }
        ["certificates"] => {
            plan.list_suffix = "/certificates";
            plan.aggregate = true;
            plan.requests = vec![certificates(), servers()];
        }
        ["certificates", "new"] | ["certificates", "import"] => {
            plan.list_suffix = "/certificates";
            plan.action = Action::ConfigWrite;
        }
        ["certificates", id] | ["certificates", id, "edit"] | ["certificates", id, "replace"] => {
            plan.list_suffix = "/certificates";
            plan.aggregate = true;
            if parts.len() == 3 {
                plan.action = if parts[2] == "replace" {
                    Action::NginxReload
                } else {
                    Action::ConfigWrite
                };
            }
            plan.requests.push(request(
                "certificate",
                format!("/api/v1/certificates/{}", segment(id)),
                "그 인증서를",
            ));
            if parts.len() == 2 {
                plan.requests.push(servers());
            }
        }
        _ => return None,
    }
    plan.runtime_dns = match parts {
        ["reverse-proxy", "upstreams", ..] => parts.len() >= 3,
        ["reverse-proxy", _]
        | ["reverse-proxy", _, "edit"]
        | ["stream", _]
        | ["stream", _, "edit"] => true,
        _ => false,
    };
    Some(plan)
}
