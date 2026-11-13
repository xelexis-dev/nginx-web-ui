mod auth_support;
mod g13_support;
use auth_support::*;
use nwui_api::{contracts::Actor,control::{model,resources::{self,Mutation,Resource}}};
use serde_json::json;

#[cfg(unix)]
#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn fxb_rollback_refuses_all_missing_or_nonissued_certificates_before_any_mutation() {
    use std::os::unix::fs::PermissionsExt;
    let temp=tempfile::tempdir().unwrap();let bin=temp.path().join("nginx");std::fs::write(&bin,"#!/bin/sh\nexit 0\n").unwrap();std::fs::set_permissions(&bin,std::fs::Permissions::from_mode(0o700)).unwrap();
    let conf=temp.path().join("conf");let streams=temp.path().join("streams");let certs=temp.path().join("certs");
    let mut db=test_db_with(&[("NGINX_BIN",bin.to_str().unwrap()),("NGINX_CONF_D",conf.to_str().unwrap()),("NGINX_STREAM_D",streams.to_str().unwrap()),("CERTS_DIR",certs.to_str().unwrap())]).await;
    g13_support::witness(&mut db.state);let actor=Actor{id:None,label:Some("fxb".into()),ip:None};
    let live=resources::mutate(&db.state,Resource::Server,Mutation::Create,None,json!({"serverName":"live.test","listen":18087,"proxyPass":"http://127.0.0.1:9000"}),&actor).await.unwrap();
    let baseline=nwui_api::publisher_client::apply_saved(&db.state,&actor,Some("current")).await.unwrap();assert!(baseline.ok,"{}",baseline.output);
    db.client.batch_execute("INSERT INTO certificates(id,domains,status) VALUES('failed-cert','[\"failed.test\"]','failed')").await.unwrap();
    let rows_query="SELECT coalesce(jsonb_agg(to_jsonb(s) ORDER BY id),'[]') FROM nginx_servers s";let before:serde_json::Value=db.client.query_one(rows_query,&[]).await.unwrap().get(0);let file=conf.join(format!("server-{}.conf",live["id"].as_str().unwrap()));let bytes=std::fs::read(&file).unwrap();let generation=std::fs::read(conf.join(".nwui-generation")).unwrap();let version:i64=db.client.query_one("SELECT row_version FROM nginx_servers WHERE id=$1",&[&live["id"].as_str().unwrap()]).await.unwrap().get(0);
    let marker:serde_json::Value=db.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get(0);
    let data=json!({"upstreams":[],"streams":[],"servers":[{"id":"missing","serverName":"missing.test","listen":18088,"enabled":true,"certificateId":"missing-cert","config":{"proxyPass":"http://127.0.0.1:9000","tlsPort":18443,"redirectHttp":true}},{"id":"failed","serverName":"failed.test","listen":18089,"enabled":true,"certificateId":"failed-cert","config":{"proxyPass":"http://127.0.0.1:9000","tlsPort":18444}}]});
    db.client.execute("INSERT INTO config_snapshots(id,data) VALUES('77777777-7777-4777-8777-777777777777',$1)",&[&data]).await.unwrap();let count:i64=db.client.query_one("SELECT count(*) FROM config_snapshots",&[]).await.unwrap().get(0);
    let result=nwui_api::publisher_client::rollback_snapshot(&db.state,"77777777-7777-4777-8777-777777777777",&actor).await.unwrap();
    assert!(!result.result.ok,"rollback silently accepted unavailable TLS and downgraded it: {}",result.result.output);
    for label in ["missing-cert","failed-cert","missing.test","failed.test"]{assert!(result.result.output.contains(label),"missing label {label}: {}",result.result.output);}
    assert_eq!(db.client.query_one(rows_query,&[]).await.unwrap().get::<_,serde_json::Value>(0),before);assert_eq!(std::fs::read(file).unwrap(),bytes);assert_eq!(std::fs::read(conf.join(".nwui-generation")).unwrap(),generation);assert_eq!(db.client.query_one("SELECT row_version FROM nginx_servers WHERE id=$1",&[&live["id"].as_str().unwrap()]).await.unwrap().get::<_,i64>(0),version);
    assert_eq!(db.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get::<_,serde_json::Value>(0),marker);
    assert_eq!(db.client.query_one("SELECT count(*) FROM config_snapshots",&[]).await.unwrap().get::<_,i64>(0),count);
}

