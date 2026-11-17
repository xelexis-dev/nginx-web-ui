mod auth_support;
use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use nwui_api::{
    certificates::{jobs, pem, service, store},
    contracts::Actor,
    state::AppState,
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
struct Ca {
    base: String,
    status: String,
    cert: String,
    fail: bool,
    orders: usize,
    finish_gate: Option<tokio::sync::oneshot::Sender<deadpool_postgres::Object>>,
    late_cancel_gate: Option<tokio::sync::oneshot::Sender<tokio_postgres::Client>>,
    state: AppState,
}
struct CsrPublic(Vec<u8>);
impl rcgen::PublicKeyData for CsrPublic {
    fn der_bytes(&self) -> &[u8] {
        &self.0
    }
    fn algorithm(&self) -> &'static rcgen::SignatureAlgorithm {
        &rcgen::PKCS_RSA_SHA256
    }
}
async fn ca(
    State(shared): State<Arc<tokio::sync::Mutex<Ca>>>,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    let mut s = shared.lock().await;
    let mut headers = HeaderMap::new();
    headers.insert("replay-nonce", "dGVzdC1ub25jZQ".parse().unwrap());
    let payload = if method == Method::POST {
        let jws: Value = serde_json::from_slice(&body).unwrap();
        serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(jws["payload"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let mut status = StatusCode::OK;
    let reply = match uri.path() {
        "/directory" => {
            json!({"newNonce":format!("{}/nonce",s.base),"newAccount":format!("{}/account",s.base),"newOrder":format!("{}/new-order",s.base)})
        }
        "/account" => {
            headers.insert("location", format!("{}/account/1", s.base).parse().unwrap());
            json!({"status":"valid","orders":format!("{}/orders",s.base)})
        }
        "/new-order" => {
            s.orders += 1;
            headers.insert("location", format!("{}/order/1", s.base).parse().unwrap());
            order(&s)
        }
        "/order/1" => order(&s),
        "/auth/1" => {
            json!({"identifier":{"type":"dns","value":"example.com"},"status":"pending","challenges":[{"type":"http-01","url":format!("{}/challenge/http",s.base),"status":"pending","token":"worker-token"},{"type":"dns-01","url":format!("{}/challenge/dns",s.base),"status":"pending","token":"worker-token"}]})
        }
        "/challenge/http" | "/challenge/dns" => {
            if uri.path().ends_with("http") {
                let db = s.state.db.get().await.unwrap();
                assert!(
                    db.query_opt(
                        "SELECT token FROM acme_challenges WHERE token='worker-token'",
                        &[]
                    )
                    .await
                    .unwrap()
                    .is_some(),
                    "challenge must be committed before CA validation"
                );
            }
            if s.fail {
                status = StatusCode::BAD_REQUEST;
                json!({"type":"urn:ietf:params:acme:error:unauthorized","detail":"mock denied","status":400})
            } else {
                s.status = "ready".into();
                json!({"type":"http-01","url":format!("{}{}",s.base,uri.path()),"status":"pending","token":"worker-token"})
            }
        }
        "/finalize" => {
            use x509_parser::prelude::FromDer;
            let der = URL_SAFE_NO_PAD
                .decode(payload["csr"].as_str().unwrap())
                .unwrap();
            let (_, csr) =
                x509_parser::certification_request::X509CertificationRequest::from_der(&der)
                    .unwrap();
            let public = CsrPublic(
                csr.certification_request_info
                    .subject_pki
                    .subject_public_key
                    .data
                    .to_vec(),
            );
            let issuer = rcgen::Issuer::new(
                rcgen::CertificateParams::default(),
                rcgen::KeyPair::generate().unwrap(),
            );
            s.cert = rcgen::CertificateParams::new(vec!["example.com".into()])
                .unwrap()
                .signed_by(&public, &issuer)
                .unwrap()
                .pem();
            s.status = "valid".into();
            order(&s)
        }
        "/cert" => {
            if let Some(gate) = s.finish_gate.take() {
                let held = s.state.db.get().await.unwrap();
                gate.send(held).ok().unwrap();
            }
            if let Some(gate)=s.late_cancel_gate.take() {
                let(client,connection)=tokio_postgres::connect(&s.state.config.database_url,tokio_postgres::NoTls).await.unwrap();
                tokio::spawn(async move {connection.await.unwrap();});
                client.batch_execute("BEGIN").await.unwrap();
                client.query_one("SELECT c.id FROM certificates c JOIN certificate_jobs j ON j.certificate_id=c.id WHERE j.state='active' FOR UPDATE OF c",&[]).await.unwrap();
                gate.send(client).ok().unwrap();
            }
            return (status, headers, s.cert.clone()).into_response();
        }
        _ => json!({"status":"valid"}),
    };
    (status, headers, axum::Json(reply)).into_response()
}
fn order(s: &Ca) -> Value {
    json!({"status":s.status,"identifiers":[{"type":"dns","value":"example.com"}],"authorizations":[format!("{}/auth/1",s.base)],"finalize":format!("{}/finalize",s.base),"certificate":if s.status=="valid"{Some(format!("{}/cert",s.base))}else{None}})
}
fn actor() -> Actor {
    Actor {
        via: None,
        id: None,
        label: Some("test:issuance".into()),
        ip: None,
    }
}
async fn setup(fail: bool) -> (auth_support::TestDb, tokio::task::JoinHandle<()>) {
    let (t, server, _) = setup_with(fail, &[]).await;
    (t, server)
}
async fn setup_with(
    fail: bool,
    extra: &[(&str, &str)],
) -> (
    auth_support::TestDb,
    tokio::task::JoinHandle<()>,
    Arc<tokio::sync::Mutex<Ca>>,
) {
    let (mut t,socket,base)=setup_environment(extra).await;
    if extra.iter().any(|(key,_)|*key=="NGINX_BIN") {auth_support::nginx::witness(&mut t.state);}
    let (task,ca)=start_ca(&t,socket,base,fail);
    (t,task,ca)
}
async fn setup_environment(extra:&[(&str,&str)])->(auth_support::TestDb,tokio::net::TcpListener,String) {
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", socket.local_addr().unwrap());
    let dir = format!("{base}/directory");
    let mut settings = vec![
        ("ACME_DIRECTORY_URL", dir.as_str()),
        ("API_ACME_TIMEOUT_SECS", "3"),
    ];
    settings.extend_from_slice(extra);
    let t = auth_support::test_db_with(&settings).await;
    (t,socket,base)
}
fn start_ca(t:&auth_support::TestDb,socket:tokio::net::TcpListener,base:String,fail:bool)->(tokio::task::JoinHandle<()>,Arc<tokio::sync::Mutex<Ca>>) {
    let ca = Arc::new(tokio::sync::Mutex::new(Ca {
        base,
        status: "pending".into(),
        cert: String::new(),
        fail,
        orders: 0,
        finish_gate: None,
        late_cancel_gate: None,
        state: t.state.clone(),
    }));
    let app = Router::new().fallback(ca_handler).with_state(ca.clone());
    let task = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
    (task, ca)
}
async fn ca_handler(
    state: State<Arc<tokio::sync::Mutex<Ca>>>,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    ca(state, method, uri, body).await
}
async fn wait(t: &auth_support::TestDb, id: &str, status: &str) {
    for _ in 0..300 {
        let r = t
            .client
            .query_one("SELECT status::text FROM certificates WHERE id=$1", &[&id])
            .await
            .unwrap();
        if r.get::<_, String>(0) == status {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("certificate did not reach {status}");
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn http_worker_issues_matching_certificate_and_cleans_challenge() {
    let (t, server) = setup(false).await;
    let input =
        service::parse_input(&json!({"domains":["example.com"],"email":"admin@example.com"}))
            .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    wait(&t, &id, "issued").await;
    worker.shutdown().await.unwrap();
    let cert = store::get(&t.client, &id, false).await.unwrap();
    let key = t.state.secrets.open(cert.key_pem.as_deref()).unwrap();
    pem::parse_certificate_pem(cert.cert_pem.as_deref().unwrap(), &key).unwrap();
    assert_eq!(
        t.client
            .query_one("SELECT count(*) FROM acme_challenges", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    let row = t
        .client
        .query_one(
            "SELECT state,payload_enc FROM certificate_jobs WHERE certificate_id=$1",
            &[&id],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "succeeded");
    assert_eq!(row.get::<_, String>(1), "");
    server.abort();
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn failed_http_renew_preserves_issued_pair() {
    let (t, server) = setup(true).await;
    let input =
        service::parse_input(&json!({"domains":["example.com"],"email":"admin@example.com"}))
            .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    t.client
        .execute(
            "UPDATE certificate_jobs SET state='cancelled',payload_enc='' WHERE certificate_id=$1",
            &[&id],
        )
        .await
        .unwrap();
    t.client.execute("UPDATE certificates SET status='issued',cert_pem='live certificate',key_pem='live key',expires_at=now()+interval '90 days' WHERE id=$1",&[&id]).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(8),
        service::renew(&t.state, &id, &actor()),
    )
    .await
    .unwrap();
    assert!(result.is_err());
    worker.shutdown().await.unwrap();
    let cert = store::get(&t.client, &id, false).await.unwrap();
    assert_eq!(cert.status, "issued");
    assert_eq!(cert.cert_pem.as_deref(), Some("live certificate"));
    assert_eq!(cert.key_pem.as_deref(), Some("live key"));
    assert_eq!(cert.failure_count, 1);
    server.abort();
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn manual_dns_complete_resumes_persisted_csr_and_sealed_key() {
    let (t, server) = setup(false).await;
    let input = service::parse_input(
        &json!({"domains":["example.com"],"email":"admin@example.com","challengeType":"dns-01"}),
    )
    .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    wait(&t, &id, "awaiting-dns").await;
    // This CA mock supplies no external DNS dependency. Wire/DNS TXT gates cover
    // record checking separately; here the submitted persisted order is exercised.
    for _ in 0..100 {
        if !t.client.query_one("SELECT EXISTS(SELECT 1 FROM certificate_jobs WHERE certificate_id=$1 AND state IN ('active','cleanup'))",&[&id]).await.unwrap().get::<_,bool>(0){break;}
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    t.client
        .execute(
            "UPDATE certificates SET dns_order=jsonb_set(dns_order,'{records}','[]') WHERE id=$1",
            &[&id],
        )
        .await
        .unwrap();
    assert_eq!(
        service::submit_dns(&t.state, &id, &actor()).await.unwrap()["submitted"],
        true
    );
    wait(&t, &id, "issued").await;
    worker.shutdown().await.unwrap();
    let cert = store::get(&t.client, &id, false).await.unwrap();
    assert!(cert.dns_order.is_none());
    let key = t.state.secrets.open(cert.key_pem.as_deref()).unwrap();
    pem::parse_certificate_pem(cert.cert_pem.as_deref().unwrap(), &key).unwrap();
    server.abort();
}

async fn terminal(t: &auth_support::TestDb, id: &str) -> String {
    for _ in 0..1500 {
        let state:String=t.client.query_one("SELECT state FROM certificate_jobs WHERE certificate_id=$1 ORDER BY created_at DESC LIMIT 1",&[&id]).await.unwrap().get(0);
        if ["succeeded", "failed", "cancelled"].contains(&state.as_str()) {
            return state;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("job did not settle");
}
async fn linked_child(name:&str)->bool {
    if std::env::var_os("NWUI_CERT_BOOT_CHILD").is_some(){return true;}
    let root=tempfile::tempdir().unwrap();
    let output=tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact",name,"--ignored","--nocapture","--test-threads=1"])
        .env("NWUI_CERT_BOOT_CHILD",root.path()).env("PATH",format!("{}:/usr/sbin:/usr/bin:/sbin:/bin",root.path().display()))
        .kill_on_drop(true).output().await.unwrap();
    assert!(output.status.success(),"{name}: {}\n{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
    let stdout=String::from_utf8_lossy(&output.stdout);
    assert!(stdout.lines().any(|line|line.starts_with("test result: ok. 1 passed; 0 failed;")),"{name}: child must execute exactly one test: {stdout}");
    false
}
async fn linked_issuance(
    fail_apply: bool,
) -> (
    auth_support::TestDb,
    tokio::task::JoinHandle<()>,
    Arc<tokio::sync::Mutex<Ca>>,
    tempfile::TempDir,
    String,
) {
    use std::os::unix::fs::PermissionsExt;
    let root=std::path::PathBuf::from(std::env::var_os("NWUI_CERT_BOOT_CHILD").expect("linked baseline runs in its admission-isolated child"));
    let tmp = tempfile::tempdir_in(&root).unwrap();
    let bin = tmp.path().join("nginx");
    let fault = tmp.path().join("apply-fault");
    std::fs::write(&bin,format!("#!/bin/sh\nif [ -f '{}' ]; then echo 'injected apply failure' >&2; exit 1; fi\nexit 0\n",fault.display())).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    let paths = [
        bin.display().to_string(),
        tmp.path().join("conf").display().to_string(),
        tmp.path().join("stream").display().to_string(),
        tmp.path().join("certs").display().to_string(),
        tmp.path().join("main.conf").display().to_string(),
        tmp.path().join("quarantine").display().to_string(),
    ];
    let (mut t, socket, base) = setup_environment(
        &[
            ("NGINX_BIN", &paths[0]),
            ("NGINX_CONF_D", &paths[1]),
            ("NGINX_STREAM_D", &paths[2]),
            ("CERTS_DIR", &paths[3]),
            ("NGINX_CONF", &paths[4]),
            ("NGINX_GUARD_QUARANTINE", &paths[5]),
        ],
    )
    .await;
    let input =
        service::parse_input(&json!({"domains":["example.com"],"email":"admin@example.com"}))
            .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    t.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES($1,'example.com',19090,$2,$3)",&[&uuid::Uuid::new_v4().to_string(),&id,&json!({"proxyPass":"http://192.0.2.5:9000"})]).await.unwrap();
    // A deploy replaces material used by a live server; saved-only linkage never reloads.
    let previous = rcgen::generate_simple_self_signed(vec!["example.com".into()]).unwrap();
    let previous_key = t.state.secrets.seal(&previous.signing_key.serialize_pem()).unwrap();
    t.client.execute("UPDATE certificates SET status='issued',cert_pem=$2,key_pem=$3 WHERE id=$1", &[&id,&previous.cert.pem(),&previous_key]).await.unwrap();
    use nwui_api::publisher::{boot,test_support};
    std::fs::create_dir_all(&t.state.config.nginx_conf_dir).unwrap();
    std::fs::create_dir_all(&t.state.config.nginx_stream_dir).unwrap();
    std::fs::write(&t.state.config.nginx_main_conf,format!("pid {}; events {{}} http {{ include {}/*.conf; server {{ listen 127.0.0.1:8080; location = /nginx_status {{ stub_status; allow 127.0.0.3; deny all; include {}/.nwui-generation; }} }} }}",tmp.path().join("nginx.pid").display(),t.state.config.nginx_conf_dir.display(),t.state.config.nginx_conf_dir.display())).unwrap();
    let version=format!("if [ \"$1\" = -V ]; then printf '%s\\n' 'nginx version: nginx/1.30.6' 'configure arguments: --prefix={} --conf-path={}' >&2; exit 0; fi\n",tmp.path().display(),t.state.config.nginx_main_conf.display());
    std::fs::write(&bin,format!("#!/bin/sh\n{version}for arg do if [ \"$arg\" = -T ]; then printf '# configuration file %s:\\n' '{}'; fi; done\nexit 0\n",t.state.config.nginx_main_conf.display())).unwrap();
    std::fs::copy(&bin,root.join("nginx")).unwrap();
    let store=test_support::open_store_fixture(t.state.config.data_key_file.parent().unwrap()).unwrap();
    // No actor/history/audit baseline: publish while stopped, then start the witness and adopt.
    let payload=boot::prepare_payload(&t.state.config,&t.state.secrets,boot::PgSource::Pool(&t.state.db),&store,nwui_api::publisher::boot::BootMode::Conversion).await.unwrap();
    boot::publish_live(&t.state.config,&store,&payload.id).await.unwrap();
    auth_support::nginx::witness(&mut t.state);
    test_support::adopt_current(&t.state,&store).await.unwrap();
    let (server,ca)=start_ca(&t,socket,base,false);
    t.client.execute("UPDATE certificates SET status='issuing' WHERE id=$1", &[&id]).await.unwrap();
    let current=t.state.config.certs_dir.join(&id).join("fullchain.pem");let previous_file=tmp.path().join("previous-cert.pem");
    std::fs::copy(&current,&previous_file).unwrap();
    // Reject only the new material so the durable previous-payload restore can validate.
    std::fs::write(&bin,format!("#!/bin/sh\n{version}if [ -f '{}' ] && {{ grep -qx persistent '{}' || ! cmp -s '{}' '{}'; }}; then echo 'injected apply failure' >&2; exit 1; fi\nif [ \"$1\" = -T ]; then printf '# configuration file %s:\\n' '{}'; fi\nexit 0\n",fault.display(),fault.display(),current.display(),previous_file.display(),t.state.config.nginx_main_conf.display())).unwrap();
    if fail_apply {
        std::fs::write(&fault, "fail").unwrap();
    }
    (t, server, ca, tmp, id)
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn issued_leaf_apply_failure_is_pending_not_success_and_retries_without_another_order() {
    if !linked_child("issued_leaf_apply_failure_is_pending_not_success_and_retries_without_another_order").await {return;}
    let (t, server, ca, tmp, id) = linked_issuance(true).await;
    t.client
        .execute(
            "UPDATE certificate_jobs SET mode='renew' WHERE certificate_id=$1",
            &[&id],
        )
        .await
        .unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    wait(&t, &id, "issued").await;
    for _ in 0..100 {
        let err: Option<String> = t
            .client
            .query_one("SELECT last_error FROM certificates WHERE id=$1", &[&id])
            .await
            .unwrap()
            .get(0);
        if err.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let success: i64 = t
        .client
        .query_one(
            "SELECT count(*) FROM notifications WHERE type='cert.renewed'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        success, 0,
        "issuance must not announce deployment success before apply succeeds"
    );
    let failures: i64 = t
        .client
        .query_one(
            "SELECT count(*) FROM notifications WHERE type='cert.renew_failed'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(failures, 1);
    let state: String = t
        .client
        .query_one(
            "SELECT state FROM certificate_jobs WHERE certificate_id=$1",
            &[&id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(state, "cleanup", "pending deployment must remain retryable");
    // One immediate retry may fail; subsequent background attempts must be spaced out.
    t.client.execute("UPDATE certificate_jobs SET updated_at=now()-interval '1 minute' WHERE certificate_id=$1",&[&id]).await.unwrap();
    tokio::time::timeout(Duration::from_secs(20),async {
        loop {
            let attempts:i64=t.client.query_one("SELECT count(*) FROM audit_logs WHERE action='nginx:apply' AND NOT (\"after\"->>'ok')::boolean",&[]).await.unwrap().get(0);
            // The apply audit commits before retry_deployment persists its backoff.
            let postponed:bool=t.client.query_one("SELECT updated_at>=now()+interval '14 minutes' FROM certificate_jobs WHERE certificate_id=$1",&[&id]).await.unwrap().get(0);
            if attempts>=2 && postponed {break;}tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    let postponed:bool=t.client.query_one("SELECT updated_at>=now()+interval '14 minutes' FROM certificate_jobs WHERE certificate_id=$1",&[&id]).await.unwrap().get(0);
    assert!(
        postponed,
        "second deployment failure must postpone further background applies"
    );
    let attempts:i64=t.client.query_one("SELECT count(*) FROM audit_logs WHERE action='nginx:apply' AND NOT (\"after\"->>'ok')::boolean",&[]).await.unwrap().get(0);
    assert!(attempts <= 2);
    let leaf = store::get(&t.client, &id, false)
        .await
        .unwrap()
        .cert_pem
        .unwrap();
    worker.shutdown().await.unwrap();
    std::fs::remove_file(tmp.path().join("apply-fault")).unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    t.client.execute("UPDATE certificate_jobs SET updated_at=now()-interval '1 minute' WHERE certificate_id=$1",&[&id]).await.unwrap();
    assert_eq!(terminal(&t, &id).await, "succeeded");
    worker.shutdown().await.unwrap();
    assert_eq!(ca.lock().await.orders, 1);
    assert_eq!(
        store::get(&t.client, &id, false)
            .await
            .unwrap()
            .cert_pem
            .as_deref(),
        Some(leaf.as_str())
    );
    assert!(
        store::get(&t.client, &id, false)
            .await
            .unwrap()
            .last_error
            .is_none()
    );
    assert_eq!(
        t.client
            .query_one(
                "SELECT count(*) FROM notifications WHERE type='cert.renewed'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn persistent_deploy_and_restore_failure_blocks_until_publisher_restart_recovers_without_a_new_order() {
    if !linked_child("persistent_deploy_and_restore_failure_blocks_until_publisher_restart_recovers_without_a_new_order").await {return;}
    use nwui_api::{control::model,publisher::test_support,publisher_client::{self,Client,ClientError}};
    let(mut t,ca_server,ca,tmp,id)=linked_issuance(true).await;
    t.client.execute("UPDATE certificate_jobs SET mode='renew' WHERE certificate_id=$1",&[&id]).await.unwrap();
    std::fs::write(tmp.path().join("apply-fault"),"persistent").unwrap();
    let directory=tmp.path().join("publisher-socket");let socket=directory.join("control.sock");
    let mut runtime=nwui_api::state::RuntimeState::default();runtime.publisher=Client::socket_for_tests(socket,Default::default());t.state.runtime=Arc::new(runtime);
    let publisher=test_support::serve(&t.state,&directory,41).unwrap();
    let worker=jobs::start(t.state.clone()).await.unwrap();wait(&t,&id,"issued").await;
    tokio::time::timeout(Duration::from_secs(20),async {
        loop {
            let failed:bool=t.client.query_one("SELECT last_error IS NOT NULL FROM certificates WHERE id=$1",&[&id]).await.unwrap().get(0);
            if failed {break;}tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    worker.shutdown().await.unwrap();
    let row=t.client.query_one("SELECT cert_pem,key_pem FROM certificates WHERE id=$1",&[&id]).await.unwrap();let leaf:String=row.get(0);let sealed:String=row.get(1);let digest=model::certificate_content_digest(&leaf,&sealed);
    let error=publisher_client::deploy_certificate(&t.state,&id,&digest,&actor()).await.unwrap_err();
    assert!(matches!(error.downcast_ref::<ClientError>(),Some(ClientError::Unavailable{..})),"{error:#}");
    assert!(t.state.config.nginx_conf_dir.join(".nwui-apply-uncertain").exists());
    let row=t.client.query_one("SELECT deploy_pending,state FROM certificate_jobs WHERE certificate_id=$1",&[&id]).await.unwrap();assert!(row.get::<_,bool>(0));assert_eq!(row.get::<_,String>(1),"cleanup");
    assert_eq!(ca.lock().await.orders,1);assert_eq!(std::fs::read(t.state.config.certs_dir.join(&id).join("fullchain.pem")).unwrap(),std::fs::read(tmp.path().join("previous-cert.pem")).unwrap());
    assert_eq!(t.client.query_one("SELECT count(*) FROM notifications WHERE type='cert.renewed'",&[]).await.unwrap().get::<_,i64>(0),0);
    drop(publisher);std::fs::remove_file(tmp.path().join("apply-fault")).unwrap();
    test_support::restart_after_start(&t.state).await.unwrap();
    let _publisher=test_support::serve(&t.state,&directory,42).unwrap();assert!(publisher_client::hello(&t.state).await.unwrap().ready);
    t.client.execute("UPDATE certificate_jobs SET updated_at=now()-interval '1 minute' WHERE certificate_id=$1",&[&id]).await.unwrap();
    let worker=jobs::start(t.state.clone()).await.unwrap();assert_eq!(terminal(&t,&id).await,"succeeded");worker.shutdown().await.unwrap();
    assert_eq!(ca.lock().await.orders,1);assert_eq!(std::fs::read_to_string(t.state.config.certs_dir.join(&id).join("fullchain.pem")).unwrap(),leaf);
    assert!(!t.state.config.nginx_conf_dir.join(".nwui-apply-uncertain").exists());
    assert!(!t.client.query_one("SELECT deploy_pending FROM certificate_jobs WHERE certificate_id=$1",&[&id]).await.unwrap().get::<_,bool>(0));
    assert_eq!(t.client.query_one("SELECT count(*) FROM notifications WHERE type='cert.renewed'",&[]).await.unwrap().get::<_,i64>(0),1);
    ca_server.abort();
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn linked_issuance_preserves_unapplied_edits_without_history_and_records_system_audit() {
    if !linked_child("linked_issuance_preserves_unapplied_edits_without_history_and_records_system_audit").await {return;}
    let (t, server, _ca, _tmp, id) = linked_issuance(false).await;
    auth_support::insert_user(&t.client,auth_support::UserRow::default(),None).await;
    let via=json!({"kind":"token","id":"issuance-token","name":"job token"});
    t.client.execute("UPDATE certificate_jobs SET actor=actor||jsonb_build_object('id','u1','via',$1::jsonb) WHERE certificate_id=$2",&[&via,&id]).await.unwrap();
    t.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('unrelated-draft','draft.example.com',19091,$1)", &[&json!({"proxyPass":"http://192.0.2.5:9000"})]).await.unwrap();
    assert_eq!(nwui_api::control::pending::status(&t.state).await.unwrap().count, 1);
    let worker = jobs::start(t.state.clone()).await.unwrap();
    assert_eq!(terminal(&t, &id).await, "succeeded");
    worker.shutdown().await.unwrap();
    let snapshots: i64 = t
        .client
        .query_one("SELECT count(*) FROM config_snapshots", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        snapshots, 0,
        "certificate-only deployment must not create configuration history"
    );
    assert_eq!(nwui_api::control::pending::status(&t.state).await.unwrap().count, 1);
    assert!(!t.state.config.nginx_conf_dir.join("server-unrelated-draft.conf").exists());
    let actor:String=t.client.query_one("SELECT actor_label FROM audit_logs WHERE action='nginx:apply' AND (\"after\"->>'ok')::boolean",&[]).await.unwrap().get(0);
    assert_eq!(actor, "system:certificate");
    assert!(t.client.query_one("SELECT via IS NULL FROM audit_logs WHERE action='nginx:apply' AND (\"after\"->>'ok')::boolean",&[]).await.unwrap().get::<_,bool>(0));
    assert_eq!(t.client.query_one("SELECT via FROM audit_logs WHERE action='cert:issue-result' AND target=$1",&[&id]).await.unwrap().get::<_,Value>(0),via);
    assert_eq!(t.client.query_one("SELECT actor_id FROM audit_logs WHERE action='cert:issue-result' AND target=$1",&[&id]).await.unwrap().get::<_,String>(0),"u1");
    server.abort();
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn finish_pool_timeout_retains_the_issued_leaf_and_retries_commit() {
    let (t, server, ca) = setup_with(false, &[("API_DB_POOL_SIZE", "1")]).await;
    let input = service::parse_input(
        &json!({"domains":["example.com"],"email":"admin@example.com","challengeType":"dns-01"}),
    )
    .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    wait(&t, &id, "awaiting-dns").await;
    assert_eq!(terminal(&t, &id).await, "succeeded");
    t.client
        .execute(
            "UPDATE certificates SET dns_order=jsonb_set(dns_order,'{records}','[]') WHERE id=$1",
            &[&id],
        )
        .await
        .unwrap();
    let (gate, held) = tokio::sync::oneshot::channel();
    ca.lock().await.finish_gate = Some(gate);
    service::submit_dns(&t.state, &id, &actor()).await.unwrap();
    let held = tokio::time::timeout(Duration::from_secs(5), held)
        .await
        .unwrap()
        .unwrap();
    let failed = worker.failure_receiver();
    tokio::time::sleep(Duration::from_secs(6)).await;
    let unexpectedly_failed = *failed.borrow();
    drop(held);
    assert!(
        !unexpectedly_failed,
        "finish pool timeout must not discard the issued leaf"
    );
    wait(&t, &id, "issued").await;
    assert_eq!(terminal(&t, &id).await, "succeeded");
    worker.shutdown().await.unwrap();
    assert_eq!(ca.lock().await.orders, 1);
    let cert = store::get(&t.client, &id, false).await.unwrap();
    let key = t.state.secrets.open(cert.key_pem.as_deref()).unwrap();
    pem::parse_certificate_pem(cert.cert_pem.as_deref().unwrap(), &key).unwrap();
    server.abort();
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn late_cancel_does_not_discard_a_validated_issued_leaf() {
    let (t, server, ca) = setup_with(false, &[]).await;
    let(gate,held)=tokio::sync::oneshot::channel();ca.lock().await.late_cancel_gate=Some(gate);
    let input =
        service::parse_input(&json!({"domains":["example.com"],"email":"admin@example.com"}))
            .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    let held=tokio::time::timeout(Duration::from_secs(5),held).await.unwrap().unwrap();
    // finish() reaches this row lock only after local PEM validation has produced Issued.
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let finishing:bool=t.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query='SELECT id FROM certificates WHERE id=$1 FOR UPDATE')",&[]).await.unwrap().get(0);
            if finishing{break;}tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("validated issuance must reach finish before late cancellation");
    tokio::time::timeout(Duration::from_secs(5),t.client.execute("UPDATE certificate_jobs SET cancel_requested=true WHERE certificate_id=$1",&[&id]))
        .await.expect("late cancellation must not wait for finish to release the job row").unwrap();
    held.batch_execute("COMMIT").await.unwrap();
    let state = terminal(&t, &id).await;
    worker.shutdown().await.unwrap();
    let cert = store::get(&t.client, &id, false).await.unwrap();
    assert_eq!(
        cert.status, "issued",
        "a late cancel must not discard a completed CA issuance ({state})"
    );
    assert_eq!(ca.lock().await.orders, 1);
    let key = t.state.secrets.open(cert.key_pem.as_deref()).unwrap();
    pem::parse_certificate_pem(cert.cert_pem.as_deref().unwrap(), &key).unwrap();
    server.abort();
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn failed_renewal_keeps_attempt_start_for_the_next_twelve_hour_sweep() {
    let (t, server) = setup(true).await;
    let input =
        service::parse_input(&json!({"domains":["example.com"],"email":"admin@example.com"}))
            .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    t.client
        .execute(
            "UPDATE certificate_jobs SET mode='renew' WHERE certificate_id=$1",
            &[&id],
        )
        .await
        .unwrap();
    t.client.execute("UPDATE certificates SET status='issued',expires_at=now()+interval '20 days',last_attempt_at=now()-interval '10 minutes' WHERE id=$1",&[&id]).await.unwrap();
    let started: chrono::DateTime<chrono::Utc> = t
        .client
        .query_one(
            "SELECT last_attempt_at FROM certificates WHERE id=$1",
            &[&id],
        )
        .await
        .unwrap()
        .get(0);
    let worker = jobs::start(t.state.clone()).await.unwrap();
    assert_eq!(terminal(&t, &id).await, "failed");
    worker.shutdown().await.unwrap();
    let finished: chrono::DateTime<chrono::Utc> = t
        .client
        .query_one(
            "SELECT last_attempt_at FROM certificates WHERE id=$1",
            &[&id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        finished, started,
        "completion must not push the retry beyond the next 12h sweep"
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn user_pem_replace_keeps_the_callers_apply_actor() {
    if !linked_child("user_pem_replace_keeps_the_callers_apply_actor").await {return;}
    let (t, server, _ca, _tmp, id) = linked_issuance(false).await;
    t.client
        .execute(
            "UPDATE certificate_jobs SET state='cancelled' WHERE certificate_id=$1",
            &[&id],
        )
        .await
        .unwrap();
    t.client
        .execute(
            "UPDATE certificates SET source='manual',status='issued' WHERE id=$1",
            &[&id],
        )
        .await
        .unwrap();
    let leaf = rcgen::generate_simple_self_signed(vec!["example.com".into()]).unwrap();
    let input = service::CertificatePemInput {
        cert_pem: leaf.cert.pem(),
        key_pem: leaf.signing_key.serialize_pem(),
    };
    let actor = Actor {
        via: None,
        id: None,
        label: Some("gateway:editor".into()),
        ip: None,
    };
    assert!(
        service::replace(&t.state, &id, &input, &actor)
            .await
            .unwrap()["applyError"]
            .is_null()
    );
    let label: String = t
        .client
        .query_one(
            "SELECT actor_label FROM audit_logs WHERE action='nginx:apply'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(label, "gateway:editor");
    server.abort();
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn cancelled_failed_deployment_does_not_promise_another_retry() {
    if !linked_child("cancelled_failed_deployment_does_not_promise_another_retry").await {return;}
    use std::os::unix::fs::PermissionsExt;
    let (t, server, _ca, tmp, id) = linked_issuance(true).await;
    let started = tmp.path().join("started");
    let release = tmp.path().join("release");
    let bin = tmp.path().join("nginx");
    std::fs::write(&bin,format!("#!/bin/sh\ntouch '{}'\nwhile [ ! -f '{}' ]; do sleep 0.02; done\necho 'injected apply failure' >&2\nexit 1\n",started.display(),release.display())).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !started.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let state = t.state.clone();
    let cancel_id = id.clone();
    let cancel = tokio::spawn(async move { service::cancel(&state, &cancel_id, &actor()).await });
    tokio::time::timeout(Duration::from_secs(3),async {
        loop {
            let blocked:bool=t.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE 'UPDATE certificates SET status=CASE WHEN%')",&[]).await.unwrap().get(0);
            if blocked {break;}tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("cancel must reach the certificate table lock held by apply");
    std::fs::write(release, "go").unwrap();
    cancel.await.unwrap().unwrap();
    assert_eq!(terminal(&t, &id).await, "cancelled");
    worker.shutdown().await.unwrap();
    let title: String = t
        .client
        .query_one(
            "SELECT title FROM notifications WHERE type='cert.failed'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !title.contains("다시 적용"),
        "cancelled jobs must not promise retry"
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn shutdown_stops_finish_retries_after_a_final_attempt() {
    let (t, server, ca) = setup_with(false, &[("API_DB_POOL_SIZE", "1")]).await;
    let input = service::parse_input(
        &json!({"domains":["example.com"],"email":"admin@example.com","challengeType":"dns-01"}),
    )
    .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    let worker = jobs::start(t.state.clone()).await.unwrap();
    wait(&t, &id, "awaiting-dns").await;
    assert_eq!(terminal(&t, &id).await, "succeeded");
    t.client
        .execute(
            "UPDATE certificates SET dns_order=jsonb_set(dns_order,'{records}','[]') WHERE id=$1",
            &[&id],
        )
        .await
        .unwrap();
    let (gate, held) = tokio::sync::oneshot::channel();
    ca.lock().await.finish_gate = Some(gate);
    service::submit_dns(&t.state, &id, &actor()).await.unwrap();
    let held = tokio::time::timeout(Duration::from_secs(5), held)
        .await
        .unwrap()
        .unwrap();
    tokio::time::sleep(Duration::from_secs(6)).await;
    let stopped = tokio::time::timeout(Duration::from_secs(12), worker.shutdown()).await;
    drop(held);
    server.abort();
    assert!(
        stopped.is_ok(),
        "finish must not ignore shutdown indefinitely while the pool remains busy"
    );
}
