//! In-memory sign-in throttle, a port of `packages/auth/src/rate-limit.ts`.
//!
//! Two independent sliding windows of 15 minutes:
//! - per login account/client-IP pair, cap 10 (email uses ECMAScript trim/lowercase);
//!   other account-bound operations keep their own caller-provided keys;
//! - per client IP, cap 30 — an attacker rotating emails never trips the account cap. Shared by
//!   sign-in and gateway pairing (`/api/v1/pair`).
//!
//! A cooldown rather than a permanent lockout (which would be a trivial denial of service).
//! State is per process and resets on restart, as in TS. One instance lives in the shared
//! application state. Time is passed in (milliseconds) so behaviour is testable.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

pub const MAX_FAILURES: usize = 10;
pub const MAX_IP_FAILURES: usize = 30;
/// Separate passkey-start budget for shared addresses, including successful starts.
pub const MAX_PASSKEY_STARTS: usize = 120;
pub const WINDOW_MS: i64 = 15 * 60 * 1000;
/// Above this many keys a write sweeps expired entries, bounding memory against one-shot
/// attacker-chosen keys.
pub const SWEEP_THRESHOLD: usize = 10_000;

#[derive(Default)]
pub struct LoginThrottle {
    inner: Mutex<Windows>,
}

#[derive(Default)]
struct Windows {
    accounts: HashMap<String, Vec<Entry>>,
    ips: HashMap<String, Vec<Entry>>,
    starts: HashMap<String, Vec<Entry>>,
    noticed_starts: HashSet<String>,
    /// Keys whose current lock has already been announced (audited).
    noticed_accounts: HashSet<String>,
    noticed_ips: HashSet<String>,
    next_id: u64,
}

/// One counted attempt. `id` lets an attempt settle or release exactly its own entry, even when
/// others share its millisecond; `settled` separates recorded failures from attempts in flight.
struct Entry {
    at: i64,
    id: u64,
    settled: bool,
}

#[derive(Clone, Copy)]
enum Scope {
    Account,
    Ip,
    PasskeyStart,
}

impl Windows {
    fn parts(&mut self, scope: Scope) -> (&mut HashMap<String, Vec<Entry>>, &mut HashSet<String>) {
        match scope {
            Scope::Account => (&mut self.accounts, &mut self.noticed_accounts),
            Scope::Ip => (&mut self.ips, &mut self.noticed_ips),
            Scope::PasskeyStart => (&mut self.starts, &mut self.noticed_starts),
        }
    }
}

/// One admitted attempt, already counted in its window. `keep` makes it the recorded failure;
/// dropping it unsettled releases it — a success, an outcome that must not count, an error or a
/// cancelled request.
#[must_use]
pub struct Attempt<'a> {
    throttle: &'a LoginThrottle,
    scope: Scope,
    key: String,
    id: u64,
    kept: bool,
}

impl Attempt<'_> {
    pub fn keep(mut self) {
        self.kept = true;
        let mut windows = self.throttle.windows();
        let (store, _) = windows.parts(self.scope);
        if let Some(entry) = store.get_mut(&self.key).and_then(|list| list.iter_mut().find(|e| e.id == self.id)) {
            entry.settled = true;
        }
    }
}

impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        if self.kept {
            return;
        }
        let mut windows = self.throttle.windows();
        let (store, _) = windows.parts(self.scope);
        if let Some(list) = store.get_mut(&self.key) {
            if let Some(i) = list.iter().position(|e| e.id == self.id) {
                list.remove(i);
            }
            if list.is_empty() {
                store.remove(&self.key);
            }
        }
    }
}

fn normalize(value: &str) -> String {
    crate::crypto::js_trim(value).to_lowercase()
}

/// Untrusted credential input must never address the user-id/reauth counter namespace.
pub fn email_key(email: &str) -> String {
    format!("email:{}", normalize(email))
}

/// Keep credential attempts separate by client address, without sharing internal user keys.
/// No address keeps the existing conservative account bucket (also used by direct callers).
pub fn login_key(account_key: &str, ip: Option<&str>) -> String {
    match ip {
        Some(ip) => serde_json::to_string(&("login", account_key, ip)).unwrap(),
        None => account_key.to_owned(),
    }
}

