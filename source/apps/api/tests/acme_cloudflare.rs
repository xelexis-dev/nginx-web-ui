use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::IntoResponse,
};
use nwui_api::certificates::{cloudflare::CloudflareClient, protocol::DnsChallengeRecord};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
struct Mock {
    calls: Vec<(Method, String, Value)>,
    creates: usize,
    fail_create: bool,
    fail_delete: bool,
}
async fn cf(
    State(state): State<Arc<Mutex<Mock>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    assert_eq!(headers["authorization"], "Bearer account-owned-test-token");
    let mut s = state.lock().unwrap();
    s.calls.push((
        method.clone(),
        uri.to_string(),
        serde_json::from_slice(&body).unwrap_or(Value::Null),
    ));
    let mut status = StatusCode::OK;
    let reply = if uri.path() == "/zones" {
        let query: std::collections::HashMap<_, _> =
            url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
                .into_owned()
                .collect();
        if let Some(name) = query.get("name") {
            json!({"success":true,"result":if name == "b.example.com" || name == "example.com" {vec![json!({"id":"z1","name":name})]} else {vec![]}})
        } else if query.get("page").map(String::as_str) == Some("2") {
            json!({"success":true,"result":[{"id":"z2","name":"second.example"}],"result_info":{"page":2,"total_pages":2}})
        } else {
            json!({"success":true,"result":[{"id":"z1","name":"example.com"}],"result_info":{"page":1,"total_pages":2}})
        }
    } else if method == Method::POST && uri.path() == "/zones/z1/dns_records" {
        s.creates += 1;
        if s.fail_create && s.creates == 2 {
            status = StatusCode::BAD_REQUEST;
            json!({"success":false,"errors":[{"code":1004,"message":"invalid content"}]})
        } else {
            json!({"success":true,"result":{"id":format!("r{}",s.creates)}})
        }
    } else if method == Method::DELETE && uri.path().starts_with("/zones/z1/dns_records/") {
        if s.fail_delete && uri.path().ends_with("/r1") {
            status = StatusCode::FORBIDDEN;
            json!({"success":false,"errors":[{"code":9109,"message":"Unauthorized"}]})
        } else {
            json!({"success":true,"result":{"id":"deleted"}})
        }
    } else {
        status = StatusCode::NOT_FOUND;
        json!({"success":false,"errors":[{"code":999,"message":"unexpected endpoint"}]})
    };
    (status, axum::Json(reply))
}
async fn server(
    mock: Mock,
) -> (
    CloudflareClient,
    Arc<Mutex<Mock>>,
    tokio::task::JoinHandle<()>,
) {
    let state = Arc::new(Mutex::new(mock));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = CloudflareClient::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        "account-owned-test-token",
        Duration::from_secs(2),
    )
    .unwrap();
    let app = Router::new().fallback(cf).with_state(state.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (client, state, task)
}
fn records() -> Vec<DnsChallengeRecord> {
    ["value1", "value2"]
        .into_iter()
        .map(|v| DnsChallengeRecord {
            domain: "example.com".into(),
            record_name: "_acme-challenge.example.com".into(),
            record_value: v.into(),
        })
        .collect()
}

#[tokio::test]
async fn account_owned_token_verification_uses_all_zone_pages_without_user_endpoint() {
    let (client, state, task) = server(Mock::default()).await;
    let zones = client.verify_token().await.unwrap();
    assert_eq!(zones.len(), 2);
    assert_eq!(zones[1].name, "second.example");
    assert!(
        state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(_, path, _)| path.starts_with("/zones?"))
    );
    task.abort();
}

#[tokio::test]
async fn longest_matching_zone_wins_and_challenges_coexist() {
    let (client, state, task) = server(Mock::default()).await;
    assert_eq!(
        client.find_zone("a.b.example.com.").await.unwrap().name,
        "b.example.com"
    );
    let published = client.publish_challenge_records(&records()).await.unwrap();
    assert_eq!(
        published
            .created
            .iter()
            .map(|r| r.record_id.as_str())
            .collect::<Vec<_>>(),
        vec!["r1", "r2"]
    );
    client.retract_challenge_records(&published).await.unwrap();
    let calls = &state.lock().unwrap().calls;
    let bodies: Vec<_> = calls
        .iter()
        .filter(|(method, _, _)| method == Method::POST)
        .map(|(_, _, body)| body)
        .collect();
    assert_eq!(bodies[0]["ttl"], 60);
    assert_eq!(bodies[0]["type"], "TXT");
    assert_eq!(bodies[1]["content"], "value2");
    assert_eq!(
        calls.iter().filter(|(m, _, _)| m == Method::DELETE).count(),
        2
    );
    task.abort();
}

#[tokio::test]
async fn partial_publish_reports_original_error_and_exact_records_left_after_failed_cleanup() {
    let (client, state, task) = server(Mock {
        fail_create: true,
        fail_delete: true,
        ..Default::default()
    })
    .await;
    let error = client
        .publish_challenge_records(&records())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("invalid content"));
    assert_eq!(error.cleanup_failures.len(), 1);
    assert_eq!(error.cleanup_failures[0].record.record_id, "r1");
    assert!(
        state
            .lock()
            .unwrap()
            .calls
            .iter()
            .any(|(m, p, _)| m == Method::DELETE && p.ends_with("/r1"))
    );
    task.abort();
}

#[tokio::test]
async fn cleanup_continues_after_permission_error_and_returns_failure() {
    let (client, state, task) = server(Mock {
        fail_delete: true,
        ..Default::default()
    })
    .await;
    let published = client.publish_challenge_records(&records()).await.unwrap();
    let failures = client
        .retract_challenge_records(&published)
        .await
        .unwrap_err();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].record.record_id, "r1");
    assert!(failures[0].error.to_string().contains("Zone:DNS:Edit"));
    assert_eq!(
        state
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(m, _, _)| m == Method::DELETE)
            .count(),
        2
    );
    task.abort();
}

#[tokio::test]
async fn only_the_missing_record_code_makes_deletion_idempotent() {
    for (status, code, success) in [
        (StatusCode::NOT_FOUND, 81044, true),
        (StatusCode::NOT_FOUND, 999, false),
        (StatusCode::FORBIDDEN, 81044, false),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = CloudflareClient::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            "token",
            Duration::from_secs(1),
        )
        .unwrap();
        let app = Router::new().fallback(move || async move {
            (
                status,
                axum::Json(json!({"success":false,"errors":[{"code":code,"message":"not found"}]})),
            )
        });
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        assert_eq!(
            client.delete_record("zone", "record").await.is_ok(),
            success
        );
        task.abort();
    }
}

#[tokio::test]
async fn malformed_and_empty_zone_results_do_not_verify_a_token() {
    for body in [
        "not json",
        r#"{"success":true,"result":null}"#,
        r#"{"success":true,"result":[]}"#,
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = CloudflareClient::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            "token",
            Duration::from_secs(1),
        )
        .unwrap();
        let app = Router::new().fallback(move || async move { body });
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        assert!(client.verify_token().await.is_err());
        task.abort();
    }
}
