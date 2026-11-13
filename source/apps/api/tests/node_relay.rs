mod outbound_support;
use nwui_api::{contracts::FormInput, nodes::relay};
use serde_json::{Value, json};

fn product_version()->String {serde_json::from_str::<serde_json::Value>(include_str!("../../web/package.json")).unwrap()["version"].as_str().unwrap().to_owned()}

#[test]
fn command_registry_encodes_ids_keeps_secrets_out_of_audit_and_rejects_arbitrary_targets() {
    let mut form = FormInput::default();
    form.fields.insert("id".into(), vec!["a/b?x=1".into()]);
    form.fields
        .insert("certPem".into(), vec!["private cert".into()]);
    form.fields
        .insert("keyPem".into(), vec!["private key".into()]);
    let cmd = relay::command("node.replaceNodeCertificate", "node-1", &form).unwrap();
    assert_eq!(cmd.path, "/api/v1/certificates/a%2Fb%3Fx%3D1/replace");
    assert!(!cmd.detail.to_string().contains("private"));
    assert_eq!(cmd.body.unwrap()["keyPem"], "private key");
    assert!(relay::command("node.proxy", "node-1", &form).is_err());
    form.fields.insert("id".into(), vec!["..".into()]);
    assert!(relay::command("node.deleteNodeServer", "n", &form).is_err());
    form.args = vec!["node-1".into(), "https://attacker.test".into()];
    assert!(relay::command("node.applyNodeConfig", "node-1", &form).is_err());
    form.args[1] = "/nodes/node-1/stream".into();
    assert!(relay::command("node.applyNodeConfig", "node-1", &form).is_ok());
}

#[path = "auth_support/mod.rs"]
mod auth_support;
use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, State},
    http::{Request, StatusCode},
    routing::post,
};
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tower::ServiceExt;
fn cookie() -> String {
    let now = nwui_api::auth::principal::now_unix_ms() / 1000;
    let claims = json!({"id":"u1","sub":"u1","role":"admin","sv":3,"requires2fa":false});
    let token = nwui_api::auth::session::encode(
        claims.as_object().unwrap(),
        auth_support::SECRET,
        nwui_api::auth::session::COOKIE_NAME_PLAIN,
        now,
        3600,
    )
    .unwrap();
    format!("authjs.session-token={token}")
}
fn form_request(command: &str, fields: &[(&str, &str)]) -> Request<Body> {
    let mut body = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in fields {
        body.append_pair(k, v);
    }
    Request::builder()
        .method("POST")
        .uri(format!("/api/ui/v1/forms/{command}"))
        .header("host", "admin.example.test")
        .header("origin", "http://admin.example.test")
        .header("cookie", cookie())
        .header("content-type", "application/x-www-form-urlencoded")
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))))
        .body(Body::from(body.finish()))
        .unwrap()
}
async fn body(response: axum::response::Response) -> Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn remote_write_uses_fresh_min_role_and_preserves_success_probe_on_malformed_payload() {
    let count = Arc::new(AtomicUsize::new(0));
    let upstream = Router::new()
        .route(
            "/api/v1/upstreams",
            post(|State(count): State<Arc<AtomicUsize>>| async move {
                count.fetch_add(1, Ordering::SeqCst);
                (
                    [("x-node-version", "1.17.0")],
                    Json(json!({"upstream":{"id":{}}})),
                )
            }),
        )
        .with_state(count.clone());
    let listener = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let db = auth_support::test_db().await;
    auth_support::insert_user(
        &db.client,
        auth_support::UserRow {
            role: "admin",
            ..Default::default()
        },
        None,
    )
    .await;
    let sealed = db.state.secrets.seal("remote-token").unwrap();
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id) VALUES('n','remote',$1,$2,'viewer','remote-instance')",&[&format!("http://{addr}"),&sealed]).await.unwrap();
    db.client.execute("UPDATE nodes SET last_version='1.17.0' WHERE id='n'",&[]).await.unwrap();
    let app =
        nwui_api::app::with_request_context(relay::router().with_state(db.state.clone()), None);
    let fields = [
        ("_apiArgs", r#"["n"]"#),
        ("name", "pool"),
        ("servers", "127.0.0.1:80"),
    ];
    let r = app
        .clone()
        .oneshot(form_request("node.createNodeUpstream", &fields))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        body(r).await["result"]["nodeError"]["failure"]["kind"],
        "forbidden"
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    db.client
        .execute("UPDATE nodes SET token_role='admin' WHERE id='n'", &[])
        .await
        .unwrap();
    db.client
        .execute("UPDATE users SET role='viewer' WHERE id='u1'", &[])
        .await
        .unwrap();
    let r = app
        .clone()
        .oneshot(form_request("node.createNodeUpstream", &fields))
        .await
        .unwrap();
    assert_eq!(
        body(r).await["result"]["nodeError"]["failure"]["kind"],
        "forbidden"
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    db.client
        .execute("UPDATE users SET role='editor' WHERE id='u1'", &[])
        .await
        .unwrap();
    let r = app
        .oneshot(form_request("node.createNodeUpstream", &fields))
        .await
        .unwrap();
    let result = body(r).await;
    assert_eq!(
        result["result"]["nodeError"]["failure"]["kind"],
        "node_error"
    );
    assert_eq!(result["refresh"], true);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let node = nwui_api::nodes::service::get(&db.state, "n").await.unwrap();
    assert_eq!(node.last_status.as_deref(), Some("ok"));
    assert_eq!(
        auth_support::audit_rows(&db.client, "node:upstream:create")
            .await
            .len(),
        1
    );
    task.abort();
}

#[tokio::test]
async fn downstream_drop_closes_upstream_stream() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        socket.read(&mut buf).await.unwrap();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), socket.read(&mut buf))
            .await
            .unwrap()
            .unwrap()
    });
    let response = reqwest::get(format!("http://{address}")).await.unwrap();
    let (_shutdown, receiver) = tokio::sync::watch::channel(false);
    let body = relay::stream_body(response, receiver);
    drop(body);
    assert_eq!(upstream.await.unwrap(), 0);
}

