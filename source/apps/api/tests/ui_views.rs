mod auth_support;
use auth_support::*;
use axum::{body::Body, extract::ConnectInfo, http::{Request, StatusCode}};
use nwui_api::auth::{principal::now_unix_ms, session};
use serde_json::json;
use tower::ServiceExt;

async fn request(db: &TestDb, path: &str, claims: Option<serde_json::Value>) -> axum::response::Response {
    let mut req = Request::builder().uri(path).header("host", "admin.example.test")
        .extension(ConnectInfo("127.0.0.1:45000".parse::<std::net::SocketAddr>().unwrap()));
    if let Some(claims) = claims {
        let token = session::encode(claims.as_object().unwrap(), SECRET, session::COOKIE_NAME_PLAIN, now_unix_ms()/1000, 3600).unwrap();
        req = req.header("cookie", format!("{}={token}", session::COOKIE_NAME_PLAIN));
    }
    nwui_api::server::router(db.state.clone()).oneshot(req.body(Body::empty()).unwrap()).await.unwrap()
}

#[tokio::test]
#[ignore = "real DB: NWUI_TEST_DATABASE_URL"]
async fn views_require_fresh_session_and_mfa_before_dispatch() {
    let db = test_db().await;
    insert_user(&db.client, UserRow::default(), None).await;
    let path = "/api/ui/v1/views?path=%2Freverse-proxy%2Fupstreams";
    assert_eq!(request(&db,path,None).await.status(),StatusCode::UNAUTHORIZED);
    let claims = json!({"id":"u1","sv":3,"requires2fa":true});
    assert_eq!(request(&db,path,Some(claims)).await.status(),StatusCode::FORBIDDEN);
    let claims = json!({"id":"u1","sv":3});
    let reply = request(&db,path,Some(claims.clone())).await;
    assert_eq!(reply.status(),StatusCode::OK);
    assert!(!reply.headers().contains_key("set-cookie"));
    let body: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(reply.into_body(),4096).await.unwrap()).unwrap();
    assert_eq!(body["upstreams"],json!([]));
    assert_eq!(body["canWrite"],true);
    db.client.execute("UPDATE users SET session_version=4", &[]).await.unwrap();
    assert_eq!(request(&db,path,Some(claims)).await.status(),StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "real DB: NWUI_TEST_DATABASE_URL"]
async fn malformed_unknown_missing_and_read_only_views_are_distinct() {
    let db = test_db().await;
    insert_user(&db.client, UserRow{role:"viewer",..UserRow::default()}, None).await;
    let claims = json!({"id":"u1","sv":3});
    for (path, status) in [
        ("/api/ui/v1/views",400),
        ("/api/ui/v1/views?path=https%3A%2F%2Fevil.test",400),
        ("/api/ui/v1/views?path=%2Funknown",404),
        ("/api/ui/v1/views?path=%2Freverse-proxy%2Fupstreams%2Fmissing",404),
        ("/api/ui/v1/views?path=%2Freverse-proxy%2Fupstreams%2Fnew",403),
    ] { assert_eq!(request(&db,path,Some(claims.clone())).await.status().as_u16(),status,"{path}"); }
}

