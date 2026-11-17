mod auth_support;
use axum::{body::Body, extract::ConnectInfo, http::Request};
use serde_json::{Value, json};
use std::net::SocketAddr;
use tower::ServiceExt;

fn document() -> Value {
    serde_json::from_str(include_str!("../../../packages/contracts/openapi.json")).unwrap()
}
fn validator(name: &str) -> jsonschema::Validator {
    let doc = document();
    let schema = json!({"$schema":"https://json-schema.org/draft/2020-12/schema", "$ref":format!("#/components/schemas/{name}"),"components":doc["components"]});
    jsonschema::draft202012::options().should_validate_formats(true).build(&schema).unwrap()
}
#[test]
fn every_published_response_schema_compiles_without_external_resolution() {
    for name in document()["components"]["schemas"].as_object().unwrap().keys() {
        validator(name);
    }
}
#[test]
fn public_contracts_reject_secret_fields_and_invalid_dates() {
    let valid = json!({"id":"u","email":null,"role":"viewer","enabled":true,"approved":true,"hasPassword":false,"totpEnabled":false,"createdAt":"2026-09-23T00:00:00Z"});
    let schema = validator("AdminUserView");
    assert!(schema.is_valid(&valid));
    for name in ["passwordHash", "totpSecret", "tokenEnc", "clientSecretEnc"] {
        let mut leaked = valid.clone(); leaked[name] = json!("must-not-leak");
        assert!(!schema.is_valid(&leaked));
    }
    let mut bad = valid; bad["createdAt"] = json!("yesterday");
    assert!(!schema.is_valid(&bad));
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn browser_api_responses_match_the_generated_client_contract() {
    let db = auth_support::test_db().await;
    auth_support::insert_user(&db.client, auth_support::UserRow {id:"contract-admin",role:"admin",email:"contract@example.test",totp_enabled:true,..Default::default()}, Some("hash-for-test")).await;
    db.client.execute("INSERT INTO oauth_providers(id,name,issuer,client_id,client_secret_enc) VALUES('contract','Contract','https://identity.example.test','cid','sealed-test-value')",&[]).await.unwrap();
    db.client.execute("INSERT INTO notifications(id,type,severity,title) VALUES('contract','cert.issued','success','issued')",&[]).await.unwrap();
    let now = nwui_api::auth::principal::now_unix_ms()/1000;
    let claims = json!({"sub":"contract-admin","id":"contract-admin","sv":3});
    let token=nwui_api::auth::session::encode(claims.as_object().unwrap(),auth_support::SECRET,nwui_api::auth::session::COOKIE_NAME_PLAIN,now,3600).unwrap();
    let app=nwui_api::server::router(db.state.clone());
    for (path,name) in [
        ("/api/ui/v1/session","SessionView"),
        ("/api/ui/v1/account","AccountView"),
        ("/api/ui/v1/account/2fa","TwoFactorView"),
        ("/api/ui/v1/admin/users/contract-admin","AdminUserView"),
        ("/api/ui/v1/admin/security","SecurityView"),
        ("/api/ui/v1/admin/oauth/contract","ProviderDetail"),
        ("/api/ui/v1/admin/settings/general","GeneralSettingsView"),
        ("/api/ui/v1/account/tokens","TokenApiList"),
        ("/api/ui/v1/admin/tokens","TokenApiList"),
        ("/api/ui/v1/notifications","NotificationFeed"),
        ("/api/ui/v1/admin/settings/smtp","SmtpView"),
    ] {
        let response=app.clone().oneshot(Request::builder().uri(path).header("host","ui.example.test").header("cookie",format!("authjs.session-token={token}")).extension(ConnectInfo(SocketAddr::from(([127,0,0,1],12345)))).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),200,"{path}");
        let body=axum::body::to_bytes(response.into_body(),1<<20).await.unwrap();
        let value:Value=serde_json::from_slice(&body).unwrap();
        let schema=validator(name);
        let errors:Vec<_>=schema.iter_errors(&value).map(|e|e.to_string()).collect();
        assert!(errors.is_empty(),"{path} violates {name}: {errors:?}");
        if name == "AdminUserView" {
            assert_eq!(value["totpEnabled"], true);
            let mut missing = value.clone(); missing.as_object_mut().unwrap().remove("totpEnabled");
            assert!(!schema.is_valid(&missing), "the factor state is required by the admin detail");
        }
    }
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn registered_local_page_projections_match_the_schema_with_real_rows() {
    let db = auth_support::test_db().await;
    auth_support::insert_user(&db.client, auth_support::UserRow {role:"admin",..Default::default()}, None).await;
    db.client.batch_execute(r#"
      INSERT INTO upstreams(id,name,servers) VALUES('item','backend','["127.0.0.1:9000"]');
      INSERT INTO certificates(id,domains,alias,status,source,cert_pem,key_pem,issue_log)
        VALUES('item','["example.test"]','Contract certificate','issued','manual','PRIVATE_CERT','PRIVATE_KEY','[{"at":"2026-09-23T00:00:00Z","level":"info","msg":"issued"}]');
      INSERT INTO nginx_servers(id,server_name,listen,upstream_id,certificate_id,config)
        VALUES('item','example.test',8081,'item','item','{"tlsPort":8443}');
      INSERT INTO nginx_streams(id,listen,protocol,proxy_pass) VALUES('item',9001,'tcp','127.0.0.1:9000');
      INSERT INTO audit_logs(id,action,target,via) VALUES('item','contract:test','item','{"kind":"token","id":"fixture","name":"<automation>"}');
      INSERT INTO config_snapshots(id,label,data) VALUES('item','contract','{"upstreams":[],"servers":[],"streams":[]}');
      INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id) VALUES('node','Test node','http://127.0.0.1:1','unreadable','admin','contract-instance');
    "#).await.unwrap();
    let claims=json!({"id":"u1","sv":3});
    let token=nwui_api::auth::session::encode(claims.as_object().unwrap(),auth_support::SECRET,nwui_api::auth::session::COOKIE_NAME_PLAIN,nwui_api::auth::principal::now_unix_ms()/1000,3600).unwrap();
    let app=nwui_api::server::router(db.state.clone());
    let doc=document();
    let registry=doc["x-view-registry"].as_object().expect("published view registry");
    assert!(registry.len()>=50);
    for (pattern,entry) in registry {
        let path=pattern.replace("{nodeId}","node").replace("{id}","item");
        let uri=format!("/api/ui/v1/views?{}",url::form_urlencoded::Serializer::new(String::new()).append_pair("path",&path).finish());
        let response=app.clone().oneshot(Request::builder().uri(uri).header("host","ui.example.test").header("cookie",format!("authjs.session-token={token}")).extension(ConnectInfo(SocketAddr::from(([127,0,0,1],12345)))).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),200,"{pattern}: {path}");
        let bytes=axum::body::to_bytes(response.into_body(),1<<20).await.unwrap();
        let body:Value=serde_json::from_slice(&bytes).unwrap();
        let schema=json!({"$schema":"https://json-schema.org/draft/2020-12/schema","allOf":[entry["schema"]],"components":doc["components"]});
        let checker=jsonschema::draft202012::options().should_validate_formats(true).build(&schema).unwrap();
        let errors:Vec<_>=checker.iter_errors(&body).map(|e|e.to_string()).collect();
        assert!(errors.is_empty(),"{pattern}: {errors:?}; body={body}");
        assert!(!String::from_utf8_lossy(&bytes).contains("PRIVATE_"),"secret leak at {path}");
    }
    nwui_api::nodes::service::ensure_instance_id(&db.state).await.unwrap();
    auth_support::gateway_issuer(&db.client).await;
    db.client.execute("INSERT INTO gateway_tokens(id,name,token_hash,role,created_by_user_id) VALUES('schema-gateway','Schema test',$1,'admin','q3-gateway-issuer')",&[&nwui_api::auth::token_digest("schema-test-token")]).await.unwrap();
    for (pattern,operations) in doc["paths"].as_object().unwrap() {
        if !pattern.starts_with("/api/v1/") { continue; }
        let Some(schema)=operations.pointer("/get/responses/200/content/application~1json/schema") else { continue; };
        let path=pattern.replace("{id}","item");
        let response=app.clone().oneshot(Request::builder().uri(&path).header("host","ui.example.test")
            .header("authorization","Bearer schema-test-token").header("x-gateway-version",&db.state.config.version)
            .extension(ConnectInfo(SocketAddr::from(([127,0,0,1],12345)))).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),200,"node {path}");
        let bytes=axum::body::to_bytes(response.into_body(),1<<20).await.unwrap();
        let body:Value=serde_json::from_slice(&bytes).unwrap();
        let wrapped=json!({"$schema":"https://json-schema.org/draft/2020-12/schema","allOf":[schema],"components":doc["components"]});
        let checker=jsonschema::draft202012::options().should_validate_formats(true).build(&wrapped).unwrap();
        let errors:Vec<_>=checker.iter_errors(&body).map(|e|e.to_string()).collect();
        assert!(errors.is_empty(),"node {pattern}: {errors:?}; body={body}");
        assert!(!String::from_utf8_lossy(&bytes).contains("PRIVATE_"),"secret leak at {path}");
    }
}

