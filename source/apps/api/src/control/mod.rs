//! nginx configuration rendering and input validation.
//! Callers validate resource fields before rendering; nginx -t remains the final check.
pub mod model;
pub(crate) mod model_render;
pub mod performance;
pub mod pending;
pub mod operations;
pub(crate) mod confirmation;
pub mod guard_targets;
pub mod internal_targets;
pub mod internal_socket;
pub mod location_refs;
pub mod render;
pub mod reserved_ports;
pub mod resources;
pub mod routes;
pub mod self_proxy;
pub mod validation;
pub mod views;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Directive {
    pub name: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ControlError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Reserved(#[from] reserved_ports::ReservedPortError),
}

impl From<String> for ControlError {
    fn from(message: String) -> Self {
        Self::Invalid(message)
    }
}
impl From<&str> for ControlError {
    fn from(message: &str) -> Self {
        Self::Invalid(message.to_owned())
    }
}
pub type Result<T> = std::result::Result<T, ControlError>;

/// JavaScript string conversion retained only where the legacy parser explicitly
/// converted untyped JSON fields (trusted IP ranges).
pub(crate) fn input_string(value: &serde_json::Value) -> String {
    use serde_json::Value;
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        Value::Array(a) => a
            .iter()
            .map(|v| {
                if v.is_null() {
                    String::new()
                } else {
                    input_string(v)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
        _ => value.to_string(),
    }
}

/// ECMAScript whitespace, matching the previous form parser (not Rust's broader
/// Unicode White_Space set, which also contains U+0085).
pub(crate) fn whitespace(c: char) -> bool {
    matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}
