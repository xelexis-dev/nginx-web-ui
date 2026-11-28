use nwui_api::control::{model::ConfigSnapshotData, pending};
use serde_json::{Value, json};

fn snapshot(value: Value) -> ConfigSnapshotData {
    serde_json::from_value(value).unwrap()
}

#[test]
fn pending_without_baseline_lists_every_row_but_first_history_has_no_diff() {
    let current = snapshot(
        json!({"upstreams":[{"id":"u","name":"backend","servers":[]}],"servers":[{"id":"s","serverName":"example.test","listen":80}],"streams":[]}),
    );
    assert!(!pending::diff_config(None, &current).has_changes);
    let diff = pending::pending_diff(None, &current);
    assert!(diff.has_changes);
    assert_eq!(
        diff.upstreams.len() + diff.servers.len() + diff.streams.len(),
        2
    );
    assert_eq!(
        serde_json::to_value(&diff.servers[0]).unwrap()["status"],
        "added"
    );
}

#[test]
fn diff_preserves_field_scope_array_order_and_certificate_references() {
    let before = snapshot(
        json!({"upstreams":[{"id":"u","name":"backend","servers":["a:80","b:80"]}],"servers":[{"id":"s","serverName":"example.test","listen":80,"certificateId":null,"config":{"a":1},"enabled":true}],"streams":[]}),
    );
    let after = snapshot(
        json!({"upstreams":[{"id":"u","name":"backend","servers":["b:80","a:80"]}],"servers":[{"id":"s","serverName":"example.test","listen":80.0,"certificateId":"cert","config":{"a":1.0},"enabled":true,"ignored":"metadata"}],"streams":[]}),
    );
    let diff = pending::diff_config(Some(&before), &after);
    assert_eq!(diff.servers.len(), 1);
    assert_eq!(diff.servers[0].changes.len(), 1);
    assert_eq!(diff.servers[0].changes[0].field, "certificateId");
    assert_eq!(diff.upstreams[0].changes[0].field, "servers");
}

#[test]
fn diff_distinguishes_absent_from_null_and_orders_added_changed_removed() {
    let before = snapshot(
        json!({"servers":[{"id":"removed","serverName":"a","listen":80},{"id":"changed","serverName":"b","listen":80}],"streams":[],"upstreams":[]}),
    );
    let after = snapshot(
        json!({"servers":[{"id":"changed","serverName":"b","listen":80,"certificateId":null},{"id":"added","serverName":"z","listen":80}],"streams":[],"upstreams":[]}),
    );
    let diff = pending::diff_config(Some(&before), &after);
    assert_eq!(
        diff.servers
            .iter()
            .map(|d| d.key.as_str())
            .collect::<Vec<_>>(),
        ["added", "changed", "removed"]
    );
    let change = serde_json::to_value(&diff.servers[1].changes[0]).unwrap();
    assert!(change.get("before").is_none());
    assert_eq!(change.get("after"), Some(&Value::Null));
}

#[test]
fn ascii_labels_keep_locale_order_and_lowercase_ties() {
    let current = snapshot(
        json!({"upstreams":[{"id":"1","name":"A"},{"id":"2","name":"_"},{"id":"3","name":"a"},{"id":"4","name":"B"},{"id":"5","name":"b"}],"servers":[],"streams":[]}),
    );
    let diff = pending::pending_diff(None, &current);
    assert_eq!(
        diff.upstreams
            .iter()
            .map(|i| i.label.as_str())
            .collect::<Vec<_>>(),
        ["_", "a", "A", "b", "B"]
    );
}

#[path = "auth_support/mod.rs"]
mod auth_support;

#[cfg(unix)]
mod live {
    use super::auth_support::{self, TestDb, UserRow};
    use super::*;
    use axum::{
        Router,
        body::Body,
        extract::ConnectInfo,
        http::{Request, StatusCode},
    };
    use nwui_api::{
        auth::{principal::SessionPrincipal, session, token_digest},
        contracts::Role,
        control::operations,
    };
    use std::{net::SocketAddr, os::unix::fs::PermissionsExt};
    use tempfile::TempDir;
    use tower::ServiceExt;

