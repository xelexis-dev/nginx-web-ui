//! Pairing and node registry. Credentials are deliberately absent from NodeView.
use super::client::{DEFAULT_TIMEOUT, FailureKind, NodeResult, NodeTarget, PROBE_TIMEOUT};
use crate::{
    auth::{
        audit::{self, AuditEntry},
        principal::{UserPrincipal, db_unavailable},
        rbac::{Action, can, parse_role, role_str},
    },
    contracts::{Actor, FormInput, Role},
    error::ApiError,
    state::AppState,
};
use reqwest::Method;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

type Fields = BTreeMap<String, String>;
pub struct PairInput {
    pub token: String,
    pub gateway_name: String,
    pub role: Role,
}
pub struct NodeInput {
    pub name: String,
    pub base_url: String,
    pub allow_insecure_tls: bool,
    pub pair: Option<PairInput>,
}

pub fn parse_pair_body(body: &Value) -> Result<PairInput, Fields> {
    let mut errors = Fields::new();
    let token = body["token"].as_str().unwrap_or("").trim().to_owned();
    let gateway_name = body["gatewayName"].as_str().unwrap_or("").trim().to_owned();
    if token.is_empty() {
        errors.insert(
            "token".into(),
            "이 노드의 관리자 화면에서 발급한 서버 토큰을 입력하세요.".into(),
        );
    }
    if gateway_name.is_empty() {
        errors.insert("gatewayName".into(), "게이트웨이 이름을 입력하세요.".into());
    } else if gateway_name.encode_utf16().count() > 100 {
        errors.insert(
            "gatewayName".into(),
            "게이트웨이 이름은 100자 이하로 입력하세요.".into(),
        );
    }
    let role = if body.get("role").is_none() {
        Some(Role::Editor)
    } else {
        body["role"].as_str().and_then(parse_role)
    };
    if role.is_none() {
        errors.insert(
            "role".into(),
            "권한은 admin, editor, viewer 중 하나여야 합니다.".into(),
        );
    }
    if errors.is_empty() {
        Ok(PairInput {
            token,
            gateway_name,
            role: role.unwrap(),
        })
    } else {
        Err(errors)
    }
}
/// New pairing tokens may delegate only the actions they explicitly carry.
pub fn require_pairing_role(role: Role, scopes: &Value) -> Result<(), ApiError> {
    if Action::ALL.into_iter().filter(|action| can(role, *action)).all(|action| {
        scopes.as_array().is_some_and(|scopes| scopes.iter().any(|scope| scope == action.as_str()))
    }) { Ok(()) } else { Err(ApiError::forbidden()) }
}

