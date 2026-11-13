//! Password sign-ins waiting for this app's second factor (two-step sign-in).
//!
//! A password that matches an account with its own TOTP does not sign in: `login.loginAction`
//! records a pending sign-in here and sets a short-lived cookie holding only its opaque random id;
//! `/login/2fa` completes it with `login.verifyTwoFactor`. Nothing about the pending sign-in is ever
//! a session: the cookie has its own name, carries no claims, and is looked up here.
//!
//! - **Bounded and in memory.** One API process owns sign-in, like the login throttle. A restart
//!   loses pending sign-ins — the browser starts again with the password. Entries expire five
//!   minutes after the password step (absolute), one per account (a new password step replaces the
//!   old one), at most [`CAPACITY`].
//! - **Checked against the account as it is now.** The entry keeps the account's session version
//!   and a digest of its stored second factor from the password step; [`current_account`] must
//!   still find the account enabled, approved, with 2FA on, the same session version (sign-out,
//!   password or role change bump it) and the same factor (enrolling a new TOTP does not).
//! - **Used once.** Each entry has its own async lock: concurrent submissions queue, and the one
//!   that succeeds takes the entry out ([`PendingLogins::take_if_current`]) before any session is
//!   issued, so the others find it finished.
//! - **Fails closed.** No cookie, an unknown or expired id, or the cookie name present more than
//!   once — nothing is validated.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::http::HeaderMap;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;

use super::audit::{self, AuditEntry};
use super::login::AuthorizedUser;
use super::principal::db_unavailable;
use super::rbac::parse_role;
use super::session;
use crate::contracts::Role;
use crate::error::ApiError;
use crate::request_context::RequestContext;
use crate::state::AppState;

pub const COOKIE_NAME_PLAIN: &str = "nwui-2fa-challenge";
/// `__Host-`: Secure, `Path=/`, no `Domain` — the browser pins it to this origin.
pub const COOKIE_NAME_SECURE: &str = "__Host-nwui-2fa-challenge";
pub const TTL_MS: i64 = 5 * 60 * 1000;
/// Wrong codes one pending sign-in may take; the account throttle counts them too.
pub const MAX_ATTEMPTS: u32 = 5;
pub const CAPACITY: usize = 10_000;
const ID_BYTES: usize = 32;

/// One pending sign-in. Held behind its own async lock while a code is checked.
#[derive(Debug)]
pub struct Pending {
    pub user_id: String,
    /// The account-throttle key of the password step (the e-mail as submitted): factor failures
    /// count against the same account limit as password failures.
    pub throttle_key: String,
    pub session_version: i32,
    /// [`factor_digest`] of the stored second factor when the password matched.
    pub factor: String,
    pub expires_ms: i64,
    pub attempts: u32,
    /// Completed, cancelled, replaced, spent or found invalid: nothing may use it again.
    pub finished: bool,
}

type Slot = Arc<tokio::sync::Mutex<Pending>>;

struct Entry {
    user_id: String,
    expires_ms: i64,
    slot: Slot,
}

#[derive(Default)]
pub struct PendingLogins {
    entries: Mutex<HashMap<String, Entry>>,
}

impl Pending {
    /// Can still be completed at `now_ms` — the actual time, read after any wait.
    pub fn live(&self, now_ms: i64) -> bool {
        !self.finished && now_ms < self.expires_ms
    }
}

impl PendingLogins {
    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Records a pending sign-in and returns its id, or `None` when no random id could be drawn or
    /// the store is full of live entries. Replaces the account's previous pending sign-in.
    pub fn create(&self, user_id: &str, throttle_key: &str, session_version: i32, factor: String, now_ms: i64) -> Option<String> {
        let mut raw = [0u8; ID_BYTES];
        getrandom::fill(&mut raw).ok()?;
        let id = URL_SAFE_NO_PAD.encode(raw);
        let mut entries = self.entries();
        entries.retain(|_, entry| entry.expires_ms > now_ms && entry.user_id != user_id);
        if entries.len() >= CAPACITY {
            return None;
        }
        let pending = Pending {
            user_id: user_id.to_owned(),
            throttle_key: throttle_key.to_owned(),
            session_version,
            factor,
            expires_ms: now_ms + TTL_MS,
            attempts: 0,
            finished: false,
        };
        entries.insert(
            id.clone(),
            Entry { user_id: user_id.to_owned(), expires_ms: now_ms + TTL_MS, slot: Arc::new(tokio::sync::Mutex::new(pending)) },
        );
        Some(id)
    }

