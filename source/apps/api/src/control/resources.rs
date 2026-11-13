//! Validated configuration CRUD. All writes share the apply owner and commit the audit atomically.
use super::{
    performance as perf,
    reserved_ports::{ListenContext, NGINX_BASE_LISTEN_PORTS, ReservedPorts},
    validation as valid,
};
use crate::{
    auth::{
        audit::{self, AuditEntry},
        principal::db_unavailable,
    },
    contracts::{Actor, FormInput},
    error::ApiError,
    state::AppState,
};
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use tokio_postgres::GenericClient;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    Upstream,
    Server,
    Stream,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    Create,
    Update,
    Delete,
    Toggle,
}
impl Resource {
    pub fn table(self) -> &'static str {
        match self {
            Self::Upstream => "upstreams",
            Self::Server => "nginx_servers",
            Self::Stream => "nginx_streams",
        }
    }
    pub fn singular(self) -> &'static str {
        match self {
            Self::Upstream => "upstream",
            Self::Server => "server",
            Self::Stream => "stream",
        }
    }
    pub fn plural(self) -> &'static str {
        match self {
            Self::Upstream => "upstreams",
            Self::Server => "servers",
            Self::Stream => "streams",
        }
    }
    pub fn base(self) -> &'static str {
        match self {
            Self::Upstream => "/reverse-proxy/upstreams",
            Self::Server => "/reverse-proxy",
            Self::Stream => "/stream",
        }
    }
    fn projection(self) -> &'static str {
        match self {
            Self::Upstream => {
                "jsonb_build_object('id',id,'version',row_version::text,'name',name,'servers',servers,'resolveAtRuntime',resolve_at_runtime)"
            }
            Self::Server => {
                "jsonb_build_object('id',id,'version',row_version::text,'serverName',server_name,'listen',listen,'upstreamId',upstream_id,'certificateId',certificate_id,'config',config,'enabled',enabled)"
            }
            Self::Stream => {
                "jsonb_build_object('id',id,'version',row_version::text,'listen',listen,'protocol',protocol::text,'proxyPass',proxy_pass,'enabled',enabled,'resolveAtRuntime',resolve_at_runtime)"
            }
        }
    }
}
pub fn field_error(field: &str, message: impl ToString) -> ApiError {
    validation_error(BTreeMap::from([(field.into(), message.to_string())]))
}
fn validation_error(errors: BTreeMap<String, String>) -> ApiError {
    let mut e = ApiError::new(
        StatusCode::BAD_REQUEST,
        "validation_failed",
        "입력값이 올바르지 않습니다.",
    );
    e.field_errors = Some(errors);
    e
}
fn record<T>(
    errors: &mut BTreeMap<String, String>,
    field: &str,
    result: super::Result<T>,
) -> Option<T> {
    match result {
        Ok(v) => Some(v),
        Err(e) => {
            errors.insert(field.into(), e.to_string());
            None
        }
    }
}
const PORT_PROMPT: &str = "1-65535 사이의 포트 번호를 입력하세요.";
/// 1.17.2 showed a prompt instead of the validator's range text, but kept the
/// reason a reserved port is refused.
fn port_prompt(result: super::Result<()>, prompt: &str) -> super::Result<()> {
    result.map_err(|e| match e {
        super::ControlError::Reserved(e) => e.into(),
        super::ControlError::Invalid(_) => prompt.into(),
    })
}
fn trimmed(v: &Value) -> String {
    v.as_str()
        .unwrap_or("")
        .trim_matches(super::whitespace)
        .into()
}
fn number(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => {
            let s = s.trim_matches(super::whitespace);
            if s.is_empty() {
                0.0
            } else {
                s.parse().unwrap_or(f64::NAN)
            }
        }
        Value::Null => 0.0,
        Value::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        _ => f64::NAN,
    }
}
fn input_number(raw: &Value, key: &str) -> f64 {
    raw.get(key).map(number).unwrap_or(f64::NAN)
}
fn has_nul(value: &Value) -> bool {
    match value {
        Value::String(s) => s.contains('\0'),
        Value::Array(values) => values.iter().any(has_nul),
        Value::Object(values) => values.iter().any(|(key,value)| key.contains('\0') || has_nul(value)),
        _ => false,
    }
}
fn object(raw: &Value) -> Result<(), ApiError> {
    let fields=raw.as_object().ok_or_else(||validation_error(BTreeMap::new()))?;
    for (field,value) in fields {
        if field.contains('\0') || has_nul(value) {
            return Err(field_error(field,"NUL 문자는 사용할 수 없습니다."));
        }
    }
    Ok(())
}
fn directives(raw: &Value, ctx: valid::DirectiveContext) -> super::Result<Value> {
    if raw.is_null() {
        return Ok(json!([]));
    }
    let list = raw.as_array().ok_or("디렉티브 형식이 올바르지 않습니다.")?;
    let mut out = Vec::new();
    for d in list {
        if !d.is_object() {
            return Err("디렉티브 형식이 올바르지 않습니다.".into());
        }
        let name = d
            .get("name")
            .filter(|v| !v.is_null())
            .map(super::input_string)
            .unwrap_or_default();
        let args: Vec<String> = d["args"]
            .as_array()
            .into_iter()
            .flatten()
            .map(super::input_string)
            .collect();
        valid::assert_valid_directive(&name, &args, ctx)?;
        out.push(json!({"name":name,"args":args}));
    }
    valid::assert_directive_constraints(&serde_json::from_value::<Vec<super::Directive>>(json!(out)).map_err(|_| "Invalid directives")?)?;
    Ok(json!(out))
}
fn ip_access(raw: &Value) -> super::Result<Option<Value>> {
    if raw.is_null() || matches!(raw["mode"].as_str(),Some(""|"none"|"off")) { return Ok(None); }
    if !raw.is_object() || !matches!(raw["mode"].as_str(), Some("whitelist" | "blacklist")) {
        return Err("IP 접근 제어 형식이 올바르지 않습니다.".into());
    }
    let values=raw["entries"].as_array().ok_or("IP 접근 제어 형식이 올바르지 않습니다.")?;
    let mut entries=Vec::new();
    for value in values {
        let entry=value.as_str().ok_or("IP 접근 제어 형식이 올바르지 않습니다.")?.trim_matches(super::whitespace);
        if entry.is_empty() { continue; }
        valid::assert_valid_ip_or_cidr(entry)?;
        entries.push(entry.to_owned());
    }
    if entries.is_empty() { return Err("허용/차단할 IP를 1개 이상 입력하세요.".into()); }
    Ok(Some(json!({"mode":raw["mode"],"entries":entries})))
}
fn tls_options(raw: &Value) -> super::Result<Option<Value>> {
    if raw.is_null() { return Ok(None); }
    if !raw.is_object() { return Err("TLS 설정 형식이 올바르지 않습니다.".into()); }
    let mut out = json!({});
    for k in ["sslProtocols", "sslCiphers"] {
        if let Some(value)=raw.get(k) {
            if !matches!(value.as_str(), Some("modern" | "intermediate")) {
                return Err("TLS 설정 형식이 올바르지 않습니다.".into());
            }
            out[k]=value.clone();
        }
    }
    if raw.get("preferServerCiphers").is_some_and(|v| !v.is_null() && !v.is_boolean()) {
        return Err("TLS 설정 형식이 올바르지 않습니다.".into());
    }
    if raw["preferServerCiphers"] == true { out["preferServerCiphers"] = json!(true); }
    let h = &raw["hsts"];
    if !h.is_null() && !h.is_object() { return Err("HSTS 설정 형식이 올바르지 않습니다.".into()); }
    for key in ["enabled","includeSubDomains","preload"] {
        if h.get(key).is_some_and(|v| !v.is_null() && !v.is_boolean()) {
            return Err("HSTS 설정은 true 또는 false여야 합니다.".into());
        }
    }
    let supplied_age = h.get("maxAge").and_then(Value::as_f64)
        .filter(|age| age.is_finite() && age.fract() == 0.0 && *age >= 0.0);
    let age = supplied_age.unwrap_or(63072000.0);
    if h["enabled"] == true {
        let sub = h["includeSubDomains"] == true;
        out["hsts"] = json!({"enabled":true,"maxAge":age,"includeSubDomains":sub,"preload":h["preload"]==true&&supplied_age.is_some()&&age>=31536000.0&&sub});
    }
    Ok((!out.as_object().unwrap().is_empty()).then_some(out))
}
pub fn parse_upstream(raw: &Value) -> Result<Value, ApiError> {
    let built = parse_upstream_unbounded(raw)?;
    check_save_sizes(Resource::Upstream, &built, None)?;
    Ok(built)
}
fn parse_upstream_unbounded(raw: &Value) -> Result<Value, ApiError> {
    object(raw)?;
    let mut errors = BTreeMap::new();
    let runtime_dns = record(
        &mut errors,
        "resolveAtRuntime",
        valid::parse_runtime_dns_flag(raw),
    )
    .unwrap_or(false);
    let name = trimmed(&raw["name"]);
    if name.is_empty() {
        errors.insert("name".into(), "이름을 입력하세요.".into());
    } else {
        record(
            &mut errors,
            "name",
            valid::assert_valid_upstream_name(&name),
        );
    }
    let servers: Vec<String> = raw["servers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| {
            super::input_string(v)
                .trim_matches(super::whitespace)
                .to_owned()
        })
        .filter(|s| !s.is_empty())
        .collect();
    if servers.is_empty() {
        errors.insert(
            "servers".into(),
            "최소 1개의 서버 주소를 입력하세요. (한 줄에 하나씩)".into(),
        );
    }
    for s in &servers {
        if record(
            &mut errors,
            "servers",
            if runtime_dns {
                valid::assert_valid_runtime_host_port(s, true)
            } else {
                valid::assert_valid_upstream_server(s)
            },
        )
        .is_none()
        {
            break;
        }
    }
    if !errors.is_empty() {
        return Err(validation_error(errors));
    }
    Ok(json!({"name":name,"servers":servers,"resolveAtRuntime":runtime_dns}))
}
pub fn parse_stream(raw: &Value, reserved: &ReservedPorts) -> Result<Value, ApiError> {
    let built = parse_stream_unbounded(raw, reserved)?;
    check_save_sizes(Resource::Stream, &built, None)?;
    Ok(built)
}
fn parse_stream_unbounded(raw: &Value, reserved: &ReservedPorts) -> Result<Value, ApiError> {
    object(raw)?;
    let mut errors = BTreeMap::new();
    let runtime_dns = record(
        &mut errors,
        "resolveAtRuntime",
        valid::parse_runtime_dns_flag(raw),
    )
    .unwrap_or(false);
    let protocol = raw["protocol"].as_str().unwrap_or("");
    if !matches!(protocol, "tcp" | "udp") {
        errors.insert("protocol".into(), "tcp 또는 udp를 선택하세요.".into());
    }
    let listen = input_number(raw, "listen");
    record(
        &mut errors,
        "listen",
        port_prompt(
            valid::assert_valid_listen_port(
                listen,
                if protocol == "udp" {
                    ListenContext::StreamUdp
                } else {
                    ListenContext::StreamTcp
                },
                reserved,
            ),
            PORT_PROMPT,
        ),
    );
    let target = trimmed(&raw["proxyPass"]);
    if target.is_empty() {
        errors.insert(
            "proxyPass".into(),
            "proxy_pass 대상을 입력하세요. (host:port)".into(),
        );

    } else if let Err(error) = if runtime_dns {
        valid::assert_valid_runtime_host_port(&target, false)
    } else {
        valid::assert_valid_stream_proxy_pass(&target)
    } {
        errors.insert(
            "proxyPass".into(),
            if runtime_dns && super::internal_socket::is_unix_target(&target) {
                error.to_string()
            } else if runtime_dns {
                "DNS이름:포트 형식으로 입력하세요. IP 주소에는 실행 중 이름 확인을 켤 수 없습니다."
                    .into()
            } else {
                "host:port 형식으로 입력하세요. (예: 10.0.0.5:5432)".into()
            },
        );
    }
    if !errors.is_empty() {
        return Err(validation_error(errors));
    }
    Ok(
        json!({"listen":listen as i32,"protocol":protocol,"proxyPass":target,"resolveAtRuntime":runtime_dns}),
    )
}
pub fn parse_server(raw: &Value, reserved: &ReservedPorts) -> Result<Value, ApiError> {
    let built = parse_server_with(raw, reserved, &valid::assert_valid_server_config, None)?;
    check_save_sizes(Resource::Server, &built, None)?;
    Ok(built)
}

