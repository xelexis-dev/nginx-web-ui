use super::reserved_ports::{ListenContext, ReservedPorts, find_reserved_port_conflict};
use super::{Directive, Result, whitespace};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::net::IpAddr;
use std::sync::LazyLock;

static HOST_PORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A([A-Za-z0-9.-]+):([0-9]{1,5})\z").unwrap());
static HOST_WEIGHT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\A([A-Za-z0-9.-]+):([0-9]{1,5})(?: weight=([0-9]+))?\z").unwrap()
});

pub(crate) fn injection(value: &str) -> bool {
    value.ends_with('\\')
        || value
            .chars()
            .any(|c| whitespace(c) || ";{}#\"'".contains(c))
}

// Save-only bounds leave previously valid stored configurations renderable.
const SERVER_NAME_TOKEN_BYTES:usize=253;
pub(crate) const SERVER_NAMES_BYTES:usize=4096;
pub(crate) const CONFIG_PARAMETER_BYTES:usize=2048;

pub(crate) fn assert_server_name_save_size(value:&str)->Result<()> {
    if value.len()>SERVER_NAMES_BYTES {
        return Err(format!("server_name 전체는 {SERVER_NAMES_BYTES}바이트 이하여야 합니다.").into());
    }
    if value.split_whitespace().any(|name|name.len()>SERVER_NAME_TOKEN_BYTES) {
        return Err(format!("server_name의 각 이름은 {SERVER_NAME_TOKEN_BYTES}자 이하여야 합니다.").into());
    }
    Ok(())
}
pub(crate) fn assert_parameter_save_size(bytes:usize)->Result<()> {
    // nginx uses a 4096-byte configuration token buffer; reserve quoting headroom.
    if bytes>CONFIG_PARAMETER_BYTES {
        return Err(format!("nginx 설정 값은 {CONFIG_PARAMETER_BYTES}바이트 이하여야 합니다.").into());
    }
    Ok(())
}
pub(crate) fn assert_directive_save_sizes(name:&str,args:&[String])->Result<()> {
    for arg in args {assert_parameter_save_size(arg.len())?;}
    // The default 64-byte proxy_headers_hash bucket fits at most 46 name bytes.
    if matches!(name,"proxy_set_header"|"proxy_hide_header") && args.first().is_some_and(|arg|arg.len()>46) {
        return Err("프록시 헤더 이름은 46바이트 이하여야 합니다.".into());
    }
    Ok(())
}

pub fn assert_valid_server_name(value: &str) -> Result<()> {
    if value.is_empty() {
        return Err("server_name must not be empty".into());
    }
    if value.contains(['\r', '\n']) {
        return Err(format!("Invalid server_name (newlines are not allowed): \"{value}\"").into());
    }
    let tokens: Vec<_> = value.split(' ').filter(|s| !s.is_empty()).collect();
    if tokens.is_empty() {
        return Err("server_name must not be empty".into());
    }
    for token in tokens {
        if !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._*-".contains(&b))
        {
            return Err(format!("Invalid server_name: \"{token}\"").into());
        }
    }
    Ok(())
}

pub fn assert_valid_listen_port(
    port: f64,
    context: ListenContext,
    reserved: &ReservedPorts,
) -> Result<()> {
    if !port.is_finite() || port.fract() != 0.0 || !(1.0..=65535.0).contains(&port) {
        let value = if port == f64::INFINITY {
            "Infinity".into()
        } else if port == f64::NEG_INFINITY {
            "-Infinity".into()
        } else {
            port.to_string()
        };
        return Err(format!("Invalid listen port (must be an integer 1-65535): {value}").into());
    }
    if let Some(e) = find_reserved_port_conflict(port as u16, context, reserved) {
        return Err(e.into());
    }
    Ok(())
}

pub fn assert_valid_proxy_target(value: &str) -> Result<()> {
    if value.contains('\\') || value.chars().any(|c| c.is_ascii_control()) {
        return Err(format!("Invalid proxy_pass target: \"{value}\"").into());
    }
    assert_valid_stored_proxy_target(value)?;
    assert_no_internal_socket(value)
}

