//! Remote node transport. Redirects never carry credentials to another endpoint.
//! A shared pair of connection pools separates normal and explicitly insecure TLS.
use crate::crypto::SecretBox;
use reqwest::{Client, Method, Url};
use serde::Serialize;
use serde_json::{Value,json};
use std::{collections::BTreeMap,time::Duration};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
pub const CONFIG_APPLY_TIMEOUT: Duration = Duration::from_secs(300);
pub const APPLY_TIMEOUT: Duration = Duration::from_secs(120);
pub const RENEW_TIMEOUT: Duration = Duration::from_secs(300);
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_BODY: usize = 1024 * 1024;

#[derive(Clone)]
pub struct NodeTarget {
    pub base_url: String,
    pub node_version: Option<String>,
    pub token_enc: Option<String>,
    pub allow_insecure_tls: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all="snake_case")]
pub enum FailureKind { Unreachable, Unauthorized, Forbidden, VersionMismatch, TooOld, Conflict, NotFound, Validation, DomainFailure, RateLimited, NodeError }

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum NodeResult {
    Success { ok: bool, value: Value, #[serde(rename="nodeVersion")] node_version: Option<String> },
    Failure { ok: bool, kind: FailureKind, message: String, #[serde(rename="nodeVersion")] node_version: Option<String>, #[serde(rename="fieldErrors",skip_serializing_if="Option::is_none")] field_errors: Option<BTreeMap<String,String>> },
}
impl NodeResult {
    pub fn failure(kind: FailureKind, message: impl Into<String>, node_version: Option<String>) -> Self {
        Self::Failure { ok:false,kind,message:message.into(),node_version,field_errors:None }
    }
    pub fn is_ok(&self) -> bool { matches!(self,Self::Success{..}) }
    pub fn status(&self) -> Option<&'static str> {
        match self {
            Self::Success{..} => Some("ok"),
            Self::Failure{kind,..} => match kind {
                FailureKind::Unreachable=>Some("unreachable"), FailureKind::Unauthorized=>Some("unauthorized"),
                FailureKind::VersionMismatch|FailureKind::TooOld=>Some("version_mismatch"),
                FailureKind::NodeError=>Some("error"), _=>None,
            }
        }
    }
}
pub fn classify_response(status:u16,node_version:Option<String>,body:Value)->NodeResult {
    if (200..300).contains(&status) { return NodeResult::Success{ok:true,value:body,node_version}; }
    if node_version.as_deref().is_none_or(str::is_empty) && matches!(status,404|500) {
        return NodeResult::failure(FailureKind::TooOld,"이 노드에는 노드 기능이 없습니다 (v1.15.0 이상 필요).",node_version);
    }
    let message=body["error"]["message"].as_str().unwrap_or("노드가 예상치 못한 응답을 보냈습니다.");
    let kind=match status {
        400=>FailureKind::Validation,401=>FailureKind::Unauthorized,403=>FailureKind::Forbidden,
        404=>FailureKind::NotFound,409 if body["error"]["code"]=="version_mismatch"=>FailureKind::VersionMismatch,
        409=>FailureKind::Conflict,422=>FailureKind::DomainFailure,429=>FailureKind::RateLimited,
        // The existing conflict banner preserves the transient applying or starting message without marking the node broken.
        503 if matches!(body["error"]["code"].as_str(),Some("nginx_busy"|"publisher_unavailable"))=>FailureKind::Conflict,
        _=>FailureKind::NodeError,
    };
    let field_errors=if status==400 {Some(body["error"]["fieldErrors"].as_object().map(|fields|fields.iter()
        .filter_map(|(key,val)|val.as_str().map(|val|(key.clone(),val.to_owned()))).collect()).unwrap_or_default())}else{None};
    NodeResult::Failure{ok:false,kind,message:message.to_owned(),node_version,field_errors}
}

#[derive(Clone)]
pub struct NodeHttp { verified:crate::outbound::HttpClient, insecure:crate::outbound::HttpClient, version:String }
pub fn gateway_version<'a>(local: &'a str, node: Option<&str>) -> &'a str {
    let parsed = node.and_then(crate::auth::node_version::parse_semver);
    if parsed.is_some_and(|v| (v.major, v.minor) == (1, 19))
        && crate::auth::node_version::parse_semver(local).is_some_and(|v| (v.major, v.minor) == (2, 0)) {
        "1.19.11"
    } else { local }
}

