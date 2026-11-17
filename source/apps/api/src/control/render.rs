use super::performance::*;
use super::validation::{DEFAULT_REAL_IP_HEADER, HEADER_DIRECTIVES, REAL_IP_HEADERS};
use super::{ControlError, Directive, Result};
use regex::Regex;
use serde_json::{Value, json};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::LazyLock;

pub const ACME_CHALLENGE_INCLUDE: &str = "include /etc/nginx/snippets/acme-challenge.conf;";
const SERVER_INDENT: &str = "    ";
const LOCATION_INDENT: &str = "        ";
const STATUS_HEADER: &str = "add_header X-Cache-Status $upstream_cache_status always;";
const CLOUDFLARE_IP_RANGES: &[&str] = &[
    "173.245.48.0/20",
    "103.21.244.0/22",
    "103.22.200.0/22",
    "103.31.4.0/22",
    "141.101.64.0/18",
    "108.162.192.0/18",
    "190.93.240.0/20",
    "188.114.96.0/20",
    "197.234.240.0/22",
    "198.41.128.0/17",
    "162.158.0.0/15",
    "104.16.0.0/13",
    "104.24.0.0/14",
    "172.64.0.0/13",
    "131.0.72.0/22",
    "2400:cb00::/32",
    "2606:4700::/32",
    "2803:f800::/32",
    "2405:b500::/32",
    "2405:8100::/32",
    "2a06:98c0::/29",
    "2c0f:f248::/32",
];
const INTERMEDIATE_CIPHERS: &str = "ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-CHACHA20-POLY1305:ECDHE-RSA-CHACHA20-POLY1305:DHE-RSA-AES128-GCM-SHA256:DHE-RSA-AES256-GCM-SHA384:DHE-RSA-CHACHA20-POLY1305";
static UUID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\A[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\z")
        .unwrap()
});

fn string<'a>(obj: &'a Value, key: &str) -> Result<&'a str> {
    obj[key]
        .as_str()
        .ok_or_else(|| format!("Invalid nginx configuration field: {key}").into())
}
fn number(obj: &Value, key: &str) -> Result<String> {
    obj[key]
        .as_f64()
        .filter(|value| {
            value.is_finite() && value.fract() == 0.0 && (1.0..=65535.0).contains(value)
        })
        .map(|value| (value as u16).to_string())
        .ok_or_else(|| format!("Invalid nginx configuration field: {key}").into())
}
fn directives(value: &Value) -> Result<Vec<Directive>> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value(value.clone())
        .map_err(|_| ControlError::from("Invalid nginx directives"))
}

pub fn render_upstream(u: &Value) -> Result<String> {
    render_upstream_with_dns(u, None)
}

pub fn render_upstream_with_dns(u: &Value, resolver: Option<SocketAddr>) -> Result<String> {
    super::validation::assert_valid_upstream_name(string(u, "name")?)?;
    if super::validation::parse_runtime_dns_flag(u)? {
        super::validation::assert_valid_upstream_name(string(u, "name")?)?;
        let hex = runtime_zone_hex(&u["id"])?;
        let targets = u["servers"]
            .as_array()
            .ok_or("Invalid upstream servers")?
            .iter()
            .map(|v| {
                let member = v.as_str().ok_or("Invalid upstream server")?;
                super::validation::assert_valid_runtime_host_port(member, true)?;
                let host = member
                    .split_ascii_whitespace()
                    .next()
                    .unwrap()
                    .rsplit_once(':')
                    .unwrap()
                    .0;
                Ok((member.to_owned(), host.parse::<std::net::IpAddr>().is_err()))
            })
            .collect::<Result<Vec<_>>>()?;
        return runtime_group(
            string(u, "name")?,
            &format!("nwui_resolve_upstream_{hex}"),
            &targets,
            resolver,
        );
    }
    let name = string(u, "name")?;
    let servers = u["servers"]
        .as_array()
        .ok_or_else(|| ControlError::from("Invalid upstream servers"))?;
    let lines = servers
        .iter()
        .map(|v| {
            v.as_str()
                .map(|s| format!("    server {s};"))
                .ok_or_else(|| ControlError::from("Invalid upstream server"))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(format!("upstream {name} {{\n{}\n}}\n", lines.join("\n")))
}

fn runtime_zone_hex(id: &Value) -> Result<String> {
    id.as_str()
        .filter(|id| UUID_RE.is_match(id))
        .map(|id| id.to_ascii_lowercase().replace('-', ""))
        .ok_or_else(|| "실행 중 이름 확인에는 리소스의 전체 UUID가 필요합니다.".into())
}

fn runtime_group(
    name: &str,
    zone: &str,
    targets: &[(String, bool)],
    resolver: Option<SocketAddr>,
) -> Result<String> {
    if resolver.is_some_and(|endpoint| {
        endpoint.ip() != std::net::IpAddr::V4(Ipv4Addr::LOCALHOST) || endpoint.port() == 0
    }) {
        return Err("실행 중 이름 확인은 신뢰한 loopback DNS 서비스만 사용할 수 있습니다.".into());
    }
    let mut lines = vec![format!("    zone {zone} 64k;")];
    if let Some(endpoint) = resolver {
        lines.push(format!("    resolver {endpoint} valid=10s;"));
        lines.push("    resolver_timeout 2s;".into());
    }
    for (member, dns) in targets {
        if !dns {
            lines.push(format!("    server {member};"));
        } else if resolver.is_some() {
            lines.push(format!("    server {member} resolve;"));
        } else {
            let weight = member
                .split_ascii_whitespace()
                .skip(1)
                .collect::<Vec<_>>()
                .join(" ");
            let weight = if weight.is_empty() {
                String::new()
            } else {
                format!(" {weight}")
            };
            lines.push(format!("    # runtime DNS unavailable: {member}"));
            lines.push(format!("    server 127.0.0.1:1{weight} down;"));
        }
    }
    Ok(format!("upstream {name} {{\n{}\n}}\n", lines.join("\n")))
}

pub(crate) fn generated_http_group_names(cfg: &Value) -> Result<Vec<String>> {
    let locations = cfg["locations"].as_array().filter(|a| !a.is_empty());
    let flags = if let Some(locations) = locations {
        locations
            .iter()
            .map(super::validation::parse_runtime_dns_flag)
            .collect::<Result<Vec<_>>>()?
    } else {
        vec![super::validation::parse_runtime_dns_flag(cfg)?]
    };
    if !flags.iter().any(|flag| *flag) {
        return Ok(vec![]);
    }
    let hex = runtime_zone_hex(&cfg["serverId"])?;
    Ok(flags
        .iter()
        .enumerate()
        .filter(|(_, flag)| **flag)
        .map(|(i, _)| format!("nwui_resolve_{hex}_{i}"))
        .collect())
}

/// Check all HTTP groups before save/apply. Historical reserved-prefix names are
/// retained until an actual generated group would collide with them.
pub(crate) fn validate_http_group_names(upstreams: &[Value], servers: &[Value]) -> Result<()> {
    let mut names = std::collections::BTreeSet::new();
    for upstream in upstreams {
        if let Some(name) = upstream["name"].as_str() {
            if !names.insert(name.to_ascii_lowercase()) {
                return Err(format!("upstream 이름 {name} 이 중복됩니다.").into());
            }
        }
    }
    for server in servers {
        let mut cfg = server["config"].clone();
        cfg["serverId"] = server["id"].clone();
        for name in generated_http_group_names(&cfg)? {
            if !names.insert(name.to_ascii_lowercase()) {
                return Err(
                    format!("생성된 upstream 이름 {name} 이 기존 그룹과 충돌합니다.").into(),
                );
            }
        }
    }
    Ok(())
}

fn header_value(value: &str) -> String {
    if value.is_empty() || super::validation::injection(value) {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.into()
    }
}
fn directive_lines(value: &Value, indent: &str) -> Result<Vec<String>> {
    Ok(directives(value)?
        .into_iter()
        .map(|mut d| {
            if HEADER_DIRECTIVES.contains(&d.name.as_str()) && d.args.len() >= 2 {
                d.args[1] = header_value(&d.args[1]);
            }
            format!("{indent}{} {};", d.name, d.args.join(" "))
        })
        .collect())
}

/// The literal URL and nginx's observable authority are deliberately separate.
/// WHATWG normalization must never rewrite URI bytes or redirect matching.
#[derive(Debug, Clone)]
pub(crate) struct OriginalProxyTarget {
    pub(crate) scheme: String,
    pub(crate) hostname: String,
    pub(crate) suffix: String,
    pub(crate) proxy_host: String,
    pub(crate) proxy_port: String,
    pub(crate) endpoint: String,
    original: String,
    origin: String,
}
impl OriginalProxyTarget {
    pub(crate) fn parse(value: &str) -> Result<Self> {
        let invalid = || {
            ControlError::from(
                "실행 중 이름 확인 대상은 변수·사용자 정보·fragment가 없는 고정 DNS 이름의 HTTP(S) URL이어야 합니다.",
            )
        };
        if value.contains(['$', '#', '\\']) || super::validation::injection(value) {
            return Err(invalid());
        }
        let (scheme, rest) = value.split_once("://").ok_or_else(invalid)?;
        if !matches!(scheme, "http" | "https") {
            return Err(invalid());
        }
        let end = rest.find(['/', '?']).unwrap_or(rest.len());
        let authority = &rest[..end];
        if authority.contains('@') {
            return Err(invalid());
        }
        let parsed = url::Url::parse(value).map_err(|_| invalid())?;
        if !matches!(parsed.host(), Some(url::Host::Domain(_))) {
            return Err(invalid());
        }
        let (hostname, explicit_port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        if hostname.is_empty()
            || hostname.len() > 253
            || !hostname
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        {
            return Err(invalid());
        }
        let default_port = if scheme == "https" { 443 } else { 80 };
        let port = match explicit_port {
            Some(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
                text.parse::<u16>().map_err(|_| invalid())?
            }
            Some(_) => return Err(invalid()),
            None => default_port,
        };
        if port == 0 {
            return Err(invalid());
        }
        let proxy_host = if port == default_port {
            hostname.to_owned()
        } else {
            authority.to_owned()
        };
        let proxy_port = if port == default_port {
            default_port.to_string()
        } else {
            explicit_port.unwrap().to_owned()
        };
        Ok(Self {
            scheme: scheme.into(),
            hostname: hostname.into(),
            suffix: rest[end..].into(),
            proxy_host,
            proxy_port,
            endpoint: format!(
                "{hostname}:{}",
                explicit_port
                    .map(str::to_owned)
                    .unwrap_or_else(|| default_port.to_string())
            ),
            original: value.into(),
            origin: format!("{scheme}://{authority}"),
        })
    }
    // The bind classifier already validated an explicit admin authority; preserve its wire spelling.
    fn admin(value: &str) -> Result<Self> {
        let (scheme,rest)=value.split_once("://").ok_or("Invalid admin proxy URL")?;
        let end=rest.find(['/', '?']).unwrap_or(rest.len());let authority=&rest[..end];
        let (hostname,port)=authority.rsplit_once(':').ok_or("Invalid admin proxy port")?;
        Ok(Self {scheme:scheme.into(),hostname:hostname.trim_matches(['[',']']).into(),suffix:rest[end..].into(),proxy_host:authority.into(),proxy_port:port.into(),endpoint:authority.into(),original:value.into(),origin:format!("{scheme}://{authority}")})
    }
    pub(crate) fn with_authority(&self, name: &str) -> String {
        format!("{}://{name}{}", self.scheme, self.suffix)
    }
    pub(crate) fn redirect_default(&self, path: &str) -> (String, String) {
        if self.suffix.is_empty() {
            (format!("{}/", self.origin), "/".into())
        } else {
            (self.original.clone(), path.into())
        }
    }
    /// Replace complete nginx variable tokens, preserving backslashes and matching complete names.
    fn restore_variables(&self, input: &str) -> String {
        let bytes = input.as_bytes();
        let mut output = String::new();
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at] == b'\\' {
                let start = at;
                while at < bytes.len() && bytes[at] == b'\\' {
                    at += 1;
                }
                output.push_str(&input[start..at]);
                continue;
            }
            if bytes[at] == b'$' {
                let start = at;
                at += 1;
                let braced = bytes.get(at) == Some(&b'{');
                if braced {
                    at += 1;
                }
                let name_start = at;
                while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                    at += 1;
                }
                let name = &input[name_start..at];
                if braced && bytes.get(at) != Some(&b'}') {
                    output.push_str(&input[start..at]);
                    continue;
                }
                if braced {
                    at += 1;
                }
                match name {
                    "proxy_host" => output.push_str(&self.proxy_host),
                    "proxy_port" => output.push_str(&self.proxy_port),
                    _ => output.push_str(&input[start..at]),
                }
                continue;
            }
            let next = input[at..].chars().next().unwrap();
            output.push(next);
            at += next.len_utf8();
        }
        output
    }
    fn restore_directive(&self, d: &mut Directive) {
        // Header names, regex patterns, flags and numeric/enumerated arguments are
        // literal nginx tokens, even if their spelling happens to contain '$'.
        let positions: Vec<usize> = match d.name.as_str() {
            "proxy_set_header" => vec![1],
            // Rewrite/return run before proxy context creation; nginx sees these
            // variables as empty in both the static and generated locations.
            "rewrite" | "return" => vec![],
            "proxy_redirect" if d.args.len() == 2 => {
                if d.args[0].starts_with('~') {
                    vec![1]
                } else {
                    vec![0, 1]
                }
            }
            _ => vec![],
        };
        for index in positions {
            if let Some(arg) = d.args.get_mut(index) {
                *arg = self.restore_variables(arg);
            }
        }
    }
    fn adapt_directives(&self, location: &Value, inherited: &Value) -> Result<Vec<Directive>> {
        let path = string(location, "path")?;
        let mut dirs = directives(&location["directives"])?;
        if !dirs.iter().any(|d| d.name == "add_header") {
            dirs.extend(
                directives(inherited)?
                    .into_iter()
                    .filter(|d| d.name == "add_header"),
            );
        }
        let mut has_redirect = false;
        let mut has_host = false;
        for d in &mut dirs {
            if d.name == "proxy_set_header"
                && d.args
                    .first()
                    .is_some_and(|name| name.eq_ignore_ascii_case("host"))
            {
                has_host = true;
            }
            if d.name == "proxy_redirect" {
                has_redirect = true;
                // Variable URLs have no implicit mapping; nginx rejects an explicit default.
                if d.args == ["default"] && !self.original.contains('$') {
                    let (from, to) = self.redirect_default(path);
                    d.args = vec![from, to];
                    continue;
                }
            }
            self.restore_directive(d);
        }
        if !has_host {
            dirs.push(Directive {
                name: "proxy_set_header".into(),
                args: vec![
                    "Host".into(),
                    if location.get("directives").is_none() {
                        "$host".into()
                    } else {
                        self.proxy_host.clone()
                    },
                ],
            });
        }
        if !has_redirect && !self.original.contains('$') {
            let (from, to) = self.redirect_default(path);
            dirs.push(Directive {
                name: "proxy_redirect".into(),
                args: vec![from, to],
            });
        }
        Ok(dirs)
    }
}

