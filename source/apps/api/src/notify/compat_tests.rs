//! TS → Rust notification golden vectors from an external reference fixture directory:
//!
//!     NWUI_NOTIFY_FIXTURES=<dir> cargo test --lib -- --include-ignored notify::compat
//!
//! JSON bodies are compared as parsed values (key order is not part of the contract); every
//! other output is compared exactly. V8's own JSON.parse error text is not reproduced — only the
//! message prefix is.

use serde_json::{Value, json};

use super::*;

fn fixture() -> Value {
    let dir = std::env::var("NWUI_NOTIFY_FIXTURES").expect("set NWUI_NOTIFY_FIXTURES");
    serde_json::from_str(&std::fs::read_to_string(std::path::Path::new(&dir).join("ts-vectors.json")).unwrap()).unwrap()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("expected a string: {v}"))
}

fn event_of(input: &Value) -> NotificationEvent {
    let text = |k: &str| input.get(k).and_then(Value::as_str).map(str::to_owned);
    NotificationEvent {
        event: s(&input["event"]).to_owned(),
        severity: s(&input["severity"]).to_owned(),
        title: s(&input["title"]).to_owned(),
        message: text("message"),
        path: text("path"),
        occurred_at_ms: chrono::DateTime::parse_from_rfc3339(s(&input["occurredAt"])).unwrap().timestamp_millis(),
        data: input.get("data").and_then(Value::as_object).cloned(),
    }
}

fn same_body(ts: &str, rust: &str, what: &str) {
    match (serde_json::from_str::<Value>(ts), serde_json::from_str::<Value>(rust)) {
        (Ok(a), Ok(b)) => assert_eq!(b, a, "{what}"),
        _ => assert_eq!(rust, ts, "{what}"),
    }
}

#[test]
#[ignore = "golden vectors: set NWUI_NOTIFY_FIXTURES"]
fn compat_events_and_links() {
    let v = fixture();
    let types: Vec<&str> = CERT_EVENTS.iter().map(|(t, _)| *t).collect();
    assert_eq!(json!(types), v["eventTypes"]);
    assert_eq!(json!(TEMPLATE_VARIABLES), v["templateVariables"]);
    for k in v["known"].as_array().unwrap() {
        assert_eq!(is_known_event_type(s(&k["type"])), k["known"].as_bool().unwrap(), "{k}");
        assert_eq!(event_label(s(&k["type"])), s(&k["label"]));
    }
    for l in v["links"].as_array().unwrap() {
        assert_eq!(json!(resolve_link(l["path"].as_str(), l["baseUrl"].as_str())), l["link"], "{l}");
    }
}

#[test]
#[ignore = "golden vectors: set NWUI_NOTIFY_FIXTURES"]
fn compat_every_format_renders_like_ts() {
    let v = fixture();
    for r in v["renders"].as_array().unwrap() {
        let what = format!("{} / {}", r["event"], r["ctx"]);
        let event = event_of(&r["input"]);
        let ctx = RenderContext { base_url: r["context"]["baseUrl"].as_str() };
        assert_eq!(build_generic_payload(&event, ctx), r["payload"], "payload {what}");
        for (name, format) in [("generic", WebhookFormat::Generic), ("discord", WebhookFormat::Discord), ("dooray", WebhookFormat::Dooray)] {
            let ts = &r["formats"][name]["request"];
            let rust = render_request(format, None, &event, ctx).unwrap();
            assert_eq!(rust.content_type, s(&ts["contentType"]));
            same_body(s(&ts["body"]), &rust.body, &format!("{name} {what}"));
        }
        for c in r["custom"].as_array().unwrap() {
            let template = s(&c["template"]);
            let (body, unknown) = render_template_body(template, &event, ctx);
            same_body(s(&c["body"]["body"]), &body, &format!("template body {what}"));
            assert_eq!(json!(unknown), c["body"]["unknownVariables"]);
            match (render_request(WebhookFormat::Custom, Some(template), &event, ctx), c["render"]["ok"].as_bool().unwrap()) {
                (Ok(rendered), true) => same_body(s(&c["render"]["request"]["body"]), &rendered.body, &format!("custom {what}")),
                (Err(error), false) => assert_eq!(error, s(&c["render"]["error"]), "custom error {what}"),
                (other, _) => panic!("custom {what}: {other:?} vs {}", c["render"]),
            }
        }
    }
}

#[test]
#[ignore = "golden vectors: set NWUI_NOTIFY_FIXTURES"]
fn compat_template_validation() {
    const JSON_ERROR: &str = "렌더 결과가 올바른 JSON 이 아닙니다 — ";
    for t in fixture()["validate"].as_array().unwrap() {
        let got = validate_template(s(&t["template"]));
        match (got, t["result"]["ok"].as_bool().unwrap()) {
            (Ok(()), true) => {}
            (Err(error), false) => {
                let ts = s(&t["result"]["error"]);
                if ts.starts_with(JSON_ERROR) {
                    assert!(error.starts_with(JSON_ERROR), "{error}");
                } else {
                    assert_eq!(error, ts);
                }
            }
            (other, _) => panic!("{t}: {other:?}"),
        }
    }
}

#[test]
#[ignore = "golden vectors: set NWUI_NOTIFY_FIXTURES"]
fn compat_headers_and_urls() {
    let v = fixture();
    for h in v["headers"].as_array().unwrap() {
        match (parse_header_lines(s(&h["text"])), h["parsed"]["ok"].as_bool().unwrap()) {
            (Ok(headers), true) => {
                let ts: Vec<(String, String)> = h["parsed"]["headers"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), s(v).to_owned())).collect();
                let mut sorted = headers.clone();
                sorted.sort();
                let mut ts_sorted = ts;
                ts_sorted.sort();
                assert_eq!(sorted, ts_sorted, "{h}");
                let mut described = describe_headers(&headers);
                described.sort();
                let mut ts_described: Vec<String> = h["described"].as_array().unwrap().iter().map(|d| s(d).to_owned()).collect();
                ts_described.sort();
                assert_eq!(described, ts_described);
            }
            (Err(error), false) => assert_eq!(error, s(&h["parsed"]["error"]), "{h}"),
            (other, _) => panic!("{h}: {other:?}"),
        }
    }
    for u in v["urls"].as_array().unwrap() {
        match (parse_webhook_url(s(&u["raw"])), u["parsed"]["ok"].as_bool().unwrap()) {
            (Ok(url), true) => assert_eq!(url.as_str(), s(&u["parsed"]["href"])),
            (Err(error), false) => assert_eq!(error, s(&u["parsed"]["error"])),
            (other, _) => panic!("{u}: {other:?}"),
        }
        assert_eq!(mask_webhook_url(s(&u["raw"])), s(&u["masked"]), "{u}");
    }
}

#[test]
fn unknown_formats_fall_back_to_generic() {
    assert_eq!(WebhookFormat::parse("discord"), WebhookFormat::Discord);
    assert_eq!(WebhookFormat::parse("slack"), WebhookFormat::Generic);
    assert_eq!(WebhookFormat::Custom.as_str(), "custom");
}
