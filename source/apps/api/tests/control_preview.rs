use nwui_api::{
    config::RuntimeConfig,
    control::{resources::Resource, views},
};
use serde_json::json;
mod auth_support;
#[test]
fn template_previews_preserve_nexus_and_gitlab_directives() {
    let root = tempfile::tempdir().unwrap();
    let mut config = RuntimeConfig::read(|k| match k {
        "DATABASE_URL" => Some("postgres://test@localhost/test".into()),
        "AUTH_SECRET" => Some("test-secret".into()),
        _ => None,
    })
    .unwrap();
    config.target_dns.descriptor_path = root.path().join("target-dns.json");
    let previews = views::template_previews(&config);
    assert_eq!(previews.as_object().unwrap().len(), 5);
    for (_, preview) in previews.as_object().unwrap() {
        assert!(preview["error"].is_null(), "{preview}");
    }
    let combined = previews["nexus-combined"]["config"].as_str().unwrap();
    assert!(combined.contains("location /v2/"));
    assert!(combined.contains("proxy_http_version 1.1;"));
    assert!(combined.contains("client_max_body_size 0;"));
    let gitlab = previews["gitlab"]["config"].as_str().unwrap();
    assert!(gitlab.contains("proxy_set_header Host $http_host;"));
    assert!(gitlab.contains("proxy_read_timeout 3600s;"));
    assert!(
        previews["nexus"]["config"]
            .as_str()
            .unwrap()
            .contains("client_max_body_size 1g;")
    );
}
#[test]
fn preview_uses_instance_certificate_paths_and_only_referenced_upstreams() {
    let root = tempfile::tempdir().unwrap();
    let mut config = RuntimeConfig::read(|k| match k {
        "DATABASE_URL" => Some("postgres://test@localhost/test".into()),
        "AUTH_SECRET" => Some("test-secret".into()),
        "CERTS_DIR" => Some("/instance/certificates".into()),
        _ => None,
    })
    .unwrap();
    config.target_dns.descriptor_path = root.path().join("target-dns.json");
    let server = json!({"id":"server","serverName":"example.test","listen":8081,"enabled":false,"upstreamId":"used","certificateId":"cert","config":{"tlsPort":8443}});
    let result = views::render_preview(
        &config,
        Resource::Server,
        &server,
        &[
            json!({"id":"unused","name":"unused","servers":["bad;"]}),
            json!({"id":"used","name":"backend","servers":["127.0.0.1:9000"]}),
        ],
        &[json!({"id":"cert","tlsReady":true})],
    );
    assert!(result["error"].is_null(), "{result}");
    let text = result["config"].as_str().unwrap();
    assert!(text.contains("upstream backend"));
    assert!(!text.contains("unused"));
    assert!(text.contains("/instance/certificates/cert/fullchain.pem"));
    assert!(text.contains("listen 8443 ssl"));
}
#[test]
fn malformed_preview_is_reported_without_losing_resource_details() {
    let root = tempfile::tempdir().unwrap();
    let mut config = RuntimeConfig::read(|k| match k {
        "DATABASE_URL" => Some("postgres://test@localhost/test".into()),
        "AUTH_SECRET" => Some("test-secret".into()),
        _ => None,
    })
    .unwrap();
    config.target_dns.descriptor_path = root.path().join("target-dns.json");
    let stream =
        json!({"id":"stream","listen":9000,"enabled":true,"protocol":"tcp","proxyPass":"bad;"});
    let result = views::render_preview(&config, Resource::Stream, &stream, &[], &[]);
    assert!(result["config"].is_null());
    assert!(result["error"].is_string());
}

#[tokio::test]
#[ignore = "real DB: NWUI_TEST_DATABASE_URL"]
async fn browser_and_node_detail_envelopes_render_with_single_connection_and_no_secrets() {
    use auth_support::*;
    use axum::{body::Body, extract::ConnectInfo, http::Request};
    use tower::ServiceExt;
    let db = test_db_with(&[("API_DB_POOL_SIZE", "1"), ("CERTS_DIR", "/target/certs")]).await;
    insert_user(&db.client, UserRow::default(), None).await;
    db.client.batch_execute("INSERT INTO certificates (id,domains,status,cert_pem,key_pem) VALUES ('cert','[\"example.test\"]','issued','PRIVATE_CERT_MATERIAL','PRIVATE_KEY_MATERIAL'); INSERT INTO upstreams (id,name,servers) VALUES ('up','backend','[\"127.0.0.1:9000\"]'); INSERT INTO nginx_servers (id,server_name,listen,enabled,upstream_id,certificate_id,config) VALUES ('server','example.test',8081,false,'up','cert','{\"tlsPort\":8443}'); INSERT INTO nginx_streams (id,listen,protocol,proxy_pass,enabled) VALUES ('stream',9001,'tcp','127.0.0.1:9000',true)").await.unwrap();
    auth_support::gateway_issuer(&db.client).await;
    db.client.execute("INSERT INTO gateway_tokens (id,name,token_hash,role,created_by_user_id) VALUES ('gateway','test',$1,'viewer','q3-gateway-issuer')",&[&nwui_api::auth::token_digest("gateway-secret")]).await.unwrap();
    let cookie = nwui_api::auth::session::encode(
        json!({"id":"u1","sv":3}).as_object().unwrap(),
        SECRET,
        nwui_api::auth::session::COOKIE_NAME_PLAIN,
        nwui_api::auth::principal::now_unix_ms() / 1000,
        3600,
    )
    .unwrap();
    for (path, browser) in [
        ("/api/ui/v1/views?path=%2Freverse-proxy%2Fserver", true),
        ("/api/v1/servers/server", false),
        ("/api/ui/v1/views?path=%2Fstream%2Fstream", true),
        ("/api/v1/streams/stream", false),
        ("/api/ui/v1/views?path=%2Freverse-proxy%2Ftemplates", true),
    ] {
        let mut req = Request::builder()
            .uri(path)
            .header("host", "admin.example.test")
            .extension(ConnectInfo(
                "127.0.0.1:45000".parse::<std::net::SocketAddr>().unwrap(),
            ));
        if browser {
            req = req.header(
                "cookie",
                format!("{}={cookie}", nwui_api::auth::session::COOKIE_NAME_PLAIN),
            );
        } else {
            req = req
                .header("authorization", "Bearer gateway-secret")
                .header("x-gateway-version", &db.state.config.version);
        }
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            nwui_api::server::router(db.state.clone()).oneshot(req.body(Body::empty()).unwrap()),
        )
        .await
        .expect("single pool connection must not deadlock")
        .unwrap();
        assert_eq!(response.status(), 200, "{path}");
        let bytes = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("PRIVATE_"));
        if path.contains("templates") {
            assert_eq!(body["canWrite"], true);
            assert_eq!(body["previews"].as_object().unwrap().len(), 5);
            assert!(body["previews"]["nexus"]["config"].as_str().unwrap().contains("proxy_pass http://127.0.0.1:8081;"));
            continue;
        }
        let preview = body["preview"]["config"].as_str().expect("preview config");
        assert!(body["preview"]["error"].is_null());
        if path.contains("server") {
            assert!(preview.contains("/target/certs/cert/fullchain.pem"));
            assert!(preview.contains("upstream backend"));
            assert_eq!(body["server"]["enabled"], false);
        } else {
            assert!(preview.contains("listen 9001"));
        }
    }
}
