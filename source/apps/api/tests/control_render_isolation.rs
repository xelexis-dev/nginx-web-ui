//! Apply-time rendering isolates records that fail validation. One stale or legacy row
//! (saved by an older release, or by a path that validated less) must not stop every
//! other proxy from being rendered — an upgrade recreates the container with an empty
//! conf.d, so an all-or-nothing render there leaves every domain answering 404.
use nwui_api::{
    config::RuntimeConfig,
    control::model::{ConfigSnapshotData, SkippedKind},
};
use serde_json::{Value, json};

const OK: &str = "11111111-1111-4111-8111-111111111111";
const BAD: &str = "22222222-2222-4222-8222-222222222222";
const STREAM: &str = "33333333-3333-4333-8333-333333333333";
const UPSTREAM: &str = "44444444-4444-4444-8444-444444444444";

fn config() -> (RuntimeConfig, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let mut config = RuntimeConfig::read(|key| {
        Some(match key {
            "DATABASE_URL" => "postgres://test:test@127.0.0.1:5432/test".into(),
            "AUTH_SECRET" => "test-secret".into(),
            "NGINX_CONF_D" => "/tmp/nwui-render-isolation/conf".into(),
            "NGINX_STREAM_D" => "/tmp/nwui-render-isolation/stream".into(),
            "CERTS_DIR" => "/tmp/nwui-render-isolation/certs".into(),
            _ => return None,
        })
    })
    .unwrap();
    config.target_dns.descriptor_path = root.path().join("target-dns.json");
    (config, root)
}
fn server(id: &str, name: &str, listen: u16, enabled: bool, config: Value) -> Value {
    json!({"id":id,"serverName":name,"listen":listen,"upstreamId":null,"config":config,"enabled":enabled,"certificateId":null})
}
fn stream(id: &str, listen: u16, enabled: bool) -> Value {
    json!({"id":id,"listen":listen,"protocol":"tcp","proxyPass":"10.0.0.5:5432","enabled":enabled})
}
fn direct() -> Value {
    json!({"proxyPass":"http://127.0.0.1:9000"})
}
fn server_file(id: &str) -> String {
    format!("server-{id}.conf")
}

#[test]
fn an_invalid_server_is_skipped_and_the_others_still_render() {
    let data = ConfigSnapshotData {
        servers: vec![
            server(OK, "ok.example.com", 80, true, direct()),
            server(BAD, "bad;name.example.com", 80, true, direct()),
        ],
        ..Default::default()
    };
    let outcome = nwui_api::publisher::test_support::render_for_apply(&config().0, &data, &[]);
    assert!(outcome.rendered.conf.contains_key(&server_file(OK)));
    assert!(!outcome.rendered.conf.contains_key(&server_file(BAD)));
    assert_eq!(outcome.skipped.len(), 1, "{:?}", outcome.skipped);
    let skipped = &outcome.skipped[0];
    assert_eq!(skipped.kind, SkippedKind::Server);
    assert_eq!(skipped.id, BAD);
    assert_eq!(skipped.label, "bad;name.example.com");
    assert!(!skipped.reason.is_empty());
}

#[test]
fn a_disabled_stream_does_not_block_an_enabled_server_on_the_same_port() {
    let data = ConfigSnapshotData {
        servers: vec![server(OK, "ok.example.com", 80, true, direct())],
        streams: vec![stream(STREAM, 80, false)],
        ..Default::default()
    };
    let outcome = nwui_api::publisher::test_support::render_for_apply(&config().0, &data, &[]);
    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
    assert!(outcome.rendered.conf.contains_key(&server_file(OK)));
}

#[test]
fn a_disabled_server_does_not_block_an_enabled_stream_on_the_same_port() {
    let data = ConfigSnapshotData {
        servers: vec![server(OK, "off.example.com", 9000, false, direct())],
        streams: vec![stream(STREAM, 9000, true)],
        ..Default::default()
    };
    let outcome = nwui_api::publisher::test_support::render_for_apply(&config().0, &data, &[]);
    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
    assert!(outcome.rendered.stream.contains_key(&format!("stream-{STREAM}.conf")));
}

