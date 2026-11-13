//! Notification delivery: events, generic payload, the generic /
//! discord / dooray / custom formats, custom-header parsing, URL checks and sending.
//!
//! The generic payload is canonical: every format derives from it. Everything here except
//! [`send`] is pure. Byte-for-byte behavior is pinned by TS golden vectors
//! supplied through NWUI_NOTIFY_FIXTURES: lengths are UTF-16 code units like JS,
//! placeholders use the ECMAScript `\s` set, JSON escaping matches `JSON.stringify`.

pub mod send;
pub mod target;

#[cfg(test)]
mod compat_tests;

use serde_json::{Map, Value};

/// `notifications.type` values a webhook may subscribe to, with their labels (form order).
pub const CERT_EVENTS: [(&str, &str); 6] = [
    ("cert.renewal_due", "만기 도래 (수동 갱신 필요)"),
    ("cert.issued", "발급 완료"),
    ("cert.renewed", "갱신 완료"),
    ("cert.failed", "발급 실패"),
    ("cert.renew_failed", "갱신 실패"),
    ("cert.dns_awaiting", "DNS TXT 추가 필요"),
];
pub const SYSTEM_EVENTS: [(&str, &str); 2] = [
    ("system.db_upgrade_backup_kept", "PG15 백업을 보관하고 있습니다"),
    ("system.db_upgrade_activation_failed", "업그레이드 후 서비스 시작에 실패했습니다"),
];
pub const TEMPLATE_VARIABLES: [&str; 12] = [
    "event", "severity", "title", "message", "link", "occurredAt", "domains", "daysLeft", "expiresAt", "certificateId", "staging", "json",
];
pub const JSON_CONTENT_TYPE: &str = "application/json";

#[derive(Clone, Debug, PartialEq)]
pub struct NotificationEvent {
    pub event: String,
    /// info | success | warning | error (anything else renders like info).
    pub severity: String,
    pub title: String,
    pub message: Option<String>,
    /// In-app path ("/certificates/…"), made absolute with the public base URL.
    pub path: Option<String>,
    pub occurred_at_ms: i64,
    pub data: Option<Map<String, Value>>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderContext<'a> {
    pub base_url: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebhookFormat {
    Generic,
    Discord,
    Dooray,
    Custom,
}

impl WebhookFormat {
    /// The form value; unknown values fall back to generic (TS).
    pub fn parse(value: &str) -> Self {
        match value {
            "discord" => Self::Discord,
            "dooray" => Self::Dooray,
            "custom" => Self::Custom,
            _ => Self::Generic,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Discord => "discord",
            Self::Dooray => "dooray",
            Self::Custom => "custom",
        }
    }
}

// ─────────────────────────────── JS semantics ───────────────────────────────

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `text.length <= max ? text : text.slice(0, max - 1) + "…"` in UTF-16 units (never splitting a
/// surrogate pair, where JS would leave half of one).
fn clip(text: &str, max: usize) -> String {
    if utf16_len(text) <= max {
        return text.to_owned();
    }
    let mut units = 0;
    let mut out: String = text
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units < max
        })
        .collect();
    out.push('…');
    out
}

/// JS `String(number)` for the values events carry (integers print without a fraction).
fn js_number(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    let f = n.as_f64().unwrap_or(f64::NAN);
    if f.fract() == 0.0 && f.abs() < 1e21 { format!("{f:.0}") } else { f.to_string() }
}

/// JS `Array.prototype.join` element conversion.
fn js_join(values: &[Value], separator: &str) -> String {
    fn one(v: &Value) -> String {
        match v {
            Value::Null => String::new(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => js_number(n),
            Value::String(s) => s.clone(),
            Value::Array(a) => js_join(a, ","),
            Value::Object(_) => "[object Object]".into(),
        }
    }
    values.iter().map(one).collect::<Vec<_>>().join(separator)
}

/// `JSON.stringify(value)` without the outer quotes.
fn json_inner(value: &str) -> String {
    let quoted = serde_json::to_string(value).unwrap_or_default();
    quoted[1..quoted.len() - 1].to_owned()
}

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn empty_map() -> &'static Map<String, Value> {
    static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(Map::new)
}

