//! Shared row renderer; external observations are supplied by the caller.
use super::{model::{CertificateRef, ConfigSnapshotData, RenderOutcome, SkippedKind, SkippedRecord}, render, validation, resources::Resource};
use crate::config::RuntimeConfig;
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct RenderContext<'a> {
    pub runtime_dns: &'a crate::dns_guard::Descriptor,
    pub resolve: super::internal_targets::Resolve<'a>,
    pub is_local_addr: super::internal_targets::LocalAddr<'a>,
}

fn text<'a>(row: &'a Value, key: &str) -> Result<&'a str> {
    row[key]
        .as_str()
        .with_context(|| format!("Invalid configuration field {key}"))
}
fn port(row: &Value, key: &str) -> Result<f64> {
    row[key]
        .as_f64()
        .with_context(|| format!("Invalid configuration field {key}"))
}

fn validate_directives(raw: &Value, context: validation::DirectiveContext) -> Result<()> {
    if raw.is_null() {
        return Ok(());
    }
    let directives: Vec<super::Directive> =
        serde_json::from_value(raw.clone()).context("Invalid directives")?;
    for d in directives {
        validation::assert_valid_directive(&d.name, &d.args, context)?;
    }
    Ok(())
}
fn validate_access(raw: &Value) -> Result<()> {
    if raw.is_null() {
        return Ok(());
    }
    ensure!(
        matches!(raw["mode"].as_str(), Some("whitelist" | "blacklist")),
        "Invalid IP access mode"
    );
    for entry in raw["entries"]
        .as_array()
        .context("Invalid IP access entries")?
    {
        validation::assert_valid_ip_or_cidr(entry.as_str().context("Invalid IP access entry")?)?;
    }
    Ok(())
}

/// Build all strings before any disk mutation. Snapshot rows retain their former
/// camelCase JSON shape so existing history and pending-change clients can read them.
///
/// Every enabled row is validated again here, but a row that fails is left out and
/// reported instead of abandoning the whole render. An upgrade recreates the container
/// with an empty conf.d, so an all-or-nothing render there leaves every domain answering
/// 404 because of one legacy row. Reverse proxies claim their ports before streams, so an
/// HTTP/TCP collision between two enabled rows keeps the reverse proxy.
pub(crate) fn render_with_context(
    config: &RuntimeConfig,
    data: &ConfigSnapshotData,
    certificates: &[CertificateRef],
    context: &RenderContext<'_>,
) -> RenderOutcome {
    let mut outcome = RenderOutcome::default();
    let mut reserved = super::model::ports_for_snapshot_with_dns(config, &ConfigSnapshotData::default(), context.runtime_dns);
    let mut upstreams = Upstreams::default();
    let mut upstream_content = Vec::new();
    for row in &data.upstreams {
        match render_upstream_row(config, row, context) {
            Ok((id, name, content, all_admin, mixed_admin)) => {
                upstream_content.push(content);
                if all_admin {upstreams.admin.insert(name.to_ascii_lowercase());}
                else if mixed_admin {outcome.notices.push(format!("upstream {name} 에 관리 화면과 다른 대상이 섞여 있습니다. 관리 화면에 전달되는 클라이언트 IP·HTTPS 정보는 신뢰하지 않습니다. 관리 화면 대상을 별도 upstream으로 분리하세요."));}
                upstreams.rendered.insert(id, name);
            }
            Err(e) => {
                let skipped = skip(SkippedKind::Upstream, row, row_label(row, "name"), e);
                upstreams
                    .skipped
                    .insert(skipped.id.clone(), skipped.label.clone());
                outcome.skipped.push(skipped);
            }
        }
    }
    outcome.rendered.conf.insert(
        "upstreams.conf".into(),
        upstream_content.join("\n").into_bytes(),
    );
    for row in data.servers.iter().filter(|r| r["enabled"] == true) {
        let rendered = super::internal_targets::check_listener_targets_with(config, Resource::Server, row, &data.upstreams, context.resolve, context.is_local_addr)
            .map_err(anyhow::Error::msg)
            .and_then(|_| render_server_row(config, row, &reserved, &upstreams, certificates, context));
        match rendered {
            Ok((file, bytes, ports)) => {
                outcome.rendered.conf.insert(file, bytes);
                let name = row_label(row, "serverName");
                reserved.http_plain.push(ports[0]);
                reserved.http_tls.extend(ports.iter().skip(1).copied());
                for p in ports {
                    reserved.http.push(p);
                    reserved
                        .owners
                        .entry(p)
                        .or_insert_with(|| format!("리버스 프록시 {name}"));
                }
            }
            Err(e) => outcome.skipped.push(skip(
                SkippedKind::Server,
                row,
                row_label(row, "serverName"),
                e,
            )),
        }
    }
    for row in data.streams.iter().filter(|r| r["enabled"] == true) {
        let rendered = super::internal_targets::check_listener_targets_with(config, Resource::Stream, row, &[], context.resolve, context.is_local_addr)
            .map_err(anyhow::Error::msg)
            .and_then(|_| render_stream_row(config, row, &reserved, context));
        match rendered {
            Ok((file, bytes)) => {
                outcome.rendered.stream.insert(file, bytes);
            }
            Err(e) => {
                let label = format!(
                    "{} :{} → {}",
                    row["protocol"].as_str().unwrap_or("?"),
                    row["listen"],
                    row["proxyPass"].as_str().unwrap_or("")
                );
                outcome
                    .skipped
                    .push(skip(SkippedKind::Stream, row, label, e));
            }
        }
    }
    outcome
}