#[test]
fn an_enabled_http_and_tcp_conflict_keeps_the_reverse_proxy_and_skips_the_stream() {
    let data = ConfigSnapshotData {
        servers: vec![server(OK, "ok.example.com", 8443, true, direct())],
        streams: vec![stream(STREAM, 8443, true)],
        ..Default::default()
    };
    let outcome = nwui_api::publisher::test_support::render_for_apply(&config().0, &data, &[]);
    assert!(outcome.rendered.conf.contains_key(&server_file(OK)));
    assert!(!outcome.rendered.stream.contains_key(&format!("stream-{STREAM}.conf")));
    assert_eq!(outcome.skipped.len(), 1, "{:?}", outcome.skipped);
    assert_eq!(outcome.skipped[0].kind, SkippedKind::Stream);
    assert_eq!(outcome.skipped[0].id, STREAM);
}

#[test]
fn a_server_on_an_internal_process_port_is_skipped_not_fatal() {
    let data = ConfigSnapshotData {
        servers: vec![
            server(OK, "ok.example.com", 80, true, direct()),
            server(BAD, "api-port.example.com", 3001, true, direct()),
        ],
        ..Default::default()
    };
    let outcome = nwui_api::publisher::test_support::render_for_apply(&config().0, &data, &[]);
    assert!(outcome.rendered.conf.contains_key(&server_file(OK)));
    assert_eq!(outcome.skipped.len(), 1, "{:?}", outcome.skipped);
    assert_eq!(outcome.skipped[0].id, BAD);
}

#[test]
fn locations_may_target_an_upstream_whose_name_is_not_a_valid_url_host() {
    // `http://09` is not a WHATWG URL (a numeric-looking host is parsed as IPv4), but nginx
    // resolves it to the upstream block of that name; 1.17.2 saved and served exactly this.
    let data = ConfigSnapshotData {
        upstreams: vec![json!({"id":UPSTREAM,"name":"09","servers":["127.0.0.1:8080"]})],
        servers: vec![server(
            OK,
            "n.example.com",
            80,
            true,
            json!({"locations":[{"path":"/","proxyPass":"http://09"}]}),
        )],
        ..Default::default()
    };
    let outcome = nwui_api::publisher::test_support::render_for_apply(&config().0, &data, &[]);
    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
    let rendered = String::from_utf8(outcome.rendered.conf[&server_file(OK)].clone()).unwrap();
    assert!(rendered.contains("proxy_pass http://09"), "{rendered}");
}

#[test]
fn a_server_using_a_skipped_upstream_is_skipped_with_it() {
    let mut server_row = server(OK, "uses-bad-upstream.example.com", 80, true, json!({}));
    server_row["upstreamId"] = json!(UPSTREAM);
    let data = ConfigSnapshotData {
        upstreams: vec![json!({"id":UPSTREAM,"name":"backend","servers":["bad host;"]})],
        servers: vec![server_row],
        ..Default::default()
    };
    let outcome = nwui_api::publisher::test_support::render_for_apply(&config().0, &data, &[]);
    assert!(!outcome.rendered.conf.contains_key(&server_file(OK)));
    let kinds: Vec<_> = outcome.skipped.iter().map(|s| (s.kind, s.id.as_str())).collect();
    assert_eq!(kinds, vec![(SkippedKind::Upstream, UPSTREAM), (SkippedKind::Server, OK)]);
    assert!(outcome.skipped[1].reason.contains("backend"), "{:?}", outcome.skipped[1]);
}

#[test]
fn the_effective_snapshot_omits_skipped_rows_so_they_stay_pending() {
    let data = ConfigSnapshotData {
        servers: vec![
            server(OK, "ok.example.com", 80, true, direct()),
            server(BAD, "bad;name.example.com", 80, true, direct()),
        ],
        streams: vec![stream(STREAM, 9000, false)],
        ..Default::default()
    };
    let outcome = nwui_api::publisher::test_support::render_for_apply(&config().0, &data, &[]);
    let effective = outcome.effective(&data, None);
    let ids: Vec<_> = effective.servers.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec![OK]);
    // Disabled rows are not rendered, but they were not skipped either.
    assert_eq!(effective.streams.len(), 1);
}

fn container_config() -> (RuntimeConfig, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let mut config = RuntimeConfig::read(|key| {
        Some(match key {
            "DATABASE_URL" => "postgres://test:test@127.0.0.1:5432/test".into(),
            "AUTH_SECRET" => "test-secret".into(),
            "NGINX_CONF_D" => "/tmp/nwui-render-isolation/conf".into(),
            "NGINX_STREAM_D" => "/tmp/nwui-render-isolation/stream".into(),
            "CERTS_DIR" => "/tmp/nwui-render-isolation/certs".into(),
            "NWUI_CONTAINER" => "1".into(),
            _ => return None,
        })
    })
    .unwrap();
    config.target_dns.descriptor_path = root.path().join("target-dns.json");
    (config, root)
}
const TO_API: &str = "55555555-5555-4555-8555-555555555555";
const TO_ADMIN: &str = "66666666-6666-4666-8666-666666666666";
const TO_DB_LOCATION: &str = "77777777-7777-4777-8777-777777777777";
const STREAM_OK: &str = "88888888-8888-4888-8888-888888888888";

