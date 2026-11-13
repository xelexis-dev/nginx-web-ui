use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ListenContext {
    Http,
    StreamTcp,
    StreamUdp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReservedPorts {
    pub process: Vec<u16>,
    #[serde(default)]
    pub process_udp: Vec<u16>,
    pub http: Vec<u16>,
    #[serde(skip)]
    pub http_plain: Vec<u16>,
    #[serde(skip)]
    pub http_tls: Vec<u16>,
    pub stream_tcp: Vec<u16>,
    #[serde(default)]
    pub owners: BTreeMap<u16, String>,
}

pub const NGINX_BASE_LISTEN_PORTS: [u16; 3] = [80, 443, super::self_proxy::ADMIN_UI_PORT];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReservedPortKind {
    Process,
    Http,
    Stream,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ReservedPortError {
    pub port: u16,
    pub kind: ReservedPortKind,
    pub message: String,
}

pub fn find_reserved_port_conflict(
    port: u16,
    context: ListenContext,
    reserved: &ReservedPorts,
) -> Option<ReservedPortError> {
    if context == ListenContext::StreamUdp && !reserved.process_udp.contains(&port) {
        return None;
    }
    let by = reserved
        .owners
        .get(&port)
        .filter(|s| !s.is_empty())
        .map(|s| format!("({s})"))
        .unwrap_or_default();
    if context == ListenContext::Http && port == super::self_proxy::ADMIN_UI_PORT {
        return Some(ReservedPortError {
            port,
            kind: ReservedPortKind::Http,
            message: format!("포트 {port} 는 관리 화면 전용입니다. 다른 HTTP·HTTPS 포트를 쓰세요."),
        });
    }
    let (kind, message) = if reserved.process.contains(&port)
        || (context == ListenContext::StreamUdp && reserved.process_udp.contains(&port))
    {
        let by = if by.is_empty() {
            "(웹 앱 또는 데이터베이스)"
        } else {
            &by
        };
        (
            ReservedPortKind::Process,
            format!(
                "포트 {port} 는 이 인스턴스의 다른 구성요소{by}가 씁니다. nginx 가 이 포트를 먼저 잡으면 그 구성요소가 시작하지 못해 관리 화면까지 함께 멈춥니다. 다른 포트를 쓰세요."
            ),
        )
    } else if context == ListenContext::StreamTcp && reserved.http.contains(&port) {
        let by = if by.is_empty() {
            "(관리 화면·HTTP·HTTPS 또는 등록된 리버스 프록시)"
        } else {
            &by
        };
        (
            ReservedPortKind::Http,
            format!(
                "포트 {port} 는 이 nginx 의 HTTP 설정이 이미 차지하고 있습니다{by}. stream 이 같은 포트를 TCP 로 잡으면 nginx 가 아예 시작되지 않습니다 — 그 HTTP 항목이 지금 꺼져 있거나 인증서 발급 전이어도, 켜지는 순간 그렇게 됩니다. 다른 포트를 쓰거나 그 항목을 옮기세요."
            ),
        )
    } else if context == ListenContext::Http && reserved.stream_tcp.contains(&port) {
        (
            ReservedPortKind::Stream,
            format!(
                "포트 {port} 는 등록된 TCP stream 이 차지하고 있습니다{by}. 같은 포트를 HTTP 쪽에서도 잡으면 nginx 가 아예 시작되지 않습니다 — 그 stream 이 지금 꺼져 있어도 켜지는 순간 그렇게 됩니다. 그 stream 을 옮기거나 지운 뒤 다시 시도하세요."
            ),
        )
    } else {
        return None;
    };
    Some(ReservedPortError {
        port,
        kind,
        message,
    })
}
