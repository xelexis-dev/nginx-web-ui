//! `nwui-api healthcheck`, the image's Docker HEALTHCHECK.
//!
//! No credentials: it requires nginx -> API -> database (`/api/health` answers `{"ok":true}`)
//! and the SSR process (2xx or 3xx on `/`, redirects not followed) to answer. It replaces a
//! Node script that started a whole Node runtime every 30 seconds.
//!
//! The healthcheck runs with the image environment only. AUTH_SECRET, the generated database
//! password and other runtime secrets reach the services through s6's container_environment,
//! so this must never read `RuntimeConfig` — `main` dispatches here before it does.
use std::time::Duration;

use crate::control::self_proxy::ADMIN_UI_PORT;

#[derive(Debug, Clone)]
pub struct Targets {
    pub api: String,
    pub web: String,
    /// Per request.
    pub timeout: Duration,
}

impl Targets {
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        let web_port = get("PORT").unwrap_or_else(|| "3000".to_owned());
        Self {
            api: format!("http://127.0.0.1:{ADMIN_UI_PORT}/api/health"),
            web: format!("http://127.0.0.1:{web_port}/"),
            timeout: Duration::from_secs(3),
        }
    }
}

pub async fn check(targets: &Targets) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        // Loopback only; an operator's HTTP_PROXY must not capture it (Node's fetch ignored it).
        .no_proxy()
        .timeout(targets.timeout)
        .build()
        .map_err(|e| format!("client: {e}"))?;
    let api = client
        .get(&targets.api)
        .send()
        .await
        .map_err(|e| format!("api: {e}"))?;
    if !api.status().is_success() {
        return Err(format!("api: status {}", api.status()));
    }
    let body: serde_json::Value = api.json().await.map_err(|e| format!("api: {e}"))?;
    if body.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("api: {body}"));
    }
    let web = client
        .get(&targets.web)
        .send()
        .await
        .map_err(|e| format!("web: {e}"))?;
    let status = web.status().as_u16();
    if !(200..400).contains(&status) {
        return Err(format!("web: status {status}"));
    }
    Ok(())
}

/// Process exit code: 0 healthy, 1 unhealthy.
pub fn run() -> i32 {
    let targets = Targets::from_env(|key| std::env::var(key).ok());
    let outcome = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))
        .and_then(|runtime| runtime.block_on(check(&targets)));
    match outcome {
        Ok(()) => 0,
        Err(reason) => {
            eprintln!("healthcheck: {reason}");
            1
        }
    }
}
