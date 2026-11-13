use super::Settings;
use std::net::SocketAddr;

pub fn servers(settings: &Settings, resolv_conf: &str) -> anyhow::Result<Vec<SocketAddr>> {
    anyhow::ensure!(
        settings.configuration_error.is_none(),
        "대상 DNS 설정이 올바르지 않습니다"
    );
    let mut result = Vec::new();
    if let Some(raw) = &settings.servers_override {
        let values: Vec<_> = raw
            .split(|c: char| c == ',' || c.is_ascii_whitespace())
            .filter(|s| !s.is_empty())
            .collect();
        anyhow::ensure!(
            !values.is_empty() && values.len() <= 3,
            "NWUI_TARGET_DNS_SERVERS에는 IP 주소 또는 IP:port 형식의 주소를 1~3개 지정해야 합니다"
        );
        for value in values {
            result.push(endpoint(value)?);
        }
    } else {
        for line in resolv_conf.lines() {
            let line = line.split(['#', ';']).next().unwrap_or("");
            let mut words = line.split_whitespace();
            if words.next() == Some("nameserver") {
                if let Some(value) = words.next() {
                    if let Ok(ip) = value.parse::<std::net::IpAddr>() {
                        result.push(SocketAddr::new(ip, 53));
                    }
                }
            }
        }
        let docker: SocketAddr = "127.0.0.11:53".parse().unwrap();
        if result.contains(&docker) {
            result = vec![docker];
        } else {
            result.truncate(3);
        }
    }
    anyhow::ensure!(
        !result.is_empty(),
        "/etc/resolv.conf에 IP 주소로 지정된 nameserver가 없습니다. 공용 DNS로 대체하지 않습니다"
    );
    for server in &result {
        let ip = match server.ip() {
            std::net::IpAddr::V6(ip) => ip
                .to_ipv4()
                .map(std::net::IpAddr::V4)
                .unwrap_or(std::net::IpAddr::V6(ip)),
            ip => ip,
        };
        anyhow::ensure!(
            server.port() != 0 && !ip.is_unspecified() && !ip.is_multicast(),
            "DNS 서버 주소가 올바르지 않습니다"
        );
        anyhow::ensure!(
            SocketAddr::new(ip, server.port()) != settings.endpoint(),
            "DNS 서버가 이름 확인 서비스 자신을 가리킵니다"
        );
    }
    result.dedup();
    Ok(result)
}
fn endpoint(raw: &str) -> anyhow::Result<SocketAddr> {
    raw.parse::<SocketAddr>()
        .or_else(|_| {
            raw.parse::<std::net::IpAddr>()
                .map(|ip| SocketAddr::new(ip, 53))
        })
        .map_err(|_| anyhow::anyhow!("DNS 서버는 IP 주소 또는 IP:port 형식이어야 합니다"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn docker_dns_is_preferred_without_search_or_public_fallback() {
        let s = Settings::read(|_| None);
        let found = servers(
            &s,
            "search private.test\nnameserver 10.1.2.3\nnameserver 127.0.0.11\nnameserver 8.8.8.8\n",
        )
        .unwrap();
        assert_eq!(found, vec!["127.0.0.11:53".parse().unwrap()]);
        assert_eq!(
            servers(&s, "search example.test\n")
                .unwrap_err()
                .to_string(),
            "/etc/resolv.conf에 IP 주소로 지정된 nameserver가 없습니다. 공용 DNS로 대체하지 않습니다"
        );
    }
    #[test]
    fn explicit_literal_endpoints_and_v6_nameserver_work_with_ipv6_targets_off() {
        let mut s = Settings::read(|_| None);
        s.ipv6 = false;
        s.servers_override = Some("[fd00::53]:1053, 10.1.2.3:53".into());
        assert_eq!(
            servers(&s, "nameserver 127.0.0.11").unwrap(),
            vec![
                "[fd00::53]:1053".parse().unwrap(),
                "10.1.2.3:53".parse().unwrap()
            ]
        );
    }
    #[test]
    fn invalid_override_and_self_recursion_fail_closed() {
        let mut s = Settings::read(|_| None);
        for (raw, reason) in [
            (
                "dns.example",
                "DNS 서버는 IP 주소 또는 IP:port 형식이어야 합니다",
            ),
            (
                "https://1.1.1.1",
                "DNS 서버는 IP 주소 또는 IP:port 형식이어야 합니다",
            ),
            ("127.0.0.1:0", "DNS 서버 주소가 올바르지 않습니다"),
            (
                "127.0.0.1:5354",
                "DNS 서버가 이름 확인 서비스 자신을 가리킵니다",
            ),
            (
                "[::ffff:127.0.0.1]:5354",
                "DNS 서버가 이름 확인 서비스 자신을 가리킵니다",
            ),
            ("0.0.0.0", "DNS 서버 주소가 올바르지 않습니다"),
            ("224.0.0.1", "DNS 서버 주소가 올바르지 않습니다"),
            (
                "1.1.1.1,2.2.2.2,3.3.3.3,4.4.4.4",
                "NWUI_TARGET_DNS_SERVERS에는 IP 주소 또는 IP:port 형식의 주소를 1~3개 지정해야 합니다",
            ),
        ] {
            s.servers_override = Some(raw.into());
            assert_eq!(
                servers(&s, "nameserver 127.0.0.11")
                    .unwrap_err()
                    .to_string(),
                reason,
                "invalid override must not fall back: {raw}"
            );
        }
        s.configuration_error = Some("invalid fixture".into());
        assert_eq!(
            servers(&s, "").unwrap_err().to_string(),
            "대상 DNS 설정이 올바르지 않습니다"
        );
    }
}
