mod auth_support;
mod g13_support;
use nwui_api::control::{reserved_ports::ReservedPorts, resources};
use serde_json::{Value, json};

fn server() -> Value {
    json!({"serverName":"example.test","listen":9080,"proxyPass":"http://px-web:3001/app/"})
}

#[test]
fn runtime_dns_flags_are_optional_strict_booleans() {
    for invalid in [
        json!(null),
        json!(0),
        json!(1),
        json!("true"),
        json!([]),
        json!({}),
    ] {
        let mut raw = server();
        raw["resolveAtRuntime"] = invalid.clone();
        assert!(
            resources::parse_server(&raw, &ReservedPorts::default()).is_err(),
            "server accepted {invalid}"
        );
        assert!(
            resources::parse_upstream(
                &json!({"name":"backend","servers":["px-web:3001"],"resolveAtRuntime":invalid})
            )
            .is_err()
        );
        assert!(resources::parse_stream(&json!({"listen":9200,"protocol":"tcp","proxyPass":"px-web:3001","resolveAtRuntime":invalid}),&ReservedPorts::default()).is_err());
    }
    for raw in [server(), {
        let mut v = server();
        v["resolveAtRuntime"] = json!(false);
        v
    }] {
        assert_ne!(
            resources::parse_server(&raw, &ReservedPorts::default()).unwrap()["config"]["locations"][0]["resolveAtRuntime"],
            true
        );
    }
}

#[test]
fn opted_in_direct_and_stream_targets_require_fixed_dns_authorities() {
    for target in [
        "http://127.0.0.1:9000",
        "http://127.1:9000",
        "http://user@px-web:9000",
        "http://px-web:9000/#fragment",
        "http://px-web:9000/$request_uri",
    ] {
        let mut raw = server();
        raw["proxyPass"] = json!(target);
        raw["resolveAtRuntime"] = json!(true);
        assert!(
            resources::parse_server(&raw, &ReservedPorts::default()).is_err(),
            "{target}"
        );
    }
    assert!(resources::parse_stream(&json!({"listen":9200,"protocol":"tcp","proxyPass":"127.0.0.1:9000","resolveAtRuntime":true}),&ReservedPorts::default()).is_err());
    let mut raw = server();
    raw["resolveAtRuntime"] = json!(true);
    assert_eq!(
        resources::parse_server(&raw, &ReservedPorts::default()).unwrap()["config"]["locations"][0]["resolveAtRuntime"],
        true
    );
    assert_eq!(resources::parse_upstream(&json!({"name":"mixed","servers":["10.0.0.5:9000 weight=2","px-web:3001 weight=3"],"resolveAtRuntime":true})).unwrap()["resolveAtRuntime"],true);
}

#[test]
fn location_flag_is_validated_and_cannot_override_a_named_group() {
    let raw = |location| json!({"serverName":"example.test","listen":9080,"targetMode":"locations","locations":[location]});
    for invalid in [json!(null), json!("true"), json!(1)] {
        assert!(resources::parse_server(&raw(json!({"path":"/","mode":"direct","proxyPass":"http://px-web:3001","resolveAtRuntime":invalid})),&ReservedPorts::default()).is_err());
    }
    assert!(
        resources::parse_server(
            &raw(
                json!({"path":"/","mode":"upstream","upstreamId":"group","resolveAtRuntime":true})
            ),
            &ReservedPorts::default()
        )
        .is_err()
    );
    let parsed=resources::parse_server(&raw(json!({"path":"/api/","mode":"direct","proxyPass":"http://px-web:3001","resolveAtRuntime":true})),&ReservedPorts::default()).unwrap();
    assert_eq!(parsed["config"]["locations"][0]["resolveAtRuntime"], true);
}

