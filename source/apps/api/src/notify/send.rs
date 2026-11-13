//! Webhook HTTP delivery (`packages/webhook/src/send.ts`). Never panics or errors out: a dead
//! receiver must not stop a background sweep; failures come back in the result.
//!
//! POST with the rendered content type (operator headers may override it), no redirects (a
//! credential-bearing request must not be re-sent to another host — 3xx is reported as a
//! configuration error), a per-attempt timeout, and retries only for network errors, 429 and 5xx.

use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookRequest {
    pub url: String,
    pub body: String,
    pub content_type: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookSendResult {
    pub ok: bool,
    /// HTTP status; `None` for network errors and timeouts.
    pub status: Option<u16>,
    pub attempts: u32,
    pub error: Option<String>,
    /// The first 300 characters of the trimmed response body (UTF-16 units), for diagnosis.
    pub response_snippet: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SendOptions {
    pub timeout: Duration,
    pub attempts: u32,
    pub backoff: Vec<Duration>,
}

impl Default for SendOptions {
    fn default() -> Self {
        Self { timeout: Duration::from_secs(10), attempts: 3, backoff: vec![Duration::from_secs(1), Duration::from_secs(3)] }
    }
}

/// Characters kept from a response body; the body itself is read up to `BODY_READ_MAX` bytes (an
/// untrusted receiver's full response has no business in memory).
const SNIPPET_MAX: usize = 300;
const BODY_READ_MAX: usize = 64 * 1024;

pub use crate::outbound::HttpClient;

/// A client with redirects disabled and no proxy; build once and reuse.
pub fn client(config:&crate::config::RuntimeConfig) -> HttpClient {
    HttpClient::new(config,||reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).no_proxy())
        .expect("a webhook HTTP client with default TLS roots")
}

fn snippet(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = crate::crypto::js_trim(&text);
    if trimmed.is_empty() {
        return None;
    }
    let mut units = 0;
    let kept: String = trimmed.chars().take_while(|c| { units += c.len_utf16(); units <= SNIPPET_MAX }).collect();
    Some(if kept.len() < trimmed.len() { format!("{kept}…") } else { kept })
}

/// Shortest part of each kind that counts as a secret. Shorter ones are left alone: redacting
/// `2` or `webhooks` would destroy the snippet without protecting anything.
const MIN_HEADER_PART: usize = 8;
const MIN_PATH_SEGMENT: usize = 16;
const MIN_QUERY_VALUE: usize = 8;
const MIN_PASSWORD: usize = 4;
const REDACTED: &str = "******";

/// What an echoing receiver (httpbin, a debugging proxy, an error page quoting the request) could
/// hand back to the operator: header values and their whitespace-separated parts, long path
/// segments (a Discord/Slack path is the credential), query values and the password — each also in
/// its percent-decoded, percent-encoded and JSON-escaped (with and without `\/`) forms. Longest
/// first, so a value is replaced before its parts. Other encodings (base64 of a value, say) are not
/// recognized; the snippet is only ever shown to `user:manage` and should not be persisted.
fn secrets_of(request: &WebhookRequest) -> Vec<String> {
    let mut base: Vec<String> = Vec::new();
    for (_, value) in &request.headers {
        base.push(value.clone());
        base.extend(value.split(crate::crypto::is_ecmascript_whitespace).filter(|p| p.len() >= MIN_HEADER_PART).map(str::to_owned));
    }
    base.retain(|s| s.len() >= MIN_HEADER_PART);
    if let Ok(url) = url::Url::parse(&request.url) {
        base.extend(url.password().filter(|p| p.len() >= MIN_PASSWORD).map(str::to_owned));
        base.extend(url.path_segments().into_iter().flatten().filter(|s| s.len() >= MIN_PATH_SEGMENT).map(str::to_owned));
        base.extend(url.query_pairs().map(|(_, v)| v.into_owned()).filter(|v| v.len() >= MIN_QUERY_VALUE));
        base.extend(url.query().into_iter().flat_map(|q| q.split('&')).filter_map(|pair| pair.split_once('=').map(|(_, v)| v.to_owned())).filter(|v| v.len() >= MIN_QUERY_VALUE));
    }
    let mut all: Vec<String> = Vec::new();
    for secret in base {
        let decoded = percent_encoding::percent_decode_str(&secret).decode_utf8_lossy().into_owned();
        for form in [secret.clone(), decoded] {
            let json = serde_json::Value::from(form.as_str()).to_string();
            let json = json[1..json.len() - 1].to_owned();
            all.push(percent_encoding::utf8_percent_encode(&form, percent_encoding::NON_ALPHANUMERIC).to_string());
            all.push(url::form_urlencoded::byte_serialize(form.as_bytes()).collect());
            all.push(json.replace('/', "\\/"));
            all.push(json);
            all.push(form);
        }
    }
    all.retain(|s| s.len() >= MIN_PASSWORD);
    all.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    all.dedup();
    all
}

fn redact(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_owned();
    for secret in secrets {
        if out.contains(secret.as_str()) {
            out = out.replace(secret.as_str(), REDACTED);
        }
    }
    out
}

/// reqwest's message names the URL; the layers under it (connector, DNS, TLS) only ever see the
/// host, so the chain without the URL is safe to show — redacted anyway, in case.
fn describe_error(error: reqwest::Error, secrets: &[String]) -> String {
    let error = error.without_url();
    let mut message = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    redact(&message, secrets)
}

struct Attempt {
    status: Option<u16>,
    error: Option<String>,
    snippet: Option<String>,
}

async fn attempt_once(client: &reqwest::Client, request: &WebhookRequest, timeout: Duration, secrets: &[String]) -> Attempt {
    use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
    let failed = |error: String| Attempt { status: None, error: Some(error), snippet: None };
    // The default content type first, so an operator header may replace it.
    let mut headers = HeaderMap::new();
    match HeaderValue::from_str(&request.content_type) {
        Ok(value) => headers.insert(CONTENT_TYPE, value),
        Err(e) => return failed(e.to_string()),
    };
    for (name, value) in &request.headers {
        match (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(value)) {
            (Ok(name), Ok(value)) => {
                headers.insert(name, value);
            }
            (Err(e), _) => return failed(e.to_string()),
            (_, Err(e)) => return failed(e.to_string()),
        }
    }
    let sent = client.post(&request.url).headers(headers).body(request.body.clone()).timeout(timeout).send().await;
    let mut response = match sent {
        Ok(response) => response,
        Err(e) if e.is_timeout() => return failed(format!("응답이 {}ms 안에 오지 않았습니다.", timeout.as_millis())),
        Err(e) => return failed(describe_error(e, secrets)),
    };
    let status = response.status();
    let mut body = Vec::new();
    while body.len() < BODY_READ_MAX {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                let room = BODY_READ_MAX - body.len();
                body.extend_from_slice(&chunk[..chunk.len().min(room)]);
            }
            Ok(None) => break,
            Err(e) if e.is_timeout() => return failed(format!("응답이 {}ms 안에 오지 않았습니다.", timeout.as_millis())),
            Err(_) => break,
        }
    }
    // Redact the whole body read, then cut: a cut first could leave half a secret showing.
    let snippet = snippet(redact(&String::from_utf8_lossy(&body), secrets).as_bytes());
    let code = status.as_u16();
    let error = if status.is_redirection() {
        Some(format!("리다이렉트({code})는 따라가지 않습니다 — 최종 주소를 직접 넣으세요."))
    } else if !status.is_success() {
        Some(format!("HTTP {code} {}", status.canonical_reason().unwrap_or("")).trim_end().to_owned())
    } else {
        None
    };
    Attempt { status: Some(code), error, snippet }
}

