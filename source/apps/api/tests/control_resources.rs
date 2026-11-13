mod auth_support;
use auth_support::*;
use nwui_api::{
    contracts::Actor,
    control::resources::{self, Mutation, Resource},
};
use serde_json::{Value, json};

fn actor() -> Actor {
    Actor {
        id: None,
        label: Some("gateway:test".into()),
        ip: Some("198.51.100.7".into()),
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn writes_fail_fast_during_apply_and_never_commit_after_returning_busy() {
    let db = test_db().await;
    let lock = db.state.runtime.apply_lock.lock().await;
    let result = tokio::time::timeout(std::time::Duration::from_millis(100),
        write(&db, Resource::Upstream, Mutation::Create, None,
            json!({"name":"late-write","servers":["app:8080"]}))).await;
    assert!(result.is_ok(), "write waited behind apply and could commit after the gateway times out");
    let error = result.unwrap().unwrap_err();
    assert_eq!(error.status, 503);
    assert_eq!(error.code, "config_busy");
    drop(lock);
    assert_eq!(db.client.query_one("SELECT count(*) FROM upstreams WHERE name='late-write'",&[]).await.unwrap().get::<_,i64>(0),0);
}
async fn write(
    db: &TestDb,
    kind: Resource,
    mutation: Mutation,
    id: Option<&str>,
    body: Value,
) -> Result<Value, nwui_api::error::ApiError> {
    resources::mutate(&db.state, kind, mutation, id, body, &actor()).await
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn relationships_disabled_state_and_audit_are_preserved() {
    let db = test_db().await;
    let u = write(
        &db,
        Resource::Upstream,
        Mutation::Create,
        None,
        json!({"name":"backend","servers":["app:8080"]}),
    )
    .await
    .unwrap();
    let uid = u["id"].as_str().unwrap();
    let s = write(
        &db,
        Resource::Server,
        Mutation::Create,
        None,
        json!({"serverName":"example.test","listen":8081,"upstreamId":uid}),
    )
    .await
    .unwrap();
    let sid = s["id"].as_str().unwrap();
    assert_eq!(
        write(
            &db,
            Resource::Upstream,
            Mutation::Delete,
            Some(uid),
            Value::Null
        )
        .await
        .unwrap_err()
        .status,
        409
    );
    assert_eq!(
        write(
            &db,
            Resource::Server,
            Mutation::Toggle,
            Some(sid),
            Value::Null
        )
        .await
        .unwrap()["enabled"],
        false
    );
    let s = write(
        &db,
        Resource::Server,
        Mutation::Update,
        Some(sid),
        json!({"serverName":"new.test","listen":8081,"upstreamId":uid}),
    )
    .await
    .unwrap();
    assert_eq!(s["enabled"], false);
    assert_eq!(s["config"]["locations"][0]["directives"], json!([]));
    let rows = audit_rows(&db.client, "reverseproxy:create").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get::<_, Option<String>>("actor_label").as_deref(),
        Some("gateway:test")
    );
    assert_eq!(
        write(
            &db,
            Resource::Stream,
            Mutation::Create,
            None,
            json!({"listen":8081,"protocol":"tcp","proxyPass":"db:5432"})
        )
        .await
        .unwrap_err()
        .code,
        "validation_failed"
    );
    write(
        &db,
        Resource::Stream,
        Mutation::Create,
        None,
        json!({"listen":8081,"protocol":"udp","proxyPass":"db:5432"}),
    )
    .await
    .unwrap();
    write(
        &db,
        Resource::Server,
        Mutation::Delete,
        Some(sid),
        Value::Null,
    )
    .await
    .unwrap();
    write(
        &db,
        Resource::Upstream,
        Mutation::Delete,
        Some(uid),
        Value::Null,
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn validation_and_null_replacement_do_not_leak_database_errors() {
    let db = test_db().await;
    let body = json!({"name":"backend","servers":["app:8080"]});
    write(
        &db,
        Resource::Upstream,
        Mutation::Create,
        None,
        body.clone(),
    )
    .await
    .unwrap();
    let e = write(&db, Resource::Upstream, Mutation::Create, None, body)
        .await
        .unwrap_err();
    assert_eq!(e.status, 400);
    assert!(e.field_errors.unwrap().contains_key("name"));
    let s=write(&db,Resource::Server,Mutation::Create,None,json!({"serverName":"a.test","listen":8091,"proxyPass":"http://app:8080","cache":{"enabled":true}})).await.unwrap();
    let s = write(
        &db,
        Resource::Server,
        Mutation::Update,
        Some(s["id"].as_str().unwrap()),
        json!({"serverName":"a.test","listen":8091,"proxyPass":"http://app:8080","cache":null}),
    )
    .await
    .unwrap();
    assert!(s["config"].get("cache").is_none());
    for port in [3000, 3001, 5432] {
        let e = write(
            &db,
            Resource::Server,
            Mutation::Create,
            None,
            json!({"serverName":"a.test","listen":port,"proxyPass":"http://app"}),
        )
        .await
        .unwrap_err();
        assert!(e.field_errors.unwrap().contains_key("listen"));
    }
    assert_eq!(
        write(
            &db,
            Resource::Server,
            Mutation::Update,
            Some("missing"),
            json!({})
        )
        .await
        .unwrap_err()
        .status,
        404
    );
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn locations_tls_relationships_and_atomic_audit_failure() {
    let db = test_db().await;
    let u = write(
        &db,
        Resource::Upstream,
        Mutation::Create,
        None,
        json!({"name":"app","servers":["backend:80"]}),
    )
    .await
    .unwrap();
    let body = json!({"serverName":"test.example","listen":8100,"targetMode":"locations","locations":[{"path":"/","upstreamId":u["id"],"directives":[],"cacheEnabled":false}],"cache":{"enabled":true}});
    let s = write(&db, Resource::Server, Mutation::Create, None, body)
        .await
        .unwrap();
    assert_eq!(s["config"]["locations"][0]["upstreamId"], u["id"]);
    assert!(s["config"]["locations"][0].get("proxyPass").is_none());
    assert_eq!(s["config"]["locations"][0]["cacheEnabled"], false);
    // A location naming an upstream that no longer exists, worded as 1.17.2 did.
    let gone = json!({"serverName":"gone.example","listen":8100,"targetMode":"locations","locations":[{"path":"/","upstreamId":u["id"]},{"path":"/api","upstreamId":"deleted"}]});
    assert_eq!(
        write(&db, Resource::Server, Mutation::Create, None, gone)
            .await
            .unwrap_err()
            .field_errors
            .unwrap()["locations"],
        "location 2 (/api): 선택한 upstream을 찾을 수 없습니다."
    );
    let mut bad = json!({"serverName":"b.example","listen":8100,"proxyPass":"http://a","certificateId":"missing"});
    assert!(
        write(&db, Resource::Server, Mutation::Create, None, bad.clone())
            .await
            .unwrap_err()
            .field_errors
            .unwrap()
            .contains_key("certificateId")
    );
    bad["certificateId"] = Value::Null;
    bad["upstreamId"] = json!("ignored-because-direct");
    write(&db, Resource::Server, Mutation::Create, None, bad)
        .await
        .unwrap();
    db.client.batch_execute("CREATE FUNCTION fail_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'audit unavailable'; END $$; CREATE TRIGGER reject_audit BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION fail_audit()").await.unwrap();
    assert_eq!(
        write(
            &db,
            Resource::Upstream,
            Mutation::Create,
            None,
            json!({"name":"rollback","servers":["a:80"]})
        )
        .await
        .unwrap_err()
        .status,
        500
    );
    assert_eq!(
        db.client
            .query_one("SELECT count(*) FROM upstreams WHERE name='rollback'", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn reserved_writes_are_serialized_and_disabled_streams_reserve_ports() {
    let db = test_db().await;
    let http = json!({"serverName":"a.test","listen":8200,"proxyPass":"http://a"});
    let tcp = json!({"listen":8200,"protocol":"tcp","proxyPass":"a:80"});
    let (a, b) = tokio::join!(
        write(&db, Resource::Server, Mutation::Create, None, http),
        write(&db, Resource::Stream, Mutation::Create, None, tcp)
    );
    assert_ne!(
        a.is_ok(),
        b.is_ok(),
        "one owner wins; the other sees the committed reservation"
    );
    let stream = write(
        &db,
        Resource::Stream,
        Mutation::Create,
        None,
        json!({"listen":8300,"protocol":"tcp","proxyPass":"a:80"}),
    )
    .await
    .unwrap();
    let sid = stream["id"].as_str().unwrap();
    write(
        &db,
        Resource::Stream,
        Mutation::Toggle,
        Some(sid),
        Value::Null,
    )
    .await
    .unwrap();
    let e = write(
        &db,
        Resource::Server,
        Mutation::Create,
        None,
        json!({"serverName":"a.test","listen":8300,"proxyPass":"http://a"}),
    )
    .await
    .unwrap_err();
    assert!(e.field_errors.unwrap().contains_key("listen"));
    let stream = write(
        &db,
        Resource::Stream,
        Mutation::Update,
        Some(sid),
        json!({"listen":8301,"protocol":"udp","proxyPass":"a:81"}),
    )
    .await
    .unwrap();
    assert_eq!(stream["enabled"], false);
    assert_eq!(stream["protocol"], "udp");
}

use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use std::net::SocketAddr;
use tower::ServiceExt as _;
fn app(db: &TestDb) -> Router {
    nwui_api::app::finish_router(
        nwui_api::app::with_request_context(
            nwui_api::control::routes::router().with_state(db.state.clone()),
            None,
        ),
        &db.state.config.version,
    )
}
fn request(method: &str, path: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "admin.example.test")
        .header("x-real-ip", "198.51.100.7")
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41000))))
}
async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let r = app.clone().oneshot(req).await.unwrap();
    let status = r.status();
    let b = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    (
        status,
        if b.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&b).unwrap()
        },
    )
}
async fn gateway(db: &TestDb, role: &str) -> String {
    let token = format!("test-{}", uuid::Uuid::new_v4());
    auth_support::gateway_issuer(&db.client).await;
    db.client.execute("INSERT INTO gateway_tokens (id,name,token_hash,role,created_by_user_id) VALUES ($1,'test',$2,$3::text::role,'q3-gateway-issuer')",&[&uuid::Uuid::new_v4().to_string(),&nwui_api::auth::token_digest(&token),&role]).await.unwrap();
    token
}
fn cookie(db: &TestDb) -> String {
    let now = nwui_api::auth::principal::now_unix_ms() / 1000;
    let claims = json!({"id":"u1","sv":3});
    let token = nwui_api::auth::session::encode(
        claims.as_object().unwrap(),
        SECRET,
        nwui_api::auth::session::COOKIE_NAME_PLAIN,
        now,
        3600,
    )
    .unwrap();
    let _ = db;
    format!("authjs.session-token={token}")
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn node_http_auth_order_roles_payloads_and_statuses() {
    let db = test_db().await;
    let app = app(&db);
    let token = gateway(&db, "editor").await;
    let req = || request("POST", "/api/v1/upstreams").header("content-type", "application/json");
    let (status, body) = send(&app, req().body(Body::from("broken")).unwrap()).await;
    assert_eq!(status, 409);
    assert_eq!(body["error"]["code"], "version_mismatch");
    let (status, _) = send(
        &app,
        req()
            .header("x-gateway-version", &db.state.config.version)
            .body(Body::from("broken"))
            .unwrap(),
    )
    .await;
    assert_eq!(status, 401);
    let viewer = gateway(&db, "viewer").await;
    let (status, _) = send(
        &app,
        req()
            .header("x-gateway-version", &db.state.config.version)
            .header("authorization", format!("Bearer {viewer}"))
            .body(Body::from("broken"))
            .unwrap(),
    )
    .await;
    assert_eq!(status, 403);
    let auth = || {
        req()
            .header("x-gateway-version", &db.state.config.version)
            .header("authorization", format!("Bearer {token}"))
    };
    let (status, body) = send(&app, auth().body(Body::from("broken")).unwrap()).await;
    assert_eq!(status, 400);
    assert_eq!(body["error"]["code"], "bad_request");
    let (status, body) = send(
        &app,
        auth()
            .body(Body::from(
                json!({"name":"backend","servers":["app:80"],"future":true}).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let id = body["upstream"]["id"].as_str().unwrap();
    let (status, body) = send(
        &app,
        request("GET", &format!("/api/v1/upstreams/{id}"))
            .header("x-gateway-version", &db.state.config.version)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["upstream"]["name"], "backend");
    let (status, _) = send(
        &app,
        request("GET", "/api/v1/upstreams/missing")
            .header("x-gateway-version", &db.state.config.version)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn browser_forms_enforce_origin_session_and_field_errors() {
    let db = test_db().await;
    let app = app(&db);
    let token = gateway(&db, "admin").await;
    let form = || {
        request("POST", "/api/ui/v1/forms/upstreams.createUpstream")
            .header("content-type", "application/x-www-form-urlencoded")
    };
    let (status, _) = send(
        &app,
        form()
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from("name=a&servers=a%3A80"))
            .unwrap(),
    )
    .await;
    assert_eq!(status, 403);
    let (status, body) = send(
        &app,
        form()
            .header("origin", "http://admin.example.test")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, 401);
    assert_eq!(body["error"]["code"], "unauthorized");
    insert_user(&db.client, UserRow::default(), None).await;
    let cookie = cookie(&db);
    let auth = || {
        form()
            .header("origin", "http://admin.example.test")
            .header("cookie", &cookie)
    };
    let (status, body) = send(
        &app,
        auth()
            .body(Body::from("name=invalid-name&servers=bad"))
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["result"]["ok"], false);
    assert!(body["result"]["fieldErrors"]["name"].is_string());
    let (status, body) = send(
        &app,
        auth()
            .body(Body::from("name=web&servers=app%3A80%0Aapp%3A81"))
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["redirectTo"]
            .as_str()
            .unwrap()
            .starts_with("/reverse-proxy/upstreams/")
    );
    let rows = audit_rows(&db.client, "reverseproxy:upstream:create").await;
    assert_eq!(
        rows[0].get::<_, Option<String>>("actor_id").as_deref(),
        Some("u1")
    );
    assert_eq!(rows[0].get::<_, Option<String>>("actor_label").as_deref(),Some("u1@example.test"));
    let (status, _) = send(
        &app,
        auth()
            .body(Body::from("_apiArgs=%5B%22unexpected%22%5D"))
            .unwrap(),
    )
    .await;
    assert_eq!(status, 400);
}

#[test]
fn browser_directive_editor_and_node_validation_share_rules() {
    use nwui_api::contracts::FormInput;
    let mut form = FormInput::default();
    for (k, v) in [
        ("serverName", "a.test"),
        ("listen", "80"),
        ("mode", "direct"),
        ("proxyPass", "http://app"),
        (
            "locationDirectives",
            r#"{"headers":[{"name":"Host","value":"$host"}],"advanced":"proxy_read_timeout 60s;"}"#,
        ),
    ] {
        form.fields.insert(k.into(), vec![v.into()]);
    }
    let raw = resources::form_body(Resource::Server, &form).unwrap();
    let parsed = resources::parse_server(&raw, &Default::default()).unwrap();
    assert_eq!(
        parsed["config"]["locations"][0]["directives"],
        json!([{"name":"proxy_set_header","args":["Host","$host"]},{"name":"proxy_read_timeout","args":["60s"]}])
    );
    form.fields.insert(
        "locationDirectives".into(),
        vec![r#"{"advanced":"include /tmp/evil;"}"#.into()],
    );
    assert!(
        resources::form_body(Resource::Server, &form)
            .unwrap_err()
            .field_errors
            .unwrap()
            .contains_key("locationDirectives")
    );
    for bad in [
        json!({"serverName":"a.test","listen":80,"proxyPass":"http://a;evil"}),
        json!({"serverName":"a.test","listen":80,"proxyPass":"http://a","cache":{"enabled":true},"locationDirectives":[{"name":"proxy_buffering","args":["off"]}]}),
    ] {
        assert!(resources::parse_server(&bad, &Default::default()).is_err());
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn views_expose_only_safe_certificate_fields_and_enforce_edit_roles() {
    let db = test_db().await;
    db.client.execute("INSERT INTO certificates (id,domains,status,cert_pem,key_pem) VALUES ('cert','[\"a.test\"]','issued','CERTIFICATE_MATERIAL','PRIVATE_KEY_MATERIAL')",&[]).await.unwrap();
    let mut raw = json!({"serverName":"a.test","listen":8400,"proxyPass":"http://localhost:8080","certificateId":"cert","tls":{"tlsPort":8443,"tlsOptions":{"sslProtocols":"modern","sslCiphers":"unrecognized","hsts":{"enabled":true,"maxAge":10,"includeSubDomains":true,"preload":true}}}});
    let error=write(&db,Resource::Server,Mutation::Create,None,raw.clone()).await.unwrap_err();
    assert_eq!(error.status,400);
    assert!(error.field_errors.unwrap().contains_key("tlsOptions"));
    raw["tls"]["tlsOptions"]["sslCiphers"]=json!("intermediate");
    let server = write(&db, Resource::Server, Mutation::Create, None, raw)
        .await
        .unwrap();
    assert_eq!(server["config"]["tlsOptions"]["sslProtocols"], "modern");
    assert_eq!(server["config"]["tlsOptions"]["sslCiphers"], "intermediate");
    assert_eq!(server["config"]["tlsOptions"]["hsts"]["preload"], false);
    let s = nwui_api::auth::principal::SessionPrincipal {
        user_id: "user".into(),
        email: None,
        name: None,
        role: nwui_api::contracts::Role::Viewer,
        session_version: 0,
        totp_enabled: false,
        requires_2fa: false,
        mfa_verified: true,
        expires_at: i64::MAX,
    };
    let path = format!("/reverse-proxy/{}", server["id"].as_str().unwrap());
    let query = Default::default();
    let data = nwui_api::control::views::view(&db.state, &s, &path, &query)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(data["canWrite"], false);
    assert_eq!(data["selfProxyNotice"], true);
    assert_eq!(data["certificates"][0]["tlsReady"], true);
    assert!(!data.to_string().contains("MATERIAL"));
    assert_eq!(
        nwui_api::control::views::view(&db.state, &s, &format!("{path}/edit"), &query)
            .await
            .unwrap_err()
            .status,
        403
    );
    assert_eq!(
        nwui_api::control::views::view(&db.state, &s, "/stream/missing", &query)
            .await
            .unwrap_err()
            .status,
        404
    );
    assert!(
        nwui_api::control::views::view(&db.state, &s, "/unknown", &query)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        write(
            &db,
            Resource::Stream,
            Mutation::Create,
            None,
            json!({"listen":8443,"protocol":"tcp","proxyPass":"a:80"})
        )
        .await
        .unwrap_err()
        .field_errors
        .unwrap()
        .contains_key("listen")
    );
    db.client
        .execute(
            "INSERT INTO app_settings (id,metrics_excluded_hosts) VALUES ('settings','A.TEST')",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        nwui_api::control::views::view(&db.state, &s, &path, &query)
            .await
            .unwrap()
            .unwrap()["selfProxyNotice"],
        false
    );
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn all_node_resource_mutations_work_over_http_and_keep_disabled_state() {
    let db = test_db().await;
    let app = app(&db);
    let token = gateway(&db, "editor").await;
    for (plural, singular, body, update) in [
        (
            "servers",
            "server",
            json!({"serverName":"a.test","listen":8500,"proxyPass":"http://app"}),
            json!({"serverName":"b.test","listen":8501,"proxyPass":"http://backend"}),
        ),
        (
            "streams",
            "stream",
            json!({"listen":8600,"protocol":"tcp","proxyPass":"app:80"}),
            json!({"listen":8601,"protocol":"udp","proxyPass":"backend:81"}),
        ),
        (
            "upstreams",
            "upstream",
            json!({"name":"cluster","servers":["a:80"]}),
            json!({"name":"updated","servers":["b:81"]}),
        ),
    ] {
        let auth = |method: &str, path: &str| {
            request(method, path)
                .header("x-gateway-version", &db.state.config.version)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
        };
        let (status, data) = send(
            &app,
            auth("POST", &format!("/api/v1/{plural}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, 201, "{data}");
        let path = format!(
            "/api/v1/{plural}/{}",
            data[singular]["id"].as_str().unwrap()
        );
        if plural != "upstreams" {
            let (status, data) = send(
                &app,
                auth("POST", &format!("{path}/toggle"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, 200);
            assert_eq!(data["enabled"], false);
        }
        let (status, data) = send(
            &app,
            auth("PATCH", &path)
                .body(Body::from(update.to_string()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, 200, "{data}");
        if plural != "upstreams" {
            assert_eq!(data[singular]["enabled"], false);
        }
        let (status, data) = send(&app, auth("DELETE", &path).body(Body::empty()).unwrap()).await;
        assert_eq!(status, 200);
        assert_eq!(data["deleted"], true);
        let (status, _) = send(&app, auth("DELETE", &path).body(Body::empty()).unwrap()).await;
        assert_eq!(status, 404);
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn browser_server_and_stream_forms_support_create_update_toggle_delete() {
    let db = test_db().await;
    let app = app(&db);
    insert_user(&db.client, UserRow::default(), None).await;
    let cookie = cookie(&db);
    for (group, name, body) in [
        (
            "reverse-proxy",
            "Server",
            "serverName=a.test&listen=8700&mode=direct&proxyPass=http%3A%2F%2Fapp",
        ),
        (
            "stream",
            "Stream",
            "listen=8800&protocol=tcp&proxyPass=app%3A80",
        ),
    ] {
        let auth = |verb: &str| {
            request("POST", &format!("/api/ui/v1/forms/{group}.{verb}{name}"))
                .header("origin", "http://admin.example.test")
                .header("cookie", &cookie)
                .header("content-type", "application/x-www-form-urlencoded")
        };
        let (status, data) = send(&app, auth("create").body(Body::from(body)).unwrap()).await;
        assert_eq!(status, 200);
        let location = data["redirectTo"].as_str().unwrap();
        let id = location.rsplit('/').next().unwrap();
        let (status, data) = send(
            &app,
            auth("toggle").body(Body::from(format!("id={id}"))).unwrap(),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(data["result"]["ok"], true);
        assert_eq!(data["refresh"], true);
        let (status, data) = send(
            &app,
            auth("update")
                .body(Body::from(format!("id={id}&{body}")))
                .unwrap(),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(data["redirectTo"], location);
        let (status, data) = send(
            &app,
            auth("delete").body(Body::from(format!("id={id}"))).unwrap(),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(data["result"]["ok"], true);
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn in_the_image_saving_a_target_on_an_internal_component_port_is_refused() {
    let db = test_db_with(&[("NWUI_CONTAINER", "1")]).await;
    let refused = |r: Result<Value, nwui_api::error::ApiError>, field: &str| {
        let e = r.expect_err("must be refused");
        assert_eq!((e.status.as_u16(), e.code.to_string()), (400, "validation_failed".to_string()));
        assert!(e.field_errors.as_ref().is_some_and(|f| f.contains_key(field)), "{:?}", e.field_errors);
    };
    refused(write(&db, Resource::Server, Mutation::Create, None, json!({"serverName":"api.test","listen":8300,"proxyPass":"http://127.0.0.1:3001"})).await, "locations");
    refused(write(&db, Resource::Stream, Mutation::Create, None, json!({"listen":8301,"protocol":"tcp","proxyPass":"localhost:5432"})).await, "proxyPass");
    refused(write(&db, Resource::Upstream, Mutation::Create, None, json!({"name":"internal","servers":["127.0.0.1:3000"]})).await, "servers");
    let located = write(&db, Resource::Server, Mutation::Create, None, json!({"serverName":"loc.test","listen":8303,"targetMode":"locations","locations":[{"path":"/ok","mode":"direct","proxyPass":"http://10.0.0.5:80","directives":[]},{"path":"/db","mode":"direct","proxyPass":"http://127.1:5432","directives":[]}]})).await.unwrap_err();
    let message = located.field_errors.as_ref().and_then(|f| f.get("locations")).cloned().unwrap_or_default();
    assert!(message.starts_with("location 2 (/db): "), "the location is named as the other location errors do: {message}");
    // The documented way to publish the management UI under a domain stays allowed.
    write(&db, Resource::Server, Mutation::Create, None, json!({"serverName":"admin.test","listen":8302,"proxyPass":"http://127.0.0.1:8080"})).await.unwrap();

    let outside = test_db().await;
    write(&outside, Resource::Server, Mutation::Create, None, json!({"serverName":"api.test","listen":8300,"proxyPass":"http://127.0.0.1:3001"})).await.unwrap();
}

/// nginx routes `http://nwui_api` / `http://nwui_web` to the image's own upstream blocks, so they
/// are internal targets whatever the case or port. A user's own upstream is unaffected.
#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn in_the_image_saving_a_proxy_to_a_runtime_upstream_name_is_refused() {
    let db = test_db_with(&[("NWUI_CONTAINER", "1")]).await;
    let refused = |r: Result<Value, nwui_api::error::ApiError>, field: &str| {
        let e = r.expect_err("must be refused");
        assert_eq!((e.status.as_u16(), e.code.to_string()), (400, "validation_failed".to_string()));
        let message = e.field_errors.as_ref().and_then(|f| f.get(field)).cloned().unwrap_or_default();
        assert!(message.contains("내부 upstream"), "{:?}", e.field_errors);
    };
    refused(write(&db, Resource::Server, Mutation::Create, None, json!({"serverName":"api.test","listen":8300,"proxyPass":"http://nwui_api"})).await, "locations");
    refused(write(&db, Resource::Server, Mutation::Create, None, json!({"serverName":"web.test","listen":8301,"proxyPass":"http://NWUI_WEB:8080/app"})).await, "locations");
    refused(write(&db, Resource::Server, Mutation::Create, None, json!({"serverName":"loc.test","listen":8302,"targetMode":"locations","locations":[{"path":"/ok","mode":"direct","proxyPass":"http://10.0.0.5:80","directives":[]},{"path":"/api","mode":"direct","proxyPass":"https://Nwui_Api","directives":[]}]})).await, "locations");
    let pool = write(&db, Resource::Upstream, Mutation::Create, None, json!({"name":"nwui_pool","servers":["10.0.0.5:3001"]})).await.unwrap();
    let pool_id = pool["id"].as_str().expect("created upstream id").to_owned();
    write(&db, Resource::Server, Mutation::Create, None, json!({"serverName":"pool.test","listen":8303,"upstreamId":pool_id})).await.unwrap();

    // A variable host or port becomes either name, or a loopback port, per request.
    let refused_var = |r: Result<Value, nwui_api::error::ApiError>, field: &str| {
        let e = r.expect_err("must be refused");
        let message = e.field_errors.as_ref().and_then(|f| f.get(field)).cloned().unwrap_or_default();
        assert!(message.contains("변수"), "{:?}", e.field_errors);
    };
    refused_var(write(&db, Resource::Server, Mutation::Create, None, json!({"serverName":"var.test","listen":8304,"proxyPass":"http://$arg_u"})).await, "locations");
    // A variable port is not a URL: refused by URL validation before the authority rule.
    let port = write(&db, Resource::Server, Mutation::Create, None, json!({"serverName":"port.test","listen":8305,"targetMode":"locations","locations":[{"path":"/p","mode":"direct","proxyPass":"http://127.0.0.1:$arg_p/x","directives":[]}]})).await.unwrap_err();
    assert!(port.field_errors.as_ref().is_some_and(|f| f.contains_key("locations")), "{:?}", port.field_errors);
    write(&db, Resource::Server, Mutation::Create, None, json!({"serverName":"path.test","listen":8306,"proxyPass":"http://10.0.0.5:8080/$1?q=$arg_q"})).await.unwrap();

    let outside = test_db().await;
    write(&outside, Resource::Server, Mutation::Create, None, json!({"serverName":"api.test","listen":8300,"proxyPass":"http://nwui_api"})).await.unwrap();
    write(&outside, Resource::Server, Mutation::Create, None, json!({"serverName":"var.test","listen":8304,"proxyPass":"http://$arg_u"})).await.unwrap();
}

async fn stale_resource_edit_is_conflict(plural: &str, singular: &str, create: Value, first: Value, second: Value, field: &str) {
    let db = test_db().await;
    let app = app(&db);
    let token = gateway(&db, "editor").await;
    let auth = |method: &str, path: &str| request(method, path)
        .header("x-gateway-version", &db.state.config.version)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json");
    let (status, created) = send(&app, auth("POST", &format!("/api/v1/{plural}"))
        .body(Body::from(create.to_string())).unwrap()).await;
    assert_eq!(status, 201, "{created}");
    let path = format!("/api/v1/{plural}/{}", created[singular]["id"].as_str().unwrap());
    // Two editors open the same row before either submits a change.
    let (_, opened_a) = send(&app, auth("GET", &path).body(Body::empty()).unwrap()).await;
    let (_, opened_b) = send(&app, auth("GET", &path).body(Body::empty()).unwrap()).await;
    let version = opened_a[singular].get("version").cloned().expect("new nodes expose a version");
    assert!(version.is_string());
    assert_eq!(opened_a, opened_b);
    let mut first_request = first.clone(); first_request["version"] = version.clone();
    let (status, result) = send(&app, auth("PATCH", &path)
        .body(Body::from(first_request.to_string())).unwrap()).await;
    assert_eq!(status, 200, "{result}");
    let mut second_request = second; second_request["version"] = version;
    let (status, result) = send(&app, auth("PATCH", &path)
        .body(Body::from(second_request.to_string())).unwrap()).await;
    let (_, saved) = send(&app, auth("GET", &path).body(Body::empty()).unwrap()).await;
    assert_eq!((status, &saved[singular][field]), (StatusCode::CONFLICT, &first[field]), "stale save: {result}; saved: {saved}");
    assert!(result.to_string().contains("다른 사용자가"), "{result}");
    assert!(result.to_string().contains("새로고침"), "{result}");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_concurrency_upstream_edits_reject_the_stale_second_save() {
    stale_resource_edit_is_conflict("upstreams", "upstream", json!({"name":"opened","servers":["127.0.0.1:9000"]}),
        json!({"name":"first","servers":["127.0.0.1:9001"]}), json!({"name":"second","servers":["127.0.0.1:9002"]}), "name").await;
}
#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_concurrency_server_edits_reject_the_stale_second_save() {
    stale_resource_edit_is_conflict("servers", "server", json!({"serverName":"opened.test","listen":80,"proxyPass":"http://127.0.0.1:9000"}),
        json!({"serverName":"first.test","listen":80,"proxyPass":"http://127.0.0.1:9001"}), json!({"serverName":"second.test","listen":80,"proxyPass":"http://127.0.0.1:9002"}), "serverName").await;
}
#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_concurrency_stream_edits_reject_the_stale_second_save() {
    stale_resource_edit_is_conflict("streams", "stream", json!({"listen":19000,"protocol":"tcp","proxyPass":"127.0.0.1:9000"}),
        json!({"listen":19001,"protocol":"tcp","proxyPass":"127.0.0.1:9001"}), json!({"listen":19002,"protocol":"tcp","proxyPass":"127.0.0.1:9002"}), "listen").await;
}
