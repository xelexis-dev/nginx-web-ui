//! Process-local monotonic budgets; active streams keep their entries until Drop.
use crate::error::ApiError;
use axum::http::StatusCode;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
const WINDOW: Duration = Duration::from_secs(60);
const MAX_KEYS: usize = 10_000;
#[derive(Default)]
pub struct TokenLimits {
    inner: Arc<Mutex<Limits>>,
}
#[derive(Default)]
struct Limits {
    tokens: HashMap<String, Window>,
    owners: HashMap<String, Window>,
}
struct Window {
    start: Instant,
    reads: u16,
    mutations: u16,
    streams: u8,
}
impl Window {
    fn new(now: Instant) -> Self {
        Self {
            start: now,
            reads: 0,
            mutations: 0,
            streams: 0,
        }
    }
    fn refresh(&mut self, now: Instant) {
        if now.duration_since(self.start) >= WINDOW {
            self.start = now;
            self.reads = 0;
            self.mutations = 0;
        }
    }
    fn retry(&self, now: Instant) -> u64 {
        WINDOW
            .saturating_sub(now.duration_since(self.start))
            .as_secs()
            + 1
    }
}
pub struct StreamPermit {
    inner: Arc<Mutex<Limits>>,
    token: String,
    owner: String,
}
impl Drop for StreamPermit {
    fn drop(&mut self) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(window) = inner.tokens.get_mut(&self.token) {
            window.streams = window.streams.saturating_sub(1);
        }
        if let Some(window) = inner.owners.get_mut(&self.owner) {
            window.streams = window.streams.saturating_sub(1);
        }
    }
}
fn limited(retry: u64) -> ApiError {
    ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limited",
        "토큰 요청 한도를 초과했습니다. 잠시 후 다시 시도하세요.",
    )
    .with_retry_after(retry)
}
impl TokenLimits {
    pub fn admit(
        &self,
        token: &str,
        owner: &str,
        mutation: bool,
        stream: bool,
    ) -> Result<Option<StreamPermit>, ApiError> {
        self.admit_at(token, owner, mutation, stream, Instant::now())
    }
    fn admit_at(
        &self,
        token: &str,
        owner: &str,
        mutation: bool,
        stream: bool,
        now: Instant,
    ) -> Result<Option<StreamPermit>, ApiError> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if inner.tokens.len() + inner.owners.len() >= MAX_KEYS {
            inner
                .tokens
                .retain(|_, w| w.streams > 0 || now.duration_since(w.start) < WINDOW);
            inner
                .owners
                .retain(|_, w| w.streams > 0 || now.duration_since(w.start) < WINDOW);
        }
        let missing = usize::from(!inner.tokens.contains_key(token))
            + usize::from(!inner.owners.contains_key(owner));
        if inner.tokens.len() + inner.owners.len() + missing > MAX_KEYS {
            return Err(limited(60));
        }
        inner
            .tokens
            .entry(token.to_owned())
            .or_insert_with(|| Window::new(now))
            .refresh(now);
        inner
            .owners
            .entry(owner.to_owned())
            .or_insert_with(|| Window::new(now))
            .refresh(now);
        for (window, cap) in [
            (inner.tokens.get(token).unwrap(), 3),
            (inner.owners.get(owner).unwrap(), 6),
        ] {
            if stream && window.streams >= cap {
                return Err(limited(1));
            }
            if (mutation && window.mutations >= 60) || (!mutation && window.reads >= 600) {
                return Err(limited(window.retry(now)));
            }
        }
        {
            let window = inner.tokens.get_mut(token).unwrap();
            if mutation {
                window.mutations += 1;
            } else {
                window.reads += 1;
            }
            if stream {
                window.streams += 1;
            }
        }
        let owner_window = inner.owners.get_mut(owner).unwrap();
        if mutation {
            owner_window.mutations += 1;
        } else {
            owner_window.reads += 1;
        }
        if stream {
            owner_window.streams += 1;
        }
        Ok(stream.then(|| StreamPermit {
            inner: self.inner.clone(),
            token: token.into(),
            owner: owner.into(),
        }))
    }
}