const BROWSER_AUDIT_PATHS: [(&str, &str, &str); 7] = [
    ("/api/ui/v1/metrics", "get", "application/json"),
    ("/api/nodes/{id}/metrics", "get", "application/json"),
    ("/api/ui/v1/logs/stream", "get", "text/event-stream"),
    ("/api/nodes/{id}/logs/stream", "get", "text/event-stream"),
    ("/api/certificates/{id}/cancel", "post", "application/json"),
    ("/api/ui/v1/certificates/{id}/cancel", "post", "application/json"),
    ("/api/nodes/{id}/certificates/{cid}/cancel", "post", "application/json"),
];

fn response_validator(schema: &Value) -> jsonschema::Validator {
    let doc = document();
    let wrapped = json!({"$schema":"https://json-schema.org/draft/2020-12/schema", "allOf":[schema], "components":doc["components"]});
    jsonschema::draft202012::options().should_validate_formats(true).build(&wrapped).unwrap()
}

#[test]
fn browser_metrics_logs_and_cancel_routes_are_contractual() {
    let doc = document();
    let mut operation_ids = std::collections::HashSet::new();
    for (path, method, content_type) in BROWSER_AUDIT_PATHS {
        let operation = &doc["paths"][path][method];
        assert!(operation.is_object(), "browser endpoint missing from OpenAPI: {method} {path}");
        let id = operation["operationId"].as_str().expect("generated client operation name");
        assert!(operation_ids.insert(id), "distinct browser operations: {id}");
        assert_eq!(operation["security"], json!([{"browserSession":[]},{"userToken":[]}]), "{path}");
        let success = &operation["responses"]["200"]["content"][content_type]["schema"];
        assert!(success.is_object(), "missing {content_type} response for {path}");
        if method == "post" {
            let success = response_validator(success);
            assert!(success.is_valid(&json!({"ok":true})), "browser cancel uses ok, not the node cancelled envelope");
            assert!(!success.is_valid(&json!({"cancelled":true})));
            let failure = response_validator(&operation["responses"]["default"]["content"]["application/json"]["schema"]);
            for body in [json!({"ok":false,"error":"refused"}), json!({"ok":false,"error":"offline","code":"unreachable"}), json!({"error":{"code":"session_required","message":"sign in"}})] {
                assert!(failure.is_valid(&body), "{path} must type every real error envelope: {body}");
            }
        } else if content_type == "text/event-stream" {
            assert_eq!(success["type"], "string", "SSE carries raw log data, not a JSON response");
        } else {
            assert_eq!(success["$ref"], "#/components/schemas/MetricsResponse");
        }
    }
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn browser_stream_and_cancel_errors_match_the_documented_envelopes() {
    let db = auth_support::test_db().await;
    let app = nwui_api::server::router(db.state.clone());
    let doc = document();
    for (pattern, method, _) in BROWSER_AUDIT_PATHS {
        let path = pattern.replace("{id}", "unknown-node").replace("{cid}", "unknown-cert");
        let response = app.clone().oneshot(Request::builder().method(method.to_uppercase().as_str()).uri(path)
            .header("host", "ui.example.test").header("origin", "http://ui.example.test")
            .extension(ConnectInfo(SocketAddr::from(([127,0,0,1],12345)))).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), 401, "{method} {pattern} must require a browser session");
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        let response = &doc["paths"][pattern][method]["responses"]["default"];
        let response = response["$ref"].as_str().map_or(response, |reference| doc.pointer(reference.strip_prefix('#').unwrap()).unwrap());
        let schema = &response["content"]["application/json"]["schema"];
        assert!(schema.is_object(), "missing browser error contract for {pattern}");
        assert!(response_validator(schema).is_valid(&body), "{pattern}: {body}");
    }
}