#[test]
fn published_contract_accepts_optional_boolean_and_rejects_invalid_flag_types() {
    let document: Value =
        serde_json::from_str(include_str!("../../../packages/contracts/openapi.json")).unwrap();
    for (schema, raw) in [
        (
            "NodeUpstreamInput",
            json!({"name":"backend","servers":["px-web:3001"]}),
        ),
        (
            "NodeStreamInput",
            json!({"listen":9200,"protocol":"tcp","proxyPass":"px-web:3001"}),
        ),
        ("NodeServerInput", server()),
        (
            "NodeLocationInput",
            json!({"path":"/","mode":"direct","proxyPass":"http://px-web:3001"}),
        ),
    ] {
        let schema = json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$ref":format!("#/components/schemas/{schema}"),"components":document["components"]});
        let validator = jsonschema::draft202012::options().build(&schema).unwrap();
        assert!(validator.is_valid(&raw));
        for value in [json!(false), json!(true)] {
            let mut v = raw.clone();
            v["resolveAtRuntime"] = value;
            assert!(validator.is_valid(&v));
        }
        for value in [json!(null), json!("true"), json!(1)] {
            let mut v = raw.clone();
            v["resolveAtRuntime"] = value;
            assert!(!validator.is_valid(&v));
        }
    }
}

