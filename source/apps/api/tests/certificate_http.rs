use nwui_api::certificates::views;
use serde_json::{Value, json};

#[test]
fn certificate_projection_allowlists_nested_dns_records_and_never_returns_secrets() {
    let raw = json!({"id":"c","domains":["example.test"],"alias":"edge","source":"acme","status":"issued","staging":false,"challenge_type":"dns-01","dns_provider":"cloudflare","dns_provider_token_enc":"token-secret","key_pem":"key-secret","cert_pem":"pem-body","dns_order":{"records":[{"domain":"example.test","recordName":"_acme-challenge.example.test","recordValue":"txt","keyPem":"record-secret"}],"submittedAt":"2026-09-01T00:00:00Z","csrPem":"csr-secret","orderUrl":"order-secret"},"issue_log":[{"at":"2026-09-01T00:00:00Z","level":"info","msg":"ready","unexpected":"log-secret"}],"created_at":"2026-09-01T00:00:00Z","failure_count":0});
    let safe = views::certificate_view(&raw, Some("owner@example.test"));
    assert_eq!(safe["alias"], "edge");
    assert_eq!(safe["hasDnsToken"], true);
    assert_eq!(safe["acmeEmail"], "owner@example.test");
    assert_eq!(safe["dnsSubmittedAt"], "2026-09-01T00:00:00Z");
    let text = safe.to_string();
    for forbidden in [
        "secret",
        "keyPem",
        "certPem",
        "dnsOrder",
        "orderUrl",
        "csrPem",
        "dnsProviderTokenEnc",
    ] {
        assert!(!text.contains(forbidden), "{forbidden}");
    }
}