async fn parse_server_for_save<F>(raw: Value, reserved: ReservedPorts, validate: F, before: Option<Value>) -> Result<Value, ApiError>
where
    F: Fn(&Value) -> super::Result<()> + Send + 'static,
{
    super::internal_targets::blocking(move || parse_server_with(&raw, &reserved, &validate, before.as_ref())).await?
}

fn parse_server_with(
    raw: &Value,
    reserved: &ReservedPorts,
    validate: &dyn Fn(&Value) -> super::Result<()>,
    before: Option<&Value>,
) -> Result<Value, ApiError> {
    object(raw)?;
    let previous = before.map(|row| &row["config"]).unwrap_or(&Value::Null);
    let old_locations = super::render::effective_locations(previous);
    let validate_target = |target: &str, old: &Value| {
        if old.as_str() == Some(target) {
            valid::assert_valid_stored_proxy_target(target)
        } else {
            valid::assert_valid_proxy_target(target)
        }
    };
    let mut errors = BTreeMap::new();
    let runtime_dns = record(
        &mut errors,
        "resolveAtRuntime",
        valid::parse_runtime_dns_flag(raw),
    )
    .unwrap_or(false);
    let name = trimmed(&raw["serverName"]);
    let listen = input_number(raw, "listen");
    if name.is_empty() {
        errors.insert("serverName".into(), "server_name을 입력하세요.".into());
    } else {
        record(
            &mut errors,
            "serverName",
            valid::assert_valid_server_name(&name),
        );
    }
    record(
        &mut errors,
        "listen",
        port_prompt(
            valid::assert_valid_http_port(listen, false, reserved),
            PORT_PROMPT,
        ),
    );
    let mut out = json!({"serverName":name,"listen":listen as i32,"upstreamId":null,"certificateId":null,"config":{}});
    let cfg = &mut out["config"];
    if let Some(d) = record(
        &mut errors,
        "serverDirectives",
        directives(&raw["serverDirectives"], valid::DirectiveContext::Server),
    ) {
        if !d.as_array().unwrap().is_empty() || raw.get("serverDirectives").is_some_and(Value::is_array) {
            cfg["serverDirectives"] = d;
        }
    }
    if let Some(Some(v)) = record(&mut errors, "ipAccess", ip_access(&raw["ipAccess"])) {
        cfg["ipAccess"] = v;
    }
    if raw.get("realIp").is_some() && raw.get("realIp") == previous.get("realIp") {
        if record(&mut errors, "realIp", valid::parse_stored_real_ip_config(&raw["realIp"])).is_some() {
            cfg["realIp"] = raw["realIp"].clone();
        }
    } else if let Some(Some(v)) = record(&mut errors, "realIp", valid::parse_real_ip_config(&raw["realIp"])) {
        cfg["realIp"] = v;
    }
    if let Some(Some(v)) = record(
        &mut errors,
        "cache",
        perf::parse_cache_config(&raw["cache"]),
    ) {
        cfg["cache"] = json!(v);
    }
    if let Some(Some(v)) = record(&mut errors, "gzip", perf::parse_gzip_config(&raw["gzip"])) {
        cfg["gzip"] = json!(v);
    }
    if raw["targetMode"] == "locations" {
        if runtime_dns {
            errors.insert(
                "resolveAtRuntime".into(),
                "여러 경로에서는 각 직접 대상의 이름 확인 방식을 선택하세요.".into(),
            );
        }
        let mut locations = Vec::new();
        let list = raw["locations"].as_array().cloned().unwrap_or_default();
        if list.is_empty() {
            errors.insert("locations".into(), "location을 최소 1개 추가하세요.".into());
        }
        for (i, l) in list.iter().enumerate() {
            // Like 1.17.2: a location is named by its path once the path is accepted,
            // and its target is checked before its directives.
            let parsed = (|| -> std::result::Result<Value, String> {
                if !l.is_object() {
                    return Err(format!(
                        "location {}: 설정 형식이 올바르지 않습니다.",
                        i + 1
                    ));
                }
                let path = trimmed(&l["path"]);
                valid::assert_valid_location_path(&path)
                    .map_err(|e| format!("location {}: {e}", i + 1))?;
                let at = |e: &dyn std::fmt::Display| format!("location {} ({path}): {e}", i + 1);
                let location_runtime_dns = valid::parse_runtime_dns_flag(l).map_err(|e| at(&e))?;
                let retry=match l.get("retryOtherInstances"){None=>false,Some(Value::Bool(value))=>*value,_=>return Err(at(&"재시도 설정은 참 또는 거짓이어야 합니다."))};
                let target = if l["mode"] == "direct" {
                    let p = trimmed(&l["proxyPass"]);
                    let previous = old_locations.iter().find(|old| old["path"] == path)
                        .or_else(|| old_locations.get(i).filter(|_| list.len() == old_locations.len()))
                        .unwrap_or(&Value::Null);
                    validate_target(&p, &previous["proxyPass"]).map_err(|e| at(&e))?;
                    if location_runtime_dns {
                        valid::assert_valid_runtime_proxy_target(&p).map_err(|e| at(&e))?;
                    }
                    ("proxyPass", p)
                } else {
                    if location_runtime_dns {
                        return Err(at(
                            &"upstream의 이름 확인 방식은 그 upstream에서 선택하세요.",
                        ));
                    }
                    let id = trimmed(&l["upstreamId"]);
                    if id.is_empty() {
                        return Err(at(&"upstream을 선택하세요."));
                    }
                    ("upstreamId", id)
                };
                let dirs = directives(&l["directives"], valid::DirectiveContext::Location)
                    .map_err(|e| at(&e))?;
                let mut loc = json!({"path":path});
                if l.get("directives")!=Some(&Value::Null){loc["directives"]=dirs;}
                if location_runtime_dns {
                    loc["resolveAtRuntime"] = json!(true);
                }
                if retry{loc["retryOtherInstances"]=json!(true);}
                loc[target.0] = json!(target.1);
                if let Some(v) = ip_access(&l["ipAccess"]).map_err(|e| at(&e))? {
                    loc["ipAccess"] = v;
                }
                for k in ["cacheEnabled", "gzipEnabled"] {
                    if let Some(v) = perf::parse_performance_override(&l[k]).map_err(|e| at(&e))? {
                        loc[k] = json!(v);
                    }
                }
                Ok(loc)
            })();
            match parsed {
                Ok(v) => locations.push(v),
                Err(e) => {
                    errors.insert("locations".into(), e);
                    break;
                }
            }
        }
        cfg["locations"] = json!(locations);
    } else {
        let target = trimmed(&raw["proxyPass"]);
        let upstream = trimmed(&raw["upstreamId"]);
        if !target.is_empty() {
            record(
                &mut errors,
                "proxyPass",
                validate_target(&target, &old_locations.first().unwrap_or(&Value::Null)["proxyPass"]),
            );
            if runtime_dns {
                record(
                    &mut errors,
                    "proxyPass",
                    valid::assert_valid_runtime_proxy_target(&target),
                );
                cfg["resolveAtRuntime"] = json!(true);
            }
            cfg["proxyPass"] = json!(target);
        } else if runtime_dns {
            errors.insert(
                "resolveAtRuntime".into(),
                "upstream의 이름 확인 방식은 그 upstream에서 선택하세요.".into(),
            );
        } else if upstream.is_empty() {
            errors.insert("upstreamId".into(), "upstream을 선택하세요.".into());
        }
        if let Some(d) = record(
            &mut errors,
            "locationDirectives",
            directives(
                &raw["locationDirectives"],
                valid::DirectiveContext::Location,
            ),
        ) {
            cfg["locationDirectives"] = d;
        }
    }
    if raw["tls"].get("redirectHttp").is_some_and(|value| !value.is_boolean()) {
        errors.insert("redirectHttp".into(), "HTTP 전환 설정은 true 또는 false여야 합니다.".into());
    }
    let cert = match raw.get("certificateId") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(value)) => value.trim_matches(super::whitespace).to_owned(),
        _ => { errors.insert("certificateId".into(),"인증서 ID는 문자열이어야 합니다.".into()); String::new() }
    };
    if !cert.is_empty() {
        let tls = &raw["tls"];
        let port = tls.get("tlsPort").map(number).unwrap_or(443.0);
        record(
            &mut errors,
            "tlsPort",
            port_prompt(
                valid::assert_valid_http_port(port, true, reserved),
                "1-65535 사이의 HTTPS 포트를 입력하세요.",
            ),
        );
        if port == listen { errors.insert("tlsPort".into(), "HTTP·HTTPS 포트는 서로 달라야 합니다.".into()); }
        cfg["tlsPort"] = json!(port as i32);
        cfg["redirectHttp"] = json!(tls["redirectHttp"] != false);
        if let Some(Some(v)) = record(&mut errors,"tlsOptions",tls_options(&tls["tlsOptions"])) {
            cfg["tlsOptions"] = v;
        }
    }
    if let Err(e) = valid::assert_runtime_response_directives(raw) {
        errors.insert(
            if raw["targetMode"] == "locations" {
                "locations"
            } else {
                "locationDirectives"
            }
            .into(),
            e.to_string(),
        );
    }
    if let Err(e) = validate(cfg) {
        errors.insert(if raw["targetMode"] == "locations" {"locations"} else {"locationDirectives"}.into(),e.to_string());
    }
    if errors.is_empty() {
        if let Err(e) = perf::validate_performance_conflicts(cfg) {
            let msg = e.to_string();
            errors.insert(
                if msg.contains("gzip") {
                    "gzip"
                } else {
                    "cache"
                }
                .into(),
                msg,
            );
        }
    }
    if !errors.is_empty() {
        return Err(validation_error(errors));
    }
    if !cert.is_empty() {
        out["certificateId"] = json!(cert);
    }
    if raw["targetMode"] != "locations" {
        let cfg=&mut out["config"];let object=cfg.as_object_mut().unwrap();
        let mut location=json!({"path":"/"});
        if let Some(target)=object.remove("proxyPass"){location["proxyPass"]=target;}else{location["upstreamId"]=json!(trimmed(&raw["upstreamId"]));}
        let dirs=object.remove("locationDirectives").unwrap_or_else(||json!([]));
        if raw.get("locationDirectives")!=Some(&Value::Null){location["directives"]=dirs;}
        if object.remove("resolveAtRuntime")==Some(json!(true)){location["resolveAtRuntime"]=json!(true);}
        cfg["locations"]=json!([location]);
    }
    Ok(out)
}