/// Network errors and timeouts, 429 and 5xx are worth another try; other 4xx will answer the same.
fn retryable(status: Option<u16>) -> bool {
    status.is_none_or(|s| s == 429 || s >= 500)
}

pub async fn send_webhook(client: &HttpClient, request: &WebhookRequest, options: &SendOptions) -> WebhookSendResult {
    let url=match super::parse_webhook_url(&request.url) {
        Ok(url)=>url,
        Err(error)=>return WebhookSendResult { ok:false,status:None,attempts:0,error:Some(error),response_snippet:None },
    };
    let client=match client.for_url(&url) {
        Ok(client)=>client,
        Err(_)=>return WebhookSendResult {ok:false,status:None,attempts:0,error:Some("URL 형식이 올바르지 않습니다.".into()),response_snippet:None},
    };
    let max_attempts = options.attempts.max(1);
    let secrets = secrets_of(request);
    let mut attempt = 1;
    loop {
        let last = attempt_once(client, request, options.timeout, &secrets).await;
        if last.error.is_none() {
            return WebhookSendResult { ok: true, status: last.status, attempts: attempt, error: None, response_snippet: last.snippet };
        }
        if attempt >= max_attempts || !retryable(last.status) {
            return WebhookSendResult { ok: false, status: last.status, attempts: attempt, error: last.error, response_snippet: last.snippet };
        }
        let wait = options.backoff.get((attempt - 1) as usize).or(options.backoff.last()).copied().unwrap_or_default();
        tokio::time::sleep(wait).await;
        attempt += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn client()->HttpClient {super::client(&crate::outbound::test_support::runtime_config())}
    use std::sync::{Arc, Mutex};

    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::post;

    #[derive(Default)]
    struct Seen {
        statuses: Vec<u16>,
        headers: Vec<HeaderMap>,
        bodies: Vec<String>,
    }

    type Script = Arc<Mutex<Vec<(u16, &'static str)>>>;

    /// A receiver that answers with `script` in order (the last one repeats).
    async fn receiver(script: Vec<(u16, &'static str)>, delay: Duration) -> (String, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let state = (Arc::new(Mutex::new(script)), seen.clone());
        let app = axum::Router::new().route(
            "/hook",
            post(move |axum::extract::State((script, seen)): axum::extract::State<(Script, Arc<Mutex<Seen>>)>, headers: HeaderMap, body: String| async move {
                tokio::time::sleep(delay).await;
                let (status, text) = {
                    let mut s = script.lock().unwrap();
                    if s.len() > 1 { s.remove(0) } else { s[0] }
                };
                let mut seen = seen.lock().unwrap();
                seen.statuses.push(status);
                seen.headers.push(headers);
                seen.bodies.push(body);
                let mut response = axum::response::IntoResponse::into_response((StatusCode::from_u16(status).unwrap(), text));
                if (300..400).contains(&status) {
                    response.headers_mut().insert("location", "http://elsewhere.invalid/".parse().unwrap());
                }
                response
            }),
        ).with_state(state);
        let listener = tokio::net::TcpListener::bind((crate::outbound::test_support::peer_ip(),0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/hook"), seen)
    }

    fn request(url: &str, headers: &[(&str, &str)]) -> WebhookRequest {
        WebhookRequest {
            url: url.into(),
            body: "{\"a\":1}".into(),
            content_type: "application/json".into(),
            headers: headers.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
        }
    }

    fn fast() -> SendOptions {
        SendOptions { timeout: Duration::from_millis(500), attempts: 3, backoff: vec![Duration::from_millis(10), Duration::from_millis(10)] }
    }

    #[tokio::test]
    async fn success_reports_the_status_and_a_trimmed_snippet() {
        let long: &'static str = Box::leak(format!("  {}  ", "가".repeat(400)).into_boxed_str());
        let (url, seen) = receiver(vec![(200, long)], Duration::ZERO).await;
        let result = send_webhook(&client(), &request(&url, &[("Authorization", "Bearer t"), ("content-type", "text/plain")]), &fast()).await;
        assert!(result.ok && result.status == Some(200) && result.attempts == 1, "{result:?}");
        assert_eq!(result.response_snippet, Some(format!("{}…", "가".repeat(300))));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.headers[0]["authorization"], "Bearer t");
        assert_eq!(seen.headers[0].get_all("content-type").iter().count(), 1, "the operator header replaces the default");
        assert_eq!(seen.headers[0]["content-type"], "text/plain");
        assert_eq!(seen.bodies[0], "{\"a\":1}");
    }

    #[tokio::test]
    async fn server_errors_and_429_are_retried_client_errors_are_not() {
        let (url, _) = receiver(vec![(503, ""), (429, ""), (200, "ok")], Duration::ZERO).await;
        let result = send_webhook(&client(), &request(&url, &[]), &fast()).await;
        assert_eq!((result.ok, result.attempts), (true, 3));

        let (url, seen) = receiver(vec![(404, "no such hook")], Duration::ZERO).await;
        let result = send_webhook(&client(), &request(&url, &[]), &fast()).await;
        assert_eq!((result.ok, result.status, result.attempts), (false, Some(404), 1));
        assert_eq!(result.error.as_deref(), Some("HTTP 404 Not Found"));
        assert_eq!(result.response_snippet.as_deref(), Some("no such hook"));
        assert_eq!(seen.lock().unwrap().statuses.len(), 1);

        let (url, _) = receiver(vec![(500, "")], Duration::ZERO).await;
        let result = send_webhook(&client(), &request(&url, &[]), &fast()).await;
        assert_eq!((result.ok, result.status, result.attempts), (false, Some(500), 3), "gives up after the attempts");
    }

    #[tokio::test]
    async fn redirects_are_reported_not_followed() {
        let (url, seen) = receiver(vec![(302, "")], Duration::ZERO).await;
        let result = send_webhook(&client(), &request(&url, &[("Authorization", "secret")]), &fast()).await;
        assert_eq!((result.ok, result.status, result.attempts), (false, Some(302), 1));
        assert_eq!(result.error.as_deref(), Some("리다이렉트(302)는 따라가지 않습니다 — 최종 주소를 직접 넣으세요."));
        assert_eq!(seen.lock().unwrap().statuses.len(), 1);
    }

    /// A receiver that echoes the request line and headers, like httpbin or a debugging proxy.
    async fn echo() -> String {
        let app = axum::Router::new().fallback(|uri: axum::http::Uri, headers: HeaderMap| async move {
            let mut text = format!("echo path={} query={}\n", uri.path(), uri.query().unwrap_or(""));
            for (name, value) in &headers {
                text.push_str(&format!("{name}: {}\n", value.to_str().unwrap_or("")));
            }
            let json = serde_json::json!({"headers": headers.iter().map(|(n, v)| (n.as_str().to_owned(), v.to_str().unwrap_or("").to_owned())).collect::<std::collections::BTreeMap<_, _>>()});
            text.push_str(&json.to_string().replace('/', "\\/"));
            (StatusCode::BAD_REQUEST, text)
        });
        let listener = tokio::net::TcpListener::bind((crate::outbound::test_support::peer_ip(),0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn an_echoing_receiver_cannot_read_the_secrets_back() {
        let base = echo().await;
        let url = format!("{base}/api/webhooks/1/SECRETPATH-a%2Fb?token=SECRETQUERY&x=1");
        let headers = [("Authorization", "Bearer SECRETBEARER/1+2=="), ("X-Api-Key", "SECRETKEY")];
        let result = send_webhook(&client(), &request(&url, &headers), &fast()).await;
        assert_eq!((result.ok, result.status), (false, Some(400)));
        let snippet = result.response_snippet.unwrap();
        assert!(snippet.starts_with("echo path=/api/webhooks/1/"), "still useful: {snippet}");
        assert!(snippet.contains("x-api-key: ******"), "{snippet}");
        assert!(!snippet.contains("SECRET"), "{snippet}");
    }

    #[tokio::test]
    async fn network_errors_do_not_carry_the_url() {
        let listener = std::net::TcpListener::bind((crate::outbound::test_support::peer_ip(),0)).unwrap();
        let closed = listener.local_addr().unwrap();
        drop(listener);
        let url = format!("http://{closed}/api/webhooks/1/SECRETPATH?token=SECRETQUERY");
        let result = send_webhook(&client(), &request(&url, &[]), &SendOptions { attempts: 1, ..fast() }).await;
        let error = result.error.unwrap();
        assert!(!result.ok && !error.contains("SECRET") && !error.contains("/api/webhooks"), "{error}");
        assert!(!error.is_empty());
    }

    #[test]
    fn redaction_covers_encoded_forms_and_leaves_short_values() {
        let secrets = secrets_of(&request("https://u:PASSWORD1@h.test/a/LONGSEGMENT-0001?k=QUERYVALUE1&n=2", &[("A", "Bearer TOKEN/VALUE+1"), ("B", "2")]));
        let text = "PASSWORD1 LONGSEGMENT-0001 QUERYVALUE1 TOKEN\\/VALUE+1 TOKEN%2FVALUE%2B1 Bearer TOKEN/VALUE+1 n=2 /a/";
        assert_eq!(redact(text, &secrets), "****** ****** ****** ****** ****** ****** n=2 /a/");
    }

    #[tokio::test]
    async fn timeouts_and_bad_urls_are_results_not_errors() {
        let (url, _) = receiver(vec![(200, "")], Duration::from_millis(800)).await;
        let options = SendOptions { attempts: 1, ..fast() };
        let result = send_webhook(&client(), &request(&url, &[]), &options).await;
        assert_eq!((result.ok, result.status), (false, None));
        assert_eq!(result.error.as_deref(), Some("응답이 500ms 안에 오지 않았습니다."));

        let result = send_webhook(&client(), &request("ftp://x/", &[]), &fast()).await;
        assert_eq!((result.ok, result.attempts, result.error.as_deref()), (false, 0, Some("http:// 또는 https:// 주소만 사용할 수 있습니다.")));
        let result = send_webhook(&client(), &request(&url, &[("Bad Name", "x")]), &fast()).await;
        assert!(!result.ok && result.error.is_some(), "an invalid header is a result, not a panic");
    }
}
