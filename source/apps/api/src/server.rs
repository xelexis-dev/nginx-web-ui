use crate::{app::{finish_router, with_request_context}, error::ApiError, state::AppState};
use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use std::{future::Future, net::SocketAddr, time::Duration};
use tokio::net::TcpListener;

pub fn router(state: AppState) -> Router {
    let version = state.config.version.clone();
    let origin = state.config.auth_url.clone();
    let routes = Router::new()
        .merge(crate::auth::router())
        .merge(crate::admin::router())
        .merge(crate::nodes::routes::router())
        .merge(crate::nodes::relay::router())
        .merge(crate::observability::router())
        .merge(crate::notifications::router())
        .merge(crate::certificates::routes::router())
        .merge(crate::control::routes::router())
        .merge(crate::control::operations::router())
        .merge(crate::ui_views::router())
        .route("/api/health", get(health))
        .with_state(state);
    finish_router(with_request_context(routes, origin), &version)
}

async fn health(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let unavailable = || ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "not_ready", "API 서버가 준비되지 않았습니다.");
    if *state.runtime.shutdown.borrow() { return Err(unavailable()); }
    tokio::time::timeout(Duration::from_secs(2), async {
        let client = state.db.get().await.map_err(|_| unavailable())?;
        client.simple_query("SELECT 1").await.map_err(|_| unavailable())?;
        Ok::<_, ApiError>(())
    }).await.map_err(|_| unavailable())??;
    Ok(Json(serde_json::json!({"ok": true})))
}

pub async fn serve(
    listener: TcpListener,
    routes: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, routes.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown).await
}

/// PostgreSQL reports 57P03 while its supervised process is still starting/recovering.
pub async fn wait_for_database(state: &AppState) -> anyhow::Result<()> {
    wait_for_database_with_limits(state, Duration::from_secs(30), Duration::from_secs(900), Duration::from_secs(2)).await
}

async fn wait_for_database_with_limits(
    state: &AppState,
    normal_limit: Duration,
    recovery_limit: Duration,
    poll: Duration,
) -> anyhow::Result<()> {
    let started = tokio::time::Instant::now();
    let normal_deadline = started + normal_limit;
    let recovery_deadline = started + recovery_limit;
    let mut deadline = normal_deadline;
    tokio::time::timeout(recovery_limit, async {
        loop {
            let starting = |error: &tokio_postgres::Error| error.as_db_error()
                .is_some_and(|error| error.code() == &tokio_postgres::error::SqlState::CANNOT_CONNECT_NOW);
            let probe = async {
                match state.db.get().await {
                    Ok(client) => client.simple_query("SELECT 1").await.map(|_| ()).map_err(|error| starting(&error)),
                    Err(deadpool_postgres::PoolError::Backend(error)) => Err(starting(&error)),
                    Err(_) => Err(false),
                }
            };
            // A lost connection or silent server must not inherit the recovery budget.
            let attempt = deadline.saturating_duration_since(tokio::time::Instant::now()).min(Duration::from_secs(5));
            let recovering = match tokio::time::timeout(attempt, probe).await {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(recovering)) => recovering,
                Err(_) => false,
            };
            deadline = if recovering { recovery_deadline } else { normal_deadline };
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            anyhow::ensure!(!remaining.is_zero(), "local database was not ready within the normal connection budget");
            tokio::time::sleep(poll.min(remaining)).await;
        }
    }).await.map_err(|_| anyhow::anyhow!("local PostgreSQL recovery exceeded the bounded startup budget"))?
}

pub async fn shutdown_signal() {
    let interrupt = async { let _ = tokio::signal::ctrl_c().await; };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => { signal.recv().await; },
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = interrupt => {}, _ = terminate => {} }
}

#[cfg(test)]
mod database_wait_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

    async fn rejecting_database(code: &'static str) -> (tempfile::TempDir, AppState, tokio::task::JoinHandle<()>, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let responses = Arc::new(AtomicUsize::new(0));
        let written = responses.clone();
        let rejecting = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let written = written.clone();
                tokio::spawn(async move {
                    let mut startup = [0; 4096];
                    socket.read(&mut startup).await.unwrap();
                    if code.is_empty() { tokio::time::sleep(Duration::from_secs(1)).await; return; }
                    let fields = format!("SFATAL\0C{code}\0Mfixture refusal\0\0");
                    let mut response = vec![b'E'];
                    response.extend_from_slice(&((fields.len() + 4) as u32).to_be_bytes());
                    response.extend_from_slice(fields.as_bytes());
                    if socket.write_all(&response).await.is_ok() { written.fetch_add(1, Ordering::SeqCst); }
                });
            }
        });
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("data-key"),"A5".repeat(32)).unwrap();
        let state = AppState::new(crate::config::RuntimeConfig::read(|key| match key {
            "DATABASE_URL" => Some(format!("postgres://nginx:test@{address}/nginx_web_ui")),
            "AUTH_SECRET" => Some("test-only-recovery".into()),
            "DATA_KEY_FILE" => Some(dir.path().join("data-key").display().to_string()),
            _ => None,
        }).unwrap()).unwrap();
        (dir, state, rejecting, responses)
    }

    #[tokio::test]
    async fn postgres_recovery_is_still_waiting_after_thirty_seconds() {
        let (_dir, state, rejecting, _responses) = rejecting_database("57P03").await;
        let result = tokio::time::timeout(Duration::from_secs(31), wait_for_database(&state)).await;
        rejecting.abort();
        assert!(result.is_err(), "recovery stopped on the ordinary connection deadline: {result:?}");
    }

    #[tokio::test]
    async fn postgres_recovery_has_a_hard_cap_and_other_failures_keep_the_normal_budget() {
        for (code, recovery) in [("57P03", true), ("28P01", false), ("08006", false), ("", false)] {
            let (_dir, state, rejecting, responses) = rejecting_database(code).await;
            let normal = Duration::from_secs(1);
            let cap = Duration::from_secs(4);
            let waiting = wait_for_database_with_limits(&state, normal, cap, Duration::from_millis(20));
            tokio::pin!(waiting);
            if recovery {
                // A real 57P03 response must extend the wait beyond the ordinary budget.
                let early = tokio::time::timeout(normal * 2, &mut waiting).await;
                assert!(responses.load(Ordering::SeqCst) > 0, "fixture never sent 57P03");
                assert!(early.is_err(), "57P03 did not extend the normal budget: {early:?}");
            }
            let result = tokio::time::timeout(cap * 2, &mut waiting).await;
            rejecting.abort();
            let error = result.expect("database wait did not terminate within the bounded allowance").unwrap_err();
            if !recovery {
                assert!(error.to_string().contains("normal connection budget"), "code={code}: {error}");
            }
            if !code.is_empty() { assert!(responses.load(Ordering::SeqCst) > 0, "fixture sent no {code} response"); }
        }
    }
}
