use super::{
    client::{self, FailureKind, NodeResult, NodeTarget},
    service,
};
use crate::{
    auth::{
        principal::{UserPrincipal, db_unavailable},
        rbac::Action,
    },
    control::pending::{diff_config, snapshot_from_value},
    error::ApiError,
    state::AppState,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};
use tokio::time::Instant;

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
    let history_deadline = (plan.list_suffix == "/history")
        .then(|| Instant::now() + client::DEFAULT_TIMEOUT);
    let mut probe = None;
    let prepared = if let Some(deadline) = history_deadline {
        if service::needs_negotiation(access.target.node_version.as_deref(), true) {
            match tokio::time::timeout_at(deadline, state.node_http.negotiate_info(
                &state.secrets, &access.target, &access.node.instance_id,
            )).await.unwrap_or_else(|_| history_timeout()) {
                result @ NodeResult::Success { .. } => {
                    if let NodeResult::Success { node_version, .. } = &result {
                        access.target.node_version = node_version.clone();
                        access.node.last_version = node_version.clone();
                    }
                    probe = Some(result);
                    Ok(())
                }
                failure => Err(failure),
            }
        } else {
            Ok(())
        }
    } else {
        service::prepare_access(state, &mut access, true).await
    };
    if let Err(failure) = prepared {
        if history_deadline.is_some() || matches!(failure, NodeResult::Failure { kind: FailureKind::Unreachable, .. }) {
            let mut error = failed(node_id, path, &list_href, failure, None);
            if history_deadline.is_some() {
                history_error_title(&mut error);
            }
            error["node"] = json!(visible_node(access.node, session));
            error["canManage"] = json!(can_write);
            return Ok(Some(error));
        }
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
                    if let Some(deadline) = history_deadline {
                        history_read(&state, &target, &path, deadline).await
                    } else {
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
                            .await
                    },
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
                if history_deadline.is_some() {
                    history_error_title(&mut error);
                }
                error["node"] = json!(access.node);
                error["canManage"] = json!(can_write);
                return Ok(Some(error));
            }
        }
    }
    let can_write_config = session.can(Action::ConfigWrite)
        && crate::auth::rbac::can(access.node.token_role, Action::ConfigWrite);
    let can_apply = session.can(Action::NginxReload)
        && crate::auth::rbac::can(access.node.token_role, Action::NginxReload);
    if let Some(id) = plan.history_id.as_deref() {
        single = match history_detail(
            state,
            &access.target,
            id,
            &data,
            can_write_config && can_apply,
            history_deadline.expect("history detail has a deadline"),
        ).await {
            Ok(detail) => detail,
            Err(failure) => {
                let mut error = failed(node_id, path, &list_href, failure, Some("그 스냅샷을"));
                history_error_title(&mut error);
                error["node"] = json!(access.node);
                error["canManage"] = json!(can_write);
                return Ok(Some(error));
            }
        };
    } else if plan.aggregate {
        single = Value::Object(data);
    }
    if let Some(version) = observed_version {
        access.node.last_version = Some(version.clone());
        // History keeps any verified pairing evidence from negotiation separate
        // from the version observed on ordinary resource responses.
        if probe.is_none() {
            probe = Some(NodeResult::Success { ok: true, value: Value::Null, node_version: Some(version) });
        }
    }
    if let Some(probe) = probe {
        if let Some(deadline) = history_deadline {
            let write_deadline = deadline.min(Instant::now() + Duration::from_millis(100));
            let _ = tokio::time::timeout_at(write_deadline, service::record_history_probe(state, node_id, &probe)).await;
        } else {
            let _ = service::record_probe(state, node_id, &probe).await;
        }
    }
    let mut result = json!({"ok":true,"node":access.node,"effectiveRole":access.effective_role,"data":single,"canManage":can_write,"canWrite":can_write_config,"canApply":can_apply});
    if let Some(status) = runtime_dns {
        result["runtimeDns"] = status;
    }
    Ok(Some(result))
}

fn history_error_title(error: &mut Value) {
    let failure = &error["nodeError"]["failure"];
    if failure["kind"] != "node_error" { return; }
    let title = match failure["message"].as_str() {
        Some(client::RESPONSE_TOO_LARGE) => "스냅샷 응답이 1 MiB를 초과해 열 수 없습니다.",
        Some(message @ (UNEXPECTED_HISTORY | client::IDENTITY_ERROR)) => message,
        _ => return,
    };
    error["nodeError"]["nodeErrorTitle"] = json!(title);
}

const UNEXPECTED_HISTORY: &str = "노드가 예상과 다른 응답을 보냈습니다.";

fn unexpected_history() -> NodeResult {
    NodeResult::failure(FailureKind::NodeError, UNEXPECTED_HISTORY, None)
}

fn history_snapshot<'a>(body: &'a Value, id: &str) -> Result<&'a Value, NodeResult> {
    let snapshot = &body["snapshot"];
    if snapshot["id"].as_str() != Some(id)
        || !snapshot["createdAt"].as_str().is_some_and(|iso| chrono::DateTime::parse_from_rfc3339(iso).is_ok())
        || !snapshot.get("label").is_some_and(|label| label.is_string() || label.is_null())
        || !valid_history_data(&snapshot["data"])
    {
        return Err(unexpected_history());
    }
    Ok(snapshot)
}

