#![cfg(unix)]
mod auth_support;
mod g13_support;
use nwui_api::{runtime_boot, state::AppState};
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;
async fn setup() -> (auth_support::TestDb, TempDir) {
    let temp = TempDir::new().unwrap();
    let nginx = temp.path().join("nginx");
    let conf = temp.path().join("conf");
    let stream = temp.path().join("stream");
    let certs = temp.path().join("certs");
    let main = temp.path().join("nginx.conf");
    let quarantine = temp.path().join("quarantine");
    let log = temp.path().join("commands");
    std::fs::create_dir_all(&conf).unwrap();
    std::fs::create_dir_all(&stream).unwrap();
    std::fs::write(&main, "listen 80;\nlisten 443 ssl;\nlisten 8080;\n").unwrap();
    std::fs::write(&nginx,format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif grep -rq unresolved '{}' '{}'; then echo 'host not found'; exit 1; fi\nexit 0\n",log.display(),conf.display(),stream.display())).unwrap();
    std::fs::set_permissions(&nginx, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut t = auth_support::test_db_with(&[
        (
            "MIGRATIONS_DIR",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../packages/db/drizzle"),
        ),
        ("NGINX_BIN", nginx.to_str().unwrap()),
        ("NGINX_CONF_D", conf.to_str().unwrap()),
        ("NGINX_STREAM_D", stream.to_str().unwrap()),
        ("CERTS_DIR", certs.to_str().unwrap()),
        ("NGINX_CONF", main.to_str().unwrap()),
        ("NGINX_GUARD_QUARANTINE", quarantine.to_str().unwrap()),
    ])
    .await;
    g13_support::witness(&mut t.state);
    (t, temp)
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn api_port_upgrade_conflict_fails_before_disk_mutation() {
    let (t, temp) = setup().await;
    std::fs::write(
        t.state.config.nginx_conf_dir.join("server-old.conf"),
        "LAST-GOOD",
    )
    .unwrap();
    t.client.execute("INSERT INTO nginx_servers(id,server_name,listen,enabled) VALUES('old-rule','legacy.example',3001,false)",&[]).await.unwrap();
    let error = runtime_boot::preflight_api_port(&t.state)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("3001"));
    assert!(error.contains("old-rule"));
    assert!(error.contains("API_PORT"));
    assert!(!temp.path().join("commands").exists());
    assert_eq!(
        std::fs::read_to_string(t.state.config.nginx_conf_dir.join("server-old.conf")).unwrap(),
        "LAST-GOOD"
    );
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn api_port_conflict_names_the_rule_and_says_disabled_rules_count() {
    let (t, _temp) = setup().await;
    t.client.execute("INSERT INTO nginx_servers(id,server_name,listen,enabled) VALUES('legacy','legacy.example.com',3001,false)",&[]).await.unwrap();
    let error = runtime_boot::preflight_api_port(&t.state).await.unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("legacy.example.com"), "{message}");
    assert!(message.contains("disabled"), "{message}");
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn overridden_api_port_leaves_legacy_3001_rule_and_udp_allowed() {
    let (t, _temp) = setup().await;
    t.client.execute("INSERT INTO nginx_streams(id,listen,protocol,proxy_pass) VALUES('udp',3001,'udp','127.0.0.1:9000')",&[]).await.unwrap();
    runtime_boot::preflight_api_port(&t.state).await.unwrap();
    t.client
        .execute(
            "INSERT INTO nginx_servers(id,server_name,listen) VALUES('legacy','example.com',3001)",
            &[],
        )
        .await
        .unwrap();
    let config = nwui_api::config::RuntimeConfig::read(|key| match key {
        "DATABASE_URL" => Some(t.state.config.database_url.clone()),
        "AUTH_SECRET" => Some("test".into()),
        "DATA_KEY_FILE" => Some(t.state.config.data_key_file.display().to_string()),
        "API_PORT" => Some("3002".into()),
        _ => None,
    })
    .unwrap();
    let other = AppState::new(config).unwrap();
    runtime_boot::preflight_api_port(&other).await.unwrap();
}


#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn fxb2_apply_excludes_unavailable_tls_and_keeps_other_servers() {
    use nwui_api::{contracts::Actor};
    let (t, temp) = setup().await;
    let certificate = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
    t.client.execute("INSERT INTO certificates(id,domains,status) VALUES($1,'[\"tls.test\"]','failed')", &[&certificate]).await.unwrap();
    t.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES('bad','tls.test',18086,$1,'{\"proxyPass\":\"http://127.0.0.1:9000\",\"tlsPort\":18443,\"redirectHttp\":true}'),('good','good.test',18087,NULL,'{\"proxyPass\":\"http://127.0.0.1:9000\"}')", &[&certificate]).await.unwrap();
    let actor = Actor { via: None, id: None, label: Some("fxb2".into()), ip: None };
    let mut violations = Vec::new();
    for (status, cert, key) in [("failed",Some("pem"),Some("key")),("issuing",Some("pem"),Some("key")),("issued",None,Some("key")),("issued",Some("pem"),None),("issued",Some(""),Some("key")),("issued",Some("pem"),Some(""))] {
        t.client.execute("UPDATE certificates SET status=$2::text::cert_status,cert_pem=$3,key_pem=$4 WHERE id=$1", &[&certificate,&status,&cert,&key]).await.unwrap();
        let applied = nwui_api::publisher_client::apply_saved(&t.state,&actor,None).await.unwrap();
        assert!(applied.ok,"{}",applied.output);
        assert!(temp.path().join("conf/server-good.conf").is_file());
        if temp.path().join("conf/server-bad.conf").exists() || !applied.output.contains("tls.test") || !applied.output.contains(certificate) { violations.push(format!("apply: {status}, cert={cert:?}, key present={}: {}",key.is_some(),applied.output)); }

    }
    assert!(violations.is_empty(),"unavailable TLS remained live or was not reported: {violations:?}");
}

#[tokio::test]
#[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
async fn fxb3_legacy_admin_and_mixed_upstreams_apply_with_mixed_notice() {
    use nwui_api::{contracts::Actor};
    let (mut t,temp)=setup().await;let mut config=(*t.state.config).clone();config.in_container=true;t.state.config=std::sync::Arc::new(config);
    t.client.batch_execute("INSERT INTO upstreams(id,name,servers) VALUES('admin','admin_ui','[\"localhost:8080\",\"127.0.0.1:8080\"]'),('mixed','mixed_ui','[\"127.0.0.1:8080\",\"10.0.0.5:9000\"]'); INSERT INTO nginx_servers(id,server_name,listen,upstream_id,config) VALUES('admin','admin.test',18086,'admin','{}'),('mixed','mixed.test',18087,'mixed','{}')").await.unwrap();
    let result=nwui_api::publisher_client::apply_saved(&t.state,&Actor{via: None,id:None,label:Some("fxb3".into()),ip:None},None).await.unwrap();assert!(result.ok,"{}",result.output);
    assert!(temp.path().join("conf/server-admin.conf").is_file(),"legacy admin route was excluded");assert!(temp.path().join("conf/server-mixed.conf").is_file(),"mixed route was excluded");
    assert!(result.output.contains("mixed_ui")&&result.output.contains("관리"),"mixed trust warning missing: {}",result.output);
    let count:i64=t.client.query_one("SELECT count(*) FROM notifications WHERE type='config.apply-notice' AND body LIKE '%mixed_ui%'",&[]).await.unwrap().get(0);assert_eq!(count,1,"mixed trust notice must persist");
    let groups=std::fs::read_to_string(temp.path().join("conf/upstreams.conf")).unwrap();let admin=groups.split("upstream admin_ui {").nth(1).unwrap().split('}').next().unwrap();assert!(!admin.contains("localhost"));assert_eq!(admin.matches("server 127.0.0.1:8080;").count(),1);
    let marker:serde_json::Value=t.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get(0);let stored=marker["upstreams"].as_array().unwrap().iter().find(|row|row["id"]=="admin").unwrap();assert_eq!(stored["servers"],serde_json::json!(["localhost:8080","127.0.0.1:8080"]));assert_eq!(nwui_api::control::pending::status(&t.state).await.unwrap().count,0,"generated normalization must not create pending changes");
    let repeated=nwui_api::publisher_client::apply_saved(&t.state,&Actor{via: None,id:None,label:Some("fxb3".into()),ip:None},None).await.unwrap();assert!(repeated.ok);
    let count:i64=t.client.query_one("SELECT count(*) FROM notifications WHERE type='config.apply-notice' AND body LIKE '%mixed_ui%'",&[]).await.unwrap().get(0);assert_eq!(count,1,"repeated applies must not duplicate the notice");
}

#[tokio::test]
#[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
async fn multihop_warning_persists_once_and_never_changes_pending_state() {
    use nwui_api::{contracts::Actor};
    let (mut t,temp)=setup().await;
    let mut config=(*t.state.config).clone();config.in_container=true;t.state.config=std::sync::Arc::new(config);
    t.client.batch_execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('cycle-a','cycle-a.test',18090,'{\"proxyPass\":\"http://127.0.0.1:18091/api\"}'),('cycle-b','cycle-b.test',18091,'{\"proxyPass\":\"http://127.0.0.1:18090/auth\"}')").await.unwrap();
    for _ in 0..2 {
        let result=nwui_api::publisher_client::apply_saved(&t.state,&Actor{via: None,id:None,label:Some("multihop".into()),ip:None},None).await.unwrap();
        assert!(result.ok,"{}",result.output);
        assert!(result.output.contains("같은 요청이 순환하면 누구나 nginx 연결을 소진시킬 수 있습니다"));
        for name in ["cycle-a","cycle-b"]{assert!(temp.path().join(format!("conf/server-{name}.conf")).is_file());}
        let marker:serde_json::Value=t.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get(0);
        assert!(marker.to_string().contains("cycle-a"));assert!(marker.to_string().contains("cycle-b"));
        assert_eq!(nwui_api::control::pending::status(&t.state).await.unwrap().count,0,"a warning must not mark working rows as pending");
    }
    let count:i64=t.client.query_one("SELECT count(*) FROM notifications WHERE type='config.apply-notice' AND body LIKE '%같은 요청이 순환하면 누구나 nginx 연결을 소진시킬 수 있습니다%'",&[]).await.unwrap().get(0);
    assert_eq!(count,1,"identical repeated applies must deduplicate the warning");
}

#[tokio::test]
#[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
async fn w11_postvalidation_isolation_is_saved_as_skipped_not_applied() {
    use nwui_api::{contracts::Actor};
    let (mut t,temp)=setup().await;
    let mut config=(*t.state.config).clone();config.in_container=true;t.state.config=std::sync::Arc::new(config);
    t.client.batch_execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('bad','bad.test',18090,'{\"proxyPass\":\"http://192.0.2.1:3001\"}'),('good','good.test',18091,'{\"proxyPass\":\"http://192.0.2.2:9000\"}')").await.unwrap();
    let bad=temp.path().join("conf/server-bad.conf");
    let once=temp.path().join("changed");
    std::fs::write(temp.path().join("nginx"),format!("#!/bin/sh\nif [ \"$*\" = '-t' ] && [ ! -e '{once}' ]; then touch '{once}'; sed -i 's/192.0.2.1/127.0.0.1/g' '{bad}'; fi\nexit 0\n",once=once.display(),bad=bad.display())).unwrap();
    let result=nwui_api::publisher_client::apply_saved(&t.state,&Actor{via: None,id:None,label:Some("w11".into()),ip:None},None).await.unwrap();
    assert!(result.ok,"{}",result.output);
    assert!(!bad.exists());assert!(temp.path().join("conf/server-good.conf").exists());
    let marker:serde_json::Value=t.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get(0);
    assert_eq!(marker["servers"].as_array().unwrap().len(),1);
    assert_eq!(marker["servers"][0]["id"],"good");
    assert_eq!(marker["skipped"][0]["id"],"bad");
    assert_eq!(nwui_api::control::pending::status(&t.state).await.unwrap().count,1);
}

#[tokio::test]
#[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
async fn fxb3_metrics_collector_uses_its_exclusive_loopback_source() {
    use tokio::io::{AsyncReadExt,AsyncWriteExt};
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let endpoint=listener.local_addr().unwrap();
    let server=tokio::spawn(async move {let (mut socket,peer)=listener.accept().await.unwrap();let mut request=[0u8;4096];socket.read(&mut request).await.unwrap();let body="Active connections: 1\nserver accepts handled requests\n 1 1 1\nReading: 0 Writing: 1 Waiting: 0\n";socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();peer.ip()});
    let (mut t,_temp)=setup().await;let mut config=(*t.state.config).clone();config.nginx_status_url=format!("http://{endpoint}/nginx_status");t.state.config=std::sync::Arc::new(config);
    assert!(nwui_api::observability::collector::read_nginx_metrics(&t.state).await.is_some());assert_eq!(server.await.unwrap().to_string(),"127.0.0.3");
}
