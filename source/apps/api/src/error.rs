use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub field_errors: Option<BTreeMap<String, String>>,
    challenge: &'static str,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl ApiError {
    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "인증이 필요합니다.",
        )
    }

    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "요청을 처리하지 못했습니다.",
        )
    }

    /// Private browser-session authentication scheme; does not accept gateway Bearer tokens.
    pub fn session_required() -> Self {
        let mut error = Self::unauthorized();
        error.challenge = "Session realm=\"nginx-web-ui\"";
        error
    }

    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            field_errors: None,
            challenge: "Bearer realm=\"nginx-web-ui\"",
        }
    }

    pub fn not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "대상을 찾을 수 없습니다.",
        )
    }

    pub fn forbidden() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "이 동작을 수행할 권한이 없습니다.",
        )
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    field_errors: Option<BTreeMap<String, String>>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            Json(serde_json::json!({"error": ErrorBody {
                code: self.code, message: self.message, field_errors: self.field_errors,
            }})),
        )
            .into_response();
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        if self.status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static(self.challenge),
            );
        }
        if self.code == "config_busy" {
            response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        }
        response
    }
}