fn ip_access(value: &Value, indent: &str) -> Result<Vec<String>> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    let entries = value["entries"]
        .as_array()
        .ok_or_else(|| ControlError::from("Invalid IP access entries"))?;
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let whitelist = value["mode"] == "whitelist";
    let verb = if whitelist { "allow" } else { "deny" };
    let mut lines = Vec::new();
    for entry in entries {
        let entry = entry
            .as_str()
            .ok_or_else(|| ControlError::from("Invalid IP access entry"))?;
        lines.push(format!("{indent}{verb} {entry};"));
    }
    if whitelist {
        lines.push(format!("{indent}deny all;"));
    }
    Ok(lines)
}
fn render_location(location: &Value, extra: Vec<String>) -> Result<String> {
    let path = string(location, "path")?;
    let target = string(location, "proxyPass")?;
    let mut lines = vec![format!("{LOCATION_INDENT}proxy_pass {target};")];
    if location.get("directives").is_none() {
        lines.push(format!("{LOCATION_INDENT}proxy_set_header Host $host;"));
    } else {
        lines.extend(directive_lines(&location["directives"], LOCATION_INDENT)?);
    }
    if location["retryOtherInstances"]==true {
        lines.extend(["proxy_next_upstream error timeout http_502 http_503 http_504;","proxy_next_upstream_tries 2;","proxy_next_upstream_timeout 3s;"].map(|line|format!("{LOCATION_INDENT}{line}")));
        if !directives(&location["directives"])?.iter().any(|directive|directive.name=="proxy_connect_timeout"){
            lines.push(format!("{LOCATION_INDENT}proxy_connect_timeout 2s;"));
        }
    }
    lines.extend(ip_access(&location["ipAccess"], LOCATION_INDENT)?);
    lines.extend(extra);
    Ok(format!(
        "    location {path} {{\n{}\n    }}",
        lines.join("\n")
    ))
}

