use nwui_api::control::{render, resources};
use serde_json::json;
fn protected(target: &str) {
    let cfg = json!({"serverName":"admin.test","listen":80,"proxyPass":target});
    let text = render::render_reverse_proxy(&cfg).unwrap();
    assert!(
        text.contains("proxy_set_header X-Real-IP $remote_addr;"),
        "{target}: {text}"
    );
    assert!(
        text.contains("proxy_set_header X-Forwarded-Proto $scheme;"),
        "{target}: {text}"
    );
    for header in ["X-Real-IP", "x-forwarded-proto"] {
        let mut raw = cfg.clone();
        raw["locationDirectives"] = json!([{"name":"proxy_set_header","args":[header,"spoof"]}]);
        let err = resources::parse_server(&raw, &Default::default()).unwrap_err();
        assert_eq!(err.field_errors.unwrap()["locationDirectives"], "관리 화면 프록시의 X-Real-IP·X-Forwarded-Proto 헤더는 자동 설정되며 변경할 수 없습니다.");
        let rendered = render::render_reverse_proxy(&raw).unwrap();
        assert_eq!(rendered.matches("proxy_set_header X-Real-IP $remote_addr;").count(), 1, "{target}: {rendered}");
        assert_eq!(rendered.matches("proxy_set_header X-Forwarded-Proto $scheme;").count(), 1, "{target}: {rendered}");
        assert!(!rendered.contains("spoof"), "{target}: {rendered}");
        assert_eq!(rendered.matches("proxy_bind 127.0.0.2;").count(), 1, "{target}: {rendered}");
    }
}
#[test]
fn all_ipv4_loopback_aliases_are_protected() {
    protected("http://127.0.0.2:8080");
}
#[test]
fn dotted_localhost_is_protected() {
    protected("http://LoCaLhOsT.:8080");
}
#[test]
fn mapped_ipv4_loopback_is_protected() {
    protected("http://[::ffff:127.0.0.1]:8080");
}
#[test]
fn non_admin_port_and_external_host_keep_custom_headers() {
    for target in ["http://127.0.0.2:9000", "http://192.0.2.10:8080"] {
        let cfg = json!({"serverName":"peer.test","listen":80,"proxyPass":target});
        let text = render::render_reverse_proxy(&cfg).unwrap();
        assert!(!text.contains("proxy_set_header X-Real-IP"), "{target}");
        assert!(
            !text.contains("proxy_set_header X-Forwarded-Proto"),
            "{target}"
        );
        let mut raw = cfg;
        raw["locationDirectives"] =
            json!([{"name":"proxy_set_header","args":["X-Real-IP","custom"]}]);
        resources::parse_server(&raw, &Default::default()).unwrap();
        render::render_reverse_proxy(&raw).unwrap();
    }
}

#[test]
fn non_loopback_own_interface_ip_is_not_trusted_as_the_admin_ui() {
    // Model the instance interface; admin nginx trusts forwarded client IP only from loopback.
    let target = "http://172.18.0.3:8080";
    assert!(!nwui_api::control::self_proxy::points_at_admin_ui_with(
        target,
        &|_, _| panic!("literal addresses never resolve")
    ));
    let cfg = json!({"serverName":"own.test","listen":80,"proxyPass":target,"locationDirectives":[{"name":"proxy_set_header","args":["X-Real-IP","custom"]}]});
    resources::parse_server(&cfg, &Default::default()).unwrap();
    let text = render::render_reverse_proxy(&cfg).unwrap();
    assert!(!text.contains("X-Real-IP $remote_addr"));
    assert!(!nwui_api::control::self_proxy::points_at_admin_ui_with(
        "http://own.test:8080",
        &|_, _| vec!["172.18.0.3".parse().unwrap()]
    ));
}
