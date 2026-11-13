mod auth_support;
mod g13_support;
use auth_support::*;
use nwui_api::{contracts::Actor, control::{model, pending, resources::{self, Mutation, Resource}}};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn actor() -> Actor { Actor { id: None, label: Some("version-test".into()), ip: None } }
fn migration_dir() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle") }
fn cases() -> [(Resource, Value);3] { [
    (Resource::Upstream, json!({"name":"backend","servers":["127.0.0.1:9000"]})),
    (Resource::Server, json!({"serverName":"example.test","listen":80,"proxyPass":"http://127.0.0.1:9000"})),
    (Resource::Stream, json!({"listen":19000,"protocol":"tcp","proxyPass":"127.0.0.1:9000"})),
] }
async fn create(db: &TestDb, kind: Resource, body: Value) -> Value {
    resources::mutate(&db.state,kind,Mutation::Create,None,body,&actor()).await.unwrap()
}
async fn get(db: &TestDb, kind: Resource, id: &str) -> Value {
    resources::get(&db.client,kind,id).await.unwrap()
}
fn version(row: &Value) -> i64 { row["version"].as_str().unwrap().parse().unwrap() }

#[tokio::test]
#[ignore = "real DB: NWUI_TEST_DATABASE_URL"]
async fn migration_backfills_11911_rows_and_runs_only_once() {
    let history = tempfile::tempdir().unwrap();
    std::fs::create_dir(history.path().join("meta")).unwrap();
    let root = migration_dir();
    let mut journal: Value = serde_json::from_slice(&std::fs::read(root.join("meta/_journal.json")).unwrap()).unwrap();
    for (idx, tag) in [(29, "0029_certificate_deploy_pending"), (30, "0030_oauth_email_verified"), (31, "0031_control_resource_versions")] {
        let entry = &journal["entries"][idx];
        assert_eq!(entry["idx"], idx);
        assert_eq!(entry["tag"], tag);
    }
    // v1.19.11's real journal ends at 0027_notification_delivery_queue.
    journal["entries"].as_array_mut().unwrap().retain(|e|e["idx"].as_u64().unwrap()<=27);
    for entry in journal["entries"].as_array().unwrap() {
        let file = format!("{}.sql",entry["tag"].as_str().unwrap());
        std::fs::copy(root.join(&file),history.path().join(file)).unwrap();
    }
    std::fs::write(history.path().join("meta/_journal.json"),serde_json::to_vec(&journal).unwrap()).unwrap();
    let db = test_db_with_migrations(&[],history.path()).await;
    db.client.batch_execute("INSERT INTO upstreams(id,name,servers) VALUES('legacy-u','legacy','[\"127.0.0.1:9000\"]'); INSERT INTO nginx_servers(id,server_name,listen,upstream_id,config) VALUES('legacy-s','legacy.test',80,'legacy-u','{}'); INSERT INTO nginx_streams(id,listen,proxy_pass) VALUES('legacy-t',19000,'127.0.0.1:9000');").await.unwrap();
    let broken = tempfile::tempdir().unwrap();
    std::fs::create_dir(broken.path().join("meta")).unwrap();
    std::fs::copy(root.join("meta/_journal.json"), broken.path().join("meta/_journal.json")).unwrap();
    let current: Value = serde_json::from_slice(&std::fs::read(root.join("meta/_journal.json")).unwrap()).unwrap();
    for entry in current["entries"].as_array().unwrap() {
        let file = format!("{}.sql",entry["tag"].as_str().unwrap());
        std::fs::copy(root.join(&file),broken.path().join(file)).unwrap();
    }
    let candidate = broken.path().join("0031_control_resource_versions.sql");
    let sql = std::fs::read_to_string(&candidate).unwrap();
    std::fs::write(candidate, format!("{sql}\nSELECT missing_migration_probe();")).unwrap();
    let failure = nwui_api::migrations::run(&db.state.db,broken.path()).await.unwrap_err();
    assert!(format!("{failure:#}").contains("missing_migration_probe"),"{failure:#}");
    let absent:Option<String> = db.client.query_one("SELECT to_regclass('public.control_resource_version_seq')::text",&[]).await.unwrap().get(0);
    assert!(absent.is_none(),"failed migration leaked its sequence");
    let columns:i64 = db.client.query_one("SELECT count(*) FROM information_schema.columns WHERE table_schema='public' AND column_name='row_version'",&[]).await.unwrap().get(0);
    assert_eq!(columns,0,"failed migration leaked a partially backfilled column");
    assert!(nwui_api::migrations::run(&db.state.db,&root).await.unwrap()>0);
    let applied: i64 = db.client.query_one("SELECT count(*) FROM drizzle.__drizzle_migrations WHERE created_at IN (1792911601000,1792911602000)", &[]).await.unwrap().get(0);
    assert_eq!(applied, 2, "both frozen 0030 and version 0031 must be applied");
    let mut versions = Vec::new();
    for (kind,id) in [(Resource::Upstream,"legacy-u"),(Resource::Server,"legacy-s"),(Resource::Stream,"legacy-t")] {
        let row = get(&db,kind,id).await;
        assert!(version(&row)>0); versions.push(version(&row));
    }
    versions.sort();versions.dedup();assert_eq!(versions.len(),3);
    assert_eq!(nwui_api::migrations::run(&db.state.db,&root).await.unwrap(),0);
    assert_eq!(db.client.query_one("SELECT name FROM upstreams WHERE id='legacy-u'",&[]).await.unwrap().get::<_,String>(0),"legacy");
    let mut same = Vec::new();
    for (kind,id) in [(Resource::Upstream,"legacy-u"),(Resource::Server,"legacy-s"),(Resource::Stream,"legacy-t")] { same.push(version(&get(&db,kind,id).await)); }
    same.sort();assert_eq!(same,versions);
}