fn cache_zone_hex(server_id: &Value) -> Result<String> {
    server_id
        .as_str()
        .filter(|s| UUID_RE.is_match(s))
        .map(|s| s.to_ascii_lowercase().replace('-', ""))
        .ok_or_else(|| {
            "캐시를 켜려면 서버 id(UUID)가 필요합니다. 서버를 다시 저장한 뒤 적용하세요.".into()
        })
}
struct CacheZone {
    hex: String,
    name: String,
    bypass_var: String,
}
fn cache_prelude(zone: &CacheZone, t: &CacheConfig) -> String {
    format!(
        "proxy_cache_path /var/cache/nginx/proxy-{} levels=1:2 keys_zone={}:10m max_size={} inactive={} use_temp_path=off;\nmap \"$http_authorization:$http_cookie\" {} {{\n    \":\" 0;\n    default 1;\n}}\n",
        zone.hex, zone.name, t.max_size, t.inactive, zone.bypass_var
    )
}
fn has_status_header(list: &Value) -> bool {
    list.as_array().into_iter().flatten().any(|d| {
        d["name"] == "add_header"
            && d["args"][0]
                .as_str()
                .is_some_and(|s| s.eq_ignore_ascii_case("x-cache-status"))
    })
}
fn cache_location_lines(location: &Value, zone: &CacheZone, t: &CacheConfig) -> Vec<String> {
    let mut lines = vec![
        format!("proxy_cache {};", zone.name),
        "proxy_cache_key \"$scheme|$host|$proxy_host|$request_uri\";".into(),
        format!("proxy_cache_valid 200 301 302 {};", t.valid),
    ];
    if t.lock {
        lines.push("proxy_cache_lock on;".into());
    }
    lines.push(format!("proxy_cache_bypass {};", zone.bypass_var));
    lines.push(format!("proxy_no_cache {};", zone.bypass_var));
    if t.use_stale {
        lines.extend([
            "proxy_cache_use_stale error timeout updating http_500 http_502 http_503 http_504;"
                .into(),
            "proxy_cache_background_update on;".into(),
        ]);
    }
    let own_add_header = location["directives"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|d| d["name"] == "add_header");
    if t.status_header && own_add_header && !has_status_header(&location["directives"]) {
        lines.push(STATUS_HEADER.into());
    }
    lines
        .into_iter()
        .map(|s| format!("{LOCATION_INDENT}{s}"))
        .collect()
}
fn switch(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}
fn gzip_server_lines(gzip: Option<&GzipConfig>, override_on: bool) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(g) = gzip {
        lines.push(format!("gzip {};", switch(g.enabled)));
    }
    let default_config = GzipConfig::default();
    let t = gzip.unwrap_or(&default_config);
    if t.enabled || override_on {
        let types = if t.types == "text-and-fonts" {
            GZIP_TEXT_AND_FONTS_TYPES
        } else {
            GZIP_TEXT_TYPES
        };
        lines.extend([
            format!("gzip_types {types};"),
            format!("gzip_min_length {};", t.min_length),
            format!("gzip_comp_level {};", t.comp_level),
            format!("gzip_vary {};", switch(t.vary)),
            format!("gzip_proxied {};", if t.proxied { "any" } else { "off" }),
        ]);
    }
    lines
        .into_iter()
        .map(|s| format!("{SERVER_INDENT}{s}"))
        .collect()
}
fn tls_tuning(t: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    let protocol = match t["sslProtocols"].as_str() {
        Some("intermediate") => Some("TLSv1.2 TLSv1.3"),
        Some("modern") => Some("TLSv1.3"),
        _ => None,
    };
    if let Some(p) = protocol {
        lines.push(format!("    ssl_protocols {p};"));
    }
    if t["sslCiphers"] == "intermediate" {
        lines.push(format!("    ssl_ciphers {INTERMEDIATE_CIPHERS};"));
    }
    if t["preferServerCiphers"] == true {
        lines.push("    ssl_prefer_server_ciphers on;".into());
    }
    if t["http2"] == true {
        lines.push("    http2 on;".into());
    }
    if let Some(header) = hsts_header(t) {
        lines.push(format!(
            "    add_header Strict-Transport-Security \"{}\" always;",
            header.args[1]
        ));
    }
    lines
}
/// Keep generated TLS response headers structured so location materialization
/// uses the same values and order as the actual server-level directives.
fn hsts_header(t: &Value) -> Option<Directive> {
    if t["hsts"]["enabled"] != true {
        return None;
    }
    let max_age = t["hsts"]["maxAge"]
        .as_f64()
        .filter(|n| n.fract() == 0.0 && *n >= 0.0)
        .unwrap_or(63072000.0);
    // RFC 6797 requires unsigned decimal digits, never an exponent.
    let max_age = if max_age == 0.0 { 0.0 } else { max_age };
    let mut value = format!("max-age={max_age:.0}");
    if t["hsts"]["includeSubDomains"] == true {
        value.push_str("; includeSubDomains");
    }
    if t["hsts"]["preload"] == true {
        value.push_str("; preload");
    }
    Some(Directive {
        name: "add_header".into(),
        args: vec!["Strict-Transport-Security".into(), value, "always".into()],
    })
}

fn real_ip(r: &Value) -> Result<Vec<String>> {
    if r.is_null() {
        return Ok(Vec::new());
    }
    let cloudflare = r["preset"] == "cloudflare";
    let ranges: Vec<&str> = if cloudflare {
        CLOUDFLARE_IP_RANGES.to_vec()
    } else {
        r["trustedRanges"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| {
                v.as_str()
                    .ok_or_else(|| ControlError::from("Invalid trusted IP range"))
            })
            .collect::<Result<_>>()?
    };
    if ranges.is_empty() {
        return Ok(Vec::new());
    }
    let header = if cloudflare {
        "CF-Connecting-IP"
    } else {
        r["header"]
            .as_str()
            .filter(|s| REAL_IP_HEADERS.contains(s))
            .unwrap_or(DEFAULT_REAL_IP_HEADER)
    };
    let mut lines: Vec<_> = ranges
        .iter()
        .map(|cidr| format!("    set_real_ip_from {cidr};"))
        .collect();
    lines.push(format!("    real_ip_header {header};"));
    if !cloudflare && r["recursive"] == true {
        lines.push("    real_ip_recursive on;".into());
    }
    Ok(lines)
}

pub(crate) fn effective_locations(cfg: &Value) -> Vec<Value> {
    cfg["locations"]
        .as_array()
        .filter(|a| !a.is_empty())
        .cloned()
        .unwrap_or_else(|| {
            let target = cfg["upstreamName"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| format!("http://{s}"))
                .unwrap_or_else(|| cfg["proxyPass"].as_str().unwrap_or("").into());
            let mut location = json!({"path":"/", "proxyPass":target});
            if let Some(flag) = cfg.get("resolveAtRuntime") {
                location["resolveAtRuntime"] = flag.clone();
            }
            if let Some(d) = cfg.get("locationDirectives") {
                location["directives"] = d.clone();
            }
            vec![location]
        })
}

/// Render a previously field-validated resource. Cache/gzip JSON is reparsed here
/// because it controls privacy-sensitive behavior and contains raw nginx tokens.
pub fn render_reverse_proxy(cfg: &Value) -> Result<String> {
    render_reverse_proxy_with_dns(cfg, None)
}

pub fn render_reverse_proxy_with_dns(cfg: &Value, resolver: Option<SocketAddr>) -> Result<String> {
    render_reverse_proxy_with_dns_and_resolve(
        cfg,
        resolver,
        &super::internal_targets::system_resolve,
    )
}
pub(crate) fn render_reverse_proxy_with_dns_and_resolve(
    cfg: &Value,
    resolver: Option<SocketAddr>,
    resolve: super::internal_targets::Resolve,
) -> Result<String> {
    render_reverse_proxy_context(cfg,resolver,resolve,&std::collections::BTreeSet::new(),&super::internal_targets::is_local_addr)
}

#[cfg(test)]
pub(crate) fn render_reverse_proxy_with_admin_upstreams(cfg:&Value,resolver:Option<SocketAddr>,admin:&std::collections::BTreeSet<String>)->Result<String> {
    render_reverse_proxy_context(cfg,resolver,&super::internal_targets::system_resolve,admin,&super::internal_targets::is_local_addr)
}