fn internal_targets() -> ConfigSnapshotData {
    ConfigSnapshotData {
        upstreams: vec![json!({"id":UPSTREAM,"name":"web","servers":["localhost:3000"]})],
        servers: vec![
            server(TO_API, "api.example.com", 80, true, json!({"proxyPass":"http://127.0.0.1:3001/"})),
            server(TO_ADMIN, "admin.example.com", 80, true, json!({"proxyPass":"http://127.0.0.1:8080"})),
            server(TO_DB_LOCATION, "db.example.com", 80, true, json!({"locations":[{"path":"/","proxyPass":"http://[::1]:5432"}]})),
        ],
        streams: vec![
            json!({"id":STREAM,"listen":9100,"protocol":"tcp","proxyPass":"127.0.0.1:5432","enabled":true}),
            json!({"id":STREAM_OK,"listen":9101,"protocol":"tcp","proxyPass":"127.0.0.1:9000","enabled":true}),
        ],
    }
}

#[test]
fn in_the_image_targets_on_internal_component_ports_are_skipped() {
    // Inside the container 127.0.0.1 is this container: the web app, the API and the database
    // must be reached through the management nginx only (127.0.0.1:8080 stays allowed).
    let outcome = nwui_api::publisher::test_support::render_for_apply(&container_config().0, &internal_targets(), &[]);
    let mut skipped: Vec<_> = outcome.skipped.iter().map(|s| s.id.as_str()).collect();
    skipped.sort();
    let mut expected = vec![UPSTREAM, TO_API, TO_DB_LOCATION, STREAM];
    expected.sort();
    assert_eq!(skipped, expected, "{:?}", outcome.skipped);
    assert!(outcome.rendered.conf.contains_key(&server_file(TO_ADMIN)));
    assert!(outcome.rendered.stream.contains_key(&format!("stream-{STREAM_OK}.conf")));
}

#[test]
fn outside_the_image_loopback_ports_belong_to_other_services() {
    let outcome = nwui_api::publisher::test_support::render_for_apply(&config().0, &internal_targets(), &[]);
    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
}

#[test]
fn in_the_image_other_spellings_of_loopback_are_skipped_too() {
    // nginx resolves targets with the system resolver: an IPv4-mapped IPv6 address, the short
    // IPv4 form and /etc/hosts names all reach this container's own ports.
    let spelled = |id: &str, target: &str| server(id, "x.example.com", 80, true, json!({"proxyPass": target}));
    let data = ConfigSnapshotData {
        upstreams: vec![json!({"id":UPSTREAM,"name":"short","servers":["127.1:3001"]})],
        servers: vec![
            spelled(TO_API, "http://[::ffff:127.0.0.1]:3001"),
            spelled(TO_DB_LOCATION, "http://ip6-localhost:3001"),
            spelled(TO_ADMIN, "http://[::ffff:127.0.0.1]:8080"),
        ],
        streams: vec![json!({"id":STREAM,"listen":9100,"protocol":"tcp","proxyPass":"127.1:5432","enabled":true})],
    };
    let outcome = nwui_api::publisher::test_support::render_for_apply(&container_config().0, &data, &[]);
    let mut skipped: Vec<_> = outcome.skipped.iter().map(|s| s.id.as_str()).collect();
    skipped.sort();
    let mut expected = vec![UPSTREAM, TO_API, TO_DB_LOCATION, STREAM];
    expected.sort();
    assert_eq!(skipped, expected, "{:?}", outcome.skipped);
    assert!(outcome.rendered.conf.contains_key(&server_file(TO_ADMIN)), "the management UI port stays allowed");
}