impl NodeHttp {
    pub fn new(version:&str,config:&crate::config::RuntimeConfig)->Result<Self,reqwest::Error> {
        let builder=||Client::builder().redirect(reqwest::redirect::Policy::none()).no_proxy()
            .connect_timeout(Duration::from_secs(10)).pool_idle_timeout(Duration::from_secs(60)).pool_max_idle_per_host(4);
        Ok(Self{verified:crate::outbound::HttpClient::new(config,builder)?,insecure:crate::outbound::HttpClient::new(config,||builder().tls_danger_accept_invalid_certs(true))?,version:version.to_owned()})
    }

    fn prepare(&self,secrets:&SecretBox,target:&NodeTarget,method:Method,path:&str)->Result<reqwest::RequestBuilder,NodeResult> {
        let bad=||NodeResult::failure(FailureKind::Unreachable,"노드 주소가 올바르지 않습니다.",None);
        let base=Url::parse(&target.base_url).map_err(|_|bad())?;
        if crate::outbound::validate_url(&base).is_err() || !matches!(base.scheme(),"http"|"https") || !base.username().is_empty() || base.password().is_some()
            || base.query().is_some() || base.fragment().is_some() { return Err(bad()); }
        if !path.starts_with("/api/v1/") || path.contains('\\') || path.chars().any(char::is_control) {return Err(bad())}
        let url=base.join(path).map_err(|_|bad())?;
        if url.origin()!=base.origin() || !url.path().starts_with("/api/v1/") { return Err(bad()); }
        let client=if target.allow_insecure_tls {&self.insecure}else{&self.verified};
        let client=client.for_url(&url).map_err(|_|bad())?;
        let mut request=client.request(method,url).header("x-gateway-version",gateway_version(&self.version,target.node_version.as_deref())).header("accept","application/json");
        if let Some(sealed)=&target.token_enc {
            let token=secrets.open(Some(sealed)).filter(|s|!s.is_empty()).ok_or_else(||NodeResult::failure(FailureKind::Unauthorized,"저장된 노드 토큰을 읽을 수 없습니다. 다시 연결하세요.",None))?;
            request=request.bearer_auth(token);
        }
        Ok(request)
    }

    pub async fn call(&self,secrets:&SecretBox,target:&NodeTarget,method:Method,path:&str,body:Option<&Value>,timeout:Duration)->NodeResult {
        let mut request=match self.prepare(secrets,target,method,path){Ok(r)=>r,Err(e)=>return e};
        if let Some(body)=body {request=request.json(body);}
        let result=tokio::time::timeout(timeout,async {
            let mut response=request.send().await.map_err(|_|NodeResult::failure(FailureKind::Unreachable,"노드에 연결할 수 없습니다. 주소와 인증서 신뢰 설정을 확인하세요.",None))?;
            let version=response.headers().get("x-node-version").and_then(|v|v.to_str().ok()).map(str::to_owned);
            let status=response.status().as_u16();
            let too_large=||NodeResult::failure(FailureKind::NodeError,"노드 응답이 너무 큽니다.",version.clone());
            if response.content_length().is_some_and(|length|length>MAX_BODY as u64){return Err(too_large())}
            let mut bytes=Vec::new();
            while let Some(chunk)=response.chunk().await.map_err(|_|NodeResult::failure(FailureKind::Unreachable,"노드 응답을 끝까지 읽지 못했습니다.",version.clone()))? {
                if bytes.len().saturating_add(chunk.len())>MAX_BODY {return Err(too_large())}
                bytes.extend_from_slice(&chunk);
            }
            Ok(classify_response(status,version,serde_json::from_slice(&bytes).unwrap_or(json!(null))))
        }).await;
        match result { Ok(Ok(response))=>response,Ok(Err(error))=>error,Err(_)=>NodeResult::failure(FailureKind::Unreachable,"노드 요청 시간이 초과되었습니다.",None) }
    }