pub(crate) fn render_reverse_proxy_context(cfg:&Value,resolver:Option<SocketAddr>,resolve:super::internal_targets::Resolve,admin:&std::collections::BTreeSet<String>,is_local:super::internal_targets::LocalAddr)->Result<String> {
    let answers=std::cell::RefCell::new(std::collections::BTreeMap::<(String,u16),Vec<std::net::IpAddr>>::new());
    let cached=|host:&str,port:u16| {
        let key=(host.to_owned(),port);
        if let Some(value)=answers.borrow().get(&key).cloned(){return value;}
        let value=resolve(host,port);answers.borrow_mut().insert(key,value.clone());value
    };
    let resolve:super::internal_targets::Resolve=&cached;
    super::validation::assert_runtime_response_directives(cfg)?;
    let protected = super::validation::validate_stored_server_config_with(cfg, resolve)?;
    if !cfg.is_object() {
        return Err("Invalid reverse proxy configuration".into());
    }
    let mut location_list = effective_locations(cfg);
    let cache = parse_cache_config(&cfg["cache"])?;
    let gzip = parse_gzip_config(&cfg["gzip"])?;
    let mut overrides = Vec::new();
    for l in &mut location_list {
        if !l.is_object() {
            return Err("Invalid nginx location configuration".into());
        }
        let c = parse_performance_override(&l["cacheEnabled"])?;
        let g = parse_performance_override(&l["gzipEnabled"])?;
        l["cacheEnabled"] = json!(c);
        overrides.push((c, g));
    }
    let mut normalized = cfg.clone();
    normalized["cache"] = json!(cache);
    normalized["gzip"] = json!(gzip);
    if cfg["locations"].as_array().is_some_and(|a| !a.is_empty()) {
        normalized["locations"] = json!(location_list);
    }
    validate_performance_conflicts(&normalized)?;
    let cache_effective: Vec<_> = overrides
        .iter()
        .map(|(c, _)| is_cache_effective(cache.as_ref(), *c))
        .collect();
    let cache_tuning = cache.unwrap_or_default();
    let zone = if cache_effective.iter().any(|v| *v) {
        let hex = cache_zone_hex(&cfg["serverId"])?;
        Some(CacheZone {
            name: format!("srv_{hex}"),
            bypass_var: format!("$srv_{hex}_bypass"),
            hex,
        })
    } else {
        None
    };
    let tls = cfg.get("tls").filter(|v| !v.is_null());
    let listen = if let Some(tls) = tls {
        format!("    listen {} ssl;", number(tls, "tlsPort")?)
    } else {
        format!("    listen {};", number(cfg, "listen")?)
    };
    let name = string(cfg, "serverName")?;
    let mut inherited_headers = Vec::new();
    let mut parts = vec![
        listen,
        format!("    server_name {name};"),
        format!("    {ACME_CHALLENGE_INCLUDE}"),
    ];
    if let Some(tls) = tls {
        parts.push(format!("    ssl_certificate {};", string(tls, "certPath")?));
        parts.push(format!(
            "    ssl_certificate_key {};",
            string(tls, "keyPath")?
        ));
        parts.extend(tls_tuning(&tls["tuning"]));
        inherited_headers.extend(hsts_header(&tls["tuning"]));
    }
    parts.extend(real_ip(&cfg["realIp"])?);
    if zone.is_some() && cache_tuning.status_header && !has_status_header(&cfg["serverDirectives"])
    {
        parts.push(format!("    {STATUS_HEADER}"));
        inherited_headers.push(Directive {
            name: "add_header".into(),
            args: vec![
                "X-Cache-Status".into(),
                "$upstream_cache_status".into(),
                "always".into(),
            ],
        });
    }
    parts.extend(gzip_server_lines(
        gzip.as_ref(),
        overrides.iter().any(|(_, g)| *g == Some(true)),
    ));
    parts.extend(directive_lines(&cfg["serverDirectives"], SERVER_INDENT)?);
    inherited_headers.extend(directives(&cfg["serverDirectives"])?);
    parts.extend(ip_access(&cfg["ipAccess"], SERVER_INDENT)?);
    let mut runtime_upstreams = Vec::new();
    for (i, l) in location_list.iter_mut().enumerate() {
        let runtime=super::validation::parse_runtime_dns_flag(l)?;
        let group=super::internal_targets::target_authority(l["proxyPass"].as_str().unwrap_or(""));
        let direct_admin=super::self_proxy::binds_admin_target_with(l["proxyPass"].as_str().unwrap_or(""),runtime,resolve,is_local);
        let bind=!runtime && group.is_some_and(|name|admin.contains(&name.to_ascii_lowercase())) || direct_admin;
        let mut extra = Vec::new();
        if protected[i] || bind {
            let mut rows=directives(&l["directives"])?;
            rows.retain(|d| !(d.name=="proxy_set_header" && d.args.first().is_some_and(|name|name.eq_ignore_ascii_case("X-Real-IP")||name.eq_ignore_ascii_case("X-Forwarded-Proto"))));
            if bind {
                if l.get("directives").is_none() {
                    rows.push(Directive {name:"proxy_set_header".into(),args:vec!["Host".into(),"$http_host".into()]});
                } else {
                    for row in &mut rows {
                        if row.name == "proxy_set_header" && row.args.first().is_some_and(|name|name.eq_ignore_ascii_case("Host")) && row.args.get(1).is_some_and(|value|value == "$host") {
                            row.args[1] = "$http_host".into();
                        }
                    }
                }
            }
            if l.get("directives").is_some() || bind {l["directives"]=json!(rows);}
        }
        if bind {
            extra.push(format!("{LOCATION_INDENT}proxy_bind 127.0.0.2;"));
        }
        if protected[i] || bind {
            extra.push(format!(
                "{LOCATION_INDENT}proxy_set_header X-Real-IP $remote_addr;"
            ));
            extra.push(format!(
                "{LOCATION_INDENT}proxy_set_header X-Forwarded-Proto $scheme;"
            ));
        }
        if let Some(zone) = zone.as_ref().filter(|_| cache_effective[i]) {
            extra.extend(cache_location_lines(l, zone, &cache_tuning));
        } else if overrides[i].0 == Some(false) {
            extra.push(format!("{LOCATION_INDENT}proxy_cache off;"));
        }
        if let Some(g) = overrides[i].1 {
            extra.push(format!("{LOCATION_INDENT}gzip {};", switch(g)));
        }
        if super::validation::parse_runtime_dns_flag(l)? {
            if cfg["upstreamName"]
                .as_str()
                .is_some_and(|name| !name.is_empty())
            {
                return Err("upstream의 이름 확인 방식은 그 upstream에서 선택하세요.".into());
            }
            let original = OriginalProxyTarget::parse(string(l, "proxyPass")?)?;
            if original.scheme == "https"
                && directives(&l["directives"])?
                    .iter()
                    .chain(inherited_headers.iter())
                    .any(|d| d.name == "proxy_ssl_name")
            {
                return Err(
                    "사용자 TLS 이름 설정은 실행 중 이름 확인과 함께 변환할 수 없습니다.".into(),
                );
            }
            let name = format!("nwui_resolve_{}_{}", runtime_zone_hex(&cfg["serverId"])?, i);
            if resolver.is_none() {
                parts.push(format!(
                    "    location {} {{\n{LOCATION_INDENT}return 502;\n    }}",
                    string(l, "path")?
                ));
                continue;
            }
            runtime_upstreams.push(runtime_group(
                &name,
                &name,
                &[(original.endpoint.clone(), true)],
                resolver,
            )?);
            let mut adapted = l.clone();
            adapted["proxyPass"] = json!(original.with_authority(&name));
            let mut dirs = original.adapt_directives(l, &json!(inherited_headers))?;
            if original.scheme == "https" {
                if dirs.iter().any(|d| d.name == "proxy_ssl_name") {
                    return Err(
                        "사용자 TLS 이름 설정은 실행 중 이름 확인과 함께 변환할 수 없습니다."
                            .into(),
                    );
                }
                dirs.push(Directive {
                    name: "proxy_ssl_name".into(),
                    args: vec![original.hostname.clone()],
                });
            }
            adapted["directives"] = json!(dirs);
            for line in &mut extra {
                if line.trim_start().starts_with("proxy_cache_key ") {
                    *line = original.restore_variables(line);
                }
            }
            parts.push(render_location(&adapted, extra)?);
        } else if direct_admin {
            let original=OriginalProxyTarget::admin(string(l,"proxyPass")?)?;
            let mut adapted=l.clone();adapted["proxyPass"]=json!(original.with_authority("127.0.0.1:8080"));
            adapted["directives"]=json!(original.adapt_directives(l,&json!(inherited_headers))?);
            for line in &mut extra {if line.trim_start().starts_with("proxy_cache_key "){*line=original.restore_variables(line);}}
            parts.push(render_location(&adapted,extra)?);
        } else {
            parts.push(render_location(l, extra)?);
        }
    }
    let prelude = zone
        .as_ref()
        .map(|z| cache_prelude(z, &cache_tuning))
        .unwrap_or_default();
    let main = format!(
        "{prelude}{}server {{\n{}\n}}\n",
        runtime_upstreams.join("\n"),
        parts.join("\n")
    );
    let Some(tls) = tls.filter(|t| t["redirectHttp"] == true) else {
        return Ok(main);
    };
    let port = number(tls, "tlsPort")?;
    let redirect_target = if tls["tlsPort"] == 443 {
        "https://$host$request_uri".into()
    } else {
        format!("https://$host:{port}$request_uri")
    };
    let http_port = number(tls, "httpPort")?;
    Ok(format!(
        "{main}\nserver {{\n    listen {http_port};\n    server_name {name};\n    {ACME_CHALLENGE_INCLUDE}\n    location / {{\n        return 301 {redirect_target};\n    }}\n}}\n"
    ))
}

pub fn render_stream(cfg: &Value) -> Result<String> {
    render_stream_with_dns(cfg, None)
}

pub fn render_stream_with_dns(cfg: &Value, resolver: Option<SocketAddr>) -> Result<String> {
    if super::validation::parse_runtime_dns_flag(cfg)? {
        let target = string(cfg, "proxyPass")?;
        super::validation::assert_valid_runtime_host_port(target, false)?;
        let name = format!("nwui_resolve_stream_{}", runtime_zone_hex(&cfg["id"])?);
        let group = runtime_group(&name, &name, &[(target.to_owned(), true)], resolver)?;
        let mut adapted = cfg.clone();
        adapted["resolveAtRuntime"] = json!(false);
        adapted["proxyPass"] = json!(name);
        return Ok(format!(
            "{group}{}",
            render_stream_with_dns(&adapted, None)?
        ));
    }
    let udp = if cfg["protocol"] == "udp" { " udp" } else { "" };
    Ok(format!(
        "server {{\n    listen {}{udp};\n    proxy_pass {};\n}}\n",
        number(cfg, "listen")?,
        string(cfg, "proxyPass")?
    ))
}

#[cfg(test)]
mod runtime_dns_adapter_tests {
    use super::*;

    #[test]
    fn original_target_preserves_literal_suffix_and_nginx_observable_authority() {
        for (url, suffix, host, port, endpoint) in [
            ("http://Px-Web", "", "Px-Web", "80", "Px-Web:80"),
            ("http://px-web:80/", "/", "px-web", "80", "px-web:80"),
            (
                "http://px-web:080/app/a%2Fb?q=x%2Fy",
                "/app/a%2Fb?q=x%2Fy",
                "px-web",
                "80",
                "px-web:080",
            ),
            (
                "http://px-web:3001/app/",
                "/app/",
                "px-web:3001",
                "3001",
                "px-web:3001",
            ),
            ("https://px-web:443", "", "px-web", "443", "px-web:443"),
            (
                "https://px-web:04444?q=x",
                "?q=x",
                "px-web:04444",
                "04444",
                "px-web:04444",
            ),
        ] {
            let target = OriginalProxyTarget::parse(url).unwrap();
            assert_eq!(target.suffix, suffix, "{url}");
            assert_eq!(target.proxy_host, host, "{url}");
            assert_eq!(target.proxy_port, port, "{url}");
            assert_eq!(target.endpoint, endpoint, "{url}");
            assert_eq!(
                target.with_authority("nwui_resolve_test"),
                format!("{}://nwui_resolve_test{suffix}", target.scheme)
            );
        }
    }

    #[test]
    fn dynamic_target_rejects_inputs_that_cannot_be_safely_adapted() {
        for url in [
            "http://127.0.0.1:9000",
            "http://127.1",
            "http://2130706433",
            "http://0x7f000001",
            "http://[::1]",
            "http://user:pass@peer",
            "http://peer/#fragment",
            "http://peer/$request_uri",
            "http://$host/",
            "http://peer\\other/",
            "http://peer:0/",
            "ftp://peer/",
            "http://peer%2eexample/",
        ] {
            assert!(OriginalProxyTarget::parse(url).is_err(), "{url}");
        }
    }

    #[test]
    fn only_actual_proxy_variable_tokens_are_restored() {
        let target = OriginalProxyTarget::parse("http://px-web:3001/app/").unwrap();
        for (input, expected) in [
            ("$proxy_host:${proxy_port}", "px-web:3001:3001"),
            ("${proxy_host}/$proxy_port", "px-web:3001/3001"),
            (
                "proxy_host $proxy_hostname ${proxy_port_suffix}",
                "proxy_host $proxy_hostname ${proxy_port_suffix}",
            ),
            (r"\$proxy_host", r"\px-web:3001"),
            (r"\\$proxy_host", r"\\px-web:3001"),
            (
                "$scheme|$host|$proxy_host|$request_uri",
                "$scheme|$host|px-web:3001|$request_uri",
            ),
        ] {
            assert_eq!(target.restore_variables(input), expected, "{input}");
        }
    }