#[tokio::test]
#[ignore = "real DB: NWUI_TEST_DATABASE_URL"]
async fn malformed_versions_fail_closed_and_missing_versions_keep_legacy_behavior() {
    let db = test_db().await;
    db.client.query_one("SELECT setval('public.control_resource_version_seq',9007199254740991)",&[]).await.unwrap();
    for (kind,body) in cases() {
        let opened = create(&db,kind,body.clone()).await;
        let id = opened["id"].as_str().unwrap();
        for invalid in [Value::Null,json!(1),json!(true),json!(""),json!("0"),json!("01"),json!("-1"),json!("1 "),json!("9223372036854775808")] {
            let mut request=body.clone();request["version"]=invalid;
            let error=resources::mutate(&db.state,kind,Mutation::Update,Some(id),request,&actor()).await.unwrap_err();
            assert_eq!(error.status,400);assert!(error.field_errors.unwrap().contains_key("version"));
            assert_eq!(version(&get(&db,kind,id).await),version(&opened));
        }
        let updated=resources::mutate(&db.state,kind,Mutation::Update,Some(id),body.clone(),&actor()).await.unwrap();
        assert!(version(&updated)>version(&opened));
        let mut request=body;request["version"]=updated["version"].clone();
        assert!(resources::mutate(&db.state,kind,Mutation::Update,Some(id),request,&actor()).await.is_ok());
    }
}