#[tokio::test]
#[ignore = "real PostgreSQL: set NWUI_TEST_DATABASE_URL"]
async fn runtime_flags_persist_in_rows_and_audit_and_clear_when_old_inputs_omit_them() {
    use resources::{Mutation, Resource};
    let mut db = auth_support::test_db().await;
    let status_dir = tempfile::tempdir().unwrap();
    let config = std::sync::Arc::get_mut(&mut db.state.config).unwrap();
    config.target_dns.descriptor_path = status_dir.path().join("dns.json");
    std::fs::write(
        &config.target_dns.descriptor_path,
        serde_json::to_vec(&nwui_api::dns_guard::Descriptor::available(
            &config.target_dns,
        ))
        .unwrap(),
    )
    .unwrap();
    let actor = nwui_api::contracts::Actor {
        via: None,
        id: None,
        label: Some("runtime-dns-test".into()),
        ip: None,
    };
    for (kind, raw, column) in [
        (
            Resource::Upstream,
            json!({"name":"backend","servers":["px-web:3001"],"resolveAtRuntime":true}),
            "upstreams",
        ),
        (
            Resource::Stream,
            json!({"listen":9200,"protocol":"udp","proxyPass":"px-web:3001","resolveAtRuntime":true}),
            "nginx_streams",
        ),
    ] {
        let row = resources::mutate(&db.state, kind, Mutation::Create, None, raw.clone(), &actor)
            .await
            .unwrap();
        let id = row["id"].as_str().unwrap();
        let sql = format!("SELECT resolve_at_runtime FROM {column} WHERE id=$1");
        assert!(
            db.client
                .query_one(&sql, &[&id])
                .await
                .unwrap()
                .get::<_, bool>(0)
        );
        assert_eq!(row["resolveAtRuntime"], true);
        let listed = resources::list(&db.client, kind).await.unwrap();
        assert_eq!(
            listed.iter().find(|r| r["id"] == id).unwrap()["resolveAtRuntime"],
            true
        );
        let audit: Value = db
            .client
            .query_one(
                "SELECT after FROM audit_logs WHERE target=$1 ORDER BY created_at DESC LIMIT 1",
                &[&id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(audit["resolveAtRuntime"], true);
        let mut old_input = raw;
        old_input
            .as_object_mut()
            .unwrap()
            .remove("resolveAtRuntime");
        let replaced = resources::mutate(
            &db.state,
            kind,
            Mutation::Update,
            Some(id),
            old_input,
            &actor,
        )
        .await
        .unwrap();
        assert_eq!(replaced["resolveAtRuntime"], false);
        assert!(
            !db.client
                .query_one(&sql, &[&id])
                .await
                .unwrap()
                .get::<_, bool>(0)
        );
    }
    let row = resources::mutate(
        &db.state,
        Resource::Server,
        Mutation::Create,
        None,
        {
            let mut raw = server();
            raw["resolveAtRuntime"] = json!(true);
            raw
        },
        &actor,
    )
    .await
    .unwrap();
    assert_eq!(row["config"]["locations"][0]["resolveAtRuntime"], true);
    let replaced = resources::mutate(
        &db.state,
        Resource::Server,
        Mutation::Update,
        row["id"].as_str(),
        server(),
        &actor,
    )
    .await
    .unwrap();
    assert_ne!(replaced["config"]["locations"][0]["resolveAtRuntime"], true);
}

#[test]
fn pre_120_rows_never_become_pending_just_because_static_defaults_are_now_explicit() {
    use nwui_api::control::{model::ConfigSnapshotData, pending};
    let legacy = ConfigSnapshotData {
        upstreams: vec![json!({"id":"upstream","name":"backend","servers":["px-web:3001"]})],
        streams: vec![
            json!({"id":"stream","listen":9200,"protocol":"tcp","proxyPass":"px-web:3001","enabled":true}),
        ],
        servers: vec![
            json!({"id":"server","serverName":"example.test","listen":9080,"config":{"proxyPass":"http://px-web:3001","locations":[{"path":"/","proxyPass":"http://px-web:3001"}]},"enabled":true}),
        ],
    };
    let mut current = legacy.clone();
    current.upstreams[0]["resolveAtRuntime"] = json!(false);
    current.streams[0]["resolveAtRuntime"] = json!(false);
    current.servers[0]["config"]["resolveAtRuntime"] = json!(false);
    current.servers[0]["config"]["locations"][0]["resolveAtRuntime"] = json!(false);
    let original = legacy.clone();
    assert!(
        !pending::diff_config(Some(&legacy), &current).has_changes,
        "Pre-1.20 rows must remain static without a pending-change badge"
    );
    assert!(
        !pending::diff_config(Some(&current), &legacy).has_changes,
        "Explicit false and historical omission are equivalent in both directions"
    );
    assert_eq!(
        serde_json::to_value(legacy).unwrap(),
        serde_json::to_value(original).unwrap(),
        "Comparison must not rewrite snapshots"
    );
}

#[test]
fn changing_runtime_dns_alone_counts_in_pending_for_upstreams_streams_and_locations() {
    use nwui_api::control::{model::ConfigSnapshotData, pending};
    let before = ConfigSnapshotData {
        upstreams: vec![json!({"id":"upstream","name":"backend","servers":["px-web:3001"]})],
        streams: vec![
            json!({"id":"stream","listen":9200,"protocol":"tcp","proxyPass":"px-web:3001","enabled":true}),
        ],
        servers: vec![
            json!({"id":"server","serverName":"example.test","listen":9080,"config":{"locations":[{"path":"/","proxyPass":"http://px-web:3001"}]},"enabled":true}),
        ],
    };
    let mut after = before.clone();
    after.upstreams[0]["resolveAtRuntime"] = json!(true);
    after.streams[0]["resolveAtRuntime"] = json!(true);
    after.servers[0]["config"]["locations"][0]["resolveAtRuntime"] = json!(true);
    let diff = pending::diff_config(Some(&before), &after);
    assert!(diff.has_changes);
    assert_eq!(
        (diff.upstreams.len(), diff.streams.len(), diff.servers.len()),
        (1, 1, 1)
    );
    assert_eq!(diff.upstreams[0].changes[0].field, "resolveAtRuntime");
    assert_eq!(diff.streams[0].changes[0].field, "resolveAtRuntime");
}

fn runtime_config(available: bool) -> (nwui_api::config::RuntimeConfig, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = nwui_api::config::RuntimeConfig::read(|key| match key {
        "DATABASE_URL" => Some("postgres://test:test@127.0.0.1/test".into()),
        "AUTH_SECRET" => Some("test-only".into()),
        "NWUI_CONTAINER" => Some("1".into()),
        _ => None,
    })
    .unwrap();
    config.target_dns.descriptor_path = dir.path().join("dns.json");
    if available {
        std::fs::write(
            &config.target_dns.descriptor_path,
            serde_json::to_vec(&nwui_api::dns_guard::Descriptor::available(
                &config.target_dns,
            ))
            .unwrap(),
        )
        .unwrap();
    }
    (config, dir)
}

#[test]
fn apply_uses_trusted_descriptor_and_keeps_static_routes_when_unavailable() {
    use nwui_api::control::model::ConfigSnapshotData;
    let id = "cc7b4664-6833-4d98-9625-c0c9c724435a";
    let snapshot = ConfigSnapshotData {
        upstreams: vec![
            json!({"id":id,"name":"mixed","servers":["not-running.invalid:3001 weight=2","10.8.0.8:9000 weight=3"],"resolveAtRuntime":true}),
        ],
        servers: vec![
            json!({"id":id,"serverName":"example.test","listen":9080,"enabled":true,"config":{"locations":[{"path":"/","proxyPass":"http://not-running.invalid:3001/app/","resolveAtRuntime":true},{"path":"/static/","proxyPass":"http://10.8.0.8:9000"}]}}),
        ],
        streams: vec![
            json!({"id":id,"listen":9200,"protocol":"udp","proxyPass":"not-running.invalid:3001","enabled":true,"resolveAtRuntime":true}),
        ],
    };
    for available in [true, false] {
        let (config, _dir) = runtime_config(available);
        let outcome = g13_support::render(&config, &snapshot, &[]);
        assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
        let server =
            String::from_utf8(outcome.rendered.conf[&format!("server-{id}.conf")].clone()).unwrap();
        let group = String::from_utf8(outcome.rendered.conf["upstreams.conf"].clone()).unwrap();
        let stream =
            String::from_utf8(outcome.rendered.stream[&format!("stream-{id}.conf")].clone())
                .unwrap();
        assert!(server.contains("proxy_pass http://10.8.0.8:9000;"));
        assert!(group.contains("server 10.8.0.8:9000 weight=3;"));
        if available {
            for content in [&server, &group, &stream] {
                assert!(
                    content.contains("resolver 127.0.0.1:5354 valid=10s;"),
                    "{content}"
                );
                assert!(content.contains("not-running.invalid:3001"));
                assert!(!content.contains("return 502;"));
            }
        } else {
            assert!(server.contains("return 502;"));
            for content in [&server, &group, &stream] {
                assert!(!content.contains("resolver "));
            }
            assert!(group.contains("server 127.0.0.1:1 weight=2 down;"));
        }
    }
}

#[tokio::test]
#[ignore = "real PostgreSQL: set NWUI_TEST_DATABASE_URL"]
async fn save_rejects_unavailable_dns_and_name_collisions_without_changing_historical_groups() {
    use resources::{Mutation, Resource};
    let mut db = auth_support::test_db().await;
    let dir = tempfile::tempdir().unwrap();
    let config = std::sync::Arc::get_mut(&mut db.state.config).unwrap();
    config.target_dns.descriptor_path = dir.path().join("dns.json");
    let actor = nwui_api::contracts::Actor {
        via: None,
        id: None,
        label: Some("runtime-dns-test".into()),
        ip: None,
    };
    let raw =
        json!({"name":"backend","servers":["not-running.invalid:9000"],"resolveAtRuntime":true});
    assert!(
        resources::mutate(
            &db.state,
            Resource::Upstream,
            Mutation::Create,
            None,
            raw.clone(),
            &actor
        )
        .await
        .is_err()
    );
    assert_eq!(
        db.client
            .query_one("SELECT count(*) FROM upstreams", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    std::fs::write(
        &db.state.config.target_dns.descriptor_path,
        serde_json::to_vec(&nwui_api::dns_guard::Descriptor::available(
            &db.state.config.target_dns,
        ))
        .unwrap(),
    )
    .unwrap();
    assert!(
        resources::mutate(
            &db.state,
            Resource::Upstream,
            Mutation::Create,
            None,
            json!({"name":"nwui_resolve_reserved","servers":["10.8.0.8:9000"]}),
            &actor
        )
        .await
        .is_err()
    );
    // A pre-1.20 name is preserved when unchanged, and may be renamed to a normal name.
    db.client.execute("INSERT INTO upstreams (id,name,servers) VALUES ('legacy','nwui_resolve_historical','[\"10.8.0.8:9000\"]'::jsonb)",&[]).await.unwrap();
    resources::mutate(
        &db.state,
        Resource::Upstream,
        Mutation::Update,
        Some("legacy"),
        json!({"name":"nwui_resolve_historical","servers":["10.8.0.9:9000"]}),
        &actor,
    )
    .await
    .unwrap();
    let id = "cc7b4664-6833-4d98-9625-c0c9c724435a";
    let generated = "nwui_resolve_cc7b466468334d989625c0c9c724435a_0";
    db.client.execute("INSERT INTO upstreams (id,name,servers) VALUES ('collision',$1,'[\"10.8.0.8:9000\"]'::jsonb)",&[&generated]).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers (id,server_name,listen,config) VALUES ($1,'example.test',9080,'{\"proxyPass\":\"http://10.8.0.8:9000\"}'::jsonb)",&[&id]).await.unwrap();
    let mut dynamic = server();
    dynamic["resolveAtRuntime"] = json!(true);
    assert!(
        resources::mutate(
            &db.state,
            Resource::Server,
            Mutation::Update,
            Some(id),
            dynamic,
            &actor
        )
        .await
        .is_err()
    );
    assert_ne!(
        resources::get(&db.client, Resource::Server, id)
            .await
            .unwrap()["config"]["resolveAtRuntime"],
        true
    );
    assert_eq!(
        resources::get(&db.client, Resource::Upstream, "collision")
            .await
            .unwrap()["name"],
        generated
    );
    // A historical/imported collision must fail before nginx or managed files.
    db.client.execute("UPDATE nginx_servers SET config=jsonb_set(config,'{resolveAtRuntime}','true') WHERE id=$1",&[&id]).await.unwrap();
    let result = nwui_api::publisher_client::apply_saved(&db.state, &actor, None)
        .await
        .unwrap();
    assert!(!result.ok);
    assert!(result.output.contains("충돌"));
    assert_eq!(
        resources::get(&db.client, Resource::Upstream, "collision")
            .await
            .unwrap()["name"],
        generated
    );
}

#[test]
fn local_forms_and_node_relay_preserve_the_same_optional_runtime_flags() {
    use nwui_api::{contracts::FormInput, nodes::relay};
    use resources::Resource;
    for (kind, command, fields) in [
        (
            Resource::Upstream,
            "node.createNodeUpstream",
            vec![("name", "backend"), ("servers", "peer:9000")],
        ),
        (
            Resource::Stream,
            "node.createNodeStream",
            vec![
                ("listen", "9200"),
                ("protocol", "tcp"),
                ("proxyPass", "peer:9000"),
            ],
        ),
        (
            Resource::Server,
            "node.createNodeServer",
            vec![
                ("serverName", "example.test"),
                ("listen", "9080"),
                ("mode", "direct"),
                ("proxyPass", "http://peer:9000"),
            ],
        ),
    ] {
        for flag in [None, Some("true"), Some("false")] {
            let mut form = FormInput::default();
            for (k, v) in &fields {
                form.fields.insert(k.to_string(), vec![v.to_string()]);
            }
            if let Some(value) = flag {
                form.fields
                    .insert("resolveAtRuntime".into(), vec![value.into()]);
            }
            let local = resources::form_body(kind, &form).unwrap();
            assert_eq!(local["resolveAtRuntime"], flag == Some("true"));
            let remote = relay::command(command, "n", &form).unwrap().body.unwrap();
            assert_eq!(remote["resolveAtRuntime"], local["resolveAtRuntime"]);
            assert_eq!(remote["proxyPass"], local["proxyPass"]);
        }
    }
}

#[test]
fn runtime_response_proxy_variables_are_rejected_but_static_targets_and_other_tokens_are_unchanged()
{
    for name in ["add_header", "expires"] {
        for token in [
            "$proxy_host",
            "${proxy_host}",
            "$proxy_port",
            "${proxy_port}",
            r"\$proxy_host",
        ] {
            let args = if name == "add_header" {
                json!(["X-Original", token])
            } else {
                json!([token])
            };
            let mut raw = server();
            raw["locationDirectives"] = json!([{"name":name,"args":args}]);
            let historically_valid = !(name == "expires" && token.contains('{'))
                && !(name == "add_header" && token.contains('\\'));
            assert_eq!(
                resources::parse_server(&raw, &ReservedPorts::default()).is_ok(),
                historically_valid,
                "{name} {token}"
            );
            raw["resolveAtRuntime"] = json!(true);
            let error = resources::parse_server(&raw, &ReservedPorts::default()).unwrap_err();
            assert!(
                error
                    .field_errors
                    .unwrap()
                    .values()
                    .any(|m| m.contains("add_header·expires"))
            );
        }
    }
    let mut raw = server();
    raw["resolveAtRuntime"] = json!(true);
    raw["locationDirectives"] = json!([{"name":"add_header","args":["X-Other","$proxy_hostname:${proxy_port_suffix}:$host"]}]);
    assert!(resources::parse_server(&raw, &ReservedPorts::default()).is_ok());
    raw["serverDirectives"] = json!([{"name":"add_header","args":["X-Original","$proxy_host"]}]);
    assert!(resources::parse_server(&raw, &ReservedPorts::default()).is_err());
}

#[tokio::test]
#[ignore = "real PostgreSQL: set NWUI_TEST_DATABASE_URL"]
async fn local_server_location_flags_round_trip_and_resolver_ports_are_reserved_for_both_protocols()
{
    use nwui_api::control::reserved_ports::{ListenContext, find_reserved_port_conflict};
    use resources::{Mutation, Resource};
    let mut db = auth_support::test_db().await;
    let dir = tempfile::tempdir().unwrap();
    let config = std::sync::Arc::get_mut(&mut db.state.config).unwrap();
    config.target_dns.descriptor_path = dir.path().join("dns.json");
    std::fs::write(
        &config.target_dns.descriptor_path,
        serde_json::to_vec(&nwui_api::dns_guard::Descriptor::available(
            &config.target_dns,
        ))
        .unwrap(),
    )
    .unwrap();
    let actor = nwui_api::contracts::Actor {
        via: None,
        id: None,
        label: Some("dns-location-test".into()),
        ip: None,
    };
    let raw = json!({"serverName":"example.test","listen":9080,"targetMode":"locations","locations":[{"path":"/","mode":"direct","proxyPass":"http://not-running.invalid:9000","resolveAtRuntime":true},{"path":"/static/","mode":"direct","proxyPass":"http://10.8.0.8:9000"}]});
    db.client.execute("INSERT INTO upstreams (id,name,servers) VALUES ('group','backend','[\"10.8.0.8:9000\"]'::jsonb)",&[]).await.unwrap();
    let mut override_group = raw.clone();
    override_group["locations"][0]["proxyPass"] = json!("http://backend/path/");
    assert!(
        resources::mutate(
            &db.state,
            Resource::Server,
            Mutation::Create,
            None,
            override_group,
            &actor
        )
        .await
        .is_err()
    );
    let row = resources::mutate(
        &db.state,
        Resource::Server,
        Mutation::Create,
        None,
        raw.clone(),
        &actor,
    )
    .await
    .unwrap();
    assert_eq!(row["config"]["locations"][0]["resolveAtRuntime"], true);
    let id = row["id"].as_str().unwrap();
    let reserved = resources::reserved_ports(&db.client, &db.state)
        .await
        .unwrap();
    for context in [
        ListenContext::Http,
        ListenContext::StreamTcp,
        ListenContext::StreamUdp,
    ] {
        assert!(find_reserved_port_conflict(5354, context, &reserved).is_some());
    }
    let mut old = raw.clone();
    old["locations"][0]
        .as_object_mut()
        .unwrap()
        .remove("resolveAtRuntime");
    let updated = resources::mutate(
        &db.state,
        Resource::Server,
        Mutation::Update,
        Some(id),
        old,
        &actor,
    )
    .await
    .unwrap();
    assert_ne!(updated["config"]["locations"][0]["resolveAtRuntime"], true);
    std::fs::remove_file(&db.state.config.target_dns.descriptor_path).unwrap();
    let reserved = resources::reserved_ports(&db.client, &db.state)
        .await
        .unwrap();
    assert!(find_reserved_port_conflict(5354, ListenContext::StreamUdp, &reserved).is_none());
    assert!(
        resources::mutate(
            &db.state,
            Resource::Server,
            Mutation::Update,
            Some(id),
            raw,
            &actor
        )
        .await
        .is_err()
    );
}

#[tokio::test]
#[ignore = "real PostgreSQL: set NWUI_TEST_DATABASE_URL"]
async fn switching_a_saved_response_variable_target_to_runtime_dns_is_rejected_atomically() {
    use resources::{Mutation, Resource};
    let mut db = auth_support::test_db().await;
    let dir = tempfile::tempdir().unwrap();
    let config = std::sync::Arc::get_mut(&mut db.state.config).unwrap();
    config.target_dns.descriptor_path = dir.path().join("dns.json");
    std::fs::write(
        &config.target_dns.descriptor_path,
        serde_json::to_vec(&nwui_api::dns_guard::Descriptor::available(
            &config.target_dns,
        ))
        .unwrap(),
    )
    .unwrap();
    let actor = nwui_api::contracts::Actor {
        via: None,
        id: None,
        label: Some("response-variable-test".into()),
        ip: None,
    };
    for (index, directive) in [
        json!({"name":"add_header","args":["X-Original","$proxy_host:${proxy_port}"]}),
        json!({"name":"expires","args":["$proxy_port"]}),
    ]
    .into_iter()
    .enumerate()
    {
        let mut raw = server();
        raw["serverName"] = json!(format!("response-variable-{index}.example.test"));
        raw["locationDirectives"] = json!([directive]);
        let saved = resources::mutate(
            &db.state,
            Resource::Server,
            Mutation::Create,
            None,
            raw.clone(),
            &actor,
        )
        .await
        .unwrap();
        let id = saved["id"].as_str().unwrap();
        let before: i64 = db
            .client
            .query_one("SELECT count(*) FROM audit_logs WHERE target=$1", &[&id])
            .await
            .unwrap()
            .get(0);
        raw["resolveAtRuntime"] = json!(true);
        let error = resources::mutate(
            &db.state,
            Resource::Server,
            Mutation::Update,
            Some(id),
            raw,
            &actor,
        )
        .await
        .unwrap_err();
        assert!(
            error
                .field_errors
                .unwrap()
                .values()
                .any(|m| m.contains("add_header·expires"))
        );
        assert_eq!(
            resources::get(&db.client, Resource::Server, id)
                .await
                .unwrap(),
            saved
        );
        assert_eq!(
            db.client
                .query_one("SELECT count(*) FROM audit_logs WHERE target=$1", &[&id])
                .await
                .unwrap()
                .get::<_, i64>(0),
            before
        );
    }
}
