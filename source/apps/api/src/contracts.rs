use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Editor,
    Admin,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Actor {
    pub id: Option<String>,
    pub label: Option<String>,
    pub ip: Option<String>,
}

/// Form command arguments have an explicit per-command arity and authorization.
#[derive(Clone, Default)]
pub struct FormInput {
    pub args: Vec<String>,
    pub fields: BTreeMap<String, Vec<String>>,
}

impl FormInput {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .get(name)
            .and_then(|values| values.first())
            .map(String::as_str)
    }
    pub fn all(&self, name: &str) -> &[String] {
        self.fields.get(name).map(Vec::as_slice).unwrap_or(&[])
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh: Option<bool>,
}

/// JSON PATCH input: missing preserves the stored value; null explicitly clears it.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Patch<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Patch<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Option::<T>::deserialize(deserializer)? {
            None => Self::Null,
            Some(value) => Self::Value(value),
        })
    }
}
