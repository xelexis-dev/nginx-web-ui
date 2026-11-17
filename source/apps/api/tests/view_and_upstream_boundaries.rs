mod auth_support;
use auth_support::*;
use nwui_api::{contracts::{Actor, Role}, control::{resources::{self, Mutation, Resource}, views}};
use serde_json::{Value, json};
use std::time::Duration;

#[test]
#[ignore = "real PostgreSQL: NWUI_TEST_DATABASE_URL"]
fn w13_routes_get_preview_does_not_starve_single_connection_pool() {
    use axum::{body::Body, extract::ConnectInfo, http::Request};
    use tower::ServiceExt;
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2)
        .max_blocking_threads(1).enable_all().build().unwrap();
    runtime.block_on(async {
        let db = test_db_with(&[("API_DB_POOL_SIZE", "1")]).await;
        gateway_issuer(&db.client).await;
        db.client.execute("INSERT INTO gateway_tokens(id,name,token_hash,role,created_by_user_id) VALUES('preview-reader','preview-reader',$1,'viewer','q3-gateway-issuer')", &[&nwui_api::auth::token_digest("w13-preview-gateway")]).await.unwrap();
        db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('preview','preview.test',18083,$1)", &[&json!({"proxyPass":"http://ip6-localhost:8080"})]).await.unwrap();
        let row = resources::get(&db.client,Resource::Server,"preview").await.unwrap();
        let expected = views::render_preview(&db.state.config,Resource::Server,&row,&[],&[]);
        assert!(expected["error"].is_null(),"{expected}");
        let app = nwui_api::app::with_request_context(nwui_api::control::routes::router().with_state(db.state.clone()),None);
        let request = |path: &str| Request::builder().uri(path).header("host","admin.test")
            .header("x-gateway-version",&db.state.config.version).header("authorization","Bearer w13-preview-gateway")
            .extension(ConnectInfo(std::net::SocketAddr::from(([127,0,0,1],40000))))
            .body(Body::empty()).unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        let (started, ready) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || { let _=started.send(());wait.recv().unwrap(); });
        ready.await.unwrap();
        // ip6-localhost uses real NSS; park its preview worker at the lookup boundary.
        let preview = tokio::spawn(app.clone().oneshot(request("/api/v1/servers/preview")));
        tokio::time::sleep(Duration::from_secs(1)).await;
        let pending = !preview.is_finished();
        let other = tokio::time::timeout(Duration::from_secs(2),app.oneshot(request("/api/v1/servers"))).await;
        let _=release.send(());blocker.await.unwrap();
        let response = tokio::time::timeout(Duration::from_secs(8),preview).await.unwrap().unwrap().unwrap();
        assert_eq!(response.status(),200);
        let body:Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(),1_000_000).await.unwrap()).unwrap();
        assert_eq!(body["preview"],expected,"preview bytes changed");
        assert!(pending,"preview did not reach the blocked lookup worker");
        assert!(other.is_ok(),"routes.rs GET preview starved the next authenticated request");
        assert_eq!(other.unwrap().unwrap().status(),200);
    });
}

#[test]
#[ignore = "real PostgreSQL: NWUI_TEST_DATABASE_URL"]
fn w13_view_releases_single_connection_before_optional_blocking_notices() {
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2)
        .max_blocking_threads(1).enable_all().build().unwrap();
    runtime.block_on(async {
        let db = test_db_with(&[("API_DB_POOL_SIZE", "1")]).await;
        db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('notice','notice.test',18081,$1)", &[&json!({"locations":[{"path":"/","proxyPass":"http://127.0.0.1:8080"},{"path":"/unchecked","proxyPass":"http://skip.invalid:8080"}]})]).await.unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        let (started, ready) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || { let _ = started.send(()); wait.recv().unwrap(); });
        ready.await.unwrap();
        let session = nwui_api::auth::principal::SessionPrincipal {
            user_id:"viewer".into(), email:None, name:None, role:Role::Editor,
            session_version:0, totp_enabled:false, requires_2fa:false,
            mfa_verified:true, expires_at:i64::MAX,
        };
        let state = db.state.clone();
        let task = tokio::spawn(async move { views::view(&state, &(session).clone().into(), "/reverse-proxy/notice/edit", &Default::default()).await });
        // The sole blocking worker is parked, so the view must wait at the notice boundary.
        tokio::time::sleep(Duration::from_secs(1)).await;
        let pending = !task.is_finished();
        let available = tokio::time::timeout(Duration::from_secs(1), db.state.db.get()).await;
        let _ = release.send(());
        blocker.await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap().unwrap().unwrap();
        assert!(pending, "notice did not reach the blocked worker");
        assert!(available.is_ok(), "the view held the only DB connection across its optional lookup");
        assert_eq!(result["selfProxyNotice"], true);
        assert_eq!(result["selfProxyUncheckedPaths"], json!(["/unchecked"]));
    });
}