pub(crate) fn assert_valid_stored_proxy_target(value: &str) -> Result<()> {
    if value.is_empty() {
        return Err("proxy_pass target must not be empty".into());
    }
    if value.chars().any(|c| whitespace(c) || ";{}#\"'".contains(c)) {
        return Err(format!("Invalid proxy_pass target: \"{value}\"").into());
    }
    let url = url::Url::parse(value).map_err(|_| {
        super::ControlError::from(format!(
            "Invalid proxy_pass target (must be a URL like \"http://host:port/path\"): \"{value}\""
        ))
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "Invalid proxy_pass target (must be http:// or https://): \"{value}\""
        )
        .into());
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(format!("Invalid proxy_pass target (missing host): \"{value}\"").into());
    }
    Ok(())
}

pub(crate) fn assert_no_internal_socket(value:&str)->Result<()> {
    if let Some(path)=super::internal_socket::http_socket_path(value) {
        if path.as_os_str().as_encoded_bytes().contains(&b'$') {
            return Err("unix 소켓 경로에는 변수를 쓸 수 없습니다. 고정된 경로를 지정하세요.".into());
        }
        let roots=[std::path::Path::new(crate::db_upgrade::paths::PGROOT),std::path::Path::new(crate::db_upgrade::paths::APPROOT)];
        if let Some(prefix)=super::internal_socket::matches_lexical(path,&roots) {
            return Err(format!("이 컨테이너 내부 소켓은 프록시 대상으로 쓸 수 없습니다. ({})",prefix.display()).into());
        }
    }
    Ok(())
}

pub(crate) fn assert_valid_runtime_proxy_target(value:&str)->Result<()> {
    if super::internal_socket::is_unix_target(value) {return Err("unix 소켓 대상에는 실행 중 이름 확인을 쓸 수 없습니다.".into());}
    super::render::OriginalProxyTarget::parse(value).map(|_|())
}

/// New fields have a strict wire type; historical omission remains static.
pub fn parse_runtime_dns_flag(raw: &Value) -> Result<bool> {
    match raw.get("resolveAtRuntime") {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err("실행 중 이름 확인 값은 true 또는 false여야 합니다.".into()),
    }
}

pub fn assert_valid_runtime_host_port(value: &str, allow_literal: bool) -> Result<()> {
    if allow_literal {
        assert_valid_upstream_server(value)?;
    } else {
        assert_valid_stream_proxy_pass(value)?;
    }
    if super::internal_socket::is_unix_target(value) {
        return Err("unix 소켓 대상에는 실행 중 이름 확인을 쓸 수 없습니다.".into());
    }
    let endpoint = value.split_ascii_whitespace().next().unwrap_or("");
    let host = endpoint
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or("");
    if host.parse::<IpAddr>().is_ok() {
        return if allow_literal {
            Ok(())
        } else {
            Err("IP 주소에는 실행 중 이름 확인을 켤 수 없습니다.".into())
        };
    }
    // URL parsing detects ambiguous numeric spellings without normalizing saved data.
    super::render::OriginalProxyTarget::parse(&format!("http://{endpoint}"))?;
    Ok(())
}

