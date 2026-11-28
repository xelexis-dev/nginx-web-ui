//! Read-only model types and SQL loaders, shared with the unprivileged API.
use crate::config::RuntimeConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::{BTreeMap, BTreeSet},path::PathBuf};
use super::reserved_ports::{NGINX_BASE_LISTEN_PORTS, ReservedPorts};
use anyhow::Result;

#[derive(Debug, Clone, Default)]
pub struct RenderedConfig {
    pub conf: BTreeMap<String, Vec<u8>>,
    pub stream: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoredCounts {
    pub conf: usize,
    pub stream: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub ok: bool,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rolled_back_to: Option<RestoredCounts>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkippedKind {
    Upstream,
    Server,
    Stream,
}
/// A saved record the renderer left out because it failed apply-time validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedRecord {
    pub kind: SkippedKind,
    pub id: String,
    pub label: String,
    pub reason: String,
    #[serde(default)]
    pub kept_previous: bool,
}
#[derive(Debug, Clone, Default)]
pub struct RenderOutcome {
    pub rendered: RenderedConfig,
    pub skipped: Vec<SkippedRecord>,
    pub notices: Vec<String>,
    pub unresolved_paths: BTreeMap<String, Vec<String>>,
}
impl RenderOutcome {
    pub(crate) fn summary(&self)->String {
        let mut text=skipped_summary(&self.skipped);
        for notice in &self.notices {text.push_str(&format!("\n\n{notice}"));}
        text
    }
    /// `data` without the rows this render skipped, i.e. what is actually live.
    pub fn effective(&self, data: &ConfigSnapshotData, previous: Option<&ConfigSnapshotData>) -> ConfigSnapshotData {
        let empty = ConfigSnapshotData::default();
        let previous = previous.unwrap_or(&empty);
        let live = |kind: SkippedKind, rows: &[Value], old: &[Value]| -> Vec<Value> {
            rows.iter().filter_map(|row| {
                match self.skipped.iter().find(|s| s.kind == kind && row["id"].as_str() == Some(s.id.as_str())) {
                    Some(s) if s.kept_previous => old.iter().find(|r| r["id"] == row["id"]).cloned(),
                    Some(_) => None,
                    None => Some(row.clone()),
                }
            }).collect()
        };
        ConfigSnapshotData {
            upstreams: live(SkippedKind::Upstream, &data.upstreams, &previous.upstreams),
            servers: live(SkippedKind::Server, &data.servers, &previous.servers),
            streams: live(SkippedKind::Stream, &data.streams, &previous.streams),
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigSnapshotData {
    #[serde(default)]
    pub upstreams: Vec<Value>,
    #[serde(default)]
    pub servers: Vec<Value>,
    #[serde(default)]
    pub streams: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackKind {
    NotFound,
    DbRestoreFailed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackResult {
    #[serde(flatten)]
    pub result: ApplyResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<RollbackKind>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardReport {
    pub reserved_files: usize,
    /// Server and stream files moved out for an internal target.
    pub internal_target_files: usize,
    /// Upstream blocks cut from `upstreams.conf` (the rest of it stays live).
    pub removed_upstreams: Vec<String>,
    pub invalid_files: usize,
    pub rejected_files: Vec<String>,
    pub rejected_upstreams: Vec<String>,
    pub quarantine_dirs: Vec<PathBuf>,
    pub validation_output: String,
}

pub async fn load_snapshot(
    client: &impl deadpool_postgres::GenericClient,
) -> Result<ConfigSnapshotData> {
    let upstreams=client.query("SELECT jsonb_build_object('id',id,'name',name,'servers',servers,'resolveAtRuntime',resolve_at_runtime) FROM upstreams ORDER BY id",&[]).await?.into_iter().map(|r|r.get(0)).collect();
    let servers=client.query("SELECT jsonb_build_object('id',id,'serverName',server_name,'listen',listen,'upstreamId',upstream_id,'config',config,'enabled',enabled,'certificateId',certificate_id) FROM nginx_servers ORDER BY id",&[]).await?.into_iter().map(|r|r.get(0)).collect();
    let streams=client.query("SELECT jsonb_build_object('id',id,'listen',listen,'protocol',protocol,'proxyPass',proxy_pass,'enabled',enabled,'resolveAtRuntime',resolve_at_runtime) FROM nginx_streams ORDER BY id",&[]).await?.into_iter().map(|r|r.get(0)).collect();
    Ok(ConfigSnapshotData {
        upstreams,
        servers,
        streams,
    })
}
pub async fn load_applied_snapshot(client: &impl deadpool_postgres::GenericClient) -> Result<ConfigSnapshotData> {
    let raw: Option<Value> = client.query_opt("SELECT data FROM applied_config WHERE id='current'", &[]).await?.map(|r| r.get(0));
    Ok(raw.as_ref().map(super::pending::snapshot_from_value).unwrap_or_default())
}
pub fn skipped_summary(skipped: &[SkippedRecord]) -> String {
    if skipped.is_empty() {
        return String::new();
    }
    let any_kept = skipped.iter().any(|s| s.kept_previous);
    let mut text = if any_kept {
        format!("\n\n새 변경을 적용하지 못한 설정 {}개:", skipped.len())
    } else {
        format!("\n\n적용에서 제외한 설정 {}개 — 이 항목들은 지금 동작하지 않습니다:", skipped.len())
    };
    for s in skipped {
        let kind = match s.kind {
            SkippedKind::Upstream => "upstream",
            SkippedKind::Server => "리버스 프록시",
            SkippedKind::Stream => "stream",
        };
        text.push_str(&format!("\n- {kind} {}: {}", s.label, s.reason));
        if s.kept_previous { text.push_str(" — 직전 적용본을 유지합니다"); }
        else if any_kept { text.push_str(" — 이 항목은 지금 동작하지 않습니다"); }
        if s.reason.contains("host not found in upstream") {
            text.push_str("\n  설정을 적용할 때 대상 이름의 주소를 찾지 못했습니다. 이름이 맞는지 확인하세요. 대상이 늘 켜져 있지 않거나 주소가 바뀔 수 있다면 대상의 이름 확인 방식을 「실행 중에 이름 확인」으로 바꾸세요.");
        }
    }
    text
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CertificateRef { pub id: String, pub status: String, pub usable: bool }
pub async fn load_certificate_refs(client:&impl deadpool_postgres::GenericClient) -> anyhow::Result<Vec<CertificateRef>> {
    Ok(client.query("SELECT id,status::text,status='issued' AND cert_pem IS NOT NULL AND btrim(cert_pem)<>'' AND key_pem IS NOT NULL AND btrim(key_pem)<>'' FROM certificates ORDER BY id",&[]).await?.into_iter().map(|row|CertificateRef {id:row.get(0),status:row.get(1),usable:row.get(2)}).collect())
}
pub fn render_preview(config:&RuntimeConfig,data:&ConfigSnapshotData,certs:&[CertificateRef]) -> super::Result<RenderOutcome> {
    let dns = crate::dns_guard::Descriptor::unavailable(&config.target_dns, "Preview descriptor was not supplied");
    render_preview_inner(config, data, certs, &dns, true)
}

pub fn render_preview_with_context(config:&RuntimeConfig,data:&ConfigSnapshotData,certs:&[CertificateRef],dns:&crate::dns_guard::Descriptor) -> super::Result<RenderOutcome> {
    render_preview_inner(config, data, certs, dns, false)
}

fn render_preview_inner(config:&RuntimeConfig,data:&ConfigSnapshotData,certs:&[CertificateRef],dns:&crate::dns_guard::Descriptor,missing_descriptor:bool) -> super::Result<RenderOutcome> {
    let unanswered = std::cell::RefCell::new(BTreeSet::new());
    let resolve = |host:&str,port:u16| {
        // localhost is a fixed local classification, not an unanswered custom name.
        if host.trim_end_matches('.').eq_ignore_ascii_case("localhost") {
            return vec![std::net::Ipv4Addr::LOCALHOST.into(), std::net::Ipv6Addr::LOCALHOST.into()];
        }
        unanswered.borrow_mut().insert((host.to_ascii_lowercase(), port));
        Vec::new()
    };
    let mut outcome = super::model_render::render_with_context(config, data, certs, &super::model_render::RenderContext {
        runtime_dns: dns, resolve: &resolve, is_local_addr: &super::internal_targets::is_local_addr,
    });
    let unanswered = unanswered.into_inner();
    let asked = |target:&str| {
        let target = if target.contains("://") {target.to_owned()} else {format!("http://{}",target.split_whitespace().next().unwrap_or(""))};
        url::Url::parse(&target).ok().is_some_and(|url| match (url.host_str(),url.port_or_known_default()) {
            (Some(host),Some(port)) => unanswered.contains(&(host.to_ascii_lowercase(),port)), _ => false,
        })
    };
    let mut paths = BTreeMap::<String,BTreeSet<String>>::new();
    for row in data.servers.iter().filter(|row|row["enabled"]==true) {
        let Some(resource_id)=row["id"].as_str() else {continue;};
        let cfg = &row["config"];
        let explicit = cfg["locations"].as_array().is_some_and(|rows|!rows.is_empty());
        for location in super::render::effective_locations(cfg) {
            let target = location["proxyPass"].as_str();
            let id = location["upstreamId"].as_str().or_else(|| (!explicit).then(||row["upstreamId"].as_str()).flatten());
            let upstream = super::internal_targets::referenced_upstream(&data.upstreams,id,target);
            let runtime = location["resolveAtRuntime"]==true || upstream.is_some_and(|u|u["resolveAtRuntime"]==true);
            let unresolved = target.is_some_and(asked) || upstream.is_some_and(|u|u["servers"].as_array().into_iter().flatten().filter_map(Value::as_str).any(asked));
            if unresolved || missing_descriptor && runtime {
                paths.entry(resource_id.to_owned()).or_default().insert(location["path"].as_str().unwrap_or("/").to_owned());
            }
        }
    }
    for row in data.streams.iter().filter(|row|row["enabled"]==true) {
        let Some(resource_id)=row["id"].as_str() else {continue;};
        if row["proxyPass"].as_str().is_some_and(asked) || missing_descriptor && row["resolveAtRuntime"]==true {
            paths.entry(resource_id.to_owned()).or_default().insert("stream".into());
        }
    }
    outcome.unresolved_paths = paths.into_iter().map(|(id,paths)|(id,paths.into_iter().collect())).collect();
    Ok(outcome)
}
pub fn cert_file_paths(config:&RuntimeConfig,id:&str) -> super::Result<(PathBuf,PathBuf)> {
    if id.is_empty() || !id.bytes().all(|byte|byte.is_ascii_alphanumeric() || b"_-".contains(&byte)) {
        return Err(super::ControlError::Invalid("Invalid managed resource id".into()));
    }
    let directory=config.certs_dir.join(id);
    Ok((directory.join("fullchain.pem"),directory.join("privkey.pem")))
}

fn valid_port(value: &serde_json::Value) -> Option<u16> {
    value
        .as_f64()
        .filter(|p| p.fract() == 0.0 && (1.0..=65535.0).contains(p))
        .map(|p| p as u16)
}
pub(crate) fn process_ports(config: &RuntimeConfig) -> Vec<u16> {
    let database = url::Url::parse(&config.database_url)
        .ok()
        .and_then(|u| u.port())
        .unwrap_or(5432);
    vec![config.web_port, config.listen.port(), database]
}
pub fn ports_for_snapshot(config: &RuntimeConfig, data: &ConfigSnapshotData) -> ReservedPorts {
    ports_for_snapshot_with_dns(config, data, &crate::dns_guard::status(config))
}
pub(crate) fn ports_for_snapshot_with_dns(config: &RuntimeConfig, data: &ConfigSnapshotData, dns: &crate::dns_guard::Descriptor) -> ReservedPorts {
    let mut owners = BTreeMap::new();
    let mut http = BTreeSet::new();
    let mut stream_tcp = BTreeSet::new();
    for p in NGINX_BASE_LISTEN_PORTS {
        http.insert(p);
        owners.entry(p).or_insert_with(|| {
            if p == 8080 {
                format!("관리 화면 :{p}")
            } else {
                format!("nginx 기본 리스너 :{p}")
            }
        });
    }
    for row in &data.servers {
        let name = row["serverName"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("(이름 없음)");
        if let Some(p) = valid_port(&row["listen"]) {
            http.insert(p);
            owners
                .entry(p)
                .or_insert_with(|| format!("리버스 프록시 {name}"));
        }
        if let Some(p) = valid_port(&row["config"]["tlsPort"]) {
            http.insert(p);
            owners
                .entry(p)
                .or_insert_with(|| format!("리버스 프록시 {name} 의 HTTPS 포트"));
        }
    }
    for row in &data.streams {
        if row["protocol"] != "tcp" {
            continue;
        }
        if let Some(p) = valid_port(&row["listen"]) {
            stream_tcp.insert(p);
            owners
                .entry(p)
                .or_insert_with(|| format!("stream → {}", row["proxyPass"].as_str().unwrap_or("")));
        }
    }
    let process = process_ports(config);
    for (p, label) in process.iter().zip([
        "웹 앱 (PORT)",
        "API (API_PORT)",
        "데이터베이스 (DATABASE_URL)",
    ]) {
        owners.entry(*p).or_insert_with(|| label.into());
    }
    let mut reserved = ReservedPorts {
        process,
        process_udp: Vec::new(),
        http: http.into_iter().collect(),
        http_plain: std::iter::once(80).chain(std::iter::once(super::self_proxy::ADMIN_UI_PORT)).chain(data.servers.iter().filter_map(|r|valid_port(&r["listen"]))).collect(),
        http_tls: std::iter::once(443).chain(data.servers.iter().filter_map(|r|valid_port(&r["config"]["tlsPort"]))).collect(),
        stream_tcp: stream_tcp.into_iter().collect(),
        owners,
    };
    crate::dns_guard::reserve_port(&mut reserved, dns);
    reserved
}

#[cfg(test)]
mod preview_context_tests {
    const SERVER_ID:&str="11111111-1111-4111-8111-111111111111";
    use super::*;
    use serde_json::json;

    fn fixture() -> (RuntimeConfig, ConfigSnapshotData) {
        let mut config = RuntimeConfig::read(|key| match key {
            "DATABASE_URL" => Some("postgres://fixture@127.0.0.1/fixture".into()),
            "AUTH_SECRET" => Some("preview-context-fixture".into()),
            _ => None,
        }).unwrap();
        config.in_container = true;
        config.target_dns.descriptor_path = "/must-not-be-read/preview-descriptor".into();
        let data = ConfigSnapshotData {
            servers: vec![json!({"id":SERVER_ID,"serverName":"preview.test","listen":18086,"enabled":true,"config":{"locations":[
                {"path":"/named","proxyPass":"http://private-admin.test:8080"},
                {"path":"/literal","proxyPass":"http://127.0.0.1:8080"},
                {"path":"/localhost","proxyPass":"http://localhost:8080"},
                {"path":"/runtime","proxyPass":"http://runtime.test:8080","resolveAtRuntime":true},
                {"path":"/ordinary","proxyPass":"http://ordinary.test:9000"}
            ]}})], ..Default::default()
        };
        (config, data)
    }

    fn text(outcome: &RenderOutcome) -> String {
        assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
        String::from_utf8(outcome.rendered.conf[&format!("server-{SERVER_ID}.conf")].clone()).unwrap()
    }

    fn without_named_location(mut text: String) -> String {
        let start = text.find("    location /named {").unwrap();
        let end = start + text[start..].find("\n    }").unwrap() + "\n    }".len();
        text.replace_range(start..end, "");
        text
    }

    #[test]
    fn no_lookup_preview_differs_only_in_the_reported_named_location() {
        let (config, data) = fixture();
        let dns = crate::dns_guard::Descriptor::available(&config.target_dns);
        let apply = super::super::model_render::render_with_context(&config, &data, &[], &super::super::model_render::RenderContext {
            runtime_dns: &dns,
            is_local_addr: &super::super::internal_targets::is_local_addr,
            resolve: &|host,port| {
                assert_eq!((host,port), ("private-admin.test",8080));
                vec!["127.0.0.1".parse().unwrap()]
            },
        });
        let preview = render_preview_with_context(&config, &data, &[], &dns).unwrap();
        let actual = text(&apply);
        let predicted = text(&preview);
        assert_eq!(preview.unresolved_paths, BTreeMap::from([(SERVER_ID.into(),vec!["/named".into()])]));
        assert_ne!(actual, predicted);
        assert_eq!(without_named_location(actual), without_named_location(predicted));
    }

    #[test]
    fn bulk_preview_keeps_the_same_path_separate_for_each_server() {
        let (config,mut data)=fixture();
        let other="22222222-2222-4222-8222-222222222222";
        data.servers[0]["config"]=json!({"proxyPass":"http://private-admin.test:8080"});
        let mut second=data.servers[0].clone();second["id"]=json!(other);second["listen"]=json!(18087);
        data.servers.push(second);
        let dns=crate::dns_guard::Descriptor::available(&config.target_dns);
        let preview=render_preview_with_context(&config,&data,&[],&dns).unwrap();
        assert!(preview.skipped.is_empty(),"{:?}",preview.skipped);
        assert_eq!(preview.unresolved_paths,BTreeMap::from([
            (SERVER_ID.into(),vec!["/".into()]),(other.into(),vec!["/".into()]),
        ]));
    }

    #[test]
    fn stream_preview_uses_a_stream_marker_and_descriptorless_runtime_is_still_marked() {
        let (config,mut data)=fixture();data.servers.clear();
        let named="33333333-3333-4333-8333-333333333333";
        let runtime="44444444-4444-4444-8444-444444444444";
        data.streams=vec![
            json!({"id":named,"listen":19001,"protocol":"tcp","enabled":true,"proxyPass":"private-admin.test:19001"}),
            json!({"id":runtime,"listen":19002,"protocol":"tcp","enabled":true,"proxyPass":"runtime.test:9000","resolveAtRuntime":true}),
        ];
        let dns=crate::dns_guard::Descriptor::available(&config.target_dns);
        let preview=render_preview_with_context(&config,&data,&[],&dns).unwrap();
        assert!(preview.skipped.is_empty(),"{:?}",preview.skipped);
        assert_eq!(preview.unresolved_paths,BTreeMap::from([(named.into(),vec!["stream".into()])]));
        let preview=render_preview(&config,&data,&[]).unwrap();
        assert_eq!(preview.unresolved_paths,BTreeMap::from([
            (named.into(),vec!["stream".into()]),(runtime.into(),vec!["stream".into()]),
        ]));
    }

    #[test]
    fn descriptorless_preview_is_unavailable_and_marks_runtime_locations_too() {
        let (config, data) = fixture();
        let preview = render_preview(&config, &data, &[]).unwrap();
        let text = text(&preview);
        assert_eq!(preview.unresolved_paths, BTreeMap::from([(SERVER_ID.into(),vec!["/named".into(),"/runtime".into()])]));
        let runtime = text.split("location /runtime {").nth(1).unwrap().split('}').next().unwrap();
        assert!(runtime.contains("return 502;"));
        assert!(!text.contains("upstream nwui_resolve_"));
    }
}

/// Read-only A8 probe; any unreadable or unexpected inode remains uncertain.
pub async fn apply_uncertain(config: &RuntimeConfig) -> bool {
    match tokio::fs::symlink_metadata(config.nginx_conf_dir.join(".nwui-apply-uncertain")).await {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        _ => true,
    }
}

/// Digest of the stored certificate and sealed key; the caller never opens key material.
pub fn certificate_content_digest(cert: &str, sealed_key: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(cert.as_bytes());
    hash.update([0]);
    hash.update(sealed_key.as_bytes());
    hex::encode(hash.finalize())
}

#[cfg(test)]
mod publisher_read_tests {
    use super::*;
    #[test]
    fn certificate_digest_keeps_the_stored_material_separator() {
        assert_eq!(certificate_content_digest("certificate","sealed-key"),"93e1275b82d4ac5dcdc0ee3d8b123ea8cc9c0a9139ac638da2bc0b4e352776d7");
        assert_ne!(certificate_content_digest("ab","c"),certificate_content_digest("a","bc"));
    }
    #[tokio::test]
    async fn a8_probe_is_read_only_and_keeps_unexpected_inodes_uncertain() {
        let root=tempfile::tempdir().unwrap();
        let config=crate::config::RuntimeConfig::read(|key|match key {
            "DATABASE_URL"=>Some("postgres://fixture@127.0.0.1/fixture".into()),"AUTH_SECRET"=>Some("shared-read-fixture".into()),
            "NGINX_CONF_D"=>Some(root.path().display().to_string()),_=>None,
        }).unwrap();
        let marker=root.path().join(".nwui-apply-uncertain");
        assert!(!apply_uncertain(&config).await);
        std::fs::write(&marker,b"unknown outcome").unwrap();
        assert!(apply_uncertain(&config).await);assert_eq!(std::fs::read(&marker).unwrap(),b"unknown outcome");
        std::fs::remove_file(&marker).unwrap();std::fs::create_dir(&marker).unwrap();
        assert!(apply_uncertain(&config).await);assert!(marker.is_dir());
    }
}

#[cfg(test)]
mod skipped_summary_tests {
    use super::*;

    #[test]
    fn unresolved_targets_get_one_hint_after_each_item() {
        let hint = "  설정을 적용할 때 대상 이름의 주소를 찾지 못했습니다. 이름이 맞는지 확인하세요. 대상이 늘 켜져 있지 않거나 주소가 바뀔 수 있다면 대상의 이름 확인 방식을 「실행 중에 이름 확인」으로 바꾸세요.";
        let unresolved = SkippedRecord {
            kind: SkippedKind::Server,
            id: "one".into(),
            label: "one.example.test".into(),
            reason: "nginx: [emerg] host not found in upstream \"target.example.test\"".into(),
            kept_previous: false,
        };
        let other = SkippedRecord {
            id: "two".into(),
            label: "two.example.test".into(),
            reason: "duplicate listen".into(),
            ..unresolved.clone()
        };
        let kept = SkippedRecord {
            id: "three".into(),
            label: "three.example.test".into(),
            kept_previous: true,
            ..unresolved.clone()
        };
        let excluded = skipped_summary(std::slice::from_ref(&unresolved));
        assert!(excluded.contains("적용에서 제외한 설정 1개"));
        assert!(excluded.ends_with(&format!("{}\n{hint}", unresolved.reason)));

        let mixed = skipped_summary(&[unresolved, other, kept]);
        assert_eq!(mixed.lines().filter(|line| *line == hint).count(), 2);
        assert!(mixed.contains(&format!(" — 이 항목은 지금 동작하지 않습니다\n{hint}\n- 리버스 프록시 two.example.test: duplicate listen — 이 항목은 지금 동작하지 않습니다\n-")));
        assert!(mixed.ends_with(&format!(" — 직전 적용본을 유지합니다\n{hint}")));
    }
}
