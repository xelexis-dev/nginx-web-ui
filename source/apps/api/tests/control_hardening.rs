mod auth_support;
mod g13_support;
use nwui_api::{
    config::RuntimeConfig,
    contracts::{Actor, FormInput},
    control::{
        model::{self, ConfigSnapshotData, CertificateRef},
        render,
        resources::{self, Mutation, Resource},
        validation::{self, DirectiveContext},
    },
};
use serde_json::{Value, json};
const GOOD: &str = "11111111-1111-4111-8111-111111111111";
const BAD: &str = "22222222-2222-4222-8222-222222222222";
const CERT: &str = "33333333-3333-4333-8333-333333333333";
fn config() -> (RuntimeConfig, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let mut c = RuntimeConfig::read(|k| match k {
        "DATABASE_URL" => Some("postgres://test@127.0.0.1:5432/test".into()),
        "AUTH_SECRET" => Some("test".into()),
        _ => None,
    })
    .unwrap();
    c.target_dns.descriptor_path = temp.path().join("dns.json");
    (c, temp)
}
fn input() -> Value {
    json!({"serverName":"good.test","listen":80,"proxyPass":"http://127.0.0.1:9000"})
}
fn row(id: &str, port: u16, cfg: Value) -> Value {
    json!({"id":id,"serverName":"good.test","listen":port,"enabled":true,"config":cfg,"certificateId":null,"upstreamId":null})
}
fn cert() -> CertificateRef { CertificateRef { id:CERT.into(), status:"issued".into(), usable:true } }
fn skipped(bad: Value, upstreams: Vec<Value>) {
    let (c, _t) = config();
    let data = ConfigSnapshotData {
        upstreams,
        servers: vec![
            row(GOOD, 80, json!({"proxyPass":"http://127.0.0.1:9000"})),
            bad,
        ],
        ..Default::default()
    };
    let r = g13_support::render(&c, &data, &[cert()]);
    assert!(r.rendered.conf.contains_key(&format!("server-{GOOD}.conf")));
    assert!(!r.rendered.conf.contains_key(&format!("server-{BAD}.conf")));
    assert!(!r.skipped.is_empty());
}
#[test]
fn m2_admin_listen_save() {
    let mut r = input();
    r["listen"] = json!(8080);
    assert!(resources::parse_server(&r, &Default::default()).is_err());
}
#[test]
fn m2_admin_listen_render() {
    skipped(
        row(BAD, 8080, json!({"proxyPass":"http://127.0.0.1:9000"})),
        vec![],
    );
}
#[test]
fn s13_reserved_upstream_save() {
    for n in ["nwui_api", "NWUI_WEB"] {
        assert!(
            resources::parse_upstream(&json!({"name":n,"servers":["127.0.0.1:9000"]})).is_err(),
            "{n}"
        );
    }
}
#[test]
fn s13_reserved_upstream_render() {
    for n in ["nwui_api", "NWUI_WEB"] {
        let (c, _t) = config();
        let data = ConfigSnapshotData {
            upstreams: vec![json!({"id":BAD,"name":n,"servers":["127.0.0.1:9000"]})],
            ..Default::default()
        };
        let r = g13_support::render(&c, &data, &[]);
        assert_eq!(r.skipped.len(), 1, "{n}");
        assert!(
            String::from_utf8_lossy(&r.rendered.conf["upstreams.conf"])
                .trim()
                .is_empty()
        );
    }
}
#[test]
fn s14_tls_plain_ports_save() {
    let (c, _t) = config();
    let data = ConfigSnapshotData {
        servers: vec![row(GOOD, 9090, json!({"tlsPort":9443}))],
        ..Default::default()
    };
    let ports = model::ports_for_snapshot(&c, &data);
    for port in [80, 8080, 9090] {
        let mut r = input();
        r["certificateId"] = json!(CERT);
        r["tls"] = json!({"tlsPort":port});
        assert!(
            resources::parse_server(&r, &ports).is_err(),
            "TLS on {port}"
        );
    }
    let mut r = input();
    r["listen"] = json!(9443);
    assert!(resources::parse_server(&r, &ports).is_err());
}
#[test]
fn s14_tls_plain_ports_render() {
    for port in [80, 8080] {
        let mut r = row(
            BAD,
            8081,
            json!({"proxyPass":"http://127.0.0.1:9000","tlsPort":port}),
        );
        r["certificateId"] = json!(CERT);
        skipped(r, vec![]);
    }
}
#[test]
fn s1_admin_proxy_headers() {
    for dirs in [
        Value::Null,
        json!([]),
        json!([{"name":"proxy_set_header","args":["Host","$host"]}]),
    ] {
        let mut cfg =
            json!({"serverName":"admin.test","listen":80,"proxyPass":"http://127.0.0.1:8080"});
        if !dirs.is_null() {
            cfg["locationDirectives"] = dirs;
        }
        let out = render::render_reverse_proxy(&cfg).unwrap();
        assert!(out.contains("proxy_set_header X-Real-IP $remote_addr;"));
        assert!(out.contains("proxy_set_header X-Forwarded-Proto $scheme;"));
    }
}
#[test]
fn s1_admin_proxy_override_save() {
    for name in ["x-real-ip", "X-Forwarded-Proto"] {
        let mut r = input();
        r["proxyPass"] = json!("http://127.0.0.1:8080");
        r["locationDirectives"] = json!([{"name":"proxy_set_header","args":[name,"spoof"]}]);
        assert!(resources::parse_server(&r, &Default::default()).is_err());
    }
}
#[test]
fn s1_admin_proxy_override_render() {
    let (c, _t) = config();
    let data = ConfigSnapshotData {
        servers: vec![row(BAD, 8081, json!({"proxyPass":"http://127.0.0.1:8080","locationDirectives":[{"name":"proxy_set_header","args":["X-Real-IP","spoof"]}]}))],
        ..Default::default()
    };
    let rendered = g13_support::render(&c, &data, &[]);
    assert!(rendered.skipped.is_empty());
    let text = String::from_utf8_lossy(&rendered.rendered.conf[&format!("server-{BAD}.conf")]);
    assert_eq!(text.matches("proxy_set_header X-Real-IP $remote_addr;").count(), 1);
    assert_eq!(text.matches("proxy_set_header X-Forwarded-Proto $scheme;").count(), 1);
    assert!(!text.contains("spoof"));
}
#[test]
fn q23_unix_proxy_target_only_refuses_internal_sockets() {
    // A.27 (user Q23) supersedes I14: allow legacy external sockets, block only internal ones.
    assert!(validation::assert_valid_proxy_target("http://unix:/var/run/secret.sock:/").is_ok());
    let error=validation::assert_valid_proxy_target("http://unix:/var/run/nwui-publisher/control.sock:/").unwrap_err();
    assert!(error.to_string().contains("이 컨테이너 내부 소켓은 프록시 대상으로 쓸 수 없습니다."));
}
#[test]
fn m1_size_forms() {
    for v in ["10mb", "1kb", "-1", "18446744073709551616g"] {
        assert!(
            validation::assert_valid_directive(
                "client_max_body_size",
                &[v.into()],
                DirectiveContext::Location
            )
            .is_err(),
            "{v}"
        );
    }
}
#[test]
fn m1_time_forms() {
    for v in ["maybe", "10mb", "-1"] {
        assert!(
            validation::assert_valid_directive(
                "proxy_read_timeout",
                &[v.into()],
                DirectiveContext::Location
            )
            .is_err(),
            "{v}"
        );
    }
}
#[test]
fn m1_on_off_forms() {
    for n in ["proxy_buffering", "gzip", "server_tokens"] {
        let ctx = if n == "proxy_buffering" {
            DirectiveContext::Location
        } else {
            DirectiveContext::Server
        };
        assert!(
            validation::assert_valid_directive(n, &["maybe".into()], ctx).is_err(),
            "{n}"
        );
    }
}
#[test]
fn m1_trailing_backslash() {
    assert!(
        validation::assert_valid_directive(
            "return",
            &["302".into(), "/path\\".into()],
            DirectiveContext::Location
        )
        .is_err()
    );
}
#[test]
fn m1_duplicate_locations() {
    let mut r = input();
    r["targetMode"] = json!("locations");
    r["locations"] = json!([{"path":"/","mode":"direct","proxyPass":"http://127.0.0.1:9000"},{"path":"/","mode":"direct","proxyPass":"http://127.0.0.1:9001"}]);
    assert!(resources::parse_server(&r, &Default::default()).is_err());
}
#[test]
fn m1_duplicate_editor_and_advanced() {
    let mut form = FormInput::default();
    form.fields.insert(
        "locationDirectives".into(),
        vec![json!({"bodySize":"10m","advanced":"client_max_body_size 20m;"}).to_string()],
    );
    assert!(resources::form_body(Resource::Server, &form).is_err());
}
#[test]
fn m1_bad_legacy_directive_is_skipped() {
    skipped(
        row(
            BAD,
            8081,
            json!({"proxyPass":"http://127.0.0.1:9000","locationDirectives":[{"name":"client_max_body_size","args":["10mb"]}]}),
        ),
        vec![],
    );
}
async fn referenced_upstream(operation: Mutation) {
    let db = auth_support::test_db().await;
    let actor = Actor {
        id: None,
        label: Some("test".into()),
        ip: None,
    };
    let u = resources::mutate(
        &db.state,
        Resource::Upstream,
        Mutation::Create,
        None,
        json!({"name":"backend","servers":["127.0.0.1:9000"]}),
        &actor,
    )
    .await
    .unwrap();
    let id = u["id"].as_str().unwrap();
    // Unsaved legacy literals still block rename/delete; new saves deliberately store IDs.
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('legacy-reference','ref.test',80,$1)",&[&json!({"locations":[{"path":"/","proxyPass":"http://backend"}]})]).await.unwrap();
    let e = resources::mutate(
        &db.state,
        Resource::Upstream,
        operation,
        Some(id),
        json!({"name":"renamed","servers":["127.0.0.1:9000"]}),
        &actor,
    )
    .await
    .expect_err("referenced location must block upstream rename/delete");
    assert_eq!(e.status, 409);
    assert!(e.message.contains("location"));
    assert_eq!(
        db.client
            .query_one("SELECT name FROM upstreams WHERE id=$1", &[&id])
            .await
            .unwrap()
            .get::<_, String>(0),
        "backend"
    );
}
#[tokio::test]
#[ignore = "requires disposable DB"]
async fn s12_referenced_rename() {
    referenced_upstream(Mutation::Update).await;
}
#[tokio::test]
#[ignore = "requires disposable DB"]
async fn s12_referenced_delete() {
    referenced_upstream(Mutation::Delete).await;
}
#[cfg(unix)]
fn fake_nginx(temp: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = temp.join("nginx");
    std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}
