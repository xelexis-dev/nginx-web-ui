//! Five-minute, single-use ceremonies. Anonymous starts cannot evict account enrollments.
use std::collections::HashMap;
use std::sync::Mutex;

use axum::http::HeaderMap;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

pub const TTL_MS: i64 = 300_000;
pub const CAPACITY: usize = 10_000;

pub struct Registration {
    pub user_id: String,
    pub session_version: i32,
    pub password_hash: String,
    pub user_handle: Vec<u8>,
    pub name: String,
    pub mfa_verified: bool,
}

pub struct Ceremony {
    pub challenge: [u8; 32],
    pub rp_id: String,
    pub origin: String,
    pub expires_ms: i64,
    pub registration: Option<Registration>,
}

#[derive(Default)]
pub struct Ceremonies {
    inner: Mutex<Entries>,
}

#[derive(Default)]
struct Entries {
    signin: HashMap<String, Ceremony>,
    registration: HashMap<String, Ceremony>,
}

impl Ceremonies {
    pub fn create(
        &self,
        rp_id: String,
        origin: String,
        registration: Option<Registration>,
        now_ms: i64,
    ) -> Option<(String, [u8; 32])> {
        let mut raw = [0; 32];
        let mut challenge = [0; 32];
        getrandom::fill(&mut raw).ok()?;
        getrandom::fill(&mut challenge).ok()?;
        let id = URL_SAFE_NO_PAD.encode(raw);
        let mut entries = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let bucket = if registration.is_some() {
            &mut entries.registration
        } else {
            &mut entries.signin
        };
        bucket.retain(|_, e| now_ms < e.expires_ms);
        if bucket.len() >= CAPACITY {
            let oldest = bucket
                .iter()
                .min_by_key(|(_, e)| e.expires_ms)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                bucket.remove(&id);
            }
        }
        bucket.insert(
            id.clone(),
            Ceremony {
                challenge,
                rp_id,
                origin,
                registration,
                expires_ms: now_ms + TTL_MS,
            },
        );
        Some((id, challenge))
    }

    /// Remove before parsing or verifying, including wrong-operation and expired finishes.
    pub fn take(&self, id: &str, registration: bool, now_ms: i64) -> Option<Ceremony> {
        let mut entries = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let entry = entries
            .signin
            .remove(id)
            .or_else(|| entries.registration.remove(id))?;
        (entry.expires_ms > now_ms && entry.registration.is_some() == registration).then_some(entry)
    }

    #[cfg(feature = "test-support")]
    pub fn insert_fixture(&self, id: &str, entry: Ceremony) {
        let mut entries = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let bucket = if entry.registration.is_some() {
            &mut entries.registration
        } else {
            &mut entries.signin
        };
        bucket.insert(id.into(), entry);
    }

    pub fn remove(&self, id: &str) {
        let _ = self.take(id, false, i64::MAX);
    }
}

pub fn cookie_name(secure: bool) -> &'static str {
    if secure {
        "__Host-nwui-passkey"
    } else {
        "nwui-passkey"
    }
}

pub fn read_cookie(headers: &HeaderMap, secure: bool) -> Option<String> {
    let mut values = super::session::request_cookies(headers)
        .into_iter()
        .filter(|(n, _)| n == cookie_name(secure))
        .map(|(_, v)| v);
    match (values.next(), values.next()) {
        (Some(value), None) if !value.is_empty() => Some(value),
        _ => None,
    }
}

pub fn set_cookie(secure: bool, id: &str) -> String {
    cookie(secure, id, TTL_MS / 1000)
}
pub fn clear_cookie(secure: bool) -> String {
    cookie(secure, "", 0)
}
fn cookie(secure: bool, id: &str, age: i64) -> String {
    format!(
        "{}={id}; Path=/; Max-Age={age}; HttpOnly; SameSite=Lax{}",
        cookie_name(secure),
        if secure { "; Secure" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn registration() -> Registration {
        Registration {
            user_id: "u".into(),
            session_version: 1,
            password_hash: "hash".into(),
            user_handle: vec![1; 32],
            name: "key".into(),
            mfa_verified: true,
        }
    }
    #[test]
    fn entries_are_random_single_use_expiring_and_operation_bound() {
        let store = Ceremonies::default();
        let make = |reg| {
            store
                .create("localhost".into(), "http://localhost".into(), reg, 1000)
                .unwrap()
        };
        let (id, challenge) = make(None);
        let (other, other_challenge) = make(None);
        assert_eq!(id.len(), 43);
        assert_ne!(id, other);
        assert_ne!(challenge, other_challenge);
        assert_eq!(store.take(&id, false, 1001).unwrap().challenge, challenge);
        assert!(store.take(&id, false, 1001).is_none());
        assert!(store.take(&other, false, 1000 + TTL_MS).is_none());
        let (id, _) = make(Some(registration()));
        assert!(store.take(&id, false, 1001).is_none());
        assert!(store.take(&id, true, 1001).is_none());
    }
    #[test]
    fn anonymous_flood_cannot_evict_registration_and_oldest_is_evicted() {
        let store = Ceremonies::default();
        let (reg, _) = store
            .create(
                "localhost".into(),
                "http://localhost".into(),
                Some(registration()),
                0,
            )
            .unwrap();
        let (first, _) = store
            .create("localhost".into(), "http://localhost".into(), None, 0)
            .unwrap();
        for i in 1..=CAPACITY {
            store
                .create(
                    "localhost".into(),
                    "http://localhost".into(),
                    None,
                    i as i64,
                )
                .unwrap();
        }
        assert!(store.take(&first, false, 10001).is_none());
        assert!(store.take(&reg, true, 10001).is_some());
        assert_eq!(store.inner.lock().unwrap().signin.len(), CAPACITY);
    }
    #[test]
    fn duplicate_cookie_is_refused_and_secure_cookie_is_host_scoped() {
        let mut headers = HeaderMap::new();
        headers.insert("cookie", "nwui-passkey=a; nwui-passkey=b".parse().unwrap());
        assert!(read_cookie(&headers, false).is_none());
        for secure in [false, true] {
            for (cookie, value, max_age) in [
                (set_cookie(secure, "id"), "id", 300),
                (clear_cookie(secure), "", 0),
            ] {
                assert!(cookie.starts_with(&format!("{}={value};", cookie_name(secure))));
                for attr in ["Path=/", "HttpOnly", "SameSite=Lax"] {
                    assert!(cookie.contains(attr));
                }
                assert!(cookie.contains(&format!("Max-Age={max_age}")));
                assert_eq!(cookie.contains("; Secure"), secure);
                assert!(!cookie.contains("Domain="));
            }
        }
    }
}