    #[test]
    fn original_redirect_defaults_keep_url_spelling_and_distinguish_no_uri() {
        for (url, from, to) in [
            ("http://px-web:80", "http://px-web:80/", "/"),
            ("http://px-web:80/", "http://px-web:80/", "/nested/"),
            (
                "https://Px-Web:443/app%2F?q=x%2Fy",
                "https://Px-Web:443/app%2F?q=x%2Fy",
                "/nested/",
            ),
        ] {
            let target = OriginalProxyTarget::parse(url).unwrap();
            assert_eq!(
                target.redirect_default("/nested/"),
                (from.to_string(), to.to_string())
            );
        }
    }
    #[test]
    fn dynamic_host_defaults_preserve_absence_and_existing_host_rows() {
        let target = OriginalProxyTarget::parse("http://px-web:80/app/").unwrap();
        for (raw, expected) in [
            (json!({"path":"/"}), "$host"),
            (json!({"path":"/","directives":[]}), "px-web"),
            (
                json!({"path":"/","directives":[{"name":"proxy_set_header","args":["Host",""]}]}),
                "",
            ),
            (
                json!({"path":"/","directives":[{"name":"proxy_set_header","args":["hOsT","$http_host"]}]}),
                "$http_host",
            ),
            (
                json!({"path":"/","directives":[{"name":"proxy_set_header","args":["Host","${proxy_host}"]}]}),
                "px-web",
            ),
        ] {
            let dirs = target.adapt_directives(&raw, &json!([])).unwrap();
            let hosts: Vec<_> = dirs
                .iter()
                .filter(|d| d.name == "proxy_set_header" && d.args[0].eq_ignore_ascii_case("host"))
                .collect();
            assert_eq!(hosts.len(), 1);
            assert_eq!(hosts[0].args[1], expected);
        }
    }

    #[test]
    fn directive_adapter_changes_only_variable_aware_arguments_without_mutating_saved_data() {
        let target = OriginalProxyTarget::parse("http://px-web:3001/app/").unwrap();
        let location = json!({"path":"/nested/","directives":[
            {"name":"proxy_set_header","args":["X-Original","$proxy_host:${proxy_port}"]},
            {"name":"add_header","args":["X-Upstream","${proxy_host}","always"]},
            {"name":"proxy_hide_header","args":["$proxy_host"]},
            {"name":"rewrite","args":["^/literal$proxy_host$","/$proxy_port","break"]},
            {"name":"proxy_redirect","args":["~^http://literal$proxy_host$","https://$host/$proxy_port"]}
        ]});
        let before = location.clone();
        let dirs = target.adapt_directives(&location, &json!([])).unwrap();
        assert_eq!(location, before);
        assert_eq!(
            dirs.iter()
                .find(|d| d.name == "proxy_set_header" && d.args[0] == "X-Original")
                .unwrap()
                .args[1],
            "px-web:3001:3001"
        );
        assert_eq!(
            dirs.iter()
                .find(|d| d.name == "proxy_hide_header")
                .unwrap()
                .args[0],
            "$proxy_host"
        );
        let rewrite = dirs.iter().find(|d| d.name == "rewrite").unwrap();
        assert_eq!(
            rewrite.args,
            vec!["^/literal$proxy_host$", "/$proxy_port", "break"]
        );
        let redirect = dirs.iter().find(|d| d.name == "proxy_redirect").unwrap();
        assert_eq!(
            redirect.args,
            vec!["~^http://literal$proxy_host$", "https://$host/3001"]
        );
    }

    #[test]
    fn response_filter_directives_never_enter_proxy_variable_restoration() {
        let target = OriginalProxyTarget::parse("http://px-web:3001/app/").unwrap();
        for mut directive in [
            Directive {
                name: "add_header".into(),
                args: vec!["X-Original".into(), "$proxy_host:${proxy_port}".into()],
            },
            Directive {
                name: "expires".into(),
                args: vec!["$proxy_port".into()],
            },
        ] {
            let original = directive.clone();
            target.restore_directive(&mut directive);
            assert_eq!(directive, original);
        }
    }

    #[test]
    fn dynamic_redirect_rules_preserve_default_off_and_custom_order() {
        let target = OriginalProxyTarget::parse("http://px-web:80/app/").unwrap();
        let extract = |v: &Value| {
            target
                .adapt_directives(v, &json!([]))
                .unwrap()
                .into_iter()
                .filter(|d| d.name == "proxy_redirect")
                .map(|d| d.args)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            extract(&json!({"path":"/nested/","directives":[]})),
            vec![vec!["http://px-web:80/app/", "/nested/"]]
        );
        assert_eq!(
            extract(
                &json!({"path":"/nested/","directives":[{"name":"proxy_redirect","args":["off"]}]})
            ),
            vec![vec!["off"]]
        );
        assert_eq!(
            extract(
                &json!({"path":"/nested/","directives":[{"name":"proxy_redirect","args":["https://other/","/"]},{"name":"proxy_redirect","args":["default"]}]})
            ),
            vec![
                vec!["https://other/", "/"],
                vec!["http://px-web:80/app/", "/nested/"]
            ]
        );
    }

    #[test]
    fn inherited_response_headers_preserve_tokens_and_own_header_inheritance() {
        let target = OriginalProxyTarget::parse("http://px-web:3001").unwrap();
        let inherited = json!([{ "name":"add_header","args":["X-Original","$proxy_host","always"]},{"name":"add_header","args":["X-Cache-Status","$upstream_cache_status","always"]}]);
        let dirs = target
            .adapt_directives(&json!({"path":"/","directives":[]}), &inherited)
            .unwrap();
        let headers = dirs
            .iter()
            .filter(|d| d.name == "add_header")
            .collect::<Vec<_>>();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0].args[1], "$proxy_host");
        assert_eq!(headers[1].args[1], "$upstream_cache_status");
        let dirs = target
            .adapt_directives(
                &json!({"path":"/","directives":[{"name":"add_header","args":["X-Own","own"]}]}),
                &inherited,
            )
            .unwrap();
        assert_eq!(dirs.iter().filter(|d| d.name == "add_header").count(), 1);
    }
}

#[cfg(test)]
mod runtime_dns_native_render_tests {
    use super::*;
    const ID: &str = "11111111-1111-4111-8111-111111111111";
    fn endpoint() -> std::net::SocketAddr {
        "127.0.0.1:5354".parse().unwrap()
    }
    fn config() -> Value {
        json!({"serverId":ID,"serverName":"example.test","listen":9080,"proxyPass":"http://px-web:3001/app/a%2Fb?q=x%2Fy","resolveAtRuntime":true})
    }

    #[test]
    fn direct_native_group_preserves_suffix_cache_authority_and_implicit_headers() {
        let mut cfg = config();
        cfg["cache"] = json!({"enabled":true});
        let text = render_reverse_proxy_with_dns(&cfg, Some(endpoint())).unwrap();
        let name = "nwui_resolve_11111111111141118111111111111111_0";
        assert!(text.starts_with("proxy_cache_path"));
        assert!(text.contains(&format!("upstream {name} {{\n    zone {name} 64k;")));
        assert!(text.contains("resolver 127.0.0.1:5354 valid=10s;"));
        assert!(text.contains("resolver_timeout 2s;"));
        assert!(text.contains("server px-web:3001 resolve;"));
        assert!(text.contains(&format!("proxy_pass http://{name}/app/a%2Fb?q=x%2Fy;")));
        assert!(text.contains("proxy_set_header Host $host;"));
        assert!(text.contains("proxy_cache_key \"$scheme|$host|px-web:3001|$request_uri\";"));
        assert!(text.contains("proxy_redirect http://px-web:3001/app/a%2Fb?q=x%2Fy /;"));
    }
    #[test]
    fn unavailable_guard_affects_only_opted_in_locations_without_any_fallback_lookup() {
        let mut cfg = config();
        cfg["locations"] = json!([
            {"path":"/live/","proxyPass":"http://10.0.0.5:9000"},
            {"path":"/missing/","proxyPass":"http://missing:3001","resolveAtRuntime":true}
        ]);
        let text = render_reverse_proxy_with_dns(&cfg, None).unwrap();
        assert!(text.contains("proxy_pass http://10.0.0.5:9000;"));
        assert!(text.contains("location /missing/ {\n        return 502;"));
        assert!(!text.contains("proxy_pass http://missing"));
        assert!(!text.contains("resolver "));
    }
    #[test]
    fn named_groups_resolve_only_dns_members_and_keep_static_weights() {
        let group = json!({"id":ID,"name":"backend","servers":["10.0.0.5:9000 weight=2","px-web:3001 weight=3"],"resolveAtRuntime":true});
        let text = render_upstream_with_dns(&group, Some(endpoint())).unwrap();
        assert!(text.contains("zone nwui_resolve_upstream_11111111111141118111111111111111 64k;"));
        assert!(text.contains("server 10.0.0.5:9000 weight=2;"));
        assert!(text.contains("server px-web:3001 weight=3 resolve;"));
        let unavailable = render_upstream_with_dns(&group, None).unwrap();
        assert!(unavailable.contains("server 10.0.0.5:9000 weight=2;"));
        assert!(unavailable.contains(" down;"));
        assert!(!unavailable.contains("resolve;"));
        assert!(!unavailable.contains("resolver "));
    }
    #[test]
    fn dynamic_location_materializes_generated_hsts_cache_and_user_headers_in_server_order() {
        let mut cfg = config();
        cfg["tls"] = json!({"certPath":"/tmp/cert.pem","keyPath":"/tmp/key.pem","tlsPort":9443,"redirectHttp":false,"tuning":{"hsts":{"enabled":true,"maxAge":31536000,"includeSubDomains":true}}});
        cfg["cache"] = json!({"enabled":true,"statusHeader":true});
        cfg["serverDirectives"] =
            json!([{"name":"add_header","args":["X-Extra","value","always"]}]);
        let rendered = render_reverse_proxy_with_dns(&cfg, Some(endpoint())).unwrap();
        let location = rendered.split("    location / {").nth(1).unwrap();
        let hsts = location
            .find("add_header Strict-Transport-Security")
            .expect("generated HSTS must survive location header materialization");
        let cache = location.find("add_header X-Cache-Status").unwrap();
        let user = location.find("add_header X-Extra").unwrap();
        assert!(hsts < cache && cache < user, "{location}");
        cfg["locationDirectives"] = json!([{"name":"add_header","args":["X-Own","own","always"]}]);
        let rendered = render_reverse_proxy_with_dns(&cfg, Some(endpoint())).unwrap();
        let location = rendered.split("    location / {").nth(1).unwrap();
        assert!(!location.contains("Strict-Transport-Security"));
        assert!(!location.contains("X-Extra"));
        assert!(location.contains("X-Own"));
    }