#[tokio::test]
#[ignore = "real DB: NWUI_TEST_DATABASE_URL"]
async fn update_checks_the_version_atomically_after_waiting_for_another_writer() {
    let mut db=test_db().await;
    for (kind,body) in cases() {
        let opened=create(&db,kind,body.clone()).await;
        let id=opened["id"].as_str().unwrap().to_owned();
        let state=db.state.clone();let saved_id=id.clone();let mut request=body.clone();request["version"]=opened["version"].clone();
        let tx=db.client.transaction().await.unwrap();
        tx.query_one(&format!("SELECT id FROM {} WHERE id=$1 FOR UPDATE",kind.table()),&[&id]).await.unwrap();
        let waiting=tokio::spawn(async move { resources::mutate(&state,kind,Mutation::Update,Some(&saved_id),request,&actor()).await });
        let probe=db.state.db.get().await.unwrap();
        let prefix=format!("UPDATE {} SET%",kind.table());
        tokio::time::timeout(std::time::Duration::from_secs(10),async {
            loop {
                let blocked:bool=probe.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE $1)",&[&prefix]).await.unwrap().get(0);
                if blocked {break;}
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }).await.expect("the second writer reached its guarded UPDATE");
        let (field,value)=match kind {Resource::Upstream=>("name","first-committed"),Resource::Server=>("server_name","first.test"),Resource::Stream=>("proxy_pass","127.0.0.1:9999")};
        tx.execute(&format!("UPDATE {} SET {field}=$2 WHERE id=$1",kind.table()),&[&id,&value]).await.unwrap();tx.commit().await.unwrap();drop(probe);
        assert_eq!(waiting.await.unwrap().unwrap_err().status,409);
        let saved=get(&db,kind,&id).await;
        let key=match kind {Resource::Upstream=>"name",Resource::Server=>"serverName",Resource::Stream=>"proxyPass"};
        assert_eq!(saved[key],value);assert!(version(&saved)>version(&opened));
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "real DB: NWUI_TEST_DATABASE_URL"]
async fn toggle_and_rollback_bump_revisions_without_polluting_snapshots_pending_or_render() {
    use std::os::unix::fs::PermissionsExt;
    let temp=tempfile::tempdir().unwrap();let bin=temp.path().join("nginx");std::fs::write(&bin,"#!/bin/sh\nexit 0\n").unwrap();std::fs::set_permissions(&bin,std::fs::Permissions::from_mode(0o700)).unwrap();
    let conf=temp.path().join("conf");let streams=temp.path().join("streams");let certs=temp.path().join("certs");
    let mut db=test_db_with(&[("NGINX_BIN",bin.to_str().unwrap()),("NGINX_CONF_D",conf.to_str().unwrap()),("NGINX_STREAM_D",streams.to_str().unwrap()),("CERTS_DIR",certs.to_str().unwrap())]).await;
    g13_support::witness(&mut db.state);
    let mut rows=Vec::new();for(kind,body)in cases(){rows.push((kind,create(&db,kind,body).await));}
    let applied=nwui_api::publisher_client::apply_saved(&db.state,&actor(),Some("baseline")).await.unwrap();assert!(applied.ok,"{}",applied.output);
    let baseline:String=db.client.query_one("SELECT id FROM config_snapshots",&[]).await.unwrap().get(0);
    let server_id=rows[1].1["id"].as_str().unwrap();let file=conf.join(format!("server-{server_id}.conf"));let golden=std::fs::read(&file).unwrap();
    for(kind,row)in &rows {
        let id=row["id"].as_str().unwrap();
        let old = version(row);
        db.client.execute(&format!("UPDATE {} SET row_version=$2 WHERE id=$1",kind.table()),&[&id,&old]).await.unwrap();
        assert!(version(&get(&db,*kind,id).await)>version(row));
    }
    assert_eq!(pending::status(&db.state).await.unwrap().count,0);
    for(kind,row)in &rows {
        if *kind==Resource::Upstream {continue;}
        let id=row["id"].as_str().unwrap();
        resources::mutate(&db.state,*kind,Mutation::Toggle,Some(id),Value::Null,&actor()).await.unwrap();
        assert_eq!(pending::status(&db.state).await.unwrap().count,1);
        resources::mutate(&db.state,*kind,Mutation::Toggle,Some(id),Value::Null,&actor()).await.unwrap();
        assert_eq!(pending::status(&db.state).await.unwrap().count,0);
    }
    let mut before_restore=Vec::new();for(kind,row)in &rows{before_restore.push(version(&get(&db,*kind,row["id"].as_str().unwrap()).await));}
    let restored=nwui_api::publisher_client::rollback_snapshot(&db.state,&baseline,&actor()).await.unwrap();assert!(restored.result.ok,"{}",restored.result.output);
    assert_eq!(std::fs::read(&file).unwrap(),golden);
    assert_eq!(pending::status(&db.state).await.unwrap().count,0);
    for(i,(kind,row))in rows.iter().enumerate(){assert!(version(&get(&db,*kind,row["id"].as_str().unwrap()).await)>before_restore[i]);}
    for sql in ["SELECT data FROM config_snapshots","SELECT data FROM applied_config"] {
        for r in db.client.query(sql,&[]).await.unwrap(){let data:Value=r.get(0);let text=data.to_string();assert!(!text.contains("\"version\""));assert!(!text.contains("row_version"));}
    }
    let mut with_metadata:model::ConfigSnapshotData=serde_json::from_value(db.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get::<_,Value>(0)).unwrap();
    let ordinary=g13_support::render(&db.state.config,&with_metadata,&[]).rendered;
    for row in with_metadata.upstreams.iter_mut().chain(&mut with_metadata.servers).chain(&mut with_metadata.streams){row["version"]=json!("999");row["row_version"]=json!(999);}
    let annotated=g13_support::render(&db.state.config,&with_metadata,&[]).rendered;
    assert_eq!(ordinary.conf,annotated.conf);assert_eq!(ordinary.stream,annotated.stream);
}