pub fn parse_form(form: &FormInput, pairing: bool) -> Result<NodeInput, Fields> {
    let mut errors = Fields::new();
    let name = form.get("name").unwrap_or("").trim().to_owned();
    if name.is_empty() || name.encode_utf16().count() > 100 {
        errors.insert("name".into(), "이름은 1~100자로 입력하세요.".into());
    }
    let base_url = form
        .get("baseUrl")
        .unwrap_or("")
        .trim()
        .trim_end_matches('/')
        .to_owned();
    let scheme = base_url.to_ascii_lowercase();
    if !scheme.starts_with("http://") && !scheme.starts_with("https://") {
        errors.insert(
            "baseUrl".into(),
            "주소는 http:// 또는 https:// 로 시작해야 합니다.".into(),
        );
    } else if url::Url::parse(&base_url).ok().is_none_or(|u| {
        crate::outbound::validate_url(&u).is_err() || u.host_str().is_none()
            || !u.username().is_empty()
            || u.password().is_some()
            || u.query().is_some()
            || u.fragment().is_some()
    }) {
        errors.insert("baseUrl".into(), "주소 형식이 올바르지 않습니다.".into());
    }
    let pair = if pairing {
        let mut body = json!({"token":form.get("token"),"gatewayName":form.get("gatewayName")});
        if let Some(role) = form.get("role") {
            body["role"] = json!(role);
        }
        match parse_pair_body(&body) {
            Ok(p) => Some(p),
            Err(e) => {
                errors.extend(e);
                None
            }
        }
    } else {
        None
    };
    if errors.is_empty() {
        Ok(NodeInput {
            name,
            base_url,
            allow_insecure_tls: form.get("allowInsecureTls") == Some("true"),
            pair,
        })
    } else {
        Err(errors)
    }
}
pub fn min_role(a: Role, b: Role) -> Role {
    let rank = |r| match r {
        Role::Viewer => 0,
        Role::Editor => 1,
        Role::Admin => 2,
    };
    if rank(a) <= rank(b) { a } else { b }
}
pub async fn ensure_instance_id(state: &AppState) -> Result<String, ApiError> {
    ensure_instance_id_with_pool(&state.db).await
}
pub(crate) async fn ensure_instance_id_with_pool(pool: &deadpool_postgres::Pool) -> Result<String, ApiError> {
    let client = pool.get().await.map_err(|_| db_unavailable())?;
    let id = uuid::Uuid::new_v4().to_string();
    let row=client.query_one("INSERT INTO app_settings(id,instance_id) VALUES ('current',$1) ON CONFLICT(id) DO UPDATE SET instance_id=COALESCE(NULLIF(app_settings.instance_id,''),EXCLUDED.instance_id) RETURNING instance_id",&[&id]).await.map_err(|_|db_unavailable())?;
    Ok(row.get(0))
}
pub async fn info(state: &AppState) -> Result<Value, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let id = client
        .query_opt(
            "SELECT instance_id FROM app_settings WHERE id='current'",
            &[],
        )
        .await
        .map_err(|_| db_unavailable())?
        .and_then(|r| r.get::<_, Option<String>>(0));
    Ok(
        json!({"semver":state.config.version,"sha":state.config.git_sha,"instanceId":id,"hostname":std::env::var("HOSTNAME").unwrap_or_else(|_|std::fs::read_to_string("/etc/hostname").unwrap_or_default().trim().to_owned())}),
    )
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeView {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub token_role: Role,
    pub allow_insecure_tls: bool,
    pub instance_id: String,
    pub gateway_name: Option<String>,
    pub created_by_user_id: Option<String>,
    pub created_at: String,
    pub last_seen_at: Option<String>,
    pub last_status: Option<String>,
    pub last_version: Option<String>,
    pub last_error: Option<String>,
}
pub struct NodeAccess {
    pub node: NodeView,
    pub target: NodeTarget,
    pub effective_role: Role,
}
fn row_view(row: &tokio_postgres::Row) -> NodeView {
    let iso =
        |t: chrono::DateTime<chrono::Utc>| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    NodeView {
        id: row.get("id"),
        name: row.get("name"),
        base_url: row.get("base_url"),
        token_role: parse_role(&row.get::<_, String>("token_role")).unwrap_or(Role::Viewer),
        allow_insecure_tls: row.get("allow_insecure_tls"),
        instance_id: row.get("instance_id"),
        gateway_name: row.get("gateway_name"),
        created_by_user_id: row.get("created_by_user_id"),
        created_at: iso(row.get("created_at")),
        last_seen_at: row
            .get::<_, Option<chrono::DateTime<chrono::Utc>>>("last_seen_at")
            .map(iso),
        last_status: row.get("last_status"),
        last_version: row.get("last_version"),
        last_error: row.get("last_error"),
    }
}
const SELECT: &str = "SELECT id,name,base_url,token_role::text,allow_insecure_tls,instance_id,gateway_name,created_by_user_id,created_at,last_seen_at,last_status::text,last_version,last_error";
pub async fn list(state: &AppState) -> Result<Vec<NodeView>, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    Ok(client
        .query(&format!("{SELECT} FROM nodes ORDER BY name,id"), &[])
        .await
        .map_err(|_| db_unavailable())?
        .iter()
        .map(row_view)
        .collect())
}
pub async fn get(state: &AppState, id: &str) -> Result<NodeView, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let row = client
        .query_opt(&format!("{SELECT} FROM nodes WHERE id=$1"), &[&id])
        .await
        .map_err(|_| db_unavailable())?
        .ok_or_else(ApiError::not_found)?;
    Ok(row_view(&row))
}
pub async fn resolve(
    state: &AppState,
    session: &UserPrincipal,
    id: &str,
    action: Action,
) -> Result<NodeAccess, ApiError> {
    let access = load_access(state, id, session.role_label()).await?;
    if !session.can(action) || !can(access.node.token_role, action) {
        return Err(ApiError::forbidden());
    }
    Ok(access)
}
async fn load_access(state: &AppState, id: &str, role: Role) -> Result<NodeAccess, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let row = client
        .query_opt(
            &format!("{SELECT},token_enc FROM nodes WHERE id=$1"),
            &[&id],
        )
        .await
        .map_err(|_| db_unavailable())?
        .ok_or_else(ApiError::not_found)?;
    let node = row_view(&row);
    let target = NodeTarget {
        base_url: node.base_url.clone(),
        node_version: node.last_version.clone(),
        token_enc: Some(row.get("token_enc")),
        allow_insecure_tls: node.allow_insecure_tls,
    };
    let effective_role = min_role(role, node.token_role);
    Ok(NodeAccess {
        node,
        target,
        effective_role,
    })
}
pub fn needs_negotiation(version: Option<&str>, include_legacy: bool) -> bool {
    version.and_then(crate::auth::node_version::parse_semver)
        .is_none_or(|v| include_legacy && (v.major, v.minor) == (1, 19))
}