#[tokio::test]
async fn shutdown_finishes_relay_and_server_with_connected_downstream() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let remote = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let remote_addr = remote.local_addr().unwrap();
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = remote.accept().await.unwrap();
        let mut buf = [0; 4096];
        socket.read(&mut buf).await.unwrap();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n:hi\n\n\r\n").await.unwrap();
        socket.read(&mut buf).await.unwrap()
    });
    let (shutdown, mut stopped) = tokio::sync::watch::channel(false);
    let relay_shutdown = shutdown.subscribe();
    let app = Router::new().route(
        "/",
        axum::routing::get(move || {
            let relay_shutdown = relay_shutdown.clone();
            async move {
                let response = reqwest::get(format!("http://{remote_addr}")).await.unwrap();
                relay::stream_body(response, relay_shutdown)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                stopped.wait_for(|v| *v).await.unwrap();
            })
            .await
            .unwrap();
    });
    let mut downstream = reqwest::get(format!("http://{addr}")).await.unwrap();
    assert_eq!(downstream.chunk().await.unwrap().unwrap(), ":hi\n\n");
    shutdown.send(true).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), server)
        .await
        .expect("connected remote SSE must not block server shutdown")
        .unwrap();
    assert!(downstream.chunk().await.unwrap().is_none());
    assert_eq!(upstream.await.unwrap(), 0);
}

