#![cfg(unix)]
mod auth_support;
use nwui_api::{contracts::Actor,control::{model,pending},publisher::{protocol::{self,Op,OpResult,Request,Response},test_support},publisher_client::{self,Client,LiveValidation}};
use serde_json::json;
use std::{os::unix::fs::PermissionsExt,path::Path,sync::Arc};
use tokio::io::{AsyncReadExt,AsyncWriteExt};

async fn status(path:&Path,id:&str)->OpResult {
    let request=Request{v:1,id:uuid::Uuid::new_v4().to_string(),op:Op::OperationStatus{operation_id:id.into()},deadline_ms:15000,actor:protocol::Actor{via: None,user_id:None,label:None,ip:None}};
    let mut stream=tokio::net::UnixStream::connect(path).await.unwrap();let bytes=serde_json::to_vec(&request).unwrap();
    stream.write_u32(bytes.len() as u32).await.unwrap();stream.write_all(&bytes).await.unwrap();stream.shutdown().await.unwrap();
    let length=stream.read_u32().await.unwrap();assert!((1..=1_048_576).contains(&length));let mut body=vec![0;length as usize];stream.read_exact(&mut body).await.unwrap();
    let response:Response=serde_json::from_slice(&body).unwrap();assert_eq!(response.id,request.id);assert_eq!(response.epoch,29);response.result
}
fn pair()->(String,String) {
    let material=rcgen::generate_simple_self_signed(vec!["socket.example.test".into()]).unwrap();(material.cert.pem(),material.signing_key.serialize_pem())
}

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn client_real_socket_mutations_and_journal_status_preserve_live_state() {
    let root=tempfile::tempdir().unwrap();let bin=root.path().join("nginx");let main=root.path().join("nginx.conf");let conf=root.path().join("conf");let stream=root.path().join("stream");let certs=root.path().join("certs");
    std::fs::create_dir_all(&conf).unwrap();std::fs::create_dir_all(&stream).unwrap();std::fs::write(&main,"events {} http {}\n").unwrap();std::fs::set_permissions(&main,std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&bin,format!("#!/bin/sh\nif [ \"$1\" = -T ]; then printf '# configuration file %s:\\n' '{}'; fi\nexit 0\n",main.display())).unwrap();std::fs::set_permissions(&bin,std::fs::Permissions::from_mode(0o700)).unwrap();
    let paths=[bin.display().to_string(),main.display().to_string(),conf.display().to_string(),stream.display().to_string(),certs.display().to_string()];
    let mut db=auth_support::test_db_with(&[("NGINX_BIN",&paths[0]),("NGINX_CONF",&paths[1]),("NGINX_CONF_D",&paths[2]),("NGINX_STREAM_D",&paths[3]),("CERTS_DIR",&paths[4]),("API_DB_POOL_SIZE","2")]).await;
    let directory=root.path().join("publisher-socket");let socket=directory.join("control.sock");
    Arc::get_mut(&mut db.state.runtime).unwrap().publisher=Client::socket_for_tests(socket.clone(),Default::default());
    auth_support::nginx::witness(&mut db.state);
    // The fixture server owns a separate runtime/apply_lock, matching the real API and root publisher processes.
    let _server=test_support::serve(&db.state,&directory,29).unwrap();
    let hello=publisher_client::hello(&db.state).await.unwrap();assert!(hello.ready);assert!(!hello.uncertain);assert_eq!(hello.epoch,29);
    assert!(matches!(publisher_client::validate_live(&db.state).await.unwrap(),LiveValidation::Checked{ok:true,..}));
    auth_support::insert_user(&db.client,auth_support::UserRow::default(),None).await;
    let actor=Actor{via: Some(nwui_api::contracts::Via::Token{id:"socket-token".into(),name:"socket-job".into()}),id:Some("u1".into()),label:Some("real-socket-interop".into()),ip:Some("192.0.2.29".into())};
    let cert=uuid::Uuid::new_v4().to_string();let server=uuid::Uuid::new_v4().to_string();let(old_cert,old_key)=pair();let sealed=db.state.secrets.seal(&old_key).unwrap();
    db.client.execute("INSERT INTO certificates(id,domains,status,source,cert_pem,key_pem) VALUES($1,$2,'issued','manual',$3,$4)",&[&cert,&json!(["socket.example.test"]),&old_cert,&sealed]).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES($1,'socket.example.test',18081,$2,$3)",&[&server,&cert,&json!({"proxyPass":"http://127.0.0.1:19000","tlsPort":18443})]).await.unwrap();
    let applied=publisher_client::apply_saved(&db.state,&actor,Some("socket first")).await.unwrap();assert!(applied.ok,"{}",applied.output);
    let snapshot:String=db.client.query_one("SELECT id FROM config_snapshots WHERE label='socket first'",&[]).await.unwrap().get(0);
    let apply_id:String=db.client.query_one("SELECT id FROM audit_logs WHERE action='nginx:apply' AND actor_label='real-socket-interop'",&[]).await.unwrap().get(0);
    for _ in 0..2 {
        let OpResult::Ok{apply:Some(replayed),..}=status(&socket,&apply_id).await else{panic!("stored final apply result")};
        assert_eq!(serde_json::to_value(replayed).unwrap(),serde_json::to_value(&applied).unwrap());
    }
    assert_eq!(db.client.query_one("SELECT count(*) FROM audit_logs WHERE id=$1",&[&apply_id]).await.unwrap().get::<_,i64>(0),1);
    assert!(matches!(status(&socket,&uuid::Uuid::new_v4().to_string()).await,OpResult::Unknown));
    db.client.execute("UPDATE nginx_servers SET config=$1 WHERE id=$2",&[&json!({"proxyPass":"http://127.0.0.1:19001","tlsPort":18443}),&server]).await.unwrap();
    assert!(publisher_client::apply_saved(&db.state,&actor,Some("socket second")).await.unwrap().ok);
    let rolled=publisher_client::rollback_snapshot(&db.state,&snapshot,&actor).await.unwrap();assert!(rolled.result.ok,"{}",rolled.result.output);
    let config=std::fs::read(conf.join(format!("server-{server}.conf"))).unwrap();assert!(String::from_utf8_lossy(&config).contains("127.0.0.1:19000"));
    let snapshots:i64=db.client.query_one("SELECT count(*) FROM config_snapshots",&[]).await.unwrap().get(0);
    let applied_state:serde_json::Value=db.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get(0);
    let(new_cert,new_key)=pair();let sealed=db.state.secrets.seal(&new_key).unwrap();let digest=model::certificate_content_digest(&new_cert,&sealed);
    db.client.execute("UPDATE certificates SET cert_pem=$1,key_pem=$2 WHERE id=$3",&[&new_cert,&sealed,&cert]).await.unwrap();
    let deployed=publisher_client::deploy_certificate(&db.state,&cert,&digest,&actor).await.unwrap();assert!(deployed.ok,"{}",deployed.output);
    assert_eq!(std::fs::read(conf.join(format!("server-{server}.conf"))).unwrap(),config);
    assert_eq!(std::fs::read_to_string(certs.join(&cert).join("fullchain.pem")).unwrap(),new_cert);
    assert_eq!(std::fs::read_to_string(certs.join(&cert).join("privkey.pem")).unwrap(),new_key);
    assert_eq!(db.client.query_one("SELECT count(*) FROM config_snapshots",&[]).await.unwrap().get::<_,i64>(0),snapshots);
    assert_eq!(db.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get::<_,serde_json::Value>(0),applied_state);
    let rows=db.client.query("SELECT via,actor_id FROM audit_logs WHERE actor_label='real-socket-interop'",&[]).await.unwrap();
    assert!(rows.len()>=4);
    for row in rows { assert_eq!(row.get::<_,serde_json::Value>(0),json!({"kind":"token","id":"socket-token","name":"socket-job"})); assert_eq!(row.get::<_,String>(1),"u1"); }
    assert_eq!(pending::status(&db.state).await.unwrap().count,0);assert!(!conf.join(".nwui-apply-uncertain").exists());
    assert_eq!(db.client.query_one("SELECT count(*) FROM audit_logs WHERE action='nginx:apply' AND target=$1",&[&cert]).await.unwrap().get::<_,i64>(0),1);
}
