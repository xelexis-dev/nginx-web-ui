#![cfg(unix)]
//! A.33 replacements for the retired API-owned boot writer tests; real PostgreSQL, fake nginx.
mod auth_support;
use nwui_api::{publisher::{boot, test_support}, control::pending};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::{Path, PathBuf}, os::unix::fs::PermissionsExt, time::Duration};
const GOOD: &str = "11111111-1111-4111-8111-111111111111";
const BAD: &str = "22222222-2222-4222-8222-222222222222";

struct Fixture { db: auth_support::TestDb, root: PathBuf, store: boot::PublisherStore }
impl Fixture {
    async fn new(name: &str) -> Option<Self> {
        // PATH is process input to real admission. Isolate it instead of changing the test runner's env.
        let root = match std::env::var_os("NWUI_BOOT_REGRESSION_CHILD") {
            Some(root) => PathBuf::from(root),
            None => {
                let root = tempfile::tempdir().unwrap();
                let output = tokio::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", name, "--ignored", "--nocapture", "--test-threads=1"])
                    .env("NWUI_BOOT_REGRESSION_CHILD", root.path())
                    .env("PATH", format!("{}:/usr/sbin:/usr/bin:/sbin:/bin", root.path().display()))
                    .kill_on_drop(true).output().await.unwrap();
                assert!(output.status.success(), "{name}: {}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
                let stdout = String::from_utf8_lossy(&output.stdout);
                assert!(stdout.lines().any(|line| line.starts_with("test result: ok. 1 passed; 0 failed;")),
                    "{name}: child must run exactly one passing test, got:\n{stdout}");
                return None;
            }
        };
        let conf = root.join("conf"); let stream = root.join("stream");
        std::fs::create_dir_all(&conf).unwrap(); std::fs::create_dir_all(&stream).unwrap();
        let main = root.join("nginx.conf"); let bin = root.join("nginx"); let certs = root.join("certs");
        std::fs::write(&main, format!("pid {}; events {{}} http {{ include {}/*.conf; server {{ listen 127.0.0.1:8080; location = /nginx_status {{ stub_status; allow 127.0.0.3; deny all; include {}/.nwui-generation; }} }} }}", root.join("nginx.pid").display(), conf.display(), conf.display())).unwrap();
        let db = auth_support::test_db_with(&[
            ("NGINX_BIN",bin.to_str().unwrap()), ("NGINX_CONF",main.to_str().unwrap()),
            ("NGINX_CONF_D",conf.to_str().unwrap()), ("NGINX_STREAM_D",stream.to_str().unwrap()),
            ("CERTS_DIR",certs.to_str().unwrap()), ("NGINX_GUARD_QUARANTINE",root.join("quarantine").to_str().unwrap()),
            ("API_DB_POOL_SIZE","2"),
        ]).await;
        let app = db.state.config.data_key_file.parent().unwrap();
        let store = test_support::open_store_fixture(app).unwrap();
        std::fs::write(&bin, format!(r#"#!/bin/sh
if [ "$1" = -V ]; then printf '%s\n' 'nginx version: nginx/1.30.6' 'configure arguments: --prefix={root} --conf-path={root}/nginx.conf' >&2; exit 0; fi
who=publisher
if [ -f '{root}/guard-block' ]; then who=guard; fi
printf '%s start %s\n' "$who" "$*" >> '{root}/calls'
if [ "$who" = guard ]; then
  : > '{root}/guard-started'
  while [ ! -f '{root}/guard-release' ]; do /bin/sleep 0.01; done
fi
for arg do
  if [ "$arg" = -T ]; then printf '# configuration file %s:\n' '{root}/nginx.conf'; fi
done
mode=$(/bin/cat '{root}/failure' 2>/dev/null || true)
if [ "$mode" = all ]; then echo 'nginx: [emerg] global failure' >&2; exit 1; fi
if [ "$mode" = main ]; then echo 'nginx: [emerg] invalid main config in {root}/nginx.conf:1' >&2; exit 1; fi
if [ "$mode" = row ]; then
  staged=$(/usr/bin/find '{app}/.nwui-publisher/payloads' -path '*/.test/conf/*.conf' -type f)
  if [ -n "$staged" ]; then files="$staged"; else files=$(/usr/bin/find '{root}/conf' -name '*.conf' -type f); fi
  for file in $files; do
    if /bin/grep -q rejected.example "$file"; then echo "nginx: [emerg] rejected target in $file:1" >&2; exit 1; fi
  done
fi
printf '%s end %s\n' "$who" "$*" >> '{root}/calls'
exit 0
"#,root=root.display(),app=app.display())).unwrap();
        std::fs::set_permissions(&bin,std::fs::Permissions::from_mode(0o700)).unwrap();
        Some(Self {db, root, store})
    }
    async fn seed(&self,id:&str,name:&str,port:i32) {
        self.db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES($1,$2,$3,$4)",&[&id,&name,&port,&json!({"proxyPass":"http://127.0.0.1:19000"})]).await.unwrap();
    }
    async fn prepare(&self)->Result<boot::StagedPayload,nwui_api::upgrade::StepError> {
        boot::prepare_payload(&self.db.state.config,&self.db.state.secrets,boot::PgSource::Pool(&self.db.state.db),&self.store,nwui_api::publisher::boot::BootMode::Conversion).await
    }
    async fn publish(&self,payload:&boot::StagedPayload)->Result<boot::PublishReport,nwui_api::upgrade::StepError> {
        boot::publish_live(&self.db.state.config,&self.store,&payload.id).await
    }
    async fn initial(&self) { let payload=self.prepare().await.unwrap(); self.publish(&payload).await.unwrap(); }
    fn failure(&self,mode:&str) { std::fs::write(self.root.join("failure"),mode).unwrap(); }
    fn file(&self,id:&str)->PathBuf { self.db.state.config.nginx_conf_dir.join(format!("server-{id}.conf")) }
    fn calls(&self)->String { std::fs::read_to_string(self.root.join("calls")).unwrap_or_default() }
    fn a8(&self)->PathBuf { self.db.state.config.nginx_conf_dir.join(".nwui-apply-uncertain") }
    async fn applied(&self)->Value { self.db.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get(0) }
    async fn adopt(&mut self) { auth_support::nginx::witness(&mut self.db.state);test_support::adopt_current(&self.db.state,&self.store).await.unwrap(); }
}
fn files(path:&Path)->BTreeMap<PathBuf,Vec<u8>> {
    std::fs::read_dir(path).unwrap().map(|entry| {let path=entry.unwrap().path();(path.clone(),std::fs::read(path).unwrap())}).collect()
}

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn publisher_restart_waits_for_nginx_without_fabricating_a_reapply_event() {
    let Some(f)=Fixture::new("publisher_restart_waits_for_nginx_without_fabricating_a_reapply_event").await else{return};
    let before=files(&f.db.state.config.nginx_conf_dir);
    assert_eq!(test_support::restart_after_start(&f.db.state).await.unwrap_err().code,"nginx_not_ready");
    assert_eq!(files(&f.db.state.config.nginx_conf_dir),before);
    let events:i64=f.db.client.query_one("SELECT count(*) FROM notifications WHERE type='boot.nginx-not-ready'",&[]).await.unwrap().get(0);
    assert_eq!(events,0,"A.5 readiness waits; the removed API reapply event is not fabricated");
}

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn fresh_boot_stage_refusal_reports_no_success_or_previous_files() {
    let Some(f)=Fixture::new("fresh_boot_stage_refusal_reports_no_success_or_previous_files").await else{return};
    f.seed(GOOD,"good.example",18081).await;f.failure("all");
    let error=f.prepare().await.unwrap_err();assert_eq!(error.code,"nginx_test_failed");
    assert!(!error.to_string().contains("retained previous files"));
    assert!(files(&f.db.state.config.nginx_conf_dir).is_empty());
    assert!(f.db.client.query_opt("SELECT id FROM applied_config",&[]).await.unwrap().is_none());
    assert!(!f.calls().contains("reload"));
}

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn rejected_saved_target_is_staged_as_skip_without_touching_live_or_reloading() {
    let Some(f)=Fixture::new("rejected_saved_target_is_staged_as_skip_without_touching_live_or_reloading").await else{return};
    f.seed(GOOD,"old-good.example",18081).await;f.initial().await;
    let before=files(&f.db.state.config.nginx_conf_dir);let applied=f.applied().await;
    f.db.client.execute("UPDATE nginx_servers SET server_name='rejected.example' WHERE id=$1",&[&GOOD]).await.unwrap();f.failure("row");
    let staged=f.prepare().await.unwrap();assert_eq!(staged.skipped.len(),1);assert_eq!(staged.skipped[0].id,GOOD);assert!(!staged.skipped[0].kept_previous);
    assert_eq!(files(&f.db.state.config.nginx_conf_dir),before);
    let current=f.applied().await;assert_ne!(current,applied);assert!(current["servers"].as_array().unwrap().is_empty());assert_eq!(current["skipped"],serde_json::to_value(&staged.skipped).unwrap());
    assert_eq!(f.db.client.query_one("SELECT server_name FROM nginx_servers WHERE id=$1",&[&GOOD]).await.unwrap().get::<_,String>(0),"rejected.example");
    assert!(std::fs::read_to_string(f.file(GOOD)).unwrap().contains("old-good.example"));assert!(!f.calls().contains("reload"));
}

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn publisher_guard_quarantines_the_rejected_managed_file_for_recovery() {
    let Some(f)=Fixture::new("publisher_guard_quarantines_the_rejected_managed_file_for_recovery").await else{return};
    f.seed(GOOD,"good.example",18081).await;f.seed(BAD,"rejected.example",18082).await;f.initial().await;
    let rejected=std::fs::read(f.file(BAD)).unwrap();f.failure("row");
    let report=boot::guard(&f.db.state.config,&f.store).await.unwrap();
    assert_eq!(report.invalid_files,0);
    assert_eq!(report.rejected_files,vec![format!("server-{BAD}.conf")]);
    assert!(f.file(GOOD).is_file());assert!(!f.file(BAD).exists());
    assert!(report.quarantine_dirs.iter().any(|dir|std::fs::read(dir.join(format!("conf.d/server-{BAD}.conf"))).ok().as_ref()==Some(&rejected)));
}

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn boot_publication_materializes_sealed_certificate_and_custom_runtime_ports() {
    let Some(mut f)=Fixture::new("boot_publication_materializes_sealed_certificate_and_custom_runtime_ports").await else{return};
    std::sync::Arc::get_mut(&mut f.db.state.config).unwrap().listen.set_port(3101);
    let material=rcgen::generate_simple_self_signed(vec!["tls.example".into()]).unwrap();let key=material.signing_key.serialize_pem();
    let sealed=f.db.state.secrets.seal(&key).unwrap();let cert=uuid::Uuid::new_v4().to_string();
    f.db.client.execute("INSERT INTO certificates(id,domains,status,source,cert_pem,key_pem) VALUES($1,'[\"tls.example\"]','issued','manual',$2,$3)",&[&cert,&material.cert.pem(),&sealed]).await.unwrap();
    f.seed(GOOD,"tls.example",18081).await;f.db.client.execute("UPDATE nginx_servers SET certificate_id=$1 WHERE id=$2",&[&cert,&GOOD]).await.unwrap();
    let payload=f.prepare().await.unwrap();assert!(!f.file(GOOD).exists(),"prepare is immutable staging");
    f.publish(&payload).await.unwrap();
    assert!(std::fs::read_to_string(f.db.state.config.nginx_conf_dir.join("00-runtime-upstreams.conf")).unwrap().contains("127.0.0.1:3101"));
    let private=f.db.state.config.certs_dir.join(cert).join("privkey.pem");assert_eq!(std::fs::read_to_string(&private).unwrap(),key);
    assert_eq!(std::fs::metadata(private).unwrap().permissions().mode()&0o777,0o600);assert!(!f.calls().contains("reload"));
    assert!(f.a8().exists());assert!(f.db.client.query_one("SELECT actor_id FROM applied_config WHERE id='current'",&[]).await.unwrap().get::<_,Option<String>>(0).is_none());
}

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn boot_skipped_row_is_not_published_and_adoption_deduplicates_its_notice() {
    let Some(mut f)=Fixture::new("boot_skipped_row_is_not_published_and_adoption_deduplicates_its_notice").await else{return};
    f.seed(GOOD,"good.example",18081).await;f.seed(BAD,"bad;legacy",18082).await;
    let payload=f.prepare().await.unwrap();assert_eq!(payload.skipped.len(),1);assert_eq!(payload.skipped[0].id,BAD);
    f.publish(&payload).await.unwrap();assert!(f.file(GOOD).exists());assert!(!f.file(BAD).exists());f.adopt().await;
    let applied=f.applied().await;assert_eq!(applied["skipped"][0]["id"],BAD);
    assert_eq!(f.db.client.query_one("SELECT count(*) FROM notifications WHERE type='config.apply-skipped'",&[]).await.unwrap().get::<_,i64>(0),1);
    let body:String=f.db.client.query_one("SELECT body FROM notifications WHERE type='config.apply-skipped'",&[]).await.unwrap().get(0);
    assert!(body.contains("bad;legacy"),"{body}");
    test_support::adopt_current(&f.db.state,&f.store).await.unwrap();
    assert_eq!(f.db.client.query_one("SELECT count(*) FROM notifications WHERE type='config.apply-skipped'",&[]).await.unwrap().get::<_,i64>(0),1);
}

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn failed_boot_stage_never_reports_unpublished_containment_actions() {
    let Some(f)=Fixture::new("failed_boot_stage_never_reports_unpublished_containment_actions").await else{return};
    f.seed(GOOD,"old-a.example",18081).await;f.seed(BAD,"old-b.example",18082).await;f.initial().await;
    let before=files(&f.db.state.config.nginx_conf_dir);let previous=f.applied().await;
    f.db.client.execute("UPDATE nginx_servers SET server_name=CASE WHEN id=$1 THEN 'new-a.example' ELSE 'rejected.example' END",&[&GOOD]).await.unwrap();f.failure("all");
    assert!(f.prepare().await.is_err());assert_eq!(files(&f.db.state.config.nginx_conf_dir),before);assert_eq!(f.applied().await,previous);
    assert!(std::fs::read_to_string(f.file(BAD)).unwrap().contains("old-b.example"));
    assert_eq!(f.db.client.query_one("SELECT count(*) FROM notifications WHERE type='config.apply-skipped'",&[]).await.unwrap().get::<_,i64>(0),0);
}

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn pre_listener_publication_rotates_the_marker_and_keeps_a8_without_hup() {
    let Some(f)=Fixture::new("pre_listener_publication_rotates_the_marker_and_keeps_a8_without_hup").await else{return};
    f.seed(GOOD,"good.example",18081).await;let old=uuid::Uuid::new_v4().to_string();
    std::fs::write(f.db.state.config.nginx_conf_dir.join(".nwui-generation"),format!("add_header X-NWUI-Config-Generation \"{old}\";\n")).unwrap();
    f.initial().await;let text=std::fs::read_to_string(f.db.state.config.nginx_conf_dir.join(".nwui-generation")).unwrap();
    assert!(!text.contains(&old));let token=text.split('"').nth(1).unwrap();assert_eq!(uuid::Uuid::parse_str(token).unwrap().to_string(),token);
    assert!(f.a8().exists());assert!(!f.calls().contains("-s reload"));
}

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn uncertain_old_row_is_not_copied_into_a_new_boot_payload() {
    let Some(f)=Fixture::new("uncertain_old_row_is_not_copied_into_a_new_boot_payload").await else{return};
    f.seed(BAD,"old-b.example",18082).await;f.initial().await;assert!(f.a8().exists());
    f.db.client.execute("UPDATE nginx_servers SET server_name='bad;legacy' WHERE id=$1",&[&BAD]).await.unwrap();
    let payload=f.prepare().await.unwrap();assert_eq!(payload.skipped.len(),1);assert!(!payload.skipped[0].kept_previous);assert!(!payload.skipped[0].reason.is_empty());
    f.publish(&payload).await.unwrap();assert!(!f.file(BAD).exists());assert!(f.a8().exists());
}

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn boot_publication_waits_for_the_publisher_guard_to_finish() {
    let Some(f)=Fixture::new("boot_publication_waits_for_the_publisher_guard_to_finish").await else{return};
    f.seed(GOOD,"good.example",18081).await;let payload=f.prepare().await.unwrap();
    std::fs::write(f.root.join("guard-block"),b"block").unwrap();
    let config=f.db.state.config.as_ref().clone();let store=f.store.clone();
    let guard=tokio::spawn(async move {boot::guard(&config,&store).await});
    tokio::time::timeout(Duration::from_secs(10),async {while !f.root.join("guard-started").exists(){tokio::time::sleep(Duration::from_millis(10)).await;}}).await.unwrap();
    let before=files(&f.db.state.config.nginx_conf_dir);
    let publishing=f.publish(&payload);tokio::pin!(publishing);
    let blocked=tokio::time::timeout(Duration::from_millis(50),&mut publishing).await.is_err();
    let during=files(&f.db.state.config.nginx_conf_dir);
    std::fs::write(f.root.join("guard-release"),b"release").unwrap();guard.await.unwrap().unwrap();
    std::fs::remove_file(f.root.join("guard-block")).unwrap();
    publishing.await.unwrap();assert!(blocked);assert_eq!(during,before,"publication must not mutate under the guard");
    let calls=f.calls();let guard_end=calls.rfind("guard end").unwrap();let publisher_start=calls.find("publisher start").unwrap();
    // Ignore the completed prepare call that predates guard admission.
    let guard_start=calls.find("guard start").unwrap();
    let after_guard_start=&calls[guard_start..];
    assert!(after_guard_start.rfind("guard end").unwrap()<after_guard_start.find("publisher start").unwrap(),"{calls}");
    assert!(guard_end>publisher_start,"prepare precedes the guard");
}

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn boot_tls_fail_closed_matrix_keeps_other_servers() {
    let Some(f)=Fixture::new("boot_tls_fail_closed_matrix_keeps_other_servers").await else{return};
    let cert=uuid::Uuid::new_v4().to_string();f.db.client.execute("INSERT INTO certificates(id,domains,status) VALUES($1,'[\"tls.example\"]','failed')",&[&cert]).await.unwrap();
    f.seed(GOOD,"good.example",18081).await;f.seed(BAD,"tls.example",18082).await;
    f.db.client.execute("UPDATE nginx_servers SET certificate_id=$1,config=$2 WHERE id=$3",&[&cert,&json!({"proxyPass":"http://127.0.0.1:19000","tlsPort":18443,"redirectHttp":true}),&BAD]).await.unwrap();
    for(status,cert_pem,key_pem)in[("failed",Some("pem"),Some("key")),("issuing",Some("pem"),Some("key")),("issued",None,Some("key")),("issued",Some("pem"),None),("issued",Some(""),Some("key")),("issued",Some("pem"),Some(""))] {
        f.db.client.execute("UPDATE certificates SET status=$2::text::cert_status,cert_pem=$3,key_pem=$4 WHERE id=$1",&[&cert,&status,&cert_pem,&key_pem]).await.unwrap();
        let payload=f.prepare().await.unwrap();assert!(payload.skipped.iter().any(|row|row.id==BAD&&row.label=="tls.example"&&row.reason.contains(&cert)),"{status}: {:?}",payload.skipped);
        f.publish(&payload).await.unwrap();assert!(f.file(GOOD).exists());assert!(!f.file(BAD).exists(),"{status}");
    }
}

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn boot_adoption_uses_only_the_exclusive_status_source() {
    use tokio::io::{AsyncReadExt,AsyncWriteExt};
    let Some(mut f)=Fixture::new("boot_adoption_uses_only_the_exclusive_status_source").await else{return};
    f.seed(GOOD,"good.example",18081).await;f.initial().await;auth_support::nginx::witness(&mut f.db.state);
    let upstream=url::Url::parse(&f.db.state.config.nginx_status_url).unwrap().socket_addrs(||None).unwrap()[0];
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let port=listener.local_addr().unwrap().port();
    let peers=std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));let seen=peers.clone();
    let proxy=tokio::spawn(async move {while let Ok((mut stream,peer))=listener.accept().await {
        seen.lock().unwrap().push(peer.ip());let mut request=Vec::new();while !request.ends_with(b"\r\n\r\n"){let mut byte=[0];if stream.read(&mut byte).await.unwrap()==0{break;}request.push(byte[0]);}
        if peer.ip().to_string()!="127.0.0.3" {stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();continue;}
        let mut target=tokio::net::TcpStream::connect(upstream).await.unwrap();target.write_all(&request).await.unwrap();let mut reply=Vec::new();target.read_to_end(&mut reply).await.unwrap();stream.write_all(&reply).await.unwrap();
    }});
    let config=std::sync::Arc::get_mut(&mut f.db.state.config).unwrap();let old=url::Url::parse(&config.nginx_status_url).unwrap().port().unwrap();
    let main=std::fs::read_to_string(&config.nginx_main_conf).unwrap().replace(&format!("listen 127.0.0.1:{old};"),&format!("listen 127.0.0.1:{port};"));std::fs::write(&config.nginx_main_conf,main).unwrap();config.nginx_status_url=format!("http://127.0.0.1:{port}/nginx_status");
    let result=tokio::time::timeout(Duration::from_secs(5),test_support::adopt_current(&f.db.state,&f.store)).await;proxy.abort();result.unwrap().unwrap();
    assert!(!f.a8().exists());assert_eq!(pending::status(&f.db.state).await.unwrap().count,0);
    let peers=peers.lock().unwrap();assert!(!peers.is_empty());assert!(peers.iter().all(|ip|ip.to_string()=="127.0.0.3"),"{peers:?}");
}

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn uncontainable_main_refusal_preserves_last_good_generation_metadata_and_notifications() {
    let Some(f)=Fixture::new("uncontainable_main_refusal_preserves_last_good_generation_metadata_and_notifications").await else{return};
    f.seed(GOOD,"old-good.example",18081).await;f.initial().await;
    let before=files(&f.db.state.config.nginx_conf_dir);let applied=f.applied().await;
    let manifest=f.db.state.config.data_key_file.parent().unwrap().join(".nwui-publisher/manifest.json");let generation=std::fs::read(&manifest).unwrap();
    let notifications:i64=f.db.client.query_one("SELECT count(*) FROM notifications",&[]).await.unwrap().get(0);
    f.failure("main");let error=f.prepare().await.unwrap_err();assert_eq!(error.code,"nginx_test_failed");assert!(error.detail.unwrap().contains("nginx.conf:1"));
    assert_eq!(files(&f.db.state.config.nginx_conf_dir),before);assert_eq!(f.applied().await,applied);assert_eq!(std::fs::read(&manifest).unwrap(),generation);
    assert_eq!(f.db.client.query_one("SELECT count(*) FROM notifications",&[]).await.unwrap().get::<_,i64>(0),notifications);
    assert!(std::fs::read_to_string(f.file(GOOD)).unwrap().contains("old-good.example"));assert!(!f.calls().contains("reload"));
}