#[tokio::test]
async fn shutdown_closes_upstream_even_when_downstream_is_not_reading() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let (ready, written) = tokio::sync::oneshot::channel();
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0; 4096];
        socket.read(&mut buf).await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
            .await
            .unwrap();
        for _ in 0..8 {
            socket.write_all(b"1\r\na\r\n").await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        ready.send(()).unwrap();
        socket.read(&mut buf).await.unwrap()
    });
    let response = reqwest::get(format!("http://{address}")).await.unwrap();
    let (shutdown, receiver) = tokio::sync::watch::channel(false);
    let body = relay::stream_body(response, receiver);
    written.await.unwrap();
    shutdown.send(true).unwrap();
    // Do not consume or drop the body: shutdown must interrupt the blocked send.
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), upstream)
            .await
            .unwrap()
            .unwrap(),
        0
    );
    drop(body);
}

#[test]
fn only_config_apply_and_rollback_receive_the_three_hundred_second_reload_budget() {
    use std::time::Duration;
    let mut form=FormInput::default();
    form.fields.insert("id".into(),vec!["snapshot-1".into()]);
    form.args=vec!["n".into(),"/nodes/n/stream".into()];
    for id in ["node.applyNodeConfig","node.rollbackNodeSnapshot"] {
        assert_eq!(relay::command(id,"n",&form).unwrap().timeout,Duration::from_secs(300));
    }
    assert_eq!(relay::command("node.replaceNodeCertificate","n",&form).unwrap().timeout,Duration::from_secs(120));
    assert_eq!(relay::command("node.deleteNodeServer","n",&form).unwrap().timeout,Duration::from_secs(10));
}