#[cfg(unix)]
#[tokio::test]
async fn m2_guard_quarantines_admin_listener() {
    let (c, t) = config();
    let mut c = c;
    let conf = t.path().join("conf");
    let stream = t.path().join("stream");
    std::fs::create_dir_all(&conf).unwrap();
    std::fs::create_dir_all(&stream).unwrap();
    let main = t.path().join("main.conf");
    std::fs::write(&main, "server {listen 80;} server {listen 8080;}").unwrap();
    std::fs::write(conf.join("server-bad.conf"), "server {listen 8080;}").unwrap();
    std::fs::write(conf.join("server-good.conf"), "server {listen 8081;}").unwrap();
    c.nginx_conf_dir = conf.clone();
    c.nginx_stream_dir = stream;
    c.nginx_bin = fake_nginx(t.path());
    c.nginx_main_conf=main;
    g13_support::metadata(&c.nginx_bin,&c.nginx_main_conf);
    std::fs::create_dir(t.path().join("publisher-state")).unwrap();
    let store=nwui_api::publisher::test_support::open_store_fixture(&t.path().join("publisher-state")).unwrap();
    let r=nwui_api::publisher::boot::guard(&c,&store).await.unwrap();
    assert_eq!(r.reserved_files, 1);
    assert!(!conf.join("server-bad.conf").exists());
    assert!(conf.join("server-good.conf").exists());
}
#[cfg(unix)]
#[tokio::test]
#[ignore = "requires disposable DB"]
async fn upgrade_rows_skip_and_apply_healthy_proxy() {
    let t = tempfile::tempdir().unwrap();
    let bin = fake_nginx(t.path());
    let conf = t.path().join("conf");
    let stream = t.path().join("stream");
    let certs = t.path().join("certs");
    let mut db = auth_support::test_db_with(&[
        ("NGINX_BIN", bin.to_str().unwrap()),
        ("NGINX_CONF_D", conf.to_str().unwrap()),
        ("NGINX_STREAM_D", stream.to_str().unwrap()),
        ("CERTS_DIR", certs.to_str().unwrap()),
    ])
    .await;
    g13_support::witness(&mut db.state);
    let pair=rcgen::generate_simple_self_signed(vec!["good.test".into()]).unwrap();
    let sealed = db.state.secrets.seal(&pair.signing_key.serialize_pem()).unwrap();
    db.client.execute("INSERT INTO certificates(id,domains,status,cert_pem,key_pem) VALUES($1,'[\"good.test\"]','issued',$2,$3)",&[&CERT,&pair.cert.pem(),&sealed]).await.unwrap();
    db.client.execute("INSERT INTO upstreams(id,name,servers) VALUES('reserved','NWUI_API','[\"127.0.0.1:9000\"]')",&[]).await.unwrap();
    let mut bad_tls = row(
        "bad-tls",
        8083,
        json!({"proxyPass":"http://127.0.0.1:9000","tlsPort":80}),
    );
    bad_tls["certificateId"] = json!(CERT);
    let bad = [
        row(BAD, 8080, json!({"proxyPass":"http://127.0.0.1:9000"})),
        row(
            "bad-size",
            8081,
            json!({"proxyPass":"http://127.0.0.1:9000","locationDirectives":[{"name":"client_max_body_size","args":["10mb"]}]}),
        ),
        row(
            "bad-paths",
            8085,
            json!({"locations":[{"path":"/","proxyPass":"http://127.0.0.1:9000"},{"path":"/","proxyPass":"http://127.0.0.1:9001"}]}),
        ),
        row(
            "bad-duplicates",
            8086,
            json!({"proxyPass":"http://127.0.0.1:9000","locationDirectives":[{"name":"proxy_read_timeout","args":["10s"]},{"name":"proxy_read_timeout","args":["20s"]}]}),
        ),
        bad_tls,
    ];
    for r in [row(GOOD, 80, json!({"proxyPass":"http://127.0.0.1:9000"})),row("legacy-unix",8082,json!({"proxyPass":"http://unix:/var/run/secret.sock:/"})),row("stored-admin-header",8084,json!({"proxyPass":"http://127.0.0.1:8080","locationDirectives":[{"name":"proxy_set_header","args":["X-Real-IP","spoof"]}]}))].into_iter().chain(bad)
    {
        db.client
            .execute(
                "INSERT INTO nginx_servers(id,server_name,listen,config,certificate_id) VALUES($1,$2,$3,$4,$5)",
                &[
                    &r["id"].as_str().unwrap(),
                    &r["serverName"].as_str().unwrap(),
                    &(r["listen"].as_i64().unwrap() as i32),
                    &r["config"],
                    &r["certificateId"].as_str(),
                ],
            )
            .await
            .unwrap();
    }
    let result = nwui_api::publisher_client::apply_saved(
        &db.state,
        &Actor {
            id: None,
            label: Some("upgrade".into()),
            ip: None,
        },
        None,
    )
    .await
    .unwrap();
    assert!(result.ok, "{}", result.output);
    assert!(conf.join(format!("server-{GOOD}.conf")).exists());
    assert!(!conf.join(format!("server-{BAD}.conf")).exists());
    assert!(std::fs::read_to_string(conf.join("server-legacy-unix.conf")).unwrap().contains("proxy_pass http://unix:/var/run/secret.sock:/;"));
    let admin = std::fs::read_to_string(conf.join("server-stored-admin-header.conf")).unwrap();
    assert_eq!(admin.matches("proxy_set_header X-Real-IP $remote_addr;").count(), 1);
    assert_eq!(admin.matches("proxy_set_header X-Forwarded-Proto $scheme;").count(), 1);
    assert!(!admin.contains("spoof"));
    assert_eq!(
        db.client
            .query_one(
                "SELECT jsonb_array_length(data->'skipped') FROM applied_config WHERE id='current'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, i32>(0),
        6
    );
    assert_eq!(
        db.client
            .query_one(
                "SELECT count(*) FROM notifications WHERE type='config.apply-skipped'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
}

#[test]
fn m1_valid_measurements_and_repeatable_directives_stay_supported() {
    for (name, args, ctx) in [
        (
            "client_max_body_size",
            vec!["1g"],
            DirectiveContext::Location,
        ),
        (
            "proxy_read_timeout",
            vec!["100ms"],
            DirectiveContext::Location,
        ),
        (
            "proxy_read_timeout",
            vec!["1h30m"],
            DirectiveContext::Location,
        ),
        (
            "keepalive_timeout",
            vec!["75s", "60"],
            DirectiveContext::Server,
        ),
        ("expires", vec!["@15h30m"], DirectiveContext::Location),
        (
            "expires",
            vec!["modified", "+24h"],
            DirectiveContext::Location,
        ),
        ("server_tokens", vec!["build"], DirectiveContext::Server),
    ] {
        validation::assert_valid_directive(
            name,
            &args.into_iter().map(String::from).collect::<Vec<_>>(),
            ctx,
        )
        .unwrap();
    }
    for args in [
        vec!["@24h"],
        vec!["modified", "off"],
        vec!["modified", "@10h"],
        vec!["10ms"],
    ] {
        assert!(
            validation::assert_valid_directive(
                "expires",
                &args.into_iter().map(String::from).collect::<Vec<_>>(),
                DirectiveContext::Location
            )
            .is_err()
        );
    }
    let mut raw = input();
    raw["locationDirectives"] = json!([{"name":"add_header","args":["X-One","one"]},{"name":"add_header","args":["X-Two","two"]},{"name":"proxy_redirect","args":["http://old/","https://new/"]},{"name":"proxy_redirect","args":["http://older/","https://newer/"]}]);
    resources::parse_server(&raw, &Default::default()).unwrap();
}
#[test]
fn s14_cross_row_render_isolates_the_conflicting_later_row() {
    for tls_first in [true, false] {
        let (c, _temp) = config();
        let mut tls = row(
            if tls_first { GOOD } else { BAD },
            8081,
            json!({"proxyPass":"http://127.0.0.1:9000","tlsPort":9443}),
        );
        tls["certificateId"] = json!(CERT);
        let http = row(
            if tls_first { BAD } else { GOOD },
            9443,
            json!({"proxyPass":"http://127.0.0.1:9000"}),
        );
        let rows = if tls_first {
            vec![tls, http]
        } else {
            vec![http, tls]
        };
        let out = g13_support::render(
            &c,
            &ConfigSnapshotData {
                servers: rows,
                ..Default::default()
            },
            &[cert()],
        );
        assert!(
            out.rendered
                .conf
                .contains_key(&format!("server-{GOOD}.conf"))
        );
        assert!(
            !out.rendered
                .conf
                .contains_key(&format!("server-{BAD}.conf"))
        );
        assert_eq!(out.skipped.len(), 1);
    }
}

#[test]
fn s13_reserved_legacy_upstream_also_skips_case_insensitive_location_references() {
    let (c, _temp) = config();
    let data = ConfigSnapshotData {
        upstreams: vec![json!({"id":"reserved","name":"NWUI_API","servers":["127.0.0.1:9000"]})],
        servers: vec![
            row(GOOD, 80, json!({"proxyPass":"http://127.0.0.1:9000"})),
            row(
                BAD,
                8081,
                json!({"locations":[{"path":"/","proxyPass":"http://nwui_api/path"}]}),
            ),
        ],
        ..Default::default()
    };
    let out = g13_support::render(&c, &data, &[]);
    assert!(
        out.rendered
            .conf
            .contains_key(&format!("server-{GOOD}.conf"))
    );
    assert!(
        !out.rendered
            .conf
            .contains_key(&format!("server-{BAD}.conf"))
    );
    assert_eq!(out.skipped.len(), 2);
}

#[test]
fn review_mixed_case_flags_survive_upgrade() {
    let mut cfg =
        json!({"serverName":"legacy.test","listen":8090,"proxyPass":"http://127.0.0.1:9000"});
    cfg["serverDirectives"] =
        json!([{"name":"gzip","args":["On"]},{"name":"server_tokens","args":["Build"]}]);
    cfg["locationDirectives"] = json!([{"name":"proxy_buffering","args":["OFF"]}]);
    resources::parse_server(&cfg, &Default::default()).unwrap();
    let (c, _t) = config();
    let data = ConfigSnapshotData {
        servers: vec![row(GOOD, 8090, cfg)],
        ..Default::default()
    };
    let result = g13_support::render(&c, &data, &[]);
    assert!(result.skipped.is_empty());
    let text = String::from_utf8_lossy(&result.rendered.conf[&format!("server-{GOOD}.conf")]);
    assert!(text.contains("gzip On;"));
    assert!(text.contains("server_tokens Build;"));
    assert!(text.contains("proxy_buffering OFF;"));
}

#[tokio::test]
#[ignore = "requires disposable DB"]
async fn review_s14_edit_excludes_own_ports_but_not_other_rows() {
    let db = auth_support::test_db().await;
    let actor = Actor {
        id: None,
        label: Some("test".into()),
        ip: None,
    };
    let pair=rcgen::generate_simple_self_signed(vec!["good.test".into()]).unwrap();
    let sealed = db.state.secrets.seal(&pair.signing_key.serialize_pem()).unwrap();
    db.client.execute("INSERT INTO certificates(id,domains,status,cert_pem,key_pem) VALUES($1,'[\"good.test\"]','issued',$2,$3)",&[&CERT,&pair.cert.pem(),&sealed]).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config,certificate_id) VALUES($1,'good.test',9090,$2,$3)",&[&GOOD,&json!({"proxyPass":"http://127.0.0.1:9000","tlsPort":9443}),&CERT]).await.unwrap();
    let swapped = json!({"serverName":"good.test","listen":9443,"proxyPass":"http://127.0.0.1:9000","certificateId":CERT,"tls":{"tlsPort":9090}});
    resources::mutate(
        &db.state,
        Resource::Server,
        Mutation::Update,
        Some(GOOD),
        swapped.clone(),
        &actor,
    )
    .await
    .unwrap();
    db.client
        .execute(
            "UPDATE nginx_servers SET listen=9090,config=$1 WHERE id=$2",
            &[
                &json!({"proxyPass":"http://127.0.0.1:9000","tlsPort":9443}),
                &GOOD,
            ],
        )
        .await
        .unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config,certificate_id) VALUES($1,'other.test',9091,$2,$3)",&[&BAD,&json!({"proxyPass":"http://127.0.0.1:9000","tlsPort":9443}),&CERT]).await.unwrap();
    let error = resources::mutate(
        &db.state,
        Resource::Server,
        Mutation::Update,
        Some(GOOD),
        swapped,
        &actor,
    )
    .await
    .expect_err("another row still owns the old TLS port");
    assert_eq!(error.status, 400);
    assert!(error.field_errors.as_ref().unwrap().contains_key("listen"));
    let r = db
        .client
        .query_one(
            "SELECT listen,config->>'tlsPort' FROM nginx_servers WHERE id=$1",
            &[&GOOD],
        )
        .await
        .unwrap();
    assert_eq!(r.get::<_, i32>(0), 9090);
    assert_eq!(r.get::<_, String>(1), "9443");
}

#[tokio::test]
#[ignore = "requires disposable DB"]
async fn review_s12_unreferenced_rename_and_same_name_update_are_allowed() {
    let db = auth_support::test_db().await;
    let actor = Actor {
        id: None,
        label: Some("test".into()),
        ip: None,
    };
    let u = resources::mutate(
        &db.state,
        Resource::Upstream,
        Mutation::Create,
        None,
        json!({"name":"backend","servers":["127.0.0.1:9000"]}),
        &actor,
    )
    .await
    .unwrap();
    let id = u["id"].as_str().unwrap();
    resources::mutate(
        &db.state,
        Resource::Upstream,
        Mutation::Update,
        Some(id),
        json!({"name":"renamed","servers":["127.0.0.1:9000"]}),
        &actor,
    )
    .await
    .unwrap();
    // Unsaved legacy literals still block rename/delete; new saves deliberately store IDs.
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('legacy-reference','ref.test',80,$1)",&[&json!({"locations":[{"path":"/","proxyPass":"http://backend"}]})]).await.unwrap();
    resources::mutate(
        &db.state,
        Resource::Upstream,
        Mutation::Update,
        Some(id),
        json!({"name":"renamed","servers":["127.0.0.1:9001"]}),
        &actor,
    )
    .await
    .unwrap();
}

#[test]
fn review_unspecified_loopback_admin_target_is_protected() {
    let text = render::render_reverse_proxy(
        &json!({"serverName":"admin.test","listen":80,"proxyPass":"http://0.0.0.0:8080"}),
    )
    .unwrap();
    assert!(text.contains("proxy_set_header X-Real-IP $remote_addr;"));
}

#[test]
fn review_acme_path_is_reserved_at_save_and_legacy_render() {
    let mut cfg = input();
    cfg["targetMode"] = json!("locations");
    cfg["locations"] = json!([{"path":"/.well-known/acme-challenge/","mode":"direct","proxyPass":"http://127.0.0.1:9000"}]);
    assert!(resources::parse_server(&cfg, &Default::default()).is_err());
    skipped(row(BAD, 8090, cfg), vec![]);
}

#[test]
fn review_proxy_version_and_return_values_match_nginx() {
    for (name, args) in [
        ("proxy_http_version", vec!["3"]),
        ("return", vec!["abc"]),
        ("return", vec!["1000"]),
    ] {
        assert!(
            validation::assert_valid_directive(
                name,
                &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                DirectiveContext::Location
            )
            .is_err()
        );
    }
    for (name, args) in [
        ("proxy_http_version", vec!["1.0"]),
        ("proxy_http_version", vec!["1.1"]),
        ("proxy_http_version", vec!["2"]),
        ("return", vec!["0"]),
        ("return", vec!["999", "text"]),
        ("return", vec!["http://example.test/"]),
        ("return", vec!["$scheme://example.test/"]),
    ] {
        validation::assert_valid_directive(
            name,
            &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            DirectiveContext::Location,
        )
        .unwrap();
    }
}

#[test]
fn resolver_refusal_has_shared_runtime_dns_hint() {
    let fixture: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packages/presentation/src/nginx/directive-refusals.fixture.json"
    )))
    .unwrap();
    for row in fixture["cases"].as_array().unwrap() {
        let name = row["name"].as_str().unwrap();
        let argument = row["argument"].as_str().unwrap();
        let context = row["context"].as_str().unwrap();
        let ctx = if context == "server" {
            DirectiveContext::Server
        } else {
            DirectiveContext::Location
        };
        let expected = format!(
            "Directive not allowed in {context} context: \"{name}\" {}",
            fixture["hint"].as_str().unwrap()
        );
        assert_eq!(
            validation::assert_valid_directive(name, &[argument.into()], ctx)
                .unwrap_err()
                .to_string(),
            expected
        );
        assert_eq!(
            validation::parse_freeform_directives(&format!("{name} {argument};"), ctx)
                .unwrap_err()
                .to_string(),
            expected
        );
    }
    assert_eq!(
        validation::assert_valid_directive("include", &["/tmp/config".into()], DirectiveContext::Server)
            .unwrap_err()
            .to_string(),
        "Directive not allowed in server context: \"include\""
    );
    assert!(validation::assert_valid_directive(
        "proxy_read_timeout",
        &["60s".into()],
        DirectiveContext::Location
    )
    .is_ok());
}
