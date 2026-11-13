use std::net::IpAddr;

pub fn unsafe_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_link_local()
                || ip.octets() == [100, 100, 100, 200]
                || ip.is_broadcast()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unicast_link_local()
                || ip == std::net::Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254)
                || ip
                    .to_ipv4()
                    .is_some_and(|v4| unsafe_address(IpAddr::V4(v4)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unsafe_answers_and_their_canonical_v4_forms() {
        for raw in [
            "100.100.100.200",
            "::ffff:100.100.100.200",
            "fd00:ec2::254",
            "255.255.255.255",
            "::ffff:255.255.255.255",
            "::255.255.255.255",
            "127.0.0.1",
            "127.99.2.3",
            "0.0.0.0",
            "224.1.2.3",
            "239.255.255.255",
            "169.254.1.2",
            "::",
            "::1",
            "fe80::1",
            "febf::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::127.1.2.3",
            "::ffff:0.0.0.0",
            "::ffff:169.254.1.1",
            "::ffff:224.1.2.3",
        ] {
            assert!(
                unsafe_address(raw.parse().unwrap()),
                "{raw} must never be answered"
            );
        }
    }
    #[test]
    fn private_peers_and_public_addresses_stay_allowed() {
        for raw in [
            "100.100.100.201",
            "100.64.0.1",
            "172.18.0.2",
            "10.42.1.4",
            "192.168.10.2",
            "8.8.8.8",
            "fd00::2",
            "2001:4860:4860::8888",
            "::ffff:172.18.0.2",
        ] {
            assert!(
                !unsafe_address(raw.parse().unwrap()),
                "other-container targets must remain usable: {raw}"
            );
        }
    }
}