fn validate_host_port(value: &str, upstream: bool) -> Result<()> {
    let label = if upstream {
        "upstream server"
    } else {
        "stream proxy_pass"
    };
    if value.is_empty() {
        return Err(format!("{label} must not be empty").into());
    }
    let pattern = if upstream { &*HOST_WEIGHT } else { &*HOST_PORT };
    let Some(caps) = pattern.captures(value) else {
        let expected = if upstream {
            "\"host:port\" or \"host:port weight=N\""
        } else {
            "\"host:port\""
        };
        return Err(format!("Invalid {label} (expected {expected}): \"{value}\"").into());
    };
    if caps[2].parse::<u16>().ok().is_none_or(|p| p == 0) {
        return Err(format!(
            "Invalid {label} (expected \"host:port\" with port 1-65535): \"{value}\""
        )
        .into());
    }
    Ok(())
}
pub fn assert_valid_stream_proxy_pass(value: &str) -> Result<()> {
    validate_host_port(value, false)
}
pub fn assert_valid_upstream_server(value: &str) -> Result<()> {
    validate_host_port(value, true)
}
pub fn assert_valid_upstream_name(value: &str) -> Result<()> {
    if super::internal_targets::RUNTIME_UPSTREAMS
        .iter()
        .any(|(name, _)| value.eq_ignore_ascii_case(name))
    {
        return Err("nwui_api·nwui_web 은 내부 upstream 이름으로 예약되어 있습니다.".into());
    }
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(format!(
            "Invalid upstream name (letters, digits, underscore only): \"{value}\""
        )
        .into());
    }
    Ok(())
}
pub fn assert_valid_location_path(value: &str) -> Result<()> {
    if value == "/.well-known/acme-challenge/" {
        return Err("ACME 인증 경로는 예약되어 있습니다.".into());
    }
    if !value.starts_with('/') {
        return Err(format!("Invalid location path (must start with \"/\"): \"{value}\"").into());
    }
    if injection(value) {
        return Err(format!("Invalid location path: \"{value}\"").into());
    }
    Ok(())
}

pub fn assert_valid_ip_or_cidr(value: &str) -> Result<()> {
    if value.is_empty() {
        return Err("IP/CIDR must not be empty".into());
    }
    if value != value.trim_matches(whitespace) {
        return Err(format!("Invalid IP/CIDR (surrounding whitespace): \"{value}\"").into());
    }
    if value.contains('%') {
        return Err(format!("Invalid IP/CIDR (zone id not allowed): \"{value}\"").into());
    }
    if value.matches('/').count() > 1 {
        return Err(format!("Invalid CIDR (too many \"/\"): \"{value}\"").into());
    }
    if let Some((address, prefix)) = value.split_once('/') {
        let address: IpAddr = address
            .parse()
            .map_err(|_| super::ControlError::from(format!("Invalid CIDR address: \"{value}\"")))?;
        if prefix.is_empty() || prefix.len() > 3 || !prefix.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("Invalid CIDR prefix: \"{value}\"").into());
        }
        let max = if address.is_ipv4() { 32 } else { 128 };
        if prefix.parse::<u16>().unwrap() > max {
            return Err(format!("Invalid CIDR prefix (0-{max}): \"{value}\"").into());
        }
    } else if value.parse::<IpAddr>().is_err() {
        return Err(format!("Invalid IP address: \"{value}\"").into());
    }
    Ok(())
}

pub const REAL_IP_HEADERS: [&str; 3] = ["CF-Connecting-IP", "X-Forwarded-For", "X-Real-IP"];
pub const DEFAULT_REAL_IP_HEADER: &str = "X-Forwarded-For";

pub fn parse_real_ip_config(input: &Value) -> Result<Option<Value>> {
    if input.is_null() || input.as_object().is_some_and(|value|value.is_empty()) || input["preset"]=="none" { return Ok(None); }
    if !input.is_object() || !matches!(input["preset"].as_str(),Some("cloudflare"|"custom")) {
        return Err("실 IP 설정 형식이 올바르지 않습니다.".into());
    }
    let header=match input.get("header") {
        None => DEFAULT_REAL_IP_HEADER,
        Some(value) => value.as_str().filter(|s|REAL_IP_HEADERS.contains(s))
            .ok_or("실 IP 헤더 형식이 올바르지 않습니다.")?,
    };
    if input.get("recursive").is_some_and(|v|!v.is_boolean()) {
        return Err("실 IP 재귀 설정은 true 또는 false여야 합니다.".into());
    }
    if input["preset"] == "cloudflare" { return Ok(Some(json!({"preset":"cloudflare"}))); }
    let values=input["trustedRanges"].as_array().ok_or("실 IP 설정 형식이 올바르지 않습니다.")?;
    let mut ranges=Vec::new();
    for value in values {
        let range=value.as_str().ok_or("실 IP 설정 형식이 올바르지 않습니다.")?.trim_matches(whitespace);
        if range.is_empty() { continue; }
        assert_valid_ip_or_cidr(range)?;
        ranges.push(range.to_owned());
    }
    if ranges.is_empty() { return Err("실 IP 신뢰 대역을 최소 1개 입력하세요.".into()); }
    let mut out=json!({"preset":"custom","trustedRanges":ranges,"header":header});
    if input["recursive"] == true { out["recursive"] = json!(true); }
    Ok(Some(out))
}

