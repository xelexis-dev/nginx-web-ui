mod outbound_support;
use nwui_api::{
    contracts::{FormInput, Role},
    nodes::service,
};
use serde_json::json;

#[test]
fn node_form_rejects_credential_urls_and_counts_utf16() {
    let mut form = FormInput::default();
    for (key, value) in [
        ("name", "node"),
        ("baseUrl", "https://u:p@example.test"),
        ("gatewayName", "gateway"),
        ("token", "secret"),
    ] {
        form.fields.insert(key.into(), vec![value.into()]);
    }
    assert!(
        service::parse_form(&form, true)
            .err()
            .unwrap()
            .contains_key("baseUrl")
    );
    form.fields
        .insert("baseUrl".into(), vec!["https://example.test///".into()]);
    assert_eq!(
        service::parse_form(&form, true).unwrap().base_url,
        "https://example.test"
    );
    form.fields.insert("name".into(), vec!["😀".repeat(51)]);
    assert!(
        service::parse_form(&form, true)
            .err()
            .unwrap()
            .contains_key("name")
    );
}

#[test]
fn pair_role_defaults_only_when_absent_and_effective_role_never_escalates() {
    assert_eq!(
        service::parse_pair_body(&json!({"token":" t ","gatewayName":" g "}))
            .unwrap()
            .role,
        Role::Editor
    );
    for role in [json!(null), json!("owner"), json!(4)] {
        assert!(
            service::parse_pair_body(&json!({"token":"t","gatewayName":"g","role":role}))
                .err()
                .unwrap()
                .contains_key("role")
        );
    }
    for role in [Role::Viewer, Role::Editor, Role::Admin] {
        assert_eq!(service::min_role(role, Role::Viewer), Role::Viewer);
        assert_eq!(service::min_role(Role::Viewer, role), Role::Viewer);
        assert_eq!(service::min_role(role, Role::Admin), role);
    }
}