/// The response body owns the concurrency permit, including cancellation and disconnect.
pub struct LimitedStream {
    pub body: axum::body::BodyDataStream,
    pub _permit: StreamPermit,
    pub stopped: Option<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>>,
}
impl futures_core::Stream for LimitedStream {
    type Item = Result<axum::body::Bytes, axum::Error>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        ctx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let Some(stopped)=self.stopped.as_mut() else { return std::task::Poll::Ready(None); };
        if stopped.as_mut().poll(ctx).is_ready() {
            self.stopped=None;
            return std::task::Poll::Ready(None);
        }
        let next=std::pin::Pin::new(&mut self.body).poll_next(ctx);
        if matches!(next,std::task::Poll::Ready(None)) { self.stopped=None; }
        next
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budgets_intersect_token_and_owner_without_coupling_other_owners() {
        let limits = TokenLimits::default();
        let now = Instant::now();
        for _ in 0..60 {
            assert!(limits.admit_at("a", "owner", true, false, now).is_ok());
        }
        assert_eq!(
            limits
                .admit_at("b", "owner", true, false, now)
                .err()
                .unwrap()
                .status,
            429
        );
        assert!(limits.admit_at("b", "other", true, false, now).is_ok());
        for _ in 0..600 {
            assert!(limits.admit_at("read", "reader", false, false, now).is_ok());
        }
        assert!(
            limits
                .admit_at("read", "another-reader", false, false, now)
                .is_err()
        );
        assert!(
            limits
                .admit_at("a", "owner", true, false, now + WINDOW)
                .is_ok()
        );
    }
    #[test]
    fn concurrent_streams_release_exactly_on_drop_and_survive_window_rollover() {
        let limits = TokenLimits::default();
        let now = Instant::now();
        let mut held = vec![];
        for token in ["a", "b"] {
            for _ in 0..3 {
                held.push(
                    limits
                        .admit_at(token, "owner", false, true, now)
                        .unwrap()
                        .unwrap(),
                );
            }
        }
        assert!(
            limits
                .admit_at("a", "other", false, true, now + WINDOW)
                .is_err()
        );
        assert!(
            limits
                .admit_at("c", "owner", false, true, now + WINDOW)
                .is_err()
        );
        held.pop();
        assert!(
            limits
                .admit_at("c", "owner", false, true, now + WINDOW)
                .is_ok()
        );
        drop(held);
        assert!(
            limits
                .admit_at("a", "owner", false, true, now + WINDOW)
                .is_ok()
        );
    }
    #[test]
    fn key_storage_is_bounded_and_expired_entries_are_reclaimed() {
        let limits = TokenLimits::default();
        let now = Instant::now();
        for i in 0..MAX_KEYS / 2 {
            assert!(
                limits
                    .admit_at(&format!("t{i}"), &format!("u{i}"), false, false, now)
                    .is_ok()
            );
        }
        assert!(
            limits
                .admit_at("extra", "extra", false, false, now)
                .is_err()
        );
        assert!(
            limits
                .admit_at("extra", "extra", false, false, now + WINDOW)
                .is_ok()
        );
        let inner = limits.inner.lock().unwrap();
        assert!(inner.tokens.len() + inner.owners.len() <= MAX_KEYS);
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use http_body_util::BodyExt;
    use std::{pin::Pin,task::{Context,Poll},future::Future};
    struct Once(bool);
    impl Future for Once {
        type Output=();
        fn poll(mut self: Pin<&mut Self>,_: &mut Context<'_>) -> Poll<()> {assert!(!self.0,"completed shutdown future was polled again");self.0=true;Poll::Ready(())}
    }
    #[tokio::test]
    async fn review_n2_limited_stream_keeps_eof_after_shutdown_completes() {
        let limits=TokenLimits::default();let permit=limits.admit("token","owner",false,true).unwrap().unwrap();
        let stream=LimitedStream {body:axum::body::Body::from("pending data").into_data_stream(),_permit:permit,stopped:Some(Box::pin(Once(false)))};
        let mut body=axum::body::Body::from_stream(stream);
        assert!(body.frame().await.is_none());
        assert!(body.frame().await.is_none());
    }
}
