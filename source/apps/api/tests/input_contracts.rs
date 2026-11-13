use nwui_api::{config::RuntimeConfig, contracts::Patch};
use serde::Deserialize;

#[test]
fn protocol_timeouts_are_positive_and_bounded() {
    for key in ["API_ACME_TIMEOUT_SECS", "API_DNS_PROPAGATION_TIMEOUT_SECS"] {
        for value in ["0", "601", "18446744073709551615", "-1", "invalid"] {
            assert!(RuntimeConfig::read(|name| match name {
                "DATABASE_URL" => Some("postgres://test.invalid/db".into()),
                "AUTH_SECRET" => Some("test-only".into()),
                name if name == key => Some(value.into()),
                _ => None,
            }).is_err(), "{key}={value} must be refused");
        }
    }
}

#[derive(Deserialize)]
struct Update {
    #[serde(default)]
    alias: Patch<String>,
}

#[test]
fn old_clients_do_not_clear_alias_by_omitting_it() {
    assert_eq!(
        serde_json::from_str::<Update>("{}").unwrap().alias,
        Patch::Missing
    );
    assert_eq!(
        serde_json::from_str::<Update>(r#"{"alias":null}"#)
            .unwrap()
            .alias,
        Patch::Null
    );
    assert_eq!(
        serde_json::from_str::<Update>(r#"{"alias":"이름"}"#)
            .unwrap()
            .alias,
        Patch::Value("이름".into())
    );
    assert!(serde_json::from_str::<Update>(r#"{"alias":123}"#).is_err());
}

#[test]
fn api_never_implicitly_listens_on_all_interfaces_or_the_web_port() {
    let read = |key: &str| match key {
        "DATABASE_URL" => Some("postgres://test.invalid/db".to_owned()),
        "AUTH_SECRET" => Some("test-only".to_owned()),
        "HOSTNAME" => Some("0.0.0.0".to_owned()),
        _ => None,
    };
    let config = RuntimeConfig::read(read).unwrap();
    assert!(config.listen.ip().is_loopback());
    assert_eq!(config.listen.port(), 3001);
    for port in ["0", "3000", "65536", "bad"] {
        assert!(
            RuntimeConfig::read(|key| if key == "API_PORT" {
                Some(port.into())
            } else {
                read(key)
            })
            .is_err()
        );
    }
    assert!(
        RuntimeConfig::read(|key| if key == "AUTH_SECRET" {
            None
        } else {
            read(key)
        })
        .is_err()
    );
}

#[test]
fn secret_rotation_keeps_auth_js_newest_key_as_the_writing_key() {
    use nwui_api::auth::session;
    let config = RuntimeConfig::read(|key| match key {
        "DATABASE_URL" => Some("postgres://test.invalid/db".into()),
        "AUTH_SECRET" => Some("old-base".into()),
        "AUTH_SECRET_1" => Some("rotation-one".into()),
        "AUTH_SECRET_3" => Some("rotation-three".into()),
        _ => None,
    }).unwrap();
    let cookie = session::COOKIE_NAME_PLAIN;
    let token = session::encode(&session::Claims::new(), &config.auth_secrets[0], cookie, 1_800_000_000, 60).unwrap();
    assert!(session::decode(&token, &["rotation-three"], cookie, 1_800_000_001).is_ok());
    assert!(session::decode(&token, &["old-base"], cookie, 1_800_000_001).is_err());
}
