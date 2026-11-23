mod auth_support;
use auth_support::*;
use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, net::SocketAddr};
use tower::ServiceExt;
fn app(db: &TestDb) -> Router {
    nwui_api::app::finish_router(
        nwui_api::app::with_request_context(
            nwui_api::observability::router().with_state(db.state.clone()),
            None,
        ),
        &db.state.config.version,
    )
}
fn request(path: &str) -> axum::http::request::Builder {
    Request::builder()
        .uri(path)
        .header("host", "admin.example.test")
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41000))))
}
async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let r = app.clone().oneshot(req).await.unwrap();
    let status = r.status();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
fn cookie() -> String {
    let claims = json!({"id":"u1","sv":3});
    let now = nwui_api::auth::principal::now_unix_ms() / 1000;
    let token = nwui_api::auth::session::encode(
        claims.as_object().unwrap(),
        SECRET,
        nwui_api::auth::session::COOKIE_NAME_PLAIN,
        now,
        3600,
    )
    .unwrap();
    format!("authjs.session-token={token}")
}
#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn version_first_gateway_session_separation_viewer_and_revocation() {
    let db = test_db().await;
    let app = app(&db);
    insert_user(
        &db.client,
        UserRow {
            role: "viewer",
            ..Default::default()
        },
        None,
    )
    .await;
    let token = "observability-synthetic-token";
    auth_support::gateway_issuer(&db.client).await;
    db.client.execute("INSERT INTO gateway_tokens(id,name,token_hash,role,created_by_user_id) VALUES ('g','gateway',$1,'viewer','q3-gateway-issuer')",&[&nwui_api::auth::token_digest(token)]).await.unwrap();
    let (status, v) = send(
        &app,
        request("/api/v1/metrics").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, 409);
    assert_eq!(v["error"]["code"], "version_mismatch");
    let (status, _) = send(
        &app,
        request("/api/ui/v1/metrics")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, 401);
    let node = || {
        request("/api/v1/metrics?range=invalid")
            .header("x-gateway-version", &db.state.config.version)
            .header("authorization", format!("Bearer {token}"))
    };
    let (status, v) = send(&app, node().body(Body::empty()).unwrap()).await;
    assert_eq!(status, 200);
    assert_eq!(v["range"], "24h");
    let session = || request("/api/ui/v1/metrics?range=1h").header("cookie", cookie());
    assert_eq!(
        send(&app, session().body(Body::empty()).unwrap()).await.0,
        200
    );
    db.client
        .execute("UPDATE users SET session_version=4 WHERE id='u1'", &[])
        .await
        .unwrap();
    assert_eq!(
        send(&app, session().body(Body::empty()).unwrap()).await.0,
        401
    );
    let req = request("/api/v1/logs/stream?file=../../secret")
        .header("x-gateway-version", &db.state.config.version)
        .header("authorization", format!("Bearer {token}"));
    assert_eq!(send(&app, req.body(Body::empty()).unwrap()).await.0, 400);
}
#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_projection_pagination_and_detail_missing() {
    let db = test_db().await;
    db.client.batch_execute("INSERT INTO audit_logs(id,action,target,actor_label,before,after) SELECT n::text,CASE WHEN n<51 THEN 'one' ELSE 'two' END,'target','gateway:test','{\"enabled\":false}','{\"enabled\":true}' FROM generate_series(1,55) n").await.unwrap();
    let q = BTreeMap::from([
        ("action".into(), " one ".into()),
        ("page".into(), "1".into()),
    ]);
    let list = nwui_api::observability::query::audit_list(&db.state, &q)
        .await
        .unwrap();
    assert_eq!(list["total"], 50);
    assert_eq!(list["items"].as_array().unwrap().len(), 50);
    assert!(list["items"][0].get("before").is_none());
    assert_eq!(list["items"][0]["actorLabel"], "gateway:test");
    let detail = nwui_api::observability::query::audit_detail(&db.state, "1")
        .await
        .unwrap();
    assert_eq!(detail["log"]["before"]["enabled"], false);
    assert!(detail["log"]["createdAt"].as_str().unwrap().ends_with('Z'));
    assert_eq!(
        nwui_api::observability::query::audit_detail(&db.state, "missing")
            .await
            .unwrap_err()
            .status,
        404
    );
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_list_metadata_matches_separate_count_and_distinct_queries() {
    use nwui_api::observability::query::audit_list;

    let db = test_db().await;
    for (seed, action, page, expected_total, expected_len) in [
        (false, None, 1_i64, 0_i64, 0),
        (true, None, 1, 55, 50),
        (false, Some(" Config:read "), 1, 51, 50),
        (false, Some("missing"), 1, 0, 0),
        (false, None, 3, 55, 0),
    ] {
        if seed {
            db.client.batch_execute(
                "INSERT INTO audit_logs (id, action)
                 SELECT n::text, CASE WHEN n <= 51 THEN 'Config:read'
                     WHEN n <= 53 THEN 'config:read' ELSE 'config-read' END
                 FROM generate_series(1, 55) n",
            ).await.unwrap();
        }
        let mut query = BTreeMap::from([("page".to_owned(), page.to_string())]);
        if let Some(action) = action {
            query.insert("action".into(), action.into());
        }
        let filter = action.map(str::trim);
        let total: i64 = db.client.query_one(
            "SELECT count(*) FROM audit_logs WHERE ($1::text IS NULL OR action=$1)",
            &[&filter],
        ).await.unwrap().get(0);
        let actions: Vec<String> = db.client.query(
            "SELECT DISTINCT action FROM audit_logs ORDER BY action",
            &[],
        ).await.unwrap().iter().map(|row| row.get(0)).collect();

        let actual = audit_list(&db.state, &query).await.unwrap();
        assert_eq!(total, expected_total);
        assert_eq!(actions.len(), if total == 0 && action.is_none() { 0 } else { 3 });
        assert_eq!(actual["items"].as_array().unwrap().len(), expected_len);
        let expected = json!({
            "items": actual["items"],
            "total": total,
            "page": page,
            "totalPages": ((total + 49) / 50).max(1),
            "actions": actions,
            "actionFilter": filter.unwrap_or("")
        });
        assert_eq!(
            serde_json::to_string(&actual).unwrap(),
            serde_json::to_string(&expected).unwrap(),
            "action={action:?}, page={page}"
        );
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_e6_s7_open_node_log_stream_ends_after_gateway_token_revocation() {
    use http_body_util::BodyExt;
    use std::time::Duration;
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("access.log");
    tokio::fs::write(&path,"synthetic line\n").await.unwrap();
    let db=test_db_with(&[("NGINX_ACCESS_LOG",path.to_str().unwrap())]).await;
    let token="audit-e6-s7-node-token";
    auth_support::gateway_issuer(&db.client).await;
    db.client.execute("INSERT INTO gateway_tokens(id,name,token_hash,role,created_by_user_id) VALUES ('s7','stream',$1,'viewer','q3-gateway-issuer')",&[&nwui_api::auth::token_digest(token)]).await.unwrap();
    let response=app(&db).oneshot(request("/api/v1/logs/stream").header("x-gateway-version",&db.state.config.version).header("authorization",format!("Bearer {token}")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::OK);let mut stream=response.into_body();
    assert!(tokio::time::timeout(Duration::from_secs(2),stream.frame()).await.unwrap().is_some(),"a valid token must really stream first");
    db.client.execute("UPDATE gateway_tokens SET revoked_at=now() WHERE id='s7'",&[]).await.unwrap();
    let ended=tokio::time::timeout(Duration::from_secs(35),async{while stream.frame().await.is_some(){}}).await.is_ok();
    drop(stream);db.state.runtime.shutdown.send_replace(true);
    assert!(ended,"an already-open node stream must stop at the next authorization check");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_actor_snapshot_survives_rename_and_deletion() {
    use nwui_api::{auth::audit::{self,AuditEntry},observability::query};
    let db=test_db().await;
    insert_user(&db.client,UserRow{id:"snapshot-user",email:"before@example.test",..Default::default()},None).await;
    audit::write(&db.client,&AuditEntry{actor_id:Some("snapshot-user".into()),action:"snapshot:test".into(),..Default::default()}).await.unwrap();
    let label:Option<String>=db.client.query_one("SELECT actor_label FROM audit_logs WHERE action='snapshot:test'",&[]).await.unwrap().get(0);
    assert_eq!(label.as_deref(),Some("before@example.test"));
    db.client.execute("UPDATE users SET email='after@example.test' WHERE id='snapshot-user'",&[]).await.unwrap();
    let list=query::audit_list(&db.state,&BTreeMap::new()).await.unwrap();assert_eq!(list["items"][0]["actorEmail"],"before@example.test");
    db.client.execute("DELETE FROM users WHERE id='snapshot-user'",&[]).await.unwrap();
    let list=query::audit_list(&db.state,&BTreeMap::new()).await.unwrap();assert_eq!(list["items"][0]["actorLabel"],"before@example.test");
    let detail=query::audit_detail(&db.state,list["items"][0]["id"].as_str().unwrap()).await.unwrap();assert_eq!(detail["log"]["actorLabel"],"before@example.test");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn actor_snapshot_backfill_preserves_explicit_labels() {
    let db=test_db().await;
    insert_user(&db.client,UserRow::default(),None).await;
    db.client.batch_execute("INSERT INTO audit_logs(id,actor_id,actor_label,action) VALUES('legacy','u1',NULL,'legacy'),('explicit','u1','original-label','legacy')").await.unwrap();
    db.client.batch_execute(include_str!("../../../packages/db/drizzle/0034_audit_actor_snapshot.sql")).await.unwrap();
    let labels=db.client.query("SELECT id,actor_label FROM audit_logs ORDER BY id",&[]).await.unwrap();
    assert_eq!(labels[0].get::<_,String>(1),"original-label");assert_eq!(labels[1].get::<_,String>(1),"u1@example.test");
}

#[tokio::test]
#[ignore = "real DB"]
async fn audit_projection_carries_via_in_list_and_detail() {
    let db=test_db().await;
    let via=json!({"kind":"token","id":"id","name":"<token>"});
    db.client.execute("INSERT INTO audit_logs(id,action,actor_label,via) VALUES('via','config:write','owner@example.test',$1)",&[&via]).await.unwrap();
    let list=nwui_api::observability::query::audit_list(&db.state,&BTreeMap::new()).await.unwrap();
    assert_eq!(list["items"][0]["via"],via);
    assert_eq!(list["items"][0]["actorLabel"],"owner@example.test");
    assert_eq!(nwui_api::observability::query::audit_detail(&db.state,"via").await.unwrap()["log"]["via"],via);
}