pub async fn list(client: &impl GenericClient, kind: Resource) -> Result<Vec<Value>, ApiError> {
    let order = match kind {
        Resource::Upstream => "name",
        Resource::Server => "server_name",
        Resource::Stream => "listen",
    };
    let sql = format!(
        "SELECT {} FROM {} ORDER BY {}, id",
        kind.projection(),
        kind.table(),
        order
    );
    Ok(client
        .query(&sql, &[])
        .await
        .map_err(|_| db_unavailable())?
        .into_iter()
        .map(|r| r.get(0))
        .collect())
}
pub async fn get(client: &impl GenericClient, kind: Resource, id: &str) -> Result<Value, ApiError> {
    if id.contains('\0') { return Err(field_error("id","NUL 문자는 사용할 수 없습니다.")); }
    let sql = format!(
        "SELECT {} FROM {} WHERE id=$1",
        kind.projection(),
        kind.table()
    );
    client
        .query_opt(&sql, &[&id])
        .await
        .map_err(|_| db_unavailable())?
        .map(|r| r.get(0))
        .ok_or_else(ApiError::not_found)
}
pub async fn reserved_ports(
    client: &impl GenericClient,
    state: &AppState,
) -> Result<ReservedPorts, ApiError> {
    reserved_ports_except(client,state,None).await
}
async fn reserved_ports_except(client:&impl GenericClient,state:&AppState,exclude:Option<&str>)->Result<ReservedPorts,ApiError>{
    let dbport = state
        .config
        .database_url
        .parse::<tokio_postgres::Config>()
        .ok()
        .and_then(|c| c.get_ports().first().copied())
        .unwrap_or(5432);
    let webport = state.config.web_port;
    let mut out = ReservedPorts {
        process: vec![webport, state.config.listen.port(), dbport],
        http_plain: vec![80,super::self_proxy::ADMIN_UI_PORT],
        http_tls: vec![443],
        ..Default::default()
    };
    crate::dns_guard::reserve_port(&mut out, &crate::dns_guard::status(&state.config));
    for p in NGINX_BASE_LISTEN_PORTS {
        out.http.push(p);
        out.owners.entry(p).or_insert_with(|| {
            if p == super::self_proxy::ADMIN_UI_PORT {
                format!("관리 화면 :{p}")
            } else {
                format!("nginx 기본 리스너 :{p}")
            }
        });
    }
    for s in list(client, Resource::Server).await? {
        if s["id"].as_str().is_some_and(|id|Some(id)==exclude) {continue;}
        if let Some(port)=s["listen"].as_u64().filter(|p| *p>0 && *p<=65535) {out.http_plain.push(port as u16);}
        if let Some(port)=s["config"]["tlsPort"].as_u64().filter(|p| *p>0 && *p<=65535) {out.http_tls.push(port as u16);}
        for (v, suffix) in [
            (&s["listen"], ""),
            (&s["config"]["tlsPort"], " 의 HTTPS 포트"),
        ] {
            if let Some(p) = v.as_u64().filter(|p| *p > 0 && *p <= 65535) {
                out.http.push(p as u16);
                out.owners.entry(p as u16).or_insert_with(|| {
                    format!(
                        "리버스 프록시 {}{suffix}",
                        s["serverName"].as_str().unwrap_or("(이름 없음)")
                    )
                });
            }
        }
    }
    for s in list(client, Resource::Stream).await? {
        if s["protocol"] == "tcp" {
            if let Some(p) = s["listen"].as_u64().filter(|p| *p > 0 && *p <= 65535) {
                out.stream_tcp.push(p as u16);
                out.owners.entry(p as u16).or_insert_with(|| {
                    format!("stream → {}", s["proxyPass"].as_str().unwrap_or(""))
                });
            }
        }
    }
    for (p, label) in [
        (webport, "웹 앱 (PORT)"),
        (state.config.listen.port(), "API (API_PORT)"),
        (dbport, "데이터베이스 (DATABASE_URL)"),
    ] {
        out.owners.entry(p).or_insert_with(|| label.into());
    }
    out.http.sort_unstable();
    out.http.dedup();
    out.stream_tcp.sort_unstable();
    out.stream_tcp.dedup();
    Ok(out)
}
async fn resolve_server(client: &impl GenericClient, built: &mut Value) -> Result<Vec<Value>, ApiError> {
    let upstreams = list(client, Resource::Upstream).await?;
    let cfg = &built["config"];
    for target in std::iter::once(cfg).chain(cfg["locations"].as_array().into_iter().flatten()) {
        if target["resolveAtRuntime"] == true {
            let authority = target["proxyPass"]
                .as_str()
                .and_then(|s| s.split_once("://"))
                .map(|(_, s)| s.split(['/', '?']).next().unwrap_or(""));
            if authority.is_some_and(|a| {
                upstreams.iter().any(|u| {
                    u["name"]
                        .as_str()
                        .is_some_and(|n| n.eq_ignore_ascii_case(a))
                })
            }) {
                return Err(field_error(
                    "resolveAtRuntime",
                    "upstream의 이름 확인 방식은 그 upstream에서 선택하세요.",
                ));
            }
        }
    }
    if let Some(id) = built["upstreamId"].as_str() {
        if !upstreams.iter().any(|u| u["id"] == id) {
            return Err(field_error(
                "upstreamId",
                "선택한 upstream을 찾을 수 없습니다.",
            ));
        }
    }
    if let Some(locations) = built["config"]["locations"].as_array_mut() {
        for (i, l) in locations.iter_mut().enumerate() {
            if let Some(id) = l["upstreamId"].as_str() {
                upstreams.iter().find(|u| u["id"] == id).ok_or_else(|| {
                    field_error(
                        "locations",
                        format!(
                            "location {} ({}): 선택한 upstream을 찾을 수 없습니다.",
                            i + 1,
                            l["path"].as_str().unwrap_or("")
                        ),
                    )
                })?;
            }else if let Some(upstream)=upstreams.iter().find(|u|l["proxyPass"].as_str().is_some_and(|target|u["name"].as_str().is_some_and(|name|target==format!("http://{name}")))){
                l["upstreamId"]=upstream["id"].clone();l.as_object_mut().unwrap().remove("proxyPass");
            }
        }
    }
    if let Some(id) = built["certificateId"].as_str() {
        let row=client.query_opt("SELECT status::text FROM certificates WHERE id=$1", &[&id])
            .await.map_err(|_|db_unavailable())?
            .ok_or_else(||field_error("certificateId","선택한 인증서를 찾을 수 없습니다."))?;
        if row.get::<_,String>(0)!="issued" {
            return Err(field_error("certificateId","발급이 완료된 인증서만 선택할 수 있습니다. 인증서 발급 상태를 확인한 뒤 다시 저장하세요."));
        }
    }
    Ok(upstreams)
}
fn check_legacy_gateway_replacement(kind: Resource, before: &Value, built: &Value) -> Result<(), ApiError> {
    let loses = if kind==Resource::Server {
        let old=super::render::effective_locations(&before["config"]);
        let new=super::render::effective_locations(&built["config"]);
        let global_runtime_lost=before["config"]["resolveAtRuntime"]==true && built["config"]["resolveAtRuntime"]!=true
            && !new.iter().any(|row|row["resolveAtRuntime"]==true);
        global_runtime_lost || old.iter().any(|previous| {
            let next=new.iter().find(|row|row["path"]==previous["path"]).unwrap_or(&Value::Null);
            ["resolveAtRuntime","retryOtherInstances"].iter().any(|key|previous[*key]==true && next[*key]!=true)
        })
    } else {
        before["resolveAtRuntime"]==true && built["resolveAtRuntime"]!=true
    };
    if loses {
        return Err(ApiError::new(StatusCode::CONFLICT,"gateway_upgrade_required",
            "이 항목은 2.0 기능을 쓰고 있어 1.19 게이트웨이에서 수정할 수 없습니다 — 게이트웨이를 2.0으로 올리세요"));
    }
    Ok(())
}
// Bounds apply to changed values, so an unrelated edit never quarantines legacy data.
fn check_save_sizes(kind: Resource, built: &Value, before: Option<&Value>) -> Result<(), ApiError> {
    let old = before.unwrap_or(&Value::Null);
    let parameter = |value: &Value, previous: &Value, field: &str, max: usize| -> Result<(), ApiError> {
        if value != previous && value.as_str().is_some_and(|value| value.len()>max) {
            return Err(field_error(field,format!("nginx 설정 값은 {max}바이트 이하여야 합니다.")));
        }
        Ok(())
    };
    let directives = |values: &Value, previous: &Value, field: &str| -> Result<(), ApiError> {
        if values==previous { return Ok(()); }
        let old = previous.as_array().map(Vec::as_slice).unwrap_or(&[]);
        for (i,d) in values.as_array().into_iter().flatten().enumerate() {
            let previous = old.get(i).filter(|row| *row==d).or_else(||old.iter().find(|row| *row==d))
                .or_else(||old.iter().find(|row|row["name"]==d["name"] && row["args"][0]==d["args"][0]))
                .or_else(||old.get(i).filter(|row|row["name"]==d["name"])).unwrap_or(&Value::Null);
            for (j,arg) in d["args"].as_array().into_iter().flatten().enumerate() {
                parameter(arg,&previous["args"][j],field,valid::CONFIG_PARAMETER_BYTES)?;
            }
            if d["args"][0]!=previous["args"][0] {
                let args=d["args"][0].as_str().map(|s|vec![s.to_owned()]).unwrap_or_default();
                valid::assert_directive_save_sizes(d["name"].as_str().unwrap_or(""),&args).map_err(|e|field_error(field,e))?;
            }
        }
        Ok(())
    };
    match kind {
        Resource::Upstream => {
            parameter(&built["name"],&old["name"],"name",valid::CONFIG_PARAMETER_BYTES-"http://".len())?;
            for value in built["servers"].as_array().into_iter().flatten() {
                if !old["servers"].as_array().is_some_and(|values|values.contains(value)) {
                    parameter(value,&Value::Null,"servers",valid::CONFIG_PARAMETER_BYTES)?;
                }
            }
        }
        Resource::Stream => parameter(&built["proxyPass"],&old["proxyPass"],"proxyPass",valid::CONFIG_PARAMETER_BYTES)?,
        Resource::Server => {
            if built["serverName"]!=old["serverName"] {
                parameter(&built["serverName"],&old["serverName"],"serverName",valid::SERVER_NAMES_BYTES)?;
                let previous:HashSet<&str>=old["serverName"].as_str().unwrap_or("").split_whitespace().collect();
                let added=built["serverName"].as_str().unwrap_or("").split_whitespace()
                    .filter(|name|!previous.contains(*name)).collect::<Vec<_>>().join(" ");
                valid::assert_server_name_save_size(&added).map_err(|e|field_error("serverName",e))?;
            }
            let cfg=&built["config"];let previous=&old["config"];
            directives(&cfg["serverDirectives"],&previous["serverDirectives"],"serverDirectives")?;
            let old_locations=super::render::effective_locations(previous);
            let locations=cfg["locations"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            for (i,location) in locations.iter().enumerate() {
                let previous=old_locations.iter().find(|row|row["path"]==location["path"])
                    .or_else(||old_locations.get(i).filter(|_|locations.len()==old_locations.len())).unwrap_or(&Value::Null);
                parameter(&location["path"],&previous["path"],"locations",valid::CONFIG_PARAMETER_BYTES)?;
                parameter(&location["proxyPass"],&previous["proxyPass"],"locations",valid::CONFIG_PARAMETER_BYTES)?;
                directives(&location["directives"],&previous["directives"],"locations")?;
            }
        }
    }
    Ok(())
}
fn listener_ports(server: &Value) -> [(&'static str, Option<u64>); 2] {
    let tls = server["certificateId"].as_str().filter(|id| !id.is_empty())
        .map(|_| server["config"]["tlsPort"].as_u64().unwrap_or(443));
    [("listen", server["listen"].as_u64()), ("tlsPort", tls)]
}
async fn check_listener_conflicts(
    client: &impl GenericClient, kind: Resource, built: &Value, id: &str, before: Option<&Value>,
) -> Result<(), ApiError> {
    if kind == Resource::Upstream { return Ok(()); }
    let names: HashSet<String> = built["serverName"].as_str().unwrap_or("")
        .split_whitespace().map(str::to_ascii_lowercase).collect();
    let old_names:HashSet<String>=before.and_then(|old|old["serverName"].as_str()).unwrap_or("")
        .split_whitespace().map(str::to_ascii_lowercase).collect();
    for other in list(client, kind).await? {
        if other["id"].as_str() == Some(id) { continue; }
        if kind == Resource::Stream {
            if other["listen"] == built["listen"] && other["protocol"] == built["protocol"]
                && !before.is_some_and(|old| old["listen"]==built["listen"] && old["protocol"]==built["protocol"]) {
                return Err(field_error("listen", format!(
                    "포트 {}/{} 는 이미 stream 「{}」이 사용합니다. 다른 포트를 선택하세요.",
                    built["listen"], built["protocol"].as_str().unwrap_or("").to_ascii_uppercase(), other["id"].as_str().unwrap_or("")
                )));
            }
        } else if other["serverName"].as_str().unwrap_or("").split_whitespace()
            .any(|name| names.contains(&name.to_ascii_lowercase())) {
            let ports = listener_ports(&other);
            for (field, port) in listener_ports(built) {
                if let Some(port) = port && ports.iter().any(|(_, value)| *value == Some(port)) {
                    // Existing overlaps keep their UUID order; only newly overlapping tokens/ports fail.
                    if before.is_some_and(|old| listener_ports(old).iter().any(|(_, value)| *value==Some(port))
                        && other["serverName"].as_str().unwrap_or("").split_whitespace()
                            .filter(|name| names.contains(&name.to_ascii_lowercase()))
                            .all(|name|old_names.contains(&name.to_ascii_lowercase()))) {
                        continue;
                    }
                    return Err(field_error(field, format!(
                        "포트 {port} 에서 리버스 프록시 「{}」와 서버 이름이 겹칩니다. 이름 또는 포트를 변경하세요.",
                        other["serverName"].as_str().unwrap_or("")
                    )));
                }
            }
        }
    }
    Ok(())
}
fn sql_error(e: tokio_postgres::Error) -> ApiError {
    match e.code().map(|c| c.code()) {
        Some("23505") => field_error("name", "이미 존재하는 이름입니다."),
        Some("23503") => ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            "다른 설정에서 사용 중이거나 대상을 찾을 수 없습니다.",
        ),
        _ => ApiError::internal(),
    }
}

fn expected_version(raw: &Value) -> Result<Option<i64>, ApiError> {
    let Some(value) = raw.get("version") else {
        return Ok(None);
    };
    let version = value.as_str()
        .filter(|s| s.len() <= 19 && !s.starts_with('0') && s.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|v| *v > 0);
    version.map(Some).ok_or_else(|| {
        field_error("version", "설정 버전이 올바르지 않습니다. 페이지를 새로고침하세요.")
    })
}
fn edit_conflict() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "edit_conflict",
        "다른 사용자가 이 설정을 변경했습니다. 입력한 내용을 복사해 둔 뒤 페이지를 새로고침하고 다시 저장하세요.",
    )
}

