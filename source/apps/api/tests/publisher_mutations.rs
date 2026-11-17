#![cfg(unix)]
mod auth_support;
use nwui_api::{publisher::{protocol::{Actor,Op,OpResult,Request},test_support},control::pending};
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
const SERVER:&str="11111111-1111-4111-8111-111111111111";
fn request(id:&str,op:Op)->Request {Request{v:1,id:id.into(),op,deadline_ms:280000,actor:Actor{via: Some(nwui_api::contracts::Via::Token{id:"token-id".into(),name:"publisher-token".into()}),user_id:None,label:Some("root-mutation-fixture".into()),ip:Some("192.0.2.1".into())}}}
fn checked(result:OpResult)->nwui_api::control::model::ApplyResult {
    match result {OpResult::Ok{apply:Some(result),..}=>result,other=>panic!("{}",serde_json::to_string(&other).unwrap())}
}

// Seed persisted historical records; no new publisher test-support API is needed.
fn aged_gc_records(state:&nwui_api::state::AppState)->Vec<(std::path::PathBuf,Vec<std::path::PathBuf>)> {
    let store=state.config.data_key_file.parent().unwrap().join(".nwui-publisher");
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(store.join("manifest.json")).unwrap()).unwrap();
    [(true,false,false),(false,true,false),(true,false,true)].into_iter().map(|(uncertain,unknown,derived)|{
        let mut ids=Vec::new();let mut paths=Vec::new();
        for _ in 0..3 {
            let id=uuid::Uuid::new_v4().to_string();let path=store.join("payloads").join(&id);nwui_api::db_upgrade::files::private_dir(&path).unwrap();
            let mut payload=json!({"version":1,"id":id,"origin":"publisher","modelDigest":null,"certificateDigests":{},"files":{}});
            if derived {payload["derivedFrom"]=manifest["current"].clone();payload["reason"]=json!("aged derivation fixture");}
            nwui_api::db_upgrade::files::atomic_json(&path.join("manifest.json"),&payload).unwrap();ids.push(id);paths.push(path);
        }
        let id=uuid::Uuid::new_v4().to_string();let path=store.join("ops").join(format!("{id}.json"));
        let record=json!({"version":1,"id":id,"commandDigest":"a".repeat(64),"phase":"done","previous":ids[0],"next":ids[1],"restoreTarget":ids[2],"restoreReason":null,"commitUnknown":unknown,"previousUncertain":true,"uncertain":uncertain,"createdAt":chrono::Utc::now()-chrono::Duration::hours(48),"result":null});
        nwui_api::db_upgrade::files::atomic_json(&path,&record).unwrap();(path,paths)
    }).collect()
}

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn durable_apply_rollback_and_exact_replay_preserve_metadata_and_full_files() {
    let root=tempfile::tempdir().unwrap();let bin=root.path().join("nginx");let main=root.path().join("nginx.conf");let conf=root.path().join("conf");let stream=root.path().join("stream");let certs=root.path().join("certs");
    std::fs::create_dir_all(&conf).unwrap();std::fs::create_dir_all(&stream).unwrap();
    std::fs::write(&main,"events {} http {}\n").unwrap();std::fs::set_permissions(&main,std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&bin,format!("#!/bin/sh\nif [ \"$1\" = -T ]; then printf '# configuration file %s:\\n' '{}'; fi\nexit 0\n",main.display())).unwrap();std::fs::set_permissions(&bin,std::fs::Permissions::from_mode(0o700)).unwrap();
    let values=[bin.display().to_string(),main.display().to_string(),conf.display().to_string(),stream.display().to_string(),certs.display().to_string()];
    let mut db=auth_support::test_db_with(&[("NGINX_BIN",&values[0]),("NGINX_CONF",&values[1]),("NGINX_CONF_D",&values[2]),("NGINX_STREAM_D",&values[3]),("CERTS_DIR",&values[4]),("API_DB_POOL_SIZE","2")]).await;
    auth_support::nginx::witness(&mut db.state);
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES($1,'durable.example.test',18081,$2)",&[&SERVER,&json!({"proxyPass":"http://127.0.0.1:19000"})]).await.unwrap();
    let first=uuid::Uuid::new_v4().to_string();
    let original=test_support::dispatch(&db.state,request(&first,Op::ApplySaved{label:Some("first durable apply".into()),expected_saved_digest:None})).await;
    let original_json=serde_json::to_value(&original).unwrap();assert!(checked(original).ok);
    assert_eq!(pending::status(&db.state).await.unwrap().count,0);
    let first_snapshot:String=db.client.query_one("SELECT id FROM config_snapshots WHERE label='first durable apply'",&[]).await.unwrap().get(0);
    let replay=test_support::dispatch(&db.state,request(&first,Op::ApplySaved{label:Some("first durable apply".into()),expected_saved_digest:None})).await;
    assert_eq!(serde_json::to_value(replay).unwrap(),original_json);
    assert_eq!(db.client.query_one("SELECT count(*) FROM audit_logs WHERE id=$1",&[&first]).await.unwrap().get::<_,i64>(0),1);
    assert_eq!(db.client.query_one("SELECT via FROM audit_logs WHERE id=$1",&[&first]).await.unwrap().get::<_,serde_json::Value>(0),json!({"kind":"token","id":"token-id","name":"publisher-token"}));
    db.client.execute("UPDATE nginx_servers SET config=$1 WHERE id=$2",&[&json!({"proxyPass":"http://127.0.0.1:19001"}),&SERVER]).await.unwrap();
    let second=uuid::Uuid::new_v4().to_string();assert!(checked(test_support::dispatch(&db.state,request(&second,Op::ApplySaved{label:Some("second durable apply".into()),expected_saved_digest:None})).await).ok);
    assert!(std::fs::read_to_string(conf.join(format!("server-{SERVER}.conf"))).unwrap().contains("127.0.0.1:19001"));
    let rollback=uuid::Uuid::new_v4().to_string();assert!(checked(test_support::dispatch(&db.state,request(&rollback,Op::RollbackSnapshot{snapshot_id:first_snapshot})).await).ok);
    assert!(std::fs::read_to_string(conf.join(format!("server-{SERVER}.conf"))).unwrap().contains("127.0.0.1:19000"));
    assert_eq!(pending::status(&db.state).await.unwrap().count,0);
    assert!(!conf.join(".nwui-apply-uncertain").exists());
    assert_eq!(db.client.query_one("SELECT count(*) FROM audit_logs WHERE action='config:rollback' AND id=$1",&[&rollback]).await.unwrap().get::<_,i64>(0),1);
    let missing=uuid::Uuid::new_v4().to_string();let response=test_support::dispatch(&db.state,request(&missing,Op::RollbackSnapshot{snapshot_id:uuid::Uuid::new_v4().to_string()})).await;
    assert!(matches!(response,OpResult::Failed{code,..} if code=="snapshot_not_found"));
    assert_eq!(db.client.query_one("SELECT count(*) FROM audit_logs WHERE id=$1",&[&missing]).await.unwrap().get::<_,i64>(0),0);
    // An unattributed validation error restores the complete previous payload and leaves drafts pending.
    db.client.execute("UPDATE nginx_servers SET config=$1 WHERE id=$2",&[&json!({"proxyPass":"http://127.0.0.1:19002"}),&SERVER]).await.unwrap();
    std::fs::write(&bin,format!("#!/bin/sh\nif [ \"$1\" = -t ] && grep -q 19002 '{}'; then echo 'nginx validation refused by fixture' >&2; exit 1; fi\nif [ \"$1\" = -T ]; then printf '# configuration file %s:\\n' '{}'; fi\nexit 0\n",conf.join(format!("server-{SERVER}.conf")).display(),main.display())).unwrap();
    let failure=uuid::Uuid::new_v4().to_string();let failed=checked(test_support::dispatch(&db.state,request(&failure,Op::ApplySaved{label:None,expected_saved_digest:None})).await);
    assert!(!failed.ok);assert!(failed.rolled_back_to.is_some());
    assert!(std::fs::read_to_string(conf.join(format!("server-{SERVER}.conf"))).unwrap().contains("127.0.0.1:19000"));
    assert_eq!(pending::status(&db.state).await.unwrap().count,1);assert!(!conf.join(".nwui-apply-uncertain").exists());
    assert_eq!(db.client.query_one("SELECT count(*) FROM audit_logs WHERE id=$1",&[&failure]).await.unwrap().get::<_,i64>(0),1);
    // A failed new publication cannot clear uncertainty which predates this operation.
    std::fs::write(conf.join(".nwui-apply-uncertain"),b"older unknown outcome").unwrap();
    let aged=aged_gc_records(&db.state);
    let uncertain=uuid::Uuid::new_v4().to_string();let failed=checked(test_support::dispatch(&db.state,request(&uncertain,Op::ApplySaved{label:None,expected_saved_digest:None})).await);
    assert!(!failed.ok);assert!(failed.rolled_back_to.is_none());assert!(conf.join(".nwui-apply-uncertain").exists());
    for (record,payloads) in &aged {assert!(record.exists());for path in payloads {assert!(path.exists());}}
    // A confirmed apply records the saved state and can resolve the inherited uncertainty.
    std::fs::write(&bin,format!("#!/bin/sh\nif [ \"$1\" = -T ]; then printf '# configuration file %s:\\n' '{}'; fi\nexit 0\n",main.display())).unwrap();
    let confirmed=uuid::Uuid::new_v4().to_string();
    assert!(checked(test_support::dispatch(&db.state,request(&confirmed,Op::ApplySaved{label:None,expected_saved_digest:None})).await).ok);
    for (record,payloads) in &aged {assert!(!record.exists(),"confirmed apply must release superseded uncertainty");for path in payloads {assert!(!path.exists(),"unprotected historical payload must expire");}}
    assert!(!conf.join(".nwui-apply-uncertain").exists());
    assert_eq!(pending::status(&db.state).await.unwrap().count,0);

}

