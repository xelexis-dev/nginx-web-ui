use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/db_upgrade")
}
fn read(name: &str) -> Value {
    serde_json::from_slice(&fs::read(root().join(name)).unwrap()).unwrap()
}
fn validator(name: &str) -> jsonschema::Validator {
    jsonschema::draft202012::options().build(&read(name)).unwrap()
}

#[test]
fn migration_status_examples_and_target_spelling_are_stable() {
    let schema = validator("state-v1.schema.json");
    for name in ["not-started", "checked", "ready", "interrupted"] {
        let value = read(&format!("examples/{name}.json"));
        assert!(schema.is_valid(&value), "{name}: {:?}", schema.iter_errors(&value).collect::<Vec<_>>());
        assert_eq!(value["target"]["major"], 17);
        assert!(value["target"].get("systemIdentifier").is_some());
        assert!(value["target"].get("pgSystemIdentifier").is_none());
    }
}

#[test]
fn readiness_cannot_be_asserted_from_an_incomplete_or_running_state() {
    let schema = validator("state-v1.schema.json");
    let ready = read("examples/ready.json");
    for patch in [json!({"phase":"checked"}), json!({"operation":"running"}),
        json!({"nextPhase":"ready"}), json!({"schemaVersion":2}),
        json!({"target":{"major":17,"architecture":"arm64","systemIdentifier":null}}),
        json!({"lastError":{"code":"interrupted","message":"중단됐습니다."}})] {
        let mut value=ready.clone();
        for (key, value_patch) in patch.as_object().unwrap() { value[key]=value_patch.clone(); }
        assert!(!schema.is_valid(&value), "accepted unsafe ready state: {patch}");
    }
    let mut value=ready;
    value["target"]["systemIdentifier"]=json!(123);
    assert!(!schema.is_valid(&value), "identities must retain decimal precision as strings");
}

#[test]
fn pair_records_have_one_flat_identity_and_strict_fresh_intent_steps() {
    let schema=validator("pair-v1.schema.json");
    for name in ["pair-ready", "pair-fresh"] {
        assert!(schema.is_valid(&read(&format!("examples/{name}.json"))));
    }
    let mut value=read("examples/pair-fresh.json");
    value["freshStep"]=json!("unknown");
    assert!(!schema.is_valid(&value));
    let mut value=read("examples/pair-ready.json");
    value["systemIdentifier"]=Value::Null;
    assert!(!schema.is_valid(&value));
    value["pgSystemIdentifier"]=json!("123");
    assert!(!schema.is_valid(&value));
}

#[test]
fn unrecorded_architecture_is_source_only_and_never_a_target_or_pair_alias() {
    let schema=validator("state-v1.schema.json");let ready=read("examples/ready.json");
    assert_eq!(ready["source"]["architecture"],"unrecorded");assert!(schema.is_valid(&ready));
    let mut target=ready.clone();target["target"]["architecture"]=json!("unrecorded");assert!(!schema.is_valid(&target));
    let mut source=ready;source["source"]["architecture"]=json!("unknown");assert!(!schema.is_valid(&source));
    let mut pair=read("examples/pair-ready.json");pair["architecture"]=json!("unrecorded");assert!(!validator("pair-v1.schema.json").is_valid(&pair));
}
