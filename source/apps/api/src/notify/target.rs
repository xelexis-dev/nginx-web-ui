//! A stored webhook target, opened (`apps/web/lib/webhook-dispatch.ts` `openTarget` /
//! `deliverWithConfig`). The URL often carries the credential itself (Slack, Discord) and headers
//! may hold `Authorization`, so both are sealed at rest; this is the one place they are unsealed.
//! Delivery to subscribers (which targets, recording the attempt) belongs to the dispatcher.

use super::send::{SendOptions, WebhookRequest, WebhookSendResult, send_webhook};
use super::{NotificationEvent, RenderContext, WebhookFormat, render_request};
use crate::crypto::SecretBox;

/// An opened target, ready to send — also built from unsaved form values for the test button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryConfig {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub format: WebhookFormat,
    pub body_template: Option<String>,
}

pub const URL_UNREADABLE: &str = "저장된 URL 을 복호화할 수 없습니다(암호화 키 유실). URL 을 다시 입력하세요.";
pub const HEADERS_UNREADABLE: &str = "저장된 헤더를 복호화할 수 없습니다(암호화 키 유실). 헤더를 다시 입력하세요.";
pub const HEADERS_DAMAGED: &str = "저장된 헤더가 손상됐습니다. 헤더를 다시 입력하세요.";

/// Sealed headers are a JSON object of strings (what the admin form writes). Anything else is
/// damage — the TS silently sent with no headers for a non-object and stringified non-string
/// values, which drops an `Authorization` without telling anyone.
fn parse_sealed_headers(plain: &str) -> Option<Vec<(String, String)>> {
    let serde_json::Value::Object(map) = serde_json::from_str(plain).ok()? else { return None };
    map.into_iter().map(|(name, value)| match value {
        serde_json::Value::String(value) => Some((name, value)),
        _ => None,
    }).collect()
}

/// Unseals a stored target. Failure is almost always a lost data key: the message says "enter it
/// again" rather than the target silently never sending.
pub fn open_target(secrets: &SecretBox, url_enc: &str, headers_enc: Option<&str>, format: &str, body_template: Option<String>) -> Result<DeliveryConfig, String> {
    let url = secrets.open(Some(url_enc)).ok_or(URL_UNREADABLE)?;
    let headers = match headers_enc.filter(|h| !h.is_empty()) {
        None => Vec::new(),
        Some(sealed) => {
            let plain = secrets.open(Some(sealed)).ok_or(HEADERS_UNREADABLE)?;
            parse_sealed_headers(&plain).ok_or(HEADERS_DAMAGED)?
        }
    };
    Ok(DeliveryConfig { url, headers, format: WebhookFormat::parse(format), body_template })
}