/// Resolve/RBAC stays local; callers explicitly opt into the authenticated read.
pub async fn prepare_access(state: &AppState, access: &mut NodeAccess, include_legacy: bool) -> Result<(), NodeResult> {
    if !needs_negotiation(access.target.node_version.as_deref(), include_legacy) { return Ok(()); }
    probe_access(state, access).await
}

/// Capability-sensitive writes require a probe in this request, even for known versions.
pub async fn probe_access(state: &AppState, access: &mut NodeAccess) -> Result<(), NodeResult> {
    let result = state.node_http.negotiate_info(&state.secrets, &access.target, &access.node.instance_id).await;
    let NodeResult::Success { node_version, .. } = &result else { return Err(result); };
    record_probe(state, &access.node.id, &result).await.map_err(|_|NodeResult::failure(
        FailureKind::NodeError, "노드 버전 정보를 저장하지 못했습니다.", None))?;
    access.target.node_version = node_version.clone();
    access.node.last_version = node_version.clone();
    Ok(())
}

pub async fn record_probe(state: &AppState, id: &str, result: &NodeResult) -> Result<(), ApiError> {
    let Some(status) = result.status() else {
        return Ok(());
    };
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    match result{
        NodeResult::Success{node_version,value,..}=>{
            let instance = value["instanceId"].as_str().filter(|v| !v.is_empty());
            let gateway_token_id = value["gatewayTokenId"].as_str().filter(|v| !v.is_empty());
            client.execute("UPDATE nodes SET last_status=$2::text::node_status,last_seen_at=now(),last_error=NULL,last_version=COALESCE(NULLIF($3,''),last_version),instance_id=COALESCE(NULLIF(instance_id,''),$4,instance_id),gateway_token_id=CASE WHEN instance_id='' OR instance_id=$4 THEN COALESCE($5,gateway_token_id) ELSE gateway_token_id END WHERE id=$1",&[&id,&status,&node_version,&instance,&gateway_token_id]).await
        },
        NodeResult::Failure{message,node_version,..}=>{client.execute("UPDATE nodes SET last_status=$2::text::node_status,last_error=$3,last_version=COALESCE(NULLIF($4,''),last_version) WHERE id=$1",&[&id,&status,&message,&node_version]).await},
    }.map_err(|_|db_unavailable())?;
    Ok(())
}
pub fn audit_entry(node: &NodeView, action: &str, actor: &Actor, mut detail: Value) -> AuditEntry {
    detail["nodeId"] = json!(node.id);
    AuditEntry {
        actor_id: actor.id.clone(),
        actor_label: actor.label.clone(),
        via: actor.via.clone(),
        action: format!("node:{action}"),
        target: Some(node.name.clone()),
        after: Some(detail),
        ip: actor.ip.clone(),
        ..Default::default()
    }
}
pub async fn audit_node(
    state: &AppState,
    node: &NodeView,
    action: &str,
    actor: &Actor,
    detail: Value,
) -> Result<(), ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    audit::write(&**client, &audit_entry(node, action, actor, detail))
        .await
        .map_err(|_| db_unavailable())
}
fn remote_failure(result: NodeResult, base: &str) -> Value {
    if let NodeResult::Failure {
        kind,
        message,
        field_errors,
        ..
    } = result
    {
        let message = if matches!(kind, FailureKind::Unreachable | FailureKind::TooOld) {
            format!(
                "{message} ({base}) — 주소와 포트가 맞는지, 그 주소로 이 서버에서 접속이 되는지 확인하세요."
            )
        } else {
            message
        };
        json!({"ok":false,"error":message,"fieldErrors":field_errors})
    } else {
        json!({"ok":false,"error":"노드가 예상치 못한 응답을 보냈습니다."})
    }
}
async fn revoke(state: &AppState, target: &NodeTarget, timeout: std::time::Duration) -> bool {
    state
        .node_http
        .call(
            &state.secrets,
            target,
            Method::DELETE,
            "/api/v1/pair",
            None,
            timeout,
        )
        .await
        .is_ok()
}