// Stored 1.19 values keep their released normalization; saves validate changed input above.
pub(crate) fn parse_stored_real_ip_config(input: &Value) -> Result<Option<Value>> {
    if input["preset"] == "cloudflare" {
        return Ok(Some(json!({"preset":"cloudflare"})));
    }
    if input["preset"] != "custom" {
        return Ok(None);
    }
    let ranges: Vec<String> = input["trustedRanges"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| super::input_string(v).trim_matches(whitespace).to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    if ranges.is_empty() {
        return Err("실 IP 신뢰 대역을 최소 1개 입력하세요.".into());
    }
    for r in &ranges {
        assert_valid_ip_or_cidr(r)?;
    }
    let header = input["header"]
        .as_str()
        .filter(|s| REAL_IP_HEADERS.contains(s))
        .unwrap_or(DEFAULT_REAL_IP_HEADER);
    let mut out = json!({"preset":"custom", "trustedRanges":ranges, "header":header});
    if input["recursive"] == true {
        out["recursive"] = json!(true);
    }
    Ok(Some(out))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DirectiveContext {
    Server,
    Location,
}
impl DirectiveContext {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Location => "location",
        }
    }
}
pub const LOCATION_DIRECTIVES: &[&str] = &[
    "proxy_set_header",
    "add_header",
    "proxy_read_timeout",
    "proxy_send_timeout",
    "proxy_connect_timeout",
    "proxy_http_version",
    "proxy_buffering",
    "client_max_body_size",
    "rewrite",
    "return",
    "proxy_redirect",
    "proxy_hide_header",
    "proxy_pass_header",
    "expires",
];
pub const SERVER_DIRECTIVES: &[&str] = &[
    "add_header",
    "client_max_body_size",
    "server_tokens",
    "proxy_read_timeout",
    "proxy_send_timeout",
    "proxy_connect_timeout",
    "keepalive_timeout",
    "gzip",
];
pub const HEADER_DIRECTIVES: &[&str] = &["proxy_set_header", "add_header"];

pub fn assert_valid_directive(
    name: &str,
    args: &[String],
    context: DirectiveContext,
) -> Result<()> {
    if name.contains('\0') || args.iter().any(|value|value.contains('\0')) {
        return Err("NUL 문자는 사용할 수 없습니다.".into());
    }
    if name.is_empty() {
        return Err("directive name must not be empty".into());
    }
    let allowed = match context {
        DirectiveContext::Server => SERVER_DIRECTIVES,
        DirectiveContext::Location => LOCATION_DIRECTIVES,
    };
    if !allowed.contains(&name) {
        let hint = if matches!(name, "resolver" | "resolver_timeout") {
            " resolver는 직접 설정할 수 없습니다. 대상 주소가 바뀌는 경우 「실행 중에 이름 확인」을 켜고, DNS 서버는 NWUI_TARGET_DNS_SERVERS로 지정하세요."
        } else {
            ""
        };
        return Err(format!(
            "Directive not allowed in {} context: \"{name}\"{hint}",
            context.as_str()
        )
        .into());
    }
    if args.is_empty() {
        return Err(format!("Directive \"{name}\" needs at least one argument").into());
    }
    let plain = if HEADER_DIRECTIVES.contains(&name) {
        let header = &args[0];
        if header.is_empty() || injection(header) {
            return Err(format!("Invalid header name for \"{name}\": \"{header}\"").into());
        }
        if args.len() < 2 {
            return Err(format!("Directive \"{name}\" needs a value").into());
        }
        let value = &args[1];
        if value.contains(['"', '\\', '\r', '\n']) {
            return Err(
                format!("Invalid header value (no quotes or newlines): \"{value}\"").into(),
            );
        }
        &args[2..]
    } else {
        args
    };
    if args.iter().any(|a| a.ends_with('\\')) {
        return Err("인수 끝의 역슬래시는 사용할 수 없습니다.".into());
    }
    for t in plain {
        if t.is_empty() || injection(t) {
            return Err(format!("Invalid argument for \"{name}\": \"{t}\"").into());
        }
    }
    assert_directive_value(name, args)
}

