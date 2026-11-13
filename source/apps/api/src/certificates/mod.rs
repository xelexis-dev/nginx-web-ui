//! ACME and DNS adapters. Persistence, issuance jobs and routes belong to the caller.
pub mod cloudflare;
pub mod jobs;
pub mod service;
pub mod store;
pub mod pem;
pub mod protocol;
pub mod validation;
pub mod routes;
pub mod views;

#[derive(Debug, thiserror::Error)]
pub enum CertificateError {
    #[error("{0}")]
    Invalid(String),
    #[error("ACME: {0}")]
    Acme(#[from] instant_acme::Error),
    #[error("ACME 요청 시간 초과 — CA 연결과 DNS 전파를 확인하세요.")]
    Timeout,
    #[error("요청이 중단되었습니다.")]
    Aborted,
    #[error("인증서 암호화 처리 실패: {0}")]
    Crypto(#[from] rcgen::Error),
    #[error("{0}")]
    Callback(String),
    #[error("{cause}; HTTP-01 challenge 정리 실패 {count}건",count=.failures.len())]
    HttpCleanup {
        cause: Box<CertificateError>,
        failures: Vec<protocol::HttpCleanupFailure>,
    },
}

pub type Result<T> = std::result::Result<T, CertificateError>;
