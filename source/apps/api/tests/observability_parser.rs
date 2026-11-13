use nwui_api::observability::parser::*;
use std::collections::HashSet;

fn line(time: &str, host: &str, status: &str) -> String {
    format!(
        "127.0.0.1 - - [{time}] \"GET / HTTP/1.1\" {status} 42 \"-\" \"agent\\x22\" \"{host}\"\n"
    )
}

#[test]
fn combined_host_legacy_offsets_and_escaped_fields() {
    let raw = line("24/Jul/2026:19:31:18 +0900", "EXAMPLE.test:443", "404");
    let e = parse_access_line(raw.trim()).unwrap();
    assert_eq!(
        e.time_ms,
        chrono::DateTime::parse_from_rfc3339("2026-07-24T10:31:18Z")
            .unwrap()
            .timestamp_millis()
    );
    assert_eq!(e.host, "example.test");
    assert_eq!((e.status, e.bytes_sent), (404, 42));
    assert_eq!(
        parse_access_line(raw.trim().rsplit_once(" \"").unwrap().0)
            .unwrap()
            .host,
        "__no_host_field__"
    );
    assert!(parse_access_line(&raw.replace("404 42", "200 -1")).is_none());
    assert!(parse_access_line(&raw.replace("24/Jul", "99/Jul")).is_none());
    assert_eq!(normalize_host("[2001:DB8::1]:443"), "[2001:db8::1]");
    assert_eq!(normalize_host("_"), "__default_vhost__");
    assert_eq!(normalize_host(""), "__empty_host__");
    assert_eq!(normalize_host("bad/host"), "__invalid_host__");
}

#[test]
fn checkpoint_counts_only_complete_mature_prefix_without_loss_or_double_count() {
    let old = line("24/Jul/2026:19:00:00 +0000", "old.test", "200");
    let fresh = line("24/Jul/2026:20:00:00 +0000", "fresh.test", "500");
    let bytes = format!("{old}bad\n{fresh}{old}partial");
    let now = chrono::DateTime::parse_from_rfc3339("2026-07-24T20:00:30Z")
        .unwrap()
        .timestamp_millis();
    let a = scan_access_log(bytes.as_bytes(), now, 90_000, &HashSet::new());
    assert_eq!(a.consumed, old.len() + 4);
    assert_eq!(a.unparsed, 1);
    assert_eq!(a.buckets.values().map(|v| v.requests).sum::<i64>(), 1);
    let b = scan_access_log(
        &bytes.as_bytes()[a.consumed..],
        now + 180_000,
        90_000,
        &HashSet::new(),
    );
    assert_eq!(b.buckets.values().map(|v| v.requests).sum::<i64>(), 2);
    assert_eq!(a.consumed + b.consumed, bytes.len() - 7);
    let excluded = HashSet::from(["old.test".to_owned()]);
    let c = scan_access_log(bytes.as_bytes(), now + 180_000, 90_000, &excluded);
    assert_eq!(c.buckets.values().map(|v| v.requests).sum::<i64>(), 1);
}

#[test]
fn stub_status_requires_all_numeric_fields_and_never_synthesizes_zero() {
    let m = parse_stub_status("Active connections: 2\nserver accepts handled requests\n 100 99 500\nReading: 0 Writing: 1 Waiting: 1\n").unwrap();
    assert_eq!(
        (m.active, m.accepts, m.handled, m.requests, m.waiting),
        (2, 100, 99, 500, 1)
    );
    assert!(parse_stub_status("Active connections: 2").is_none());
    assert!(parse_stub_status("Active connections: -2\nserver accepts handled requests\n 100 99 500\nReading: 0 Writing: 1 Waiting: 1").is_none());
}