#[test]
#[ignore="requires the disposable CI container's routed network namespace"]
fn fxb_w3_nonloopback_interface_address_is_not_an_external_peer() {
    use std::net::UdpSocket;
    let socket=UdpSocket::bind("0.0.0.0:0").unwrap();
    // UDP connect only selects the local route; no datagram is sent to this documentation address.
    socket.connect("192.0.2.1:9").unwrap();let ip=socket.local_addr().unwrap().ip();assert!(!ip.is_loopback());
    let config=nwui_api::config::RuntimeConfig::read(|key|match key {"DATABASE_URL"=>Some("postgres://test@127.0.0.1/db".into()),"AUTH_SECRET"=>Some("test-only".into()),"NWUI_CONTAINER"=>Some("1".into()),_=>None}).unwrap();
    let row=json!({"listen":18086,"config":{"proxyPass":format!("http://{ip}:18086")}});
    assert!(nwui_api::control::internal_targets::check_built(&config,Resource::Server,&row).is_err(),"own interface {ip} loops too");
}

#[test]
#[ignore="requires the disposable CI container's routed network namespace"]
fn fxb3_own_interface_admin_routes_get_the_protected_source() {
    use std::net::UdpSocket;
    let socket=UdpSocket::bind("0.0.0.0:0").unwrap();socket.connect("192.0.2.1:9").unwrap();let ip=socket.local_addr().unwrap().ip();assert!(!ip.is_loopback());
    let config=nwui_api::config::RuntimeConfig::read(|key|match key {"DATABASE_URL"=>Some("postgres://test@127.0.0.1/db".into()),"AUTH_SECRET"=>Some("test-only".into()),"NWUI_CONTAINER"=>Some("1".into()),_=>None}).unwrap();
    let data=model::ConfigSnapshotData{upstreams:vec![json!({"id":"own","name":"own_admin","servers":[format!("{ip}:8080")]})],servers:vec![json!({"id":"direct-own","serverName":"direct.test","listen":18086,"enabled":true,"config":{"proxyPass":format!("http://{ip}:8080")}}),json!({"id":"group-own","serverName":"group.test","listen":18087,"enabled":true,"upstreamId":"own","config":{}})],..Default::default()};
    let outcome=g13_support::render(&config,&data,&[]);assert!(outcome.skipped.is_empty(),"{:?}",outcome.skipped);
    for name in ["server-direct-own.conf","server-group-own.conf"] {let text=String::from_utf8_lossy(&outcome.rendered.conf[name]);assert!(text.contains("proxy_bind 127.0.0.2;"),"{name}: {text}");assert!(text.contains("proxy_set_header X-Real-IP $remote_addr;"));}
}

#[tokio::test]
#[ignore="native nginx harness only: NWUI_FXB3_CONTEXT_PROBE=1"]
async fn fxb3_native_context_probe() {
    if std::env::var("NWUI_FXB3_CONTEXT_PROBE").as_deref()!=Ok("1"){return;}
    use axum::{extract::ConnectInfo,http::HeaderMap,routing::get,Json,Router};use std::net::SocketAddr;
    async fn probe(ConnectInfo(peer):ConnectInfo<SocketAddr>,headers:HeaderMap,uri:axum::http::Uri)->(axum::http::StatusCode,Json<serde_json::Value>){
        let context=nwui_api::request_context::RequestContext::resolve(&headers,peer,None).unwrap();
        let status=if uri.query().is_some_and(|query|query.contains("fail=1")){axum::http::StatusCode::FORBIDDEN}else{axum::http::StatusCode::OK};
        (status,Json(json!({"host":headers.get("host").and_then(|v|v.to_str().ok()),"original":headers.get("x-original").and_then(|v|v.to_str().ok()),"uri":uri.to_string(),"ip":context.ip,"proto":if context.secure{"https"}else{"http"},"origin":context.external_origin,"secure":context.secure,"originConfigured":context.origin_configured,"cookieName":nwui_api::auth::session::cookie_name(context.secure)})))
    }
    let listener=tokio::net::TcpListener::bind("127.0.0.1:3001").await.unwrap();
    axum::serve(listener,Router::new().route("/api/probe",get(probe)).into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
}