    /// The live pending sign-in with this id. An expired one is dropped here.
    pub fn get(&self, id: &str, now_ms: i64) -> Option<Slot> {
        let mut entries = self.entries();
        match entries.get(id) {
            Some(entry) if entry.expires_ms > now_ms => Some(entry.slot.clone()),
            Some(_) => {
                entries.remove(id);
                None
            }
            None => None,
        }
    }

    /// Takes the entry out if it is still this one (not replaced or removed meanwhile). The single
    /// point where a pending sign-in is consumed: `true` exactly once per entry.
    pub fn take_if_current(&self, id: &str, slot: &Slot) -> bool {
        let mut entries = self.entries();
        if entries.get(id).is_some_and(|entry| Arc::ptr_eq(&entry.slot, slot)) {
            entries.remove(id);
            return true;
        }
        false
    }

    /// Still in the store as this entry: not taken, replaced or cancelled. Checked again after
    /// waiting for an entry's lock, since any of those can happen while a request waits.
    pub fn is_current(&self, id: &str, slot: &Slot) -> bool {
        self.entries().get(id).is_some_and(|entry| Arc::ptr_eq(&entry.slot, slot))
    }

    pub fn remove(&self, id: &str) {
        self.entries().remove(id);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries().len()
    }
}

/// SHA-256 of the stored `totp_secret` column (sealed or legacy plain) — compared, never shown.
pub fn factor_digest(stored_secret: Option<&str>) -> String {
    super::token_digest(stored_secret.unwrap_or(""))
}

pub fn cookie_name(secure: bool) -> &'static str {
    if secure { COOKIE_NAME_SECURE } else { COOKIE_NAME_PLAIN }
}

/// The pending id from the request, only when the cookie is present exactly once with a value.
pub fn read_cookie(headers: &HeaderMap, secure: bool) -> Option<String> {
    let name = cookie_name(secure);
    let mut values = session::request_cookies(headers).into_iter().filter(|(n, _)| n == name).map(|(_, v)| v);
    match (values.next(), values.next()) {
        (Some(value), None) if !value.is_empty() => Some(value),
        _ => None,
    }
}

pub fn set_cookie(secure: bool, id: &str) -> String {
    cookie_line(secure, id, TTL_MS / 1000)
}

pub fn clear_cookie(secure: bool) -> String {
    cookie_line(secure, "", 0)
}

fn cookie_line(secure: bool, value: &str, max_age_secs: i64) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!("{}={value}; Path=/; Max-Age={max_age_secs}; HttpOnly; SameSite=Lax{secure_attr}", cookie_name(secure))
}

/// The account row a pending sign-in may still complete for.
pub struct Account {
    pub email: Option<String>,
    pub role: String,
    pub session_version: i32,
    pub totp_secret: Option<String>,
    pub backup_codes: Option<Value>,
}

/// The account as it is now, if the pending sign-in still stands for it: it exists, is enabled and
/// approved, has 2FA on, and has the same session version and second factor as at the password step.
pub async fn current_account(state: &AppState, pending: &Pending) -> Result<Option<Account>, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let row = client
        .query_opt(
            "SELECT email, role::text AS role, session_version, totp_enabled, totp_secret, backup_codes, enabled, approved \
             FROM users WHERE id = $1",
            &[&pending.user_id],
        )
        .await
        .map_err(|_| db_unavailable())?;
    Ok(row.and_then(|row| {
        let totp_secret: Option<String> = row.get("totp_secret");
        let stands = row.get::<_, bool>("enabled")
            && row.get::<_, bool>("approved")
            && row.get::<_, bool>("totp_enabled")
            && row.get::<_, i32>("session_version") == pending.session_version
            && factor_digest(totp_secret.as_deref()) == pending.factor;
        stands.then(|| Account {
            email: row.get("email"),
            role: row.get("role"),
            session_version: row.get("session_version"),
            totp_secret,
            backup_codes: row.get("backup_codes"),
        })
    }))
}