#[path = "auth_support/mod.rs"]
mod auth_support;

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn instance_identity_is_stable_under_concurrent_boots_and_preserves_settings() {
    let db = auth_support::test_db().await;
    db.client.execute("INSERT INTO app_settings(id,public_base_url,require_2fa_admin) VALUES ('current','https://gateway.test',true)",&[]).await.unwrap();
    let (one, two) = tokio::join!(
        service::ensure_instance_id(&db.state),
        service::ensure_instance_id(&db.state)
    );
    assert_eq!(one.unwrap(), two.unwrap());
    let row=db.client.query_one("SELECT require_2fa_admin,public_base_url,instance_id FROM app_settings WHERE id='current'",&[]).await.unwrap();
    assert!(row.get::<_, bool>(0));
    assert_eq!(row.get::<_, String>(1), "https://gateway.test");
    assert!(!row.get::<_, String>(2).is_empty());
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn registry_seals_returned_token_uses_remote_role_and_revokes_duplicates() {
    use axum::{Json, Router, extract::State, routing::post};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let revoked = Arc::new(AtomicUsize::new(0));
    let app=Router::new().route("/api/v1/pair",post(||async{Json(json!({"token":"remote-issued-secret","role":"viewer","instanceId":"remote-instance","semver":"1.17.0"}))}).delete(|State(count):State<Arc<AtomicUsize>>|async move{count.fetch_add(1,Ordering::SeqCst);Json(json!({"revoked":true}))})).with_state(revoked.clone());
    let listener = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let db = auth_support::test_db_with(&[("API_DB_POOL_SIZE", "1")]).await;
    let actor = nwui_api::contracts::Actor {
        via: None,
        id: None,
        label: Some("test".into()),
        ip: None,
    };
    let input = || service::NodeInput {
        name: "remote".into(),
        base_url: format!("http://{addr}"),
        allow_insecure_tls: false,
        pair: Some(service::PairInput {
            token: "presented-server-secret".into(),
            gateway_name: "gateway".into(),
            role: Role::Admin,
        }),
    };
    let id = service::pair_node(&db.state, input(), &actor)
        .await
        .unwrap()
        .unwrap();
    let node = service::get(&db.state, &id).await.unwrap();
    assert_eq!(node.token_role, Role::Viewer);
    let projection = serde_json::to_string(&node).unwrap();
    assert!(!projection.contains("secret"));
    assert!(!projection.contains("tokenEnc"));
    let sealed = db
        .client
        .query_one("SELECT token_enc FROM nodes WHERE id=$1", &[&id])
        .await
        .unwrap()
        .get::<_, String>(0);
    assert_ne!(sealed, "remote-issued-secret");
    assert_eq!(
        db.state.secrets.open(Some(&sealed)).as_deref(),
        Some("remote-issued-secret")
    );
    assert!(
        service::pair_node(&db.state, input(), &actor)
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(revoked.load(Ordering::SeqCst), 1);
    let mut update = input();
    update.name = "renamed".into();
    service::update_node(&db.state, &id, update, &actor)
        .await
        .unwrap();
    assert_eq!(
        service::get(&db.state, &id).await.unwrap().token_role,
        Role::Viewer
    );
    // Keep the existing unreachable-revocation case without redirecting a stored token.
    server.abort();
    assert!(service::unpair_node(&db.state, &id, &actor).await.unwrap());
    assert!(service::get(&db.state, &id).await.is_err());
    let audits = db
        .client
        .query(
            "SELECT after FROM audit_logs WHERE action LIKE 'node:%'",
            &[],
        )
        .await
        .unwrap();
    for row in audits {
        let text = row.get::<_, serde_json::Value>(0).to_string();
        assert!(!text.contains("secret"));
        assert!(!text.contains("tokenEnc"));
    }
    server.abort();
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn probes_preserve_health_for_domain_failures_and_last_success_for_transport_failures() {
    use nwui_api::nodes::client::{FailureKind, NodeResult};
    let db = auth_support::test_db().await;
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id) VALUES('n','node','https://node.test','sealed','editor','remote')",&[]).await.unwrap();
    service::record_probe(
        &db.state,
        "n",
        &NodeResult::Success {
            ok: true,
            value: json!({}),
            node_version: Some("1.17.0".into()),
        },
    )
    .await
    .unwrap();
    let before = service::get(&db.state, "n").await.unwrap();
    service::record_probe(
        &db.state,
        "n",
        &NodeResult::failure(FailureKind::Conflict, "in use", Some("2.0.0".into())),
    )
    .await
    .unwrap();
    let domain = service::get(&db.state, "n").await.unwrap();
    assert_eq!(domain.last_status.as_deref(), Some("ok"));
    assert_eq!(domain.last_error, None);
    service::record_probe(
        &db.state,
        "n",
        &NodeResult::failure(FailureKind::Unreachable, "offline", None),
    )
    .await
    .unwrap();
    let after = service::get(&db.state, "n").await.unwrap();
    assert_eq!(after.last_seen_at, before.last_seen_at);
    assert_eq!(after.last_version, before.last_version);
    assert_eq!(after.last_status.as_deref(), Some("unreachable"));
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn audit_i18_a_different_origin_cannot_receive_the_stored_node_token() {
    use axum::{Json, Router, http::HeaderMap, routing::get};
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
    let leaked = Arc::new(AtomicBool::new(false));
    let seen = leaked.clone();
    let routes = Router::new().route("/api/v1/info", get(move |headers: HeaderMap| {
        seen.store(headers.contains_key("authorization"), Ordering::SeqCst);
        async { Json(json!({"instanceId":"different-instance","semver":"1.20.0"})) }
    }));
    let listener = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let new_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
    let db = auth_support::test_db_with(&[("API_DB_POOL_SIZE", "1")]).await;
    let token = db.state.secrets.seal("must-not-leak").unwrap();
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id,gateway_name) VALUES('known','Known','http://127.0.0.1:9',$1,'viewer','original-instance','gateway')", &[&token]).await.unwrap();
    let actor = nwui_api::contracts::Actor { via: None, id:None, label:Some("audit".into()), ip:None };
    let result = service::update_node(&db.state, "known", service::NodeInput { name:"Renamed".into(),base_url:new_url,allow_insecure_tls:false,pair:None }, &actor).await;
    if result.is_ok() { service::refresh_statuses(&db.state).await.unwrap(); }
    assert!(!leaked.load(Ordering::SeqCst), "editing baseUrl leaked the stored token to a different node instance");
    assert!(result.is_err());
    assert_eq!(service::get(&db.state, "known").await.unwrap().base_url, "http://127.0.0.1:9");
    service::update_node(&db.state, "known", service::NodeInput { name:"Renamed".into(),base_url:"http://127.0.0.1:9".into(),allow_insecure_tls:false,pair:None }, &actor).await.unwrap();
    server.abort();
}

#[test]
fn negotiation_predicate_limits_the_legacy_read_cost_to_pages_and_writes() {
    for version in [None,Some(""),Some("bad")] {
        assert!(service::needs_negotiation(version,false));assert!(service::needs_negotiation(version,true));
    }
    assert!(!service::needs_negotiation(Some("1.19.11"),false));
    assert!(service::needs_negotiation(Some("1.19.11"),true));
    for version in ["2.0.0","1.18.9","2.1.0"] {assert!(!service::needs_negotiation(Some(version),true));}
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn first_page_and_edit_reads_use_negotiated_metadata_and_read_failures_keep_the_cache() {
    use axum::{Router,Json,http::{HeaderMap,StatusCode},routing::get};
    use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
    use nwui_api::nodes::views;
    use axum::response::IntoResponse;
    let db=auth_support::test_db_with(&[("API_DB_POOL_SIZE","1")]).await;
    let mode=Arc::new(std::sync::Mutex::new("1.19.11"));let current=mode.clone();
    let page_hits=Arc::new(AtomicUsize::new(0));let hits=page_hits.clone();let page_version=mode.clone();
    let remote=Router::new().route("/api/v1/info",get(move |headers:HeaderMap| {
        let version=*current.lock().unwrap();async move {
            if version=="slow" {tokio::time::sleep(std::time::Duration::from_secs(4)).await;}
            if version=="unauthorized" {return (StatusCode::UNAUTHORIZED,[("x-node-version","2.0.0")],Json(json!({})));}
            if version=="1.19.11" && headers["x-gateway-version"]!="1.19.11" {return (StatusCode::CONFLICT,[("x-node-version",version)],Json(json!({"error":{"code":"version_mismatch"}})));}
            (StatusCode::OK,[("x-node-version",version)],Json(json!({"instanceId":"bound","semver":version})))
        }
    })).fallback(move |headers:HeaderMap| {let hits=hits.clone();let version=*page_version.lock().unwrap();async move {
        hits.fetch_add(1,Ordering::SeqCst);
        if version=="1.19.11" && headers["x-gateway-version"]!="1.19.11" {
            return (StatusCode::CONFLICT,[("x-node-version",version)],Json(json!({"error":{"code":"version_mismatch"}}))).into_response();
        }
        Json(json!({"upstream":{"id":"u","resolveAtRuntime":true}})).into_response()
    }});
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let base=format!("http://{}",listener.local_addr().unwrap());
    let server=tokio::spawn(async move {axum::serve(listener,remote).await.unwrap()});
    let token=db.state.secrets.seal("paired-secret").unwrap();
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id) VALUES('n','node',$1,$2,'admin','bound')",&[&base,&token]).await.unwrap();
    let session=nwui_api::auth::principal::SessionPrincipal {user_id:"viewer".into(),email:None,name:None,role:Role::Editor,session_version:0,totp_enabled:false,requires_2fa:false,mfa_verified:true,expires_at:i64::MAX};
    let refused=views::view(&db.state,&(session).clone().into(),"/nodes/n",&Default::default()).await.unwrap().unwrap();
    assert_eq!(refused["ok"],false);assert_eq!(refused["nodeError"]["failure"]["kind"],"version_mismatch");
    assert!(service::get(&db.state,"n").await.unwrap().last_version.is_none());
    for (stored,actual,path) in [(None,"2.0.0","/nodes/n"),(None,"2.1.0","/nodes/n"),(None,"2.0.0","/nodes/n/reverse-proxy/upstreams/u/edit"),(Some("1.19.11"),"2.0.0","/nodes/n/reverse-proxy/upstreams/u/edit")] {
        *mode.lock().unwrap()=actual;
        db.client.execute("UPDATE nodes SET last_version=$1 WHERE id='n'",&[&stored]).await.unwrap();
        let result=views::view(&db.state, &(session).clone().into(),path,&Default::default()).await.unwrap().unwrap();
        assert_eq!(result["ok"],true,"{result}");assert_eq!(result["node"]["lastVersion"],actual);
        assert_eq!(service::get(&db.state,"n").await.unwrap().last_version.as_deref(),Some(actual));
    }
    // Failed probes after a remote upgrade must preserve the stale 1.19 cache.
    for failure in ["unauthorized","slow"] {
        *mode.lock().unwrap()=failure;
        db.client.execute("UPDATE nodes SET last_version='1.19.11' WHERE id='n'",&[]).await.unwrap();
        let before=page_hits.load(Ordering::SeqCst);let started=std::time::Instant::now();
        let result=views::view(&db.state, &(session).clone().into(),"/nodes/n",&Default::default()).await.unwrap().unwrap();
        assert_eq!(service::get(&db.state,"n").await.unwrap().last_version.as_deref(),Some("1.19.11"));
        if failure=="slow" {
            assert_eq!(result["ok"],false);assert!(result.to_string().contains("unreachable"));
            assert_eq!(page_hits.load(Ordering::SeqCst),before);assert!(started.elapsed()<std::time::Duration::from_millis(3300));
        } else {assert_eq!(result["ok"],true);assert_eq!(result["node"]["lastVersion"],"1.19.11");}
    }
    server.abort();
}

#[tokio::test]
#[ignore = "real DB"]
async fn local_node_relay_audit_keeps_token_via_and_browser_null() {
    let db=auth_support::test_db().await;
    auth_support::insert_user(&db.client,auth_support::UserRow::default(),None).await;
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id) VALUES('via-node','node','https://node.test','sealed','editor','remote')",&[]).await.unwrap();
    let node=service::get(&db.state,"via-node").await.unwrap();
    for via in [None,Some(nwui_api::contracts::Via::Token{id:"relay-id".into(),name:"relay-token".into()})] {
        let actor=nwui_api::contracts::Actor{id:Some("u1".into()),label:Some("owner@example.test".into()),ip:None,via:via.clone()};
        service::audit_node(&db.state,&node,"config:write",&actor,json!({"operation":"test"})).await.unwrap();
    }
    let rows=db.client.query("SELECT via,actor_id FROM audit_logs ORDER BY created_at,id",&[]).await.unwrap();
    assert_eq!(rows.len(),2);
    assert!(rows.iter().all(|row|row.get::<_,String>(1)=="u1"));
    assert_eq!(rows[0].get::<_,Option<serde_json::Value>>(0),None);
    assert_eq!(rows[1].get::<_,serde_json::Value>(0),json!({"kind":"token","id":"relay-id","name":"relay-token"}));
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn live_reads_refresh_the_context_bar_version_without_an_extra_node_request() {
    use axum::{Json,Router,http::StatusCode,routing::get};
    use std::sync::{Arc,atomic::{AtomicUsize,AtomicBool,Ordering}};
    let version=Arc::new(std::sync::Mutex::new(Some("2.1.0")));
    let rejected=Arc::new(AtomicBool::new(false));let reject=rejected.clone();
    let remote_version=version.clone();let calls=Arc::new(AtomicUsize::new(0));let hits=calls.clone();let unexpected=calls.clone();
    let server_hits=calls.clone();let pending_hits=calls.clone();
    let remote=Router::new().route("/api/v1/status",get(move || {
        let version=*remote_version.lock().unwrap();let rejected=reject.load(Ordering::SeqCst);hits.fetch_add(1,Ordering::SeqCst);
        async move {
            let mut headers=axum::http::HeaderMap::new();if let Some(version)=version {headers.insert("x-node-version",version.parse().unwrap());}
            (if rejected {StatusCode::CONFLICT}else{StatusCode::OK},headers,Json(json!({"semver":version,"instanceId":"bound","gatewayTokenId":"untrusted-proof"})))
        }
    })).route("/api/v1/servers",get(move || {server_hits.fetch_add(1,Ordering::SeqCst);async {([("x-node-version","2.0.0")],Json(json!({"servers":[]})))}}))
    .route("/api/v1/pending",get(move || {pending_hits.fetch_add(1,Ordering::SeqCst);async {StatusCode::SERVICE_UNAVAILABLE}}))
    .fallback(move || {unexpected.fetch_add(1,Ordering::SeqCst);async {StatusCode::NOT_FOUND}});
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
    let server=tokio::spawn(async move{axum::serve(listener,remote).await.unwrap()});
    let db=auth_support::test_db().await;let sealed=db.state.secrets.seal("paired-secret").unwrap();
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id,gateway_token_id,last_version,last_seen_at) VALUES('live','node',$1,$2,'admin','bound','original-proof','2.1.0','2026-01-01')",&[&format!("http://{address}"),&sealed]).await.unwrap();
    let principal=nwui_api::auth::principal::SessionPrincipal{user_id:"viewer".into(),email:None,name:None,role:Role::Editor,session_version:0,totp_enabled:false,requires_2fa:false,mfa_verified:true,expires_at:i64::MAX};
    *version.lock().unwrap()=Some("2.1.1");
    let live=nwui_api::nodes::views::view(&db.state,&principal.clone().into(),"/nodes/live",&Default::default()).await.unwrap().unwrap();
    assert_eq!(live["data"]["semver"],"2.1.1");assert_eq!(live["node"]["lastVersion"],"2.1.1");assert_eq!(calls.load(Ordering::SeqCst),1);
    let context=nwui_api::nodes::views::view(&db.state,&principal.clone().into(),"/nodes/live/layout",&Default::default()).await.unwrap().unwrap();
    assert_eq!(context["node"]["lastVersion"],"2.1.1");
    assert!(db.client.query_one("SELECT last_seen_at > '2026-01-01'::timestamptz FROM nodes WHERE id='live'",&[]).await.unwrap().get::<_,bool>(0));
    let row=db.client.query_one("SELECT instance_id,gateway_token_id FROM nodes WHERE id='live'",&[]).await.unwrap();assert_eq!(row.get::<_,String>(0),"bound");assert_eq!(row.get::<_,String>(1),"original-proof");
    db.client.execute("UPDATE nodes SET instance_id='' WHERE id='live'",&[]).await.unwrap();
    let live=nwui_api::nodes::views::view(&db.state,&principal.clone().into(),"/nodes/live",&Default::default()).await.unwrap().unwrap();assert_eq!(live["ok"],true);
    let row=db.client.query_one("SELECT instance_id,gateway_token_id FROM nodes WHERE id='live'",&[]).await.unwrap();assert_eq!(row.get::<_,String>(0),"");assert_eq!(row.get::<_,String>(1),"original-proof");
    db.client.batch_execute("CREATE FUNCTION reject_node_observation() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected observation write failure'; END $$;CREATE TRIGGER reject_node_observation BEFORE UPDATE ON nodes FOR EACH ROW EXECUTE FUNCTION reject_node_observation()").await.unwrap();
    *version.lock().unwrap()=Some("2.1.2");
    let observed=nwui_api::nodes::views::view(&db.state,&principal.clone().into(),"/nodes/live",&Default::default()).await.unwrap().unwrap();
    assert_eq!(observed["ok"],true,"metadata write failure must not hide a successful node read");
    assert_eq!(observed["node"]["lastVersion"],"2.1.2");
    assert_eq!(service::get(&db.state,"live").await.unwrap().last_version.as_deref(),Some("2.1.1"));
    db.client.batch_execute("DROP TRIGGER reject_node_observation ON nodes;DROP FUNCTION reject_node_observation()").await.unwrap();
    *version.lock().unwrap()=None;
    let live=nwui_api::nodes::views::view(&db.state,&principal.clone().into(),"/nodes/live",&Default::default()).await.unwrap().unwrap();assert_eq!(live["ok"],true);
    assert_eq!(service::get(&db.state,"live").await.unwrap().last_version.as_deref(),Some("2.1.1"));assert_eq!(calls.load(Ordering::SeqCst),4);
    rejected.store(true,Ordering::SeqCst);*version.lock().unwrap()=Some("2.0.0");
    let failure=nwui_api::nodes::views::view(&db.state,&principal.clone().into(),"/nodes/live",&Default::default()).await.unwrap().unwrap();assert_eq!(failure["ok"],false);
    assert_eq!(service::get(&db.state,"live").await.unwrap().last_version.as_deref(),Some("2.1.1"));
    let partial=nwui_api::nodes::views::view(&db.state,&principal.clone().into(),"/nodes/live/reverse-proxy",&Default::default()).await.unwrap().unwrap();assert_eq!(partial["ok"],false);
    assert_eq!(service::get(&db.state,"live").await.unwrap().last_version.as_deref(),Some("2.1.1"),"partial failures must not lower the stored version");
    assert_eq!(calls.load(Ordering::SeqCst),8,"the existing RP view has three resource requests");
    server.abort();server.await.unwrap_err();
    let closed=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let unavailable=closed.local_addr().unwrap();drop(closed);
    db.client.execute("UPDATE nodes SET base_url=$1 WHERE id='live'",&[&format!("http://{unavailable}")]).await.unwrap();
    let started=std::time::Instant::now();
    let failed=nwui_api::nodes::views::view(&db.state,&principal.clone().into(),"/nodes/live",&Default::default()).await.unwrap().unwrap();assert_eq!(failed["ok"],false);
    assert_eq!(service::get(&db.state,"live").await.unwrap().last_version.as_deref(),Some("2.1.1"));
    println!("STOPPED_NODE_VIEW_MS={}",started.elapsed().as_millis());
    assert_eq!(calls.load(Ordering::SeqCst),8,"unreachable nodes do not gain an info probe");
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn stopped_node_read_preserves_the_context_bar_version() {
    let closed=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=closed.local_addr().unwrap();drop(closed);
    let db=auth_support::test_db().await;let sealed=db.state.secrets.seal("paired-secret").unwrap();
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id,last_version) VALUES('stopped','node',$1,$2,'admin','bound','2.1.0')",&[&format!("http://{address}"),&sealed]).await.unwrap();
    let principal=nwui_api::auth::principal::SessionPrincipal{user_id:"viewer".into(),email:None,name:None,role:Role::Editor,session_version:0,totp_enabled:false,requires_2fa:false,mfa_verified:true,expires_at:i64::MAX};
    let started=std::time::Instant::now();
    let result=nwui_api::nodes::views::view(&db.state,&principal.into(),"/nodes/stopped",&Default::default()).await.unwrap().unwrap();
    println!("STOPPED_NODE_READ_MS={}",started.elapsed().as_millis());
    assert_eq!(result["ok"],false);assert_eq!(result["node"]["lastVersion"],"2.1.0");
    assert_eq!(service::get(&db.state,"stopped").await.unwrap().last_version.as_deref(),Some("2.1.0"));
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn pairing_sends_one_current_version_and_rejects_cross_major_before_auth() {
    use axum::{
        Json, Router, body::{Body, to_bytes}, extract::ConnectInfo,
        http::{HeaderMap, Request, StatusCode}, routing::post,
    };
    use nwui_api::{contracts::Actor, nodes::client::NodeHttp, state::AppState};
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    use tower::ServiceExt;

    let db = auth_support::test_db().await;
    let mut gateway = db.state.clone();
    Arc::make_mut(&mut gateway.config).version = "2.2.0".into();
    gateway.node_http = NodeHttp::new(&gateway.config.version, &gateway.config).unwrap();
    let actor = Actor { via: None, id: None, label: Some("test".into()), ip: None };
    for version in ["2.0.3", "2.1.2", "2.2.0"] {
        let posts = Arc::new(AtomicUsize::new(0));
        let hits = posts.clone();
        let remote = Router::new().route("/api/v1/pair", post(move |headers: HeaderMap| {
            let hits = hits.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                assert_eq!(headers.get_all("x-gateway-version").iter().collect::<Vec<_>>(), ["2.2.0"]);
                ([("x-node-version", version)], Json(json!({
                    "token": "paired-token", "role": "editor", "instanceId": version, "semver": version,
                })))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, remote).await.unwrap() });
        let id = service::pair_node(&gateway, service::NodeInput {
            name: version.into(),
            base_url,
            allow_insecure_tls: false,
            pair: Some(service::PairInput {
                token: "pairing-token".into(), gateway_name: "gateway".into(), role: Role::Editor,
            }),
        }, &actor).await.unwrap().unwrap();
        assert_eq!(posts.load(Ordering::SeqCst), 1, "{version}");
        assert_eq!(service::get(&gateway, &id).await.unwrap().last_version.as_deref(), Some(version));
        server.abort();
    }

    for (node_version, incompatible) in [
        ("1.19.11", ["2.2.0", "2.0.0"]),
        ("2.0.0", ["1.19.11", "1.19.0"]),
        ("2.2.0", ["1.19.11", "1.19.0"]),
    ] {
        let mut config = (*db.state.config).clone();
        config.version = node_version.into();
        // A compatible request reaches the token lookup and fails on this nonexistent database.
        let (server, _) = config.database_url.rsplit_once('/').unwrap();
        config.database_url = format!("{server}/nwui_nonexistent_{}", uuid::Uuid::new_v4().simple());
        let state = AppState::new(config).unwrap();
        let app = nwui_api::app::with_request_context(nwui_api::nodes::routes::router().with_state(state), None);
        for (method, path) in [("POST", "/api/v1/pair"), ("GET", "/api/v1/info")] {
            for version in incompatible.into_iter().chain([node_version]) {
                let request = Request::builder()
                    .method(method)
                    .uri(path)
                    .header("host", "admin.test")
                    .header("x-gateway-version", version)
                    .header("authorization", "Bearer gateway-token")
                    .extension(ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 40000))))
                    .body(Body::from(json!({"token":"pairing-token", "gatewayName":"gateway", "role":"editor"}).to_string()))
                    .unwrap();
                let response = app.clone().oneshot(request).await.unwrap();
                let expected = if version == node_version { StatusCode::SERVICE_UNAVAILABLE } else { StatusCode::CONFLICT };
                assert_eq!(response.status(), expected, "{version} -> {node_version} {path}");
                if expected == StatusCode::CONFLICT {
                    let body: serde_json::Value = serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
                    assert_eq!(body["error"]["code"], "version_mismatch");
                }
            }
        }
    }
}
