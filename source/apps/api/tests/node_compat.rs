#[path = "auth_support/mod.rs"]
mod auth_support;

use axum::{Json, Router, http::{HeaderMap, StatusCode}, routing::{get, post}};
use nwui_api::{contracts::{Actor, Role}, nodes::{client::NodeResult, service}};
use serde_json::json;

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn legacy_pair_negotiates_once_and_remembers_version_for_refresh() {
    let remote = Router::new()
        .route("/api/v1/pair", post(|headers: HeaderMap| async move {
            if headers["x-gateway-version"] != "1.19.11" {
                return (StatusCode::CONFLICT, [("x-node-version", "1.19.11")], Json(json!({"error":{"code":"version_mismatch"}})));
            }
            (StatusCode::OK, [("x-node-version", "1.19.11")], Json(json!({"token":"legacy-issued","role":"editor","instanceId":"legacy-instance","semver":"1.19.11"})))
        }))
        .route("/api/v1/info", get(|headers: HeaderMap| async move {
            assert_eq!(headers["x-gateway-version"], "1.19.11");
            assert_eq!(headers["authorization"], "Bearer legacy-issued");
            ([("x-node-version", "1.19.11")], Json(json!({"instanceId":"legacy-instance","semver":"1.19.11"})))
        }));
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url=format!("http://{}",listener.local_addr().unwrap());
    let server=tokio::spawn(async move { axum::serve(listener,remote).await.unwrap() });
    let db=auth_support::test_db().await;
    let id=service::pair_node(&db.state, service::NodeInput {
        name:"legacy".into(),base_url,allow_insecure_tls:false,
        pair:Some(service::PairInput {token:"server-token".into(),gateway_name:"new-gateway".into(),role:Role::Editor}),
    }, &Actor {id:None,label:Some("test".into()),ip:None}).await.unwrap().unwrap();
    assert_eq!(service::get(&db.state,&id).await.unwrap().last_version.as_deref(),Some("1.19.11"));
    service::refresh_statuses(&db.state).await.unwrap();
    assert_eq!(service::get(&db.state,&id).await.unwrap().last_status.as_deref(),Some("ok"));
    server.abort();
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn info_refresh_records_the_remote_version() {
    let db=auth_support::test_db().await;
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id) VALUES('legacy','legacy','http://node.test','unused','editor','legacy-instance')",&[]).await.unwrap();
    service::record_probe(&db.state,"legacy",&NodeResult::Success {ok:true,value:json!({"instanceId":"legacy-instance","semver":"1.19.11"}),node_version:Some("1.19.11".into())}).await.unwrap();
    assert_eq!(service::get(&db.state,"legacy").await.unwrap().last_version.as_deref(),Some("1.19.11"));
    service::record_probe(&db.state,"legacy",&NodeResult::failure(nwui_api::nodes::client::FailureKind::VersionMismatch,"mismatch",Some("2.0.1".into()))).await.unwrap();
    assert_eq!(service::get(&db.state,"legacy").await.unwrap().last_version.as_deref(),Some("2.0.1"));
}