    #[test]
    fn stream_groups_belong_to_their_own_file_and_keep_udp_mode() {
        let row = json!({"id":ID,"listen":9200,"protocol":"udp","proxyPass":"px-web:3001","resolveAtRuntime":true});
        let text = render_stream_with_dns(&row, Some(endpoint())).unwrap();
        assert!(
            text.starts_with("upstream nwui_resolve_stream_11111111111141118111111111111111 {")
        );
        assert!(text.contains("server px-web:3001 resolve;"));
        assert!(text.contains("listen 9200 udp;"));
        assert!(text.contains("proxy_pass nwui_resolve_stream_11111111111141118111111111111111;"));
        let unavailable = render_stream_with_dns(&row, None).unwrap();
        assert!(unavailable.contains(" down;"));
        assert!(!unavailable.contains("resolver "));
    }
    #[test]
    fn generated_names_use_full_uuid_and_new_fields_never_change_static_goldens() {
        let a = render_reverse_proxy_with_dns(&config(), Some(endpoint())).unwrap();
        let mut cfg = config();
        cfg["serverId"] = json!("11111111-1111-4111-8111-222222222222");
        let b = render_reverse_proxy_with_dns(&cfg, Some(endpoint())).unwrap();
        assert!(a.contains("nwui_resolve_11111111111141118111111111111111_0"));
        assert!(b.contains("nwui_resolve_11111111111141118111222222222222_0"));
        cfg["resolveAtRuntime"] = json!(false);
        assert_eq!(
            render_reverse_proxy_with_dns(&cfg, Some(endpoint())).unwrap(),
            render_reverse_proxy(&cfg).unwrap()
        );
    }
    #[test]
    fn base_upgrade_map_is_single_and_before_managed_http_includes() {
        let base = include_str!("../../../../docker/nginx.conf");
        assert_eq!(
            base.matches("map $http_upgrade $connection_upgrade")
                .count(),
            1
        );
        assert!(base.contains("default upgrade;\n        '' close;"));
        assert!(
            base.find("map $http_upgrade").unwrap()
                < base.find("include /etc/nginx/conf.d/*.conf;").unwrap()
        );
        assert!(!base.contains("proxy_set_header Connection $http_connection"));
    }
}

#[cfg(test)]
mod runtime_dns_tls_tests {
    use super::*;
    fn config() -> Value {
        json!({"serverId":"11111111-1111-4111-8111-111111111111","serverName":"example.test","listen":9080,"proxyPass":"https://Px-Web:443/app/","resolveAtRuntime":true})
    }
    fn endpoint() -> Option<SocketAddr> {
        Some("127.0.0.1:5354".parse().unwrap())
    }
    #[test]
    fn https_uses_original_tls_name_without_forcing_sni_or_verification() {
        let text = render_reverse_proxy_with_dns(&config(), endpoint()).unwrap();
        assert!(text.contains("proxy_ssl_name Px-Web;"));
        assert!(!text.contains("proxy_ssl_server_name"));
        assert!(!text.contains("proxy_ssl_verify"));
        for enabled in ["on", "off"] {
            let mut cfg = config();
            cfg["locationDirectives"] = json!([
                {"name":"proxy_ssl_server_name","args":[enabled]},
                {"name":"proxy_ssl_verify","args":[enabled]}
            ]);
            let text = render_reverse_proxy_with_dns(&cfg, endpoint()).unwrap();
            assert!(text.contains(&format!("proxy_ssl_server_name {enabled};")));
            assert!(text.contains(&format!("proxy_ssl_verify {enabled};")));
        }
    }
    #[test]
    fn a_managed_tls_name_override_is_rejected_instead_of_silently_replaced() {
        for field in ["locationDirectives", "serverDirectives"] {
            let mut cfg = config();
            cfg[field] = json!([{ "name":"proxy_ssl_name","args":["custom.example"] }]);
            assert!(
                render_reverse_proxy_with_dns(&cfg, endpoint()).is_err(),
                "{field}"
            );
        }
    }
    #[test]
    fn shipped_base_sets_connection_limits_and_hides_the_version_at_the_http_level() {
        use crate::control::guard_targets::{End,statements};
        let (mut blocks,mut found)=(Vec::new(),Vec::new());
        for statement in statements(include_str!("../../../../docker/nginx.conf")) {
            match statement.end {
                End::Open=>blocks.push(statement.words.first().cloned().unwrap_or_default()),
                End::Close=>{blocks.pop();},
                End::Semicolon=>if matches!(statement.words.first().map(String::as_str),Some("worker_rlimit_nofile"|"worker_connections"|"server_tokens")){found.push((blocks.join("/"),statement.words.join(" ")));},
            }
        }
        assert_eq!(found,[(String::new(),"worker_rlimit_nofile 16384".to_owned()),("events".to_owned(),"worker_connections 4096".to_owned()),("http".to_owned(),"server_tokens off".to_owned())]);
    }
    #[test]
    fn shipped_base_has_no_inherited_tls_override_revisit_adapter_if_that_changes() {
        let base = include_str!("../../../../docker/nginx.conf");
        let tls = Regex::new(r"(?m)^\s*proxy_ssl_\w+\s").unwrap();
        assert!(
            !tls.is_match(base),
            "Shipped nginx.conf now sets proxy_ssl_*; revisit the runtime DNS TLS-name inheritance adapter"
        );
    }
}

#[cfg(test)]
mod admin_alias_tests {
    use super::*;
    fn assert_safe_headers(text: &str) {
        assert_eq!(text.matches("proxy_set_header X-Real-IP $remote_addr;").count(), 1, "{text}");
        assert_eq!(text.matches("proxy_set_header X-Forwarded-Proto $scheme;").count(), 1, "{text}");
        assert!(!text.contains("spoof"), "{text}");
    }