fn check_new_upstream_loops(
    config: &crate::config::RuntimeConfig,
    candidate: &Value,
    previous: Option<&Value>,
    consumers: &[Value],
) -> Result<(), ApiError> {
    if !config.in_container { return Ok(()); }
    // Before/after checks share one DNS answer per endpoint, including rebinding names.
    let answers = std::cell::RefCell::new(BTreeMap::new());
    let resolve_cached = |host: &str, port: u16| {
        answers.borrow_mut().entry((host.to_owned(), port))
            .or_insert_with(|| super::internal_targets::system_resolve(host, port)).clone()
    };
    for server in consumers {
        let uses = |group: &Value| {
            let groups = std::slice::from_ref(group);
            let references = |row: &Value, target: &Value| super::internal_targets::referenced_upstream(
                groups, row["upstreamId"].as_str(), target.as_str(),
            ).is_some();
            references(server, &server["config"]["proxyPass"])
                || server["config"]["locations"].as_array().into_iter().flatten()
                    .any(|location| references(location, &location["proxyPass"]))
        };
        if !uses(candidate) { continue; }
        let loops = |group: &Value, port: u16| {
            let empty = |_: &str, _: u16| Vec::new();
            let resolve: super::internal_targets::Resolve = if group["resolveAtRuntime"] == true {
                &empty
            } else { &resolve_cached };
            group["servers"].as_array().into_iter().flatten().filter_map(Value::as_str).any(|member| {
                member.split_whitespace().next().and_then(|entry| entry.rsplit_once(':'))
                    .and_then(|(host, value)| value.parse::<u16>().ok().map(|value| (host, value)))
                    .is_some_and(|(host, value)| super::internal_targets::loops_to_own_listener(host, value, &[port], resolve))
            })
        };
        let mut ports: Vec<_> = server["listen"].as_u64().and_then(|port| u16::try_from(port).ok()).into_iter().collect();
        if server["certificateId"].as_str().is_some() {
            ports.push(server["config"]["tlsPort"].as_u64().and_then(|port| u16::try_from(port).ok()).unwrap_or(443));
        }
        for port in ports {
            // Compare edges by consumer and port; an old HTTP loop cannot excuse a new TLS loop.
            if loops(candidate, port) && !previous.is_some_and(|old| uses(old) && loops(old, port)) {
                return Err(field_error("servers", format!("서버 {}의 포트 {port}로 되돌아가는 upstream 멤버를 추가할 수 없습니다.", server["serverName"].as_str().unwrap_or("?"))));
            }
        }
    }
    Ok(())
}