async fn register(db: &auth_support::TestDb, instance: &str, proof_id: Option<&str>, version: &str) {
    let sealed=db.state.secrets.seal("paired-secret").unwrap();
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id,gateway_token_id,last_version) VALUES('n','node','http://127.0.0.1:9',$1,'editor',$2,$3,$4)", &[&sealed,&instance,&proof_id,&version]).await.unwrap();
}
fn actor() -> Actor { Actor {id:None,label:Some("test".into()),ip:None} }
fn update(base_url: String) -> service::NodeInput {
    service::NodeInput {name:"node".into(),base_url,allow_insecure_tls:false,pair:None}
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn address_change_proves_real_node_possession_before_authenticated_refresh() {
    let remote=auth_support::test_db_with(&[("API_DB_POOL_SIZE","1")]).await;
    let instance=service::ensure_instance_id(&remote.state).await.unwrap();
    auth_support::insert_user(&remote.client,auth_support::UserRow {role:"admin",..Default::default()},None).await;
    remote.client.execute("INSERT INTO gateway_tokens(id,name,token_hash,role,created_by_user_id) VALUES('proof-id','gateway',$1,'editor','u1')", &[&nwui_api::auth::token_digest("paired-secret")]).await.unwrap();
    let routes=nwui_api::app::with_request_context(nwui_api::nodes::routes::router().with_state(remote.state.clone()),None);
    let routes=nwui_api::app::finish_router(routes,&remote.state.config.version);
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base=format!("http://{}",listener.local_addr().unwrap());
    let server=tokio::spawn(async move {axum::serve(listener,routes.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap()});
    let gateway=auth_support::test_db_with(&[("API_DB_POOL_SIZE","1")]).await;
    register(&gateway,&instance,Some("proof-id"),"2.0.0").await;
    service::update_node(&gateway.state,"n",update(base.clone()),&actor()).await.unwrap();
    assert_eq!(service::get(&gateway.state,"n").await.unwrap().base_url,base);
    assert!(remote.client.query_one("SELECT last_used_at IS NULL FROM gateway_tokens WHERE id='proof-id'",&[]).await.unwrap().get::<_,bool>(0),"the address proof never sends a bearer token");
    service::refresh_statuses(&gateway.state).await.unwrap();
    assert_eq!(service::get(&gateway.state,"n").await.unwrap().last_status.as_deref(),Some("ok"));
    server.abort();
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn copied_instance_id_is_not_proof_and_legacy_nodes_cannot_change_address() {
    use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
    let hits=Arc::new(AtomicUsize::new(0));
    let seen=hits.clone();
    let routes=Router::new().route("/api/v1/identity-challenge",post(move |headers:HeaderMap,Json(body):Json<serde_json::Value>| {
        assert!(!headers.contains_key("authorization"));
        seen.fetch_add(1,Ordering::SeqCst);
        async move {
            use base64::Engine as _;
            let nonce: [u8;32]=base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(body["nonce"].as_str().unwrap()).unwrap().try_into().unwrap();
            Json(json!({"instanceId":"copied-id","proof":nwui_api::nodes::identity::proof(&nwui_api::auth::token_digest("wrong-secret"),&nonce,"copied-id")}))
        }
    }));
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base=format!("http://{}",listener.local_addr().unwrap());
    let server=tokio::spawn(async move {axum::serve(listener,routes).await.unwrap()});
    let db=auth_support::test_db().await;
    register(&db,"copied-id",Some("proof-id"),"2.0.0").await;
    let impostor=service::update_node(&db.state,"n",update(base.clone()),&actor()).await.unwrap_err();
    assert_eq!(hits.load(Ordering::SeqCst),1);
    assert_eq!(service::get(&db.state,"n").await.unwrap().base_url,"http://127.0.0.1:9");
    db.client.execute("UPDATE nodes SET last_version='1.19.11' WHERE id='n'",&[]).await.unwrap();
    let legacy=service::update_node(&db.state,"n",update(base),&actor()).await.unwrap_err();
    assert_eq!(legacy.message,impostor.message);
    assert_eq!(hits.load(Ordering::SeqCst),1,"legacy nodes are refused before any request");
    server.abort();
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn status_refresh_backfills_old_pairing_proof_metadata() {
    let routes=Router::new().route("/api/v1/info",get(|headers:HeaderMap| async move {
        assert_eq!(headers["authorization"],"Bearer paired-secret");
        ([("x-node-version","2.0.0")],Json(json!({"instanceId":"stable-id","gatewayTokenId":"proof-id","semver":"2.0.0"})))
    }));
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base=format!("http://{}",listener.local_addr().unwrap());
    let server=tokio::spawn(async move {axum::serve(listener,routes).await.unwrap()});
    let db=auth_support::test_db().await;
    register(&db,"",None,"2.0.0").await;
    let error=service::update_node(&db.state,"n",update(base.clone()),&actor()).await.unwrap_err();
    assert!(error.message.contains("상태를 새로고침"));
    db.client.execute("UPDATE nodes SET base_url=$1 WHERE id='n'",&[&base]).await.unwrap();
    service::refresh_statuses(&db.state).await.unwrap();
    let row=db.client.query_one("SELECT instance_id,gateway_token_id FROM nodes WHERE id='n'",&[]).await.unwrap();
    assert_eq!(row.get::<_,String>(0),"stable-id");
    assert_eq!(row.get::<_,String>(1),"proof-id");
    server.abort();
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn challenge_unknown_revoked_and_malformed_share_errors_and_requests_are_limited() {
    use axum::{body::Body,extract::ConnectInfo,http::Request};
    use tower::ServiceExt;
    let db=auth_support::test_db().await;
    db.client.execute("INSERT INTO gateway_tokens(id,name,token_hash,role,revoked_at) VALUES('revoked','gateway','digest','editor',now())",&[]).await.unwrap();
    let app=nwui_api::app::with_request_context(nwui_api::nodes::routes::router().with_state(db.state.clone()),None);
    let request=|id:&str,nonce:&str| Request::builder().method("POST").uri("/api/v1/identity-challenge").header("host","admin.test")
        .extension(ConnectInfo(std::net::SocketAddr::from(([127,0,0,1],40000))))
        .body(Body::from(json!({"gatewayTokenId":id,"nonce":nonce}).to_string())).unwrap();
    use base64::Engine as _;
    let nonce=base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([5;32]);
    let mut errors=vec![];
    for (id,value) in [("missing",nonce.as_str()),("revoked",nonce.as_str()),("missing","bad")] {
        let response=app.clone().oneshot(request(id,value)).await.unwrap();
        assert_eq!(response.status(),StatusCode::BAD_REQUEST);
        errors.push(axum::body::to_bytes(response.into_body(),8192).await.unwrap());
    }
    assert_eq!(errors[0],errors[1]);assert_eq!(errors[0],errors[2]);
    for _ in 3..30 {assert_eq!(app.clone().oneshot(request("missing",&nonce)).await.unwrap().status(),StatusCode::BAD_REQUEST);}
    assert_eq!(app.oneshot(request("missing",&nonce)).await.unwrap().status(),StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn server_reads_project_upstream_urls_only_for_legacy_gateway_headers() {
    use axum::{body::Body,extract::ConnectInfo,http::Request};
    use tower::ServiceExt;
    let db=auth_support::test_db().await;
    auth_support::insert_user(&db.client,auth_support::UserRow {role:"admin",..Default::default()},None).await;
    db.client.execute("INSERT INTO gateway_tokens(id,name,token_hash,role,created_by_user_id) VALUES('gateway','gateway',$1,'editor','u1')",&[&nwui_api::auth::token_digest("read-secret")]).await.unwrap();
    db.client.execute("INSERT INTO upstreams(id,name,servers) VALUES('up','backend',$1)",&[&json!(["192.0.2.10:9000"])]).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('s','compat.example.test',18440,$1)",&[&json!({"locations":[{"path":"/","upstreamId":"up"}]})]).await.unwrap();
    let app=nwui_api::app::with_request_context(nwui_api::control::routes::router().with_state(db.state.clone()),None);
    for version in ["1.19.11","2.0.0"] {
        for path in ["/api/v1/servers","/api/v1/servers/s"] {
            let response=app.clone().oneshot(Request::builder().uri(path).header("host","admin.test").header("x-gateway-version",version).header("authorization","Bearer read-secret")
                .extension(ConnectInfo(std::net::SocketAddr::from(([127,0,0,1],40000)))).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(),StatusCode::OK);
            let body:serde_json::Value=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),65536).await.unwrap()).unwrap();
            let row=if path.ends_with("/s"){&body["server"]}else{&body["servers"][0]};
            let location=&row["config"]["locations"][0];
            if version=="1.19.11" {
                assert_eq!(location["proxyPass"],"http://backend");
                assert!(location.get("upstreamId").is_none());
                assert!(location.get("upstreamName").is_none());
            } else {
                assert_eq!(location["upstreamId"],"up");
                assert_eq!(location["upstreamName"],"backend");
                assert!(location.get("proxyPass").is_none());
            }
        }
    }
}