    fn config() -> Value {
        json!({"serverName":"admin.test","listen":80,"serverId":"11111111-1111-4111-8111-111111111111","proxyPass":"http://mapped.test:8080"})
    }
    fn stored_headers() -> Value {
        json!([
            {"name":"proxy_set_header","args":["x-real-ip","spoof"]},
            {"name":"proxy_set_header","args":["X-FoRwArDeD-PrOtO","spoof"]},
            {"name":"proxy_set_header","args":["Host","$host"]},
            {"name":"proxy_set_header","args":["X-Forwarded-For","$proxy_add_x_forwarded_for"]},
            {"name":"add_header","args":["X-Real-IP","response"]}
        ])
    }
    #[test]
    fn stored_headers_are_normalized_without_mutating_legacy_or_explicit_locations() {
        let resolve = |_: &str, _| vec!["127.0.0.2".parse().unwrap()];
        for target in ["http://127.0.0.1:8080", "http://localhost:8080", "http://mapped.test:8080"] {
            for explicit in [false, true] {
                let mut cfg = config();
                cfg["proxyPass"] = json!(target);
                if explicit {
                    cfg["locations"] = json!([{"path":"/", "proxyPass":target, "directives":stored_headers()}]);
                } else { cfg["locationDirectives"] = stored_headers(); }
                let before = cfg.clone();
                let text = render_reverse_proxy_with_dns_and_resolve(&cfg, None, &resolve).unwrap();
                assert_safe_headers(&text);
                assert!(text.contains("proxy_set_header Host $http_host;"));
                assert!(text.contains("proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;"));
                assert!(text.contains("add_header X-Real-IP response;"));
                assert_eq!(cfg, before);
            }
        }
    }
    #[test]
    fn all_admin_groups_strip_headers_but_mixed_groups_keep_custom_values() {
        let mut cfg = config();
        cfg["proxyPass"] = json!("http://admin_pool");
        cfg["locationDirectives"] = stored_headers();
        let admin = std::collections::BTreeSet::from(["admin_pool".into()]);
        let never = |_: &str, _| -> Vec<std::net::IpAddr> {panic!("upstream names must not resolve")};
        let text = render_reverse_proxy_context(&cfg, None, &never, &admin, &|_|false).unwrap();
        assert_safe_headers(&text);
        assert!(text.contains("proxy_bind 127.0.0.2;"));
        let text = render_reverse_proxy_context(&cfg, None, &never, &Default::default(), &|_|false).unwrap();
        assert!(text.contains("proxy_set_header x-real-ip spoof;"));
        assert!(text.contains("proxy_set_header X-FoRwArDeD-PrOtO spoof;"));
        assert!(!text.contains("proxy_bind"));
    }
    #[test]
    fn admin_host_defaults_follow_binding_and_preserve_customizations_on_both_tls_ports() {
        let resolve = |host: &str, _| vec![if host == "assigned.test" {"192.0.2.10"} else {"127.0.0.2"}.parse().unwrap()];
        let local = |ip: std::net::IpAddr| ip.is_loopback() || ip == "192.0.2.10".parse::<std::net::IpAddr>().unwrap();
        for target in ["http://127.0.0.1:8080", "http://localhost:8080", "http://mapped.test:8080", "http://assigned.test:8080", "http://admin_pool"] {
            let admin = std::collections::BTreeSet::from(["admin_pool".into()]);
            for shape in 0..3 {
                for host in [None, Some("$host"), Some("$proxy_host"), Some("literal.test"), Some("$http_host"), Some("${host}")] {
                    // The legacy shape has no directive list; all other shapes preserve an omitted Host.
                    if shape == 0 && host.is_some() {continue;}
                    let mut cfg = config();
                    cfg["proxyPass"] = json!(target);
                    if shape != 0 {
                        let mut rows = stored_headers();
                        rows.as_array_mut().unwrap().retain(|r| r["args"][0] != "Host");
                        if let Some(value) = host.filter(|h| !h.is_empty()) {
                            rows.as_array_mut().unwrap().push(json!({"name":"proxy_set_header","args":["hOsT",value]}));
                        }
                        if shape == 1 {cfg["locationDirectives"] = rows;}
                        else {cfg["locations"] = json!([{"path":"/", "proxyPass":target, "directives":rows}]);}
                    }
                    if matches!(target, "http://assigned.test:8080" | "http://admin_pool") {
                        assert_eq!(super::super::validation::validate_server_config_with(&cfg, &resolve).unwrap(), vec![false], "R1 save scope must remain protected only");
                    }
                    let before = cfg.clone();
                    let mut outputs = Vec::new();
                    for port in [443, 4443] {
                        cfg["tls"] = json!({"tlsPort":port,"certPath":"/cert.pem","keyPath":"/key.pem"});
                        let text = render_reverse_proxy_context(&cfg, None, &resolve, &admin, &local).unwrap();
                        assert_safe_headers(&text);
                        assert_eq!(text.matches("proxy_bind 127.0.0.2;").count(), 1);
                        let effective = if shape == 0 || host == Some("$host") {"$http_host"} else if host == Some("$proxy_host") && target != "http://admin_pool" {target.strip_prefix("http://").unwrap()} else if let Some(value) = host {value} else if target == "http://admin_pool" {""} else {target.strip_prefix("http://").unwrap()};
                        let host_lines: Vec<_> = text.lines().filter(|line| line.trim_start().starts_with("proxy_set_header ") && line.split_whitespace().nth(1).is_some_and(|name|name.eq_ignore_ascii_case("Host"))).collect();
                        if effective.is_empty() {assert!(host_lines.is_empty(), "{text}");}
                        else {
                            assert_eq!(host_lines.len(), 1, "{text}");
                            let rendered = if effective == "${host}" {"\"${host}\""} else {effective};
                            assert!(host_lines[0].ends_with(&format!(" {rendered};")), "{text}");
                        }
                        assert!(!text.contains(" Host $host;"));
                        outputs.push(text.replace(&format!("listen {port} ssl;"), "listen PORT ssl;"));
                    }
                    assert_eq!(outputs[0], outputs[1]);
                    cfg.as_object_mut().unwrap().remove("tls");
                    assert_eq!(cfg, before);
                }
            }
        }
    }
    #[test]
    fn non_bound_locations_keep_explicit_and_implicit_host_on_both_tls_ports() {
        let never = |_: &str, _| -> Vec<std::net::IpAddr> {panic!("runtime names and upstream groups must not resolve")};
        for (target, runtime) in [("http://192.0.2.20:9000", false), ("http://mixed_pool", false), ("http://external.test:8080", true)] {
            for rows in [None, Some(json!([{"name":"proxy_set_header","args":["Host","$host"]}]))] {
                for port in [443, 4443] {
                    let mut cfg = config();
                    cfg["proxyPass"] = json!(target);
                    cfg["resolveAtRuntime"] = json!(runtime);
                    cfg["tls"] = json!({"tlsPort":port,"certPath":"/cert.pem","keyPath":"/key.pem"});
                    if let Some(rows) = &rows {cfg["locationDirectives"] = rows.clone();}
                    let text = render_reverse_proxy_context(&cfg, Some("127.0.0.1:5354".parse().unwrap()), &never, &Default::default(), &|_|false).unwrap();
                    assert_eq!(text.matches("proxy_set_header Host $host;").count(), 1, "{text}");
                    assert!(!text.contains("$http_host"));
                    assert!(!text.contains("proxy_bind"));
                }
            }
        }
    }
    #[test]
    fn stored_render_retains_other_validation_failures() {
        let resolve = |_: &str, _| vec!["127.0.0.2".parse().unwrap()];
        for rows in [json!({"invalid":true}), json!([{"name":"proxy_read_timeout","args":["bad"]}])] {
            let mut cfg = config(); cfg["locationDirectives"] = rows;
            assert!(render_reverse_proxy_with_dns_and_resolve(&cfg, None, &resolve).is_err());
        }
        let mut cfg = config(); cfg["resolveAtRuntime"] = json!("true");
        assert!(render_reverse_proxy_with_dns_and_resolve(&cfg, None, &resolve).is_err());
        cfg = config(); cfg["locations"] = json!([{"path":"/","proxyPass":"http://127.0.0.1:8080"},{"path":"/","proxyPass":"http://127.0.0.1:8080"}]);
        assert!(render_reverse_proxy_with_dns_and_resolve(&cfg, None, &resolve).is_err());
    }
    #[test]
    fn snapshot_and_preview_paths_keep_saved_headers_and_no_pending_change() {
        use super::super::{model, resources::Resource, views, pending};
        let config = crate::config::RuntimeConfig::read(|key| match key {
            "DATABASE_URL" => Some("postgres://fixture@127.0.0.1/fixture".into()),
            "AUTH_SECRET" => Some("stored-header-fixture".into()), _ => None,
        }).unwrap();
        let row = json!({"id":"admin", "serverName":"admin.test", "listen":18086, "enabled":true,
            "config":{"proxyPass":"http://127.0.0.1:8080", "locationDirectives":stored_headers()}});
        let data = model::ConfigSnapshotData {servers:vec![row.clone()], ..Default::default()};
        // ApplySaved and restored snapshots share this renderer; boot supplies each certificate root.
        for cert_root in ["/etc/nginx/certs", "/var/lib/nginx-web-ui/publisher/generations/fixture/certs"] {
            let mut config = config.clone(); config.certs_dir = cert_root.into();
            let dns = crate::dns_guard::Descriptor::unavailable(&config.target_dns, "fixture");
            let outcome = super::super::model_render::render_with_context(&config, &data, &[], &super::super::model_render::RenderContext {
                runtime_dns: &dns, resolve: &|_, _| vec![], is_local_addr: &super::super::internal_targets::is_local_addr,
            });
            assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
            assert_safe_headers(&String::from_utf8_lossy(&outcome.rendered.conf["server-admin.conf"]));
            let effective = outcome.effective(&data, None);
            assert_eq!(effective.servers, data.servers);
            assert!(!pending::diff_config(Some(&effective), &data).has_changes);
        }
        let bulk = model::render_preview(&config, &data, &[]).unwrap();
        assert!(bulk.skipped.is_empty());
        assert_safe_headers(&String::from_utf8_lossy(&bulk.rendered.conf["server-admin.conf"]));
        // Local and node GET views use this same preview adapter.
        let preview = views::render_preview(&config, Resource::Server, &row, &[], &[]);
        assert!(preview["error"].is_null(), "{preview}");
        assert_safe_headers(preview["config"].as_str().unwrap());
        let mut unknown = row.clone(); unknown["config"]["proxyPass"] = json!("http://unknown.test:8080");
        let preview = views::render_preview(&config, Resource::Server, &unknown, &[], &[]);
        assert_eq!(preview["unresolvedPaths"], json!(["/"]));
        assert!(preview["config"].as_str().unwrap().contains("proxy_set_header x-real-ip spoof;"));
        assert_eq!(data.servers, vec![row]);
    }
    #[test]
    fn resolver_mapped_alias_forces_headers_and_refuses_both_overrides() {
        let calls = std::cell::Cell::new(0);
        let resolve = |host: &str, port| {
            calls.set(calls.get() + 1);
            assert_eq!((host, port), ("mapped.test", 8080));
            vec!["127.0.0.2".parse().unwrap()]
        };
        let cfg = config();
        let text = render_reverse_proxy_with_dns_and_resolve(&cfg, None, &resolve).unwrap();
        assert_eq!(
            calls.get(),
            1,
            "validation and emission must share the same resolved answer"
        );
        assert!(text.contains("proxy_pass http://127.0.0.1:8080;"));
        assert!(text.contains("proxy_redirect http://mapped.test:8080/ /;"));
        assert!(text.contains("proxy_set_header X-Real-IP $remote_addr;"));
        assert!(text.contains("proxy_set_header X-Forwarded-Proto $scheme;"));
        for header in ["X-Real-IP", "X-Forwarded-Proto"] {
            let mut cfg = cfg.clone();
            cfg["locationDirectives"] =
                json!([{"name":"proxy_set_header","args":[header,"spoof"]}]);
            assert!(super::super::validation::validate_server_config_with(&cfg, &resolve).is_err());
            assert_safe_headers(&render_reverse_proxy_with_dns_and_resolve(&cfg, None, &resolve).unwrap());
        }
    }
    #[test]
    fn external_resolver_answers_preserve_custom_headers() {
        let mut cfg = config();
        cfg["locationDirectives"] = json!([{"name":"proxy_set_header","args":["X-Real-IP","custom"]},{"name":"proxy_set_header","args":["X-Forwarded-Proto","custom"]}]);
        let text = render_reverse_proxy_with_dns_and_resolve(&cfg, None, &|_, _| {
            vec!["192.0.2.1".parse().unwrap()]
        })
        .unwrap();
        assert!(text.contains("proxy_set_header X-Real-IP custom;"));
        assert!(text.contains("proxy_set_header X-Forwarded-Proto custom;"));
        assert!(!text.contains("X-Real-IP $remote_addr"));
        assert!(!text.contains("X-Forwarded-Proto $scheme"));
    }
    #[test]
    fn mixed_locations_keep_their_own_classification() {
        let mut cfg = config();
        cfg["locations"] = json!([{"path":"/admin","proxyPass":"http://mapped.test:8080"},{"path":"/peer","proxyPass":"http://external.test:8080","directives":[{"name":"proxy_set_header","args":["X-Real-IP","custom"]}]}]);
        let resolve = |host: &str, _| {
            vec![
                if host == "mapped.test" {
                    "127.0.0.2"
                } else {
                    "192.0.2.1"
                }
                .parse()
                .unwrap(),
            ]
        };
        let text = render_reverse_proxy_with_dns_and_resolve(&cfg, None, &resolve).unwrap();
        assert_eq!(
            text.matches("proxy_set_header X-Real-IP $remote_addr;")
                .count(),
            1
        );
        assert!(text.contains("proxy_set_header X-Real-IP custom;"));
        cfg["locations"][0]["directives"] =
            json!([{"name":"proxy_set_header","args":["X-Real-IP","spoof"]}]);
        assert_safe_headers(&render_reverse_proxy_with_dns_and_resolve(&cfg, None, &resolve).unwrap());
    }
    #[test]
    fn runtime_dns_names_never_use_nss_for_admin_classification() {
        let mut cfg = config();
        cfg["resolveAtRuntime"] = json!(true);
        let never =
            |_: &str, _| -> Vec<std::net::IpAddr> { panic!("runtime DNS must not call NSS") };
        let text = render_reverse_proxy_with_dns_and_resolve(
            &cfg,
            Some("127.0.0.1:5354".parse().unwrap()),
            &never,
        )
        .unwrap();
        assert!(text.contains("proxy_set_header X-Real-IP $remote_addr;"));
        assert!(text.contains("proxy_set_header X-Forwarded-Proto $scheme;"));
        assert!(text.contains("server mapped.test:8080 resolve;"));
        for header in ["X-Real-IP", "X-Forwarded-Proto"] {
            let mut cfg = cfg.clone();
            cfg["locationDirectives"] =
                json!([{"name":"proxy_set_header","args":[header,"spoof"]}]);
            assert!(super::super::validation::validate_server_config_with(&cfg, &never).is_err());
            let text = render_reverse_proxy_with_dns_and_resolve(
                &cfg, Some("127.0.0.1:5354".parse().unwrap()), &never,
            ).unwrap();
            assert_safe_headers(&text);
            assert!(!text.contains("proxy_bind"));
        }
        cfg["proxyPass"] = json!("http://mapped.test:9000");
        let text = render_reverse_proxy_with_dns_and_resolve(
            &cfg,
            Some("127.0.0.1:5354".parse().unwrap()),
            &never,
        )
        .unwrap();
        assert!(!text.contains("proxy_set_header X-Real-IP $remote_addr;"));
    }
}