/// The account once more, locked (`FOR SHARE`) in the transaction that writes the `login` audit:
/// still enabled, approved, with 2FA on, the same session version and factor — and the pending
/// sign-in still within its lifetime by `clock`, read after the row lock and before the audit, so a
/// slow database cannot carry it past expiry. `None` writes nothing and issues nothing.
pub async fn complete_login(
    state: &AppState,
    pending: &Pending,
    ctx: &RequestContext,
    clock: &(dyn Fn() -> i64 + Sync),
) -> Result<Option<AuthorizedUser>, ApiError> {
    let failed = |_| db_unavailable();
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(failed)?;
    let row = tx
        .query_opt(
            "SELECT email, role::text AS role, session_version, totp_enabled, totp_secret, enabled, approved \
             FROM users WHERE id = $1 FOR SHARE",
            &[&pending.user_id],
        )
        .await
        .map_err(failed)?;
    let Some(row) = row else { return Ok(None) };
    let totp_secret: Option<String> = row.get("totp_secret");
    if !(row.get::<_, bool>("enabled")
        && row.get::<_, bool>("approved")
        && row.get::<_, bool>("totp_enabled")
        && row.get::<_, i32>("session_version") == pending.session_version
        && factor_digest(totp_secret.as_deref()) == pending.factor)
    {
        return Ok(None);
    }
    let role = parse_role(row.get("role")).ok_or_else(ApiError::internal)?;
    let policy = tx.query_opt("SELECT require_2fa_admin, require_2fa_user FROM app_settings LIMIT 1", &[]).await.map_err(failed)?;
    let (require_admin, require_user) = policy.map(|p| (p.get::<_, bool>(0), p.get::<_, bool>(1))).unwrap_or((false, false));
    if clock() >= pending.expires_ms {
        return Ok(None);
    }
    let entry = AuditEntry { actor_id: Some(pending.user_id.clone()), action: "login".into(), ip: ctx.ip.clone(), ..AuditEntry::default() };
    audit::write(&*tx, &entry).await.map_err(failed)?;
    tx.commit().await.map_err(failed)?;
    Ok(Some(AuthorizedUser {
        id: pending.user_id.clone(),
        email: row.get("email"),
        role,
        session_version: pending.session_version,
        totp_enabled: true,
        requires_2fa: if role == Role::Admin { require_admin } else { require_user },
        mfa_verified: true,
        external_login: false,
    }))
}

