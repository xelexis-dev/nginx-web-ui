mod auth_support;
use auth_support::*;
use axum::{body::Body, extract::ConnectInfo, http::Request};
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
#[ignore = "real DB: NWUI_TEST_DATABASE_URL"]
async fn status_requires_gateway_and_reports_counts_pending_and_nginx_validation() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("nwui-status-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("nginx");
    std::fs::write(
        &bin,
        "#!/bin/sh\n[ \"$1\" = \"-t\" ] || exit 99\necho config-valid\n",
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    let db = test_db_with(&[
        ("NGINX_BIN", bin.to_str().unwrap()),
        ("API_DB_POOL_SIZE", "1"),
    ])
    .await;
    auth_support::gateway_issuer(&db.client).await;
    db.client.execute("INSERT INTO gateway_tokens (id,name,token_hash,role,created_by_user_id) VALUES ('gateway','test',$1,'viewer','q3-gateway-issuer')",&[&nwui_api::auth::token_digest("gateway-secret")]).await.unwrap();
    db.client.batch_execute("INSERT INTO nginx_servers (id,server_name,listen,config) VALUES ('server','example.test',8080,'{\"proxyPass\":\"http://backend\"}'); INSERT INTO certificates (id,domains,expires_at) VALUES ('soon','[]',now()+interval '20 days'),('later','[]',now()+interval '40 days'),('unknown','[]',null)").await.unwrap();
    let app = nwui_api::server::router(db.state.clone());
    for (auth, status) in [(false, 401), (true, 200)] {
        let mut req = Request::builder()
            .uri("/api/v1/status")
            .header("host", "admin.example.test")
            .header("x-gateway-version", &db.state.config.version)
            .extension(ConnectInfo(
                "127.0.0.1:45000".parse::<std::net::SocketAddr>().unwrap(),
            ));
        if auth {
            req = req.header("authorization", "Bearer gateway-secret");
        }
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            app.clone().oneshot(req.body(Body::empty()).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.status(), status);
        if auth {
            let bytes = axum::body::to_bytes(response.into_body(), 65536)
                .await
                .unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["semver"], db.state.config.version);
            assert_eq!(
                body["counts"],
                serde_json::json!({"servers":1,"streams":0,"upstreams":0})
            );
            assert_eq!(
                body["certificates"],
                serde_json::json!({"total":3,"expiringSoon":1})
            );
            assert_eq!(body["pending"]["count"], 1);
            assert_eq!(body["pending"]["hasBaseline"], false);
            assert_eq!(body["nginx"]["configOk"], true);
            assert!(
                body["nginx"]["output"]
                    .as_str()
                    .unwrap()
                    .contains("config-valid")
            );
        }
    }
    std::fs::write(&bin, "#!/bin/sh\necho config-invalid\nexit 1\n").unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/status")
                .header("host", "admin.example.test")
                .header("x-gateway-version", &db.state.config.version)
                .header("authorization", "Bearer gateway-secret")
                .extension(ConnectInfo(
                    "127.0.0.1:45000".parse::<std::net::SocketAddr>().unwrap(),
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["nginx"]["configOk"], false);
    assert!(
        body["nginx"]["output"]
            .as_str()
            .unwrap()
            .contains("config-invalid")
    );
    std::fs::remove_dir_all(dir).unwrap();
}
