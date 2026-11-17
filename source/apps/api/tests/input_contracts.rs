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

#[test]
fn http2_strict_tls_input_is_shared_by_local_and_node_writes() {
    use nwui_api::control::{resources::{parse_server, form_body, Resource}, reserved_ports::ReservedPorts};
    use serde_json::{json, Value};
    let base = json!({"serverName":"h2.test","listen":18080,"proxyPass":"http://127.0.0.1:9000","certificateId":"cert","tls":{"tlsPort":18443}});
    for flag in [None, Some(json!(false)), Some(json!(true))] {
        let mut raw=base.clone();
        if let Some(value)=&flag { raw["tls"]["tlsOptions"]=json!({"http2":value}); }
        for wire in [raw.clone(), serde_json::from_slice(&serde_json::to_vec(&raw).unwrap()).unwrap()] {
            let row=parse_server(&wire,&ReservedPorts::default()).unwrap();
            assert_eq!(row["config"]["tlsOptions"].get("http2"), if flag==Some(json!(true)) {Some(&json!(true))} else {None});
        }
    }
    for invalid in [Value::Null,json!(0),json!("true"),json!({}),json!([])] {
        let mut raw=base.clone();raw["tls"]["tlsOptions"]=json!({"http2":invalid});
        assert!(parse_server(&raw,&ReservedPorts::default()).unwrap_err().field_errors.unwrap().contains_key("http2"));
    }
    for invalid in [json!(0),json!("tls"),json!([])] {
        let mut raw=base.clone();raw["tls"]=invalid;
        assert!(parse_server(&raw,&ReservedPorts::default()).is_err());
    }
    let mut raw=base.clone();raw["certificateId"]=Value::Null;raw["tls"]["tlsOptions"]=json!({"http2":true});
    assert!(parse_server(&raw,&ReservedPorts::default()).unwrap_err().field_errors.unwrap().contains_key("http2"));
    raw["tls"]["tlsOptions"]["http2"]=json!(false);
    assert!(parse_server(&raw,&ReservedPorts::default()).unwrap()["config"].get("tlsOptions").is_none());
    assert!(!nwui_api::control::validation::SERVER_DIRECTIVES.contains(&"http2"));
    raw=base;raw["serverDirectives"]=json!([{"name":"http2","args":["on"]}]);
    assert!(parse_server(&raw,&ReservedPorts::default()).is_err());
    let mut form=nwui_api::contracts::FormInput::default();
    for (key,value) in [("serverName","h2.test"),("listen","18080"),("mode","direct"),("proxyPass","http://127.0.0.1:9000"),("certificateId","cert"),("tlsOptions",r#"{"http2":true}"#)] { form.fields.insert(key.into(),vec![value.into()]); }
    let wire=form_body(Resource::Server,&form).unwrap();
    assert_eq!(parse_server(&wire,&ReservedPorts::default()).unwrap()["config"]["tlsOptions"]["http2"],true);
    form.fields.get_mut("tlsOptions").unwrap().push("{}".into());
    assert!(form_body(Resource::Server,&form).is_err());
}

#[test]
fn http2_legacy_pending_is_off_and_on_is_a_real_change() {
    use nwui_api::control::{model::ConfigSnapshotData,pending::diff_config};
    use serde_json::json;
    let before=ConfigSnapshotData {servers:vec![json!({"id":"s","serverName":"h2.test","listen":80,"config":{"proxyPass":"http://127.0.0.1:9000"}})],..Default::default()};
    let mut after=before.clone();after.servers[0]["config"]["tlsOptions"]=json!({"http2":false});
    assert!(!diff_config(Some(&before),&after).has_changes);
    after.servers[0]["config"]["tlsOptions"]["http2"]=json!(true);
    assert!(diff_config(Some(&before),&after).has_changes);
}

#[test]
fn null_tls_is_no_tls_in_local_forms_and_node_create_update_bodies() {
    use nwui_api::control::{resources::{parse_server,form_body,Resource},reserved_ports::ReservedPorts};
    use serde_json::{json,Value};
    let base=json!({"serverName":"plain.test","listen":18080,"proxyPass":"http://127.0.0.1:9000"});
    let expected=parse_server(&base,&ReservedPorts::default()).unwrap();
    let mut explicit=base.clone();explicit["tls"]=Value::Null;
    assert_eq!(parse_server(&explicit,&ReservedPorts::default()).unwrap(),expected);
    for bad in [json!(5),json!("x"),json!([])] {
        explicit["tls"]=bad;
        assert!(parse_server(&explicit,&ReservedPorts::default()).unwrap_err().field_errors.unwrap().contains_key("tlsOptions"));
    }
    let mut form=nwui_api::contracts::FormInput::default();
    for (key,value) in [("serverName","plain.test"),("listen","18080"),("mode","direct"),("proxyPass","http://127.0.0.1:9000")] {form.fields.insert(key.into(),vec![value.into()]);}
    let local=form_body(Resource::Server,&form).unwrap();
    assert!(local["tls"].is_object());assert!(parse_server(&local,&ReservedPorts::default()).unwrap()["config"].get("tlsPort").is_none());
    for command in ["node.createNodeServer","node.updateNodeServer"] {
        form.fields.insert("id".into(),vec!["aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into()]);
        let body=nwui_api::nodes::relay::command(command,"n",&form).unwrap().body.unwrap();
        assert_eq!(body.get("tls"),Some(&Value::Null));assert_eq!(body.get("certificateId"),Some(&Value::Null));
        let row=parse_server(&body,&ReservedPorts::default()).unwrap();
        assert!(row["config"].get("tlsPort").is_none());assert!(row["config"].get("tlsOptions").is_none());
        form.fields.insert("tlsOptions".into(),vec![json!({"http2":false}).to_string()]);
        let off=nwui_api::nodes::relay::command(command,"n",&form).unwrap().body.unwrap();
        assert_eq!(off.get("tls"),Some(&Value::Null));assert!(parse_server(&off,&ReservedPorts::default()).is_ok());
        form.fields.remove("tlsOptions");
    }
}

#[test]
fn null_tls_relay_does_not_erase_http2_validation() {
    use nwui_api::contracts::FormInput;
    use serde_json::{json,Value};
    for command in ["node.createNodeServer","node.updateNodeServer"] {
        for flag in [json!(true),Value::Null,json!(5),json!("true"),json!([]),json!({})] {
            let mut form=FormInput::default();
            for (key,value) in [("id","aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),("serverName","plain.test"),("listen","18080"),("mode","direct"),("proxyPass","http://127.0.0.1:9000")] {form.fields.insert(key.into(),vec![value.into()]);}
            form.fields.insert("tlsOptions".into(),vec![json!({"http2":flag}).to_string()]);
            let error=nwui_api::nodes::relay::command(command,"n",&form).err().expect("http2 must be checked before TLS is erased");
            assert!(error.field_errors.unwrap().contains_key("http2"));
        }
    }
}