#[tokio::test]
#[ignore = "real PostgreSQL and a mock paired node"]
async fn paired_node_history_view_preserves_snapshot_and_comparison_states() {
    use axum::{Json, Router, extract::State, http::Uri, response::IntoResponse};
    use serde_json::Value;
    use std::{collections::{BTreeMap, VecDeque}, sync::{Arc, Mutex}, time::{Duration, Instant}};

    #[derive(Default)]
    struct Remote {
        calls: Vec<String>,
        responses: BTreeMap<String, Value>,
        next: BTreeMap<String, VecDeque<Option<Value>>>,
        delays: BTreeMap<String, Duration>,
    }
    let remote = Arc::new(Mutex::new(Remote::default()));
    let app = Router::new().fallback(
        |State(remote): State<Arc<Mutex<Remote>>>, uri: Uri| async move {
            let (response, delay) = {
                let mut remote = remote.lock().unwrap();
                let path = uri.path();
                remote.calls.push(path.to_owned());
                let next = remote.next.get_mut(path).and_then(VecDeque::pop_front);
                let response = next.unwrap_or_else(|| remote.responses.get(path).cloned());
                (response, remote.delays.get(path).copied())
            };
            if let Some(delay) = delay { tokio::time::sleep(delay).await; }
            let headers = [("x-node-version", "2.0.3")];
            match response {
                Some(value) => (headers, Json(value)).into_response(),
                None => (StatusCode::NOT_FOUND, headers, Json(json!({"error":{"code":"not_found"}}))).into_response(),
            }
        }
    ).with_state(remote.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let db = test_db().await;
    insert_user(&db.client, UserRow::default(), None).await;
    let sealed = db.state.secrets.seal("fixture-node-token").unwrap();
    db.client.execute(
        "INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id,last_version)
         VALUES('n','Target',$1,$2,'editor','target-instance','2.0.3')",
        &[&address, &sealed],
    ).await.unwrap();
    let snapshot = |id, hour, address| json!({"snapshot":{
        "id":id,"createdAt":format!("2026-11-21T{hour:02}:00:00Z"),"label":"fixture",
        "actorId":"private-node-user",
        "data":{"upstreams":[{"id":"up","name":"backend","servers":[address],"version":"1"}],
                "servers":[],"streams":[]}
    }});
    let previous = snapshot("previous", 0, "192.0.2.1:8080");
    let current = snapshot("current", 1, "192.0.2.2:8080");
    let newer = snapshot("newer", 2, "192.0.2.3:8080");
    for row in [&previous, &current, &newer] {
        let row = &row["snapshot"];
        db.client.execute(
            "INSERT INTO config_snapshots(id,created_at,data,label) VALUES($1,$2::text::timestamptz,$3,'fixture')",
            &[&row["id"].as_str().unwrap(), &row["createdAt"].as_str().unwrap(), &row["data"]],
        ).await.unwrap();
    }
    let list_path = "/api/v1/history";
    let current_path = "/api/v1/history/current";
    let previous_path = "/api/v1/history/previous";
    let newer_path = "/api/v1/history/newer";
    let set_list = |ids: &[&str]| {
        remote.lock().unwrap().responses.insert(list_path.into(),
            json!({"snapshots":ids.iter().map(|id| json!({"id":id})).collect::<Vec<_>>()}));
    };
    remote.lock().unwrap().responses.extend([
        (current_path.into(), current.clone()),
        (previous_path.into(), previous.clone()),
        (newer_path.into(), newer),
    ]);

    let get = |path: &str| {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("path", path);
        let uri = format!("/api/ui/v1/views?{}", query.finish());
        let db = &db;
        async move {
            let r = request(db, &uri, Some(json!({"id":"u1","sv":3}))).await;
            assert_eq!(r.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(r.into_body(), 2 << 20).await.unwrap();
            serde_json::from_slice::<Value>(&bytes).unwrap()
        }
    };
    let path = "/nodes/n/history/current";
    set_list(&["newer", "current", "previous"]);
    assert_eq!(get("/nodes/n/history").await["data"]["snapshots"].as_array().unwrap().len(), 3);
    assert_eq!(std::mem::take(&mut remote.lock().unwrap().calls), [list_path]);

    let local = get("/history/current").await;
    let node = get(path).await;
    let calls = std::mem::take(&mut remote.lock().unwrap().calls);
    assert_eq!(calls.len(), 3);
    assert!(calls[..2].contains(&current_path.to_owned()) && calls[..2].contains(&list_path.to_owned()));
    assert_eq!(calls[2], previous_path);
    assert_eq!(node["data"]["comparison"], "compared");
    assert_eq!(node["data"]["canRollback"], true);
    assert_eq!(node["data"]["diff"], local["diff"]);
    assert_eq!(node["data"]["diff"]["hasChanges"], true);
    assert_eq!(node["data"]["upstreams"], local["upstreams"]);
    assert_eq!(node["data"]["snapshot"], json!({"id":"current","label":"fixture"}));

    let doc: Value = serde_json::from_str(include_str!("../../../packages/contracts/openapi.json")).unwrap();
    let schema = json!({"$ref":"#/components/schemas/NodeHistoryDetailView","components":doc["components"]});
    let checker = jsonschema::draft202012::options().build(&schema).unwrap();
    assert!(checker.is_valid(&node), "{node}");

    let mut same = previous.clone();
    same["snapshot"]["data"] = current["snapshot"]["data"].clone();
    remote.lock().unwrap().responses.insert(previous_path.into(), same);
    assert_eq!(get(path).await["data"]["diff"]["hasChanges"], false);

    remote.lock().unwrap().responses.remove(previous_path);
    let missing_previous = get(path).await;
    assert_eq!(missing_previous["data"]["comparison"], "previous_deleted");
    assert_eq!(missing_previous["data"]["canRollback"], true);

    let newer_ids = (0..99).map(|i| format!("newer-{i}")).collect::<Vec<_>>();
    let mut ids = newer_ids.iter().map(String::as_str).collect::<Vec<_>>();
    ids.push("current");
    set_list(&ids);
    assert_eq!(get(path).await["data"]["comparison"], "outside_recent");
    set_list(&ids[..99]);
    let unexpected = "노드가 예상과 다른 응답을 보냈습니다.";
    let inconsistent = get(path).await;
    assert_eq!(inconsistent["nodeError"]["nodeErrorTitle"], unexpected);
    assert!(checker.is_valid(&inconsistent), "{inconsistent}");

    remote.lock().unwrap().next.insert(current_path.into(), [Some(current.clone()), None].into());
    let removed = get(path).await;
    assert_eq!(removed["nodeError"]["failure"]["kind"], "not_found");
    assert_eq!(removed["nodeError"]["subject"], "그 스냅샷을");

    remote.lock().unwrap().next.insert(list_path.into(), [
        Some(json!({"snapshots":[{"id":"newer"}]})),
        Some(json!({"snapshots":[{"id":"current"},{"id":"previous"}]})),
    ].into());
    remote.lock().unwrap().responses.insert(previous_path.into(), previous.clone());
    let reconciled = get(path).await;
    assert_eq!(reconciled["data"]["diff"], local["diff"]);
    assert!(checker.is_valid(&reconciled), "{reconciled}");

    set_list(&["current", "previous"]);
    for (snapshot_path, valid) in [(current_path, &current), (previous_path, &previous)] {
        for data in [Value::Null, json!({"servers":[null]}), json!({"upstreams":[{"id":"bad"}]})] {
            let mut invalid = valid.clone();
            invalid["snapshot"]["data"] = data;
            remote.lock().unwrap().responses.insert(snapshot_path.into(), invalid);
            let result = get(path).await;
            assert_eq!(result["nodeError"]["nodeErrorTitle"], unexpected);
            assert!(checker.is_valid(&result), "{result}");
        }
        remote.lock().unwrap().responses.insert(snapshot_path.into(), valid.clone());
    }
    let mut historical = previous.clone();
    historical["snapshot"]["data"].as_object_mut().unwrap().remove("streams");
    historical["snapshot"]["data"]["upstreams"][0].as_object_mut().unwrap().remove("version");
    remote.lock().unwrap().responses.insert(previous_path.into(), historical);
    let compared = get(path).await;
    assert_eq!(compared["data"]["diff"], local["diff"]);
    assert!(checker.is_valid(&compared), "{compared}");

    set_list(&["newer", "current"]);
    assert_eq!(get(path).await["data"]["comparison"], "first");

    db.client.execute("UPDATE nodes SET token_role='viewer' WHERE id='n'", &[]).await.unwrap();
    assert_eq!(get(path).await["data"]["canRollback"], false);

    db.client.batch_execute("BEGIN; SELECT id FROM nodes WHERE id='n' FOR NO KEY UPDATE").await.unwrap();
    let held = tokio::time::timeout(Duration::from_secs(1), get(path)).await;
    db.client.batch_execute("ROLLBACK").await.unwrap();
    assert_eq!(held.expect("metadata row must not block the detail")["ok"], true);

    let info_path = "/api/v1/info";
    db.client.execute("UPDATE nodes SET last_version=NULL WHERE id='n'", &[]).await.unwrap();
    {
        let mut remote = remote.lock().unwrap();
        remote.calls.clear();
        remote.responses.insert(info_path.into(), json!({"instanceId":"different-instance","semver":"2.0.3"}));
    }
    for path in ["/nodes/n/history/current", "/nodes/n/history"] {
        let failure = get(path).await;
        assert_eq!(failure["nodeError"]["nodeErrorTitle"], "노드의 버전 또는 식별 정보를 확인할 수 없습니다.");
        assert_eq!(std::mem::take(&mut remote.lock().unwrap().calls), [info_path]);
    }
    let version: Option<String> = db.client.query_one("SELECT last_version FROM nodes WHERE id='n'", &[]).await.unwrap().get(0);
    assert!(version.is_none());

    {
        let mut remote = remote.lock().unwrap();
        remote.responses.insert(info_path.into(), json!({"instanceId":"target-instance","semver":"2.0.3"}));
        remote.delays.extend([
            (info_path.into(), Duration::from_secs(2)),
            (current_path.into(), Duration::from_millis(9500)),
        ]);
    }
    let started = Instant::now();
    let timed_out = get(path).await;
    let elapsed = started.elapsed();
    assert_eq!(timed_out["nodeError"]["failure"]["kind"], "unreachable");
    assert!((Duration::from_secs(9)..Duration::from_secs(11)).contains(&elapsed), "{elapsed:?}");
    println!("node history shared deadline {elapsed:?}");
    remote.lock().unwrap().delays.clear();
    db.client.execute("UPDATE nodes SET last_version='2.0.3' WHERE id='n'", &[]).await.unwrap();

    let mut large = current.clone();
    large["snapshot"]["data"]["padding"] = json!("x".repeat(1024 * 1024));
    remote.lock().unwrap().responses.insert(current_path.into(), large);
    let oversize = get(path).await;
    assert_eq!(oversize["nodeError"]["nodeErrorTitle"], "스냅샷 응답이 1 MiB를 초과해 열 수 없습니다.");
    assert!(checker.is_valid(&oversize), "{oversize}");

    remote.lock().unwrap().responses.remove(current_path);
    let gone = get(path).await;
    assert_eq!(gone["nodeError"]["failure"]["kind"], "not_found");
    assert_eq!(gone["nodeError"]["subject"], "그 스냅샷을");
    assert_eq!(gone["nodeError"]["listHref"], "/nodes/n/history");
    assert!(checker.is_valid(&gone), "{gone}");

    server.abort();
    let _ = server.await;
    let unreachable = get(path).await;
    assert_eq!(unreachable["nodeError"]["failure"]["kind"], "unreachable");
    assert_eq!(unreachable["nodeError"]["retryHref"], path);
}
