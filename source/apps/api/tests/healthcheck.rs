//! Docker HEALTHCHECK. It runs with the image environment only: AUTH_SECRET, the generated
//! database password and the other runtime secrets reach the services through s6's
//! container_environment, not through the healthcheck process. The subcommand must therefore
//! decide before any configuration is read.
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn the_subcommand_needs_no_runtime_configuration() {
    let started = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_nwui-api"))
        .arg("healthcheck")
        .env_clear()
        // Nothing listens on the fixed management port in the test environment.
        .env("PORT", "3999")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(!stderr.contains("AUTH_SECRET"), "read the runtime configuration: {stderr}");
    assert!(stderr.contains("healthcheck"), "{stderr}");
    assert!(started.elapsed() < Duration::from_secs(10));
}

// ── The same verdicts as the former `healthcheck.mjs` ──────────────────────────────────────

use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::get};
use nwui_api::healthcheck::{self, Targets};

async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{address}")
}
fn api(status: StatusCode, body: &'static str) -> Router {
    Router::new().route("/api/health", get(move || async move { (status, body) }))
}
fn web(status: StatusCode) -> Router {
    // A redirect to a closed port: following it would fail, so success proves it is not followed.
    Router::new().route("/", get(move || async move { (status, [("location", "http://127.0.0.1:1/login")]).into_response() }))
}
fn targets(api: &str, web: &str) -> Targets {
    Targets { api: format!("{api}/api/health"), web: format!("{web}/"), timeout: Duration::from_millis(500) }
}
const OK: &str = r#"{"ok":true}"#;

#[tokio::test]
async fn healthy_when_the_api_says_ok_and_the_web_answers_without_following_its_redirect() {
    let t = targets(&serve(api(StatusCode::OK, OK)).await, &serve(web(StatusCode::TEMPORARY_REDIRECT)).await);
    assert_eq!(healthcheck::check(&t).await, Ok(()));
    let t = targets(&serve(api(StatusCode::OK, OK)).await, &serve(web(StatusCode::OK)).await);
    assert_eq!(healthcheck::check(&t).await, Ok(()));
}

#[tokio::test]
async fn unhealthy_unless_the_api_answers_ok_true() {
    let web = serve(web(StatusCode::OK)).await;
    for (status, body) in [
        (StatusCode::SERVICE_UNAVAILABLE, OK),
        (StatusCode::OK, r#"{"ok":false}"#),
        (StatusCode::OK, r#"{"ok":"true"}"#),
        (StatusCode::OK, "not json"),
    ] {
        let t = targets(&serve(api(status, body)).await, &web);
        assert!(healthcheck::check(&t).await.is_err(), "{status} {body}");
    }
}

#[tokio::test]
async fn unhealthy_when_the_web_does_not_answer_2xx_or_3xx() {
    let api = serve(api(StatusCode::OK, OK)).await;
    for status in [StatusCode::INTERNAL_SERVER_ERROR, StatusCode::NOT_FOUND] {
        let t = targets(&api, &serve(web(status)).await);
        assert!(healthcheck::check(&t).await.is_err(), "{status}");
    }
}

#[tokio::test]
async fn unhealthy_when_a_port_is_closed_or_too_slow() {
    let ok_api = serve(api(StatusCode::OK, OK)).await;
    let ok_web = serve(web(StatusCode::OK)).await;
    assert!(healthcheck::check(&targets("http://127.0.0.1:1", &ok_web)).await.is_err());
    assert!(healthcheck::check(&targets(&ok_api, "http://127.0.0.1:1")).await.is_err());
    let slow = serve(Router::new().route(
        "/api/health",
        get(|| async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Json(serde_json::json!({"ok": true}))
        }),
    ))
    .await;
    let started = Instant::now();
    assert!(healthcheck::check(&targets(&slow, &ok_web)).await.is_err());
    assert!(started.elapsed() < Duration::from_secs(2), "the timeout was not applied");
}

#[test]
fn targets_come_from_the_image_environment() {
    let t = Targets::from_env(|_| None);
    assert_eq!((t.api.as_str(), t.web.as_str()), ("http://127.0.0.1:8080/api/health", "http://127.0.0.1:3000/"));
    assert_eq!(t.timeout, Duration::from_secs(3));
    let t = Targets::from_env(|key| (key == "PORT").then(|| "4000".to_owned()));
    assert_eq!(t.web, "http://127.0.0.1:4000/");
}
