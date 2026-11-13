//! Phase 2 observes services only; publisher alone writes its marker and apply-uncertain state.
use super::StepError;
use crate::config::RuntimeConfig;
use std::time::{Duration,Instant};
fn timeout()->StepError {StepError{code:"activation_timeout",message:"서비스가 정상적으로 응답하는지 제한 시간 안에 확인하지 못했습니다.",detail:None}}
fn remaining(deadline:Instant)->Result<Duration,StepError> {deadline.checked_duration_since(Instant::now()).filter(|d|!d.is_zero()).ok_or_else(timeout)}
/// N1 owns marker parsing and observation; W1 never confirms, reloads or clears A8.
async fn observe_nginx(config:&RuntimeConfig,deadline:Instant)->Result<bool,StepError> {
    crate::control::confirmation::observe_current(config,deadline).await.map(|_|true).map_err(|_|timeout())
}
async fn http_ready(targets:&crate::healthcheck::Targets,deadline:Instant)->Result<(),StepError> {
    loop {
        remaining(deadline)?;
        let result=tokio::time::timeout_at(tokio::time::Instant::from_std(deadline),crate::healthcheck::check(targets)).await.map_err(|_|timeout())?;
        if result.is_ok(){return Ok(());}
        tokio::time::sleep(remaining(deadline)?.min(Duration::from_millis(250))).await;
    }
}
/// Call under A-only custody after readiness. No journal outcome/proof is written here.
/// The caller keeps custody until blocking service waits and HTTP requests have completed.
pub(crate) async fn check(config:&RuntimeConfig,deadline:Instant)->Result<(),StepError> {
    let bound=remaining(deadline)?;
    tokio::task::spawn_blocking(move||super::service::wait_dependents(bound)).await.map_err(|_|timeout())??;
    let targets=crate::healthcheck::Targets {
        api:format!("http://127.0.0.1:{}/api/health",crate::control::self_proxy::ADMIN_UI_PORT),
        web:format!("http://127.0.0.1:{}/",config.web_port),timeout:remaining(deadline)?.min(Duration::from_secs(3)),
    };
    loop {
        let bound=remaining(deadline)?.min(Duration::from_secs(3));
        let ready=tokio::task::spawn_blocking(move||super::service::phase2_ready(bound)).await.map_err(|_|timeout())?.unwrap_or(false);
        if ready {
            http_ready(&targets,deadline).await?;remaining(deadline)?;
            if observe_nginx(config,deadline).await? {
                let bound=remaining(deadline)?.min(Duration::from_secs(3));
                if tokio::task::spawn_blocking(move||super::service::phase2_ready(bound)).await.map_err(|_|timeout())?.unwrap_or(false){return Ok(());}
            }
        }
        tokio::time::sleep(remaining(deadline)?.min(Duration::from_millis(250))).await;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn config()->RuntimeConfig {RuntimeConfig::read(|name|match name{"AUTH_SECRET"=>Some("fixture-only".into()),"DATABASE_URL"=>Some("postgres://nginx:fixture-only@127.0.0.1/nginx_web_ui".into()),_=>None}).unwrap()}
    #[tokio::test]
    async fn w1_phase2_http_smokes_retry_startup_transients_but_share_one_deadline() {
        use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
        let calls=Arc::new(AtomicUsize::new(0));let count=calls.clone();let app=axum::Router::new().route("/api",axum::routing::get(move||{let count=count.clone();async move{if count.fetch_add(1,Ordering::Relaxed)<2 {(axum::http::StatusCode::SERVICE_UNAVAILABLE,axum::Json(serde_json::json!({"ok":false})))}else{(axum::http::StatusCode::OK,axum::Json(serde_json::json!({"ok":true})))}}})).route("/down",axum::routing::get(||async{axum::http::StatusCode::SERVICE_UNAVAILABLE})).route("/",axum::routing::get(||async{"ready"}));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap();let server=tokio::spawn(async move{axum::serve(listener,app).await.unwrap()});let mut targets=crate::healthcheck::Targets{api:format!("http://{addr}/api"),web:format!("http://{addr}/"),timeout:Duration::from_millis(100)};
        http_ready(&targets,Instant::now()+Duration::from_secs(3)).await.unwrap();assert!(calls.load(Ordering::Relaxed)>=3);
        targets.api=format!("http://{addr}/down");let start=Instant::now();assert_eq!(http_ready(&targets,start+Duration::from_millis(120)).await.unwrap_err().code,"activation_timeout");assert!(start.elapsed()<Duration::from_secs(2));server.abort();let _=server.await;
    }
    #[tokio::test]
    async fn w1_phase2_expired_deadline_starts_no_service_wait_or_http_request() {
        assert_eq!(check(&config(),Instant::now()).await.unwrap_err().code,"activation_timeout");
    }
    #[tokio::test]
    async fn w1_phase2_owner_observer_never_executes_nginx_or_writes_the_config_directory() {
        use std::{fs,os::unix::fs::PermissionsExt};let t=tempfile::tempdir().unwrap();let mut config=config();config.nginx_main_conf=t.path().join("nginx.conf");config.nginx_conf_dir=t.path().to_owned();config.nginx_bin=t.path().join("nginx");
        fs::write(&config.nginx_main_conf,b"fixture config").unwrap();fs::write(&config.nginx_bin,b"#!/bin/sh\necho invoked > \"$0.invoked\"\n").unwrap();fs::set_permissions(&config.nginx_bin,fs::Permissions::from_mode(0o700)).unwrap();
        let before=crate::db_upgrade::files::snapshot(t.path()).unwrap();assert_eq!(observe_nginx(&config,Instant::now()+Duration::from_secs(1)).await.unwrap_err().code,"activation_timeout");assert_eq!(before,crate::db_upgrade::files::snapshot(t.path()).unwrap());
    }
}
