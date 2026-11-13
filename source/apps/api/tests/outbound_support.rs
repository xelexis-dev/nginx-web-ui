//! Shared outbound fixtures: approved private-network peers and pure runtime configuration.
#![allow(dead_code)]
use nwui_api::config::RuntimeConfig;
pub fn peer_ip() -> std::net::IpAddr {
    let socket=std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    // UDP connect selects a route without sending a packet.
    socket.connect("192.0.2.1:9").expect("run outbound tests through verify-api on a helper network");
    let ip=socket.local_addr().unwrap().ip();
    assert!(matches!(ip,std::net::IpAddr::V4(v) if v.octets()[..2]==[10,213]),"outbound fixtures require the approved 10.213 helper network, got {ip}");
    ip
}
pub fn peer_host() -> String {peer_ip().to_string()}

pub fn runtime_config()->RuntimeConfig {
    RuntimeConfig::read(|key|match key {
        "DATABASE_URL"=>Some("postgresql://127.0.0.1:5432/nwui".into()),
        "AUTH_SECRET"=>Some("outbound-test-secret".into()),
        _=>None,
    }).unwrap()
}