#[test]
fn audit_concurrency_local_and_node_forms_forward_the_optional_version_exactly() {
    use nwui_api::control::resources::{self, Resource};
    for (kind, command) in [(Resource::Server,"node.updateNodeServer"), (Resource::Upstream,"node.updateNodeUpstream"), (Resource::Stream,"node.updateNodeStream")] {
        let mut form = FormInput::default();
        for (name, value) in [("id","row-1"),("name","backend"),("servers","127.0.0.1:9000"),("serverName","example.test"),("listen","19000"),("protocol","tcp"),("mode","direct"),("proxyPass","http://127.0.0.1:9000")] {
            form.fields.insert(name.into(), vec![value.into()]);
        }
        assert!(resources::form_body(kind, &form).unwrap().get("version").is_none());
        assert!(relay::command(command, "n", &form).unwrap().body.unwrap().get("version").is_none());
        form.fields.insert("version".into(), vec!["9007199254740993".into()]);
        assert_eq!(resources::form_body(kind, &form).unwrap()["version"], "9007199254740993");
        assert_eq!(relay::command(command, "n", &form).unwrap().body.unwrap()["version"], "9007199254740993");
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_e6_s7_open_gateway_relay_ends_after_local_unpair() {
    use http_body_util::BodyExt;
    use tokio::io::{AsyncReadExt,AsyncWriteExt};
    use std::time::Duration;
    let db=auth_support::test_db().await;
    auth_support::insert_user(&db.client,auth_support::UserRow::default(),None).await;
    let listener=tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();let addr=listener.local_addr().unwrap();let version=db.state.config.version.clone();
    let upstream=tokio::spawn(async move{let(mut socket,_)=listener.accept().await.unwrap();let mut request=Vec::new();let mut chunk=[0;4096];while !request.windows(4).any(|x|x==b"\r\n\r\n"){let n=socket.read(&mut chunk).await.unwrap();assert!(n>0);request.extend_from_slice(&chunk[..n]);assert!(request.len()<16*1024);}socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nX-Node-Version: {version}\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n:hi\n\n\r\n").as_bytes()).await.unwrap();let mut byte=[0];socket.read(&mut byte).await.unwrap()});
    let sealed=db.state.secrets.seal("audit-e6-remote-token").unwrap();
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id) VALUES('s7','remote',$1,$2,'admin','s7-instance')",&[&format!("http://{addr}"),&sealed]).await.unwrap();
    db.client.execute("UPDATE nodes SET last_version='2.0.0' WHERE id='s7'",&[]).await.unwrap();
    let router=nwui_api::app::with_request_context(relay::router().with_state(db.state.clone()),None);
    let request=Request::builder().uri("/api/nodes/s7/logs/stream").header("host","admin.example.test").header("cookie",cookie()).extension(ConnectInfo(SocketAddr::from(([127,0,0,1],40000)))).body(Body::empty()).unwrap();
    let response=router.oneshot(request).await.unwrap();assert_eq!(response.status(),StatusCode::OK);let mut stream=response.into_body();
    assert!(tokio::time::timeout(Duration::from_secs(2),stream.frame()).await.unwrap().is_some());
    db.client.execute("DELETE FROM nodes WHERE id='s7'",&[]).await.unwrap();
    let ended=tokio::time::timeout(Duration::from_secs(35),async{while stream.frame().await.is_some(){}}).await.is_ok();
    drop(stream);db.state.runtime.shutdown.send_replace(true);
    assert_eq!(tokio::time::timeout(Duration::from_secs(2),upstream).await.unwrap().unwrap(),0);
    assert!(ended,"local unpair must terminate the relay even if the old upstream stays open");
}

#[test]
fn stale_form_protection_survives_resubmission_and_cross_tab_cache_refresh() {
    for command in ["node.updateNodeServer", "node.updateNodeStream", "node.updateNodeUpstream"] {
        let mut input=FormInput::default();
        input.fields.insert("nodeVersionFamily".into(),vec!["1.19".into()]);
        for previous in [Some("1.19.11"),Some("2.0.0"),Some("2.0.0")] {
            let failure=relay::check_form_version(command,&input,previous,Some("2.0.0")).unwrap_err();
            assert_eq!(serde_json::to_value(failure).unwrap()["kind"],"conflict");
        }
        input.fields.insert("nodeVersionFamily".into(),vec!["2.0".into()]);
        assert!(relay::check_form_version(command,&input,Some("1.19.11"),Some("2.0.0")).is_ok());
        input.fields.clear();
        assert!(relay::check_form_version(command,&input,Some("1.19.11"),Some("2.0.0")).is_err());
        assert!(relay::check_form_version(command,&input,Some("2.0.0"),Some("2.0.0")).is_ok());
    }
    let mut legacy=FormInput::default();
    legacy.fields.insert("nodeVersionFamily".into(),vec!["1.19".into()]);
    for command in ["node.toggleNodeServer","node.toggleNodeStream","node.deleteNodeServer","node.deleteNodeStream","node.deleteNodeUpstream","node.applyNodeConfig","node.rollbackNodeSnapshot","node.createNodeServer"] {
        assert!(relay::check_form_version(command,&legacy,Some("1.19.11"),Some("2.0.0")).is_ok(),"{command}");
    }
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn first_actions_negotiate_before_sending_once_and_keep_stale_forms_blocked() {
    use axum::{http::HeaderMap, routing::get};
    let db=auth_support::test_db_with(&[("API_DB_POOL_SIZE","1")]).await;
    auth_support::insert_user(&db.client,auth_support::UserRow {role:"admin",..Default::default()},None).await;
    let version=Arc::new(std::sync::Mutex::new("1.19.11"));
    let observed=version.clone();
    let infos=Arc::new(AtomicUsize::new(0));let info_count=infos.clone();
    let writes=Arc::new(AtomicUsize::new(0));let count=writes.clone();
    let remote_db=db.state.clone();
    let wire_version=version.clone();
    let remote=Router::new()
        .route("/api/v1/info",get(move |headers:HeaderMap| {
            let version=*observed.lock().unwrap();let info_count=info_count.clone();
            async move {
                info_count.fetch_add(1,Ordering::SeqCst);
                assert_eq!(headers["authorization"],"Bearer paired-secret");
                if version=="unauthorized" {return (StatusCode::UNAUTHORIZED,[("x-node-version","2.0.0")],Json(json!({})));}
                if version=="1.19.11" && headers["x-gateway-version"]!="1.19.11" {
                    return (StatusCode::CONFLICT,[("x-node-version",version)],Json(json!({"error":{"code":"version_mismatch"}})));
                }
                (StatusCode::OK,[("x-node-version",version)],Json(json!({"instanceId":"bound","semver":version})))
            }
        }))
        .fallback(move |request:Request<Body>| {
            let version=*wire_version.lock().unwrap();let count=count.clone();let state=remote_db.clone();
            async move {
                // A 1.19.11 node receives the compatibility version; a 2.x node receives this product's own version.
                let gateway=if version=="1.19.11" {"1.19.11".to_owned()} else {product_version()};
                assert_eq!(request.headers()["x-gateway-version"].to_str().unwrap(),gateway);
                assert_eq!(service_version(&state).await,version,"persist before write, without holding the sole pool connection");
                if version=="2.0.0" && request.method()=="PATCH" {
                    let payload:Value=serde_json::from_slice(&axum::body::to_bytes(request.into_body(),1<<20).await.unwrap()).unwrap();
                    assert_eq!(payload["resolveAtRuntime"],true,"modern reload keeps 2.0 settings");
                }
                count.fetch_add(1,Ordering::SeqCst);
                ([("x-node-version",version)],Json(json!({"ok":true,"output":"ok","revoked":true,"upstream":{"id":"u"},"server":{"id":"s"},"stream":{"id":"s"}})))
            }
        });
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let base=format!("http://{}",listener.local_addr().unwrap());
    let task=tokio::spawn(async move {axum::serve(listener,remote).await.unwrap()});
    let sealed=db.state.secrets.seal("paired-secret").unwrap();
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id) VALUES('n','remote',$1,$2,'admin','bound')",&[&base,&sealed]).await.unwrap();
    let router=nwui_api::app::with_request_context(relay::router().with_state(db.state.clone()),None);
    let common=[("_apiArgs",r#"["n"]"#),("id","s"),("name","pool"),("servers","127.0.0.1:9000"),("listen","19000"),("protocol","tcp"),("proxyPass","127.0.0.1:9000")];
    for (command,args) in [("node.applyNodeConfig",r#"["n","/nodes/n/stream"]"#),("node.rollbackNodeSnapshot",r#"["n","snapshot"]"#),("node.updateNodeUpstream",r#"["n"]"#),("node.deleteNodeStream",r#"["n"]"#),("node.toggleNodeServer",r#"["n"]"#)] {
        db.client.execute("UPDATE nodes SET last_version=NULL WHERE id='n'",&[]).await.unwrap();
        let before=writes.load(Ordering::SeqCst);let probes=infos.load(Ordering::SeqCst);
        let mut fields=common.to_vec();fields[0].1=args;
        let result=body(router.clone().oneshot(form_request(command,&fields)).await.unwrap()).await;
        assert_eq!(writes.load(Ordering::SeqCst),before+1,"{command}: {result}");
        assert_eq!(infos.load(Ordering::SeqCst),probes+2,"{command}");
    }
    // The cache is legacy, then the node upgrades. The old draft must remain blocked on every retry.
    *version.lock().unwrap()="2.0.0";
    let mut fields=common.to_vec();fields.push(("nodeVersionFamily","1.19"));
    for _ in 0..3 {
        let before=writes.load(Ordering::SeqCst);
        let result=body(router.clone().oneshot(form_request("node.updateNodeUpstream",&fields)).await.unwrap()).await;
        assert_eq!(result["result"]["nodeError"]["failure"]["kind"],"conflict","{result}");
        assert_eq!(writes.load(Ordering::SeqCst),before);
        assert_eq!(service_version(&db.state).await,"2.0.0");
    }
    // A reloaded modern form proceeds, with no extra probe for a known modern cache.
    fields.last_mut().unwrap().1="2.0";fields.push(("resolveAtRuntime","true"));let probes=infos.load(Ordering::SeqCst);let before=writes.load(Ordering::SeqCst);
    router.clone().oneshot(form_request("node.updateNodeUpstream",&fields)).await.unwrap();
    assert_eq!(writes.load(Ordering::SeqCst),before+1);assert_eq!(infos.load(Ordering::SeqCst),probes);
    // Non-UI callers retain the transition conflict.
    fields.retain(|(name,_)| *name!="nodeVersionFamily");db.client.execute("UPDATE nodes SET last_version='1.19.11' WHERE id='n'",&[]).await.unwrap();
    let before=writes.load(Ordering::SeqCst);
    let result=body(router.clone().oneshot(form_request("node.updateNodeUpstream",&fields)).await.unwrap()).await;
    assert_eq!(result["result"]["nodeError"]["failure"]["kind"],"conflict");assert_eq!(writes.load(Ordering::SeqCst),before);
    for (command,args) in [("node.toggleNodeServer",r#"["n"]"#),("node.deleteNodeStream",r#"["n"]"#),("node.applyNodeConfig",r#"["n","/nodes/n/stream"]"#),("node.rollbackNodeSnapshot",r#"["n","snapshot"]"#)] {
        db.client.execute("UPDATE nodes SET last_version='1.19.11' WHERE id='n'",&[]).await.unwrap();
        let mut fields=common.to_vec();fields[0].1=args;fields.push(("nodeVersionFamily","1.19"));
        let before=writes.load(Ordering::SeqCst);
        router.clone().oneshot(form_request(command,&fields)).await.unwrap();
        assert_eq!(writes.load(Ordering::SeqCst),before+1,"{command} still sent after node upgrade");
    }
    *version.lock().unwrap()="unauthorized";
    db.client.execute("UPDATE nodes SET last_version=NULL WHERE id='n'",&[]).await.unwrap();
    let before=writes.load(Ordering::SeqCst);
    let result=body(router.clone().oneshot(form_request("node.updateNodeUpstream",&common)).await.unwrap()).await;
    assert_eq!(result["result"]["nodeError"]["failure"]["kind"],"unauthorized");assert_eq!(writes.load(Ordering::SeqCst),before);
    *version.lock().unwrap()="2.0.0";
    // Direct cancellation and remote unpair also prepare their registered target.
    db.client.execute("UPDATE nodes SET last_version=NULL WHERE id='n'",&[]).await.unwrap();
    let cancel=Request::builder().method("POST").uri("/api/nodes/n/certificates/c/cancel").header("host","admin.example.test").header("origin","http://admin.example.test").header("cookie",cookie()).extension(ConnectInfo(SocketAddr::from(([127,0,0,1],40000)))).body(Body::empty()).unwrap();
    let before=writes.load(Ordering::SeqCst);assert_eq!(router.oneshot(cancel).await.unwrap().status(),StatusCode::OK);assert_eq!(writes.load(Ordering::SeqCst),before+1);
    db.client.execute("UPDATE nodes SET last_version=NULL WHERE id='n'",&[]).await.unwrap();
    let before=writes.load(Ordering::SeqCst);
    assert!(!nwui_api::nodes::service::unpair_node(&db.state,"n",&nwui_api::contracts::Actor {id:None,label:Some("test".into()),ip:None}).await.unwrap());
    assert_eq!(writes.load(Ordering::SeqCst),before+1);
    task.abort();
}
async fn service_version(state:&nwui_api::state::AppState)->String {
    nwui_api::nodes::service::get(state,"n").await.unwrap().last_version.unwrap()
}

#[test]
fn node_form_rejects_an_unknown_version_family_before_egress() {
    let mut form=FormInput::default();
    form.fields.insert("nodeVersionFamily".into(),vec!["garbage".into()]);
    assert!(relay::command("node.applyNodeConfig","n",&form).is_err());
}
