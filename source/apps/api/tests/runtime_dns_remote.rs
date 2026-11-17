mod outbound_support;
mod auth_support;
mod g13_support;
use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, State},
    http::{Request, StatusCode},
    response::IntoResponse,
};
use nwui_api::{auth::principal::SessionPrincipal, contracts::Role, nodes::views};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;

fn peer() -> ConnectInfo<SocketAddr> {
    ConnectInfo(([127, 0, 0, 1], 32100).into())
}
async fn body(response: axum::response::Response) -> Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap(),
    )
    .unwrap()
}
fn session() -> SessionPrincipal {
    SessionPrincipal {
        user_id: "u1".into(),
        email: None,
        name: None,
        role: Role::Editor,
        session_version: 3,
        totp_enabled: false,
        requires_2fa: false,
        mfa_verified: false,
        expires_at: i64::MAX,
    }
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn runtime_dns_route_authenticates_all_read_roles_and_matches_public_shape_without_nginx() {
    let temp = tempfile::tempdir().unwrap();
    let nginx = temp.path().join("must-not-run");
    let mut db = auth_support::test_db_with(&[("NGINX_BIN", nginx.to_str().unwrap())]).await;
    let config = Arc::get_mut(&mut db.state.config).unwrap();
    config.target_dns.descriptor_path = temp.path().join("descriptor.json");
    let descriptor = nwui_api::dns_guard::Descriptor::available(&config.target_dns);
    nwui_api::dns_guard::write_descriptor(&config.target_dns, &descriptor)
        .await
        .unwrap();
    let app = nwui_api::server::router(db.state.clone());
    for (token, expected) in [(None, 401), (Some("bad"), 401)] {
        let mut req = Request::builder()
            .uri("/api/v1/runtime-dns")
            .header("host", "ui.example.test")
            .header("x-gateway-version", &db.state.config.version)
            .extension(peer());
        if let Some(token) = token {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        assert_eq!(
            app.clone()
                .oneshot(req.body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            expected
        );
    }
    let doc: Value =
        serde_json::from_str(include_str!("../../../packages/contracts/openapi.json")).unwrap();
    let schema =
        json!({"$ref":"#/components/schemas/RuntimeDnsStatus","components":doc["components"]});
    let checker = jsonschema::draft202012::options().build(&schema).unwrap();
    for role in ["viewer", "editor", "admin"] {
        auth_support::gateway_issuer(&db.client).await;
        db.client.execute("INSERT INTO gateway_tokens(id,name,token_hash,role,created_by_user_id) VALUES($1,'Reader',$2,$3::text::role,'q3-gateway-issuer')",&[&role,&nwui_api::auth::token_digest(role),&role]).await.unwrap();
        let r = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/runtime-dns")
                    .header("host", "ui.example.test")
                    .header("x-gateway-version", &db.state.config.version)
                    .header("authorization", format!("Bearer {role}"))
                    .extension(peer())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(r.headers()["cache-control"], "no-store");
        let value = body(r).await;
        assert_eq!(value, json!({"available":true,"reason":null}));
        assert!(checker.is_valid(&value), "{value}");
    }
    std::fs::remove_file(&db.state.config.target_dns.descriptor_path).unwrap();
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/runtime-dns")
                .header("host", "ui.example.test")
                .header("x-gateway-version", &db.state.config.version)
                .header("authorization", "Bearer viewer")
                .extension(peer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let value = body(r).await;
    assert_eq!(value["available"], false);
    assert!(checker.is_valid(&value));
    let r = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/runtime-dns")
                .header("host", "ui.example.test")
                .extension(peer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), 405);
    assert!(!nginx.exists());
    assert_eq!(
        db.client
            .query_one("SELECT count(*) FROM audit_logs", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
}

#[derive(Clone)]
struct MockNode {
    mode: Arc<Mutex<&'static str>>,
    calls: Arc<Mutex<Vec<String>>>,
    barrier: Option<Arc<tokio::sync::Barrier>>,
}
async fn remote(State(mock): State<MockNode>, request: Request<Body>) -> axum::response::Response {
    let path = request.uri().path().to_owned();
    assert_eq!(request.headers()["authorization"], "Bearer remote-token");
    mock.calls.lock().unwrap().push(path.clone());
    if let Some(barrier) = &mock.barrier {
        barrier.wait().await;
    }
    let mode = *mock.mode.lock().unwrap();
    let mut status = StatusCode::OK;
    let mut version = "1.20.0";
    let mut value = json!({"resourcePath":path});
    if path == "/api/v1/runtime-dns" {
        value = match mode {
            "unavailable" => json!({"available":false,"reason":"target-only sentinel"}),
            "error" => {
                status = StatusCode::SERVICE_UNAVAILABLE;
                json!({"error":{"message":"unavailable"}})
            }
            "old" => {
                status = StatusCode::NOT_FOUND;
                version = "1.19.11";
                json!({})
            }
            "old-success" => {
                version = "1.19.11";
                json!({"available":true,"reason":null})
            }
            "malformed" => json!({"available":"true","reason":null}),
            "timeout" => {
                tokio::time::sleep(Duration::from_secs(30)).await;
                json!({"available":true,"reason":null})
            }
            _ => json!({"available":true,"reason":null}),
        };
    }
    (status, [("x-node-version", version)], Json(value)).into_response()
}
async fn fixture(
    barrier: Option<Arc<tokio::sync::Barrier>>,
) -> (auth_support::TestDb, MockNode, tokio::task::JoinHandle<()>) {
    let mock = MockNode {
        mode: Arc::new(Mutex::new("success")),
        calls: Default::default(),
        barrier,
    };
    let routes = Router::new().fallback(remote).with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
    let db = auth_support::test_db().await;
    let sealed = db.state.secrets.seal("remote-token").unwrap();
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id) VALUES('n','Target',$1,$2,'editor','target-instance')",&[&format!("http://{addr}"),&sealed]).await.unwrap();
    // This fixture measures DNS fanout, with a known version matching its mock.
    db.client.execute("UPDATE nodes SET last_version='1.20.0' WHERE id='n'",&[]).await.unwrap();
    (db, mock, task)
}
async fn view(db: &auth_support::TestDb, path: &str) -> Value {
    views::view(&db.state, &(session()).clone().into(), path, &BTreeMap::new())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn runtime_dns_is_outer_target_status_on_every_create_edit_detail_with_exactly_one_extra_read()
 {
    let (db, mock, task) = fixture(None).await;
    // Gateway has no descriptor and is unavailable; target status must win.
    assert!(!nwui_api::dns_guard::status(&db.state.config).available);
    for (path, existing) in [
        ("/nodes/n/reverse-proxy/new", 2),
        ("/nodes/n/reverse-proxy/s", 3),
        ("/nodes/n/reverse-proxy/s/edit", 3),
        ("/nodes/n/reverse-proxy/upstreams/new", 0),
        ("/nodes/n/reverse-proxy/upstreams/u", 1),
        ("/nodes/n/reverse-proxy/upstreams/u/edit", 1),
        ("/nodes/n/stream/new", 0),
        ("/nodes/n/stream/s", 1),
        ("/nodes/n/stream/s/edit", 1),
    ] {
        mock.calls.lock().unwrap().clear();
        let value = view(&db, path).await;
        assert_eq!(value["ok"], true, "{path}: {value}");
        assert_eq!(
            value["runtimeDns"],
            json!({"available":true,"reason":null}),
            "{path}: {value}"
        );
        assert!(value["data"].get("runtimeDns").is_none());
        if existing == 0 {
            assert!(value["data"].is_null());
        }
        let calls = mock.calls.lock().unwrap();
        assert_eq!(
            calls.iter().filter(|p| *p == "/api/v1/runtime-dns").count(),
            1,
            "{path}"
        );
        assert_eq!(calls.len(), existing + 1, "{path}: {calls:?}");
        assert!(!calls.iter().any(|p| p == "/api/v1/status"));
    }
    task.abort();
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn optional_runtime_dns_failures_old_nodes_and_malformed_responses_never_fail_the_page() {
    let (db, mock, task) = fixture(None).await;
    for mode in ["unavailable", "error", "old", "old-success", "malformed"] {
        *mock.mode.lock().unwrap() = mode;
        let value = view(&db, "/nodes/n/stream/new").await;
        assert_eq!(value["ok"], true, "{mode}: {value}");
        if mode == "unavailable" {
            assert_eq!(
                value["runtimeDns"],
                json!({"available":false,"reason":"target-only sentinel"})
            );
        } else {
            assert!(value.get("runtimeDns").is_none(), "{mode}: {value}");
        }
        assert!(value["data"].is_null());
    }
    task.abort();
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn optional_runtime_dns_timeout_is_bounded_and_omitted() {
    let (db, mock, task) = fixture(None).await;
    *mock.mode.lock().unwrap() = "timeout";
    let value = tokio::time::timeout(Duration::from_secs(12), view(&db, "/nodes/n/stream/new"))
        .await
        .expect("bounded optional read");
    assert_eq!(value["ok"], true);
    assert!(value.get("runtimeDns").is_none());
    assert_eq!(mock.calls.lock().unwrap().len(), 1);
    task.abort();
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn runtime_dns_and_existing_three_reads_are_polled_in_parallel() {
    let (db, _mock, task) = fixture(Some(Arc::new(tokio::sync::Barrier::new(4)))).await;
    let value = tokio::time::timeout(
        Duration::from_secs(2),
        view(&db, "/nodes/n/reverse-proxy/s/edit"),
    )
    .await
    .expect("all four HTTP reads must arrive before any completes");
    assert_eq!(value["ok"], true);
    assert_eq!(value["runtimeDns"]["available"], true);
    task.abort();
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn runtime_dns_is_not_requested_before_authorization_or_for_unrelated_views() {
    let (db, mock, task) = fixture(None).await;
    let mut viewer = session();
    viewer.role = Role::Viewer;
    let value = views::view(&db.state, &(viewer).clone().into(), "/nodes/n/stream/new", &BTreeMap::new())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(value["ok"], false);
    assert!(mock.calls.lock().unwrap().is_empty());
    let value = view(&db, "/nodes/n/certificates/new").await;
    assert_eq!(value["ok"], true);
    assert!(value.get("runtimeDns").is_none());
    assert!(mock.calls.lock().unwrap().is_empty());
    task.abort();
}

// A.33 retires the API-owned reapply_on_boot notification producer. There is no
// I2 boot.config-apply-failed event to simulate through the publisher socket.
