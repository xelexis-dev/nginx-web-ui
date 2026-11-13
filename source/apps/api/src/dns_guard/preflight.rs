use super::*;
use crate::control::model::ConfigSnapshotData;

pub fn descriptor_for_snapshot(config: &RuntimeConfig, data: &ConfigSnapshotData) -> Descriptor {
    let settings = &config.target_dns;
    let port = settings.port;
    if let Some(reason) = &settings.configuration_error {
        return Descriptor::unavailable(settings, reason);
    }
    let db_port = url::Url::parse(&config.database_url)
        .ok()
        .and_then(|u| u.port())
        .unwrap_or(5432);
    if [config.listen.port(), config.web_port, db_port].contains(&port)
        || crate::control::reserved_ports::NGINX_BASE_LISTEN_PORTS.contains(&port)
    {
        return Descriptor::unavailable(
            settings,
            format!(
                "NWUI_TARGET_DNS_PORT={port} 포트가 내부 웹/API/데이터베이스 또는 기본 nginx 수신 포트와 충돌합니다. 사용하지 않는 DNS 포트를 선택하고 다시 시작하세요."
            ),
        );
    }
    let mut conflicts = Vec::new();
    for row in &data.servers {
        if row["listen"].as_u64() == Some(u64::from(port))
            || row["config"]["tlsPort"].as_u64() == Some(u64::from(port))
        {
            conflicts.push(format!(
                "HTTP/TLS {} ({}, enabled={})",
                row["serverName"].as_str().unwrap_or(""),
                row["id"].as_str().unwrap_or("?"),
                row["enabled"]
            ));
        }
    }
    for row in &data.streams {
        if row["listen"].as_u64() == Some(u64::from(port)) {
            conflicts.push(format!(
                "{} stream {} ({}, enabled={})",
                row["protocol"].as_str().unwrap_or("?"),
                row["proxyPass"].as_str().unwrap_or(""),
                row["id"].as_str().unwrap_or("?"),
                row["enabled"]
            ));
        }
    }
    if !conflicts.is_empty() {
        return Descriptor::unavailable(
            settings,
            format!(
                "NWUI_TARGET_DNS_PORT={port} 포트가 저장된 수신 설정과 충돌합니다: {}. 비활성 규칙도 검사하며 저장된 규칙은 변경하지 않았습니다. 사용하지 않는 DNS 포트를 선택하고 다시 시작하세요.",
                conflicts.join(", ")
            ),
        );
    }
    Descriptor::available(settings)
}
pub async fn probe(
    config: &RuntimeConfig,
    data: &ConfigSnapshotData,
    resolv_conf: &str,
) -> Descriptor {
    let settings = &config.target_dns;
    let descriptor = descriptor_for_snapshot(config, data);
    if !descriptor.available {
        return descriptor;
    }
    if let Err(error) = super::upstream::servers(settings, resolv_conf) {
        return Descriptor::unavailable(settings, error.to_string());
    }
    // Test both sockets together, without querying target DNS. A failed optional
    // listener disables dynamic use and must not abort the static management UI.
    match tokio::try_join!(
        tokio::net::UdpSocket::bind(settings.endpoint()),
        tokio::net::TcpListener::bind(settings.endpoint())
    ) {
        Ok((_udp, _tcp)) => descriptor,
        Err(error) => Descriptor::unavailable(
            settings,
            format!(
                "대상 DNS 포트 {}에서 TCP/UDP 수신을 시작할 수 없습니다: {error}",
                settings.endpoint()
            ),
        ),
    }
}

