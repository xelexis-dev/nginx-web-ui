//! A1 relies on these bind addresses, rather than enumerating container interfaces.
use super::*;

#[test]
fn internal_services_remain_loopback_only() {
    const REVISIT: &str =
        "Internal services must bind loopback only; interface filtering has to be revisited.";
    let config = RuntimeConfig::read(|key| match key {
        "DATABASE_URL" => Some("postgres://test:test@127.0.0.1/test".into()),
        "AUTH_SECRET" => Some("test-only".into()),
        "HOSTNAME" | "HOST" => Some("0.0.0.0".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(
        config.listen.ip(),
        std::net::IpAddr::V4(Ipv4Addr::LOCALHOST),
        "{REVISIT}"
    );
    assert_eq!(
        config.target_dns.endpoint().ip(),
        std::net::IpAddr::V4(Ipv4Addr::LOCALHOST),
        "{REVISIT}"
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let web = std::fs::read_to_string(root.join("docker/s6-rc.d/web/run")).unwrap();
    assert!(
        web.split_whitespace()
            .any(|token| token == "HOSTNAME=127.0.0.1"),
        "{REVISIT}"
    );
    let docker = std::fs::read_to_string(root.join("Dockerfile")).unwrap();
    let hosts: Vec<_> = docker
        .lines()
        .filter(|line| line.starts_with("ENV "))
        .flat_map(str::split_whitespace)
        .filter_map(|token| token.strip_prefix("HOSTNAME="))
        .collect();
    assert!(hosts.is_empty(), "Container hostname must remain available to the node info endpoint");
    let pg = std::fs::read_to_string(root.join("docker/s6-rc.d/postgres/run")).unwrap();
    assert!(pg.contains("-c listen_addresses=127.0.0.1"), "{REVISIT}");
    // The module is wired by W2 later; its command builder has a separate executable test.
    let private = std::fs::read_to_string(root.join("apps/api/src/db_upgrade/postgres.rs")).unwrap();
    let expression = regex::Regex::new(r#""listen_addresses=([^"]*)""#).unwrap();
    let addresses = expression.captures_iter(&private).map(|value| value[1].to_owned()).collect::<Vec<_>>();
    assert_eq!(addresses, [""], "fresh private PostgreSQL must be socket-only: {REVISIT}");
}

#[tokio::test]
async fn stale_udp_listener_is_isolated_only_while_the_guard_port_is_reserved() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let mut config = RuntimeConfig::read(|key| match key {
        "DATABASE_URL" => Some("postgres://test:test@127.0.0.1/test".into()),
        "AUTH_SECRET" => Some("test-only".into()),
        "NWUI_CONTAINER" => Some("1".into()),
        _ => None,
    })
    .unwrap();
    config.nginx_conf_dir = root.path().join("conf");
    config.nginx_stream_dir = root.path().join("stream");
    config.nginx_quarantine_dir = root.path().join("quarantine");
    config.target_dns.descriptor_path = root.path().join("dns.json");
    config.nginx_main_conf = root.path().join("nginx.conf");
    config.nginx_bin = root.path().join("nginx");
    config.data_key_file = root.path().join("key");
    std::fs::create_dir_all(&config.nginx_conf_dir).unwrap();
    std::fs::create_dir_all(&config.nginx_stream_dir).unwrap();
    std::fs::write(&config.nginx_bin, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&config.nginx_bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(&config.nginx_main_conf,format!("pid {}; events {{}} http {{ server {{ listen 80; }} server {{ listen 127.0.0.1:8080; location = /nginx_status {{ stub_status; allow 127.0.0.3; deny all; include {}/.nwui-generation; }} }} }}",root.path().join("nginx.pid").display(),config.nginx_conf_dir.display())).unwrap();
    let safe = config.nginx_conf_dir.join("server-static.conf");
    let stale = config.nginx_stream_dir.join("stream-stale.conf");
    std::fs::write(
        &safe,
        "server { listen 80; location / { proxy_pass http://172.18.0.2:8080; } }",
    )
    .unwrap();
    let old = "server { listen 5354 udp; proxy_pass 172.18.0.2:3001; }";
    std::fs::write(&stale, old).unwrap();
    write_descriptor(
        &config.target_dns,
        &Descriptor::available(&config.target_dns),
    )
    .await
    .unwrap();
    let result = crate::publisher::test_support::run_guard(&config)
    .await
    .unwrap();
    assert_eq!(result.reserved_files, 1);
    assert!(safe.exists());
    assert!(!stale.exists());
    std::fs::write(&stale, old).unwrap();
    write_descriptor(
        &config.target_dns,
        &Descriptor::unavailable(&config.target_dns, "existing saved UDP listener"),
    )
    .await
    .unwrap();
    let result = crate::publisher::test_support::run_guard(&config)
    .await
    .unwrap();
    assert_eq!(result.reserved_files, 0);
    assert!(safe.exists());
    assert!(
        stale.exists(),
        "unavailable guard must not reserve an existing static listener"
    );
}

#[test]
fn supervision_orders_dns_before_nginx_and_excludes_api_secrets() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docker");
    assert_eq!(
        std::fs::read_to_string(root.join("s6-rc.d/dns-guard/type"))
            .unwrap()
            .trim(),
        "longrun"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("s6-rc.d/dns-guard/notification-fd"))
            .unwrap()
            .trim(),
        "3"
    );
    for path in [
        "s6-rc.d/dns-guard/dependencies.d/api-boot",
        "s6-rc.d/nginx/dependencies.d/dns-guard",
        "user-bundles.d/user/contents.d/dns-guard",
    ] {
        assert!(root.join(path).is_file(), "missing supervision edge {path}");
    }
    let script = std::fs::read_to_string(root.join("s6-rc.d/dns-guard/run")).unwrap();
    assert!(script.contains("exec env -i"));
    assert!(script.contains("NWUI_READY_FD=3"));
    assert!(script.contains("/usr/local/bin/nwui-api dns-guard"));
    assert!(
        !script.contains("DATABASE_URL")
            && !script.contains("AUTH_SECRET")
            && !script.contains("DATA_KEY")
    );
}