    /// Retry only the authenticated info GET; both attempts share one deadline.
    pub async fn negotiate_info(&self, secrets: &SecretBox, target: &NodeTarget, instance: &str) -> NodeResult {
        let probe = async {
            let mut target = target.clone();
            let mut result = self.call(secrets, &target, Method::GET, "/api/v1/info", None, PROBE_TIMEOUT).await;
            if let NodeResult::Failure { kind: FailureKind::VersionMismatch, node_version: Some(version), .. } = &result {
                if gateway_version(&self.version, Some(version)) != gateway_version(&self.version, target.node_version.as_deref()) {
                    target.node_version = Some(version.clone());
                    result = self.call(secrets, &target, Method::GET, "/api/v1/info", None, PROBE_TIMEOUT).await;
                }
            }
            if let NodeResult::Success { value, node_version, .. } = &mut result {
                let body_version = value["semver"].as_str();
                let version = node_version.as_deref().or(body_version);
                let valid = value.is_object() && version.is_some_and(|v| crate::auth::node_version::is_compatible(&self.version, v))
                    && body_version.is_none_or(|v| crate::auth::node_version::parse_semver(v).is_some()
                        && node_version.as_deref().is_none_or(|h| crate::auth::node_version::parse_semver(h) == crate::auth::node_version::parse_semver(v)))
                    && value["instanceId"].as_str().is_some_and(|id| !id.is_empty() && (instance.is_empty() || id == instance));
                if !valid {
                    return NodeResult::failure(FailureKind::NodeError, "노드의 버전 또는 식별 정보를 확인할 수 없습니다.", None);
                }
                *node_version = version.map(str::to_owned);
            }
            result
        };
        tokio::time::timeout(PROBE_TIMEOUT, probe).await.unwrap_or_else(|_|NodeResult::failure(FailureKind::Unreachable, "노드 요청 시간이 초과되었습니다.", None))
    }

    /// Only the response handshake is bounded. The caller owns the returned
    /// streaming body and must drop it immediately on downstream disconnect.
    pub async fn open_stream(&self,secrets:&SecretBox,target:&NodeTarget,path:&str,handshake_timeout:Duration)->Result<reqwest::Response,NodeResult> {
        let request=self.prepare(secrets,target,Method::GET,path)?.header("accept","text/event-stream");
        tokio::time::timeout(handshake_timeout,async {
            let mut response=request.send().await.map_err(|_|NodeResult::failure(FailureKind::Unreachable,"노드 로그에 연결할 수 없습니다.",None))?;
            let version=response.headers().get("x-node-version").and_then(|v|v.to_str().ok()).map(str::to_owned);
            if !response.status().is_success() {
                let status=response.status().as_u16();
                let mut bytes=Vec::new();
                while let Some(chunk)=response.chunk().await.map_err(|_|NodeResult::failure(FailureKind::Unreachable,"노드 응답을 읽지 못했습니다.",version.clone()))? {
                    if bytes.len().saturating_add(chunk.len())>MAX_BODY {return Err(NodeResult::failure(FailureKind::NodeError,"노드 응답이 너무 큽니다.",version))}
                    bytes.extend_from_slice(&chunk);
                }
                return Err(classify_response(status,version,serde_json::from_slice(&bytes).unwrap_or(Value::Null)));
            }
            let content_type=response.headers().get("content-type").and_then(|v|v.to_str().ok()).unwrap_or("").split(';').next().unwrap_or("").trim();
            if !content_type.eq_ignore_ascii_case("text/event-stream") {
                return Err(NodeResult::failure(FailureKind::NodeError,"노드가 로그 스트림 형식으로 응답하지 않았습니다.",version));
            }
            Ok(response)
        }).await.unwrap_or_else(|_|Err(NodeResult::failure(FailureKind::Unreachable,"노드 로그 연결 시간이 초과되었습니다.",None)))
    }
}