#[path = "auth_support/mod.rs"]
mod auth_support;
mod g13_support;
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use std::net::SocketAddr;

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn public_challenge_aliases_return_only_active_key_authorization_without_cache() {
    let db = auth_support::test_db().await;
    db.client.execute("INSERT INTO acme_challenges(token,key_authorization) VALUES('valid_Token-123','valid_Token-123.thumbprint')",&[]).await.unwrap();
    let app = nwui_api::certificates::routes::router().with_state(db.state.clone());
    for path in [
        "/api/acme-challenge/valid_Token-123",
        "/.well-known/acme-challenge/valid_Token-123",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["content-type"], "text/plain");
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap(),
            "valid_Token-123.thumbprint"
        );
    }
    for path in [
        "/api/acme-challenge/missing",
        "/api/acme-challenge/invalid.token",
    ] {
        assert_eq!(
            app.clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    db.client
        .execute("DELETE FROM acme_challenges", &[])
        .await
        .unwrap();
    assert_eq!(
        app.oneshot(
            Request::builder()
                .uri("/api/acme-challenge/valid_Token-123")
                .body(Body::empty())
                .unwrap()
        )
        .await
        .unwrap()
        .status(),
        StatusCode::NOT_FOUND
    );
}
use tower::ServiceExt;
fn node_request(
    state: &nwui_api::state::AppState,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "admin.example.test")
        .header("x-gateway-version", &state.config.version)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))))
        .body(Body::from(body.to_string()))
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
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn node_certificate_routes_project_safe_metadata_enforce_role_and_queue_cancel_without_ca() {
    let db = auth_support::test_db().await;
    for role in ["viewer", "editor"] {
        auth_support::gateway_issuer(&db.client).await;
        db.client.execute("INSERT INTO gateway_tokens(id,name,role,token_hash,created_by_user_id) VALUES($1,$1,$1::text::role,$2,'q3-gateway-issuer')",&[&role,&nwui_api::auth::token_digest(role)]).await.unwrap();
    }
    let app = nwui_api::app::with_request_context(
        nwui_api::certificates::routes::router().with_state(db.state.clone()),
        None,
    );
    let input = json!({"domains":["example.test"],"email":"owner@example.test","alias":"edge"});
    let r = app
        .clone()
        .oneshot(node_request(
            &db.state,
            "POST",
            "/api/v1/certificates",
            "viewer",
            input.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = app
        .clone()
        .oneshot(node_request(
            &db.state,
            "POST",
            "/api/v1/certificates",
            "editor",
            input,
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let id = body(r).await["certificate"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let r = app
        .clone()
        .oneshot(node_request(
            &db.state,
            "GET",
            &format!("/api/v1/certificates/{id}"),
            "viewer",
            Value::Null,
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let data = body(r).await;
    assert_eq!(data["certificate"]["alias"], "edge");
    assert_eq!(data["certificate"]["acmeEmail"], "owner@example.test");
    assert!(data["certificate"].get("keyPem").is_none());
    let r = app
        .clone()
        .oneshot(node_request(
            &db.state,
            "POST",
            &format!("/api/v1/certificates/{id}/cancel"),
            "editor",
            Value::Null,
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(
        db.client
            .query_one(
                "SELECT cancel_requested FROM certificate_jobs WHERE certificate_id=$1",
                &[&id]
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
    let r = app
        .oneshot(node_request(
            &db.state,
            "GET",
            "/api/v1/certificates/missing",
            "viewer",
            Value::Null,
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

fn principal(role: nwui_api::contracts::Role) -> nwui_api::auth::principal::SessionPrincipal {
    nwui_api::auth::principal::SessionPrincipal {
        user_id: "u1".into(),
        email: Some("me@example.test".into()),
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
    let claims = json!({"id":"u1","sub":"u1","sv":3,"requires2fa":false});
    let token = nwui_api::auth::session::encode(
        claims.as_object().unwrap(),
        auth_support::SECRET,
        nwui_api::auth::session::COOKIE_NAME_PLAIN,
        nwui_api::auth::principal::now_unix_ms() / 1000,
        3600,
    )
    .unwrap();
    format!("authjs.session-token={token}")
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn browser_views_resolve_saved_email_dns_phase_link_count_and_read_authority() {
    use nwui_api::contracts::Role;
    let db = auth_support::test_db().await;
    let directory = nwui_api::certificates::protocol::resolve_directory_url(false, None);
    db.client.execute("INSERT INTO acme_accounts(id,directory_url,email,account_key_pem) VALUES('a',$1,'saved@example.test','account-secret')",&[&directory]).await.unwrap();
    db.client.execute("INSERT INTO certificates(id,domains,status,alias,challenge_type,dns_order,dns_provider_token_enc,key_pem,cert_pem) VALUES('c','[\"example.test\"]','issued','edge','dns-01',$1,'dns-secret','key-secret','certificate-body')",&[&json!({"records":[{"domain":"example.test","recordName":"txt","recordValue":"value"}],"submittedAt":"2026-09-01T00:00:00Z","orderUrl":"order-secret"})]).await.unwrap();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,certificate_id) VALUES('s','example.test','c')",&[]).await.unwrap();
    let query = std::collections::BTreeMap::new();
    let editor = principal(Role::Editor);
    let viewer = principal(Role::Viewer);
    let listing = views::view(&db.state, &(viewer).clone().into(), "/certificates", &query)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(listing["data"]["items"][0]["linkedCount"], 1);
    assert_eq!(listing["canWrite"], false);
    let detail = views::view(&db.state, &(viewer).clone().into(), "/certificates/c", &query)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail["data"]["dnsOrder"]["phase"], "verifying");
    assert!(!detail.to_string().contains("secret"));
    assert!(
        views::view(&db.state, &(viewer).clone().into(), "/certificates/new", &query)
            .await
            .is_err()
    );
    let new = views::view(&db.state, &(editor).clone().into(), "/certificates/new", &query)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(new["defaultEmail"], "me@example.test");
    let edit = views::view(&db.state, &(editor).clone().into(), "/certificates/c/edit", &query)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(edit["defaults"]["email"], "saved@example.test");
    assert_eq!(edit["hasStoredToken"], true);
    db.client
        .execute("UPDATE certificates SET source='manual' WHERE id='c'", &[])
        .await
        .unwrap();
    assert_eq!(
        views::view(&db.state, &(editor).clone().into(), "/certificates/c/edit", &query)
            .await
            .unwrap()
            .unwrap()["redirectTo"],
        "/certificates/c/replace"
    );
    assert_eq!(
        views::view(&db.state, &(editor).clone().into(), "/certificates/c/replace", &query)
            .await
            .unwrap()
            .unwrap(),
        json!({"id":"c"})
    );
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn browser_cancel_requires_origin_and_cookie_and_forms_keep_field_errors() {
    let db = auth_support::test_db().await;
    auth_support::insert_user(&db.client, auth_support::UserRow::default(), None).await;
    db.client
        .execute(
            "INSERT INTO certificates(id,domains,status) VALUES('c','[]','issuing')",
            &[],
        )
        .await
        .unwrap();
    let app = nwui_api::app::with_request_context(
        nwui_api::certificates::routes::router().with_state(db.state.clone()),
        None,
    );
    let request = |path: &str, origin: bool, body: &str| {
        let mut r = Request::builder()
            .method("POST")
            .uri(path)
            .header("host", "admin.example.test")
            .header("cookie", cookie())
            .header("content-type", "application/x-www-form-urlencoded")
            .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
        if origin {
            r = r.header("origin", "http://admin.example.test")
        }
        r.body(Body::from(body.to_owned())).unwrap()
    };
    assert_eq!(
        app.clone()
            .oneshot(request("/api/certificates/c/cancel", false, ""))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.clone()
            .oneshot(request("/api/certificates/c/cancel", true, ""))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let r = app
        .oneshot(request(
            "/api/ui/v1/forms/certificates.createCertificate",
            true,
            "domains=&email=bad",
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(body(r).await["result"]["fieldErrors"].is_object());
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn failed_apply_after_pem_replace_returns_422_without_claiming_material_rollback() {
    if g13_support::isolated("failed_apply_after_pem_replace_returns_422_without_claiming_material_rollback").await {return;}
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("fake-nginx");
    std::fs::write(
        &bin,
        "#!/bin/sh\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    let paths = [
        bin.to_string_lossy().to_string(),
        temp.path().join("conf").to_string_lossy().to_string(),
        temp.path().join("stream").to_string_lossy().to_string(),
        temp.path().join("certs").to_string_lossy().to_string(),
    ];
    let mut db = auth_support::test_db_with(&[
        ("NGINX_BIN", &paths[0]),
        ("NGINX_CONF_D", &paths[1]),
        ("NGINX_STREAM_D", &paths[2]),
        ("CERTS_DIR", &paths[3]),
    ])
    .await;
    g13_support::witness(&mut db.state);
    auth_support::gateway_issuer(&db.client).await;
    db.client.execute("INSERT INTO gateway_tokens(id,name,role,token_hash,created_by_user_id) VALUES('editor','editor','editor',$1,'q3-gateway-issuer')",&[&nwui_api::auth::token_digest("editor")]).await.unwrap();
    let app = nwui_api::app::with_request_context(
        nwui_api::certificates::routes::router().with_state(db.state.clone()),
        None,
    );
    let initial = rcgen::generate_simple_self_signed(vec!["example.com".into()]).unwrap();
    let imported = app
        .clone()
        .oneshot(node_request(
            &db.state,
            "POST",
            "/api/v1/certificates/import",
            "editor",
            json!({"certPem":initial.cert.pem(),"keyPem":initial.signing_key.serialize_pem()}),
        ))
        .await
        .unwrap();
    assert_eq!(imported.status(), StatusCode::CREATED);
    let id = body(imported).await["certificate"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES('s','example.com',8081,$1,$2)",&[&id,&json!({"proxyPass":"http://127.0.0.1:9000"})]).await.unwrap();
    // Saved-only linkage must not publish the server or write/reload live material.
    let saved_only = rcgen::generate_simple_self_signed(vec!["example.com".into()]).unwrap();
    let saved = app.clone().oneshot(node_request(
        &db.state, "POST", &format!("/api/v1/certificates/{id}/replace"), "editor",
        json!({"certPem":saved_only.cert.pem(),"keyPem":saved_only.signing_key.serialize_pem()}),
    )).await.unwrap();
    assert_eq!(saved.status(), StatusCode::OK);
    assert!(!db.state.config.nginx_conf_dir.join("server-s.conf").exists());
    assert!(!db.state.config.certs_dir.join(&id).exists());
    assert!(!temp.path().join(".fake-nginx-active-generation").exists());
    assert_eq!(db.client.query_one("SELECT count(*) FROM applied_config", &[]).await.unwrap().get::<_, i64>(0), 0);
    assert_eq!(db.client.query_one("SELECT count(*) FROM config_snapshots", &[]).await.unwrap().get::<_, i64>(0), 0);
    // Establish a real live consumer before injecting a deploy failure.
    let applied = g13_support::baseline(&db.state).await.unwrap();
    assert!(applied.ok, "{}", applied.output);
    assert!(db.state.config.nginx_conf_dir.join("server-s.conf").exists());
    let fault=temp.path().join("fail-deploy-once");std::fs::write(&fault,"armed").unwrap();
    std::fs::write(&bin,format!("#!/bin/sh\nif [ -f '{}' ]; then rm '{}'; echo 'fixture config rejected' >&2; exit 1; fi\nif [ \"$1\" = -T ]; then printf '# configuration file %s:\\n' '{}'; fi\nexit 0\n",fault.display(),fault.display(),db.state.config.nginx_main_conf.display())).unwrap();
    let material = rcgen::generate_simple_self_signed(vec!["www.example.com".into()]).unwrap();
    let replaced = app
        .oneshot(node_request(
            &db.state,
            "POST",
            &format!("/api/v1/certificates/{id}/replace"),
            "editor",
            json!({"certPem":material.cert.pem(),"keyPem":material.signing_key.serialize_pem()}),
        ))
        .await
        .unwrap();
    assert_eq!(replaced.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body(replaced).await["error"]["code"], "apply_failed");
    let stored = nwui_api::certificates::store::get(&db.client, &id, false)
        .await
        .unwrap();
    assert_eq!(stored.names(), ["www.example.com"]);
    assert!(
        db.state.secrets.open(stored.key_pem.as_deref()).as_deref()
            == Some(material.signing_key.serialize_pem().trim())
    );
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn certificate_view_projection_matches_full_row_bytes() {
    let db = auth_support::test_db().await;
    for (staging, email) in [(false, "production@example.test"), (true, "staging@example.test")] {
        let directory = nwui_api::certificates::protocol::resolve_directory_url(staging, None);
        db.client.execute("INSERT INTO acme_accounts(id,directory_url,email,account_key_pem) VALUES($1,$2,$1,'account-secret')", &[&email, &directory]).await.unwrap();
    }
    db.client.batch_execute("INSERT INTO certificates(id,domains) VALUES('nulls','[]'),('empty-token','[\"empty.test\"]'); UPDATE certificates SET dns_provider_token_enc='' WHERE id='empty-token';").await.unwrap();
    db.client.execute("INSERT INTO certificates(id,domains,alias,status,source,staging,challenge_type,dns_provider,dns_provider_token_enc,dns_order,issued_at,expires_at,last_attempt_at,renewal_notice_sent_at,created_at,last_error,failure_count,issue_log,cert_pem,key_pem) VALUES('populated','[\"example.test\",\"www.example.test\"]','edge','issued','acme',true,'dns-01','cloudflare','sealed',$1,'2026-09-01T00:00:00.123Z','2026-12-01T00:00:00Z','2026-09-02T00:00:00Z','2026-09-03T00:00:00Z','2026-09-01T01:00:00Z','attempt failed',3,$2,'pem-secret','key-secret')",&[
        &json!({"records":[{"domain":"example.test","recordName":"_acme-challenge.example.test","recordValue":"txt","extra":"secret"}],"submittedAt":"2026-09-01T00:00:00Z","extra":"secret"}),
        &json!([{"at":"2026-09-01T00:00:00Z","level":"info","msg":"ready","extra":"secret"},{"at":"2026-09-02T00:00:00Z","level":"error","msg":"failed"}]),
    ]).await.unwrap();
    let reference = db.client.query("SELECT to_jsonb(c) FROM certificates c ORDER BY created_at DESC,id DESC", &[]).await.unwrap().into_iter().map(|row| {
        let raw: Value = row.get(0);
        views::certificate_view(&raw, Some(if raw["staging"] == true { "staging@example.test" } else { "production@example.test" }))
    }).collect::<Vec<_>>();
    assert_eq!(serde_json::to_vec(&views::list(&db.state).await.unwrap()).unwrap(), serde_json::to_vec(&reference).unwrap());
    let viewer = principal(nwui_api::contracts::Role::Viewer);
    let query = std::collections::BTreeMap::new();
    let items = reference.iter().map(|c| {
        let names = c["domains"].as_array().unwrap().iter().map(|v|v.as_str().unwrap()).collect::<Vec<_>>();
        json!({"id":c["id"],"domains":{"primary":names.first(),"all":names.join(", "),"extra":names.len().saturating_sub(1)},"alias":c["alias"],"status":c["status"],"source":c["source"],"staging":c["staging"],"expiresAt":c["expiresAt"],"linkedCount":0})
    }).collect::<Vec<_>>();
    assert_eq!(serde_json::to_vec(&views::view(&db.state, &(viewer).clone().into(),"/certificates",&query).await.unwrap().unwrap()).unwrap(), serde_json::to_vec(&json!({"data":{"items":items},"canWrite":false})).unwrap());
    for expected in &reference {
        let id = expected["id"].as_str().unwrap();
        let raw: Value = db.client.query_one("SELECT to_jsonb(c) FROM certificates c WHERE id=$1", &[&id]).await.unwrap().get(0);
        let old_detail = views::certificate_view(&raw, Some(if raw["staging"] == true { "staging@example.test" } else { "production@example.test" }));
        assert_eq!(serde_json::to_vec(&views::detail(&db.state,id).await.unwrap()).unwrap(), serde_json::to_vec(&old_detail).unwrap());
        let mut data = old_detail.clone();
        let names = old_detail["domains"].as_array().unwrap().iter().map(|v|v.as_str().unwrap()).collect::<Vec<_>>();
        data["domains"] = json!({"primary":names.first(),"all":names.join(", "),"extra":names.len().saturating_sub(1)});
        data["linkedServers"] = json!([]);
        data["dnsOrder"] = if id == "populated" { json!({"phase":"verifying"}) } else { Value::Null };
        data["issueLog"] = json!(old_detail["issueLog"].as_array().unwrap().iter().map(|r|json!({"at":r["at"].as_str().unwrap().replace('T'," ").chars().take(19).collect::<String>(),"level":r["level"],"msg":r["msg"]})).collect::<Vec<_>>());
        let actual = views::view(&db.state, &(viewer).clone().into(),&format!("/certificates/{id}"),&query).await.unwrap().unwrap();
        assert_eq!(serde_json::to_vec(&actual).unwrap(), serde_json::to_vec(&json!({"data":data,"canWrite":false,"uncovered":[]})).unwrap());
    }
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn certificate_views_need_no_pem_column_privileges() {
    let db = auth_support::test_db_with(&[("API_DB_POOL_SIZE", "1")]).await;
    db.client.batch_execute("INSERT INTO certificates(id,domains,cert_pem,key_pem) VALUES('c','[\"example.test\"]','pem-secret','key-secret')").await.unwrap();
    let role = format!("nwui_cert_view_{}", uuid::Uuid::new_v4().simple());
    // Derive the grant independently of VIEW_COLUMNS so an accidental whole-row read is rejected.
    let columns: String = db.client.query_one("SELECT string_agg(quote_ident(attname),',') FROM pg_attribute WHERE attrelid='certificates'::regclass AND attnum>0 AND NOT attisdropped AND attname NOT IN ('cert_pem','key_pem')",&[]).await.unwrap().get(0);
    db.client.batch_execute(&format!("CREATE ROLE {role} NOLOGIN; GRANT USAGE ON SCHEMA public TO {role}; GRANT SELECT ({columns}) ON certificates TO {role}; GRANT SELECT ON acme_accounts,nginx_servers TO {role}")).await.unwrap();
    {
        let client = db.state.db.get().await.unwrap();
        client.batch_execute(&format!("SET ROLE {role}")).await.unwrap();
    }
    let viewer = principal(nwui_api::contracts::Role::Viewer);
    let query = std::collections::BTreeMap::new();
    let list = views::list(&db.state).await;
    let detail = views::detail(&db.state,"c").await;
    let browser_list = views::view(&db.state, &(viewer).clone().into(),"/certificates",&query).await;
    let browser_detail = views::view(&db.state, &(viewer).clone().into(),"/certificates/c",&query).await;
    let (full_row_error, store_error) = {
        let client = db.state.db.get().await.unwrap();
        let full_row_error = client.query("SELECT to_jsonb(c) FROM certificates c",&[]).await.unwrap_err();
        let store_error = nwui_api::certificates::store::get(&**client,"c",false).await.err();
        client.batch_execute("RESET ROLE").await.unwrap();
        (full_row_error, store_error)
    };
    db.client.batch_execute(&format!("DROP OWNED BY {role}; DROP ROLE {role}")).await.unwrap();
    assert_eq!(full_row_error.code(), Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE));
    assert_eq!(store_error.expect("store::get still requires PEM column access").code,"service_unavailable");
    assert_eq!(list.unwrap().len(),1);
    assert_eq!(detail.unwrap()["id"],"c");
    assert!(browser_list.unwrap().is_some());
    assert!(browser_detail.unwrap().is_some());
}
