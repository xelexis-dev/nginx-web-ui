use axum::http::HeaderMap;
use nwui_api::request_context::RequestContext;

#[test]
fn external_peers_cannot_forge_audit_ip_or_secure_origin() {
    let mut headers = HeaderMap::new();
    headers.insert("host", "ui.example.test:8080".parse().unwrap());
    headers.insert("x-forwarded-host", "attacker.test".parse().unwrap());
    headers.insert("x-forwarded-proto", "https".parse().unwrap());
    headers.insert("x-real-ip", "192.0.2.2".parse().unwrap());
    let context =
        RequestContext::resolve(&headers, "192.0.2.1:45000".parse().unwrap(), None).unwrap();
    assert_eq!(context.ip.as_deref(), Some("192.0.2.1"));
    assert_eq!(context.external_origin, "http://ui.example.test:8080");
    assert!(!context.secure);
}

#[test]
fn loopback_proxy_headers_preserve_origin_port_and_real_client_ip() {
    let mut headers = HeaderMap::new();
    headers.insert("host", "127.0.0.1:3001".parse().unwrap());
    headers.insert("x-forwarded-host", "ui.example.test:8443".parse().unwrap());
    headers.insert("x-forwarded-proto", "https".parse().unwrap());
    headers.insert("x-real-ip", "2001:db8::1".parse().unwrap());
    let context =
        RequestContext::resolve(&headers, "127.0.0.1:45000".parse().unwrap(), None).unwrap();
    assert_eq!(context.ip.as_deref(), Some("2001:db8::1"));
    assert_eq!(context.external_origin, "https://ui.example.test:8443");
    assert!(context.secure);
    headers.append("x-forwarded-host", "evil.test".parse().unwrap());
    assert!(RequestContext::resolve(&headers, "127.0.0.1:45000".parse().unwrap(), None).is_err());
}
