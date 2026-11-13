use serde_json::{Value, json};

fn document() -> Value {
    serde_json::from_slice(&std::fs::read(std::env::var("NWUI_NODE_OPENAPI_FIXTURE").expect("set NWUI_NODE_OPENAPI_FIXTURE")).expect("node contract fixture")).unwrap()
}
fn validator(name: &str) -> jsonschema::Validator {
    let schema = json!({"$schema":"https://json-schema.org/draft/2020-12/schema", "$ref":format!("#/components/schemas/{name}"), "components":document()["components"]});
    jsonschema::draft202012::options().should_validate_formats(true).build(&schema).unwrap()
}

#[test]
#[ignore = "golden vectors: set NWUI_NODE_OPENAPI_FIXTURE"]
fn all_node_schemas_compile_and_safe_certificate_projection_matches() {
    for name in document()["components"]["schemas"].as_object().unwrap().keys() {
        validator(name);
    }
    let raw = json!({
        "id":"certificate-id","alias":null,"status":"issued","source":"acme",
        "challenge_type":"dns-01","dns_provider":"cloudflare","last_error":null,
        "domains":["example.test",123],"staging":false,"failure_count":0,
        "created_at":"2026-09-23T00:00:00+09:00","issued_at":"2026-09-23T00:00:00Z",
        "expires_at":"2026-12-23T00:00:00Z","last_attempt_at":null,"renewal_notice_sent_at":null,
        "cert_pem":"private projection sentinel","key_pem":"private key sentinel",
        "dns_provider_token_enc":"sealed token sentinel",
        "dns_order":{"url":"private URL","key":"private key","records":[{"domain":"example.test","recordName":"_acme-challenge.example.test","recordValue":"value"}],"submittedAt":"legacy display timestamp"},
        "issue_log":[{"at":"legacy display timestamp","level":"info","msg":"issued"}]
    });
    let projected = nwui_api::certificates::views::certificate_view(&raw, Some("admin@example.test"));
    let schema = validator("NodeCertificate");
    let errors: Vec<_> = schema.iter_errors(&projected).map(|e|e.to_string()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    for key in ["certPem", "keyPem", "dnsProviderTokenEnc", "dnsOrder", "token"] {
        let mut leaked = projected.clone();
        leaked[key] = json!("secret");
        assert!(!schema.is_valid(&leaked), "must reject {key}");
    }
    let mut bad_date = projected;
    bad_date["createdAt"] = json!("yesterday");
    assert!(!schema.is_valid(&bad_date));
}

#[test]
#[ignore = "golden vectors: set NWUI_NODE_OPENAPI_FIXTURE"]
fn pending_serialization_and_request_omission_contracts_match() {
    let pending = nwui_api::control::pending::PendingStatus {
        count: 1, has_baseline: true,
        diff: nwui_api::control::pending::ConfigDiff {
            has_changes: true,
            servers: vec![nwui_api::control::pending::ItemDiff {
                key: "server".into(), label: "example.test".into(),
                status: nwui_api::control::pending::ItemStatus::Added,
                changes: vec![nwui_api::control::pending::FieldChange {
                    field: "config".into(), before: None,
                    after: Some(json!({"locations":[{"path":"/", "proxyPass":"http://localhost:8080"}]})),
                }],
                reason: None,
            },
            nwui_api::control::pending::ItemDiff {
                key: "refused".into(), label: "broken.test:80".into(),
                status: nwui_api::control::pending::ItemStatus::Skipped,
                changes: vec![],
                reason: Some("listen 443 is already used by a stream".into()),
            }], ..Default::default()
        },
    };
    let value = serde_json::to_value(pending).unwrap();
    assert!(validator("PendingStatus").is_valid(&value));
    assert!(value["diff"]["servers"][0]["changes"][0].get("before").is_none());
    // A refused row travels with its reason; a row nobody refused carries no empty `reason` key.
    assert_eq!(value["diff"]["servers"][1]["status"], "skipped");
    assert_eq!(value["diff"]["servers"][1]["reason"], "listen 443 is already used by a stream");
    assert!(value["diff"]["servers"][0].get("reason").is_none());
    let pair = validator("NodePairRequest");
    assert!(pair.is_valid(&json!({"token":"opaque", "gatewayName":"gateway", "futureOptional":true})));
    assert!(!pair.is_valid(&json!({"token":"opaque", "gatewayName":"gateway", "role":null})));
    let cert = validator("NodeCertificateInput");
    for alias in [None, Some(Value::Null), Some(json!("friendly"))] {
        let mut input = json!({"domains":["example.test"],"email":"admin@example.test"});
        if let Some(alias) = alias { input["alias"] = alias; }
        assert!(cert.is_valid(&input));
    }
}