/// Update is full validated replacement, like the former PATCH DTO, not a partial JSON merge.
/// Missing optional config fields clear their previous value; enabled is retained separately.
pub async fn mutate(
    state: &AppState, kind: Resource, operation: Mutation, id: Option<&str>, raw: Value, actor: &Actor,
) -> Result<Value, ApiError> {
    mutate_for_gateway(state,kind,operation,id,raw,actor,None).await
}

/// Keep legacy gateways from silently clearing settings that their UI cannot represent.
pub async fn mutate_for_gateway(
    state: &AppState,
    kind: Resource,
    operation: Mutation,
    id: Option<&str>,
    raw: Value,
    actor: &Actor,
    gateway_version: Option<&str>,
) -> Result<Value, ApiError> {
    // A queued write could outlive a gateway's deadline and commit after reporting
    // failure. Reject contention before starting a transaction instead.
    let _guard = state.runtime.apply_lock.try_lock().map_err(|_| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "config_busy",
            "다른 설정 작업을 처리 중입니다. 잠시 후 다시 시도하세요.",
        )
    })?;
    if id.is_some_and(|value|value.contains('\0')) { return Err(field_error("id","NUL 문자는 사용할 수 없습니다.")); }
    if matches!(operation,Mutation::Create|Mutation::Update) { object(&raw)?; }
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(|_| db_unavailable())?;
    let id = if operation == Mutation::Create {
        uuid::Uuid::new_v4().to_string()
    } else {
        id.filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "잘못된 요청입니다.")
            })?
            .into()
    };
    let before = if operation != Mutation::Create {
        Some(get(&*tx, kind, &id).await?)
    } else {
        None
    };
    let version = if operation == Mutation::Update { expected_version(&raw)? } else { None };
    if let Some(version) = version {
        if before.as_ref().and_then(|row|row["version"].as_str()).and_then(|s|s.parse::<i64>().ok()) != Some(version) {
            return Err(edit_conflict());
        }
    }
    if kind == Resource::Upstream && matches!(operation,Mutation::Delete|Mutation::Update) {
        if let Some(old) = before.as_ref().and_then(|v|v["name"].as_str()).filter(|name|operation==Mutation::Delete || raw["name"].as_str().map(str::trim)!=Some(*name)) {
            let target=old;
            let used:bool=tx.query_one("SELECT EXISTS(SELECT 1 FROM nginx_servers s CROSS JOIN LATERAL jsonb_array_elements(CASE WHEN jsonb_typeof(s.config->'locations')='array' THEN s.config->'locations' ELSE '[]'::jsonb END) l WHERE substring(lower(l->>'proxyPass') from '^http://([^/?#]+)')=lower($1) OR ($2::boolean AND l->>'upstreamId'=$3))",&[&target,&(operation==Mutation::Delete),&id]).await.map_err(|_|db_unavailable())?.get(0);
            if used {return Err(ApiError::new(StatusCode::CONFLICT,"conflict","이 upstream을 사용하는 location이 있습니다. 해당 경로를 먼저 변경한 뒤 이름 변경·삭제를 다시 시도하세요."));}
        }
    }
    let result = match operation {
        Mutation::Delete => {
            let sql = format!("DELETE FROM {} WHERE id=$1", kind.table());
            tx.execute(&sql, &[&id]).await.map_err(sql_error)?;
            json!({"deleted":true})
        }
        Mutation::Toggle => {
            if kind == Resource::Upstream {
                return Err(ApiError::not_found());
            }
            if let Some(row) = before.as_ref().filter(|r| r["enabled"] != true) {
                let dynamic = if kind == Resource::Server {
                    &row["config"]
                } else {
                    row
                };
                if dynamic["resolveAtRuntime"] == true
                    || dynamic["locations"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|l| l["resolveAtRuntime"] == true)
                {
                    crate::dns_guard::require_runtime_dns(&state.config)
                        .map_err(|m| field_error("resolveAtRuntime", m))?;
                }
            }
            let sql = format!(
                "UPDATE {} SET enabled=NOT enabled WHERE id=$1 RETURNING enabled",
                kind.table()
            );
            let r = tx.query_one(&sql, &[&id]).await.map_err(sql_error)?;
            json!({"enabled":r.get::<_,bool>(0)})
        }
        Mutation::Create | Mutation::Update => {
            let mut resolved_upstreams=vec![];
            let built = match kind {
                Resource::Upstream => parse_upstream_unbounded(&raw)?,
                Resource::Stream => parse_stream_unbounded(&raw, &reserved_ports(&*tx, state).await?)?,
                Resource::Server => {
                    let reserved = reserved_ports_except(&*tx, state, Some(&id)).await?;
                    let mut b = parse_server_for_save(raw.clone(), reserved, valid::assert_valid_server_config, before.clone()).await?;
                    resolved_upstreams=resolve_server(&*tx, &mut b).await?;
                    b
                }
            };
            if operation==Mutation::Update && gateway_version.and_then(crate::auth::node_version::parse_semver)
                .is_some_and(|version| version.major==1 && version.minor==19) {
                check_legacy_gateway_replacement(kind, before.as_ref().unwrap(), &built)?;
            }
            check_save_sizes(kind, &built, before.as_ref())?;
            check_listener_conflicts(&*tx,kind,&built,&id,before.as_ref()).await?;
            let dynamic = if kind == Resource::Server {
                &built["config"]
            } else {
                &built
            };
            let needs_dns = dynamic["resolveAtRuntime"] == true
                || dynamic["locations"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|l| l["resolveAtRuntime"] == true);
            if needs_dns {
                crate::dns_guard::require_runtime_dns(&state.config)
                    .map_err(|m| field_error("resolveAtRuntime", m))?;
            }
            if kind == Resource::Upstream {
                let name = built["name"].as_str().unwrap();
                let unchanged = before.as_ref().and_then(|v| v["name"].as_str()) == Some(name);
                if name.to_ascii_lowercase().starts_with("nwui_resolve_") && !unchanged {
                    return Err(field_error(
                        "name",
                        "nwui_resolve_ 로 시작하는 이름은 실행 중 이름 확인용으로 예약되어 있습니다.",
                    ));
                }
            }
            let checked_config = state.config.clone();
            let checked_built = built.clone();
            let consumers = if kind == Resource::Upstream && state.config.in_container {
                list(&*tx, Resource::Server).await?
            } else { vec![] };
            let checked_before = before.clone();
            let checked_id = id.clone();
            super::internal_targets::blocking(move || {
                super::internal_targets::check_built_with_upstreams(&checked_config, kind, &checked_built,&resolved_upstreams)?;
                if kind == Resource::Upstream {
                    let mut candidate = checked_built;
                    candidate["id"] = json!(checked_id);
                    check_new_upstream_loops(&checked_config, &candidate, checked_before.as_ref(), &consumers)?;
                }
                Ok::<_, ApiError>(())
            }).await??;
            let create = operation == Mutation::Create;
            match kind {
                Resource::Upstream => {
                    let name = built["name"].as_str().unwrap();
                    let servers = &built["servers"];
                    let runtime_dns = built["resolveAtRuntime"].as_bool().unwrap_or(false);
                    let sql = if create {
                        "INSERT INTO upstreams (id,name,servers,resolve_at_runtime) VALUES ($1,$2,$3,$4)"
                    } else {
                        "UPDATE upstreams SET name=$2,servers=$3,resolve_at_runtime=$4 WHERE id=$1 AND ($5::bigint IS NULL OR row_version=$5)"
                    };
                    let mut parameters: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&id, &name, servers, &runtime_dns];
                    if !create { parameters.push(&version); }
                    if tx.execute(sql, &parameters).await.map_err(sql_error)? == 0 { return Err(edit_conflict()); }
                }
                Resource::Stream => {
                    let port = built["listen"].as_i64().unwrap() as i32;
                    let protocol = built["protocol"].as_str().unwrap();
                    let target = built["proxyPass"].as_str().unwrap();
                    let runtime_dns = built["resolveAtRuntime"].as_bool().unwrap_or(false);
                    let sql = if create {
                        "INSERT INTO nginx_streams (id,listen,protocol,proxy_pass,resolve_at_runtime) VALUES ($1,$2,$3::text::protocol,$4,$5)"
                    } else {
                        "UPDATE nginx_streams SET listen=$2,protocol=$3::text::protocol,proxy_pass=$4,resolve_at_runtime=$5 WHERE id=$1 AND ($6::bigint IS NULL OR row_version=$6)"
                    };
                    let mut parameters: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&id, &port, &protocol, &target, &runtime_dns];
                    if !create { parameters.push(&version); }
                    if tx.execute(sql, &parameters).await.map_err(sql_error)? == 0 { return Err(edit_conflict()); }
                }
                Resource::Server => {
                    let name = built["serverName"].as_str().unwrap();
                    let port = built["listen"].as_i64().unwrap() as i32;
                    let upstream = built["upstreamId"].as_str();
                    let cert = built["certificateId"].as_str();
                    let cfg = &built["config"];
                    let sql = if create {
                        "INSERT INTO nginx_servers (id,server_name,listen,upstream_id,certificate_id,config) VALUES ($1,$2,$3,$4,$5,$6)"
                    } else {
                        "UPDATE nginx_servers SET server_name=$2,listen=$3,upstream_id=$4,certificate_id=$5,config=$6 WHERE id=$1 AND ($7::bigint IS NULL OR row_version=$7)"
                    };
                    let mut parameters: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&id, &name, &port, &upstream, &cert, cfg];
                    if !create { parameters.push(&version); }
                    if tx.execute(sql, &parameters).await.map_err(sql_error)? == 0 { return Err(edit_conflict()); }
                }
            }
            get(&*tx, kind, &id).await?
        }
    };
    if operation != Mutation::Delete {
        let upstreams = list(&*tx, Resource::Upstream).await?;
        let servers = list(&*tx, Resource::Server).await?;
        super::render::validate_http_group_names(&upstreams, &servers).map_err(|m| {
            field_error(
                if kind == Resource::Upstream {
                    "name"
                } else {
                    "resolveAtRuntime"
                },
                m,
            )
        })?;
    }
    let prefix = match kind {
        Resource::Upstream => "reverseproxy:upstream",
        Resource::Server => "reverseproxy",
        Resource::Stream => "stream",
    };
    let verb = match operation {
        Mutation::Create => "create",
        Mutation::Update => "update",
        Mutation::Delete => "delete",
        Mutation::Toggle => "toggle",
    };
    let before = if operation == Mutation::Toggle {
        before.map(|b| json!({"enabled":b["enabled"]}))
    } else {
        before
    };
    audit::write(
        &*tx,
        &AuditEntry {
            actor_id: actor.id.clone(),
            actor_label: actor.label.clone(),
            action: format!("{prefix}:{verb}"),
            target: Some(id),
            before,
            after: (operation != Mutation::Delete).then(|| result.clone()),
            ip: actor.ip.clone(),
        },
    )
    .await
    .map_err(|_| ApiError::internal())?;
    tx.commit().await.map_err(|_| ApiError::internal())?;
    Ok(result)
}