fn assert_directive_value(name: &str, args: &[String]) -> Result<()> {
    if args.iter().any(|a| a.ends_with('\\')) {
        return Err("인수 끝의 역슬래시는 사용할 수 없습니다.".into());
    }
    let valid_value = match name {
        "client_max_body_size" => args.len() == 1 && nginx_size(&args[0]),
        "proxy_read_timeout" | "proxy_send_timeout" | "proxy_connect_timeout" => {
            args.len() == 1 && nginx_time(&args[0], false)
        }
        "keepalive_timeout" => {
            (1..=2).contains(&args.len())
                && nginx_time(&args[0], false)
                && args.get(1).is_none_or(|v| nginx_time(v, true))
        }
        "proxy_buffering" | "gzip" => {
            args.len() == 1
                && ["on", "off"]
                    .iter()
                    .any(|v| args[0].eq_ignore_ascii_case(v))
        }
        "server_tokens" => {
            args.len() == 1
                && ["on", "off", "build"]
                    .iter()
                    .any(|v| args[0].eq_ignore_ascii_case(v))
        }
        "proxy_http_version" => args.len() == 1 && matches!(args[0].as_str(), "1.0" | "1.1" | "2"),
        "return" => {
            (1..=2).contains(&args.len())
                && args.first().is_some_and(|value| {
                    (!value.is_empty()
                        && value.bytes().all(|c| c.is_ascii_digit())
                        && value.parse::<u64>().is_ok_and(|n| n <= 999))
                        || (args.len() == 1
                            && ["http://", "https://", "$scheme"]
                                .iter()
                                .any(|prefix| value.starts_with(prefix)))
                })
        }
        "expires" => valid_expires(args),
        _ => true,
    };
    if !valid_value {
        return Err(format!("Invalid value for directive \"{name}\"").into());
    }
    Ok(())
}

fn tokenize(line: &str) -> Result<Vec<String>> {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut tokens = Vec::new();
    while i < chars.len() {
        while i < chars.len() && whitespace(chars[i]) {
            i += 1;
        }
        if i == chars.len() {
            break;
        }
        let c = chars[i];
        match c {
            '#' => {
                return Err(format!(
                    "Comments (#) are not allowed in a directive line: \"{line}\""
                )
                .into());
            }
            '{' | '}' => {
                return Err(format!(
                    "Blocks ({{ }}) are not allowed in a directive line: \"{line}\""
                )
                .into());
            }
            ';' => {
                if chars[i + 1..].iter().any(|c| !whitespace(*c)) {
                    return Err(format!(
                        "Only one directive per line (unexpected \";\"): \"{line}\""
                    )
                    .into());
                }
                break;
            }
            '"' | '\'' => {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != c {
                    i += 1;
                }
                if i == chars.len() {
                    return Err(format!("Unterminated quoted string: \"{line}\"").into());
                }
                tokens.push(chars[start..i].iter().collect());
                i += 1;
            }
            _ => {
                let start = i;
                while i < chars.len() && !whitespace(chars[i]) && !";{}#\"'".contains(chars[i]) {
                    i += 1;
                }
                tokens.push(chars[start..i].iter().collect());
            }
        }
    }
    Ok(tokens)
}

