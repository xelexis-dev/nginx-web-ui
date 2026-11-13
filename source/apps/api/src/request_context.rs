use crate::error::ApiError;
use axum::http::{HeaderMap, StatusCode};
use std::net::SocketAddr;

#[derive(Clone, Debug)]
pub struct RequestContext {
    pub ip: Option<String>,
    pub external_origin: String,
    pub secure: bool,
    /// `external_origin` came from AUTH_URL/NEXTAUTH_URL rather than the request.
    pub origin_configured: bool,
}

impl RequestContext {
    pub fn resolve(
        headers: &HeaderMap,
        peer: SocketAddr,
        configured_origin: Option<&str>,
    ) -> Result<Self, ApiError> {
        let invalid = || {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "요청 주소를 확인할 수 없습니다.",
            )
        };
        let single = |name: &str| -> Result<Option<&str>, ApiError> {
            if headers.get_all(name).iter().count() > 1 {
                return Err(invalid());
            }
            headers
                .get(name)
                .map(|v| v.to_str().map_err(|_| invalid()))
                .transpose()
        };
        // Only the container's nginx and SSR process are trusted proxy peers.
        let trusted = peer.ip().is_loopback();
        let protocol = if trusted {
            single("x-forwarded-proto")?.unwrap_or("http")
        } else {
            "http"
        };
        if !matches!(protocol, "http" | "https") {
            return Err(invalid());
        }
        let forwarded_host = if trusted {
            single("x-forwarded-host")?
        } else {
            None
        };
        let host = forwarded_host.or(single("host")?).ok_or_else(invalid)?;
        if host.contains([',', '/', '\\', '@', '?', '#']) || host.chars().any(char::is_whitespace) {
            return Err(invalid());
        }
        let url = url::Url::parse(&format!("{protocol}://{host}")).map_err(|_| invalid())?;
        if url.host_str().is_none() {
            return Err(invalid());
        }
        let origin = if let Some(configured) = configured_origin {
            let configured = url::Url::parse(configured).map_err(|_| invalid())?;
            if !matches!(configured.scheme(), "http" | "https") || configured.host_str().is_none() {
                return Err(invalid());
            }
            configured.origin().ascii_serialization()
        } else {
            url.origin().ascii_serialization()
        };
        let ip = if trusted {
            single("x-real-ip")?
                .and_then(|s| s.parse::<std::net::IpAddr>().ok())
                .unwrap_or(peer.ip())
        } else {
            peer.ip()
        };
        Ok(Self {
            ip: Some(ip.to_string()),
            secure: origin.starts_with("https://"),
            external_origin: origin,
            origin_configured: configured_origin.is_some(),
        })
    }
}