fn editor_directives(raw: &Value, context: valid::DirectiveContext) -> super::Result<Value> {
    let mut dirs = Vec::new();
    for h in raw["headers"].as_array().into_iter().flatten() {
        let name = trimmed(&h["name"]);
        let value = h["value"].as_str().unwrap_or("");
        if !name.is_empty() || !value.trim().is_empty() {
            dirs.push(json!({"name":"proxy_set_header","args":[name,value]}));
        }
    }
    for (field, name) in [
        ("read", "proxy_read_timeout"),
        ("send", "proxy_send_timeout"),
        ("connect", "proxy_connect_timeout"),
    ] {
        let v = trimmed(&raw["timeouts"][field]);
        if !v.is_empty() {
            dirs.push(json!({"name":name,"args":[v]}));
        }
    }
    let body = trimmed(&raw["bodySize"]);
    if !body.is_empty() {
        dirs.push(json!({"name":"client_max_body_size","args":[body]}));
    }
    for h in raw["addHeaders"].as_array().into_iter().flatten() {
        let name = trimmed(&h["name"]);
        let value = h["value"].as_str().unwrap_or("");
        if !name.is_empty() || !value.trim().is_empty() {
            dirs.push(json!({"name":"add_header","args":[name,value]}));
        }
    }
    for r in raw["rewrites"].as_array().into_iter().flatten() {
        let from = trimmed(&r["from"]);
        let to = trimmed(&r["to"]);
        if from.is_empty() && to.is_empty() {
            continue;
        }
        let mut args = vec![from, to];
        let flag = trimmed(&r["flag"]);
        if !flag.is_empty() {
            args.push(flag);
        }
        dirs.push(json!({"name":"rewrite","args":args}));
    }
    for (field, name) in [
        ("httpVersion", "proxy_http_version"),
        ("buffering", "proxy_buffering"),
    ] {
        let v = trimmed(&raw[field]);
        if !v.is_empty() {
            dirs.push(json!({"name":name,"args":[v]}));
        }
    }
    let advanced = valid::parse_freeform_directives(raw["advanced"].as_str().unwrap_or(""), context)?;
    for d in &advanced {
        if dirs.iter().any(|e| e["name"] == d.name && (!matches!(d.name.as_str(), "proxy_set_header"|"add_header"|"rewrite"|"proxy_hide_header"|"proxy_pass_header") || e["args"][0].as_str().zip(d.args.first()).is_some_and(|(a,b)| a.eq_ignore_ascii_case(b)))) {
            return Err(format!("편집기와 고급 입력에 같은 디렉티브가 있습니다: {}",d.name).into());
        }
    }
    dirs.extend(advanced.into_iter().map(|d| json!(d)));
    directives(&json!(dirs), context)
}
fn form_runtime_dns(form: &FormInput) -> Result<bool, ApiError> {
    match form.get("resolveAtRuntime") {
        None | Some("" | "false") => Ok(false),
        Some("true") => Ok(true),
        _ => Err(field_error(
            "resolveAtRuntime",
            "실행 중 이름 확인 값은 true 또는 false여야 합니다.",
        )),
    }
}