pub fn parse_freeform_directives(text: &str, context: DirectiveContext) -> Result<Vec<Directive>> {
    let mut out = Vec::new();
    for raw in text.split('\n') {
        let line = raw.trim_matches(whitespace);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut tokens = tokenize(line)?;
        if tokens.is_empty() {
            continue;
        }
        let name = tokens.remove(0);
        assert_valid_directive(&name, &tokens, context)?;
        out.push(Directive { name, args: tokens });
    }
    Ok(out)
}

/// Proxy variables do not exist for every response path (for example `return`).
/// Keep response-filter semantics exact by refusing this opt-in combination.
pub(crate) fn assert_runtime_response_directives(cfg: &Value) -> Result<()> {
    static TOKENS: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(
            r"\$(?:\{(?:proxy_host|proxy_port)\}|(?:proxy_host|proxy_port)(?:[^A-Za-z0-9_]|$))",
        )
        .unwrap()
    });
    let reject = |rows: &Value| -> Result<()> {
        for row in rows.as_array().into_iter().flatten() {
            let values = match row["name"].as_str() {
                Some("add_header") => row["args"]
                    .as_array()
                    .and_then(|a| a.get(1))
                    .into_iter()
                    .collect::<Vec<_>>(),
                Some("expires") => row["args"].as_array().into_iter().flatten().collect(),
                _ => vec![],
            };
            if values
                .into_iter()
                .filter_map(Value::as_str)
                .any(|v| TOKENS.is_match(v))
            {
                return Err("실행 중 이름 확인을 켠 대상에서는 add_header·expires 값에 $proxy_host·$proxy_port 를 쓸 수 없습니다. 고정 값이나 다른 변수를 사용하세요.".into());
            }
        }
        Ok(())
    };
    let mut dynamic = parse_runtime_dns_flag(cfg)?;
    if dynamic {
        reject(&cfg["locationDirectives"])?;
    }
    for location in cfg["locations"].as_array().into_iter().flatten() {
        if parse_runtime_dns_flag(location)? {
            dynamic = true;
            reject(&location["directives"])?;
        }
    }
    if dynamic {
        reject(&cfg["serverDirectives"])?;
    }
    Ok(())
}

fn nginx_size(value: &str) -> bool {
    let (digits, scale) = match value.as_bytes().last() {
        Some(b'k' | b'K') => (&value[..value.len() - 1], 1024u64),
        Some(b'm' | b'M') => (&value[..value.len() - 1], 1024u64.pow(2)),
        Some(b'g' | b'G') => (&value[..value.len() - 1], 1024u64.pow(3)),
        _ => (value, 1),
    };
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && digits
            .parse::<u64>()
            .ok()
            .and_then(|n| n.checked_mul(scale))
            .is_some_and(|n| n <= i64::MAX as u64)
}
fn nginx_time(value: &str, seconds: bool) -> bool {
    nginx_time_value(value, seconds).is_some()
}
fn nginx_time_value(value: &str, seconds: bool) -> Option<u64> {
    // Match ngx_parse_time's unit order and checked arithmetic on the supported 64-bit images.
    let bytes = value.as_bytes();
    let mut i = 0;
    let mut rank = if seconds { 0 } else { 2 };
    let mut number = 0u64;
    let mut total = 0u64;
    let mut valid = false;
    while i < bytes.len() {
        let byte = bytes[i];
        i += 1;
        if byte.is_ascii_digit() {
            number = number
                .checked_mul(10)?
                .checked_add(u64::from(byte - b'0'))?;
            if number > i64::MAX as u64 {
                return None;
            }
            valid = true;
            continue;
        }
        let (next, scale) = match byte {
            b'y' => (1, 31536000),
            b'M' => (2, 2592000),
            b'w' => (3, 604800),
            b'd' => (4, 86400),
            b'h' => (5, 3600),
            b'm' if bytes.get(i) == Some(&b's') => {
                i += 1;
                (8, 1)
            }
            b'm' => (6, 60),
            b's' => (7, 1),
            _ => return None,
        };
        if next <= rank || seconds && next == 8 {
            return None;
        }
        rank = next;
        let scale = if !seconds && next != 8 {
            scale * 1000u64
        } else {
            scale
        };
        total = total.checked_add(number.checked_mul(scale)?)?;
        number = 0;
        if total > i64::MAX as u64 {
            return None;
        }
    }
    total = total.checked_add(number.checked_mul(if seconds { 1 } else { 1000 })?)?;
    (valid && total <= i64::MAX as u64).then_some(total)
}

