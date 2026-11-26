mod outbound_support;
use nwui_api::{crypto::SecretBox, nodes::client::{NodeHttp,NodeTarget,classify_response}};
use serde_json::json;
use std::{sync::{Arc,atomic::{AtomicUsize,Ordering}},time::Duration};
use axum::{Router,Json,http::{StatusCode,HeaderMap},routing::get};

#[test]
fn domain_errors_do_not_mark_a_healthy_node_offline() {
    for status in [400,403,404,409,422,429] {
        let result=classify_response(status,Some("1.17.2".into()),json!({"error":{"code":"conflict","message":"domain"}}));
        assert_eq!(result.status(),None,"{status}");
    }
    assert_eq!(classify_response(404,None,json!({})).status(),Some("version_mismatch"));
    assert_eq!(classify_response(409,Some("1.16.0".into()),json!({"error":{"code":"version_mismatch"}})).status(),Some("version_mismatch"));
}

#[tokio::test]
async fn transport_never_follows_redirects_and_keeps_credentials_out_of_errors() {
    let hits=Arc::new(AtomicUsize::new(0));
    let counter=hits.clone();
    let routes=Router::new()
        .route("/api/v1/info",get(|headers:HeaderMap|async move {
            assert_eq!(headers["x-gateway-version"],"1.17.2");
            assert_eq!(headers["authorization"],"Bearer secret-token");
            Json(json!({"ready":true}))
        }))
        .route("/api/v1/redirect",get(||async{(StatusCode::FOUND,[("location","/stolen")],"")}))
        .route("/stolen",get(move||{counter.fetch_add(1,Ordering::SeqCst);async{"bad"}}))
        .route("/api/v1/large",get(||async{"x".repeat(1024*1024+1)}))
        .route("/api/v1/logs/stream",get(||async{([("content-type","text/event-stream")],"data: hello\n\n")}))
        .route("/api/v1/slow",get(||async{tokio::time::sleep(Duration::from_secs(2)).await;"slow"}));
    let listener=tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let base=format!("http://{}",listener.local_addr().unwrap());
    let server=tokio::spawn(async move{axum::serve(listener,routes).await.unwrap()});
    let secret=SecretBox::from_key([42;32]);
    let target=NodeTarget{node_version:None,base_url:base,token_enc:Some(secret.seal("secret-token").unwrap()),allow_insecure_tls:false};
    let client=NodeHttp::new("1.17.2",&outbound_support::runtime_config()).unwrap();
    let good=client.call(&secret,&target,reqwest::Method::GET,"/api/v1/info",None,Duration::from_secs(1)).await;
    assert_eq!(serde_json::to_value(good).unwrap()["value"],json!({"ready":true}));
    for path in ["/api/v1/redirect","/api/v1/large","/api/v1/slow","//attacker.test/api/v1/info","/api/v1/../../stolen"] {
        let result=client.call(&secret,&target,reqwest::Method::GET,path,None,Duration::from_millis(100)).await;
        let serialized=serde_json::to_string(&result).unwrap();
        assert!(!serialized.contains("secret-token"));
        assert!(!result.is_ok(),"{path}");
    }
    assert_eq!(hits.load(Ordering::SeqCst),0);
    let stream=client.open_stream(&secret,&target,"/api/v1/logs/stream",Duration::from_secs(1)).await.unwrap();
    assert_eq!(stream.text().await.unwrap(),"data: hello\n\n");
    assert!(client.open_stream(&secret,&target,"/api/v1/info",Duration::from_secs(1)).await.is_err());
    assert!(client.open_stream(&secret,&target,"/api/v1/redirect",Duration::from_secs(1)).await.is_err());
    assert!(client.open_stream(&secret,&target,"/api/v1/slow",Duration::from_millis(50)).await.is_err());
    server.abort();
}

#[test]
fn audit_e6_r2_nginx_busy_uses_the_existing_visible_conflict_banner() {
    let message="설정 적용 중입니다. 잠시 후 새로고침하세요.";
    let result=classify_response(503,Some("2.0.0".into()),json!({"error":{"code":"nginx_busy","message":message}}));
    assert_eq!(result.status(),None,"a busy apply is not a persistent node failure");
    let value=serde_json::to_value(result).unwrap();
    assert_eq!(value["kind"],"conflict");assert_eq!(value["message"],message);
    assert_eq!(classify_response(503,Some("2.0.0".into()),json!({"error":{"code":"other","message":"failure"}})).status(),Some("error"));
}

#[test]
fn a_node_still_starting_its_publisher_is_not_recorded_as_broken() {
    let message="설정 적용 서비스가 아직 준비되지 않았습니다. 잠시 후 다시 시도하세요.";
    let result=classify_response(503,Some("2.0.1".into()),json!({"error":{"code":"publisher_unavailable","message":message}}));
    assert_eq!(result.status(),None);
    let value=serde_json::to_value(result).unwrap();
    assert_eq!(value["kind"],"conflict");assert_eq!(value["message"],message);
}

