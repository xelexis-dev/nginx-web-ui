#[path = "auth_support/mod.rs"]
mod auth_support;
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use std::net::SocketAddr;
use tower::ServiceExt;

fn request(
    method: &str,
    path: &str,
    version: Option<&str>,
    token: Option<&str>,
    body: &str,
) -> Request<Body> {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "admin.example.test")
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
    if let Some(v) = version {
        req = req.header("x-gateway-version", v)
    }
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"))
    }
    req.body(Body::from(body.to_owned())).unwrap()
}
async fn body(response: axum::response::Response) -> Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn pairing_issues_hashed_token_audits_provenance_and_self_revokes() {
    let db = auth_support::test_db().await;
    nwui_api::nodes::service::ensure_instance_id(&db.state)
        .await
        .unwrap();
    auth_support::insert_user(
        &db.client,
        auth_support::UserRow {
            role: "admin",
            ..Default::default()
        },
        None,
    )
    .await;
    db.client.execute("INSERT INTO access_tokens(id,name,kind,scopes,token_hash,token_suffix,created_by_user_id,legacy_pairing) VALUES('access-id','server-secret-name','server',$1,$2,'test','u1',true)",&[&json!(["node:pair"]),&nwui_api::auth::token_digest("server-secret")]).await.unwrap();
    let app = nwui_api::app::with_request_context(
        nwui_api::nodes::routes::router().with_state(db.state.clone()),
        None,
    );
    let version = &db.state.config.version;
    let response = app
        .clone()
        .oneshot(request("POST", "/api/v1/pair", None, None, "not-json"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let response = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/pair",
            Some(version),
            None,
            &json!({"token":"server-secret","gatewayName":"gateway","role":"viewer"}).to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload = body(response).await;
    let issued = payload["token"].as_str().unwrap();
    assert_eq!(issued.len(), 43);
    assert_eq!(payload["role"], "viewer");
    let stored = db
        .client
        .query_one(
            "SELECT token_hash,created_by_user_id FROM gateway_tokens",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        stored.get::<_, String>(0),
        nwui_api::auth::token_digest(issued)
    );
    assert_eq!(stored.get::<_, String>(1), "u1");
    let audits = auth_support::audit_rows(&db.client, "node:pair").await;
    assert_eq!(audits[0].get::<_, Option<String>>("actor_id"), None);
    assert_eq!(audits[0].get::<_, String>("actor_label"), "token:access-id");
    assert!(
        !audits[0]
            .get::<_, Value>("after")
            .to_string()
            .contains("secret")
    );
    assert_eq!(
        app.clone()
            .oneshot(request(
                "GET",
                "/api/v1/info",
                Some(version),
                Some(issued),
                ""
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.clone()
            .oneshot(request(
                "DELETE",
                "/api/v1/pair",
                Some(version),
                Some(issued),
                ""
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.oneshot(request(
            "GET",
            "/api/v1/info",
            Some(version),
            Some(issued),
            ""
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn pairing_lock_precedes_body_parsing_and_role_validation_precedes_token_lookup() {
    let db = auth_support::test_db().await;
    let app = nwui_api::app::with_request_context(
        nwui_api::nodes::routes::router().with_state(db.state.clone()),
        None,
    );
    let version = &db.state.config.version;
    let r = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/v1/pair",
            Some(version),
            None,
            r#"{"token":"unknown","gatewayName":"g","role":null}"#,
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(body(r).await["error"]["fieldErrors"]["role"].is_string());
    for _ in 0..100 {
        db.state
            .runtime
            .login_throttle
            .record_ip_failure("127.0.0.1", nwui_api::auth::principal::now_unix_ms());
    }
    let r = app
        .oneshot(request(
            "POST",
            "/api/v1/pair",
            Some(version),
            None,
            "bad-json",
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
#[ignore = "requires NWUI_TEST_DATABASE_URL"]
async fn a_server_token_stops_pairing_when_its_issuer_is_no_longer_an_active_admin() {
    // The token delegates the issuing admin's authority to connect gateways; it must not outlive
    // that authority (disabled, demoted or deleted admin).
    let db = auth_support::test_db().await;
    nwui_api::nodes::service::ensure_instance_id(&db.state).await.unwrap();
    auth_support::insert_user(&db.client, auth_support::UserRow { role: "admin", ..Default::default() }, None).await;
    for (id, secret, issuer) in [("t-u1", "secret-u1", Some("u1")), ("t-orphan", "secret-orphan", None)] {
        db.client.execute("INSERT INTO access_tokens(id,name,kind,scopes,token_hash,token_suffix,created_by_user_id,legacy_pairing) VALUES($1,$1,'server',$2,$3,'test',$4,true)",&[&id,&json!(["node:pair"]),&nwui_api::auth::token_digest(secret),&issuer]).await.unwrap();
    }
    let app = nwui_api::app::with_request_context(nwui_api::nodes::routes::router().with_state(db.state.clone()), None);
    let version = db.state.config.version.clone();
    let pair = |secret: &'static str| {
        let (app, version) = (app.clone(), version.clone());
        async move {
            let body = json!({"token": secret, "gatewayName": "gateway", "role": "viewer"}).to_string();
            app.oneshot(request("POST", "/api/v1/pair", Some(&version), None, &body)).await.unwrap().status()
        }
    };
    db.client.execute("UPDATE users SET enabled = false WHERE id = 'u1'", &[]).await.unwrap();
    assert_eq!(pair("secret-u1").await, StatusCode::FORBIDDEN, "disabled issuer");
    db.client.execute("UPDATE users SET enabled = true, role = 'editor' WHERE id = 'u1'", &[]).await.unwrap();
    assert_eq!(pair("secret-u1").await, StatusCode::FORBIDDEN, "demoted issuer");
    assert_eq!(pair("secret-orphan").await, StatusCode::FORBIDDEN, "deleted issuer");
    db.client.execute("UPDATE users SET role = 'admin' WHERE id = 'u1'", &[]).await.unwrap();
    assert_eq!(pair("secret-u1").await, StatusCode::OK, "an active admin's token still pairs");
}


#[tokio::test]
#[ignore = "real DB"]
async fn q3r1_pairing_token_reuse_and_cascade_revocation() {
    let db = auth_support::test_db().await;
    nwui_api::nodes::service::ensure_instance_id(&db.state)
        .await
        .unwrap();
    auth_support::insert_user(
        &db.client,
        auth_support::UserRow {
            role: "admin",
            ..Default::default()
        },
        None,
    )
    .await;
    db.client.execute("INSERT INTO access_tokens(id,name,kind,scopes,token_hash,token_suffix,created_by_user_id) VALUES('access-id','server-secret-name','server',$1,$2,'test','u1')",&[&json!(["node:pair","config:read","config:write","nginx:reload"]),&nwui_api::auth::token_digest("server-secret")]).await.unwrap();
    let app = nwui_api::app::with_request_context(
        nwui_api::nodes::routes::router().with_state(db.state.clone()),
        None,
    );
    let version = &db.state.config.version;

    let payload=json!({"token":"server-secret","gatewayName":"gateway"}).to_string();
    let (a,b)=tokio::join!(app.clone().oneshot(request("POST","/api/v1/pair",Some(version),None,&payload)),app.clone().oneshot(request("POST","/api/v1/pair",Some(version),None,&payload)));
    let mut statuses=[a.unwrap().status().as_u16(),b.unwrap().status().as_u16()];statuses.sort();
    assert_eq!(statuses,[200,200]);
    let count:i64=db.client.query_one("SELECT count(*) FROM gateway_tokens",&[]).await.unwrap().get(0);
    assert_eq!(count,2);
    let actor=nwui_api::auth::principal::SessionPrincipal{user_id:"u1".into(),email:None,name:None,role:nwui_api::contracts::Role::Admin,session_version:3,totp_enabled:false,requires_2fa:false,mfa_verified:true,expires_at:0};
    nwui_api::auth::tokens::revoke(&db.state,&actor,&auth_support::ctx(),&nwui_api::auth::tokens::TokenScope::Server,"access-id").await.unwrap();
    let revoked:i64=db.client.query_one("SELECT count(*) FROM gateway_tokens WHERE revoked_at IS NOT NULL",&[]).await.unwrap().get(0);
    assert_eq!(revoked,2);
}


#[tokio::test]
#[ignore = "real DB"]
async fn q3_w10_gateway_issuer_must_remain_an_active_admin() {
    let db=auth_support::test_db().await;
    auth_support::insert_user(&db.client,auth_support::UserRow {role:"admin",..Default::default()},None).await;
    db.client.execute("INSERT INTO gateway_tokens(id,name,token_hash,role,created_by_user_id) VALUES('g','gateway',$1,'viewer','u1')", &[&nwui_api::auth::token_digest("gateway-secret")]).await.unwrap();
    let mut headers=axum::http::HeaderMap::new();
    headers.insert("authorization","Bearer gateway-secret".parse().unwrap());
    headers.insert("x-gateway-version",db.state.config.version.parse().unwrap());
    let ctx=auth_support::ctx();
    assert!(nwui_api::auth::principal::require_gateway(&db.state,&headers,&ctx,&[]).await.is_ok());
    let app=nwui_api::app::finish_router(nwui_api::app::with_request_context(nwui_api::nodes::routes::router().with_state(db.state.clone()),None),&db.state.config.version);
    let response=app.oneshot(request("GET","/api/v1/info",Some(&db.state.config.version),Some("gateway-secret"),"")).await.unwrap();
    assert_eq!(response.status(),StatusCode::OK);
    assert_eq!(response.headers()["x-node-version"],db.state.config.version);
    for change in ["enabled=false","enabled=true,approved=false","approved=true,role='editor'"] {
        db.client.batch_execute(&format!("UPDATE users SET {change} WHERE id='u1'")).await.unwrap();
        assert!(nwui_api::auth::principal::require_gateway(&db.state,&headers,&ctx,&[]).await.is_err(),"{change}");
    }
    db.client.batch_execute("DELETE FROM users WHERE id='u1'").await.unwrap();
    assert!(nwui_api::auth::principal::require_gateway(&db.state,&headers,&ctx,&[]).await.is_err());
}

#[tokio::test]
#[ignore = "real DB"]
async fn q3_i5_anonymous_node_response_has_no_version() {
    let db=auth_support::test_db().await;
    let app=nwui_api::app::finish_router(nwui_api::app::with_request_context(nwui_api::nodes::routes::router().with_state(db.state.clone()),None),&db.state.config.version);
    let response=app.oneshot(request("GET","/api/v1/info",Some(&db.state.config.version),None,"")).await.unwrap();
    assert_eq!(response.status(),StatusCode::UNAUTHORIZED);
    assert!(!response.headers().contains_key("x-node-version"));
}

#[tokio::test]
#[ignore = "real DB"]
async fn q3_i5_viewers_do_not_receive_node_addresses_or_transport_errors() {
    use nwui_api::auth::principal::SessionPrincipal;
    use nwui_api::contracts::Role;
    let db=auth_support::test_db().await;
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id,last_error) VALUES('n','Node','https://private-node.test','sealed','viewer','remote','private resolver error')",&[]).await.unwrap();
    let mut actor=SessionPrincipal{user_id:"viewer".into(),email:None,name:None,role:Role::Viewer,session_version:0,totp_enabled:false,requires_2fa:false,mfa_verified:false,expires_at:0};
    for path in ["nodes","nodes/n/layout"] {
        let value=nwui_api::nodes::views::view(&db.state, &(actor).clone().into(),path,&Default::default()).await.unwrap().unwrap();
        assert!(!value.to_string().contains("private-node.test"));
        assert!(!value.to_string().contains("private resolver error"));
    }
    actor.role=Role::Admin;
    let value=nwui_api::nodes::views::view(&db.state, &(actor).clone().into(),"nodes/n",&Default::default()).await.unwrap().unwrap();
    assert_eq!(value["node"]["baseUrl"],"https://private-node.test");
    assert_eq!(value["node"]["lastError"],"private resolver error");
}

#[tokio::test]
#[ignore = "real DB"]
async fn q3r1_version_mismatch_keeps_the_node_version_header() {
    let db=auth_support::test_db().await;
    let app=nwui_api::app::finish_router(nwui_api::app::with_request_context(nwui_api::nodes::routes::router().with_state(db.state.clone()),None),&db.state.config.version);
    let response=app.oneshot(request("GET","/api/v1/info",Some("0.0.0"),None,"")).await.unwrap();
    assert_eq!(response.status(),StatusCode::CONFLICT);
    assert_eq!(response.headers().get("x-node-version").and_then(|v|v.to_str().ok()),Some(db.state.config.version.as_str()));
}

#[tokio::test]
#[ignore="real DB"]
async fn q3r1_new_read_token_cannot_grant_admin() {
    let db = auth_support::test_db().await;
    nwui_api::nodes::service::ensure_instance_id(&db.state)
        .await
        .unwrap();
    auth_support::insert_user(
        &db.client,
        auth_support::UserRow {
            role: "admin",
            ..Default::default()
        },
        None,
    )
    .await;
    db.client.execute("INSERT INTO access_tokens(id,name,kind,scopes,token_hash,token_suffix,created_by_user_id) VALUES('access-id','server-secret-name','server',$1,$2,'test','u1')",&[&json!(["node:pair","config:read"]),&nwui_api::auth::token_digest("server-secret")]).await.unwrap();
    let app = nwui_api::app::with_request_context(
        nwui_api::nodes::routes::router().with_state(db.state.clone()),
        None,
    );
    let version = &db.state.config.version;

    let response=app.oneshot(request("POST","/api/v1/pair",Some(version),None,&json!({"token":"server-secret","gatewayName":"new","role":"admin"}).to_string())).await.unwrap();
    assert_eq!(response.status(),StatusCode::FORBIDDEN);
    let count:i64=db.client.query_one("SELECT count(*) FROM gateway_tokens",&[]).await.unwrap().get(0);assert_eq!(count,0);
}

#[tokio::test]
#[ignore="real DB"]
async fn q3r1_failed_gateway_insert_does_not_spend_the_server_token() {
    let db = auth_support::test_db().await;
    nwui_api::nodes::service::ensure_instance_id(&db.state)
        .await
        .unwrap();
    auth_support::insert_user(
        &db.client,
        auth_support::UserRow {
            role: "admin",
            ..Default::default()
        },
        None,
    )
    .await;
    db.client.execute("INSERT INTO access_tokens(id,name,kind,scopes,token_hash,token_suffix,created_by_user_id) VALUES('access-id','server-secret-name','server',$1,$2,'test','u1')",&[&json!(["node:pair","config:read"]),&nwui_api::auth::token_digest("server-secret")]).await.unwrap();
    let app = nwui_api::app::with_request_context(
        nwui_api::nodes::routes::router().with_state(db.state.clone()),
        None,
    );
    let version = &db.state.config.version;

    db.client.batch_execute("CREATE FUNCTION fail_gateway_insert() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test-only gateway insert failure'; END $$; CREATE TRIGGER fail_gateway_insert BEFORE INSERT ON gateway_tokens FOR EACH ROW EXECUTE FUNCTION fail_gateway_insert()").await.unwrap();
    let payload=json!({"token":"server-secret","gatewayName":"gateway","role":"viewer"}).to_string();
    let response=app.clone().oneshot(request("POST","/api/v1/pair",Some(version),None,&payload)).await.unwrap();
    assert_eq!(response.status(),StatusCode::SERVICE_UNAVAILABLE);
    let revoked:bool=db.client.query_one("SELECT revoked_at IS NOT NULL FROM access_tokens WHERE id='access-id'",&[]).await.unwrap().get(0);assert!(!revoked);
    db.client.batch_execute("DROP TRIGGER fail_gateway_insert ON gateway_tokens").await.unwrap();
    assert_eq!(app.oneshot(request("POST","/api/v1/pair",Some(version),None,&payload)).await.unwrap().status(),StatusCode::OK);
}

#[tokio::test]
#[ignore="real DB"]
async fn q3r1_migration_preserves_legacy_pairing_and_orphan_gateways() {
    let db=auth_support::test_db().await;
    auth_support::insert_user(&db.client,auth_support::UserRow{role:"admin",..Default::default()},None).await;
    // Recreate the pre-upgrade auth schema in this disposable DB; exercise the real additive SQL.
    db.client.batch_execute("ALTER TABLE access_tokens DROP COLUMN IF EXISTS legacy_pairing; ALTER TABLE gateway_tokens DROP COLUMN IF EXISTS legacy_issuer; ALTER TABLE gateway_tokens DROP COLUMN IF EXISTS access_token_id").await.unwrap();
    db.client.execute("INSERT INTO access_tokens(id,name,kind,scopes,token_hash,token_suffix,created_by_user_id) VALUES('old-access','legacy','server','[\"node:pair\"]',$1,'test','u1')",&[&nwui_api::auth::token_digest("old-access-secret")]).await.unwrap();
    db.client.execute("INSERT INTO gateway_tokens(id,name,token_hash,role) VALUES('old-gateway','legacy orphan',$1,'admin')",&[&nwui_api::auth::token_digest("old-gateway-secret")]).await.unwrap();
    let before_access:Value=db.client.query_one("SELECT to_jsonb(a) FROM access_tokens a WHERE id='old-access'",&[]).await.unwrap().get(0);
    let before_gateway:Value=db.client.query_one("SELECT to_jsonb(g) FROM gateway_tokens g WHERE id='old-gateway'",&[]).await.unwrap().get(0);
    let migration=nwui_api::migrations::read_migrations(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle")).await.unwrap()
        .into_iter().find(|migration|migration.tag.ends_with("_auth_compatibility")).unwrap();
    let sql=migration.statements.join("\n");
    db.client.batch_execute(&sql).await.unwrap();
    let after_access:Value=db.client.query_one("SELECT to_jsonb(a) - 'legacy_pairing' FROM access_tokens a WHERE id='old-access'",&[]).await.unwrap().get(0);
    let after_gateway:Value=db.client.query_one("SELECT to_jsonb(g) - 'legacy_issuer' - 'access_token_id' FROM gateway_tokens g WHERE id='old-gateway'",&[]).await.unwrap().get(0);
    assert_eq!(before_access,after_access);assert_eq!(before_gateway,after_gateway);
    let legacy:bool=db.client.query_one("SELECT legacy_pairing FROM access_tokens WHERE id='old-access'",&[]).await.unwrap().get(0);assert!(legacy);
    let mut headers=axum::http::HeaderMap::new();headers.insert("authorization","Bearer old-gateway-secret".parse().unwrap());headers.insert("x-gateway-version",db.state.config.version.parse().unwrap());
    assert!(nwui_api::auth::principal::require_gateway(&db.state,&headers,&auth_support::ctx(),&[]).await.is_ok());
    let listed=nwui_api::admin::security::view(&db.state).await.unwrap();assert!(listed.gateways.iter().any(|g|g.id=="old-gateway"));
    nwui_api::nodes::service::ensure_instance_id(&db.state).await.unwrap();
    let app=nwui_api::app::with_request_context(nwui_api::nodes::routes::router().with_state(db.state.clone()),None);
    let payload=json!({"token":"old-access-secret","gatewayName":"legacy admin","role":"admin"}).to_string();
    let response=app.oneshot(request("POST","/api/v1/pair",Some(&db.state.config.version),None,&payload)).await.unwrap();assert_eq!(response.status(),StatusCode::OK);
    assert_eq!(body(response).await["role"],"admin");
    db.client.execute("INSERT INTO gateway_tokens(id,name,token_hash,role) VALUES('new-orphan','new',$1,'admin')",&[&nwui_api::auth::token_digest("new-gateway-secret")]).await.unwrap();
    headers.insert("authorization","Bearer new-gateway-secret".parse().unwrap());
    assert!(nwui_api::auth::principal::require_gateway(&db.state,&headers,&auth_support::ctx(),&[]).await.is_err());
}

#[tokio::test]
#[ignore="real DB"]
async fn q3r1_ui_default_token_pairs_with_omitted_role_and_revokes_all_links() {
    let db=auth_support::test_db().await;
    auth_support::insert_user(&db.client,auth_support::UserRow{role:"admin",totp_enabled:true,..Default::default()},None).await;
    nwui_api::nodes::service::ensure_instance_id(&db.state).await.unwrap();
    let now=nwui_api::auth::principal::now_unix_ms()/1000;
    let claims=json!({"id":"u1","sv":3,"mfaVerified":true,"totpEnabled":true});
    let jwt=nwui_api::auth::session::encode(claims.as_object().unwrap(),auth_support::SECRET,nwui_api::auth::session::COOKIE_NAME_PLAIN,now,3600).unwrap();
    let cookie=format!("authjs.session-token={jwt}");
    let app=nwui_api::server::router(db.state.clone());
    let browser=|method:&str,path:&str,payload:&str| Request::builder().method(method).uri(path)
        .header("host","admin.example.test").header("origin","http://admin.example.test")
        .header("cookie",&cookie).header("content-type","application/x-www-form-urlencoded")
        .extension(ConnectInfo(SocketAddr::from(([127,0,0,1],40000))))
        .body(Body::from(payload.to_owned())).unwrap();
    let response=app.clone().oneshot(browser("POST","/api/ui/v1/forms/admin.tokens.issue","name=UI-default&expiry=90d&scopes=node%3Apair&scopes=config%3Aread&scopes=config%3Awrite&scopes=nginx%3Areload")).await.unwrap();
    assert_eq!(response.status(),StatusCode::OK);let issued=body(response).await;
    let token=issued["result"]["token"].as_str().unwrap();let id=issued["result"]["id"].as_str().unwrap();
    let legacy:bool=db.client.query_one("SELECT legacy_pairing FROM access_tokens WHERE id=$1",&[&id]).await.unwrap().get(0);assert!(!legacy);
    let response=app.clone().oneshot(request("POST","/api/v1/pair",Some(&db.state.config.version),None,&json!({"token":token,"gatewayName":"UI default"}).to_string())).await.unwrap();
    assert_eq!(response.status(),StatusCode::OK);
    assert_eq!(response.headers()["x-node-version"],db.state.config.version);
    let paired=body(response).await;assert_eq!(paired["role"],"editor");
    let response=app.clone().oneshot(browser("GET","/api/ui/v1/admin/tokens","")).await.unwrap();
    let listed=body(response).await;assert_eq!(listed["tokens"][0]["status"],"active");assert!(!listed["tokens"][0]["lastUsedAt"].is_null());
    let response=app.clone().oneshot(browser("POST","/api/ui/v1/forms/admin.tokens.revoke",&format!("id={id}"))).await.unwrap();assert_eq!(response.status(),StatusCode::OK);
    let response=app.oneshot(request("GET","/api/v1/info",Some(&db.state.config.version),Some(paired["token"].as_str().unwrap()),"")).await.unwrap();assert_eq!(response.status(),StatusCode::UNAUTHORIZED);
}