fn valid_expires(args: &[String]) -> bool {
    let modified = args.first().is_some_and(|v| v == "modified");
    let args = if modified { &args[1..] } else { args };
    if args.len() != 1 {
        return false;
    }
    let value = &args[0];
    if matches!(value.as_str(), "off" | "epoch" | "max") {
        return !modified;
    }
    if value.contains('$') {
        return true;
    }
    if let Some(time) = value.strip_prefix('@') {
        return !modified && nginx_time_value(time, true).is_some_and(|seconds| seconds < 86400);
    }
    nginx_time(value.strip_prefix(['+', '-']).unwrap_or(value), true)
}

pub(crate) fn assert_valid_http_port(port: f64, tls: bool, reserved: &ReservedPorts) -> Result<()> {
    assert_valid_listen_port(port, ListenContext::Http, reserved)?;
    let port = port as u16;
    if tls && (port == 80 || reserved.http_plain.contains(&port))
        || !tls && (port == 443 || reserved.http_tls.contains(&port))
    {
        return Err(format!(
            "포트 {port} 는 다른 HTTP·HTTPS 방식의 리스너가 사용합니다. 다른 포트를 쓰세요."
        )
        .into());
    }
    Ok(())
}

pub(crate) fn assert_directive_constraints(rows: &[Directive]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for row in rows {
        assert_directive_value(&row.name, &row.args)?;
        if matches!(
            row.name.as_str(),
            "client_max_body_size"
                | "proxy_read_timeout"
                | "proxy_send_timeout"
                | "proxy_connect_timeout"
                | "keepalive_timeout"
                | "proxy_http_version"
                | "proxy_buffering"
                | "server_tokens"
                | "gzip"
                | "expires"
        ) && !seen.insert(&row.name)
        {
            return Err(format!("중복 디렉티브: {}", row.name).into());
        }
    }
    Ok(())
}
pub(crate) fn assert_valid_server_config(cfg: &Value) -> Result<()> {
    validate_server_config_with(cfg, &super::internal_targets::system_resolve).map(|_| ())
}
// Return the decisions so header validation and emission share one resolution per location.
pub(crate) fn validate_server_config_with(
    cfg: &Value,
    resolve: super::internal_targets::Resolve,
) -> Result<Vec<bool>> {
    server_config(cfg, resolve, true)
}

/// Stored rows keep all validation; rendering replaces automatic admin headers.
pub(crate) fn validate_stored_server_config_with(
    cfg: &Value,
    resolve: super::internal_targets::Resolve,
) -> Result<Vec<bool>> {
    server_config(cfg, resolve, false)
}

fn server_config(
    cfg: &Value,
    resolve: super::internal_targets::Resolve,
    refuse_admin_headers: bool,
) -> Result<Vec<bool>> {
    let validate = |raw: &Value| -> Result<Vec<Directive>> {
        let rows: Vec<Directive> = if raw.is_null() {
            vec![]
        } else {
            serde_json::from_value(raw.clone()).map_err(|_| "Invalid directives")?
        };
        // Save/apply own lexical and allowlist checks; keep the pure renderer's existing input domain.
        assert_directive_constraints(&rows)?;
        Ok(rows)
    };
    validate(&cfg["serverDirectives"])?;
    let check = |location: &Value| -> Result<bool> {
        let rows = validate(&location["directives"])?;
        let protected = super::self_proxy::requires_admin_headers(location, resolve)?;
        if refuse_admin_headers && protected
            && rows.iter().any(|d| {
                d.name == "proxy_set_header"
                    && d.args.first().is_some_and(|name| {
                        (name.eq_ignore_ascii_case("X-Real-IP") && (d.args.len() != 2 || d.args[1] != "$remote_addr"))
                            || (name.eq_ignore_ascii_case("X-Forwarded-Proto") && (d.args.len() != 2 || d.args[1] != "$scheme"))
                    })
            })
        {
            return Err("관리 화면 프록시의 X-Real-IP·X-Forwarded-Proto 헤더는 자동 설정되며 변경할 수 없습니다.".into());
        }
        Ok(protected)
    };
    let mut paths = std::collections::HashSet::new();
    let mut protected = Vec::new();
    for location in super::render::effective_locations(cfg) {
        let path = location["path"].as_str().ok_or("Invalid location path")?;
        assert_valid_location_path(path)?;
        if !paths.insert(path.to_owned()) {
            return Err(format!("중복 location 경로: {path}").into());
        }
        protected.push(check(&location)?);
    }
    Ok(protected)
}