fn prune(list: &mut Vec<Entry>, now_ms: i64) {
    let cutoff = now_ms - WINDOW_MS;
    list.retain(|e| e.at > cutoff);
}

fn locked_in(store: &mut HashMap<String, Vec<Entry>>, key: String, cap: usize, now_ms: i64) -> bool {
    let Some(list) = store.get_mut(&key) else { return false };
    prune(list, now_ms);
    let count = list.len();
    if count == 0 {
        store.remove(&key);
    }
    count >= cap
}

fn record_in(store: &mut HashMap<String, Vec<Entry>>, key: String, entry: Entry) {
    let now_ms = entry.at;
    let list = store.entry(key).or_default();
    prune(list, now_ms);
    list.push(entry);
    if store.len() > SWEEP_THRESHOLD {
        store.retain(|_, list| {
            prune(list, now_ms);
            !list.is_empty()
        });
    }
}

impl LoginThrottle {
    fn windows(&self) -> std::sync::MutexGuard<'_, Windows> {
        // A panic while holding the lock leaves plain counters behind; keep using them.
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Checks the lock and counts the attempt in one critical section. Checking first and
    /// recording after the password check let a concurrent burst pass the check before any
    /// failure landed. `None` when locked (nothing recorded).
    pub fn begin_account(&self, email: &str, now_ms: i64) -> Option<Attempt<'_>> {
        self.admit(Scope::Account, normalize(email), MAX_FAILURES, now_ms)
    }

    pub fn begin_ip(&self, ip: &str, now_ms: i64) -> Option<Attempt<'_>> {
        self.admit(Scope::Ip, normalize(ip), MAX_IP_FAILURES, now_ms)
    }

    pub fn begin_passkey_start(&self, ip: &str, now_ms: i64) -> bool {
        match self.admit(Scope::PasskeyStart, normalize(ip), MAX_PASSKEY_STARTS, now_ms) {
            Some(attempt) => { attempt.keep(); true }
            None => false,
        }
    }

    fn admit(&self, scope: Scope, key: String, cap: usize, now_ms: i64) -> Option<Attempt<'_>> {
        let mut windows = self.windows();
        let id = windows.next_id;
        let (store, noticed) = windows.parts(scope);
        if locked_in(store, key.clone(), cap, now_ms) {
            return None;
        }
        // Not locked: a notice still on record belonged to a lock that has ended.
        noticed.remove(&key);
        record_in(store, key.clone(), Entry { at: now_ms, id, settled: false });
        if noticed.len() > SWEEP_THRESHOLD {
            noticed.retain(|k| store.contains_key(k));
        }
        windows.next_id += 1;
        Some(Attempt { throttle: self, scope, key, id, kept: false })
    }

    /// True exactly once per lock episode, so a burst writes one audit row, not one per request.
    pub fn take_account_lock_notice(&self, email: &str, now_ms: i64) -> bool {
        self.take_notice(Scope::Account, normalize(email), MAX_FAILURES, now_ms)
    }

    pub fn take_ip_lock_notice(&self, ip: &str, now_ms: i64) -> bool {
        self.take_notice(Scope::Ip, normalize(ip), MAX_IP_FAILURES, now_ms)
    }

    fn take_notice(&self, scope: Scope, key: String, cap: usize, now_ms: i64) -> bool {
        let mut windows = self.windows();
        let (store, noticed) = windows.parts(scope);
        if !locked_in(store, key.clone(), cap, now_ms) {
            noticed.remove(&key);
            return false;
        }
        noticed.insert(key)
    }

    pub fn is_account_locked(&self, email: &str, now_ms: i64) -> bool {
        locked_in(&mut self.windows().accounts, normalize(email), MAX_FAILURES, now_ms)
    }

    pub fn record_account_failure(&self, email: &str, now_ms: i64) {
        self.record_settled(Scope::Account, normalize(email), now_ms);
    }

    fn record_settled(&self, scope: Scope, key: String, now_ms: i64) {
        let mut windows = self.windows();
        let id = windows.next_id;
        windows.next_id += 1;
        let (store, _) = windows.parts(scope);
        record_in(store, key, Entry { at: now_ms, id, settled: true });
    }

    /// A success forgives the failures already recorded. Attempts still in flight stay: one that
    /// settles as a failure afterwards counts, as when failures were recorded after the check.
    pub fn clear_account(&self, email: &str) {
        let mut windows = self.windows();
        let key = normalize(email);
        if let Some(list) = windows.accounts.get_mut(&key) {
            list.retain(|e| !e.settled);
            if list.is_empty() {
                windows.accounts.remove(&key);
            }
        }
        windows.noticed_accounts.remove(&key);
    }

    pub fn is_ip_locked(&self, ip: &str, now_ms: i64) -> bool {
        locked_in(&mut self.windows().ips, normalize(ip), MAX_IP_FAILURES, now_ms)
    }

    pub fn record_ip_failure(&self, ip: &str, now_ms: i64) {
        self.record_settled(Scope::Ip, normalize(ip), now_ms);
    }

    #[cfg(test)]
    fn sizes(&self) -> (usize, usize) {
        let w = self.windows();
        (w.accounts.len(), w.ips.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passkey_starts_are_counted_and_never_lock_password_signin() {
        let t = LoginThrottle::default();
        for _ in 0..MAX_PASSKEY_STARTS { assert!(t.begin_passkey_start("ip", 1000)); }
        assert!(!t.begin_passkey_start("ip",1001));
        assert!(t.begin_ip("ip",1001).is_some());
        assert!(t.begin_passkey_start("ip",1000 + WINDOW_MS));
    }

    const T0: i64 = 1_800_000_000_000;

    #[test]
    fn account_locks_at_the_cap_within_the_window() {
        let t = LoginThrottle::default();
        for i in 0..MAX_FAILURES - 1 {
            t.record_account_failure("a@example.test", T0 + i as i64);
        }
        assert!(!t.is_account_locked("a@example.test", T0 + 100));
        t.record_account_failure("a@example.test", T0 + 100);
        assert!(t.is_account_locked("a@example.test", T0 + 100));
    }

    #[test]
    fn failures_age_out_of_the_sliding_window() {
        let t = LoginThrottle::default();
        for _ in 0..MAX_FAILURES {
            t.record_account_failure("a@example.test", T0);
        }
        assert!(t.is_account_locked("a@example.test", T0 + WINDOW_MS - 1));
        // TS keeps `t > now − WINDOW`: a failure exactly one window old no longer counts.
        assert!(!t.is_account_locked("a@example.test", T0 + WINDOW_MS));
        assert_eq!(t.sizes().0, 0, "an emptied entry is removed");
    }

    #[test]
    fn account_keys_are_trimmed_and_lowercased() {
        let t = LoginThrottle::default();
        for _ in 0..MAX_FAILURES {
            t.record_account_failure("  Admin@Example.TEST\u{feff}", T0);
        }
        assert!(t.is_account_locked("admin@example.test", T0));
        t.clear_account(" ADMIN@example.test ");
        assert!(!t.is_account_locked("admin@example.test", T0));
    }

    #[test]
    fn ip_window_is_independent_and_looser() {
        let t = LoginThrottle::default();
        for _ in 0..MAX_IP_FAILURES - 1 {
            t.record_ip_failure("203.0.113.5", T0);
        }
        assert!(!t.is_ip_locked("203.0.113.5", T0));
        t.record_ip_failure("203.0.113.5", T0);
        assert!(t.is_ip_locked("203.0.113.5", T0));
        assert!(!t.is_account_locked("203.0.113.5", T0));
        // Clearing an account never clears an IP: one valid login must not reset a spray.
        t.clear_account("203.0.113.5");
        assert!(t.is_ip_locked("203.0.113.5", T0));
    }

    #[test]
    fn admission_counts_the_attempt_so_a_burst_stops_at_the_cap() {
        let t = LoginThrottle::default();
        let mut held = Vec::new();
        for _ in 0..100 {
            if let Some(attempt) = t.begin_account("a@example.test", T0) {
                held.push(attempt);
            }
        }
        assert_eq!(held.len(), MAX_FAILURES, "nothing settled yet, still capped");
        let mut ips = Vec::new();
        for _ in 0..100 {
            if let Some(attempt) = t.begin_ip("203.0.113.5", T0) {
                ips.push(attempt);
            }
        }
        assert_eq!(ips.len(), MAX_IP_FAILURES);
    }

    #[test]
    fn an_attempt_dropped_unsettled_is_released_and_a_kept_one_counts() {
        let t = LoginThrottle::default();
        for _ in 0..MAX_FAILURES {
            drop(t.begin_account("a@example.test", T0).unwrap());
            drop(t.begin_ip("203.0.113.5", T0).unwrap());
        }
        assert!(!t.is_account_locked("a@example.test", T0));
        assert!(!t.is_ip_locked("203.0.113.5", T0));
        for _ in 0..MAX_FAILURES {
            t.begin_account("a@example.test", T0).unwrap().keep();
        }
        assert!(t.is_account_locked("a@example.test", T0));
    }

    #[test]
    fn a_lock_is_announced_once_per_episode() {
        let t = LoginThrottle::default();
        for _ in 0..MAX_FAILURES {
            t.begin_account("a@example.test", T0).unwrap().keep();
        }
        assert!(t.take_account_lock_notice("a@example.test", T0));
        assert!(!t.take_account_lock_notice("a@example.test", T0), "same lock, no second notice");
        // The window empties, a new burst locks again: that is a new episode.
        let later = T0 + WINDOW_MS;
        for _ in 0..MAX_FAILURES {
            t.begin_account("a@example.test", later).unwrap().keep();
        }
        assert!(t.take_account_lock_notice("a@example.test", later));
        for _ in 0..MAX_IP_FAILURES {
            t.begin_ip("203.0.113.5", T0).unwrap().keep();
        }
        assert!(t.take_ip_lock_notice("203.0.113.5", T0));
        assert!(!t.take_ip_lock_notice("203.0.113.5", T0));
    }

    #[test]
    fn a_release_removes_only_its_own_entry_even_in_the_same_millisecond() {
        let t = LoginThrottle::default();
        let released = t.begin_account("a@example.test", T0).unwrap();
        t.clear_account("a@example.test");
        for _ in 0..MAX_FAILURES - 1 {
            t.begin_account("a@example.test", T0).unwrap().keep();
        }
        drop(released);
        t.begin_account("a@example.test", T0).unwrap().keep();
        assert!(t.is_account_locked("a@example.test", T0), "another request's failure was erased");
    }

    #[test]
    fn a_success_clears_settled_failures_but_not_ones_still_in_flight() {
        // As when failures were recorded after the password check: one that settles after a
        // success still counts.
        let t = LoginThrottle::default();
        let in_flight = t.begin_account("a@example.test", T0).unwrap();
        for _ in 0..MAX_FAILURES - 1 {
            t.begin_account("a@example.test", T0).unwrap().keep();
        }
        t.clear_account("a@example.test");
        in_flight.keep();
        for _ in 0..MAX_FAILURES - 2 {
            t.begin_account("a@example.test", T0).unwrap().keep();
        }
        assert!(!t.is_account_locked("a@example.test", T0));
        t.begin_account("a@example.test", T0).unwrap().keep();
        assert!(t.is_account_locked("a@example.test", T0), "the in-flight failure must count");
    }

    #[test]
    fn large_stores_sweep_expired_one_shot_keys() {
        let t = LoginThrottle::default();
        for i in 0..=SWEEP_THRESHOLD {
            t.record_account_failure(&format!("user{i}@example.test"), T0);
        }
        assert_eq!(t.sizes().0, SWEEP_THRESHOLD + 1);
        t.record_account_failure("late@example.test", T0 + WINDOW_MS + 1);
        assert_eq!(t.sizes().0, 1, "only the fresh key survives the sweep");
    }

    #[test]
    fn login_pair_keys_isolate_addresses_and_clear_only_the_matching_pair() {
        let t = LoginThrottle::default();
        let a = login_key(&email_key(" Admin@Example.TEST "), Some("203.0.113.1"));
        let b = login_key(&email_key("admin@example.test"), Some("203.0.113.2"));
        for _ in 0..MAX_FAILURES { t.begin_account(&a, T0).unwrap().keep(); }
        assert!(t.is_account_locked(&a, T0));
        assert!(t.begin_account(&b, T0).is_some());
        t.clear_account(&b);
        assert!(t.is_account_locked(&a, T0));
        assert_ne!(a, email_key("[\"login\",\"admin@example.test\",\"203.0.113.1\"]"));
        assert_eq!(login_key(&email_key("admin@example.test"), None), email_key("admin@example.test"));
    }

}