#[test]
fn http2_contract_is_optional_strict_boolean_in_tls_options() {
    let options=validator("NodeTlsOptions");
    for value in [json!({}),json!({"http2":true}),json!({"http2":false})] {assert!(options.is_valid(&value));}
    for value in [Value::Null,json!(0),json!("true"),json!({}),json!([])] {assert!(!options.is_valid(&json!({"http2":value})));}
    for name in ["NodeServerInput","NodeServerConfig"] {
        assert!(document()["components"]["schemas"][name].to_string().contains("NodeTlsOptions"));
    }
}

#[test]
fn audit_provenance_accepts_old_browser_rows_and_rejects_unknown_token_fields() {
    for name in ["AuditRowView","AuditRecordView","NodeAuditRow","NodeAuditDetail"] {
        let schema=validator(name);
        let mut row=json!({"id":"audit","createdAt":"2026-11-17T00:00:00Z","action":"config:write","target":null,"actorEmail":"owner@example.test","actorLabel":"owner@example.test","ip":null});
        if matches!(name,"AuditRecordView"|"NodeAuditDetail") { row["before"]=Value::Null; row["after"]=Value::Null; }
        assert!(schema.is_valid(&row),"{name}: older rows may omit via");
        row["via"]=Value::Null; assert!(schema.is_valid(&row),"{name}: browser via is null");
        row["via"]=json!({"kind":"token","id":"id","name":"<token>"}); assert!(schema.is_valid(&row),"{name}: token via");
        row["via"]["rawToken"]=json!("must-not-appear"); assert!(!schema.is_valid(&row),"{name}: extra via fields rejected");
    }
}

#[test]
fn null_tls_server_input_contract_preserves_plain_server_requests() {
    let schema=&document()["components"]["schemas"]["NodeServerInput"]["properties"]["tls"];
    let doc=document();let full=json!({"$ref":"#/components/schemas/NodeServerInput","components":doc["components"]});
    let check=jsonschema::validator_for(&full).unwrap();
    let base=json!({"serverName":"plain.test","listen":18080,"proxyPass":"http://127.0.0.1:9000","tls":null});
    assert!(check.is_valid(&base),"{schema}");
    for bad in [json!(5),json!("x"),json!([])] {let mut value=base.clone();value["tls"]=bad;assert!(!check.is_valid(&value));}
}