#[tokio::test]
#[ignore = "real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn w13_upstream_edits_refuse_only_new_consuming_listener_loops_atomically() {
    let db = test_db_with(&[("NWUI_CONTAINER", "1")]).await;
    let actor = Actor { via: None, id:None, label:Some("w13".into()), ip:None };
    db.client.execute("INSERT INTO upstreams(id,name,servers) VALUES('up','backend',$1)", &[&json!(["127.0.0.1:19000"])]).await.unwrap();
    for (id, cfg) in [
        ("id-consumer", json!({"locations":[{"path":"/","upstreamId":"up"}]})),
        ("literal-consumer", json!({"proxyPass":"http://backend"})),
    ] {
        db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES($1,$2,18081,$3)", &[&id,&format!("{id}.test"),&cfg]).await.unwrap();
    }
    let before:Value = db.client.query_one("SELECT to_jsonb(u) FROM upstreams u WHERE id='up'", &[]).await.unwrap().get(0);
    let audit:i64 = db.client.query_one("SELECT count(*) FROM audit_logs", &[]).await.unwrap().get(0);
    let result = resources::mutate(&db.state,Resource::Upstream,Mutation::Update,Some("up"),json!({"name":"backend","servers":["127.0.0.1:18081"]}),&actor).await;
    assert!(result.is_err(), "upstream edit introduced a loop into both consuming servers");
    let error = result.unwrap_err();
    assert_eq!(error.status,400);
    assert!(error.field_errors.as_ref().is_some_and(|e| e.get("servers").is_some_and(|m|m.contains("consumer"))), "{error:?}");
    assert_eq!(db.client.query_one("SELECT to_jsonb(u) FROM upstreams u WHERE id='up'", &[]).await.unwrap().get::<_,Value>(0), before);
    assert_eq!(db.client.query_one("SELECT count(*) FROM audit_logs", &[]).await.unwrap().get::<_,i64>(0), audit);

    // Legacy rows may already loop: weight/other-member edits keep that same edge editable.
    db.client.execute("UPDATE upstreams SET servers=$1 WHERE id='up'", &[&json!(["127.0.0.1:18081"])]).await.unwrap();
    resources::mutate(&db.state,Resource::Upstream,Mutation::Update,Some("up"),json!({"name":"backend","servers":["127.0.0.1:18081 weight=2","10.0.0.5:19000"]}),&actor).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('another','another.test',18082,$1)", &[&json!({"locations":[{"path":"/","upstreamId":"up"}]})]).await.unwrap();
    assert!(resources::mutate(&db.state,Resource::Upstream,Mutation::Update,Some("up"),json!({"name":"backend","servers":["127.0.0.1:18081 weight=2","127.0.0.1:18082"]}),&actor).await.is_err(), "an existing loop must not excuse a new loop for a different consumer");
    resources::mutate(&db.state,Resource::Upstream,Mutation::Update,Some("up"),json!({"name":"backend","servers":["10.0.0.5:18081"]}),&actor).await.unwrap();
    db.client.execute("UPDATE upstreams SET servers=$1 WHERE id='up'", &[&json!(["169.254.169.254:18081"])]).await.unwrap();
    assert!(resources::mutate(&db.state,Resource::Upstream,Mutation::Update,Some("up"),json!({"name":"backend","servers":["127.0.0.1:18081"]}),&actor).await.is_err(), "a previous metadata refusal is not a previous self-loop");
    db.client.batch_execute("INSERT INTO certificates(id,domains,status) VALUES('tls','[\"tls.test\"]','issued'); DELETE FROM nginx_servers WHERE id='literal-consumer'").await.unwrap();
    db.client.execute("UPDATE nginx_servers SET certificate_id='tls',config=config || '{\"tlsPort\":18443}'::jsonb WHERE id='id-consumer'", &[]).await.unwrap();
    db.client.execute("UPDATE upstreams SET servers=$1 WHERE id='up'", &[&json!(["127.0.0.1:18081"])]).await.unwrap();
    assert!(resources::mutate(&db.state,Resource::Upstream,Mutation::Update,Some("up"),json!({"name":"backend","servers":["127.0.0.1:18081","127.0.0.1:18443"]}),&actor).await.is_err(), "old HTTP loop cannot excuse new TLS loop");
    resources::mutate(&db.state,Resource::Upstream,Mutation::Update,Some("up"),json!({"name":"renamed","servers":["127.0.0.1:18081 weight=3"]}),&actor).await.unwrap();
}

#[tokio::test]
#[ignore = "real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn w13_location_count_is_not_a_save_or_apply_limit() {
    let db = test_db_with(&[("NWUI_CONTAINER", "1")]).await;
    let actor = Actor { via: None, id:None, label:Some("many-locations".into()), ip:None };
    let locations: Vec<_> = (0..200).map(|i| json!({"path":format!("/route-{i}"),"mode":"direct","proxyPass":"http://192.0.2.1:19000"})).collect();
    resources::mutate(&db.state,Resource::Server,Mutation::Create,None,json!({"serverName":"many.test","listen":18089,"targetMode":"locations","locations":locations}),&actor).await.unwrap();
    let client = db.state.db.get().await.unwrap();
    let snapshot = nwui_api::control::model::ConfigSnapshotData { servers:resources::list(&**client,Resource::Server).await.unwrap(),upstreams:vec![],streams:vec![] };
    drop(client);
    assert_eq!(snapshot.servers[0]["config"]["locations"].as_array().unwrap().len(),200);
    let rendered = nwui_api::publisher::test_support::render_for_apply(&db.state.config,&snapshot,&[]);
    assert!(rendered.skipped.is_empty(),"{:?}",rendered.skipped);
    let text = String::from_utf8(rendered.rendered.conf.values().next().unwrap().clone()).unwrap();
    assert_eq!(text.matches("proxy_pass http://192.0.2.1:19000;").count(),200);
}