/// `AuthOptions.twoFactorPending`: this browser has a pending sign-in that can still complete.
/// One that no longer can is finished and dropped here, so `/login` and `/login/2fa` never send the
/// browser back and forth.
pub async fn is_pending(state: &AppState, headers: &HeaderMap, ctx: &RequestContext, now_ms: i64) -> Result<bool, ApiError> {
    let store = &state.runtime.pending_logins;
    let Some(id) = read_cookie(headers, ctx.secure) else { return Ok(false) };
    let Some(slot) = store.get(&id, now_ms) else { return Ok(false) };
    // A verification in flight holds the lock across its database work and bcrypt: the page does
    // not wait for it. The sign-in is pending while its entry is current; the verification decides.
    let Ok(mut pending) = slot.try_lock() else { return Ok(store.is_current(&id, &slot)) };
    if !pending.live(super::principal::now_unix_ms()) || !store.is_current(&id, &slot) {
        return Ok(false);
    }
    if current_account(state, &pending).await?.is_none() {
        pending.finished = true;
        store.take_if_current(&id, &slot);
        return Ok(false);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_900_000_000_000;

    fn create(store: &PendingLogins, user: &str, now: i64) -> String {
        store.create(user, &format!("{user}@example.test"), 3, factor_digest(Some("sealed")), now).unwrap()
    }

    #[test]
    fn ids_are_random_url_safe_and_long() {
        let store = PendingLogins::default();
        let a = create(&store, "a", T0);
        let b = create(&store, "b", T0);
        assert_ne!(a, b);
        assert_eq!(a.len(), 43, "256 bits, base64url without padding");
        assert!(a.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
    }

    #[test]
    fn a_pending_sign_in_lives_five_minutes_from_the_password_step() {
        let store = PendingLogins::default();
        let id = create(&store, "u1", T0);
        assert!(store.get(&id, T0 + TTL_MS - 1).is_some());
        assert!(store.get(&id, T0 + TTL_MS).is_none(), "absolute expiry");
        assert_eq!(store.len(), 0, "an expired entry is dropped when looked up");
        assert!(store.get("unknown", T0).is_none());
    }

    #[test]
    fn a_new_password_step_replaces_the_accounts_previous_one() {
        let store = PendingLogins::default();
        let first = create(&store, "u1", T0);
        let other = create(&store, "u2", T0);
        let second = create(&store, "u1", T0 + 1);
        assert!(store.get(&first, T0 + 2).is_none());
        assert!(store.get(&second, T0 + 2).is_some());
        assert!(store.get(&other, T0 + 2).is_some(), "other accounts are untouched");
    }

    #[test]
    fn the_store_is_bounded_and_expired_entries_make_room() {
        let store = PendingLogins::default();
        for i in 0..CAPACITY {
            create(&store, &format!("u{i}"), T0);
        }
        assert!(store.create("late", "late@example.test", 1, String::new(), T0 + 1).is_none(), "full of live entries");
        assert!(store.create("late", "late@example.test", 1, String::new(), T0 + TTL_MS).is_some(), "the expired ones are swept");
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn a_restart_forgets_every_pending_sign_in() {
        let before = PendingLogins::default();
        let id = create(&before, "u1", T0);
        let after = PendingLogins::default();
        assert!(after.get(&id, T0).is_none());
    }

    #[tokio::test]
    async fn cancel_or_replacement_while_a_verification_waits_ends_that_sign_in() {
        let store = PendingLogins::default();
        let id = create(&store, "u1", T0);
        let slot = store.get(&id, T0).unwrap();
        // A verification holds the entry's lock; a second one waits for it.
        let held = slot.lock().await;
        store.remove(&id); // cancel meanwhile
        drop(held);
        let waiting = slot.lock().await;
        assert!(waiting.live(T0), "the entry itself does not know it was cancelled");
        assert!(!store.is_current(&id, &slot), "the store does: nothing may complete it");
        assert!(!store.take_if_current(&id, &slot));
        drop(waiting);

        let id = create(&store, "u1", T0);
        let slot = store.get(&id, T0).unwrap();
        let _held = slot.lock().await;
        create(&store, "u1", T0 + 1); // a new password step meanwhile
        assert!(!store.is_current(&id, &slot));
        assert!(!store.take_if_current(&id, &slot));
    }

    #[test]
    fn liveness_is_read_with_the_actual_time() {
        let store = PendingLogins::default();
        let id = create(&store, "u1", T0);
        let slot = store.get(&id, T0).unwrap();
        let mut pending = slot.try_lock().unwrap();
        assert!(pending.live(T0 + TTL_MS - 1));
        assert!(!pending.live(T0 + TTL_MS), "expired while waiting");
        pending.finished = true;
        assert!(!pending.live(T0));
    }

    #[test]
    fn an_entry_is_consumed_once_and_only_while_current() {
        let store = PendingLogins::default();
        let id = create(&store, "u1", T0);
        let slot = store.get(&id, T0).unwrap();
        assert!(store.take_if_current(&id, &slot));
        assert!(!store.take_if_current(&id, &slot), "once");
        let replaced = create(&store, "u2", T0);
        let stale = store.get(&replaced, T0).unwrap();
        create(&store, "u2", T0 + 1);
        assert!(!store.take_if_current(&replaced, &stale), "replaced meanwhile");
    }

    fn headers(cookie: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("cookie", cookie.parse().unwrap());
        headers
    }

    #[test]
    fn the_cookie_is_read_only_when_present_exactly_once() {
        assert_eq!(read_cookie(&headers("nwui-2fa-challenge=abc"), false).as_deref(), Some("abc"));
        assert_eq!(read_cookie(&headers("x=1; nwui-2fa-challenge=abc; y=2"), false).as_deref(), Some("abc"));
        for ambiguous in ["nwui-2fa-challenge=abc; nwui-2fa-challenge=def", "nwui-2fa-challenge=abc; nwui-2fa-challenge=abc", "nwui-2fa-challenge="] {
            assert_eq!(read_cookie(&headers(ambiguous), false), None, "{ambiguous}");
        }
        assert_eq!(read_cookie(&headers("nwui-2fa-challenge=abc"), true), None, "https reads only the __Host- name");
        assert_eq!(read_cookie(&headers("__Host-nwui-2fa-challenge=abc"), true).as_deref(), Some("abc"));
        assert_eq!(read_cookie(&HeaderMap::new(), false), None);
    }

    #[test]
    fn cookie_attributes() {
        assert_eq!(set_cookie(false, "abc"), "nwui-2fa-challenge=abc; Path=/; Max-Age=300; HttpOnly; SameSite=Lax");
        assert_eq!(set_cookie(true, "abc"), "__Host-nwui-2fa-challenge=abc; Path=/; Max-Age=300; HttpOnly; SameSite=Lax; Secure");
        assert_eq!(clear_cookie(true), "__Host-nwui-2fa-challenge=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax; Secure");
    }

    #[test]
    fn the_factor_digest_changes_with_the_stored_secret() {
        assert_ne!(factor_digest(Some("a")), factor_digest(Some("b")));
        assert_eq!(factor_digest(Some("a")), factor_digest(Some("a")));
        assert_ne!(factor_digest(None), factor_digest(Some("a")));
    }
}