fn skip(kind: SkippedKind, row: &Value, label: String, error: anyhow::Error) -> SkippedRecord {
    SkippedRecord {
        kind,
        id: row["id"].as_str().unwrap_or("").to_owned(),
        label,
        reason: format!("{error:#}"),
        kept_previous: false,
    }
}
pub(crate) fn row_label(row: &Value, key: &str) -> String {
    row[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("(이름 없음)")
        .to_owned()
}

/// Upstreams by id -> name, split by whether this render kept them.
#[derive(Default)]
struct Upstreams {
    rendered: BTreeMap<String, String>,
    skipped: BTreeMap<String, String>,
    admin: BTreeSet<String>,
}
impl Upstreams {
    fn is_target(&self, target: &str) -> bool {
        super::internal_targets::target_authority(target).is_some_and(|host|
            self.rendered.values().any(|name| name.eq_ignore_ascii_case(host)))
    }
    fn check_url(&self, config: &RuntimeConfig, target: &str, dynamic: bool, resolve: super::internal_targets::Resolve) -> Result<()> {
        if dynamic {
            self.check_runtime_override(target)?;
            super::internal_targets::check_dynamic_url(config, target).map_err(anyhow::Error::msg)?;
        } else if !self.is_target(target) {
            super::internal_targets::check_url_with(config, target, resolve).map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
    /// `http://<name>` is how a proxy_pass refers to an upstream. nginx resolves it to the
    /// upstream block, so it is not validated as a URL: a numeric-looking name such as `09`
    /// is a valid upstream name but not a valid WHATWG URL host.
    fn check_runtime_override(&self, target: &str) -> Result<()> {
        let authority = target
            .split_once("://")
            .map(|(_, rest)| rest.split(['/', '?']).next().unwrap_or(""));
        ensure!(
            !authority.is_some_and(|host| self
                .rendered
                .values()
                .chain(self.skipped.values())
                .any(|name| name.eq_ignore_ascii_case(host))),
            "upstream의 이름 확인 방식은 그 upstream에서 선택하세요."
        );
        Ok(())
    }
    fn check_target(&self, target: &str) -> Result<()> {
        let named = |names: &BTreeMap<String, String>| {
            names
                .values()
                .find(|name| super::internal_targets::target_authority(target).is_some_and(|host|host.eq_ignore_ascii_case(name)))
                .cloned()
        };
        if named(&self.rendered).is_some() {
            return Ok(());
        }
        if let Some(name) = named(&self.skipped) {
            bail!(upstream_skipped(&name));
        }
        Ok(validation::assert_valid_stored_proxy_target(target)?)
    }
}
fn upstream_skipped(name: &str) -> String {
    format!("이 설정이 쓰는 upstream {name} 을(를) 적용에서 제외해 함께 제외했습니다")
}

fn runtime_endpoint(context: &RenderContext<'_>) -> Option<std::net::SocketAddr> {
    context.runtime_dns.available.then_some(context.runtime_dns.endpoint)
}

fn render_upstream_row(config: &RuntimeConfig, row: &Value, context: &RenderContext<'_>) -> Result<(String, String, String, bool, bool)> {
    let id = text(row, "id")?.to_owned();
    let name = text(row, "name")?.to_owned();
    validation::assert_valid_upstream_name(&name)?;
    let mut servers = row["servers"].as_array().cloned().unwrap_or_default();
    let runtime_dns = validation::parse_runtime_dns_flag(row)?;
    for target in &servers {
        let target = target.as_str().context("Invalid upstream server")?;
        validation::assert_valid_upstream_server(target)?;
        if runtime_dns {
            super::internal_targets::check_dynamic_host_port(config, target)
        } else {
            super::internal_targets::check_host_port_with(config, target, context.resolve)
        }.map_err(anyhow::Error::msg)?;
    }
    let admin=servers.iter().filter(|target|super::self_proxy::admin_member_with(target.as_str().unwrap(),runtime_dns,context.resolve,context.is_local_addr)).count();
    let all_admin=!servers.is_empty() && admin==servers.len();
    let mixed_admin=admin>0 && !all_admin;
    if all_admin {
        let mut seen=BTreeSet::new();
        servers=servers.into_iter().filter_map(|target|{
            let target=target.as_str().unwrap();let endpoint=target.split_whitespace().next().unwrap();
            let canonical=format!("127.0.0.1:8080{}",&target[endpoint.len()..]);
            seen.insert(canonical.clone()).then(||json!(canonical))
        }).collect();
    }
    let content = render::render_upstream_with_dns(
        &json!({"id":id,"name":name,"servers":servers,"resolveAtRuntime":runtime_dns}),
        runtime_endpoint(context),
    )?;
    Ok((id, name, content, all_admin, mixed_admin))
}

/// Returns the file name, its bytes and the HTTP ports this server listens on.
fn render_server_row(
    config: &RuntimeConfig,
    row: &Value,
    reserved: &super::reserved_ports::ReservedPorts,
    upstreams: &Upstreams,
    certificates: &[CertificateRef],
    context: &RenderContext<'_>,
) -> Result<(String, Vec<u8>, Vec<u16>)> {
    let id = text(row, "id")?;
    super::model::cert_file_paths(config, id)?;
    let name = text(row, "serverName")?;
    validation::assert_valid_server_name(name)?;
    let listen = port(row, "listen")?;
    validation::assert_valid_http_port(listen, false, reserved)?;
    // Validated above as an integer 1-65535.
    let mut ports = vec![listen as u16];
    let extra = row["config"].as_object();
    let mut cfg = json!({"serverId":id,"serverName":name,"listen":row["listen"]});
    for key in [
        "proxyPass",
        "resolveAtRuntime",
        "locations",
        "serverDirectives",
        "locationDirectives",
        "ipAccess",
        "realIp",
        "cache",
        "gzip",
    ] {
        if let Some(value) = extra.and_then(|e| e.get(key)) {
            cfg[key] = value.clone();
        }
    }
    if let Some(upstream_id) = row["upstreamId"].as_str() {
        if let Some(name) = upstreams.skipped.get(upstream_id) {
            bail!(upstream_skipped(name));
        }
        if let Some(name) = upstreams.rendered.get(upstream_id) {
            cfg["upstreamName"] = json!(name);
        }
    }
    for location in cfg["locations"].as_array().into_iter().flatten(){
        if let Some(name)=location["upstreamId"].as_str().and_then(|id|upstreams.skipped.get(id)){bail!(upstream_skipped(name));}
    }
    cfg=super::location_refs::resolve_locations(&cfg,&upstreams.rendered)?;
    for name in render::generated_http_group_names(&cfg)? {
        ensure!(
            !upstreams
                .rendered
                .values()
                .any(|n| n.eq_ignore_ascii_case(&name)),
            "생성된 upstream 이름 {name} 이 기존 그룹과 충돌합니다."
        );
    }
    validate_directives(
        &cfg["serverDirectives"],
        validation::DirectiveContext::Server,
    )?;
    validate_directives(
        &cfg["locationDirectives"],
        validation::DirectiveContext::Location,
    )?;
    validate_access(&cfg["ipAccess"])?;
    if let Some(ip) = validation::parse_stored_real_ip_config(&cfg["realIp"])? {
        cfg["realIp"] = ip;
    } else {
        cfg.as_object_mut().unwrap().remove("realIp");
    }
    if let Some(locations) = cfg["locations"].as_array().filter(|a| !a.is_empty()) {
        for location in locations {
            validation::assert_valid_location_path(text(location, "path")?)?;
            let target = text(location, "proxyPass")?;
            upstreams.check_target(target)?;
            let runtime=validation::parse_runtime_dns_flag(location)?;
            if location["upstreamId"].as_str().is_some(){
                ensure!(!runtime,"upstream의 이름 확인 방식은 그 upstream에서 선택하세요.");
            }else{
            upstreams.check_url(config, target, runtime, context.resolve)?;
            }
            validate_directives(
                &location["directives"],
                validation::DirectiveContext::Location,
            )?;
            validate_access(&location["ipAccess"])?;
        }
    } else if cfg.get("upstreamName").is_none() {
        let target = cfg["proxyPass"].as_str().unwrap_or("");
        upstreams.check_target(target)?;
        upstreams.check_url(config, target, validation::parse_runtime_dns_flag(&cfg)?, context.resolve)?;
    }
    if let Some(certificate_id) = row["certificateId"].as_str() {
        let cert = certificates.iter().find(|cert| cert.id == certificate_id)
            .filter(|cert| cert.status == "issued" && cert.usable)
            .with_context(|| format!("인증서 {certificate_id} 를 사용할 수 없습니다. 발급 상태와 인증서·개인키를 확인하세요."))?;
        let tls=extra.and_then(|e|e.get("tlsPort")).map(|p|p.as_f64().context("Invalid TLS port")).transpose()?.unwrap_or(443.0);
        validation::assert_valid_http_port(tls,true,reserved)?;
        ensure!(tls!=listen,"HTTP·HTTPS 포트는 서로 달라야 합니다.");
        let (cert_path, key_path) = super::model::cert_file_paths(config, &cert.id)?;
        let tls_port = extra
            .and_then(|e| e.get("tlsPort"))
            .filter(|v| v.is_number())
            .cloned()
            .unwrap_or(json!(443));
        let tls = tls_port.as_f64().context("Invalid TLS port")?;
        ports.push(tls as u16);
        cfg["tls"] = json!({"certPath":cert_path.to_string_lossy(),"keyPath":key_path.to_string_lossy(),"tlsPort":tls_port,"redirectHttp":extra.and_then(|e|e.get("redirectHttp"))!=Some(&json!(false)),"httpPort":row["listen"],"tuning":extra.and_then(|e|e.get("tlsOptions"))});
    }
    Ok((
        format!("server-{id}.conf"),
        render::render_reverse_proxy_context(&cfg, runtime_endpoint(context), context.resolve, &upstreams.admin, context.is_local_addr)?.into_bytes(),
        ports,
    ))
}

fn render_stream_row(
    config: &RuntimeConfig,
    row: &Value,
    reserved: &super::reserved_ports::ReservedPorts,
    context: &RenderContext<'_>,
) -> Result<(String, Vec<u8>)> {
    let id = text(row, "id")?;
    super::model::cert_file_paths(config, id)?;
    let protocol = text(row, "protocol")?;
    ensure!(matches!(protocol, "tcp" | "udp"), "Invalid stream protocol");
    let listen_context = if protocol == "udp" {
        super::reserved_ports::ListenContext::StreamUdp
    } else {
        super::reserved_ports::ListenContext::StreamTcp
    };
    validation::assert_valid_listen_port(port(row, "listen")?, listen_context, reserved)?;
    let target = text(row, "proxyPass")?;
    validation::assert_valid_stream_proxy_pass(target)?;
    if validation::parse_runtime_dns_flag(row)? {
        super::internal_targets::check_dynamic_host_port(config, target)
    } else {
        super::internal_targets::check_host_port_with(config, target, context.resolve)
    }.map_err(anyhow::Error::msg)?;
    Ok((
        format!("stream-{id}.conf"),
        render::render_stream_with_dns(row, runtime_endpoint(context))?.into_bytes(),
    ))
}


#[cfg(test)]
mod locality_context_tests {
    use super::*;

    #[test]
    fn injected_probe_preserves_literal_classification_and_owns_interface_observation() {
        let config = RuntimeConfig::read(|key| match key {
            "DATABASE_URL" => Some("postgres://fixture@127.0.0.1/fixture".into()),
            "AUTH_SECRET" => Some("local-probe-fixture".into()),
            _ => None,
        }).unwrap();
        let dns = crate::dns_guard::Descriptor::unavailable(&config.target_dns, "fixture");
        let never_resolve = |_:&str,_| -> Vec<std::net::IpAddr> {panic!("literal classification must not resolve")};
        for (target, expected) in [
            ("http://127.0.0.1:8080",true), ("http://127.1:8080",true),
            ("http://[::1]:8080",true), ("http://192.0.2.1:8080",false),
        ] {
            let calls = std::cell::Cell::new(0);
            let probe = |ip| {calls.set(calls.get()+1); super::super::internal_targets::is_local_addr(ip)};
            let data = ConfigSnapshotData {servers:vec![json!({"id":"literal","serverName":"literal.test","listen":18086,"enabled":true,"config":{"proxyPass":target}})],..Default::default()};
            let outcome = render_with_context(&config,&data,&[],&RenderContext {runtime_dns:&dns,resolve:&never_resolve,is_local_addr:&probe});
            assert!(outcome.skipped.is_empty(),"{:?}",outcome.skipped);
            let text = String::from_utf8(outcome.rendered.conf["server-literal.conf"].clone()).unwrap();
            assert_eq!(text.contains("proxy_bind 127.0.0.2;"), expected, "{target}");
            assert!(calls.get()>0,"the caller owns the probe");
        }
        // An assigned-address answer comes exclusively from the injected observation.
        let data = ConfigSnapshotData {servers:vec![json!({"id":"assigned","serverName":"assigned.test","listen":18086,"enabled":true,"config":{"proxyPass":"http://192.0.2.1:8080"}})],..Default::default()};
        let outcome = render_with_context(&config,&data,&[],&RenderContext {
            runtime_dns:&dns,resolve:&never_resolve,is_local_addr:&|ip|ip=="192.0.2.1".parse::<std::net::IpAddr>().unwrap(),
        });
        assert!(outcome.skipped.is_empty());
        assert!(String::from_utf8_lossy(&outcome.rendered.conf["server-assigned.conf"]).contains("proxy_bind 127.0.0.2;"));
    }
}
