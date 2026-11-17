//! Long-lived responses (log streams, relayed node streams) are authorized when they open. The
//! guard re-checks the session that opened one and ends it once that session stops being valid
//! — sign-out on another device, a disabled account, a role change, a 2FA policy change — so it
//! does not keep streaming until the browser tab closes.
use std::time::Duration;

use axum::http::{HeaderMap, StatusCode};
use tokio::sync::watch;

use super::principal::{TwoFactorGate, require_user};
use super::rbac::Action;
use crate::request_context::RequestContext;
use crate::state::AppState;

/// How often an open stream re-checks its session.
pub const REVALIDATE_EVERY: Duration = Duration::from_secs(30);

/// A shutdown receiver for one stream: fires on process shutdown and when the session that
/// opened the stream is no longer valid for `action`. A transient failure (database briefly
/// unavailable) does not end the stream; the next check decides.
pub fn session_bound_shutdown(
    state: &AppState,
    headers: HeaderMap,
    ctx: RequestContext,
    action: Option<Action>,
    every: Duration,
) -> watch::Receiver<bool> {
    let state_owned = state.clone();
    checked_shutdown(state, every, move || {
        let state = state_owned.clone();
        let headers = headers.clone();
        let ctx = ctx.clone();
        async move {
            require_user(&state, &headers, &ctx, action, TwoFactorGate::Enforce, crate::auth::principal::Use::SessionOrToken)
                .await
                .map(|_| ())
        }
    })
}

pub fn checked_shutdown<F, Fut>(
    state: &AppState,
    every: Duration,
    mut check: F,
) -> watch::Receiver<bool>
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<(), crate::error::ApiError>> + Send,
{
    let (tx, rx) = watch::channel(false);
    let mut shutdown = state.runtime.shutdown.subscribe();
    enum Event {
        Stop,
        Gone,
        Check,
    }
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(every);
        timer.tick().await;
        loop {
            // Only choose here; the session check is awaited outside `select!`.
            let event = tokio::select! {
                _ = async { let _ = shutdown.wait_for(|stopping| *stopping).await; } => Event::Stop,
                _ = tx.closed() => Event::Gone,
                _ = timer.tick() => Event::Check,
            };
            match event {
                Event::Stop => break,
                // The stream is gone; nothing to end.
                Event::Gone => return,
                Event::Check => {
                    let checked = check().await;
                    if checked.is_err_and(|e| matches!(e.status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND)) {
                        break;
                    }
                }
            }
        }
        let _ = tx.send(true);
    });
    rx
}

/// The body owns this future: dropping the response cancels revalidation without a detached task.
pub fn token_lifetime(state: &AppState, headers: HeaderMap, ctx: RequestContext, actions: Vec<Action>, expires_ms: Option<i64>) -> std::pin::Pin<Box<dyn std::future::Future<Output=()> + Send>> {
    let state=state.clone();
    Box::pin(async move {
        let mut shutdown=state.runtime.shutdown.subscribe();
        let until_expiry=async move {
            match expires_ms {
                Some(expiry)=>tokio::time::sleep(Duration::from_millis(expiry.saturating_sub(super::principal::now_unix_ms()).max(0) as u64)).await,
                None=>std::future::pending::<()>().await,
            }
        };
        tokio::pin!(until_expiry);
        loop {
            tokio::select! {
                _=shutdown.wait_for(|stop|*stop)=>return,
                _=&mut until_expiry=>return,
                _=tokio::time::sleep(REVALIDATE_EVERY)=>{}
            }
            let checked=async {
                let user=super::principal::require_user(&state,&headers,&ctx,None,TwoFactorGate::Enforce,super::principal::Use::SessionOrToken).await?;
                for &action in &actions {super::principal::authorize(&state,&user,&ctx,action).await?;}
                Ok::<_,crate::error::ApiError>(())
            };
            // A stalled database must not extend an expired credential's stream.
            tokio::select! {
                _=&mut until_expiry=>return,
                _=shutdown.wait_for(|stop|*stop)=>return,
                result=checked=>if result.is_err(){return;}
            }
        }
    })
}
