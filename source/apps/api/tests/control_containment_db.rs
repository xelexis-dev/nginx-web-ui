#![cfg(unix)]
mod auth_support;
mod g13_support;
use nwui_api::{
    config::RuntimeConfig,
    contracts::Actor,
    control::pending,
};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};
use tempfile::TempDir;

// The size-one pool’s 5 s wait timeout proves release; this only stops a true hang.
const HANG_BACKSTOP: Duration = Duration::from_secs(120);
// Post-commit SQL allows 2 s; leave 10 s for scheduling under full-suite load.
const AUDIT_BOUND: Duration = Duration::from_secs(2 + 10);

fn layout(dir: &Path) -> Vec<(String, String)> {
    let conf = dir.join("conf");
    let stream = dir.join("stream");
    let bin = dir.join("nginx");
    std::fs::create_dir_all(&conf).unwrap();
    std::fs::create_dir_all(&stream).unwrap();
    std::fs::write(
        dir.join("nginx.conf"),
        "listen 80;\nlisten 443;\nlisten 8080;\n",
    )
    .unwrap();
    std::fs::write(
        &bin,
        format!(
            r#"#!/bin/sh
[ "$1" = -t ] || exit 0
for f in '{conf}'/*.conf '{stream}'/*.conf; do
 [ -f "$f" ] || continue
 n=$(grep -n -E 'rejected\.example|BAD' "$f" | head -1 | cut -d: -f1)
 if [ -n "$n" ]; then echo "nginx: [emerg] unknown directive BAD in $f:$n"; exit 1; fi
done
exit 0
"#,
            conf = conf.display(),
            stream = stream.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    vec![
        (
            "MIGRATIONS_DIR",
            Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../packages/db/drizzle"
            ))
            .to_path_buf(),
        ),
        ("NGINX_BIN", bin),
        ("NGINX_CONF_D", conf),
        ("NGINX_STREAM_D", stream),
        ("CERTS_DIR", dir.join("certs")),
        ("NGINX_CONF", dir.join("nginx.conf")),
        ("NGINX_GUARD_QUARANTINE", dir.join("quarantine")),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.display().to_string()))
    .chain([("API_DB_POOL_SIZE".into(), "1".into())])
    .collect()
}
async fn setup() -> (auth_support::TestDb, TempDir) {
    let temp = TempDir::new().unwrap();
    let env = layout(temp.path());
    let refs: Vec<_> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let mut t = auth_support::test_db_with(&refs).await;
    g13_support::witness(&mut t.state);
    (t, temp)
}
fn server(id: &str, name: &str, listen: u16) -> Value {
    json!({"id":id,"serverName":name,"listen":listen,"enabled":true,"certificateId":null,"upstreamId":null,"config":{"proxyPass":"http://192.0.2.1:80"}})
}
async fn seed(t: &auth_support::TestDb, rows: &[Value]) {
    for row in rows {
        t.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config,enabled) VALUES($1,$2,$3,$4,true)",&[&row["id"].as_str().unwrap(),&row["serverName"].as_str().unwrap(),&(row["listen"].as_i64().unwrap() as i32),&row["config"]]).await.unwrap();
    }
}
async fn marker(t: &auth_support::TestDb, rows: &[Value]) {
    let value = json!({"upstreams":[],"servers":rows,"streams":[]});
    t.client
        .execute(
            "INSERT INTO applied_config(id,data) VALUES('current',$1)",
            &[&value],
        )
        .await
        .unwrap();
}
fn old_file(t: &auth_support::TestDb, id: &str, name: &str, listen: u16) {
    std::fs::write(
        t.state
            .config
            .nginx_conf_dir
            .join(format!("server-{id}.conf")),
        format!("server {{ listen {listen}; server_name {name}; }}\n"),
    )
    .unwrap();
}
fn actor() -> Actor {
    Actor {
        id: None,
        label: Some("containment test".into()),
        ip: None,
    }
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn apply_retains_previous_marker_row_pending_and_notifies_once_at_pool_one() {
    if g13_support::isolated("apply_retains_previous_marker_row_pending_and_notifies_once_at_pool_one").await { return; }
    let (t, _temp) = setup().await;
    let old = [
        server("a", "old-a.example", 8081),
        server("b", "old-b.example", 8082),
    ];
    let new = [
        server("a", "new-a.example", 8081),
        server("b", "rejected.example", 8082),
    ];
    seed(&t, &new).await;
    marker(&t, &old).await;
    old_file(&t, "a", "old-a.example", 8081);
    old_file(&t, "b", "old-b.example", 8082);
    let r = tokio::time::timeout(
        HANG_BACKSTOP,
        nwui_api::publisher_client::apply_saved(&t.state, &actor(), Some("partial")),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(r.ok, "{}", r.output);
    let m: Value = t
        .client
        .query_one("SELECT data FROM applied_config WHERE id='current'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(m["servers"][0]["serverName"], "new-a.example");
    assert_eq!(m["servers"][1]["serverName"], "old-b.example");
    assert_eq!(m["skipped"][0]["kept_previous"], true);
    let diff = pending::status(&t.state).await.unwrap();
    assert_eq!(diff.count, 1);
    assert_eq!(
        t.client
            .query_one("SELECT count(*) FROM config_snapshots", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    assert_eq!(
        t.client
            .query_one(
                "SELECT count(*) FROM audit_logs WHERE action='nginx:apply'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    let notice = t
        .client
        .query_one(
            "SELECT body FROM notifications WHERE type='config.apply-skipped'",
            &[],
        )
        .await
        .unwrap()
        .get::<_, String>(0);
    assert!(notice.contains("직전 적용본"));
    assert!(notice.contains("rejected.example"));
    assert!(nwui_api::publisher_client::apply_saved(&t.state,&actor(),None).await.unwrap().ok);
    assert_eq!(
        t.client
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
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn unmarked_previous_file_is_dropped_on_disk_as_well_as_from_the_marker() {
    if g13_support::isolated("unmarked_previous_file_is_dropped_on_disk_as_well_as_from_the_marker").await { return; }
    let (t, _temp) = setup().await;
    seed(
        &t,
        &[
            server("a", "good.example", 8081),
            server("b", "rejected.example", 8082),
        ],
    )
    .await;
    old_file(&t, "b", "unmarked.example", 8082);
    let r = nwui_api::publisher_client::apply_saved(&t.state,&actor(),None).await.unwrap();
    assert!(r.ok, "{}", r.output);
    assert!(!t.state.config.nginx_conf_dir.join("server-b.conf").exists());
    let m: Value = t
        .client
        .query_one("SELECT data FROM applied_config WHERE id='current'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(m["servers"].as_array().unwrap().len(), 1);
    assert_eq!(m["skipped"][0]["kept_previous"], false);
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn rollback_contains_the_rejected_row_and_emits_one_notice_at_pool_one() {
    if g13_support::isolated("rollback_contains_the_rejected_row_and_emits_one_notice_at_pool_one").await { return; }
    let (t, _temp) = setup().await;
    let old = [
        server("a", "old-a.example", 8081),
        server("b", "old-b.example", 8082),
    ];
    seed(&t, &old).await;
    marker(&t, &old).await;
    old_file(&t, "a", "old-a.example", 8081);
    old_file(&t, "b", "old-b.example", 8082);
    let snap = json!({"upstreams":[],"servers":[server("a","new-a.example",8081),server("b","rejected.example",8082)],"streams":[]});
    t.client
        .execute(
            "INSERT INTO config_snapshots(id,data) VALUES('7a000000-0000-4000-8000-000000000003',$1)",
            &[&snap],
        )
        .await
        .unwrap();
    let r = tokio::time::timeout(
        HANG_BACKSTOP,
        nwui_api::publisher_client::rollback_snapshot(&t.state, "7a000000-0000-4000-8000-000000000003", &actor()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(r.result.ok, "{}", r.result.output);
    let m: Value = t
        .client
        .query_one("SELECT data FROM applied_config WHERE id='current'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(m["servers"][1]["serverName"], "old-b.example");
    assert_eq!(m["skipped"][0]["kept_previous"], true);
    assert_eq!(
        t.client
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



#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn an_uncertainty_receipt_hides_the_baseline_without_destroying_the_saved_marker() {
    if g13_support::isolated("an_uncertainty_receipt_hides_the_baseline_without_destroying_the_saved_marker").await { return; }
    let (t,_temp)=setup().await;let rows=[server("a","a.example",8081)];
    seed(&t,&rows).await;marker(&t,&rows).await;
    assert!(pending::status(&t.state).await.unwrap().has_baseline);
    let path=t.state.config.nginx_conf_dir.join(".nwui-apply-uncertain");
    std::fs::write(&path,"unconfirmed").unwrap();
    let view=pending::status(&t.state).await.unwrap();
    assert!(!view.has_baseline,"a stored marker cannot attest the running worker after an uncertain reload");
    assert_eq!(view.count,1);
    let mut config=RuntimeConfig::read(|key|Some(match key {
        "DATABASE_URL"=>t.state.config.database_url.clone(),"AUTH_SECRET"=>auth_support::SECRET.into(),
        "DATA_KEY_FILE"=>t.state.config.data_key_file.display().to_string(),"NGINX_CONF_D"=>t.state.config.nginx_conf_dir.display().to_string(),"API_DB_POOL_SIZE"=>"1".into(),_=>return None,
    })).unwrap();config.target_dns.descriptor_path=t.state.config.target_dns.descriptor_path.clone();
    let restarted=nwui_api::state::AppState::new(config).unwrap();
    assert!(!pending::status(&restarted).await.unwrap().has_baseline,"the uncertainty receipt survives API state recreation");
    assert_eq!(t.client.query_one("SELECT count(*) FROM applied_config",&[]).await.unwrap().get::<_,i64>(0),1);
    t.client.execute("DELETE FROM nginx_servers",&[]).await.unwrap();
    let empty=pending::status(&t.state).await.unwrap();
    assert!(!empty.has_baseline);assert_eq!(empty.count,0);assert!(!empty.diff.has_changes);
    assert_eq!(t.client.query_one("SELECT jsonb_array_length(data->'servers') FROM applied_config",&[]).await.unwrap().get::<_,i32>(0),1);
    std::fs::remove_file(path).unwrap();assert!(pending::status(&t.state).await.unwrap().has_baseline);
}


#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn uncertain_backup_is_dropped_with_a_reason_and_keep_resumes_after_confirmation() {
    if g13_support::isolated("uncertain_backup_is_dropped_with_a_reason_and_keep_resumes_after_confirmation").await { return; }
    let (t,_temp)=setup().await;
    seed(&t,&[server("a","good.example",8081),server("b","rejected.example",8082)]).await;
    marker(&t,&[server("a","good.example",8081),server("b","old-b.example",8082)]).await;
    old_file(&t,"b","old-b.example",8082);
    std::fs::write(t.state.config.nginx_conf_dir.join(".nwui-apply-uncertain"),"interrupted").unwrap();
    let first=nwui_api::publisher_client::apply_saved(&t.state,&actor(),None).await.unwrap();assert!(first.ok,"{}",first.output);
    assert!(!t.state.config.nginx_conf_dir.join("server-b.conf").exists(),"an uncertain candidate cannot stand in for the old marker row");
    let state:Value=t.client.query_one("SELECT data FROM applied_config",&[]).await.unwrap().get(0);
    assert_eq!(state["skipped"][0]["kept_previous"],false);
    assert!(state["skipped"][0]["reason"].as_str().unwrap().contains("확인하지"));
    let body:String=t.client.query_one("SELECT body FROM notifications WHERE type='config.apply-skipped'",&[]).await.unwrap().get(0);
    assert!(body.contains("확인하지"),"the notice must explain why previous bytes were not kept");
    t.client.execute("UPDATE nginx_servers SET server_name='confirmed-b.example' WHERE id='b'",&[]).await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&t.state,&actor(),None).await.unwrap().ok);
    assert!(!t.state.config.nginx_conf_dir.join(".nwui-apply-uncertain").exists());
    t.client.execute("UPDATE nginx_servers SET server_name='rejected.example' WHERE id='b'",&[]).await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&t.state,&actor(),None).await.unwrap().ok);
    assert!(std::fs::read_to_string(t.state.config.nginx_conf_dir.join("server-b.conf")).unwrap().contains("confirmed-b.example"));
}


#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn failed_commit_keeps_uncertainty_until_the_next_confirmed_apply() {
    if g13_support::isolated("failed_commit_keeps_uncertainty_until_the_next_confirmed_apply").await { return; }
    let (t,_temp)=setup().await;seed(&t,&[server("a","old-a.example",8081)]).await;
    assert!(nwui_api::publisher_client::apply_saved(&t.state,&actor(),None).await.unwrap().ok);
    let file=t.state.config.nginx_conf_dir.join("server-a.conf");let old=std::fs::read(&file).unwrap();
    t.client.batch_execute("CREATE FUNCTION fail_ack_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'ack commit fault'; END $$; CREATE CONSTRAINT TRIGGER fail_ack_commit AFTER INSERT OR UPDATE ON applied_config DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION fail_ack_commit(); UPDATE nginx_servers SET server_name='new-a.example';").await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&t.state,&actor(),None).await.is_err());assert_eq!(std::fs::read(file).unwrap(),old);
    assert!(t.state.config.nginx_conf_dir.join(".nwui-apply-uncertain").exists());
    assert!(!pending::status(&t.state).await.unwrap().has_baseline);
    t.client.batch_execute("DROP TRIGGER fail_ack_commit ON applied_config; DROP FUNCTION fail_ack_commit();").await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&t.state,&actor(),None).await.unwrap().ok);
    assert!(!t.state.config.nginx_conf_dir.join(".nwui-apply-uncertain").exists());
    assert!(pending::status(&t.state).await.unwrap().has_baseline);
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn restoring_an_uncertain_backup_after_validation_failure_does_not_reestablish_a_baseline() {
    if g13_support::isolated("restoring_an_uncertain_backup_after_validation_failure_does_not_reestablish_a_baseline").await { return; }
    let (t,temp)=setup().await;seed(&t,&[server("a","new-a.example",8081)]).await;
    marker(&t,&[server("a","old-a.example",8081)]).await;old_file(&t,"a","uncommitted-a.example",8081);
    let receipt=t.state.config.nginx_conf_dir.join(".nwui-apply-uncertain");std::fs::write(&receipt,"interrupted").unwrap();
    let first=temp.path().join("first-validation");
    std::fs::write(temp.path().join("nginx"),format!("#!/bin/sh\nif [ \"$1\" = -t ] && [ ! -e '{}' ]; then touch '{}'; echo 'nginx: [emerg] injected global failure'; exit 1; fi\nexit 0\n",first.display(),first.display())).unwrap();
    let result=nwui_api::publisher_client::apply_saved(&t.state,&actor(),None).await.unwrap();assert!(!result.ok);
    assert!(result.rolled_back_to.is_none(),"restoring unknown bytes cannot establish the old baseline");
    assert!(receipt.exists());assert!(!pending::status(&t.state).await.unwrap().has_baseline);
    assert!(std::fs::read_to_string(t.state.config.nginx_conf_dir.join("server-a.conf")).unwrap().contains("uncommitted-a.example"));
    assert!(nwui_api::publisher_client::apply_saved(&t.state,&actor(),None).await.unwrap().ok);assert!(!receipt.exists());
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn restoring_an_uncertain_backup_after_metadata_failure_does_not_reestablish_a_baseline() {
    if g13_support::isolated("restoring_an_uncertain_backup_after_metadata_failure_does_not_reestablish_a_baseline").await { return; }
    let (t,_temp)=setup().await;seed(&t,&[server("a","new-a.example",8081)]).await;
    marker(&t,&[server("a","old-a.example",8081)]).await;old_file(&t,"a","uncommitted-a.example",8081);
    let receipt=t.state.config.nginx_conf_dir.join(".nwui-apply-uncertain");std::fs::write(&receipt,"interrupted").unwrap();
    t.client.batch_execute("CREATE FUNCTION fail_unknown_marker() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'unknown marker fault'; END $$; CREATE TRIGGER fail_unknown_marker BEFORE UPDATE ON applied_config FOR EACH ROW EXECUTE FUNCTION fail_unknown_marker();").await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&t.state,&actor(),None).await.is_err());
    assert!(receipt.exists(),"known rollback cannot make the pre-existing unknown bytes trustworthy");
    assert!(!pending::status(&t.state).await.unwrap().has_baseline);
    assert!(std::fs::read_to_string(t.state.config.nginx_conf_dir.join("server-a.conf")).unwrap().contains("uncommitted-a.example"));
    t.client.batch_execute("DROP TRIGGER fail_unknown_marker ON applied_config; DROP FUNCTION fail_unknown_marker();").await.unwrap();
    assert!(nwui_api::publisher_client::apply_saved(&t.state,&actor(),None).await.unwrap().ok);assert!(!receipt.exists());
}


async fn blocked_notice_tail(rollback: bool) {
    let (mut t, temp) = setup().await;
    let directory=temp.path().join("publisher-socket");
    let mut runtime=nwui_api::state::RuntimeState::default();
    runtime.publisher=nwui_api::publisher_client::Client::socket_for_tests(directory.join("control.sock"),nwui_api::publisher_client::Bounds{admission:Duration::from_millis(100),..Default::default()});
    t.state.runtime=std::sync::Arc::new(runtime);g13_support::witness(&mut t.state);
    let _server=nwui_api::publisher::test_support::serve(&t.state,&directory,29).unwrap();
    seed(&t, &[server("bad", "rejected.example", 8082)]).await;
    if rollback {
        let snapshot = json!({"upstreams":[],"servers":[server("bad","rejected.example",8082)],"streams":[]});
        t.client.execute("INSERT INTO config_snapshots(id,data) VALUES('7a000000-0000-4000-8000-000000000004',$1)", &[&snapshot]).await.unwrap();
    }
    t.client.batch_execute("BEGIN; LOCK TABLE notifications IN ACCESS EXCLUSIVE MODE").await.unwrap();
    let state = t.state.clone();
    let task = tokio::spawn(async move {
        if rollback { nwui_api::publisher_client::rollback_snapshot(&state,"7a000000-0000-4000-8000-000000000004",&actor()).await.map(|r|r.result) }
        else { nwui_api::publisher_client::apply_saved(&state,&actor(),None).await }
    });
    tokio::time::timeout(AUDIT_BOUND, async {
        loop {
            if t.client.query_one("SELECT count(*) FROM applied_config", &[]).await.unwrap().get::<_,i64>(0)>0 { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("confirmed metadata must commit before the notification");
    let busy=tokio::time::timeout(AUDIT_BOUND,nwui_api::publisher_client::apply_saved(&t.state,&actor(),None)).await.expect("concurrent admission must be bounded").unwrap_err();
    assert_eq!(busy.to_string(),"설정 적용 대기가 30초를 넘었습니다. 잠시 후 다시 시도하세요.");
    let result = tokio::time::timeout(AUDIT_BOUND,task).await.expect("notification SQL must be bounded").unwrap().unwrap();
    assert!(result.ok,"{}",result.output);
    assert!(!t.state.config.nginx_conf_dir.join(".nwui-apply-uncertain").exists());
    assert_eq!(t.client.query_one("SELECT count(*) FROM config_snapshots",&[]).await.unwrap().get::<_,i64>(0), if rollback {2} else {1});
    t.client.batch_execute("ROLLBACK").await.unwrap();
    assert!(tokio::time::timeout(AUDIT_BOUND,nwui_api::publisher_client::apply_saved(&t.state,&actor(),None)).await.unwrap().unwrap().ok,"a later apply must enter at pool size one");
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn blocked_notification_releases_apply_owner_and_preserves_confirmation() {
    if g13_support::isolated("blocked_notification_releases_apply_owner_and_preserves_confirmation").await { return; } blocked_notice_tail(false).await; }
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn blocked_notification_releases_rollback_owner_and_preserves_confirmation() {
    if g13_support::isolated("blocked_notification_releases_rollback_owner_and_preserves_confirmation").await { return; } blocked_notice_tail(true).await; }

async fn failed_apply_with_audit_fault(lock: bool, rollback: bool) {
    let (t,temp)=setup().await;
    seed(&t,&[server("a","new.example",8081)]).await;
    marker(&t,&[server("a","old.example",8081)]).await;
    old_file(&t,"a","old.example",8081);
    let first=temp.path().join("fail-once");
    std::fs::write(temp.path().join("nginx"),format!("#!/bin/sh\nif [ \"$1\" = -t ] && [ ! -e '{}' ]; then touch '{}'; echo 'nginx: [emerg] injected global failure'; exit 1; fi\nexit 0\n",first.display(),first.display())).unwrap();
    if rollback {
        let snapshot=json!({"upstreams":[],"servers":[server("a","new.example",8081)],"streams":[]});
        t.client.execute("INSERT INTO config_snapshots(id,data) VALUES('7a000000-0000-4000-8000-000000000005',$1)",&[&snapshot]).await.unwrap();
    }
    if lock { t.client.batch_execute("BEGIN; LOCK TABLE audit_logs IN ACCESS EXCLUSIVE MODE").await.unwrap(); }
    else { t.client.batch_execute("CREATE FUNCTION reject_failure_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'failure audit fault'; END $$; CREATE TRIGGER reject_failure_audit BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION reject_failure_audit()").await.unwrap(); }
    let state=t.state.clone();
    let task=tokio::spawn(async move {
        if rollback {nwui_api::publisher_client::rollback_snapshot(&state,"7a000000-0000-4000-8000-000000000005",&actor()).await.map(|r|r.result)}
        else {nwui_api::publisher_client::apply_saved(&state,&actor(),None).await}
    });
    let result=tokio::time::timeout(AUDIT_BOUND,task).await.expect("failure audit must be bounded").unwrap().expect("failure audit must preserve the original result");
    assert!(!result.ok);assert!(result.rolled_back_to.is_some(),"known recovery must survive audit failure");
    assert!(!t.state.config.nginx_conf_dir.join(".nwui-apply-uncertain").exists());
    assert_eq!(t.client.query_one("SELECT count(*) FROM config_snapshots",&[]).await.unwrap().get::<_,i64>(0),if rollback {1} else {0});
    let owner=t.state.runtime.apply_lock.try_lock().expect("failure audit must release the owner");drop(owner);
    if lock {t.client.batch_execute("ROLLBACK").await.unwrap();}
    assert!(tokio::time::timeout(AUDIT_BOUND,g13_support::baseline(&t.state)).await.unwrap().unwrap().ok);
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn blocked_failure_audit_is_bounded_and_preserves_recovery() {
    if g13_support::isolated("blocked_failure_audit_is_bounded_and_preserves_recovery").await { return; }failed_apply_with_audit_fault(true,false).await;}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn rejected_failure_audit_preserves_recovery() {
    if g13_support::isolated("rejected_failure_audit_preserves_recovery").await { return; }failed_apply_with_audit_fault(false,false).await;}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn blocked_rollback_failure_audit_is_bounded_and_preserves_recovery() {
    if g13_support::isolated("blocked_rollback_failure_audit_is_bounded_and_preserves_recovery").await { return; }failed_apply_with_audit_fault(true,true).await;}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn rejected_rollback_failure_audit_preserves_recovery() {
    if g13_support::isolated("rejected_rollback_failure_audit_preserves_recovery").await { return; }failed_apply_with_audit_fault(false,true).await;}