fn data_of(event: &NotificationEvent) -> &Map<String, Value> {
    event.data.as_ref().unwrap_or_else(|| empty_map())
}

fn domains_of(data: &Map<String, Value>) -> &[Value] {
    data.get("domains").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn non_empty_string<'a>(data: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    data.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Receivers reject relative links (Discord, Dooray): only an absolute http(s) link is used.
fn absolute_link(payload: &Value) -> Option<&str> {
    payload["link"].as_str().filter(|l| {
        let lower = l.get(..8).unwrap_or(l).to_ascii_lowercase();
        lower.starts_with("http://") || lower.starts_with("https://")
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedRequest {
    pub body: String,
    pub content_type: &'static str,
}

pub fn is_known_event_type(event: &str) -> bool {
    CERT_EVENTS.iter().chain(SYSTEM_EVENTS.iter()).any(|(t, _)| *t == event)
}

/// The event category and label, or the type itself when unknown.
pub fn event_label(event: &str) -> String {
    CERT_EVENTS
        .iter()
        .find(|(t, _)| *t == event)
        .map(|(_, label)| format!("인증서 · {label}"))
        .or_else(||SYSTEM_EVENTS.iter().find(|(t,_)|*t==event).map(|(_,label)|format!("시스템 · {label}")))
        .unwrap_or_else(|| event.to_owned())
}

/// Absolute with the base URL when both parse (WHATWG `new URL(path, base + "/")`), else the path.
pub fn resolve_link(path: Option<&str>, base_url: Option<&str>) -> Option<String> {
    let path = path.filter(|p| !p.is_empty())?;
    let Some(base) = base_url.filter(|b| !b.is_empty()) else { return Some(path.to_owned()) };
    let base = if base.ends_with('/') { base.to_owned() } else { format!("{base}/") };
    // An operator typo in the base URL: a relative link beats losing the link.
    Some(url::Url::parse(&base).and_then(|b| b.join(path)).map(|u| u.to_string()).unwrap_or_else(|_| path.to_owned()))
}

/// `{source, event, severity, title, message, link, occurredAt, data}`; `data` without null and
/// empty-array values.
pub fn build_generic_payload(event: &NotificationEvent, ctx: RenderContext<'_>) -> Value {
    let data: Map<String, Value> = data_of(event)
        .iter()
        .filter(|(_, v)| !v.is_null() && !v.as_array().is_some_and(Vec::is_empty))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    serde_json::json!({
        "source": "nginx-web-ui",
        "event": event.event,
        "severity": event.severity,
        "title": event.title,
        "message": event.message,
        "link": resolve_link(event.path.as_deref(), ctx.base_url),
        "occurredAt": iso(event.occurred_at_ms),
        "data": data,
    })
}

/// The sample used by test sends and template validation (shaped like a real renewal notice).
pub fn sample_event(now_ms: i64) -> NotificationEvent {
    const ID: &str = "00000000-0000-0000-0000-000000000000";
    let mut data = Map::new();
    data.insert("certificateId".into(), ID.into());
    data.insert("domains".into(), serde_json::json!(["example.com", "www.example.com"]));
    data.insert("expiresAt".into(), iso(now_ms + 12 * 24 * 60 * 60 * 1000).into());
    data.insert("daysLeft".into(), 12.into());
    data.insert("staging".into(), false.into());
    data.insert("test".into(), true.into());
    NotificationEvent {
        event: "cert.renewal_due".into(),
        severity: "warning".into(),
        title: "인증서 갱신 필요 (DNS-01)".into(),
        message: Some("example.com, www.example.com — 약 12일 남음".into()),
        path: Some(format!("/certificates/{ID}")),
        occurred_at_ms: now_ms,
        data: Some(data),
    }
}

fn render_generic(event: &NotificationEvent, ctx: RenderContext<'_>) -> RenderedRequest {
    RenderedRequest { body: build_generic_payload(event, ctx).to_string(), content_type: JSON_CONTENT_TYPE }
}

fn render_discord(event: &NotificationEvent, ctx: RenderContext<'_>) -> RenderedRequest {
    let payload = build_generic_payload(event, ctx);
    let color = match event.severity.as_str() {
        "success" => 0x22c55e,
        "warning" => 0xf59e0b,
        "error" => 0xef4444,
        _ => 0x3b82f6,
    };
    let mut embed = Map::new();
    embed.insert("title".into(), clip(&event.title, 256).into());
    embed.insert("color".into(), color.into());
    embed.insert("timestamp".into(), payload["occurredAt"].clone());
    embed.insert("footer".into(), serde_json::json!({"text": format!("nginx-web-ui · {}", event.event)}));
    if let Some(message) = payload["message"].as_str().filter(|m| !m.is_empty()) {
        embed.insert("description".into(), clip(message, 4096).into());
    }
    if let Some(url) = absolute_link(&payload) {
        embed.insert("url".into(), url.into());
    }
    let data = data_of(event);
    let mut fields = Vec::new();
    let domains = domains_of(data);
    if !domains.is_empty() {
        fields.push(serde_json::json!({"name": "도메인", "value": clip(&js_join(domains, "\n"), 1024)}));
    }
    if let Some(Value::Number(days)) = data.get("daysLeft") {
        fields.push(serde_json::json!({"name": "남은 기간", "value": format!("{}일", js_number(days)), "inline": true}));
    }
    if let Some(expires) = non_empty_string(data, "expiresAt") {
        fields.push(serde_json::json!({"name": "만료", "value": expires, "inline": true}));
    }
    if data.get("staging") == Some(&Value::Bool(true)) {
        fields.push(serde_json::json!({"name": "구분", "value": "STAGING (테스트 발급)", "inline": true}));
    }
    if !fields.is_empty() {
        embed.insert("fields".into(), fields.into());
    }
    let body = serde_json::json!({"username": "nginx-web-ui", "embeds": [embed]});
    RenderedRequest { body: body.to_string(), content_type: JSON_CONTENT_TYPE }
}

fn render_dooray(event: &NotificationEvent, ctx: RenderContext<'_>) -> RenderedRequest {
    // Dooray documents no limits: these are our own safety lines.
    const TITLE_MAX: usize = 500;
    const TEXT_MAX: usize = 4000;
    let payload = build_generic_payload(event, ctx);
    let color = match event.severity.as_str() {
        "success" => "green",
        "warning" => "orange",
        "error" => "red",
        _ => "blue",
    };
    let summary = payload["message"].as_str().filter(|m| !m.is_empty()).unwrap_or(&event.title);
    let mut attachment = Map::new();
    attachment.insert("title".into(), clip(summary, TITLE_MAX).into());
    attachment.insert("color".into(), color.into());
    if let Some(url) = absolute_link(&payload) {
        attachment.insert("titleLink".into(), url.into());
    }
    let data = data_of(event);
    let domains = domains_of(data);
    let detail: Vec<String> = [
        (!domains.is_empty()).then(|| format!("도메인: {}", js_join(domains, ", "))),
        match data.get("daysLeft") {
            Some(Value::Number(days)) => Some(format!("남은 기간: {}일", js_number(days))),
            _ => None,
        },
        non_empty_string(data, "expiresAt").map(|e| format!("만료: {e}")),
        (data.get("staging") == Some(&Value::Bool(true))).then(|| "구분: STAGING (테스트 발급)".to_owned()),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !detail.is_empty() {
        attachment.insert("text".into(), clip(&detail.join("\n"), TEXT_MAX).into());
    }
    let body = serde_json::json!({"botName": "nginx-web-ui", "text": clip(&event.title, TITLE_MAX), "attachments": [attachment]});
    RenderedRequest { body: body.to_string(), content_type: JSON_CONTENT_TYPE }
}

fn template_values(event: &NotificationEvent, ctx: RenderContext<'_>) -> Map<String, Value> {
    let payload = build_generic_payload(event, ctx);
    let data = data_of(event);
    let text = |v: &Value| json_inner(v.as_str().unwrap_or(""));
    let string_field = |key: &str| json_inner(data.get(key).and_then(Value::as_str).unwrap_or(""));
    let mut values = Map::new();
    values.insert("event".into(), text(&payload["event"]).into());
    values.insert("severity".into(), text(&payload["severity"]).into());
    values.insert("title".into(), text(&payload["title"]).into());
    values.insert("message".into(), text(&payload["message"]).into());
    values.insert("link".into(), text(&payload["link"]).into());
    values.insert("occurredAt".into(), text(&payload["occurredAt"]).into());
    values.insert("domains".into(), json_inner(&js_join(domains_of(data), ", ")).into());
    values.insert("daysLeft".into(), match data.get("daysLeft") { Some(Value::Number(n)) => js_number(n), _ => "null".into() }.into());
    values.insert("expiresAt".into(), string_field("expiresAt").into());
    values.insert("certificateId".into(), string_field("certificateId").into());
    values.insert("staging".into(), match data.get("staging") { Some(Value::Bool(b)) => b.to_string(), _ => "null".into() }.into());
    values.insert("json".into(), payload.to_string().into());
    values
}

fn placeholder() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        // `\s` of ECMAScript, not Rust's Unicode White_Space.
        const WS: &str = r"[\t\n\x0B\x0C\r \x{00A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]";
        regex::Regex::new(&format!(r"\{{\{{{WS}*([a-zA-Z][a-zA-Z0-9_]*){WS}*\}}\}}")).expect("placeholder regex")
    })
}

/// `{{name}}` substitution; returns the body and the unknown names (first-seen order).
pub fn render_template_body(template: &str, event: &NotificationEvent, ctx: RenderContext<'_>) -> (String, Vec<String>) {
    let values = template_values(event, ctx);
    let mut unknown: Vec<String> = Vec::new();
    let body = placeholder()
        .replace_all(template, |caps: &regex::Captures<'_>| match values.get(&caps[1]) {
            Some(Value::String(value)) => value.clone(),
            _ => {
                if !unknown.iter().any(|u| u == &caps[1]) {
                    unknown.push(caps[1].to_owned());
                }
                String::new()
            }
        })
        .into_owned();
    (body, unknown)
}

fn render_custom(template: Option<&str>, event: &NotificationEvent, ctx: RenderContext<'_>) -> Result<RenderedRequest, String> {
    let template = template.filter(|t| !crate::crypto::js_trim(t).is_empty()).ok_or_else(|| "본문 템플릿이 비어 있습니다.".to_owned())?;
    let (body, unknown) = render_template_body(template, event, ctx);
    if !unknown.is_empty() {
        let names: Vec<String> = unknown.iter().map(|v| format!("{{{{{v}}}}}")).collect();
        return Err(format!("알 수 없는 변수: {}", names.join(", ")));
    }
    Ok(RenderedRequest { body, content_type: JSON_CONTENT_TYPE })
}

pub fn render_request(format: WebhookFormat, template: Option<&str>, event: &NotificationEvent, ctx: RenderContext<'_>) -> Result<RenderedRequest, String> {
    match format {
        WebhookFormat::Generic => Ok(render_generic(event, ctx)),
        WebhookFormat::Discord => Ok(render_discord(event, ctx)),
        WebhookFormat::Dooray => Ok(render_dooray(event, ctx)),
        WebhookFormat::Custom => render_custom(template, event, ctx),
    }
}

/// Save-time check: renders the sample (epoch) and requires valid JSON with known variables only.
/// A broken template would otherwise fail silently at the next incident — when it matters most.
pub fn validate_template(template: &str) -> Result<(), String> {
    let rendered = render_custom(Some(template), &sample_event(0), RenderContext::default())?;
    serde_json::from_str::<Value>(&rendered.body)
        .map(|_| ())
        .map_err(|e| format!("렌더 결과가 올바른 JSON 이 아닙니다 — {e}"))
}

/// Header names our transport sets itself (content-type is allowed: receivers may need another).
const RESERVED_HEADERS: [&str; 4] = ["host", "content-length", "connection", "transfer-encoding"];

/// RFC 9110 §5.6.2 token characters.
fn is_token(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// `Name: value` lines (blank and `#` lines skipped) → headers in input order; the TS messages.
pub fn parse_header_lines(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut headers: Vec<(String, String)> = Vec::new();
    if crate::crypto::js_trim(text).is_empty() {
        return Ok(headers);
    }
    for (index, line) in text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).enumerate() {
        let n = index + 1;
        let raw = crate::crypto::js_trim(line);
        if raw.is_empty() || raw.starts_with('#') {
            continue;
        }
        let at = raw.find(':').filter(|&i| i > 0).ok_or_else(|| format!("{n}번째 줄: \"이름: 값\" 형식이어야 합니다."))?;
        let name = crate::crypto::js_trim(&raw[..at]);
        let value = crate::crypto::js_trim(&raw[at + 1..]);
        if !is_token(name) {
            return Err(format!("{n}번째 줄: 헤더 이름 \"{name}\" 에 쓸 수 없는 문자가 있습니다."));
        }
        let lower = name.to_ascii_lowercase();
        if RESERVED_HEADERS.contains(&lower.as_str()) {
            return Err(format!("{n}번째 줄: \"{name}\" 은(는) 직접 지정할 수 없는 헤더입니다."));
        }
        if headers.iter().any(|(existing, _)| existing.eq_ignore_ascii_case(name)) {
            return Err(format!("{n}번째 줄: \"{name}\" 헤더가 중복됐습니다."));
        }
        if value.is_empty() {
            return Err(format!("{n}번째 줄: \"{name}\" 의 값이 비어 있습니다."));
        }
        // No line breaks smuggled into a header.
        if value.chars().any(|c| c < '\u{20}' || c == '\u{7f}') {
            return Err(format!("{n}번째 줄: \"{name}\" 의 값에 제어문자가 있습니다."));
        }
        headers.push((name.to_owned(), value.to_owned()));
    }
    Ok(headers)
}

/// `Name: ******` — never the values.
pub fn describe_headers(headers: &[(String, String)]) -> Vec<String> {
    headers.iter().map(|(name, _)| format!("{name}: ******")).collect()
}

/// WHATWG URL, http/https only.
pub fn parse_webhook_url(raw: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(raw).map_err(|_| "URL 형식이 올바르지 않습니다.".to_owned())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("http:// 또는 https:// 주소만 사용할 수 있습니다.".to_owned());
    }
    crate::outbound::validate_url(&url).map_err(|_| "URL 형식이 올바르지 않습니다.".to_owned())?;
    Ok(url)
}

/// `<origin>/<first segment>/...` — never the full URL (a Discord path is the credential).
pub fn mask_webhook_url(raw: &str) -> String {
    let Ok(url) = url::Url::parse(raw) else { return "...".into() };
    let first = url.path().split('/').find(|s| !s.is_empty());
    format!("{}/{}...", url.origin().ascii_serialization(), first.map(|f| format!("{f}/")).unwrap_or_default())
}

#[cfg(test)]
mod upgrade_event_tests {
    #[test] fn g22_upgrade_events_are_known_and_labelled() {
        for (kind,title) in [("system.db_upgrade_backup_kept","PG15 백업을 보관하고 있습니다"),("system.db_upgrade_activation_failed","업그레이드 후 서비스 시작에 실패했습니다")] {
            assert!(super::is_known_event_type(kind));assert!(super::event_label(kind).contains(title));
        }
    }
}
