//! Password hashing and policy, compatible with `packages/auth/src/password.ts` (bcryptjs 3.0.3).
//!
//! bcrypt is not an RFC (OpenBSD / Provos–Mazières); the contract here is data compatibility:
//! existing `$2b$12$` hashes must keep verifying for exactly the same inputs. bcrypt uses only the
//! first 72 bytes of the UTF-8 password; bcryptjs truncates silently, so we use the truncating
//! `bcrypt::hash`/`verify` (never `non_truncating_*`). The policy has no maximum length, so
//! passwords longer than 72 bytes (≈25 Korean characters) exist and must keep working.
//!
//! bcrypt at cost 12 is CPU-heavy; async callers go through [`PasswordHasher`], which runs it on
//! Tokio's blocking pool behind a semaphore so it never stalls the async worker threads.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const BCRYPT_COST: u32 = 12;
/// Sign-in checks allowed to wait per concurrent bcrypt worker before new ones are refused.
pub const SIGN_IN_QUEUE_PER_WORKER: usize = 32;
pub const PASSWORD_MIN_LENGTH: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("password hashing failed")]
    Hash,
    #[error("password worker was cancelled")]
    Cancelled,
    #[error("too many password checks are waiting")]
    Busy,
}

/// Blocking (~cost 12); async code uses [`PasswordHasher::hash`].
pub fn hash_password(password: &str) -> Result<String, PasswordError> {
    bcrypt::hash(password, BCRYPT_COST).map_err(|_| PasswordError::Hash)
}

/// `false` for a wrong password and for a malformed hash (never panics, never errors out).
/// Blocking; async code uses [`PasswordHasher::verify`].
pub fn verify_password(password: &str, hash: &str) -> bool {
    bcrypt::verify(password, hash).unwrap_or(false)
}

/// Korean policy message, or `None` when acceptable. Length counts UTF-16 code units like the TS
/// `password.length` (an emoji counts as 2), letters/digits are ASCII like `/[A-Za-z]/`, `/[0-9]/`.
pub fn validate_password(password: &str) -> Option<String> {
    if password.encode_utf16().count() < PASSWORD_MIN_LENGTH {
        return Some(format!("비밀번호는 최소 {PASSWORD_MIN_LENGTH}자 이상이어야 합니다."));
    }
    if !password.bytes().any(|b| b.is_ascii_alphabetic()) || !password.bytes().any(|b| b.is_ascii_digit()) {
        return Some("비밀번호는 영문자와 숫자를 모두 포함해야 합니다.".to_owned());
    }
    None
}

/// Runs bcrypt off the async workers with bounded concurrency. Build **one** per process (it
/// lives in the shared application state) so the bound is global, not per request.
#[derive(Clone)]
pub struct PasswordHasher {
    permits: Arc<tokio::sync::Semaphore>,
    /// Sign-in checks currently waiting for a permit (see [`Self::verify_or_busy`]).
    waiting: Arc<AtomicUsize>,
    queue_limit: usize,
}

/// Leaves the sign-in queue when dropped, including when the caller is cancelled.
struct QueueSlot<'a>(&'a AtomicUsize);

impl Drop for QueueSlot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl PasswordHasher {
    pub fn new(max_concurrent: usize) -> Self {
        let workers = max_concurrent.max(1);
        Self::with_queue_limit(workers, workers * SIGN_IN_QUEUE_PER_WORKER)
    }

    pub fn with_queue_limit(max_concurrent: usize, queue_limit: usize) -> Self {
        Self {
            permits: Arc::new(tokio::sync::Semaphore::new(max_concurrent.max(1))),
            waiting: Arc::new(AtomicUsize::new(0)),
            queue_limit,
        }
    }