#[cfg(target_os="linux")]
#[tokio::test]
#[ignore="real PostgreSQL in disposable Linux fixture: NWUI_TEST_DATABASE_URL"]
async fn q23_apply_and_rollback_isolate_only_unprovable_internal_socket_consumers() {
    use std::os::unix::{fs::symlink,net::UnixListener};
    // Only the disposable verifier container's task-owned child is created/removed.
    let created_parent=match std::fs::create_dir("/run/nwui-locks") {Ok(())=>true,Err(error) if error.kind()==std::io::ErrorKind::AlreadyExists=>false,Err(error)=>panic!("{error}")};
    let internal=tempfile::Builder::new().prefix("q23-n1-").tempdir_in("/run/nwui-locks").unwrap();
    let socket=internal.path().join("fdholder.sock");let _listener=UnixListener::bind(&socket).unwrap();
    for case in 0..4 {
        let root=tempfile::tempdir().unwrap();let bin=root.path().join("nginx");let main=root.path().join("nginx.conf");let conf=root.path().join("conf");let stream=root.path().join("stream");let certs=root.path().join("certs");
        std::fs::create_dir_all(&conf).unwrap();std::fs::create_dir_all(&stream).unwrap();
        std::fs::write(&main,"events {} http {}\n").unwrap();std::fs::set_permissions(&main,std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&bin,format!("#!/bin/sh\nif [ \"$1\" = -T ]; then printf '# configuration file %s:\\n' '{}'; fi\nexit 0\n",main.display())).unwrap();std::fs::set_permissions(&bin,std::fs::Permissions::from_mode(0o700)).unwrap();
        let values=[bin.display().to_string(),main.display().to_string(),conf.display().to_string(),stream.display().to_string(),certs.display().to_string()];
        let mut db=auth_support::test_db_with(&[("NGINX_BIN",&values[0]),("NGINX_CONF",&values[1]),("NGINX_CONF_D",&values[2]),("NGINX_STREAM_D",&values[3]),("CERTS_DIR",&values[4]),("API_DB_POOL_SIZE","2")]).await;
        auth_support::nginx::witness(&mut db.state);
        let alias=root.path().join("mounted.sock");symlink(&socket,&alias).unwrap();
        let looped=root.path().join("loop.sock");symlink("loop.sock",&looped).unwrap();
        let blocked=match case {0=>format!("http://unix:{}:/api/",socket.display()),1=>format!("http://unix:{}:/api/",alias.display()),2=>"http://unix:/tmp/$socket:/api/".into(),_=>format!("http://unix:{}:/api/",looped.display())};
        let bad=uuid::Uuid::new_v4().to_string();let tcp=uuid::Uuid::new_v4().to_string();let public=uuid::Uuid::new_v4().to_string();
        let user_target=format!("http://unix:/tmp/q23-{}.sock:$request_uri",uuid::Uuid::new_v4());
        for (id,name,port,target) in [(&bad,"blocked.example.test",18081,blocked.as_str()),(&tcp,"tcp.example.test",18082,"http://192.0.2.44:9000"),(&public,"socket.example.test",18083,user_target.as_str())] {
            db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES($1,$2,$3,$4)",&[id,&name,&port,&json!({"proxyPass":target})]).await.unwrap();
        }
        let label=format!("q23 unix isolation {case}");let apply_id=uuid::Uuid::new_v4().to_string();
        let applied=checked(test_support::dispatch(&db.state,request(&apply_id,Op::ApplySaved{label:Some(label.clone()),expected_saved_digest:None})).await);
        assert!(applied.ok,"{}",applied.output);assert!(applied.output.contains("unix"),"{}",applied.output);
        assert!(!conf.join(format!("server-{bad}.conf")).exists());assert!(conf.join(format!("server-{tcp}.conf")).exists());
        let user_bytes=std::fs::read_to_string(conf.join(format!("server-{public}.conf"))).unwrap();assert!(user_bytes.contains(&user_target));assert!(!user_bytes.contains("proxy_bind "));
        let snapshot:String=db.client.query_one("SELECT id FROM config_snapshots WHERE label=$1",&[&label]).await.unwrap().get(0);
        // Rollback reads the saved unsafe Unix target again and isolates the same whole file.
        db.client.execute("UPDATE nginx_servers SET config=$1 WHERE id=$2",&[&json!({"proxyPass":"http://192.0.2.45:9000"}),&bad]).await.unwrap();
        let rollback_id=uuid::Uuid::new_v4().to_string();
        let restored=checked(test_support::dispatch(&db.state,request(&rollback_id,Op::RollbackSnapshot{snapshot_id:snapshot})).await);
        assert!(restored.ok,"{}",restored.output);assert!(restored.output.contains("unix"),"{}",restored.output);
        assert!(!conf.join(format!("server-{bad}.conf")).exists());assert!(conf.join(format!("server-{tcp}.conf")).exists());assert_eq!(std::fs::read_to_string(conf.join(format!("server-{public}.conf"))).unwrap(),user_bytes);
        assert!(!conf.join(".nwui-apply-uncertain").exists());
        let applied:serde_json::Value=db.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get(0);
        let servers=applied["servers"].as_array().unwrap();assert_eq!(servers.len(),2);assert!(!servers.iter().any(|row|row["id"]==bad));
        assert!(applied["skipped"].as_array().unwrap().iter().any(|row|row["id"]==bad&&row["reason"].as_str().unwrap().contains("unix")&&row["kept_previous"]==false));
        for id in [&apply_id,&rollback_id] {
            let rows=db.client.query("SELECT \"after\" FROM audit_logs WHERE id=$1",&[id]).await.unwrap();assert_eq!(rows.len(),1);let entry:serde_json::Value=rows[0].get(0);assert_eq!(entry["ok"],true);assert!(entry["output"].as_str().unwrap().contains("unix"));
        }
    }
    drop(_listener);drop(internal);
    if created_parent {std::fs::remove_dir("/run/nwui-locks").unwrap();}

}
