pub use nwui_api::{upgrade,nonroot,certificates};
#[path="../src/crypto.rs"] mod crypto;
#[path="../src/db_upgrade/mod.rs"] mod db_upgrade;
use db_upgrade::{state::{State,Operation},output};
#[test]
fn incomplete_human_output_never_invites_switching_and_quotes_bind_metadata() {
    let mut state=State::not_started("arm64");state.destinations=Some(db_upgrade::state::Destinations{db:db_upgrade::state::Destination::Volume{source:"target-db".into()},app:db_upgrade::state::Destination::Bind{source:"/host/app: quoted \"name\"".into()}});
    let text=output::human(&state).unwrap();assert!(text.contains("전환하지 마세요"));assert!(text.contains("external: true"));assert!(text.contains("name: \"target-db\""));assert!(text.contains("/host/app: quoted \\\"name\\\""));
}
#[test]
fn failure_artifacts_are_schema_valid_incomplete_and_use_only_sanitized_static_errors() {
    let error=db_upgrade::error::Error::refusal("pair_mismatch");
    let artifact=output::failure(State::not_started("arm64"),&error);artifact.validate().unwrap();
    assert!(!artifact.ready);assert_eq!(artifact.operation,Operation::Failed);assert!(artifact.last_error.is_some());
    let json=serde_json::to_string(&artifact).unwrap();assert!(!json.contains("db_password"));assert!(!json.contains("data_key"));
}