#[cfg(test)]
mod seamless_restart_tests {
    use super::*;
    const POLICY:&str="        proxy_next_upstream error timeout http_502 http_503 http_504;\n        proxy_next_upstream_tries 2;\n        proxy_next_upstream_timeout 3s;\n        proxy_connect_timeout 2s;\n";
    fn fixture(kind:u8)->Value{
        let mut location=json!({"path":"/api","proxyPass":"http://127.0.0.1:9000/path/"});
        if kind==1{location["proxyPass"]=json!("http://app.test:9000/path/");location["resolveAtRuntime"]=json!(true);}
        if kind==2{location=json!({"path":"/api","upstreamId":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"});}
        let cfg=json!({"serverId":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","serverName":"example.test","listen":8081,"locations":[location]});
        super::super::location_refs::resolve_locations(&cfg,&std::collections::BTreeMap::from([("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(),"backend".into())])).unwrap()
    }
    fn rendered(cfg:&Value)->String{render_reverse_proxy_with_dns_and_resolve(cfg,Some("127.0.0.1:5354".parse().unwrap()),&|_,_|vec![]).unwrap()}
    #[test]
    fn sr_sa_off_bytes_and_exactly_one_retry_policy_per_route(){
        use sha2::{Digest,Sha256};
        let hashes=["cb804a94bcb21035812e51500970362649d33845820e02944e9cf21afdb16077","177d0b871443375067c019f094c0bf0499a771dd87d727a01e1b546fb4b61780","4c37c09d551d3cb87bbcc7f7e23c2faaaf8e7134e7799cb71823eac211aad544"];
        for kind in 0..3{let cfg=fixture(kind);let absent=rendered(&cfg);assert_eq!(hex::encode(Sha256::digest(absent.as_bytes())),hashes[kind as usize]);let mut off=cfg.clone();off["locations"][0]["retryOtherInstances"]=json!(false);assert_eq!(rendered(&off),absent);}
        for kind in 0..3{let mut cfg=fixture(kind);let absent=rendered(&cfg);cfg["locations"][0]["retryOtherInstances"]=json!(true);let on=rendered(&cfg);assert_eq!(on.matches(POLICY).count(),1,"{on}");assert_eq!(on.replace(POLICY,""),absent);assert!(!on.contains("non_idempotent"));}
    }
    #[test]
    fn sr_sa_route_connect_wins_server_connect_does_not_and_placement_is_stable(){
        for kind in 0..3{let mut cfg=fixture(kind);cfg["locations"][0]["retryOtherInstances"]=json!(true);cfg["serverDirectives"]=json!([{"name":"proxy_connect_timeout","args":["9s"]}]);let text=rendered(&cfg);assert_eq!(text.matches("proxy_connect_timeout 2s;").count(),1);assert!(text.contains("proxy_connect_timeout 9s;"));
            cfg["locations"][0]["directives"]=json!([{"name":"proxy_connect_timeout","args":["4s"]}]);let text=rendered(&cfg);assert_eq!(text.matches("proxy_next_upstream_tries 2;").count(),1);assert_eq!(text.matches("proxy_connect_timeout 4s;").count(),1);assert!(!text.contains("proxy_connect_timeout 2s;"));
        }
        let location=json!({"path":"/","proxyPass":"http://127.0.0.1:9000","retryOtherInstances":true,"ipAccess":{"mode":"whitelist","entries":["10.0.0.0/8"]}});
        let text=render_location(&location,vec!["        proxy_cache zone;".into()]).unwrap();assert!(text.find("proxy_set_header Host").unwrap()<text.find("proxy_next_upstream error").unwrap());assert!(text.find("proxy_connect_timeout 2s").unwrap()<text.find("allow 10.0.0.0/8").unwrap());assert!(text.find("deny all").unwrap()<text.find("proxy_cache zone").unwrap());
    }
    #[test]
    fn sr_sa_runtime_retry_group_keeps_the_guard_scanner_shape(){
        let root=tempfile::tempdir().unwrap();let mut config=crate::config::RuntimeConfig::read(|key|match key{"DATABASE_URL"=>Some("postgres://test@127.0.0.1/test".into()),"AUTH_SECRET"=>Some("sr-test-only".into()),"NWUI_CONTAINER"=>Some("1".into()),_=>None}).unwrap();config.target_dns.descriptor_path=root.path().join("dns.json");std::fs::write(&config.target_dns.descriptor_path,serde_json::to_vec(&crate::dns_guard::Descriptor::available(&config.target_dns)).unwrap()).unwrap();
        let mut cfg=fixture(1);cfg["locations"][0]["retryOtherInstances"]=json!(true);let conf=std::collections::BTreeMap::from([("server-aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa.conf".into(),rendered(&cfg).into_bytes())]);assert!(super::super::guard_targets::scan(&config,&conf,&std::collections::BTreeMap::new(),&|_,_|vec![]).is_empty());
    }
    #[test]
    #[ignore="native nginx 1.30.6: NWUI_SR_NATIVE_NGINX=1"]
    fn sr_sa_rendered_retry_fixtures_pass_real_nginx_t(){
        if std::env::var("NWUI_SR_NATIVE_NGINX").as_deref()!=Ok("1"){return;}
        use std::{fs,process::Command};
        let version=Command::new("/usr/sbin/nginx").arg("-v").output().unwrap();assert!(String::from_utf8_lossy(&version.stderr).contains("1.30.6"));
        fs::create_dir_all("/etc/nginx/snippets").unwrap();fs::write("/etc/nginx/snippets/acme-challenge.conf","# isolated syntax-test include\n").unwrap();let root=tempfile::tempdir().unwrap();let mut count=0;
        for kind in 0..3{for retry in [false,true]{for route_connect in [false,true]{
            let mut cfg=fixture(kind);cfg["locations"][0]["retryOtherInstances"]=json!(retry);cfg["serverDirectives"]=json!([{"name":"proxy_connect_timeout","args":["9s"]}]);if route_connect{cfg["locations"][0]["directives"]=json!([{"name":"proxy_connect_timeout","args":["4s"]}]);}
            let path=root.path().join("nginx.conf");fs::write(&path,format!("pid {}/nginx.pid;\nerror_log {}/error.log;\nevents {{}}\nhttp {{ access_log off; upstream backend {{ server 127.0.0.1:9000; }}\n{}\n}}",root.path().display(),root.path().display(),rendered(&cfg))).unwrap();let output=Command::new("/usr/sbin/nginx").arg("-t").arg("-p").arg(root.path()).arg("-c").arg(&path).output().unwrap();assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));count+=1;
        }}}
        println!("real nginx -t: {count} retry/target/connect fixtures passed");assert_eq!(count,12);
    }

}