fn valid_history_data(data: &Value) -> bool {
    fn rows(data: &Value, key: &str, valid: impl Fn(&Value) -> bool) -> bool {
        data.get(key).is_none_or(|items| items.as_array().is_some_and(|items| {
            items.iter().all(|row| {
                row.is_object() && row["id"].is_string()
                    && row.get("version").is_none_or(|value| {
                        value.as_str().is_some_and(|version| {
                            version.len() <= 19 && matches!(version.as_bytes().first(), Some(b'1'..=b'9'))
                                && version.bytes().all(|digit| digit.is_ascii_digit())
                        })
                    })
                    && row.get("resolveAtRuntime").is_none_or(Value::is_boolean)
                    && valid(row)
            })
        }))
    }
    fn listener(row: &Value) -> bool {
        row["listen"].as_u64().is_some_and(|port| (1..=65535).contains(&port)) && row["enabled"].is_boolean()
    }
    fn nullable_id(row: &Value, name: &str) -> bool {
        row.get(name).is_some_and(|id| id.is_null() || id.is_string())
    }
    data.is_object()
        && rows(data, "upstreams", |row| {
            row["name"].is_string() && row["servers"].as_array()
                .is_some_and(|servers| servers.iter().all(Value::is_string))
        })
        && rows(data, "servers", |row| {
            listener(row) && row["serverName"].is_string()
                && nullable_id(row, "upstreamId") && nullable_id(row, "certificateId")
                && row["config"].is_object()
        })
        && rows(data, "streams", |row| {
            listener(row) && matches!(row["protocol"].as_str(), Some("tcp" | "udp"))
                && row["proxyPass"].is_string()
        })
}

fn history_path(id: &str) -> String {
    format!("/api/v1/history/{}", segment(id))
}

fn history_timeout() -> NodeResult {
    NodeResult::failure(FailureKind::Unreachable, "노드 요청 시간이 초과되었습니다.", None)
}

async fn history_read(state: &AppState, target: &NodeTarget, path: &str, deadline: Instant) -> NodeResult {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() { return history_timeout(); }
    tokio::time::timeout_at(deadline, state.node_http.call(
        &state.secrets, target, reqwest::Method::GET, path, None, remaining,
    )).await.unwrap_or_else(|_| history_timeout())
}

fn history_value(result: NodeResult) -> Result<Value, NodeResult> {
    match result {
        NodeResult::Success { value, .. } => Ok(value),
        failure => Err(failure),
    }
}

fn history_rows(body: &Value) -> Result<&[Value], NodeResult> {
    body["snapshots"].as_array().map(Vec::as_slice).filter(|rows| {
        rows.len() <= 100 && rows.iter().all(|row| row["id"].as_str().is_some_and(|id| {
            !id.is_empty() && !matches!(id, "." | "..") && !id.chars().any(char::is_control)
        }))
    }).ok_or_else(unexpected_history)
}

async fn history_detail(
    state: &AppState,
    target: &NodeTarget,
    id: &str,
    responses: &serde_json::Map<String, Value>,
    can_rollback: bool,
    deadline: Instant,
) -> Result<Value, NodeResult> {
    let mut snapshot = history_snapshot(&responses["snapshot"], id)?;
    let mut list = history_rows(&responses["history"])?;
    let locate = |rows: &[Value]| rows.iter().position(|row| row["id"].as_str() == Some(id));
    let mut position = locate(list);
    let refreshed_snapshot;
    let refreshed_list;
    if position.is_none() && list.len() < 100 {
        let selected_path = history_path(id);
        let (selected, latest) = tokio::join!(
            history_read(state, target, &selected_path, deadline),
            history_read(state, target, "/api/v1/history", deadline),
        );
        refreshed_snapshot = history_value(selected)?;
        snapshot = history_snapshot(&refreshed_snapshot, id)?;
        refreshed_list = history_value(latest)?;
        list = history_rows(&refreshed_list)?;
        position = Some(locate(list).ok_or_else(unexpected_history)?);
    }
    let (comparison, previous) = match position {
        Some(index) if index + 1 < list.len() => {
            let previous_id = list[index + 1]["id"].as_str().ok_or_else(unexpected_history)?;
            match history_read(state, target, &history_path(previous_id), deadline).await {
                NodeResult::Success { value, .. } => {
                    let previous = history_snapshot(&value, previous_id)?;
                    ("compared", Some(snapshot_from_value(&previous["data"])))
                }
                NodeResult::Failure { kind: FailureKind::NotFound, .. } => ("previous_deleted", None),
                failure => return Err(failure),
            }
        }
        Some(_) if list.len() < 100 => ("first", None),
        _ => ("outside_recent", None),
    };
    let current = snapshot_from_value(&snapshot["data"]);
    let diff = diff_config(previous.as_ref(), &current);
    Ok(json!({
        "snapshot": {"id":id,"label":snapshot["label"]},
        "iso":snapshot["createdAt"],
        "upstreams":current.upstreams,
        "servers":current.servers,
        "streams":current.streams,
        "diff":diff,
        "comparison":comparison,
        "canRollback":can_rollback,
    }))
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
    history_id: Option<String>,
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
        history_id: None,
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
        ["history", id] => {
            let id = percent_encoding::percent_decode_str(id).decode_utf8().ok()?;
            if id.is_empty() || matches!(id.as_ref(), "." | "..") || id.chars().any(char::is_control) {
                return None;
            }
            plan.list_suffix = "/history";
            plan.aggregate = true;
            plan.requests = vec![
                request("snapshot", history_path(&id), "그 스냅샷을"),
                request("history", "/api/v1/history".into(), "적용 이력을"),
            ];
            plan.history_id = Some(id.into_owned());
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
