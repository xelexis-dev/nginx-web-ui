#![cfg(unix)]
mod auth_support;
mod g13_support;
use nwui_api::{publisher::{boot, test_support},control::pending};
use serde_json::json;
use std::{path::Path,os::unix::fs::{OpenOptionsExt,PermissionsExt},io::Write};

const GOOD:&str="11111111-1111-4111-8111-111111111111";
const BAD:&str="22222222-2222-4222-8222-222222222222";
fn marker(state:&nwui_api::state::AppState)->bool {state.config.nginx_conf_dir.join(".nwui-apply-uncertain").exists()}
fn superseded_unknown(root:&Path,current:&str) {
    let ops=root.join(".nwui-publisher/ops");std::fs::create_dir_all(&ops).unwrap();std::fs::set_permissions(&ops,std::fs::Permissions::from_mode(0o700)).unwrap();
    let id=uuid::Uuid::new_v4().to_string();
    let value=json!({"version":1,"id":id,"commandDigest":"a".repeat(64),"phase":{"superseded":{"by":uuid::Uuid::new_v4().to_string()}},"previous":current,"next":current,"restoreTarget":current,"restoreReason":"lost commit response","commitUnknown":true,"uncertain":true,"createdAt":chrono::Utc::now(),"result":{"status":"failed","code":"superseded","output":"no replacement published"}});
    let mut file=std::fs::OpenOptions::new().create_new(true).write(true).mode(0o600).open(ops.join(format!("{id}.json"))).unwrap();
    file.write_all(&serde_json::to_vec(&value).unwrap()).unwrap();file.sync_all().unwrap();std::fs::File::open(&ops).unwrap().sync_all().unwrap();
}

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL; isolated child owns process PATH"]
async fn boot_applied_state_skipped_notice_and_this_boot_adoption_match_119() {
    let Some(root)=std::env::var_os("NWUI_PUBLISHER_BOOT_CHILD") else {
        let temp=tempfile::tempdir().unwrap();
        let output=tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","boot_applied_state_skipped_notice_and_this_boot_adoption_match_119","--ignored","--nocapture","--test-threads=1"])
            .env("NWUI_PUBLISHER_BOOT_CHILD",temp.path()).env("PATH",format!("{}:{}",temp.path().display(),std::env::var("PATH").unwrap_or_default()))
            .kill_on_drop(true).output().await.unwrap();
        assert!(output.status.success(),"{}\n{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));assert!(String::from_utf8_lossy(&output.stdout).lines().any(|line|line.starts_with("test result: ok. 1 passed; 0 failed;")));return;
    };
    let root=std::path::PathBuf::from(root);let bin=root.join("nginx");let main=root.join("nginx.conf");let conf=root.join("conf");let stream=root.join("stream");let certs=root.join("certs");
    std::fs::create_dir_all(&conf).unwrap();std::fs::create_dir_all(&stream).unwrap();
    std::fs::write(&main,format!("pid {}; events {{}} http {{ include {}/*.conf; server {{ listen 127.0.0.1:8080; location = /nginx_status {{ stub_status; allow 127.0.0.3; deny all; include {}/.nwui-generation; }} }} }}",root.join("nginx.pid").display(),conf.display(),conf.display())).unwrap();
    std::fs::set_permissions(&main,std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&bin,format!("#!/bin/sh\nif [ \"$1\" = -V ]; then printf '%s\\n' 'nginx version: nginx/1.30.6' 'configure arguments: --prefix={} --conf-path={}' >&2; exit 0; fi\nif [ \"$1\" = -T ] || [ \"$2\" = -T ]; then printf '# configuration file %s:\\n' '{}'; fi\nexit 0\n",root.display(),main.display(),main.display())).unwrap();
    std::fs::set_permissions(&bin,std::fs::Permissions::from_mode(0o700)).unwrap();
    let values=[bin.display().to_string(),main.display().to_string(),conf.display().to_string(),stream.display().to_string(),certs.display().to_string()];
    let mut db=auth_support::test_db_with(&[("NGINX_BIN",&values[0]),("NGINX_CONF",&values[1]),("NGINX_CONF_D",&values[2]),("NGINX_STREAM_D",&values[3]),("CERTS_DIR",&values[4]),("API_DB_POOL_SIZE","2")]).await;
    let boot_config=db.state.config.as_ref().clone();
    let store_root=root.join("publisher-state");std::fs::create_dir(&store_root).unwrap();let store=test_support::open_store_fixture(&store_root).unwrap();
    let snapshots:i64=db.client.query_one("SELECT count(*) FROM config_snapshots",&[]).await.unwrap().get(0);
    let audits:i64=db.client.query_one("SELECT count(*) FROM audit_logs",&[]).await.unwrap().get(0);
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES($1,'boot.example.test',18081,$2)",&[&GOOD,&json!({"proxyPass":"http://127.0.0.1:19000"})]).await.unwrap();
    assert_eq!(pending::status(&db.state).await.unwrap().count,1);
    let first=boot::prepare_payload(&boot_config,&db.state.secrets,boot::PgSource::Pool(&db.state.db),&store,nwui_api::publisher::boot::BootMode::Conversion).await.unwrap();
    assert!(!conf.join(format!("server-{GOOD}.conf")).exists(),"prepare must not publish live files");
    g13_support::publish_stopped(&db.state,&boot_config,&store,&first.id).await.unwrap();assert!(marker(&db.state));
    g13_support::witness(&mut db.state);test_support::adopt_current(&db.state,&store).await.unwrap();
    assert!(!marker(&db.state));assert_eq!(pending::status(&db.state).await.unwrap().count,0);
    assert!(std::fs::read_to_string(conf.join(format!("server-{GOOD}.conf"))).unwrap().contains("127.0.0.1:19000"));
    assert!(db.client.query_one("SELECT actor_id FROM applied_config WHERE id='current'",&[]).await.unwrap().get::<_,Option<String>>(0).is_none());
    // No replacement after a superseded unknown commit must keep A8 and pending visible.
    superseded_unknown(&store_root,&first.id);std::fs::write(conf.join(".nwui-apply-uncertain"),b"unknown commit").unwrap();
    test_support::adopt_current(&db.state,&store).await.unwrap();assert!(marker(&db.state));assert!(pending::status(&db.state).await.unwrap().count>0);
    db.client.execute("UPDATE nginx_servers SET config=$1 WHERE id=$2",&[&json!({"proxyPass":"http://127.0.0.1:19001"}),&GOOD]).await.unwrap();
    let replacement=boot::prepare_payload(&boot_config,&db.state.secrets,boot::PgSource::Pool(&db.state.db),&store,nwui_api::publisher::boot::BootMode::Conversion).await.unwrap();
    g13_support::publish_stopped(&db.state,&boot_config,&store,&replacement.id).await.unwrap();assert!(marker(&db.state));
    test_support::adopt_current(&db.state,&store).await.unwrap();assert!(!marker(&db.state));assert_eq!(pending::status(&db.state).await.unwrap().count,0);
    assert!(std::fs::read_to_string(conf.join(format!("server-{GOOD}.conf"))).unwrap().contains("127.0.0.1:19001"));
    // Legacy-invalid rows are omitted with the same applied marker and deduped notice as apply_current.
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES($1,'bad;legacy',18082,$2)",&[&BAD,&json!({"proxyPass":"http://127.0.0.1:19002"})]).await.unwrap();
    let skipped=boot::prepare_payload(&boot_config,&db.state.secrets,boot::PgSource::Pool(&db.state.db),&store,nwui_api::publisher::boot::BootMode::Conversion).await.unwrap();assert_eq!(skipped.skipped.len(),1);assert_eq!(skipped.skipped[0].id,BAD);
    let prepared_marker:serde_json::Value=db.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get(0);
    assert_eq!(prepared_marker["servers"].as_array().unwrap().len(),1);assert_eq!(prepared_marker["skipped"],serde_json::to_value(&skipped.skipped).unwrap());
    g13_support::publish_stopped(&db.state,&boot_config,&store,&skipped.id).await.unwrap();test_support::adopt_current(&db.state,&store).await.unwrap();
    assert!(!conf.join(format!("server-{BAD}.conf")).exists());assert_eq!(pending::status(&db.state).await.unwrap().count,1);
    let notice:i64=db.client.query_one("SELECT count(*) FROM notifications WHERE type='config.apply-skipped'",&[]).await.unwrap().get(0);assert_eq!(notice,1);
    let repeated=boot::prepare_payload(&boot_config,&db.state.secrets,boot::PgSource::Pool(&db.state.db),&store,nwui_api::publisher::boot::BootMode::Conversion).await.unwrap();
    g13_support::publish_stopped(&db.state,&boot_config,&store,&repeated.id).await.unwrap();
    test_support::adopt_current(&db.state,&store).await.unwrap();
    let legacy_marker:serde_json::Value=db.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get(0);assert_eq!(legacy_marker,prepared_marker);
    assert_eq!(db.client.query_one("SELECT count(*) FROM notifications WHERE type='config.apply-skipped'",&[]).await.unwrap().get::<_,i64>(0),notice);
    assert_eq!(db.client.query_one("SELECT count(*) FROM config_snapshots",&[]).await.unwrap().get::<_,i64>(0),snapshots);
    assert_eq!(db.client.query_one("SELECT count(*) FROM audit_logs",&[]).await.unwrap().get::<_,i64>(0),audits);
}