/// The image's own upstreams (`nwui_api`, `nwui_web`) reach its API and web app through nginx's
/// upstream-name routing, whatever the port. A user upstream is still routed to normally.
#[test]
fn in_the_image_runtime_upstream_names_are_skipped() {
    let data = ConfigSnapshotData {
        upstreams: vec![json!({"id":UPSTREAM,"name":"pool","servers":["10.0.0.5:3001"]})],
        servers: vec![
            server(TO_API, "a.example.com", 80, true, json!({"proxyPass":"http://nwui_api"})),
            server(TO_DB_LOCATION, "b.example.com", 80, true, json!({"locations":[{"path":"/ok","proxyPass":"http://10.0.0.5:80"},{"path":"/web","proxyPass":"https://NWUI_Web:8443/x"}]})),
            server(TO_ADMIN, "c.example.com", 80, true, json!({"locations":[{"path":"/","proxyPass":"http://pool"}]})),
        ],
        streams: vec![],
    };
    let outcome = nwui_api::publisher::test_support::render_for_apply(&container_config().0, &data, &[]);
    let mut skipped: Vec<_> = outcome.skipped.iter().map(|s| s.id.as_str()).collect();
    skipped.sort();
    let mut expected = vec![TO_API, TO_DB_LOCATION];
    expected.sort();
    assert_eq!(skipped, expected, "{:?}", outcome.skipped);
    assert!(outcome.skipped.iter().all(|s| s.reason.contains("내부 upstream")), "{:?}", outcome.skipped);
    assert!(outcome.rendered.conf.contains_key(&server_file(TO_ADMIN)), "an ordinary managed upstream stays");
    let outside = nwui_api::publisher::test_support::render_for_apply(&config().0, &data, &[]);
    assert!(outside.skipped.is_empty(), "{:?}", outside.skipped);
}

/// A variable host or port is decided per request (`?u=nwui_api` routes to the API's upstream),
/// so it is skipped in the image. Variables after a fixed authority keep the destination fixed.
#[test]
fn in_the_image_a_variable_host_or_port_is_skipped_but_a_variable_path_is_not() {
    let data = ConfigSnapshotData {
        upstreams: vec![],
        servers: vec![
            server(TO_API, "a.example.com", 80, true, json!({"proxyPass":"http://$arg_u"})),
            server(TO_DB_LOCATION, "b.example.com", 80, true, json!({"locations":[{"path":"/ok","proxyPass":"http://10.0.0.5:80"},{"path":"/p","proxyPass":"http://127.0.0.1:$arg_p/x"}]})),
            server(TO_ADMIN, "c.example.com", 80, true, json!({"locations":[{"path":"/","proxyPass":"http://10.0.0.5:8080/$1?q=$arg_q"}]})),
        ],
        streams: vec![],
    };
    let outcome = nwui_api::publisher::test_support::render_for_apply(&container_config().0, &data, &[]);
    let skipped: std::collections::BTreeSet<_> = outcome.skipped.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(skipped, [TO_API, TO_DB_LOCATION].into_iter().collect(), "{:?}", outcome.skipped);
    // A variable host is refused by the authority rule; a variable port is not even a URL.
    let api = outcome.skipped.iter().find(|s| s.id == TO_API).unwrap();
    assert!(api.reason.contains("변수"), "{api:?}");
    assert!(outcome.rendered.conf.contains_key(&server_file(TO_ADMIN)), "a variable path behind a fixed host stays");
    let outside = nwui_api::publisher::test_support::render_for_apply(&config().0, &data, &[]);
    assert!(!outside.skipped.iter().any(|s| s.id == TO_API), "{:?}", outside.skipped);
}

#[test]
fn fxb2_tls_preview_uses_only_readiness_metadata_and_preserves_fail_closed_rendering() {
    let (config,_root)=config();let certificate="cccccccc-cccc-4ccc-8ccc-cccccccccccc";
    let mut row=server(OK,"tls-preview.test",18086,true,json!({"proxyPass":"http://127.0.0.1:9000","tlsPort":18443}));row["certificateId"]=json!(certificate);
    let ready=nwui_api::control::views::render_preview(&config,nwui_api::control::resources::Resource::Server,&row,&[],&[json!({"id":certificate,"tlsReady":true})]);
    assert!(ready["error"].is_null(),"issued metadata-only preview failed: {ready}");
    let text=ready["config"].as_str().unwrap();assert!(text.contains("ssl_certificate "));assert!(text.contains(certificate));assert!(text.contains("18443 ssl"));
    let unavailable=nwui_api::control::views::render_preview(&config,nwui_api::control::resources::Resource::Server,&row,&[],&[json!({"id":certificate,"tlsReady":false})]);
    assert!(unavailable["config"].is_null());assert!(unavailable["error"].as_str().unwrap().contains(certificate));
}