    async fn db(temp: &TempDir) -> TestDb {
        let bin = temp.path().join("nginx-fake");
        std::fs::write(&bin, "#!/bin/sh\necho valid\nexit 0\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        let paths = [
            bin.display().to_string(),
            temp.path().join("conf").display().to_string(),
            temp.path().join("stream").display().to_string(),
            temp.path().join("certs").display().to_string(),
        ];
        let mut db=auth_support::test_db_with(&[
            ("NGINX_BIN", &paths[0]),
            ("NGINX_CONF_D", &paths[1]),
            ("NGINX_STREAM_D", &paths[2]),
            ("CERTS_DIR", &paths[3]),
        ])
        .await;
        auth_support::nginx::witness(&mut db.state);
        db
    }
    fn app(db: &TestDb) -> Router {
        nwui_api::app::with_request_context(operations::router().with_state(db.state.clone()), None)
    }
    fn request(method: &str, path: &str) -> axum::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(path)
            .header("host", "admin.example.test")
            .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))))
    }
    fn node(db: &TestDb, method: &str, path: &str, token: &str) -> Request<Body> {
        request(method, path)
            .header("x-gateway-version", &db.state.config.version)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }
    async fn response(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
        let r = app.clone().oneshot(request).await.unwrap();
        let status = r.status();
        let bytes = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
        (
            status,
            if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            },
        )
    }
    async fn gateway(db: &TestDb, id: &str, role: &str) {
        auth_support::gateway_issuer(&db.client).await;
        db.client.execute("INSERT INTO gateway_tokens(id,name,token_hash,role,created_by_user_id) VALUES($1,$1,$2,$3::text::role,'q3-gateway-issuer')",&[&id,&token_digest(id),&role]).await.unwrap();
    }
    async fn server(db: &TestDb) {
        db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('ba33a41d-393c-44fd-83ae-b844491d77a1','example.test',8081,$1)",&[&json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
    }
    fn principal(role: Role) -> SessionPrincipal {
        SessionPrincipal {
            user_id: "u1".into(),
            email: None,
            name: None,
            role,
            session_version: 3,
            totp_enabled: false,
            requires_2fa: false,
            mfa_verified: true,
            expires_at: i64::MAX,
        }
    }
    fn cookie() -> String {
        let now = nwui_api::auth::principal::now_unix_ms() / 1000;
        let claims = json!({"id":"u1","sub":"u1","sv":3,"requires2fa":false});
        let token = session::encode(
            claims.as_object().unwrap(),
            auth_support::SECRET,
            session::COOKIE_NAME_PLAIN,
            now,
            3600,
        )
        .unwrap();
        format!("authjs.session-token={token}")
    }

    #[tokio::test]
    #[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
    async fn node_pending_apply_history_and_failed_rollback_keep_contracts() {
        let temp = TempDir::new().unwrap();
        let db = db(&temp).await;
        gateway(&db, "editor", "editor").await;
        gateway(&db, "viewer", "viewer").await;
        server(&db).await;
        let app = app(&db);
        let (status, pending) = response(&app, node(&db, "GET", "/api/v1/pending", "viewer")).await;
        assert_eq!(status, 200);
        assert_eq!(pending["count"], 1);
        assert_eq!(pending["hasBaseline"], false);
        assert_eq!(
            response(&app, node(&db, "POST", "/api/v1/apply", "viewer"))
                .await
                .0,
            403
        );
        assert_eq!(
            response(&app, node(&db, "GET", "/api/v1/apply", "editor"))
                .await
                .0,
            405
        );
        assert_eq!(
            response(&app, node(&db, "POST", "/api/v1/apply", "editor"))
                .await
                .0,
            200
        );
        let (_, pending) = response(&app, node(&db, "GET", "/api/v1/pending", "viewer")).await;
        assert_eq!(pending["count"], 0);
        assert_eq!(pending["hasBaseline"], true);
        let (_, history) = response(&app, node(&db, "GET", "/api/v1/history", "viewer")).await;
        let row = &history["snapshots"][0];
        assert!(row.get("data").is_none());
        assert!(row["createdAt"].as_str().unwrap().ends_with('Z'));
        let id = row["id"].as_str().unwrap();
        let (_, detail) = response(
            &app,
            node(&db, "GET", &format!("/api/v1/history/{id}"), "viewer"),
        )
        .await;
        assert_eq!(
            detail["snapshot"]["data"]["servers"][0]["serverName"],
            "example.test"
        );
        assert_eq!(
            response(
                &app,
                node(&db, "POST", "/api/v1/history/missing/rollback", "editor")
            )
            .await
            .0,
            404
        );
        db.client
            .execute("UPDATE nginx_servers SET listen=8082", &[])
            .await
            .unwrap();
        let fault=temp.path().join("fail-validation-once");
        // Keep the witness and permit validation/reloads of the full previous payload.
        std::fs::write(temp.path().join("nginx-fake"),format!("#!/bin/sh\nif [ -f '{}' ]; then rm '{}'; echo invalid >&2; exit 1; fi\nif [ \"$1\" = -T ]; then printf '# configuration file %s:\\n' '{}'; fi\necho valid\n",fault.display(),fault.display(),db.state.config.nginx_main_conf.display())).unwrap();
        std::fs::write(&fault,"armed").unwrap();
        let (status, failure) = response(&app, node(&db, "POST", "/api/v1/apply", "editor")).await;
        assert_eq!(status, 422);
        assert_eq!(failure["error"]["code"], "apply_failed");
        std::fs::write(&fault,"armed").unwrap();
        let (status, failure) = response(
            &app,
            node(
                &db,
                "POST",
                &format!("/api/v1/history/{id}/rollback"),
                "editor",
            ),
        )
        .await;
        assert_eq!(status, 422);
        assert_eq!(failure["error"]["code"], "apply_failed");
        let (_, pending) = response(&app, node(&db, "GET", "/api/v1/pending", "viewer")).await;
        assert_eq!(pending["count"], 1);
        let snapshots: i64 = db
            .client
            .query_one("SELECT count(*) FROM config_snapshots", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(snapshots, 1);
    }

    #[tokio::test]
    #[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
    async fn unresolved_apply_restore_is_503_then_fences_following_operations_and_keeps_a8() {
        let temp=TempDir::new().unwrap();let db=db(&temp).await;gateway(&db,"unconfirmed-editor","editor").await;server(&db).await;
        let router=app(&db);assert_eq!(response(&router,node(&db,"POST","/api/v1/apply","unconfirmed-editor")).await.0,StatusCode::OK);
        db.client.execute("UPDATE nginx_servers SET listen=8082",&[]).await.unwrap();
        std::fs::write(temp.path().join("nginx-fake"),"#!/bin/sh\necho persistent-validation-failure >&2\nexit 1\n").unwrap();
        let(status,body)=response(&router,node(&db,"POST","/api/v1/apply","unconfirmed-editor")).await;
        assert_eq!(status,StatusCode::SERVICE_UNAVAILABLE);assert_eq!(body["error"]["code"],"apply_unconfirmed");assert!(db.state.config.nginx_conf_dir.join(".nwui-apply-uncertain").exists());
        let(status,body)=response(&router,node(&db,"POST","/api/v1/apply","unconfirmed-editor")).await;
        assert_eq!(status,StatusCode::SERVICE_UNAVAILABLE);assert_eq!(body["error"]["code"],"publisher_unavailable");assert!(db.state.config.nginx_conf_dir.join(".nwui-apply-uncertain").exists());
        assert_eq!(db.client.query_one("SELECT count(*) FROM config_snapshots",&[]).await.unwrap().get::<_,i64>(0),1);
    }

    #[tokio::test]
    #[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
    async fn archive_capacity_refusal_preserves_korean_text_in_the_operations_422() {
        use nwui_api::{publisher::protocol::{Op,OpResult},publisher_client::{Client,fake}};
        let temp=TempDir::new().unwrap();let mut db=db(&temp).await;gateway(&db,"capacity-editor","editor").await;
        let output="설정 보관 공간이 부족해 적용하지 않았습니다. 현재 설정은 그대로입니다. 디스크 공간을 확보한 뒤 다시 시도하세요.";
        let publisher=fake::Server::start([fake::Script::Reply(OpResult::Failed{code:"publisher_storage_full".into(),output:output.into()})]).await.unwrap();
        let mut runtime=nwui_api::state::RuntimeState::default();runtime.publisher=Client::socket_for_tests(publisher.path().into(),Default::default());db.state.runtime=std::sync::Arc::new(runtime);
        let (status,body)=response(&app(&db),node(&db,"POST","/api/v1/apply","capacity-editor")).await;
        assert_eq!(status,StatusCode::UNPROCESSABLE_ENTITY);assert_eq!(body["error"]["code"],"apply_failed");assert_eq!(body["error"]["message"],output);
        let requests=publisher.requests.lock().await;assert_eq!(requests.len(),1);assert!(matches!(requests[0].op,Op::ApplySaved{..}));
        assert_eq!(db.client.query_one("SELECT count(*) FROM config_snapshots",&[]).await.unwrap().get::<_,i64>(0),0);
    }

    #[tokio::test]
    #[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
    async fn history_view_uses_timestamp_and_id_predecessor_and_fifty_row_list() {
        let temp = TempDir::new().unwrap();
        let db = db(&temp).await;
        for (id, at, name) in [
            ("first", "2025-01-01T00:00:00Z", "a"),
            ("same", "2025-01-01T00:00:00Z", "b"),
            ("latest", "2025-01-02T00:00:00Z", "c"),
        ] {
            db.client.execute("INSERT INTO config_snapshots(id,created_at,label,data) VALUES($1,$2::text::timestamptz,$1,$3)",&[&id,&at,&json!({"servers":[{"id":"s","serverName":name,"listen":80}],"streams":[],"upstreams":[]})]).await.unwrap();
        }
        let query = std::collections::BTreeMap::new();
        let first = operations::view(&db.state, &(principal(Role::Viewer)).clone().into(), "/history/first", &query)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first["isFirst"], true);
        assert_eq!(first["diff"]["hasChanges"], false);
        assert_eq!(first["canRollback"], false);
        let tied = operations::view(&db.state, &(principal(Role::Viewer)).into(), "/history/same", &query)
            .await.unwrap().unwrap();
        assert_eq!(tied["isFirst"], false);
        assert_eq!(tied["diff"]["servers"][0]["changes"][0]["field"], "serverName");
        let last = operations::view(
            &db.state, &(principal(Role::Editor)).clone().into(),
            "/history/latest",
            &query,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(last["isFirst"], false);
        assert_eq!(last["canRollback"], true);
        assert_eq!(
            last["diff"]["servers"][0]["changes"][0]["field"],
            "serverName"
        );
        db.client.batch_execute("INSERT INTO config_snapshots(id,created_at,data) SELECT 'extra-'||n,'2024-01-01'::timestamptz+make_interval(secs=>n),'{}'::jsonb FROM generate_series(1,60) n").await.unwrap();
        let list = operations::view(&db.state, &(principal(Role::Viewer)).clone().into(), "/history", &query)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(list["data"]["items"].as_array().unwrap().len(), 50);
        assert_eq!(list["data"]["items"][0]["serverCount"], 1);
        assert_eq!(
            operations::view(
                &db.state, &(principal(Role::Viewer)).clone().into(),
                "/history/missing",
                &query
            )
            .await
            .unwrap_err()
            .status,
            404
        );
    }

    #[tokio::test]
    #[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
    async fn browser_commands_enforce_origin_session_and_arity() {
        let temp = TempDir::new().unwrap();
        let db = db(&temp).await;
        server(&db).await;
        auth_support::insert_user(&db.client, UserRow::default(), None).await;
        let app = app(&db);
        let form = |path: &str, body: &str, origin: bool| {
            let builder = request("POST", path)
                .header("cookie", cookie())
                .header("content-type", "application/x-www-form-urlencoded");
            let builder = if origin {
                builder.header("origin", "http://admin.example.test")
            } else {
                builder
            };
            builder.body(Body::from(body.to_owned())).unwrap()
        };
        assert_eq!(
            response(&app, form("/api/ui/v1/forms/apply.applyConfig", "", false))
                .await
                .0,
            403
        );
        // Exercise the published local apply examples through the real handler and form decoder.
        const APPLY: &str = "/api/ui/v1/forms/apply.applyConfig";
        let contract: Value =
            serde_json::from_str(include_str!("../../../packages/contracts/openapi.json")).unwrap();
        let examples = &contract["paths"][APPLY]["post"]["requestBody"]["content"]
            ["application/x-www-form-urlencoded"]["examples"];
        let encoded = |case: &str| {
            let mut body = url::form_urlencoded::Serializer::new(String::new());
            for (name, value) in examples[case]["value"].as_object().unwrap() {
                body.append_pair(name, value.as_str().unwrap());
            }
            body.finish()
        };
        let (status, error) = response(&app, form(APPLY, &encoded("invalid"), true)).await;
        assert_eq!(status, 400);
        assert_eq!(error["error"]["code"], "bad_request");
        assert_eq!(
            response(&app, form("/api/ui/v1/forms/history.rollback", "", true))
                .await
                .0,
            400
        );
        let (status, result) = response(&app, form(APPLY, &encoded("valid"), true)).await;
        let expected = &contract["paths"][APPLY]["post"]["responses"]["200"]["content"]
            ["application/json"]["examples"]["success"]["value"];
        assert_eq!(status, 200);
        assert_eq!(result["result"]["ok"], expected["result"]["ok"]);
        assert_eq!(result["refresh"], expected["refresh"]);
        assert!(result["result"]["output"].is_string());
        let (status, result) = response(
            &app,
            form(
                "/api/ui/v1/forms/history.rollback",
                "_apiArgs=%5B%22missing%22%5D",
                true,
            ),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(result["result"]["ok"], false);
        assert_eq!(result["result"]["kind"], "not_found");
        db.client
            .execute("UPDATE users SET role='viewer' WHERE id='u1'", &[])
            .await
            .unwrap();
        assert_eq!(
            response(&app, form("/api/ui/v1/forms/apply.applyConfig", "", true))
                .await
                .0,
            403
        );
        let count: i64 = db
            .client
            .query_one("SELECT count(*) FROM config_snapshots", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(count, 1);
    }
    #[tokio::test]
    #[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
    async fn audit_e6_i17_status_does_not_validate_mid_file_transaction() {
        use std::time::Duration;
        let temp=tempfile::tempdir().unwrap();let db=db(&temp).await;gateway(&db,"s17","viewer").await;
        let marker=temp.path().join("validated");
        std::fs::write(&db.state.config.nginx_bin,format!("#!/bin/sh\n: > '{}'\necho valid\n",marker.display())).unwrap();
        let lock=nwui_api::publisher::test_support::hold_files_lock_for_tests(&db.state.config).await.unwrap();
        let router=app(&db);let req=node(&db,"GET","/api/v1/status","s17");let pending=tokio::spawn(async move{router.oneshot(req).await.unwrap()});
        let ran_locked=tokio::time::timeout(Duration::from_secs(1),async{while !marker.exists(){tokio::time::sleep(Duration::from_millis(10)).await;}}).await.is_ok();
        drop(lock);let response=tokio::time::timeout(Duration::from_secs(3),pending).await.unwrap().unwrap();assert_eq!(response.status(),StatusCode::OK);assert!(marker.exists());
        assert!(!ran_locked,"nginx -t must not read a writer's intermediate configuration");
    }

    #[tokio::test]
    #[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
    async fn audit_e6_r2_status_reports_busy_before_the_gateway_timeout() {
        use std::time::Duration;
        for writer_wait in [60, 1] {
        let temp=tempfile::tempdir().unwrap();let mut db=db(&temp).await;
        std::sync::Arc::get_mut(&mut db.state.config).unwrap().nginx_lock_wait=Duration::from_secs(writer_wait);
        gateway(&db,"busy","viewer").await;
        let marker=temp.path().join("validated");
        std::fs::write(&db.state.config.nginx_bin,format!("#!/bin/sh\n: > '{}'\necho valid\n",marker.display())).unwrap();
        let lock=nwui_api::publisher::test_support::hold_files_lock_for_tests(&db.state.config).await.unwrap();
        let router=app(&db);
        let waited=tokio::time::timeout(Duration::from_secs(6),response(&router,node(&db,"GET","/api/v1/status","busy"))).await;
        let ran_locked=marker.exists();drop(lock);
        let (status,body)=waited.expect("status must report busy well before the gateway's ten-second deadline");
        assert_eq!(status,503);assert_eq!(body["error"]["code"],"nginx_busy");
        assert!(body["error"]["message"].as_str().unwrap().contains("설정 적용 중"));
        assert!(!ran_locked,"busy status must never run nginx -t against intermediate files");
        assert_eq!(response(&router,node(&db,"GET","/api/v1/status","busy")).await.0,200);
        assert!(marker.exists());
        assert_eq!(db.state.config.nginx_lock_wait,Duration::from_secs(writer_wait),"status must not change the writer's wait policy");
        }
    }

}