    /// For unauthenticated sign-in only: `Busy` instead of joining a queue that already holds
    /// `queue_limit` waiters. Other checks come from signed-in users and keep plain [`Self::verify`].
    pub async fn verify_or_busy(&self, password: String, hash: String) -> Result<bool, PasswordError> {
        let slot = QueueSlot(&self.waiting);
        if self.waiting.fetch_add(1, Ordering::SeqCst) >= self.queue_limit {
            return Err(PasswordError::Busy);
        }
        let permit = Arc::clone(&self.permits).acquire_owned().await.map_err(|_| PasswordError::Cancelled)?;
        drop(slot);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            verify_password(&password, &hash)
        })
        .await
        .map_err(|_| PasswordError::Cancelled)
    }

    pub async fn hash(&self, password: String) -> Result<String, PasswordError> {
        self.run(move || hash_password(&password)).await?
    }

    pub async fn verify(&self, password: String, hash: String) -> bool {
        self.run(move || verify_password(&password, &hash)).await.unwrap_or(false)
    }

    /// Runs any bcrypt-bound work (e.g. a backup-code search) under the same bound.
    ///
    /// The permit moves into the blocking closure: dropping this future (the client went away)
    /// does not stop bcrypt, so the permit must stay held until bcrypt itself ends.
    pub async fn run<T: Send + 'static>(&self, work: impl FnOnce() -> T + Send + 'static) -> Result<T, PasswordError> {
        let permit = Arc::clone(&self.permits).acquire_owned().await.map_err(|_| PasswordError::Cancelled)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        })
        .await
        .map_err(|_| PasswordError::Cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ascii(n: usize) -> String {
        "Ab1".chars().cycle().take(n).collect()
    }

    #[test]
    fn hash_is_bcrypt_2b_cost_12_and_verifies() {
        let hash = hash_password("Passw0rd").unwrap();
        assert!(hash.starts_with("$2b$12$"), "{hash}");
        assert!(verify_password("Passw0rd", &hash));
        assert!(!verify_password("Passw0rd!", &hash));
    }

    #[test]
    fn bytes_beyond_72_are_ignored_like_bcryptjs() {
        let hash = hash_password(&ascii(100)).unwrap();
        assert!(verify_password(&ascii(72), &hash));
        assert!(verify_password(&format!("{}zzz", ascii(72)), &hash));
        assert!(!verify_password(&ascii(71), &hash));
    }

    #[test]
    fn malformed_hash_is_false_not_a_panic() {
        for hash in ["", "not-a-hash", "$2b$12$short", "$9z$12$aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"] {
            assert!(!verify_password("x", hash), "{hash}");
        }
    }

    #[test]
    fn policy_matches_the_ts_messages_and_utf16_length() {
        let short = "비밀번호는 최소 8자 이상이어야 합니다.";
        let mix = "비밀번호는 영문자와 숫자를 모두 포함해야 합니다.";
        assert_eq!(validate_password("short1a").as_deref(), Some(short));
        assert_eq!(validate_password("abcdefgh").as_deref(), Some(mix));
        assert_eq!(validate_password("12345678").as_deref(), Some(mix));
        assert_eq!(validate_password("abcdefg1"), None);
        // Three emoji are 6 UTF-16 units + "a1" = 8 → long enough.
        assert_eq!(validate_password("🔐🔐🔐a1"), None);
        // One emoji = 2 units + 3 = 5 → too short, although 4 characters would be <8 anyway.
        assert_eq!(validate_password("🔐a1b").as_deref(), Some(short));
        assert_eq!(validate_password("가나다라마바사아").as_deref(), Some(mix));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hasher_runs_off_the_async_workers() {
        let hasher = PasswordHasher::new(2);
        let hash = hasher.hash("Passw0rd".to_owned()).await.unwrap();
        assert!(hasher.verify("Passw0rd".to_owned(), hash.clone()).await);
        assert!(!hasher.verify("nope".to_owned(), hash).await);
    }

    #[tokio::test]
    async fn zero_concurrency_is_raised_to_one_instead_of_deadlocking() {
        let hasher = PasswordHasher::new(0);
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), hasher.run(|| 42)).await;
        assert_eq!(result.expect("must not wait forever").unwrap(), 42);
    }

    /// Unauthenticated sign-in refuses instead of queueing once the queue is full, so a flood
    /// cannot stall every other password check (password change, backup codes) behind it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sign_in_checks_refuse_once_the_queue_is_full() {
        use std::sync::atomic::Ordering;
        let hasher = PasswordHasher::with_queue_limit(1, 1);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let running = tokio::spawn({
            let hasher = hasher.clone();
            async move {
                hasher
                    .run(move || {
                        started_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                    })
                    .await
            }
        });
        started_rx.await.unwrap();
        let queued = tokio::spawn({
            let hasher = hasher.clone();
            async move { hasher.verify_or_busy("x".to_owned(), "not-a-hash".to_owned()).await }
        });
        for _ in 0..200 {
            if hasher.waiting.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(hasher.waiting.load(Ordering::SeqCst), 1);
        let refused = hasher.verify_or_busy("x".to_owned(), "not-a-hash".to_owned()).await;
        assert!(matches!(refused, Err(PasswordError::Busy)), "{refused:?}");
        release_tx.send(()).unwrap();
        running.await.unwrap().unwrap();
        assert!(matches!(queued.await.unwrap(), Ok(false)), "the one that fit in the queue still ran");
        assert_eq!(hasher.waiting.load(Ordering::SeqCst), 0);
    }

    /// A cancelled request (client gone) must not free the permit while its bcrypt still runs,
    /// or repeated cancellations would exceed the CPU bound.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelling_the_caller_keeps_the_permit_until_the_work_ends() {
        let hasher = PasswordHasher::new(1);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let task = tokio::spawn({
            let hasher = hasher.clone();
            async move {
                hasher
                    .run(move || {
                        started_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                    })
                    .await
            }
        });
        started_rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(hasher.permits.available_permits(), 0, "the running closure still holds the permit");

        release_tx.send(()).unwrap();
        for _ in 0..200 {
            if hasher.permits.available_permits() == 1 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("the permit was not returned after the work finished");
    }
}