#[tokio::test]
async fn cached_legacy_target_uses_the_local_version_for_json_and_streams() {
    let app = Router::new().route("/api/v1/info", get(|headers: HeaderMap| async move {
        assert_eq!(headers["x-gateway-version"], "2.0.0");
        assert_eq!(headers["authorization"], "Bearer legacy-token");
        Json(json!({"semver":"1.19.11"}))
    })).route("/api/v1/logs/stream", get(|headers: HeaderMap| async move {
        assert_eq!(headers["x-gateway-version"], "2.0.0");
        ([("content-type", "text/event-stream")], "data: legacy\n\n")
    }));
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target=NodeTarget {
        base_url:format!("http://{}",listener.local_addr().unwrap()),
        node_version:Some("1.19.11".into()),
        token_enc:Some(SecretBox::from_key([42;32]).seal("legacy-token").unwrap()),
        allow_insecure_tls:false,
    };
    let server=tokio::spawn(async move { axum::serve(listener,app).await.unwrap() });
    let client=NodeHttp::new("2.0.0",&outbound_support::runtime_config()).unwrap();
    let secrets=SecretBox::from_key([42;32]);
    assert!(client.call(&secrets,&target,reqwest::Method::GET,"/api/v1/info",None,Duration::from_secs(1)).await.is_ok());
    assert_eq!(client.open_stream(&secrets,&target,"/api/v1/logs/stream",Duration::from_secs(1)).await.unwrap().text().await.unwrap(),"data: legacy\n\n");
    server.abort();
}

#[tokio::test]
async fn info_probe_is_sent_once_and_requires_authenticated_consistent_identity() {
    use axum::response::IntoResponse;
    use nwui_api::nodes::client::{NodeResult, FailureKind};
    for mode in ["legacy", "modern", "foreign", "inconsistent", "invalid", "missing", "unsupported", "malformed", "redirect", "busy", "slow"] {
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();
        let remote = Router::new().route("/api/v1/info", get(move |headers: HeaderMap| {
            seen.fetch_add(1, Ordering::SeqCst);
            async move {
                assert_eq!(headers["authorization"], "Bearer paired-secret");
                if mode == "slow" { tokio::time::sleep(Duration::from_secs(4)).await; }
                assert_eq!(headers["x-gateway-version"], "2.0.0");
                if mode == "legacy" {
                    return (StatusCode::CONFLICT, [("x-node-version", "1.19.11")], Json(json!({"error":{"code":"version_mismatch"}}))).into_response();
                }
                if mode == "redirect" { return (StatusCode::FOUND, [("location", "/stolen")]).into_response(); }
                if mode == "busy" { return StatusCode::TOO_MANY_REQUESTS.into_response(); }
                let version = match mode { "invalid" => "garbage", "unsupported" => "2.1.0", _ => "2.0.0" };
                if mode == "malformed" { return ([("x-node-version", "2.0.0")],Json(json!(null))).into_response(); }
                if mode == "missing" { return Json(json!({"instanceId":"bound"})).into_response(); }
                ([("x-node-version", version)], Json(json!({"semver":if mode=="inconsistent" {"1.19.11"} else {version}, "instanceId":if mode=="foreign" {"other"} else {"bound"}}))).into_response()
            }
        }));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target=NodeTarget {base_url:format!("http://{}", listener.local_addr().unwrap()),node_version:None,token_enc:Some(SecretBox::from_key([42;32]).seal("paired-secret").unwrap()),allow_insecure_tls:false};
        let server=tokio::spawn(async move {axum::serve(listener,remote).await.unwrap()});
        let client=NodeHttp::new("2.0.0",&outbound_support::runtime_config()).unwrap();
        let started=std::time::Instant::now();
        let result=client.negotiate_info(&SecretBox::from_key([42;32]),&target,"bound").await;
        assert_eq!(result.is_ok(), mode == "modern", "{mode}: {result:?}");
        assert_eq!(hits.load(Ordering::SeqCst), 1, "{mode}");
        if mode=="slow" {
            assert!(matches!(result,NodeResult::Failure {kind:FailureKind::Unreachable,..}));
            assert!(started.elapsed()<Duration::from_millis(3300));
        }
        server.abort();
    }
}

#[tokio::test]
async fn gateway_first_info_negotiation() {
    use axum::response::IntoResponse;
    for (gateway, node, compatible) in [
        ("2.1.0", "2.0.0", true),
        ("2.9.3", "2.1.0", true),
        ("2.0.5", "2.0.0", true),
        ("2.0.0", "2.0.9", true),
        ("3.0.0", "3.0.9", true),
        ("2.0.0", "1.19.11", false),
        ("1.19.11", "2.0.0", false),
        ("2.1.0", "1.19.11", false),
        ("1.19.11", "2.1.0", false),
        ("2.0.0", "3.0.0", false),
    ] {
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();
        let app = Router::new().route("/api/v1/info", get(move |headers: HeaderMap| {
            seen.fetch_add(1, Ordering::SeqCst);
            async move {
                assert_eq!(headers["x-gateway-version"], gateway);
                assert_eq!(headers["authorization"], "Bearer paired-secret");
                // Released 1.19 nodes refuse any gateway header outside their own family.
                if node == "1.19.11" && headers["x-gateway-version"] != "1.19.11" {
                    return (StatusCode::CONFLICT, [("x-node-version", node)], Json(json!({"error":{"code":"version_mismatch"}}))).into_response();
                }
                ([("x-node-version", node)], Json(json!({"semver":node,"instanceId":"bound"}))).into_response()
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let secrets = SecretBox::from_key([42;32]);
        let target = NodeTarget { base_url: format!("http://{}", listener.local_addr().unwrap()), node_version: None,
            token_enc: Some(secrets.seal("paired-secret").unwrap()), allow_insecure_tls: false };
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let result = NodeHttp::new(gateway, &outbound_support::runtime_config()).unwrap().negotiate_info(&secrets, &target, "bound").await;
        assert_eq!(result.is_ok(), compatible, "{gateway} -> {node}: {result:?}");
        assert_eq!(hits.load(Ordering::SeqCst), 1, "no negotiation retry for {gateway} -> {node}");
        server.abort();
    }
}