pub async fn prepare(state: &crate::state::AppState) -> anyhow::Result<Descriptor> {
    prepare_with(&state.config, &state.db).await
}
pub(crate) async fn prepare_with(config: &RuntimeConfig, pool: &deadpool_postgres::Pool) -> anyhow::Result<Descriptor> {
    let settings = &config.target_dns;
    // Invalidate old availability before any query/probe, including endpoint changes.
    write_descriptor(
        settings,
        &Descriptor::unavailable(settings, "대상 DNS 사전 검사 중입니다"),
    )
    .await?;
    let data = {
        let db = pool.get().await?;
        crate::control::model::load_snapshot(&db).await?
    };
    let resolv = tokio::fs::read_to_string("/etc/resolv.conf")
        .await
        .unwrap_or_default();
    let descriptor = probe(config, &data, &resolv).await;
    write_descriptor(settings, &descriptor).await?;
    if let Some(reason) = &descriptor.reason {
        eprintln!("[dns-guard] {reason}");
    }
    Ok(descriptor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn config() -> RuntimeConfig {
        RuntimeConfig::read(|key| match key {
            "DATABASE_URL" => Some("postgres://test:test@127.0.0.1/test".into()),
            "AUTH_SECRET" => Some("test-only".into()),
            _ => None,
        })
        .unwrap()
    }
    #[test]
    fn disabled_http_tls_and_tcp_udp_rules_all_disable_guard_without_reserving_their_port() {
        let c = config();
        for data in [
            ConfigSnapshotData {
                servers: vec![
                    json!({"id":"http-old","serverName":"old.test","listen":5354,"enabled":false}),
                ],
                ..Default::default()
            },
            ConfigSnapshotData {
                servers: vec![
                    json!({"id":"tls-old","listen":80,"config":{"tlsPort":5354},"enabled":false}),
                ],
                ..Default::default()
            },
            ConfigSnapshotData {
                streams: vec![
                    json!({"id":"tcp-old","listen":5354,"protocol":"tcp","enabled":false}),
                ],
                ..Default::default()
            },
            ConfigSnapshotData {
                streams: vec![
                    json!({"id":"udp-old","listen":5354,"protocol":"udp","enabled":false}),
                ],
                ..Default::default()
            },
        ] {
            let result = descriptor_for_snapshot(&c, &data);
            assert!(!result.available, "disabled listeners count");
            let reason = result.reason.as_deref().unwrap();
            assert!(
                reason.starts_with(
                    "NWUI_TARGET_DNS_PORT=5354 포트가 저장된 수신 설정과 충돌합니다: "
                )
            );
            assert!(reason.contains("old"));
            assert!(reason.contains("enabled=false"));
            assert!(reason.ends_with("비활성 규칙도 검사하며 저장된 규칙은 변경하지 않았습니다. 사용하지 않는 DNS 포트를 선택하고 다시 시작하세요."));
            let mut ports = ReservedPorts::default();
            reserve_port(&mut ports, &result);
            assert!(!ports.process.contains(&5354));
            assert!(!ports.process_udp.contains(&5354));
        }
    }
    #[test]
    fn internal_and_base_ports_are_unavailable_without_failing_api_config() {
        let mut c = config();
        for port in [80, 443, 8080, 3000, 3001, 5432] {
            c.target_dns.port = port;
            let descriptor = descriptor_for_snapshot(&c, &ConfigSnapshotData::default());
            assert!(!descriptor.available);
            assert_eq!(
                descriptor.reason.unwrap(),
                format!(
                    "NWUI_TARGET_DNS_PORT={port} 포트가 내부 웹/API/데이터베이스 또는 기본 nginx 수신 포트와 충돌합니다. 사용하지 않는 DNS 포트를 선택하고 다시 시작하세요."
                )
            );
        }
    }
    #[tokio::test]
    async fn occupied_udp_disables_guard_and_never_queries_that_process() {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut c = config();
        c.target_dns.port = socket.local_addr().unwrap().port();
        let result = probe(&c, &ConfigSnapshotData::default(), "nameserver 127.0.0.11").await;
        assert!(!result.available);
        let reason = result.reason.as_deref().unwrap();
        assert!(reason.starts_with("대상 DNS 포트 127.0.0.1:"));
        assert!(reason.contains("에서 TCP/UDP 수신을 시작할 수 없습니다: "));
        let raw_error = tokio::net::UdpSocket::bind(socket.local_addr().unwrap())
            .await
            .unwrap_err()
            .to_string();
        assert!(
            reason.ends_with(&raw_error),
            "raw OS error must be preserved"
        );
        assert!(
            result
                .reason
                .as_deref()
                .unwrap()
                .contains(&c.target_dns.port.to_string())
        );
    }
    #[tokio::test]
    async fn dns_server_failure_is_not_probed_at_startup() {
        let mut c = config();
        c.target_dns.servers_override = Some("127.0.0.2:11991".into());
        let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        c.target_dns.port = tcp.local_addr().unwrap().port();
        drop(tcp);
        assert!(
            probe(&c, &ConfigSnapshotData::default(), "")
                .await
                .available
        );
    }
}