pub async fn pair_node(
    state: &AppState,
    input: NodeInput,
    actor: &Actor,
) -> Result<Result<String, Value>, ApiError> {
    let pair = input.pair.ok_or_else(ApiError::internal)?;
    let mut target = NodeTarget {
        base_url: input.base_url.clone(),
        node_version: None,
        token_enc: None,
        allow_insecure_tls: input.allow_insecure_tls,
    };
    let body = json!({"token":pair.token,"gatewayName":pair.gateway_name,"role":role_str(pair.role)});
    let mut response=state.node_http.call(&state.secrets,&target,Method::POST,"/api/v1/pair",Some(&body),DEFAULT_TIMEOUT).await;
    // Pairing alone may retry a POST: legacy nodes refuse it before issuing a token.
    if let NodeResult::Failure { kind: FailureKind::VersionMismatch, node_version: Some(version), .. } = &response {
        if super::client::gateway_version(&state.config.version, Some(version)) == "1.19.11" {
            target.node_version = Some(version.clone());
            response=state.node_http.call(&state.secrets,&target,Method::POST,"/api/v1/pair",Some(&body),DEFAULT_TIMEOUT).await;
        }
    }
    let NodeResult::Success { value, node_version, .. } = response else {
        return Ok(Err(remote_failure(response, &input.base_url)));
    };
    let Some(token) = value["token"].as_str().filter(|v| !v.is_empty()) else {
        return Ok(Err(
            json!({"ok":false,"error":"노드가 예상치 못한 응답을 보냈습니다."}),
        ));
    };
    target.node_version = node_version.or_else(|| value["semver"].as_str().map(str::to_owned));
    target.token_enc = Some(
        state
            .secrets
            .seal(token)
            .map_err(|_| ApiError::internal())?,
    );
    // Everything after issuance compensates by revoking when registration fails, including races.
    let result=async {
        let own=ensure_instance_id(state).await?;
        let instance=value["instanceId"].as_str().filter(|v|!v.is_empty()).ok_or_else(||ApiError::new(axum::http::StatusCode::BAD_REQUEST,"invalid_node","이 노드가 자기 id 를 알려주지 않습니다 — 노드를 재시작하거나 업그레이드하세요."))?;
        if instance==own{return Err(ApiError::new(axum::http::StatusCode::BAD_REQUEST,"self_node","자기 자신은 노드로 등록할 수 없습니다."));}
        let role=value["role"].as_str().and_then(parse_role).ok_or_else(||ApiError::new(axum::http::StatusCode::BAD_GATEWAY,"invalid_node","노드가 예상치 못한 응답을 보냈습니다."))?;
        let mut client=state.db.get().await.map_err(|_|db_unavailable())?;
        let tx=client.transaction().await.map_err(|_|db_unavailable())?;
        if let Some(row)=tx.query_opt("SELECT name FROM nodes WHERE instance_id=$1",&[&instance]).await.map_err(|_|db_unavailable())?{return Err(ApiError::new(axum::http::StatusCode::CONFLICT,"duplicate_node",format!("이미 「{}」으로 등록된 노드입니다.",row.get::<_,String>(0))));}
        let id=uuid::Uuid::new_v4().to_string();
        tx.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,allow_insecure_tls,instance_id,gateway_name,created_by_user_id) VALUES($1,$2,$3,$4,$5::text::role,$6,$7,$8,$9)",&[&id,&input.name,&input.base_url,&target.token_enc,&role_str(role),&input.allow_insecure_tls,&instance,&pair.gateway_name,&actor.id]).await.map_err(|e|if e.code()==Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION){ApiError::new(axum::http::StatusCode::CONFLICT,"duplicate_node","이미 등록된 노드입니다.")}else{db_unavailable()})?;
        audit::write(&*tx,&AuditEntry{actor_id:actor.id.clone(),actor_label:actor.label.clone(),via:actor.via.clone(),action:"node:pair".into(),target:Some(input.name.clone()),after:Some(json!({"nodeId":id,"baseUrl":input.base_url,"role":role,"gatewayName":pair.gateway_name})),ip:actor.ip.clone(),..Default::default()}).await.map_err(|_|db_unavailable())?;
        tx.execute("UPDATE nodes SET last_version=$2,gateway_token_id=$3 WHERE id=$1", &[&id, &target.node_version, &value["gatewayTokenId"].as_str().filter(|v| !v.is_empty())]).await.map_err(|_|db_unavailable())?;
        tx.commit().await.map_err(|_|db_unavailable())?;
        Ok(id)
    }.await;
    match result {
        Ok(id) => Ok(Ok(id)),
        Err(error) => {
            revoke(state, &target, DEFAULT_TIMEOUT).await;
            if error.status.is_server_error() {
                Err(error)
            } else {
                Ok(Err(json!({"ok":false,"error":error.message})))
            }
        }
    }
}
pub async fn update_node(
    state: &AppState,
    id: &str,
    input: NodeInput,
    actor: &Actor,
) -> Result<(), ApiError> {
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(|_| db_unavailable())?;
    let before = tx.query_opt("SELECT base_url,instance_id,gateway_token_id,last_version,token_enc FROM nodes WHERE id=$1 FOR UPDATE", &[&id]).await.map_err(|_| db_unavailable())?.ok_or_else(ApiError::not_found)?;
    let old_origin = reqwest::Url::parse(before.get::<_, &str>(0)).map(|url| url.origin());
    let new_origin = reqwest::Url::parse(&input.base_url).map(|url| url.origin());
    if !matches!((old_origin, new_origin), (Ok(old), Ok(new)) if old == new) {
        let instance: String = before.get("instance_id");
        let gateway_token_id: Option<String> = before.get("gateway_token_id");
        let version: Option<String> = before.get("last_version");
        if version.as_deref().and_then(crate::auth::node_version::parse_semver)
            .is_some_and(|v| (v.major, v.minor) == (1, 19)) {
            return Err(super::identity::refused());
        }
        let Some(gateway_token_id) = gateway_token_id.filter(|v| !v.is_empty() && !instance.is_empty()) else {
            let mut error = super::identity::refused();
            let message = "같은 노드인지 확인할 정보가 없습니다. 기존 주소에서 상태를 새로고침한 뒤 다시 시도하세요.";
            error.message = message.into();
            error.field_errors = Some(BTreeMap::from([("baseUrl".into(), message.into())]));
            return Err(error);
        };
        let mut nonce = [0u8; 32];
        getrandom::fill(&mut nonce).map_err(|_| ApiError::internal())?;
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
        let target = NodeTarget {base_url:input.base_url.clone(),node_version:version,token_enc:None,allow_insecure_tls:input.allow_insecure_tls};
        let result = state.node_http.call(&state.secrets,&target,Method::POST,"/api/v1/identity-challenge",
            Some(&json!({"gatewayTokenId":gateway_token_id,"nonce":URL_SAFE_NO_PAD.encode(nonce)})),PROBE_TIMEOUT).await;
        let sealed: String = before.get("token_enc");
        let raw = state.secrets.open(Some(&sealed)).filter(|v| !v.is_empty()).ok_or_else(super::identity::refused)?;
        if !matches!(result, NodeResult::Success {ref value,..} if super::identity::verify(&crate::auth::token_digest(&raw),&nonce,&instance,value)) {
            return Err(super::identity::refused());
        }
    }
    let row=tx.query_opt(&format!("UPDATE nodes SET name=$2,base_url=$3,allow_insecure_tls=$4 WHERE id=$1 RETURNING {}",SELECT.trim_start_matches("SELECT ")),&[&id,&input.name,&input.base_url,&input.allow_insecure_tls]).await.map_err(|_|db_unavailable())?.ok_or_else(ApiError::not_found)?;
    audit::write(
        &*tx,
        &audit_entry(
            &row_view(&row),
            "update",
            actor,
            json!({"baseUrl":input.base_url,"allowInsecureTls":input.allow_insecure_tls}),
        ),
    )
    .await
    .map_err(|_| db_unavailable())?;
    tx.commit().await.map_err(|_| db_unavailable())
}
pub async fn unpair_node(state: &AppState, id: &str, actor: &Actor) -> Result<bool, ApiError> {
    let mut access = load_access(state, id, Role::Admin).await?;
    let started = std::time::Instant::now();
    let revoked = prepare_access(state, &mut access, true).await.is_ok()
        && revoke(state, &access.target, DEFAULT_TIMEOUT.saturating_sub(started.elapsed())).await;
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(|_| db_unavailable())?;
    tx.execute("DELETE FROM nodes WHERE id=$1", &[&id])
        .await
        .map_err(|_| db_unavailable())?;
    audit::write(
        &*tx,
        &audit_entry(
            &access.node,
            "unpair",
            actor,
            json!({"baseUrl":access.node.base_url,"remoteRevoked":revoked}),
        ),
    )
    .await
    .map_err(|_| db_unavailable())?;
    tx.commit().await.map_err(|_| db_unavailable())?;
    Ok(!revoked)
}
pub async fn refresh_statuses(state: &AppState) -> Result<(), ApiError> {
    let nodes = list(state).await?;
    let mut tasks = tokio::task::JoinSet::new();
    for node in nodes {
        let state = state.clone();
        tasks.spawn(async move {
            let access = load_access(&state, &node.id, Role::Admin).await?;
            let result = state.node_http.negotiate_info(&state.secrets, &access.target, &access.node.instance_id).await;
            record_probe(&state, &node.id, &result).await
        });
    }
    while let Some(result) = tasks.join_next().await {
        if !matches!(result, Ok(Ok(()))) {
            eprintln!("[nodes] probe persistence failed");
        }
    }
    Ok(())
}