fn form_directives(raw:Option<&Value>,context:valid::DirectiveContext)->super::Result<Value>{
    match raw{Some(Value::Null)=>Ok(Value::Null),Some(value)if value.is_array()=>directives(value,context),Some(value)if value.is_object()=>editor_directives(value,context),None=>editor_directives(&Value::Null,context),_=>Err("디렉티브 형식이 올바르지 않습니다.".into())}
}

pub fn form_body(kind: Resource, form: &FormInput) -> Result<Value, ApiError> {
    for (field,values) in &form.fields {
        if field.contains('\0') || values.iter().any(|value|value.contains('\0')) {
            return Err(field_error(field,"NUL 문자는 사용할 수 없습니다."));
        }
    }
    let get = |k| form.get(k).unwrap_or("");
    let body: Result<Value, ApiError> = match kind {
        Resource::Upstream => Ok(
            json!({"name":get("name"),"servers":get("servers").lines().collect::<Vec<_>>(),"resolveAtRuntime":form_runtime_dns(form)?}),
        ),
        Resource::Stream => Ok(
            json!({"listen":get("listen"),"protocol":get("protocol").trim(),"proxyPass":get("proxyPass"),"resolveAtRuntime":form_runtime_dns(form)?}),
        ),
        Resource::Server => {
            let mut raw = json!({"serverName":get("serverName"),"listen":get("listen"),"targetMode":get("targetMode"),"upstreamId":null,"proxyPass":null,"certificateId":get("certificateId"),"resolveAtRuntime":form_runtime_dns(form)?});
            for (k,message) in [
                ("ipAccess","IP 접근 제어 형식이 올바르지 않습니다."),
                ("realIp","실 IP 설정 형식이 올바르지 않습니다."),
                ("tlsOptions","TLS 설정 형식이 올바르지 않습니다."),
                ("cache","캐시 설정 형식이 올바르지 않습니다."),
                ("gzip","gzip 설정 형식이 올바르지 않습니다."),
            ] {
                raw[k]=if get(k).trim().is_empty() { Value::Null } else {
                    serde_json::from_str(get(k)).map_err(|_|field_error(k,message))?
                };
            }
            for (k, ctx) in [
                ("serverDirectives", valid::DirectiveContext::Server),
                ("locationDirectives", valid::DirectiveContext::Location),
            ] {
                if k == "locationDirectives" && raw["targetMode"] == "locations" {
                    continue;
                }
                let parsed = if get(k).trim().is_empty() {None} else {Some(serde_json::from_str(get(k)).map_err(|_| field_error(k, "디렉티브 데이터 형식이 올바르지 않습니다."))?)};
                raw[k] = form_directives(parsed.as_ref(),ctx).map_err(|e| field_error(k,e))?;
            }
            if raw["targetMode"] == "locations" {
                let mut locations: Value =
                    if get("locations").trim().is_empty() {Value::Null} else {serde_json::from_str(get("locations")).map_err(|_|field_error("locations","location 설정 형식이 올바르지 않습니다."))?};
                if let Some(list) = locations.as_array_mut() {
                    for (i, l) in list.iter_mut().enumerate() {
                        if !l.is_object() {
                            continue;
                        }
                        l["directives"] =
                            form_directives(l.get("directives"), valid::DirectiveContext::Location)
                                .map_err(|e| {
                                    field_error(
                                        "locations",
                                        format!(
                                            "location {} ({}): {e}",
                                            i + 1,
                                            trimmed(&l["path"])
                                        ),
                                    )
                                })?;
                    }
                }
                raw["locations"] = locations;
            } else if get("mode") == "direct" {
                if get("proxyPass").trim().is_empty() {
                    return Err(field_error("proxyPass", "proxy_pass 대상을 입력하세요."));
                }
                raw["proxyPass"] = json!(get("proxyPass"));
            } else {
                raw["upstreamId"] = json!(get("upstreamId"));
            }
            let redirect = match form.get("redirectHttp") {
                None | Some("true") => true,
                Some("false") => false,
                _ => return Err(field_error("redirectHttp", "HTTP 전환 설정은 true 또는 false여야 합니다.")),
            };
            raw["tls"] = json!({"tlsPort":form.get("tlsPort").unwrap_or("443"),"redirectHttp":redirect,"tlsOptions":raw["tlsOptions"]});
            Ok(raw)
        }
    };
    let mut body = body?;
    if let Some(version) = form.get("version") { body["version"] = json!(version); }
    object(&body)?;
    Ok(body)
}