/// Renders and sends once. A render failure (a broken custom template) is a result with no
/// attempt, like a bad URL.
pub async fn deliver_with_config(client: &super::send::HttpClient, config: &DeliveryConfig, event: &NotificationEvent, ctx: RenderContext<'_>, options: &SendOptions) -> WebhookSendResult {
    let rendered = match render_request(config.format, config.body_template.as_deref(), event, ctx) {
        Ok(rendered) => rendered,
        Err(error) => return WebhookSendResult { ok: false, status: None, attempts: 0, error: Some(error), response_snippet: None },
    };
    let request = WebhookRequest { url: config.url.clone(), body: rendered.body, content_type: rendered.content_type.to_owned(), headers: config.headers.clone() };
    send_webhook(client, &request, options).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn secrets() -> SecretBox {
        SecretBox::from_key([3u8; 32])
    }

    #[test]
    fn opens_url_headers_and_format() {
        let s = secrets();
        let url = s.seal("https://h.test/x").unwrap();
        let headers = s.seal("{\"Authorization\":\"t\",\"X-A\":\"b\"}").unwrap();
        let opened = open_target(&s, &url, Some(&headers), "discord", Some("{}".into())).unwrap();
        assert_eq!(
            opened,
            DeliveryConfig {
                url: "https://h.test/x".into(),
                headers: vec![("Authorization".into(), "t".into()), ("X-A".into(), "b".into())],
                format: WebhookFormat::Discord,
                body_template: Some("{}".into()),
            }
        );
        let bare = open_target(&s, &url, None, "nonsense", None).unwrap();
        assert_eq!((bare.headers.len(), bare.format), (0, WebhookFormat::Generic), "unknown format falls back like the TS");
    }

    #[test]
    fn a_lost_key_or_damaged_headers_are_reported_in_words() {
        let s = secrets();
        let url = s.seal("https://h.test/x").unwrap();
        let foreign = SecretBox::from_key([7u8; 32]).seal("x").unwrap();
        assert_eq!(open_target(&s, &foreign, None, "generic", None).unwrap_err(), "저장된 URL 을 복호화할 수 없습니다(암호화 키 유실). URL 을 다시 입력하세요.");
        assert_eq!(open_target(&s, &url, Some(&foreign), "generic", None).unwrap_err(), "저장된 헤더를 복호화할 수 없습니다(암호화 키 유실). 헤더를 다시 입력하세요.");
        for damaged in ["not json", "[1,2]", "{\"a\":1}", "null"] {
            let sealed = s.seal(damaged).unwrap();
            assert_eq!(open_target(&s, &url, Some(&sealed), "generic", None).unwrap_err(), "저장된 헤더가 손상됐습니다. 헤더를 다시 입력하세요.", "{damaged}");
        }
    }

    async fn receiver() -> (String, Arc<Mutex<Vec<(axum::http::HeaderMap, String)>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let app = axum::Router::new().route(
            "/hook",
            axum::routing::post(move |headers: axum::http::HeaderMap, body: String| {
                let log = log.clone();
                async move {
                    log.lock().unwrap().push((headers, body));
                    axum::http::StatusCode::NO_CONTENT
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind((crate::outbound::test_support::peer_ip(),0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/hook"), seen)
    }

    fn fast() -> SendOptions {
        SendOptions { timeout: Duration::from_secs(2), attempts: 1, backoff: vec![] }
    }

    #[tokio::test]
    async fn delivers_the_rendered_body_with_the_target_headers() {
        let (url, seen) = receiver().await;
        let config = DeliveryConfig { url, headers: vec![("Authorization".into(), "Bearer z".into())], format: WebhookFormat::Custom, body_template: Some("{\"t\":\"{{title}}\",\"l\":\"{{link}}\"}".into()) };
        let event = super::super::sample_event(1_800_000_000_000);
        let result = deliver_with_config(&crate::notify::send::client(&crate::outbound::test_support::runtime_config()), &config, &event, RenderContext { base_url: Some("https://n.test") }, &fast()).await;
        assert_eq!((result.ok, result.status, result.attempts), (true, Some(204), 1), "{result:?}");
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0["authorization"], "Bearer z");
        let body: serde_json::Value = serde_json::from_str(&seen[0].1).unwrap();
        assert_eq!(body["t"], event.title);
        assert_eq!(body["l"], "https://n.test/certificates/00000000-0000-0000-0000-000000000000");
    }

    #[tokio::test]
    async fn a_template_that_does_not_render_is_a_result_with_no_attempt() {
        let (url, seen) = receiver().await;
        let config = DeliveryConfig { url, headers: vec![], format: WebhookFormat::Custom, body_template: None };
        let result = deliver_with_config(&crate::notify::send::client(&crate::outbound::test_support::runtime_config()), &config, &super::super::sample_event(0), RenderContext::default(), &fast()).await;
        let expected = render_request(WebhookFormat::Custom, None, &super::super::sample_event(0), RenderContext::default()).unwrap_err();
        assert_eq!((result.ok, result.status, result.attempts, result.error), (false, None, 0, Some(expected)));
        assert!(seen.lock().unwrap().is_empty());
    }
}
