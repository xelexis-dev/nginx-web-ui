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
                assert_eq!(headers["x-gateway-version"],product_version());
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
                assert_ne!(version,"1.19.11","incompatible nodes must receive no writes");
                assert_eq!(request.headers()["x-gateway-version"].to_str().unwrap(),product_version());
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
    for remote_version in ["1.19.11","2.0.0","2.1.0"] {
        *version.lock().unwrap()=remote_version;
        for (command,args) in [("node.applyNodeConfig",r#"["n","/nodes/n/stream"]"#),("node.rollbackNodeSnapshot",r#"["n","snapshot"]"#),("node.updateNodeUpstream",r#"["n"]"#),("node.deleteNodeStream",r#"["n"]"#),("node.toggleNodeServer",r#"["n"]"#)] {
            db.client.execute("UPDATE nodes SET last_version=NULL WHERE id='n'",&[]).await.unwrap();
            let before=writes.load(Ordering::SeqCst);let probes=infos.load(Ordering::SeqCst);
            let mut fields=common.to_vec();fields[0].1=args;fields.push(("resolveAtRuntime","true"));
            let result=body(router.clone().oneshot(form_request(command,&fields)).await.unwrap()).await;
            if remote_version=="1.19.11" {
                let failure=result["result"].get("nodeError").map(|error|&error["failure"]).unwrap_or(&result["result"]);
                assert_eq!(failure["kind"],"version_mismatch","{command}: {result}");
                assert_eq!(writes.load(Ordering::SeqCst),before);
                assert!(nwui_api::nodes::service::get(&db.state,"n").await.unwrap().last_version.is_none());
            } else {
                assert_eq!(writes.load(Ordering::SeqCst),before+1,"{command}: {result}");
                assert_eq!(service_version(&db.state).await,remote_version);
            }
            assert_eq!(infos.load(Ordering::SeqCst),probes+1,"{command}: no 1.19 retry");
        }
    }
    // The cache is legacy, then the node upgrades. The old draft must remain blocked on every retry.
    *version.lock().unwrap()="2.0.0";
    db.client.execute("UPDATE nodes SET last_version='1.19.11' WHERE id='n'",&[]).await.unwrap();
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
    assert!(!nwui_api::nodes::service::unpair_node(&db.state,"n",&nwui_api::contracts::Actor {via: None,id:None,label:Some("test".into()),ip:None}).await.unwrap());
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

#[test]
fn http2_node_gate_refuses_unknown_and_old_and_strips_false_only() {
    for version in [None,Some("unknown"),Some("1.19.11"),Some("2.0.99")] {
        for name in ["node.createNodeServer","node.updateNodeServer"] {
            let mut on=json!({"tls":{"tlsOptions":{"http2":true,"sslProtocols":"modern"}}});
            let error=relay::check_http2_capability(name,Some(&mut on),version).unwrap_err();
            let value=serde_json::to_value(error).unwrap();assert_eq!(value["kind"],"conflict");assert!(value["fieldErrors"]["http2"].is_string());assert_eq!(on["tls"]["tlsOptions"]["http2"],true);
            let mut off=json!({"tls":{"tlsOptions":{"http2":false,"sslProtocols":"modern"}}});
            relay::check_http2_capability(name,Some(&mut off),version).unwrap();assert!(off["tls"]["tlsOptions"].get("http2").is_none());assert_eq!(off["tls"]["tlsOptions"]["sslProtocols"],"modern");
            let mut absent=json!({"tls":{}});relay::check_http2_capability(name,Some(&mut absent),version).unwrap();assert_eq!(absent,json!({"tls":{}}));
        }
    }
    for version in ["2.1.0","2.1.1","2.2.0"] {let mut on=json!({"tls":{"tlsOptions":{"http2":true}}});relay::check_http2_capability("node.createNodeServer",Some(&mut on),Some(version)).unwrap();assert_eq!(on["tls"]["tlsOptions"]["http2"],true);}
}

#[tokio::test]
#[ignore="requires NWUI_TEST_DATABASE_URL"]
async fn http2_writes_probe_in_this_request_and_gate_downgrades_and_upgrades() {
    use axum::routing::get;
    let mut db=auth_support::test_db().await;
    db.state.node_http=nwui_api::nodes::client::NodeHttp::new("2.1.0",&db.state.config).unwrap();
    auth_support::insert_user(&db.client,auth_support::UserRow{role:"admin",..Default::default()},None).await;
    let live=Arc::new(std::sync::Mutex::new("2.0.3"));let actual=live.clone();
    let probes=Arc::new(AtomicUsize::new(0));let seen_probes=probes.clone();
    let writes=Arc::new(AtomicUsize::new(0));let seen_writes=writes.clone();
    let sent=Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));let seen_bodies=sent.clone();
    let race=Arc::new(std::sync::atomic::AtomicBool::new(false));let raced=race.clone();
    let remote=Router::new().route("/api/v1/info",get(move || {
        let version=*actual.lock().unwrap();seen_probes.fetch_add(1,Ordering::SeqCst);
        async move {([("x-node-version",version)],Json(json!({"instanceId":"http2-bound","semver":version})))}
    })).route("/api/v1/servers",post(move |Json(value):Json<Value>| {
        seen_writes.fetch_add(1,Ordering::SeqCst);seen_bodies.lock().unwrap().push(value.clone());let changed=raced.load(Ordering::SeqCst);
        async move {([("x-node-version",if changed {"2.0.3"} else {"2.1.0"})],Json(if changed {json!({"server":{"id":"s","config":{}}})} else {json!({"server":{"id":"s","config":{"tlsOptions":value["tls"]["tlsOptions"]}}})}))}
    }));
    let listener=tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();let address=listener.local_addr().unwrap();
    let task=tokio::spawn(async move {axum::serve(listener,remote).await.unwrap()});
    let token=db.state.secrets.seal("http2-paired-test").unwrap();
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id,last_version) VALUES('n','http2',$1,$2,'admin','http2-bound','2.1.0')",&[&format!("http://{address}"),&token]).await.unwrap();
    let app=nwui_api::app::with_request_context(relay::router().with_state(db.state.clone()),None);
    let mut fields=vec![("_apiArgs",r#"["n"]"#),("serverName","h2.test"),("listen","18080"),("mode","direct"),("proxyPass","http://127.0.0.1:9000"),("certificateId","cert"),("tlsOptions",r#"{"http2":true}"#)];
    let response=body(app.clone().oneshot(form_request("node.createNodeServer",&fields)).await.unwrap()).await;
    assert!(response["result"]["fieldErrors"]["http2"].is_string(),"{response}");assert_eq!(writes.load(Ordering::SeqCst),0);assert_eq!(probes.load(Ordering::SeqCst),1);
    *live.lock().unwrap()="2.1.0";
    let response=body(app.clone().oneshot(form_request("node.createNodeServer",&fields)).await.unwrap()).await;
    assert!(response["redirectTo"].is_string(),"{response}");assert_eq!(writes.load(Ordering::SeqCst),1);assert_eq!(probes.load(Ordering::SeqCst),2);assert_eq!(sent.lock().unwrap()[0]["tls"]["tlsOptions"]["http2"],true);
    // Another process may replace the node after the probe. Do not claim success.
    race.store(true,Ordering::SeqCst);
    let response=body(app.clone().oneshot(form_request("node.createNodeServer",&fields)).await.unwrap()).await;
    assert!(response["redirectTo"].is_null());assert!(response.to_string().contains("다른 설정이 저장되었을 수"));assert_eq!(writes.load(Ordering::SeqCst),2);
    *live.lock().unwrap()="2.0.3";race.store(false,Ordering::SeqCst);
    fields.last_mut().unwrap().1=r#"{"http2":false}"#;
    let before=probes.load(Ordering::SeqCst);
    let response=body(app.clone().oneshot(form_request("node.createNodeServer",&fields)).await.unwrap()).await;
    assert!(response["redirectTo"].is_string(),"{response}");assert_eq!(probes.load(Ordering::SeqCst),before);assert!(sent.lock().unwrap().last().unwrap()["tls"]["tlsOptions"].get("http2").is_none());
    *live.lock().unwrap()="unknown";fields.last_mut().unwrap().1=r#"{"http2":true}"#;
    db.client.execute("UPDATE nodes SET last_version=NULL WHERE id='n'",&[]).await.unwrap();
    let before=writes.load(Ordering::SeqCst);
    let response=body(app.oneshot(form_request("node.createNodeServer",&fields)).await.unwrap()).await;
    assert!(response["result"]["fieldErrors"]["http2"].is_string(),"{response}");assert_eq!(writes.load(Ordering::SeqCst),before);
    task.abort();
}

async fn plain_node_write_through_real_router(update:bool) {
    use axum::{extract::Request as AxumRequest,middleware::{from_fn,Next}};
    let mut gateway=auth_support::test_db().await;let mut node=auth_support::test_db().await;
    std::sync::Arc::make_mut(&mut node.state.config).version="2.1.0".into();
    gateway.state.node_http=nwui_api::nodes::client::NodeHttp::new("2.1.0",&gateway.state.config).unwrap();
    auth_support::insert_user(&gateway.client,auth_support::UserRow{role:"admin",..Default::default()},None).await;
    auth_support::gateway_issuer(&node.client).await;
    node.client.execute("INSERT INTO gateway_tokens(id,name,token_hash,role,created_by_user_id) VALUES('plain','plain',$1,'admin','q3-gateway-issuer')",&[&nwui_api::auth::token_digest("plain-node-fixture")]).await.unwrap();
    let server_id="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    if update {node.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES($1,'plain-before.test',18081,$2)",&[&server_id,&json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();}
    let writes=Arc::new(AtomicUsize::new(0));let observed=writes.clone();
    let remote=nwui_api::server::router(node.state.clone()).layer(from_fn(move |request:AxumRequest,next:Next| {
        let observed=observed.clone();async move {
            let(parts,body)=request.into_parts();let bytes=axum::body::to_bytes(body,1<<20).await.unwrap();
            let payload:Value=serde_json::from_slice(&bytes).unwrap();
            assert_eq!(payload.get("tls"),Some(&Value::Null));assert_eq!(payload.get("certificateId"),Some(&Value::Null));
            observed.fetch_add(1,Ordering::SeqCst);
            next.run(axum::http::Request::from_parts(parts,Body::from(bytes))).await
        }
    }));
    let listener=tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();let address=listener.local_addr().unwrap();
    let task=tokio::spawn(async move {axum::serve(listener,remote.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap()});
    let sealed=gateway.state.secrets.seal("plain-node-fixture").unwrap();
    gateway.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id,last_version) VALUES('n','plain',$1,$2,'admin','plain-instance','2.1.0')",&[&format!("http://{address}"),&sealed]).await.unwrap();
    let app=nwui_api::server::router(gateway.state.clone());
    let fields=[("_apiArgs",r#"["n"]"#),("id",server_id),("serverName","plain-node.test"),("listen","18080"),("mode","direct"),("proxyPass","http://127.0.0.1:9000")];
    let response=app.oneshot(form_request(if update {"node.updateNodeServer"} else {"node.createNodeServer"},&fields)).await.unwrap();
    assert_eq!(response.status(),200);let result=body(response).await;
    assert!(result["redirectTo"].is_string(),"plain server {update}: {result}");assert_eq!(writes.load(Ordering::SeqCst),1);
    let row=node.client.query_one("SELECT certificate_id,config FROM nginx_servers WHERE server_name='plain-node.test'",&[]).await.unwrap();
    assert!(row.get::<_,Option<String>>(0).is_none());let config:Value=row.get(1);assert!(config.get("tlsPort").is_none());assert!(config.get("tlsOptions").is_none());
    task.abort();
}
#[tokio::test]
#[ignore="requires NWUI_TEST_DATABASE_URL"]
async fn plain_node_create_accepts_null_tls() {plain_node_write_through_real_router(false).await;}
#[tokio::test]
#[ignore="requires NWUI_TEST_DATABASE_URL"]
async fn plain_node_update_accepts_null_tls() {plain_node_write_through_real_router(true).await;}

#[tokio::test]
#[ignore="requires NWUI_TEST_DATABASE_URL"]
async fn plain_local_browser_create_and_update_keep_tls_absent() {
    let db=auth_support::test_db().await;
    auth_support::insert_user(&db.client,auth_support::UserRow{role:"admin",..Default::default()},None).await;
    let app=nwui_api::server::router(db.state.clone());
    let mut fields=vec![("serverName","plain-local.test"),("listen","18080"),("mode","direct"),("proxyPass","http://127.0.0.1:9000")];
    let created=body(app.clone().oneshot(form_request("reverse-proxy.createServer",&fields)).await.unwrap()).await;
    assert!(created["redirectTo"].is_string(),"{created}");
    let row=db.client.query_one("SELECT id FROM nginx_servers WHERE server_name='plain-local.test'",&[]).await.unwrap();let id:String=row.get(0);
    fields.push(("id",&id));fields[0].1="plain-local-edited.test";
    let updated=body(app.oneshot(form_request("reverse-proxy.updateServer",&fields)).await.unwrap()).await;
    assert!(updated["redirectTo"].is_string(),"{updated}");
    let row=db.client.query_one("SELECT certificate_id,config FROM nginx_servers WHERE id=$1",&[&id]).await.unwrap();assert!(row.get::<_,Option<String>>(0).is_none());let config:Value=row.get(1);assert!(config.get("tlsOptions").is_none());assert!(config.get("tlsPort").is_none());
}