#[cfg(test)]
mod review_executor_tests {
    use super::*;
    #[tokio::test(flavor="current_thread")]
    async fn audit_e6_r1_server_save_parse_keeps_the_executor_responsive() {
        use std::sync::{Arc,Mutex,atomic::{AtomicBool,AtomicUsize,Ordering}};
        let progressed=Arc::new(AtomicBool::new(false));let seen=progressed.clone();
        let (started,ready)=tokio::sync::oneshot::channel();
        let observer=tokio::spawn(async move {ready.await.unwrap();seen.store(true,Ordering::SeqCst);});
        let started=Mutex::new(Some(started));let calls=Arc::new(AtomicUsize::new(0));let counted=calls.clone();
        let raw=json!({"serverName":"save.test","listen":19090,"targetMode":"locations","locations":(0..3).map(|i|json!({"path":format!("/{i}"),"mode":"direct","proxyPass":"http://slow.test:8080"})).collect::<Vec<_>>()});
        let parsed=parse_server_for_save(raw,ReservedPorts::default(),move |cfg| {
            valid::validate_server_config_with(cfg,&|host,port| {
                assert_eq!((host,port),("slow.test",8080));counted.fetch_add(1,Ordering::SeqCst);
                if let Some(tx)=started.lock().unwrap().take(){let _=tx.send(());}
                std::thread::sleep(std::time::Duration::from_millis(100));
                vec!["127.0.0.1".parse().unwrap()]
            }).map(|_| ())
        },None).await.unwrap();
        let responsive=progressed.load(Ordering::SeqCst);observer.await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst),3);assert_eq!(parsed["config"]["locations"].as_array().unwrap().len(),3);
        assert!(responsive,"server save parsing must not park the async executor during per-location DNS checks");
        let forged=json!({"serverName":"save.test","listen":19090,"proxyPass":"http://127.0.0.1:8080","locationDirectives":[{"name":"proxy_set_header","args":["X-Real-IP","forged"]}]});
        let denied=parse_server_for_save(forged,ReservedPorts::default(),valid::assert_valid_server_config,None).await.unwrap_err();
        assert_eq!(denied.status,StatusCode::BAD_REQUEST);
        assert!(denied.field_errors.unwrap()["locationDirectives"].contains("X-Real-IP"));
    }
}

#[cfg(test)]
mod conversion_equality_tests;


#[cfg(test)]
mod q23_tests {
    use super::*;
    #[test]
    fn q23_server_and_member_save_paths_preserve_legacy_unix() {
        let reserved=ReservedPorts::default();
        for target in ["http://unix:/var/run/app.sock:/api/","http://unix:/tmp/x.sock"] {
            let raw=json!({"serverName":"socket.test","listen":19080,"proxyPass":target});
            let row=parse_server(&raw,&reserved).unwrap();
            assert_eq!(super::super::render::effective_locations(&row["config"])[0]["proxyPass"],target);
        }
        assert!(parse_upstream(&json!({"name":"socket","servers":["unix:8080"]})).is_ok());
        assert!(parse_stream(&json!({"listen":19081,"protocol":"tcp","proxyPass":"unix:8080"}),&reserved).is_ok());
    }
    #[test]
    fn q23_runtime_unix_diagnostic_reaches_every_parser() {
        let reserved=ReservedPorts::default();let message="unix 소켓 대상에는 실행 중 이름 확인을 쓸 수 없습니다.";
        for raw in [json!({"serverName":"socket.test","listen":19080,"proxyPass":"http://unix:/tmp/x.sock","resolveAtRuntime":true}),json!({"serverName":"socket.test","listen":19080,"targetMode":"locations","locations":[{"path":"/","mode":"direct","proxyPass":"http://unix:/tmp/x.sock","resolveAtRuntime":true}]})] {
            let e=parse_server(&raw,&reserved).unwrap_err();assert!(e.field_errors.unwrap().values().any(|s|s.contains(message)));
        }
        let e=parse_upstream(&json!({"name":"socket","servers":["unix:8080"],"resolveAtRuntime":true})).unwrap_err();assert!(e.field_errors.unwrap()["servers"].contains(message));
        let e=parse_stream(&json!({"listen":19081,"protocol":"tcp","proxyPass":"unix:8080","resolveAtRuntime":true}),&reserved).unwrap_err();assert!(e.field_errors.unwrap()["proxyPass"].contains(message));
    }
    #[test]
    fn q23_unchanged_internal_socket_can_save_unrelated_field_but_new_target_cannot() {
        let target="http://unix:/run/nwui-publisher/control.sock:/";
        let before=json!({"config":{"proxyPass":target}});
        let raw=json!({"serverName":"edited.test","listen":19080,"proxyPass":target});
        assert!(parse_server_with(&raw,&ReservedPorts::default(),&valid::assert_valid_server_config,Some(&before)).is_ok());
        let e=parse_server(&raw,&ReservedPorts::default()).unwrap_err();assert!(e.field_errors.unwrap()["proxyPass"].contains("이 컨테이너 내부 소켓은 프록시 대상으로 쓸 수 없습니다."));
    }
}

#[cfg(test)]
mod stored_admin_header_save_tests {
    use super::*;
    #[tokio::test]
    async fn json_and_form_create_and_noop_edit_accept_only_default_admin_headers() {
        const MESSAGE: &str = "관리 화면 프록시의 X-Real-IP·X-Forwarded-Proto 헤더는 자동 설정되며 변경할 수 없습니다.";
        let headers = json!([
            {"name":"proxy_set_header","args":["x-real-ip","$remote_addr"]},
            {"name":"proxy_set_header","args":["X-Forwarded-Proto","$scheme"]},
            {"name":"proxy_set_header","args":["Host","$host"]},
            {"name":"proxy_set_header","args":["X-Forwarded-For","$proxy_add_x_forwarded_for"]}
        ]);
        for explicit in [false, true] {
            for form_editor in [None, Some(false), Some(true)] {
                let mut raw = json!({"serverName":"admin.test","listen":18086,"proxyPass":"http://127.0.0.1:8080","locationDirectives":headers});
                if explicit {
                    raw["targetMode"] = json!("locations");
                    raw["locations"] = json!([{"path":"/","mode":"direct","proxyPass":"http://127.0.0.1:8080","directives":headers}]);
                }
                if let Some(editor) = form_editor {
                    let dirs = if editor {
                        json!({"headers":[{"name":"x-real-ip","value":"$remote_addr"},{"name":"Host","value":"$host"}],
                            "advanced":"proxy_set_header X-Forwarded-Proto $scheme;\nproxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;"})
                    } else { headers.clone() };
                    let mut form = FormInput::default();
                    for (key, value) in [("serverName","admin.test"),("listen","18086"),("mode","direct"),("proxyPass","http://127.0.0.1:8080")] {
                        form.fields.insert(key.into(), vec![value.into()]);
                    }
                    if explicit {
                        form.fields.insert("targetMode".into(), vec!["locations".into()]);
                        let locations = json!([{"path":"/","mode":"direct","proxyPass":"http://127.0.0.1:8080","directives":dirs}]);
                        form.fields.insert("locations".into(), vec![locations.to_string()]);
                    } else {form.fields.insert("locationDirectives".into(), vec![dirs.to_string()]);}
                    raw = form_body(Resource::Server, &form).unwrap();
                }
                let before = json!({"config":if explicit {json!({"locations":raw["locations"]})} else {json!({"proxyPass":raw["proxyPass"],"locationDirectives":raw["locationDirectives"]})}});
                let original = before.clone();
                let expected = if explicit {&raw["locations"][0]["directives"]} else {&raw["locationDirectives"]};
                for previous in [None, Some(before.clone())] {
                    let saved = parse_server_for_save(raw.clone(), ReservedPorts::default(), valid::assert_valid_server_config, previous.clone()).await.unwrap();
                    assert_eq!(&saved["config"]["locations"][0]["directives"], expected);
                    for (name, value) in [("X-Real-IP", "spoof"), ("X-Forwarded-Proto", "https"), ("x-real-ip", "${remote_addr}"), ("X-Forwarded-Proto", "$Scheme")] {
                        let mut custom = raw.clone();
                        let dirs = if explicit {&mut custom["locations"][0]["directives"]} else {&mut custom["locationDirectives"]};
                        dirs.as_array_mut().unwrap().push(json!({"name":"proxy_set_header","args":[name,value]}));
                        let err = parse_server_for_save(custom, ReservedPorts::default(), valid::assert_valid_server_config, previous.clone()).await.unwrap_err();
                        assert_eq!(err.status, StatusCode::BAD_REQUEST);
                        assert_eq!(err.field_errors.unwrap()[if explicit {"locations"} else {"locationDirectives"}], MESSAGE);
                    }
                }
                assert_eq!(before, original);
            }
        }
    }
}