#[cfg(test)]
mod q23_tests {
    use super::*;
    #[test]
    fn q23_legacy_http_and_numeric_unix_shapes_are_accepted() {
        for target in ["http://unix:/var/run/app.sock:/api/","http://unix:/tmp/x.sock","https://unix:/tmp/tls.sock:/","http://unix:8080"] {
            assert!(assert_valid_proxy_target(target).is_ok(),"{target}");
            assert!(assert_valid_stored_proxy_target(target).is_ok(),"stored {target}");
        }
        for target in ["unix:1","unix:8080","unix:65535"] {assert!(assert_valid_upstream_server(target).is_ok());assert!(assert_valid_stream_proxy_pass(target).is_ok());}
        assert!(assert_valid_upstream_server("unix:8080 weight=2").is_ok());
        for target in ["unix:/tmp/app.sock","unix:0","unix:65536","unix:123456"] {assert!(assert_valid_upstream_server(target).is_err());assert!(assert_valid_stream_proxy_pass(target).is_err());}
    }
    #[test]
    fn q23_save_refusal_names_each_internal_prefix_but_stored_validator_keeps_it() {
        for path in ["/run/nwui-publisher/control.sock","/run/nwui-locks/fdholder.sock","/run/nginx-web-ui/x","/run/postgresql/.s.PGSQL.5432","/run/service/x","/run/s6/x","/run/s6-rc/x","/run/s6-rc:servicedirs/x","/run/uncaught-logs/x","/var/run/nwui-publisher/control.sock","/run/./postgresql/../postgresql/.s.PGSQL.5432","/var/lib/postgresql/data/work/x","/var/lib/nginx-web-ui/x"] {
            let target=format!("http://unix:{path}:/api/");let error=assert_valid_proxy_target(&target).unwrap_err().to_string();
            assert!(error.contains("이 컨테이너 내부 소켓은 프록시 대상으로 쓸 수 없습니다."),"{target}: {error}");assert!(error.contains('/'),"name matched prefix: {error}");
            assert!(assert_valid_stored_proxy_target(&target).is_ok(),"root isolation owns stored {target}");
        }
    }
    #[test]
    fn q23_socket_path_variables_are_refused_but_uri_variables_are_preserved() {
        let target="http://unix:/tmp/$arg_socket:/api/";
        assert!(assert_valid_proxy_target(target).unwrap_err().to_string().contains("unix 소켓 경로에는 변수를 쓸 수 없습니다."));
        assert!(assert_valid_stored_proxy_target(target).is_ok());
        assert!(assert_valid_proxy_target("http://unix:/tmp/app.sock:/api/?q=$arg_q").is_ok());
    }
    #[test]
    fn q23_runtime_dns_is_not_available_for_relative_unix_members_or_streams() {
        assert!(assert_valid_runtime_proxy_target("http://unix:/tmp/x.sock:/").unwrap_err().to_string().contains("unix 소켓 대상에는 실행 중 이름 확인을 쓸 수 없습니다."));
        for allow_literal in [true,false] {let error=assert_valid_runtime_host_port("unix:8080",allow_literal).unwrap_err().to_string();assert!(error.contains("unix 소켓 대상에는 실행 중 이름 확인을 쓸 수 없습니다."),"{error}");}
    }
}
