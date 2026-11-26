#![cfg(unix)]
use nwui_api::{
    config::RuntimeConfig,
    control::model,
    state::AppState,
};
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt, path::Path};
use tempfile::TempDir;
mod auth_support;
mod g13_support;



fn state(dir: &Path, body: &str) -> AppState {
    let bin = dir.join("nginx-fake");
    std::fs::write(&bin, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut config = RuntimeConfig::read(|key| {
        Some(match key {
            "DATABASE_URL" => "postgres://test:test@127.0.0.1:1/test".into(),
            "AUTH_SECRET" => "test-secret".into(),
            "DATA_KEY_FILE" => dir.join("key").display().to_string(),
            "NGINX_BIN" => bin.display().to_string(),
            "NGINX_CONF_D" => dir.join("conf").display().to_string(),
            "NGINX_STREAM_D" => dir.join("stream").display().to_string(),
            "CERTS_DIR" => dir.join("certs").display().to_string(),
            _ => return None,
        })
    })
    .unwrap();
    config.target_dns.descriptor_path = dir.join("target-dns.json");
    std::fs::write(&config.data_key_file,"A5".repeat(32)).unwrap();let mut state=AppState::new(config).unwrap();
    g13_support::witness(&mut state);
    state
}

























async fn real_db(temp: &TempDir) -> auth_support::TestDb {
    let fake = state(temp.path(), "exit 0");
    let paths = [
        fake.config.nginx_bin.display().to_string(),
        fake.config.nginx_conf_dir.display().to_string(),
        fake.config.nginx_stream_dir.display().to_string(),
        fake.config.certs_dir.display().to_string(),
    ];
    let mut db=auth_support::test_db_with(&[
        ("NGINX_BIN", &paths[0]),
        ("NGINX_CONF_D", &paths[1]),
        ("NGINX_STREAM_D", &paths[2]),
        ("CERTS_DIR", &paths[3]),
    ])
    .await;
    g13_support::witness(&mut db.state);
    db
}
fn actor() -> nwui_api::contracts::Actor {
    nwui_api::contracts::Actor {
        via: None,
        id: None,
        label: Some("test-control".into()),
        ip: None,
    }
}

fn certificate_input() -> nwui_api::certificates::service::CertificatePemInput {
    let material = rcgen::generate_simple_self_signed(vec!["example.test".into()]).unwrap();
    nwui_api::certificates::service::CertificatePemInput {
        cert_pem: material.cert.pem(),
        key_pem: material.signing_key.serialize_pem(),
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn certificate_replace_preserves_live_bytes_drafts_marker_and_history() {
    if g13_support::isolated("certificate_replace_preserves_live_bytes_drafts_marker_and_history").await { return; }
    use nwui_api::{certificates::service, control::pending};
    let temp = TempDir::new().unwrap();
    let db = real_db(&temp).await;
    let cert = service::import(&db.state, &certificate_input(), &actor()).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES('live','example.test',8081,$1,$2)", &[&cert, &serde_json::json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&db.state, &actor(), None).await.unwrap().ok);
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('draft','draft.example.test',8082,$1)", &[&serde_json::json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
    let before = FixtureFiles::capture(&db.state.config).await.unwrap();
    let marker: serde_json::Value = db.client.query_one("SELECT data FROM applied_config WHERE id='current'", &[]).await.unwrap().get(0);
    let snapshots: i64 = db.client.query_one("SELECT count(*) FROM config_snapshots", &[]).await.unwrap().get(0);
    let audits: i64 = db.client.query_one("SELECT count(*) FROM audit_logs WHERE action='nginx:apply'", &[]).await.unwrap().get(0);
    let replacement = certificate_input();
    let result = service::replace(&db.state, &cert, &replacement, &actor()).await.unwrap();
    assert!(result["applyError"].is_null(), "{result}");
    assert_eq!(pending::status(&db.state).await.unwrap().count, 1, "certificate replacement must not activate drafts");
    let after = FixtureFiles::capture(&db.state.config).await.unwrap();
    assert_eq!(before.conf, after.conf);
    assert_eq!(before.stream, after.stream);
    assert_eq!(std::fs::read_to_string(db.state.config.certs_dir.join(&cert).join("fullchain.pem")).unwrap(), replacement.cert_pem);
    assert_eq!(db.client.query_one("SELECT data FROM applied_config WHERE id='current'", &[]).await.unwrap().get::<_, serde_json::Value>(0), marker);
    assert_eq!(db.client.query_one("SELECT count(*) FROM config_snapshots", &[]).await.unwrap().get::<_, i64>(0), snapshots);
    assert_eq!(db.client.query_one("SELECT count(*) FROM audit_logs WHERE action='nginx:apply'", &[]).await.unwrap().get::<_, i64>(0), audits + 1);
}

#[tokio::test]
#[ignore="real DB: set NWUI_TEST_DATABASE_URL"]
async fn certificate_deploy_refuses_preexisting_live_drift_without_writes_or_a8_change() {
    if g13_support::isolated("certificate_deploy_refuses_preexisting_live_drift_without_writes_or_a8_change").await {return;}
    use nwui_api::certificates::service;
    let temp=TempDir::new().unwrap();let db=real_db(&temp).await;
    let cert=service::import(&db.state,&certificate_input(),&actor()).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES('live','example.test',8081,$1,$2)",&[&cert,&serde_json::json!({"proxyPass":"http://192.0.2.1:3001"})]).await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&db.state,&actor(),None).await.unwrap().ok);
    let file=db.state.config.nginx_conf_dir.join("server-live.conf");
    let changed=std::fs::read_to_string(&file).unwrap().replace("192.0.2.1","127.0.0.1");std::fs::write(&file,changed).unwrap();
    let before=FixtureFiles::capture(&db.state.config).await.unwrap();let a8=db.state.config.nginx_conf_dir.join(".nwui-apply-uncertain");let a8_before=std::fs::read(&a8).ok();
    let paths=[db.state.config.certs_dir.join(&cert).join("fullchain.pem"),db.state.config.certs_dir.join(&cert).join("privkey.pem")];let material:Vec<_>=paths.iter().map(|p|std::fs::read(p).unwrap()).collect();
    let result=service::replace(&db.state,&cert,&certificate_input(),&actor()).await.unwrap();
    assert_eq!(result["applyError"],"Live payload differs from its durable generation");
    let after=FixtureFiles::capture(&db.state.config).await.unwrap();assert_eq!(before.conf,after.conf);assert_eq!(before.stream,after.stream);
    assert_eq!(std::fs::read(a8).ok(),a8_before);assert_eq!(paths.iter().map(|p|std::fs::read(p).unwrap()).collect::<Vec<_>>(),material);
}

#[tokio::test]
#[ignore="real DB: set NWUI_TEST_DATABASE_URL"]
async fn w11_certificate_scan_io_error_does_not_fail_confirmed_deploy() {
    if g13_support::isolated("w11_certificate_scan_io_error_does_not_fail_confirmed_deploy").await { return; }
    use nwui_api::certificates::service;
    let temp=TempDir::new().unwrap();
    let db=real_db(&temp).await;
    let cert=service::import(&db.state,&certificate_input(),&actor()).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES('live','example.test',8081,$1,$2)",&[&cert,&serde_json::json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&db.state,&actor(),None).await.unwrap().ok);
    let file=db.state.config.nginx_conf_dir.join("server-live.conf");
    let before=std::fs::read(&file).unwrap();
    let marker:serde_json::Value=db.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get(0);
    let bad=db.state.config.nginx_conf_dir.join("server-scan-error.conf");
    std::fs::write(temp.path().join("nginx-fake"),format!("#!/bin/sh\nif [ \"$*\" = '-s reload' ]; then mkdir -p '{}'; fi\nexit 0\n",bad.display())).unwrap();
    let replacement=certificate_input();
    let result=service::replace(&db.state,&cert,&replacement,&actor()).await.unwrap();
    assert!(result["applyError"].is_null(),"report-only scan I/O must not roll back a confirmed deploy: {result}");
    assert_eq!(std::fs::read(&file).unwrap(),before);
    assert_eq!(std::fs::read_to_string(db.state.config.certs_dir.join(&cert).join("fullchain.pem")).unwrap(),replacement.cert_pem);
    assert_eq!(db.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get::<_,serde_json::Value>(0),marker);
    let count:i64=db.client.query_one("SELECT count(*) FROM notifications WHERE severity='warning' AND body LIKE '%재검사를 완료하지 못했습니다%'",&[]).await.unwrap().get(0);
    assert_eq!(count,1);
    let audit:serde_json::Value=db.client.query_one("SELECT \"after\" FROM audit_logs WHERE action='nginx:apply' AND \"after\"->>'source'='certificate' AND target=$1",&[&cert]).await.unwrap().get(0);
    assert_eq!(audit["ok"],true);assert!(audit["output"].as_str().unwrap().contains("재검사를 완료하지 못했습니다"));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn certificate_replace_without_live_consumer_does_not_activate_saved_server() {
    if g13_support::isolated("certificate_replace_without_live_consumer_does_not_activate_saved_server").await { return; }
    use nwui_api::certificates::service;
    let temp = TempDir::new().unwrap();
    let db = real_db(&temp).await;
    let cert = service::import(&db.state, &certificate_input(), &actor()).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES('draft','example.test',8081,$1,$2)", &[&cert, &serde_json::json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
    let result = service::replace(&db.state, &cert, &certificate_input(), &actor()).await.unwrap();
    assert!(result["applyError"].is_null(), "{result}");
    assert!(!db.state.config.nginx_conf_dir.join("server-draft.conf").exists());
    assert!(!db.state.config.certs_dir.join(&cert).exists(), "no live consumer means no live material write");
    assert!(!temp.path().join(".fake-nginx-active-generation").exists(), "no reload");
    let notices: i64 = db.client.query_one("SELECT count(*) FROM notifications WHERE body LIKE '%설정 적용%'", &[]).await.unwrap().get(0);
    assert_eq!(notices, 1, "saved-only consumers need an apply notice");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn certificate_replace_follows_live_only_reference_even_without_applied_marker() {
    if g13_support::isolated("certificate_replace_follows_live_only_reference_even_without_applied_marker").await { return; }
    use nwui_api::certificates::service;
    let temp = TempDir::new().unwrap();
    let db = real_db(&temp).await;
    let cert = service::import(&db.state, &certificate_input(), &actor()).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES('live','example.test',8081,$1,$2)", &[&cert, &serde_json::json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&db.state, &actor(), None).await.unwrap().ok);
    let before = std::fs::read(db.state.config.nginx_conf_dir.join("server-live.conf")).unwrap();
    // A draft removes TLS, and an old/failed boot may have left no marker at all.
    db.client.batch_execute("UPDATE nginx_servers SET certificate_id=NULL; DELETE FROM applied_config").await.unwrap();
    let replacement = certificate_input();
    let result = service::replace(&db.state, &cert, &replacement, &actor()).await.unwrap();
    assert!(result["applyError"].is_null(), "{result}");
    assert_eq!(std::fs::read_to_string(db.state.config.certs_dir.join(&cert).join("fullchain.pem")).unwrap(), replacement.cert_pem);
    assert_eq!(std::fs::read(db.state.config.nginx_conf_dir.join("server-live.conf")).unwrap(), before);
    assert_eq!(db.client.query_one("SELECT count(*) FROM applied_config", &[]).await.unwrap().get::<_, i64>(0), 0);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn certificate_replace_validation_failure_restores_all_files_without_containment() {
    if g13_support::isolated("certificate_replace_validation_failure_restores_all_files_without_containment").await { return; }
    use nwui_api::{certificates::service, control::model};
    let temp = TempDir::new().unwrap();
    let db = real_db(&temp).await;
    let original = certificate_input();
    let cert = service::import(&db.state, &original, &actor()).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES('live','example.test',8081,$1,$2)", &[&cert, &serde_json::json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&db.state, &actor(), None).await.unwrap().ok);
    let before = FixtureFiles::capture(&db.state.config).await.unwrap();
    let replacement = certificate_input();
    // Reject only the replacement; rollback validation can still succeed.
    let cert_path = db.state.config.certs_dir.join(&cert).join("fullchain.pem");
    std::fs::write(temp.path().join("nginx-fake"), format!("#!/bin/sh\nif [ \"$1\" = -t ] && ! cmp -s '{}' '{}'; then echo bad-material; exit 1; fi\nif [ \"$1\" = -T ]; then printf '# configuration file %s:\\n' '{}'; fi\nexit 0\n", cert_path.display(), temp.path().join("old.pem").display(),db.state.config.nginx_main_conf.display())).unwrap();
    std::fs::write(temp.path().join("old.pem"), &original.cert_pem).unwrap();
    let result = service::replace(&db.state, &cert, &replacement, &actor()).await.unwrap();
    assert!(!result["applyError"].is_null());
    let after = FixtureFiles::capture(&db.state.config).await.unwrap();
    assert_eq!(before.conf, after.conf);
    assert_eq!(before.stream, after.stream);
    assert_eq!(std::fs::read_to_string(cert_path).unwrap(), original.cert_pem);
    assert_eq!(std::fs::read_to_string(db.state.config.certs_dir.join(&cert).join("privkey.pem")).unwrap(), original.key_pem);
    assert!(!temp.path().join("quarantine").exists());
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn certificate_replace_node_route_preserves_pending_drafts() {
    if g13_support::isolated("certificate_replace_node_route_preserves_pending_drafts").await { return; }
    use nwui_api::certificates::service;
    use tower::ServiceExt;
    let temp = TempDir::new().unwrap();
    let db = real_db(&temp).await;
    let cert = service::import(&db.state, &certificate_input(), &actor()).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES('live','example.test',8081,$1,$2)", &[&cert, &serde_json::json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&db.state, &actor(), None).await.unwrap().ok);
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('draft','draft.example.test',8082,$1)", &[&serde_json::json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
    auth_support::gateway_issuer(&db.client).await;
    db.client.execute("INSERT INTO gateway_tokens(id,name,role,token_hash,created_by_user_id) VALUES('gateway','gateway','editor',$1,'q3-gateway-issuer')", &[&nwui_api::auth::token_digest("ops2-test-token")]).await.unwrap();
    let input = certificate_input();
    let request = axum::http::Request::builder().method("POST")
        .uri(format!("/api/v1/certificates/{cert}/replace"))
        .header("host", "admin.example.test")
        .header("x-gateway-version", &db.state.config.version)
        .header("authorization", "Bearer ops2-test-token")
        .header("content-type", "application/json")
        .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from(([127,0,0,1],40000))))
        .body(axum::body::Body::from(serde_json::json!({"certPem":input.cert_pem,"keyPem":input.key_pem}).to_string())).unwrap();
    let app = nwui_api::app::with_request_context(nwui_api::certificates::routes::router().with_state(db.state.clone()), None);
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(!db.state.config.nginx_conf_dir.join("server-draft.conf").exists());
    assert_eq!(nwui_api::control::pending::status(&db.state).await.unwrap().count, 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn certificate_job_deployment_preserves_drafts_and_retains_failed_retry() {
    if g13_support::isolated("certificate_job_deployment_preserves_drafts_and_retains_failed_retry").await { return; }
    use nwui_api::certificates::{jobs, service};
    for fail in [false, true] {
        let temp = TempDir::new().unwrap();
        let db = real_db(&temp).await;
        let original = certificate_input();
        let cert = service::import(&db.state, &original, &actor()).await.unwrap();
        db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES('live','example.test',8081,$1,$2)", &[&cert, &serde_json::json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
        assert!(nwui_api::publisher_client::apply_saved(&db.state, &actor(), None).await.unwrap().ok);
        db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('draft','draft.example.test',8082,$1)", &[&serde_json::json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
        let replacement = certificate_input();
        let sealed = db.state.secrets.seal(&replacement.key_pem).unwrap();
        db.client.execute("UPDATE certificates SET cert_pem=$2,key_pem=$3 WHERE id=$1", &[&cert,&replacement.cert_pem,&sealed]).await.unwrap();
        if fail {
            std::fs::write(temp.path().join("old.pem"), &original.cert_pem).unwrap();
            std::fs::write(temp.path().join("nginx-fake"), format!("#!/bin/sh\nif [ \"$1\" = -t ] && ! cmp -s '{}' '{}'; then echo bad-material; exit 1; fi\nif [ \"$1\" = -T ]; then printf '# configuration file %s:\\n' '{}'; fi\nexit 0\n", db.state.config.certs_dir.join(&cert).join("fullchain.pem").display(), temp.path().join("old.pem").display(),db.state.config.nginx_main_conf.display())).unwrap();
        }
        db.client.execute("INSERT INTO certificate_jobs(id,certificate_id,kind,mode,state,payload_enc,actor,deploy_pending,updated_at) VALUES('deploy',$1,'http','renew','cleanup','','{}',true,now()-interval '1 minute')", &[&cert]).await.unwrap();
        let worker = jobs::start(db.state.clone()).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                let row = db.client.query_one("SELECT deploy_pending,error FROM certificate_jobs WHERE id='deploy'", &[]).await.unwrap();
                if if fail { row.get::<_, Option<String>>(1).is_some() } else { !row.get::<_, bool>(0) } { break; }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }).await.unwrap();
        worker.shutdown().await.unwrap();
        assert_eq!(db.client.query_one("SELECT deploy_pending FROM certificate_jobs WHERE id='deploy'", &[]).await.unwrap().get::<_, bool>(0), fail);
        assert!(!db.state.config.nginx_conf_dir.join("server-draft.conf").exists());
        assert_eq!(nwui_api::control::pending::status(&db.state).await.unwrap().count, 1);
        assert_eq!(std::fs::read_to_string(db.state.config.certs_dir.join(&cert).join("fullchain.pem")).unwrap(), if fail { original.cert_pem } else { replacement.cert_pem });
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn certificate_deploy_refuses_stale_invalid_and_unopenable_material_before_writing() {
    if g13_support::isolated("certificate_deploy_refuses_stale_invalid_and_unopenable_material_before_writing").await { return; }
    use nwui_api::certificates::service;
    let temp = TempDir::new().unwrap();
    let db = real_db(&temp).await;
    let original = certificate_input();
    let cert = service::import(&db.state, &original, &actor()).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES('live','example.test',8081,$1,$2)", &[&cert, &serde_json::json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&db.state, &actor(), None).await.unwrap().ok);
    let marker = std::fs::read(db.state.config.nginx_conf_dir.join(".nwui-generation")).unwrap();
    let original_key: String = db.client.query_one("SELECT key_pem FROM certificates WHERE id=$1", &[&cert]).await.unwrap().get(0);
    let stale = nwui_api::control::model::certificate_content_digest(&original.cert_pem, &original_key);
    let other = certificate_input();
    for (pem, key) in [
        (other.cert_pem.clone(), db.state.secrets.seal(&other.key_pem).unwrap()),
        ("malformed".into(), original_key.clone()),
        (original.cert_pem.clone(), db.state.secrets.seal(&other.key_pem).unwrap()),
        (original.cert_pem.clone(), "enc:v1:broken".into()),
        ("x".repeat(1024 * 1024 + 1), original_key.clone()),
    ] {
        db.client.execute("UPDATE certificates SET cert_pem=$2,key_pem=$3 WHERE id=$1", &[&cert,&pem,&key]).await.unwrap();
        let digest = if pem == other.cert_pem { stale.clone() } else { nwui_api::control::model::certificate_content_digest(&pem, &key) };
        let error = nwui_api::publisher_client::deploy_certificate(&db.state, &cert, &digest, &actor()).await.unwrap_err();
        if pem == other.cert_pem { assert!(error.to_string().contains("stale_material")); }
        assert_eq!(std::fs::read_to_string(db.state.config.certs_dir.join(&cert).join("fullchain.pem")).unwrap(), original.cert_pem);
        assert_eq!(std::fs::read(db.state.config.nginx_conf_dir.join(".nwui-generation")).unwrap(), marker);
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn certificate_deploy_validates_every_live_http_and_stream_reference_before_any_write() {
    if g13_support::isolated("certificate_deploy_validates_every_live_http_and_stream_reference_before_any_write").await { return; }
    use nwui_api::{certificates::service, control::model};
    let temp = TempDir::new().unwrap();
    let db = real_db(&temp).await;
    let original = certificate_input();
    let cert = service::import(&db.state, &original, &actor()).await.unwrap();
    let other_input = certificate_input();
    let other = service::import(&db.state, &other_input, &actor()).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES('live','example.test',8081,$1,$2)", &[&cert, &serde_json::json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&db.state, &actor(), None).await.unwrap().ok);
    std::fs::create_dir_all(&db.state.config.nginx_stream_dir).unwrap();
    let stream = format!("server {{ listen 19000 ssl; ssl_certificate {}; ssl_certificate_key {}; proxy_pass 10.0.0.5:9000; }}\n",db.state.config.certs_dir.join(&other).join("fullchain.pem").display(),db.state.config.certs_dir.join(&other).join("privkey.pem").display());
    std::fs::write(db.state.config.nginx_stream_dir.join("stream-live-only.conf"), &stream).unwrap();
    db.client.execute("UPDATE certificates SET key_pem='enc:v1:broken' WHERE id=$1", &[&other]).await.unwrap();
    let before = FixtureFiles::capture(&db.state.config).await.unwrap();
    let a8=db.state.config.nginx_conf_dir.join(".nwui-apply-uncertain");
    let a8_before=std::fs::read(&a8).ok();
    let paths=[db.state.config.certs_dir.join(&cert).join("fullchain.pem"),db.state.config.certs_dir.join(&cert).join("privkey.pem"),db.state.config.certs_dir.join(&other).join("fullchain.pem"),db.state.config.certs_dir.join(&other).join("privkey.pem")];
    let material_before:Vec<_>=paths.iter().map(|path|std::fs::read(path).ok()).collect();
    let calls=temp.path().join("unexpected-stream-tls-deploy-call");
    std::fs::write(temp.path().join("nginx-fake"),format!("#!/bin/sh\nprintf x >> '{}'\nexit 0\n",calls.display())).unwrap();
    let result = service::replace(&db.state, &cert, &certificate_input(), &actor()).await.unwrap();
    assert!(result["applyError"].as_str().is_some_and(|error|error.contains("nginx vocabulary")),"{result}");
    assert_eq!(std::fs::read(&a8).ok(),a8_before);
    assert_eq!(paths.iter().map(|path|std::fs::read(path).ok()).collect::<Vec<_>>(),material_before);
    assert!(!calls.exists());
    assert_eq!(std::fs::read_to_string(db.state.config.certs_dir.join(&cert).join("fullchain.pem")).unwrap(), original.cert_pem);
    let after = FixtureFiles::capture(&db.state.config).await.unwrap();
    assert_eq!(before.conf, after.conf);
    assert_eq!(before.stream, after.stream);
    // Even a valid plaintext stream key cannot admit a non-renderer stream-TLS file.
    db.client.execute("UPDATE certificates SET key_pem=$2 WHERE id=$1", &[&other,&other_input.key_pem]).await.unwrap();
    std::fs::write(&a8,b"prior uncertainty must remain unchanged").unwrap();
    let marker_before=std::fs::read(db.state.config.nginx_conf_dir.join(".nwui-generation")).unwrap();
    let replacement = certificate_input();
    let result = service::replace(&db.state, &cert, &replacement, &actor()).await.unwrap();
    assert!(result["applyError"].as_str().is_some_and(|error|error.contains("nginx vocabulary")),"{result}");
    assert_eq!(std::fs::read(&a8).unwrap(),b"prior uncertainty must remain unchanged");
    assert_eq!(std::fs::read(db.state.config.nginx_conf_dir.join(".nwui-generation")).unwrap(),marker_before);
    assert_eq!(paths.iter().map(|path|std::fs::read(path).ok()).collect::<Vec<_>>(),material_before);
    let after=FixtureFiles::capture(&db.state.config).await.unwrap();assert_eq!(before.conf,after.conf);assert_eq!(before.stream,after.stream);
    assert_eq!(std::fs::read(db.state.config.nginx_stream_dir.join("stream-live-only.conf")).unwrap(), stream.as_bytes());
    assert!(!calls.exists());
    let audits=db.client.query("SELECT actor_label,\"after\" FROM audit_logs WHERE action='nginx:apply' AND target=$1 AND \"after\"->>'source'='certificate' ORDER BY created_at,id",&[&cert]).await.unwrap();
    assert_eq!(audits.len(),2);
    for row in audits {let entry:serde_json::Value=row.get(1);assert_eq!(row.get::<_,String>(0),"test-control");assert_eq!(entry["ok"],false);assert!(entry["output"].as_str().unwrap().contains("nginx vocabulary"));}
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn database_apply_and_rollback_preserve_snapshots_markers_audits_and_failed_state() {
    if g13_support::isolated("database_apply_and_rollback_preserve_snapshots_markers_audits_and_failed_state").await { return; }
    let temp = TempDir::new().unwrap();
    let db = real_db(&temp).await;
    let id = "1d215cf4-7a1e-4ddf-9677-416bc3e6aef1";
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES($1,'example.test',8081,$2)",&[&id,&serde_json::json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
    assert!(
        nwui_api::publisher_client::apply_saved(&db.state, &actor(), Some("original"))
            .await
            .unwrap()
            .ok
    );
    let snapshot: String = db
        .client
        .query_one(
            "SELECT id FROM config_snapshots WHERE label='original'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    db.client
        .execute("UPDATE nginx_servers SET listen=8082 WHERE id=$1", &[&id])
        .await
        .unwrap();
    assert!(
        nwui_api::publisher_client::apply_saved(&db.state, &actor(), Some("second"))
            .await
            .unwrap()
            .ok
    );
    assert!(
        nwui_api::publisher_client::rollback_snapshot(&db.state, &snapshot, &actor())
            .await
            .unwrap()
            .result
            .ok
    );
    let port: i32 = db
        .client
        .query_one("SELECT listen FROM nginx_servers WHERE id=$1", &[&id])
        .await
        .unwrap()
        .get(0);
    assert_eq!(port, 8081);
    let marker: serde_json::Value = db
        .client
        .query_one("SELECT data FROM applied_config WHERE id='current'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(marker["servers"][0]["listen"], 8081);
    let snapshots: i64 = db
        .client
        .query_one("SELECT count(*) FROM config_snapshots", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(snapshots, 3);
    let audit: i64 = db
        .client
        .query_one(
            "SELECT count(*) FROM audit_logs WHERE action IN ('nginx:apply','config:rollback')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(audit, 3);
    db.client
        .execute("UPDATE nginx_servers SET listen=8083 WHERE id=$1", &[&id])
        .await
        .unwrap();
    let file = db
        .state
        .config
        .nginx_conf_dir
        .join(format!("server-{id}.conf"));
    let previous = std::fs::read(&file).unwrap();
    let fault=temp.path().join("fail-rollback-once");std::fs::write(&fault,"armed").unwrap();
    std::fs::write(temp.path().join("nginx-fake"),format!("#!/bin/sh\nif [ -f '{}' ]; then rm '{}'; echo invalid >&2; exit 1; fi\nif [ \"$1\" = -T ]; then printf '# configuration file %s:\\n' '{}'; fi\nexit 0\n",fault.display(),fault.display(),db.state.config.nginx_main_conf.display())).unwrap();
    let failed = nwui_api::publisher_client::rollback_snapshot(&db.state, &snapshot, &actor())
        .await
        .unwrap();
    assert!(!failed.result.ok);
    let port: i32 = db
        .client
        .query_one("SELECT listen FROM nginx_servers WHERE id=$1", &[&id])
        .await
        .unwrap()
        .get(0);
    assert_eq!(port, 8083);
    assert_eq!(std::fs::read(file).unwrap(), previous);
    let snapshots: i64 = db
        .client
        .query_one("SELECT count(*) FROM config_snapshots", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(snapshots, 3);
    let failed_audits:i64=db.client.query_one("SELECT count(*) FROM audit_logs WHERE action='config:rollback' AND (\"after\"->>'ok')::boolean=false",&[]).await.unwrap().get(0);
    assert_eq!(failed_audits, 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn metadata_failure_compensates_files_and_does_not_record_success() {
    if g13_support::isolated("metadata_failure_compensates_files_and_does_not_record_success").await { return; }
    let temp = TempDir::new().unwrap();
    let db = real_db(&temp).await;
    let id = "b7f9f25d-1474-4163-b7cd-cddfba81429b";
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES($1,'example.test',8081,$2)",&[&id,&serde_json::json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
    assert!(
        nwui_api::publisher_client::apply_saved(&db.state, &actor(), None)
            .await
            .unwrap()
            .ok
    );
    let file = db
        .state
        .config
        .nginx_conf_dir
        .join(format!("server-{id}.conf"));
    let before = std::fs::read(&file).unwrap();
    db.client.batch_execute("CREATE FUNCTION fail_applied_marker() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'marker failure'; END $$; CREATE TRIGGER fail_marker BEFORE UPDATE ON applied_config FOR EACH ROW EXECUTE FUNCTION fail_applied_marker(); UPDATE nginx_servers SET listen=8082").await.unwrap();
    assert!(
        nwui_api::publisher_client::apply_saved(&db.state, &actor(), None)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(file).unwrap(), before);
    let snapshots: i64 = db
        .client
        .query_one("SELECT count(*) FROM config_snapshots", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(snapshots, 1);
    let failures:i64=db.client.query_one("SELECT count(*) FROM audit_logs WHERE action='nginx:apply' AND (\"after\"->>'ok')::boolean=false",&[]).await.unwrap().get(0);
    assert_eq!(failures, 1);
    assert!(!db.state.config.nginx_conf_dir.join(".nwui-apply-uncertain").exists(),"known rollback plus R2 clears uncertainty");
}





#[tokio::test]
async fn reserved_sets_include_dynamic_api_port_and_integral_json_numbers() {
    if g13_support::isolated("reserved_sets_include_dynamic_api_port_and_integral_json_numbers").await { return; }
    use nwui_api::control::model::{ConfigSnapshotData, ports_for_snapshot};
    let temp = TempDir::new().unwrap();
    let s = state(temp.path(), "exit 0");
    let data = ConfigSnapshotData {
        servers: vec![
            serde_json::json!({"serverName":"example.test","listen":8081.0,"config":{"tlsPort":9443.0},"enabled":false}),
        ],
        ..Default::default()
    };
    let reserved = ports_for_snapshot(&s.config, &data);
    assert!(reserved.process.contains(&s.config.listen.port()));
    assert!(reserved.process.contains(&s.config.web_port));
    assert!(reserved.http.contains(&8081));
    assert!(reserved.http.contains(&9443));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn apply_skips_an_invalid_row_notifies_and_keeps_it_pending() {
    if g13_support::isolated("apply_skips_an_invalid_row_notifies_and_keeps_it_pending").await { return; }
    use nwui_api::control::pending;
    let temp = TempDir::new().unwrap();
    let db = real_db(&temp).await;
    let good = "5b0c3c52-1e0f-4a8e-9a53-0d7f3f1f6a01";
    let bad = "5b0c3c52-1e0f-4a8e-9a53-0d7f3f1f6a02";
    let target = serde_json::json!({"proxyPass":"http://127.0.0.1:9000"});
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES($1,'good.example.test',8081,$2)",&[&good,&target]).await.unwrap();
    // Save-time validation refuses this name; an older release could have left it behind.
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES($1,'bad;name.example.test',8081,$2)",&[&bad,&target]).await.unwrap();
    let result = nwui_api::publisher_client::apply_saved(&db.state, &actor(), Some("with-bad-row"))
        .await
        .unwrap();
    assert!(result.ok, "{}", result.output);
    assert!(
        result.output.contains("bad;name.example.test"),
        "{}",
        result.output
    );
    let conf = &db.state.config.nginx_conf_dir;
    assert!(conf.join(format!("server-{good}.conf")).exists());
    assert!(!conf.join(format!("server-{bad}.conf")).exists());
    let notices: i64 = db
        .client
        .query_one(
            "SELECT count(*) FROM notifications WHERE type='config.apply-skipped' AND body LIKE '%bad;name.example.test%'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(notices, 1);
    let status = pending::status(&db.state).await.unwrap();
    assert_eq!(
        status.count, 1,
        "the skipped row is not live, so it stays pending"
    );

    // The same skipped set again (boot, certificate renewals, another apply): no new notice.
    assert!(
        nwui_api::publisher_client::apply_saved(&db.state, &actor(), None)
            .await
            .unwrap()
            .ok
    );
    assert_eq!(skipped_notices(&db).await, 1);
    // A different set is news.
    let other = "5b0c3c52-1e0f-4a8e-9a53-0d7f3f1f6a03";
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES($1,'other;bad.example.test',8081,$2)",&[&other,&target]).await.unwrap();
    assert!(
        nwui_api::publisher_client::apply_saved(&db.state, &actor(), None)
            .await
            .unwrap()
            .ok
    );
    assert_eq!(skipped_notices(&db).await, 2);
}
async fn skipped_notices(db: &auth_support::TestDb) -> i64 {
    db.client
        .query_one(
            "SELECT count(*) FROM notifications WHERE type='config.apply-skipped'",
            &[],
        )
        .await
        .unwrap()
        .get(0)
}



#[tokio::test]
#[ignore = "real PostgreSQL: set NWUI_TEST_DATABASE_URL"]
async fn runtime_dns_snapshot_and_rollback_round_trip_flags_without_requiring_dns_answers() {
    if g13_support::isolated("runtime_dns_snapshot_and_rollback_round_trip_flags_without_requiring_dns_answers").await { return; }
    use serde_json::{Value, json};
    let temp = TempDir::new().unwrap();
    let db = real_db(&temp).await;
    let upstream = "11111111-1111-4111-8111-111111111111";
    let stream = "22222222-2222-4222-8222-222222222222";
    let server = "33333333-3333-4333-8333-333333333333";
    db.client.execute("INSERT INTO upstreams(id,name,servers,resolve_at_runtime) VALUES($1,'dynamic_backend',$2,true)",&[&upstream,&json!(["px-web:3001"])]).await.unwrap();
    db.client.execute("INSERT INTO nginx_streams(id,listen,protocol,proxy_pass,resolve_at_runtime) VALUES($1,9200,'tcp','px-web:3001',true)",&[&stream]).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES($1,'example.test',9080,$2)",&[&server,&json!({"proxyPass":"http://px-web:3001/app/","resolveAtRuntime":true})]).await.unwrap();
    assert!(
        nwui_api::publisher_client::apply_saved(&db.state, &actor(), Some("runtime-dns"))
            .await
            .unwrap()
            .ok
    );
    let row = db
        .client
        .query_one(
            "SELECT id,data FROM config_snapshots WHERE label='runtime-dns'",
            &[],
        )
        .await
        .unwrap();
    let id: String = row.get(0);
    let data: Value = row.get(1);
    assert_eq!(data["upstreams"][0]["resolveAtRuntime"], true);
    assert_eq!(data["streams"][0]["resolveAtRuntime"], true);
    assert_eq!(data["servers"][0]["config"]["resolveAtRuntime"], true);
    db.client.batch_execute("UPDATE upstreams SET resolve_at_runtime=false;UPDATE nginx_streams SET resolve_at_runtime=false;UPDATE nginx_servers SET config='{}';").await.unwrap();
    let result = nwui_api::publisher_client::rollback_snapshot(&db.state, &id, &actor())
        .await
        .unwrap();
    assert!(result.result.ok, "{}", result.result.output);
    assert!(
        db.client
            .query_one(
                "SELECT resolve_at_runtime FROM upstreams WHERE id=$1",
                &[&upstream]
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
    assert!(
        db.client
            .query_one(
                "SELECT resolve_at_runtime FROM nginx_streams WHERE id=$1",
                &[&stream]
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
    let marker: Value = db
        .client
        .query_one("SELECT data FROM applied_config WHERE id='current'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(marker["upstreams"][0]["resolveAtRuntime"], true);
    assert_eq!(marker["streams"][0]["resolveAtRuntime"], true);
}

#[tokio::test]
#[ignore = "real PostgreSQL: set NWUI_TEST_DATABASE_URL"]
async fn runtime_dns_rollback_rejects_wrong_flag_types_before_replacing_any_rows() {
    if g13_support::isolated("runtime_dns_rollback_rejects_wrong_flag_types_before_replacing_any_rows").await { return; }
    use serde_json::json;
    let temp = TempDir::new().unwrap();
    let db = real_db(&temp).await;
    db.client
        .execute(
            "INSERT INTO upstreams(id,name,servers) VALUES('keep','keep',$1)",
            &[&json!(["10.0.0.5:9000"])],
        )
        .await
        .unwrap();
    let snapshot = json!({"upstreams":[{"id":"replace","name":"replace","servers":["10.0.0.5:9000"],"resolveAtRuntime":"true"}],"servers":[],"streams":[]});
    db.client.execute("INSERT INTO config_snapshots(id,label,data) VALUES('7a000000-0000-4000-8000-000000000001','invalid',$1)",&[&snapshot]).await.unwrap();
    assert!(
        nwui_api::publisher_client::rollback_snapshot(&db.state, "7a000000-0000-4000-8000-000000000001", &actor())
            .await
            .is_err()
    );
    assert_eq!(
        db.client
            .query_one("SELECT count(*) FROM upstreams WHERE id='keep'", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
}

#[tokio::test]
#[ignore = "real PostgreSQL: set NWUI_TEST_DATABASE_URL"]
async fn runtime_dns_legacy_snapshot_omission_restores_static_false() {
    if g13_support::isolated("runtime_dns_legacy_snapshot_omission_restores_static_false").await { return; }
    use serde_json::json;
    let temp = TempDir::new().unwrap();
    let db = real_db(&temp).await;
    let snapshot = json!({"upstreams":[{"id":"legacy","name":"legacy","servers":["10.0.0.5:9000"]}],"servers":[],"streams":[{"id":"legacy-stream","listen":9200,"protocol":"tcp","proxyPass":"10.0.0.5:9000","enabled":true}]});
    db.client
        .execute(
            "INSERT INTO config_snapshots(id,label,data) VALUES('7a000000-0000-4000-8000-000000000002','legacy',$1)",
            &[&snapshot],
        )
        .await
        .unwrap();
    assert!(
        nwui_api::publisher_client::rollback_snapshot(&db.state, "7a000000-0000-4000-8000-000000000002", &actor())
            .await
            .unwrap()
            .result
            .ok
    );
    assert!(
        !db.client
            .query_one(
                "SELECT resolve_at_runtime FROM upstreams WHERE id='legacy'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
    assert!(
        !db.client
            .query_one(
                "SELECT resolve_at_runtime FROM nginx_streams WHERE id='legacy-stream'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
}






#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn certificate_vocabulary_refusal_is_audited_and_returned_without_writing_or_reload() {
    if g13_support::isolated("certificate_vocabulary_refusal_is_audited_and_returned_without_writing_or_reload").await { return; }
    use nwui_api::certificates::service;
    let temp=TempDir::new().unwrap();let db=real_db(&temp).await;
    let input=certificate_input();
    let cert=service::import(&db.state,&input,&actor()).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES('live','live.example.test',8081,$1,$2)",&[&cert,&serde_json::json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&db.state,&actor(),None).await.unwrap().ok);
    let path=db.state.config.nginx_conf_dir.join("server-live.conf");
    let invalid=format!("server {{ listen 8081; ssl_certificate {}; ssl_certificate_key {}; include /fixture/not-renderer-vocabulary; }}\n",db.state.config.certs_dir.join(&cert).join("fullchain.pem").display(),db.state.config.certs_dir.join(&cert).join("privkey.pem").display());
    std::fs::write(&path,&invalid).unwrap();
    let calls=temp.path().join("unexpected-call");
    std::fs::write(temp.path().join("nginx-fake"),format!("#!/bin/sh\nprintf x >> '{}'\nexit 0\n",calls.display())).unwrap();
    let sealed:String=db.client.query_one("SELECT key_pem FROM certificates WHERE id=$1",&[&cert]).await.unwrap().get(0);
    let digest=nwui_api::control::model::certificate_content_digest(&input.cert_pem,&sealed);
    let result=nwui_api::publisher_client::deploy_certificate(&db.state,&cert,&digest,&actor()).await.unwrap();
    assert!(!result.ok);assert!(result.output.contains("nginx vocabulary"),"{}",result.output);
    assert_eq!(std::fs::read_to_string(path).unwrap(),invalid);
    assert_eq!(std::fs::read_to_string(db.state.config.certs_dir.join(&cert).join("fullchain.pem")).unwrap(),input.cert_pem);
    assert!(!calls.exists());
    let rows=db.client.query("SELECT actor_label,\"after\" FROM audit_logs WHERE action='nginx:apply' AND target=$1 AND \"after\"->>'source'='certificate' AND \"after\"->>'ok'='false'",&[&cert]).await.unwrap();
    assert_eq!(rows.len(),1);assert_eq!(rows[0].get::<_,String>(0),"test-control");
    assert_eq!(rows[0].get::<_,serde_json::Value>(1)["output"],result.output);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn rollback_vocabulary_refusal_rolls_back_rows_and_audits_without_writing_or_reload() {
    if g13_support::isolated("rollback_vocabulary_refusal_rolls_back_rows_and_audits_without_writing_or_reload").await { return; }
    use nwui_api::certificates::service;
    let temp=TempDir::new().unwrap();let mut db=real_db(&temp).await;
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('live','live.example.test',8081,$1)",&[&serde_json::json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&db.state,&actor(),None).await.unwrap().ok);
    let before=std::fs::read(db.state.config.nginx_conf_dir.join("server-live.conf")).unwrap();
    let cert=service::import(&db.state,&certificate_input(),&actor()).await.unwrap();
    // An unquoted root override makes the generated certificate statement unrepresentable.
    std::sync::Arc::make_mut(&mut db.state.config).certs_dir=temp.path().join("certs with spaces");
    let id=uuid::Uuid::new_v4().to_string();
    let data=serde_json::json!({"servers":[{"id":"replacement","serverName":"replacement.example.test","listen":18081,"enabled":true,"certificateId":cert,"config":{"proxyPass":"http://127.0.0.1:9000","tlsPort":18443}}],"upstreams":[],"streams":[]});
    db.client.execute("INSERT INTO config_snapshots(id,label,data) VALUES($1,'vocabulary refusal',$2)",&[&id,&data]).await.unwrap();
    let calls=temp.path().join("unexpected-call");
    std::fs::write(temp.path().join("nginx-fake"),format!("#!/bin/sh\nprintf x >> '{}'\nexit 0\n",calls.display())).unwrap();
    let result=nwui_api::publisher_client::rollback_snapshot(&db.state,&id,&actor()).await.unwrap();
    assert!(!result.result.ok);assert!(result.kind.is_none());
    assert!(result.result.output.contains("nginx vocabulary"),"{}",result.result.output);
    assert_eq!(db.client.query_one("SELECT id FROM nginx_servers",&[]).await.unwrap().get::<_,String>(0),"live");
    assert_eq!(std::fs::read(db.state.config.nginx_conf_dir.join("server-live.conf")).unwrap(),before);
    assert!(!db.state.config.certs_dir.exists());assert!(!calls.exists());
    let rows=db.client.query("SELECT actor_label,\"after\" FROM audit_logs WHERE action='config:rollback' AND target=$1",&[&id]).await.unwrap();
    assert_eq!(rows.len(),1);assert_eq!(rows[0].get::<_,String>(0),"test-control");
    let audit:serde_json::Value=rows[0].get(1);
    assert_eq!(audit["ok"],false);assert_eq!(audit["output"],result.result.output);
}

struct FixtureFiles { conf: BTreeMap<String,Vec<u8>>, stream: BTreeMap<String,Vec<u8>> }
impl FixtureFiles {
    async fn capture(config:&RuntimeConfig)->std::io::Result<Self> {
        fn read(directory:&Path,prefix:&str)->std::io::Result<BTreeMap<String,Vec<u8>>> {
            let mut files=BTreeMap::new();
            if !directory.exists(){return Ok(files);}
            for entry in std::fs::read_dir(directory)? {let entry=entry?;let name=entry.file_name().to_string_lossy().into_owned();let managed=(prefix=="server-" && name=="upstreams.conf") || (name.starts_with(prefix)&&name.ends_with(".conf")&&name.len()>prefix.len()+5);if managed {files.insert(name,std::fs::read(entry.path())?);}}
            Ok(files)
        }
        Ok(Self{conf:read(&config.nginx_conf_dir,"server-")?,stream:read(&config.nginx_stream_dir,"stream-")?})
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn history_prune_and_publisher_rollback_can_complete_in_either_order() {
    let name = "history_prune_and_publisher_rollback_can_complete_in_either_order";
    if g13_support::isolated(name).await { return; }
    let temp = TempDir::new().unwrap();
    let db = real_db(&temp).await;
    db.client.batch_execute(
        "INSERT INTO nginx_servers (id,server_name,listen,config) VALUES
            ('1d215cf4-7a1e-4ddf-9677-416bc3e6aef1','race.example.test',8081,'{\"proxyPass\":\"http://127.0.0.1:9000\"}');
         INSERT INTO app_settings(id,history_retention_days) VALUES('current',30);
         INSERT INTO config_snapshots(id,created_at,data)
            SELECT 'floor-'||n, now()-interval '90 days', '{}'::jsonb FROM generate_series(1,100) n"
    ).await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&db.state, &actor(), Some("admissible rollback")).await.unwrap().ok);
    let snapshot: String = db.client.query_one(
        "UPDATE config_snapshots SET created_at=now()-interval '100 days' WHERE label='admissible rollback' RETURNING id", &[],
    ).await.unwrap().get(0);
    db.client.batch_execute("UPDATE nginx_servers SET listen=8082").await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&db.state, &actor(), Some("current")).await.unwrap().ok);

    let mut client = db.state.db.get().await.unwrap();
    let row_lock = client.transaction().await.unwrap();
    row_lock.query_one("SELECT id FROM nginx_servers FOR UPDATE", &[]).await.unwrap();
    let writer = db.state.clone();
    let target = snapshot.clone();
    let rolling_back = tokio::spawn(async move {
        nwui_api::publisher_client::rollback_snapshot(&writer, &target, &actor()).await.unwrap()
    });
    // The publisher has read the snapshot once it attempts to replace the locked resource row.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let admitted: bool = db.client.query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock'
                    AND query LIKE 'DELETE FROM nginx_servers; DELETE FROM nginx_streams;%')", &[],
            ).await.unwrap().get(0);
            if admitted { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("rollback reached the resource write");
    db.client.batch_execute("UPDATE retention_state SET next_run_at=now() WHERE collection='history'").await.unwrap();
    nwui_api::retention::prune_history_for_tests(&db.state).await;
    assert!(db.client.query_opt("SELECT id FROM config_snapshots WHERE id=$1", &[&snapshot]).await.unwrap().is_none());
    row_lock.rollback().await.unwrap();
    assert!(rolling_back.await.unwrap().result.ok);
    assert_eq!(db.client.query_one("SELECT listen FROM nginx_servers", &[]).await.unwrap().get::<_, i32>(0), 8081);

    let before = FixtureFiles::capture(&db.state.config).await.unwrap();
    let marker: serde_json::Value = db.client.query_one("SELECT data FROM applied_config WHERE id='current'", &[]).await.unwrap().get(0);
    let refused = nwui_api::publisher_client::rollback_snapshot(&db.state, &snapshot, &actor()).await.unwrap();
    assert!(matches!(refused.kind, Some(model::RollbackKind::NotFound)));
    assert!(!refused.result.ok);
    let after = FixtureFiles::capture(&db.state.config).await.unwrap();
    assert_eq!(before.conf, after.conf);
    assert_eq!(before.stream, after.stream);
    assert_eq!(db.client.query_one("SELECT data FROM applied_config WHERE id='current'", &[]).await.unwrap().get::<_, serde_json::Value>(0), marker);
    assert_eq!(db.client.query_one("SELECT listen FROM nginx_servers", &[]).await.unwrap().get::<_, i32>(0), 8081);

    for n in 0..3 {
        db.client.batch_execute("UPDATE retention_state SET next_run_at=now() WHERE collection='history'").await.unwrap();
        let label = format!("concurrent apply {n}");
        let writer_actor = actor();
        let (applied, _) = tokio::join!(
            nwui_api::publisher_client::apply_saved(&db.state, &writer_actor, Some(&label)),
            nwui_api::retention::prune_history_for_tests(&db.state),
        );
        assert!(applied.unwrap().ok);
        assert!(db.client.query_one("SELECT count(*) FROM config_snapshots", &[]).await.unwrap().get::<_, i64>(0) >= 100);

        let recent: String = db.client.query_one(
            "SELECT id FROM config_snapshots WHERE label=$1", &[&label],
        ).await.unwrap().get(0);
        db.client.batch_execute("UPDATE retention_state SET next_run_at=now() WHERE collection='history'").await.unwrap();
        let (rolled_back, _) = tokio::join!(
            nwui_api::publisher_client::rollback_snapshot(&db.state, &recent, &writer_actor),
            nwui_api::retention::prune_history_for_tests(&db.state),
        );
        assert!(rolled_back.unwrap().result.ok);
        assert!(db.client.query_one("SELECT count(*) FROM config_snapshots", &[]).await.unwrap().get::<_, i64>(0) >= 100);
    }
}
