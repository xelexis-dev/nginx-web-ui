use crate::control::model::{ApplyResult, SkippedRecord};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Request { pub v: u8, pub id: String, pub op: Op, pub deadline_ms: u32, pub actor: Actor }
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Actor { pub user_id: Option<String>, pub label: Option<String>, pub ip: Option<String> }
#[derive(Serialize, Deserialize)]
#[serde(tag = "op", content = "args", rename_all = "PascalCase", deny_unknown_fields)]
pub enum Op { Hello, ApplySaved { label: Option<String>, expected_saved_digest: Option<String> },
    RollbackSnapshot { snapshot_id: String }, DeployCertificate { certificate_id: String, content_digest: String },
    ValidateLive, OperationStatus { operation_id: String } }
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response { pub v: u8, pub id: String, pub epoch: u64, pub result: OpResult }
#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum OpResult { Ok { apply: Option<ApplyResult>, skipped: Vec<SkippedRecord>, uncertain: bool, phase: String },
    Busy, Conflict, StaleMaterial, Unknown, Unavailable, BadRequest { code: String }, Failed { code: String, output: String } }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn n0_protocol_rejects_unknown_request_fields_and_operations() {
        let value = json!({"v":1,"id":"fixture","op":{"op":"Hello"},"deadlineMs":5000,
            "actor":{"userId":null,"label":null,"ip":null}});
        assert!(serde_json::from_value::<Request>(value.clone()).is_ok());
        let mut unknown = value.clone(); unknown["shell"] = json!("nginx -s reload");
        assert!(serde_json::from_value::<Request>(unknown).is_err());
        let mut unknown = value; unknown["op"] = json!({"op":"Shell"});
        assert!(serde_json::from_value::<Request>(unknown).is_err());
        let result = Response { v:1, id:"fixture".into(), epoch:7, result:OpResult::StaleMaterial };
        assert_eq!(serde_json::to_value(result).unwrap()["result"], json!({"status":"stale-material"}));
    }
}
