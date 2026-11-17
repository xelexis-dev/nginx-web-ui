//! Administration requires live user:manage authority and enforced 2FA. Explicitly classified
//! business operations accept UserPrincipal; credential services retain an actual SessionPrincipal.

pub mod mail_recipients;
pub mod oauth;
pub mod routes;
pub mod secrets;
pub mod security;
pub mod settings;
pub mod smtp;
pub mod users;
pub mod webhooks;

pub use routes::router;

use std::collections::BTreeMap;

use crate::error::ApiError;

/// A refused admin command, in the TS `ActionState` vocabulary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdminError {
    /// Field validation (`fieldErrors`), shown next to the inputs.
    Invalid(BTreeMap<String, String>),
    /// A business rule refused it (`error`), e.g. "본인 계정은 삭제할 수 없습니다."
    Refused(String),
    /// The caller cannot delegate this authority through a user token.
    Forbidden,
    NotFound,
    /// Database or other internal failure, already safe to send.
    Api(ApiErrorCode),
}

/// `ApiError` is not `Clone`/`Eq`; keep only what tests and mapping need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiErrorCode {
    Unavailable,
    Internal,
}

impl From<AdminError> for ApiError {
    fn from(error: AdminError) -> Self {
        use axum::http::StatusCode;
        match error {
            AdminError::Invalid(fields) => {
                let mut api = ApiError::new(StatusCode::BAD_REQUEST, "validation_failed", "입력값이 올바르지 않습니다.");
                api.field_errors = Some(fields);
                api
            }
            AdminError::Refused(message) => ApiError::new(StatusCode::CONFLICT, "conflict", message),
            AdminError::Forbidden => ApiError::forbidden(),
            AdminError::NotFound => ApiError::not_found(),
            AdminError::Api(ApiErrorCode::Unavailable) => crate::auth::principal::db_unavailable(),
            AdminError::Api(ApiErrorCode::Internal) => ApiError::internal(),
        }
    }
}

pub type AdminResult<T> = Result<T, AdminError>;

/// A test button's outcome. The command ran either way; whether the receiver took it is the
/// result (`{result: {ok, message | error}}`), not an API error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestOutcome {
    Sent(String),
    Failed(String),
}

impl TestOutcome {
    pub fn into_result(self) -> crate::contracts::FormResult {
        let result = match self {
            Self::Sent(message) => serde_json::json!({"ok": true, "message": message}),
            Self::Failed(error) => serde_json::json!({"ok": false, "error": error}),
        };
        crate::contracts::FormResult { result: Some(result), ..Default::default() }
    }
}

pub(crate) fn unavailable<E>(_: E) -> AdminError {
    AdminError::Api(ApiErrorCode::Unavailable)
}

pub(crate) fn field(name: &str, message: impl Into<String>) -> AdminError {
    AdminError::Invalid(BTreeMap::from([(name.to_owned(), message.into())]))
}
