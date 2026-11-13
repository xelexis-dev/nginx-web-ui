//! Session cookie codec, wire-compatible with Auth.js v5 (`@auth/core` 0.41.2) JWT sessions.
//!
//! Existing browsers hold these cookies for up to 8 hours, so the API must read them without a
//! forced logout, and must keep issuing the same format so a rollback to the TS stack keeps
//! sessions alive. Normative references (standards matrix ID JWE-1, HKDF-1, COOKIE-1):
//!
//! - RFC 7516 JWE compact serialization (five base64url parts).
//! - RFC 7518 §4.5 `dir`: the JWE Encrypted Key is the empty octet sequence. §5.2.2.2
//!   AES_CBC_HMAC_SHA2 checks the tag before decrypting and halts on mismatch. §5.2.5
//!   A256CBC-HS512 uses a 64-octet key (32 MAC + 32 ENC) and a 32-octet tag.
//! - RFC 7519 §4.1.4 `exp` MUST NOT be accepted on or after its value; implementers MAY allow a
//!   small leeway (we use Auth.js's 15 s). §4.1.5 `nbf`, §4.1.6 `iat` are NumericDate numbers.
//! - RFC 8725 (BCP 225) §3.1: libraries use only the caller's allowed algorithms, and each key
//!   is used with exactly one algorithm.
//!
//! **Application policy (deliberate compatibility change, not an RFC prohibition of A256GCM):**
//! only `dir` + `A256CBC-HS512` is accepted. Auth.js decode also accepts `A256GCM`, keyed with
//! the same HKDF info at 32 bytes — i.e. a prefix of the A256CBC-HS512 key — so accepting both
//! would use one key with two algorithms. Auth.js v5 issues only A256CBC-HS512; if a browser still
//! presents an A256GCM session it is rejected and that user signs in once more.
//! - RFC 7638 JWK thumbprint (SHA-512) for `kid`; RFC 5869 HKDF-SHA256 for the key.
//! - RFC 6265 §6.1 user agents SHOULD support at least 4096 bytes per cookie — the reason for
//!   chunking. (SameSite and `__Secure-` are defined by the 6265bis draft, not RFC 6265.)
//!
//! Project interoperability (not RFC): HKDF salt = cookie name, info =
//! `"Auth.js Generated Encryption Key (<cookie name>)"`; claims carry `id, role, sv,
//! totpEnabled, requires2fa` besides `sub/email/name/picture/iat/exp/jti`. The decoded claims
//! only prove the cookie was issued by us; account state, sessionVersion, role and 2FA are
//! re-checked against the database by the principal layer.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hkdf::hmac::{Hmac, Mac as _};
use josekit::jwe::{self, Dir, JweHeader};
use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256, Sha512};

pub const COOKIE_NAME_PLAIN: &str = "authjs.session-token";
pub const COOKIE_NAME_SECURE: &str = "__Secure-authjs.session-token";
/// Auth.js `session.maxAge` in `packages/auth/src/auth.config.ts`.
pub const SESSION_MAX_AGE_SECS: i64 = 8 * 60 * 60;
/// `POST /api/ui/v1/session/refresh` re-issues only below this remaining lifetime (half of
/// maxAge). Auth.js re-issued on every proxied request; this bounds cookie writes instead.
pub const RENEW_BELOW_SECS: i64 = SESSION_MAX_AGE_SECS / 2;
/// jose `clockTolerance` used by `@auth/core/jwt.js` decode.
pub const CLOCK_TOLERANCE_SECS: i64 = 15;
/// `@auth/core` `lib/utils/cookie.js`: 4096 − 160 estimated attribute bytes.
pub const CHUNK_SIZE: usize = 4096 - 160;
pub const ALGORITHM: &str = "dir";
pub const CONTENT_ENCRYPTION: &str = "A256CBC-HS512";
const KEY_LEN: usize = 64;

/// An active browser must sign in again at least once every 30 days.
pub const AUTH_MAX_AGE_SECS: i64 = 30 * 24 * 60 * 60;
pub type Claims = Map<String, Value>;

pub fn authentication_time(claims: &Claims) -> Option<i64> {
    claims.get("authTime").and_then(Value::as_i64)
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    #[error("malformed session token")]
    Malformed,
    #[error("unsupported JWE algorithm or header parameter")]
    UnsupportedAlgorithm,
    #[error("no configured secret matches the token key id")]
    NoMatchingKey,
    #[error("session token failed authentication")]
    Decryption,
    #[error("invalid {0} claim")]
    InvalidClaim(&'static str),
    #[error("session expired")]
    Expired,
    #[error("session not yet valid")]
    NotYetValid,
    #[error("could not encode session")]
    Encode,
}

/// The session cookie name for the externally visible scheme (Auth.js `useSecureCookies`).
pub fn cookie_name(secure: bool) -> &'static str {
    if secure { COOKIE_NAME_SECURE } else { COOKIE_NAME_PLAIN }
}

/// HKDF-SHA256 (RFC 5869) exactly as `@auth/core/jwt.js` `getDerivedEncryptionKey`.
pub fn derive_key(secret: &str, cookie_name: &str) -> [u8; KEY_LEN] {
    let info = format!("Auth.js Generated Encryption Key ({cookie_name})");
    let mut okm = [0u8; KEY_LEN];
    hkdf::Hkdf::<Sha256>::new(Some(cookie_name.as_bytes()), secret.as_bytes())
        .expand(info.as_bytes(), &mut okm)
        .expect("64 bytes is within the HKDF-SHA256 output limit");
    okm
}

/// RFC 7638 thumbprint of `{"k":…,"kty":"oct"}` with SHA-(8·len) — SHA-512 for our 64-byte key.
pub fn key_id(key: &[u8; KEY_LEN]) -> String {
    // RFC 7638 §3.2: required members only, lexicographic order, no whitespace.
    let jwk = format!(r#"{{"k":"{}","kty":"oct"}}"#, URL_SAFE_NO_PAD.encode(key));
    URL_SAFE_NO_PAD.encode(Sha512::digest(jwk.as_bytes()))
}

/// Decode and validate a session token. `secrets` is ordered: the first is the writing secret.
///
/// Order: shape and header allowlist → key by `kid` (the first secret when absent, as Auth.js)
/// → MAC (constant time) → decrypt → claims. Nothing is decrypted before the MAC passes.
pub fn decode(token: &str, secrets: &[&str], cookie_name: &str, now: i64) -> Result<Claims, SessionError> {
    let parts: Vec<&str> = token.split('.').collect();
    let [header_b64, encrypted_key, iv_b64, ciphertext_b64, tag_b64] = parts[..] else {
        return Err(SessionError::Malformed);
    };
    let b64 = |part: &str| URL_SAFE_NO_PAD.decode(part).map_err(|_| SessionError::Malformed);
    let header: Map<String, Value> = serde_json::from_slice(&b64(header_b64)?).map_err(|_| SessionError::Malformed)?;
    let (iv, ciphertext, tag) = (b64(iv_b64)?, b64(ciphertext_b64)?, b64(tag_b64)?);

    // RFC 8725 §3.1 algorithm allowlist; `zip`/`crit` are never written by Auth.js.
    if header.get("alg").and_then(Value::as_str) != Some(ALGORITHM)
        || header.get("enc").and_then(Value::as_str) != Some(CONTENT_ENCRYPTION)
        || header.contains_key("zip")
        || header.contains_key("crit")
    {
        return Err(SessionError::UnsupportedAlgorithm);
    }
    // RFC 7518 §4.5: `dir` uses an empty JWE Encrypted Key.
    if !encrypted_key.is_empty() {
        return Err(SessionError::Malformed);
    }

    let key = match header.get("kid") {
        None => secrets.first().map(|secret| derive_key(secret, cookie_name)),
        Some(Value::String(kid)) => secrets
            .iter()
            .map(|secret| derive_key(secret, cookie_name))
            .find(|key| key_id(key) == *kid),
        Some(_) => return Err(SessionError::Malformed),
    }
    .ok_or(SessionError::NoMatchingKey)?;

    if !mac_is_valid(&key, header_b64.as_bytes(), &iv, &ciphertext, &tag) {
        return Err(SessionError::Decryption);
    }
    let decrypter = Dir.decrypter_from_bytes(key).map_err(|_| SessionError::Decryption)?;
    let (payload, _) = jwe::deserialize_compact(token, &decrypter).map_err(|_| SessionError::Decryption)?;
    let claims: Claims = serde_json::from_slice(&payload).map_err(|_| SessionError::Malformed)?;
    validate_claims(&claims, now)?;
    Ok(claims)
}

/// jose `validateClaimsSet` with `clockTolerance: 15`, plus: `exp` is required (Auth.js always
/// writes it; a session without an expiry would never end).
fn validate_claims(claims: &Claims, now: i64) -> Result<(), SessionError> {
    let numeric = |name: &'static str| match claims.get(name) {
        None => Ok(None),
        Some(value) => value.as_f64().map(Some).ok_or(SessionError::InvalidClaim(name)),
    };
    let now = now as f64;
    let tolerance = CLOCK_TOLERANCE_SECS as f64;
    numeric("iat")?;
    if numeric("nbf")?.is_some_and(|nbf| nbf > now + tolerance) {
        return Err(SessionError::NotYetValid);
    }
    if let Some(auth_time) = numeric("authTime")? {
        if auth_time > now + tolerance { return Err(SessionError::NotYetValid); }
        if now >= auth_time + AUTH_MAX_AGE_SECS as f64 { return Err(SessionError::Expired); }
    }
    let exp = numeric("exp")?.ok_or(SessionError::InvalidClaim("exp"))?;
    if exp <= now - tolerance {
        return Err(SessionError::Expired);
    }
    Ok(())
}

/// Preserve authTime, or start it now for an unmarked legacy cookie; never reuse its old iat.
/// Encode like Auth.js `encode`: header `{alg:"dir",enc:"A256CBC-HS512",kid}`, then
/// `iat = now`, `exp = now + max_age`, `jti` = random UUID (these override the input claims).
pub fn encode(claims: &Claims, secret: &str, cookie_name: &str, now: i64, max_age: i64) -> Result<String, SessionError> {
    let key = derive_key(secret, cookie_name);
    let mut payload = claims.clone();
    if !payload.contains_key("authTime") {
        payload.insert("authTime".into(), authentication_time(claims).unwrap_or(now).into());
    }
    payload.insert("iat".into(), now.into());
    payload.insert("exp".into(), (now + max_age).into());
    payload.insert("jti".into(), uuid::Uuid::new_v4().to_string().into());
    encrypt_payload(&serde_json::to_vec(&payload).map_err(|_| SessionError::Encode)?, key)
}

fn encrypt_payload(payload: &[u8], key: [u8; KEY_LEN]) -> Result<String, SessionError> {
    let mut header = JweHeader::new();
    header.set_content_encryption(CONTENT_ENCRYPTION);
    header.set_key_id(key_id(&key));
    let encrypter = Dir.encrypter_from_bytes(key).map_err(|_| SessionError::Encode)?;
    jwe::serialize_compact(payload, &header, &encrypter).map_err(|_| SessionError::Encode)
}

/// Upper bound on accepted chunks (8 × 3936 characters). Real session tokens are far smaller.
pub const MAX_CHUNKS: usize = 8;

/// Reassemble the session cookie value from the request's cookie pairs (in header order).
///
/// - Chunks are exactly `<name>.<N>` with canonical decimal `N` (no leading zeros), non-empty
///   values of at most [`CHUNK_SIZE`] bytes, contiguous from 0, no duplicates, at most
///   [`MAX_CHUNKS`]. Any violation — duplicate name, gap, leading zero, excess count or size — is
///   ambiguous and yields `Err(Malformed)` (the caller clears the cookies). A valid chunk set
///   supersedes a stale unchunked `<name>` cookie.
/// - Without chunks, the exact `<name>` cookie is used (at most `MAX_CHUNKS × CHUNK_SIZE`
///   bytes); a duplicated `<name>` is ambiguous.
/// - Names that merely start with `<name>` (e.g. `<name>-legacy`) are unrelated and ignored.
///   Auth.js concatenates every such name and its base+chunk order is undefined
///   (`@auth/core/lib/utils/cookie.js:129-147`); observed result for a mixed set: no session.
pub fn read_cookie<'a, I>(cookies: I, cookie_name: &str) -> Result<Option<String>, SessionError>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut bases = Vec::new();
    let mut chunks: [Option<&str>; MAX_CHUNKS] = [None; MAX_CHUNKS];
    for (name, value) in cookies {
        if name == cookie_name {
            bases.push(value);
            continue;
        }
        let Some(index) = name.strip_prefix(cookie_name).and_then(|rest| rest.strip_prefix('.')) else {
            continue;
        };
        if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
            continue; // `<name>.legacy` etc. is not a chunk name
        }
        if index.len() > 1 && index.starts_with('0') {
            return Err(SessionError::Malformed);
        }
        let slot = index
            .parse::<usize>()
            .ok()
            .and_then(|i| chunks.get_mut(i))
            .ok_or(SessionError::Malformed)?;
        if slot.is_some() || value.is_empty() || value.len() > CHUNK_SIZE {
            return Err(SessionError::Malformed);
        }
        *slot = Some(value);
    }

    let count = chunks.iter().filter(|c| c.is_some()).count();
    if count > 0 {
        return chunks[..count]
            .iter()
            .map(|chunk| chunk.ok_or(SessionError::Malformed))
            .collect::<Result<String, _>>()
            .map(Some);
    }
    match bases[..] {
        [] => Ok(None),
        [value] if value.len() > MAX_CHUNKS * CHUNK_SIZE => Err(SessionError::Malformed),
        [value] => Ok(Some(value).filter(|v| !v.is_empty()).map(str::to_owned)),
        _ => Err(SessionError::Malformed),
    }
}

/// Split a value into Auth.js-compatible cookies: one `<name>` if it fits, else `<name>.0..N`.
/// Session tokens are ASCII, so characters, bytes and Auth.js's UTF-16 units coincide.
pub fn chunk_cookie(cookie_name: &str, value: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= CHUNK_SIZE {
        return vec![(cookie_name.to_owned(), value.to_owned())];
    }
    chars
        .chunks(CHUNK_SIZE)
        .enumerate()
        .map(|(i, chunk)| (format!("{cookie_name}.{i}"), chunk.iter().collect()))
        .collect()
}

/// Every `name=value` pair of every `Cookie` header, in order (RFC 6265 §5.4 cookie-string;
/// HTTP/2 may split it over several headers). Pairs without `=` or with a non-UTF-8 header are
/// skipped; values are not unquoted or decoded (Auth.js writes bare base64url).
pub fn request_cookies(headers: &axum::http::HeaderMap) -> Vec<(String, String)> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|line| line.split(';'))
        .filter_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            let name = name.trim_matches([' ', '\t']);
            (!name.is_empty()).then(|| (name.to_owned(), value.trim_matches([' ', '\t']).to_owned()))
        })
        .collect()
}

/// `<name>` itself or `<name>.<digits>` — the names Auth.js's `SessionStore` owns.
fn is_session_cookie(name: &str, cookie_name: &str) -> bool {
    name == cookie_name
        || name
            .strip_prefix(cookie_name)
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(|index| !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()))
}

fn cookie_line(name: &str, value: &str, expires: &str, max_age_zero: bool, secure: bool) -> String {
    let max_age = if max_age_zero { "; Max-Age=0" } else { "" };
    let secure = if secure { "; Secure" } else { "" };
    format!("{name}={value}; Path=/; Expires={expires}{max_age}; HttpOnly; SameSite=Lax{secure}")
}

/// RFC 9110 §5.6.7 IMF-fixdate.
fn http_date(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .unwrap_or_default()
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}

fn expire_others(cookie_name: &str, present: &[(String, String)], keep: &[String], secure: bool) -> Vec<String> {
    let mut seen: Vec<&str> = Vec::new();
    present
        .iter()
        .map(|(name, _)| name.as_str())
        .filter(|name| is_session_cookie(name, cookie_name) && !keep.iter().any(|k| k == name))
        .filter(|name| {
            let first = !seen.contains(name);
            seen.push(name);
            first
        })
        .map(|name| cookie_line(name, "", &http_date(0), true, secure))
        .collect()
}

/// `Set-Cookie` values that store `token` under `cookie_name` (chunked like Auth.js) and expire
/// every other session cookie of that name the request still carries (stale base or chunks),
/// mirroring Auth.js `SessionStore`. Attributes are Auth.js's: `Path=/; HttpOnly;
/// SameSite=Lax`, `Secure` on https, `Expires` = the session expiry.
pub fn issue_cookies(cookie_name: &str, token: &str, present: &[(String, String)], secure: bool, expires_at: i64) -> Vec<String> {
    let chunks = chunk_cookie(cookie_name, token);
    let names: Vec<String> = chunks.iter().map(|(name, _)| name.clone()).collect();
    let expires = http_date(expires_at);
    let mut lines: Vec<String> = chunks.iter().map(|(name, value)| cookie_line(name, value, &expires, false, secure)).collect();
    lines.extend(expire_others(cookie_name, present, &names, secure));
    lines
}

/// `Set-Cookie` values that expire every session cookie of that name the request carries.
pub fn clear_cookies(cookie_name: &str, present: &[(String, String)], secure: bool) -> Vec<String> {
    expire_others(cookie_name, present, &[], secure)
}

/// RFC 7518 §5.2.2.2 step 2: HMAC over `AAD ‖ IV ‖ E ‖ AL`, compared in constant time with the
/// first 32 octets. Run before josekit, whose A256CBC-HS512 decrypt checks padding first.
fn mac_is_valid(key: &[u8; KEY_LEN], aad: &[u8], iv: &[u8], ciphertext: &[u8], tag: &[u8]) -> bool {
    let mut mac = <Hmac<Sha512> as hkdf::hmac::KeyInit>::new_from_slice(&key[..KEY_LEN / 2])
        .expect("HMAC accepts any key length");
    mac.update(aad);
    mac.update(iv);
    mac.update(ciphertext);
    mac.update(&((aad.len() as u64) * 8).to_be_bytes());
    // Checks the length (32) and compares in constant time.
    mac.verify_truncated_left(tag).is_ok() && tag.len() == KEY_LEN / 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SECRET: &str = "unit-test-secret-not-for-production";
    const NOW: i64 = 1_800_000_000;

    fn claims() -> Claims {
        json!({"sub":"u1","email":"u1@example.test","id":"u1","role":"viewer","sv":2,"totpEnabled":false,"requires2fa":true})
            .as_object().unwrap().clone()
    }

    /// RFC 5869 Appendix A.1 through the same HKDF call shape (salt, info, 42 bytes).
    #[test]
    fn hkdf_matches_rfc5869_appendix_a1() {
        let ikm = [0x0bu8; 22];
        let salt: Vec<u8> = (0x00..=0x0c).collect();
        let info: Vec<u8> = (0xf0..=0xf9).collect();
        let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(&salt), &ikm);
        let mut okm = [0u8; 42];
        hk.expand(&info, &mut okm).unwrap();
        assert_eq!(
            hex::encode(okm),
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
        );
    }

    #[test]
    fn derive_key_depends_on_cookie_name() {
        assert_ne!(derive_key(SECRET, COOKIE_NAME_PLAIN), derive_key(SECRET, COOKIE_NAME_SECURE));
        assert_eq!(derive_key(SECRET, COOKIE_NAME_PLAIN), derive_key(SECRET, COOKIE_NAME_PLAIN));
    }

    #[test]
    fn key_id_is_an_86_char_base64url_sha512_thumbprint() {
        let kid = key_id(&derive_key(SECRET, COOKIE_NAME_PLAIN));
        assert_eq!(URL_SAFE_NO_PAD.decode(&kid).unwrap().len(), 64);
        assert_eq!(kid.len(), 86);
    }

    #[test]
    fn encode_then_decode_round_trips_and_sets_registered_claims() {
        let token = encode(&claims(), SECRET, COOKIE_NAME_PLAIN, NOW, SESSION_MAX_AGE_SECS).unwrap();
        assert_eq!(token.split('.').count(), 5);
        let decoded = decode(&token, &[SECRET], COOKIE_NAME_PLAIN, NOW).unwrap();
        assert_eq!(decoded["id"], "u1");
        assert_eq!(decoded["iat"], NOW);
        assert_eq!(decoded["exp"], NOW + SESSION_MAX_AGE_SECS);
        assert!(decoded["jti"].as_str().is_some_and(|j| j.len() == 36));
    }

    #[test]
    fn header_is_exactly_what_auth_js_writes() {
        let token = encode(&claims(), SECRET, COOKIE_NAME_PLAIN, NOW, 60).unwrap();
        let header: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(token.split('.').next().unwrap()).unwrap()).unwrap();
        assert_eq!(header["alg"], ALGORITHM);
        assert_eq!(header["enc"], CONTENT_ENCRYPTION);
        assert_eq!(header["kid"], key_id(&derive_key(SECRET, COOKIE_NAME_PLAIN)));
        assert!(header.get("zip").is_none());
    }

    /// RFC 7519 §4.1.4 with a 15 s leeway: accepted while now < exp + 15, rejected from exp + 15.
    #[test]
    fn exp_leeway_boundary_matches_jose() {
        let token = encode(&claims(), SECRET, COOKIE_NAME_PLAIN, NOW, 60).unwrap();
        let exp = NOW + 60;
        assert!(decode(&token, &[SECRET], COOKIE_NAME_PLAIN, exp - 1).is_ok());
        assert!(decode(&token, &[SECRET], COOKIE_NAME_PLAIN, exp + 14).is_ok());
        assert_eq!(decode(&token, &[SECRET], COOKIE_NAME_PLAIN, exp + 15), Err(SessionError::Expired));
    }

    #[test]
    fn wrong_cookie_name_or_secret_is_rejected() {
        let token = encode(&claims(), SECRET, COOKIE_NAME_PLAIN, NOW, 60).unwrap();
        assert!(decode(&token, &[SECRET], COOKIE_NAME_SECURE, NOW).is_err());
        assert!(decode(&token, &["another-secret"], COOKIE_NAME_PLAIN, NOW).is_err());
    }

    #[test]
    fn older_secret_is_found_by_key_id() {
        let token = encode(&claims(), "old-secret", COOKIE_NAME_PLAIN, NOW, 60).unwrap();
        assert!(decode(&token, &["new-secret", "old-secret"], COOKIE_NAME_PLAIN, NOW).is_ok());
    }

    /// RFC 7518 §5.2.2.2: any bit flip in header (AAD), IV, ciphertext or tag fails authentication.
    #[test]
    fn tampering_any_part_is_rejected() {
        let token = encode(&claims(), SECRET, COOKIE_NAME_PLAIN, NOW, 60).unwrap();
        let parts: Vec<&str> = token.split('.').collect();
        for index in [2usize, 3, 4] {
            let mut raw = URL_SAFE_NO_PAD.decode(parts[index]).unwrap();
            raw[0] ^= 1;
            let mut mutated: Vec<String> = parts.iter().map(|p| (*p).to_owned()).collect();
            mutated[index] = URL_SAFE_NO_PAD.encode(raw);
            assert!(decode(&mutated.join("."), &[SECRET], COOKIE_NAME_PLAIN, NOW).is_err(), "part {index}");
        }
    }

    /// RFC 7519 §4.1.5 with the same 15 s leeway as `exp`.
    #[test]
    fn nbf_leeway_boundary_matches_jose() {
        let at = |nbf: i64| {
            let mut c = claims();
            c.insert("nbf".into(), nbf.into());
            decode(&encode(&c, SECRET, COOKIE_NAME_PLAIN, NOW, 60).unwrap(), &[SECRET], COOKIE_NAME_PLAIN, NOW)
        };
        assert!(at(NOW + 15).is_ok());
        assert_eq!(at(NOW + 16), Err(SessionError::NotYetValid));
    }

    fn raw_token(payload: Value) -> String {
        encrypt_payload(&serde_json::to_vec(&payload).unwrap(), derive_key(SECRET, COOKIE_NAME_PLAIN)).unwrap()
    }

    #[test]
    fn registered_claims_must_be_numbers_and_exp_is_required() {
        let decode_raw = |payload| decode(&raw_token(payload), &[SECRET], COOKIE_NAME_PLAIN, NOW);
        assert_eq!(decode_raw(json!({"sub": "u1"})), Err(SessionError::InvalidClaim("exp")));
        assert_eq!(decode_raw(json!({"exp": NOW + 60, "iat": "x"})), Err(SessionError::InvalidClaim("iat")));
        assert_eq!(decode_raw(json!({"exp": NOW + 60, "nbf": "x"})), Err(SessionError::InvalidClaim("nbf")));
        assert!(decode_raw(json!({"exp": NOW + 60})).is_ok());
        assert_eq!(decode_raw(json!(["not", "an", "object"])), Err(SessionError::Malformed));
    }

    /// Rewrites the protected header (which also invalidates the MAC, so these must be rejected
    /// by the header allowlist before any key is used).
    fn with_header(token: &str, edit: impl FnOnce(&mut Map<String, Value>)) -> String {
        let mut parts: Vec<String> = token.split('.').map(str::to_owned).collect();
        let mut header: Map<String, Value> = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(&parts[0]).unwrap()).unwrap();
        edit(&mut header);
        parts[0] = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap());
        parts.join(".")
    }

    #[test]
    fn header_parameters_outside_the_allowlist_are_rejected() {
        let token = encode(&claims(), SECRET, COOKIE_NAME_PLAIN, NOW, 60).unwrap();
        let decode_with = |edit: fn(&mut Map<String, Value>)| decode(&with_header(&token, edit), &[SECRET], COOKIE_NAME_PLAIN, NOW);
        assert_eq!(decode_with(|h| { h.insert("alg".into(), "A256KW".into()); }), Err(SessionError::UnsupportedAlgorithm));
        assert_eq!(decode_with(|h| { h.remove("alg"); }), Err(SessionError::UnsupportedAlgorithm));
        assert_eq!(decode_with(|h| { h.insert("zip".into(), "DEF".into()); }), Err(SessionError::UnsupportedAlgorithm));
        assert_eq!(decode_with(|h| { h.insert("crit".into(), json!(["exp"])); }), Err(SessionError::UnsupportedAlgorithm));
        assert_eq!(decode_with(|h| { h.insert("kid".into(), 7.into()); }), Err(SessionError::Malformed));
        assert_eq!(decode_with(|h| { h.insert("kid".into(), "unknown".into()); }), Err(SessionError::NoMatchingKey));
        // Without `kid`, Auth.js uses the first secret; the rewritten header then fails the MAC.
        assert_eq!(decode_with(|h| { h.remove("kid"); }), Err(SessionError::Decryption));
    }

    /// RFC 7518 Appendix B.3 (AES_256_CBC_HMAC_SHA_512): the MAC we check before josekit decrypts.
    #[test]
    fn mac_check_matches_rfc7518_appendix_b3() {
        let key: [u8; KEY_LEN] = std::array::from_fn(|i| i as u8);
        let aad = b"The second principle of Auguste Kerckhoffs";
        let iv = hex::decode("1af38c2dc2b96ffdd86694092341bc04").unwrap();
        let e = hex::decode(concat!(
            "4affaaadb78c31c5da4b1b590d10ffbd3dd8d5d302423526912da037ecbcc7bd822c301dd67c373bccb584ad3e9279c2",
            "e6d12a1374b77f077553df829410446b36ebd97066296ae6427ea75c2e0846a11a09ccf5370dc80bfecbad28c73f09b3",
            "a3b75e662a2594410ae496b2e2e6609e31e6e02cc837f053d21f37ff4f51950bbe2638d09dd7a4930930806d0703b1f6",
        )).unwrap();
        let t = hex::decode("4dd3b4c088a7f45c216839645b2012bf2e6269a8c56a816dbc1b267761955bc5").unwrap();
        assert!(mac_is_valid(&key, aad, &iv, &e, &t));
        let mut flipped = t.clone();
        flipped[31] ^= 1;
        assert!(!mac_is_valid(&key, aad, &iv, &e, &flipped));
        assert!(!mac_is_valid(&key, aad, &iv, &e, &t[..16]), "a truncated tag is not accepted");
        assert!(!mac_is_valid(&key, b"other aad", &iv, &e, &t));
    }

    /// RFC 7518 §4.5: with `dir` the Encrypted Key MUST be empty.
    #[test]
    fn non_empty_encrypted_key_is_rejected() {
        let token = encode(&claims(), SECRET, COOKIE_NAME_PLAIN, NOW, 60).unwrap();
        let mut parts: Vec<String> = token.split('.').map(str::to_owned).collect();
        parts[1] = "AAAA".to_owned();
        assert_eq!(decode(&parts.join("."), &[SECRET], COOKIE_NAME_PLAIN, NOW), Err(SessionError::Malformed));
    }

    #[test]
    fn malformed_tokens_are_rejected_without_panicking() {
        for token in ["", "a.b.c", "a.b.c.d.e", "....", "a.b.c.d.e.f", "!!.!!.!!.!!.!!"] {
            assert!(decode(token, &[SECRET], COOKIE_NAME_PLAIN, NOW).is_err(), "{token:?}");
        }
    }

    #[test]
    fn chunking_splits_at_the_auth_js_size() {
        let value = "v".repeat(CHUNK_SIZE * 2 + 5);
        let chunks = chunk_cookie(COOKIE_NAME_PLAIN, &value);
        assert_eq!(chunks.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), [
            "authjs.session-token.0", "authjs.session-token.1", "authjs.session-token.2"
        ]);
        assert_eq!(chunks[0].1.len(), CHUNK_SIZE);
        assert_eq!(chunks[2].1.len(), 5);
        assert_eq!(chunk_cookie(COOKIE_NAME_PLAIN, "short"), [(COOKIE_NAME_PLAIN.to_owned(), "short".to_owned())]);
        // Auth.js: `Math.ceil(length / CHUNK_SIZE) === 1` keeps a single unchunked cookie.
        let exact = "v".repeat(CHUNK_SIZE);
        assert_eq!(chunk_cookie(COOKIE_NAME_PLAIN, &exact), [(COOKIE_NAME_PLAIN.to_owned(), exact.clone())]);
        assert_eq!(chunk_cookie(COOKIE_NAME_PLAIN, &format!("{exact}v")).len(), 2);
    }

    const B: &str = COOKIE_NAME_PLAIN;

    fn read(pairs: &[(&str, &str)]) -> Result<Option<String>, SessionError> {
        read_cookie(pairs.iter().copied(), B)
    }

    #[test]
    fn contiguous_chunks_in_any_header_order_supersede_a_stale_base() {
        let got = read(&[("authjs.session-token.1", "B"), ("authjs.session-token", "stale"), ("authjs.session-token.0", "A"),
                         ("authjs.session-token.2", "C"), ("authjs.session-token-legacy", "junk")]);
        assert_eq!(got, Ok(Some("ABC".to_owned())));
    }

    #[test]
    fn exact_base_is_used_without_chunks_and_unrelated_prefixes_are_ignored() {
        assert_eq!(read(&[("authjs.session-token", "base")]), Ok(Some("base".to_owned())));
        assert_eq!(read(&[("authjs.session-token-legacy", "junk")]), Ok(None));
        assert_eq!(read(&[("authjs.session-token", "")]), Ok(None));
        assert_eq!(read(&[("__Secure-authjs.session-token", "s")]), Ok(None));
        assert_eq!(read(&[]), Ok(None));
    }

    #[test]
    fn ambiguous_chunk_sets_are_rejected() {
        let cases: [&[(&str, &str)]; 6] = [
            &[("authjs.session-token.0", "A"), ("authjs.session-token.2", "C")],                           // gap
            &[("authjs.session-token.1", "B")],                                                           // not from zero
            &[("authjs.session-token.0", "A"), ("authjs.session-token.0", "X")],                           // duplicate
            &[("authjs.session-token.00", "A")],                                                          // leading zero
            &[("authjs.session-token.0", "A"), ("authjs.session-token.1", "")],                            // empty chunk
            &[("authjs.session-token", "a"), ("authjs.session-token", "b")],                              // duplicate base
        ];
        for pairs in cases {
            assert_eq!(read(pairs), Err(SessionError::Malformed), "{pairs:?}");
        }
    }

    #[test]
    fn request_cookies_reads_every_cookie_header() {
        let mut headers = axum::http::HeaderMap::new();
        headers.append("cookie", "a=1; authjs.session-token.0=X;b=2".parse().unwrap());
        headers.append("cookie", "authjs.session-token.1=Y; novalue; c==d".parse().unwrap());
        let got = request_cookies(&headers);
        let pairs: Vec<(&str, &str)> = got.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
        assert_eq!(pairs, [("a", "1"), ("authjs.session-token.0", "X"), ("b", "2"), ("authjs.session-token.1", "Y"), ("c", "=d")]);
        assert_eq!(read_cookie(pairs.iter().copied(), B), Ok(Some("XY".to_owned())));
    }

    fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn issuing_a_small_token_expires_stale_chunks() {
        let present = owned(&[("authjs.session-token.0", "old"), ("authjs.session-token.1", "old"), ("other", "x")]);
        let cookies = issue_cookies(B, "tok", &present, false, 1_800_000_000);
        assert_eq!(cookies, [
            "authjs.session-token=tok; Path=/; Expires=Fri, 15 Jan 2027 08:00:00 GMT; HttpOnly; SameSite=Lax",
            "authjs.session-token.0=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=0; HttpOnly; SameSite=Lax",
            "authjs.session-token.1=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=0; HttpOnly; SameSite=Lax",
        ]);
    }

    #[test]
    fn issuing_a_large_token_chunks_and_expires_the_stale_base() {
        let token = "t".repeat(CHUNK_SIZE + 1);
        let present = owned(&[("__Secure-authjs.session-token", "old"), ("__Secure-authjs.session-token.3", "old")]);
        let cookies = issue_cookies(COOKIE_NAME_SECURE, &token, &present, true, 1_800_000_000);
        let names: Vec<&str> = cookies.iter().map(|c| c.split('=').next().unwrap()).collect();
        assert_eq!(names, ["__Secure-authjs.session-token.0", "__Secure-authjs.session-token.1", "__Secure-authjs.session-token", "__Secure-authjs.session-token.3"]);
        assert!(cookies.iter().all(|c| c.ends_with("; HttpOnly; SameSite=Lax; Secure")), "{cookies:?}");
        assert!(cookies[2].contains("Max-Age=0") && cookies[3].contains("Max-Age=0"));
    }

    #[test]
    fn clearing_expires_only_that_names_cookies() {
        let present = owned(&[("authjs.session-token", "a"), ("authjs.session-token.0", "b"), ("authjs.session-token-legacy", "c"), ("x", "y")]);
        let cookies = clear_cookies(B, &present, false);
        assert_eq!(cookies.len(), 2);
        assert!(cookies[0].starts_with("authjs.session-token=; ") && cookies[1].starts_with("authjs.session-token.0=; "));
        assert!(clear_cookies(B, &[], false).is_empty());
    }

    /// Size bounds: each chunk ≤ CHUNK_SIZE (what Auth.js writes), a lone base ≤ the chunked total.
    #[test]
    fn oversized_values_are_rejected() {
        let full = "x".repeat(CHUNK_SIZE);
        let over = "x".repeat(CHUNK_SIZE + 1);
        assert!(read(&[("authjs.session-token.0", full.as_str())]).unwrap().is_some());
        assert_eq!(read(&[("authjs.session-token.0", over.as_str())]), Err(SessionError::Malformed));
        assert_eq!(read(&[("authjs.session-token.0", full.as_str()), ("authjs.session-token.1", over.as_str())]),
                   Err(SessionError::Malformed));
        let base_max = "x".repeat(MAX_CHUNKS * CHUNK_SIZE);
        let base_over = "x".repeat(MAX_CHUNKS * CHUNK_SIZE + 1);
        assert!(read(&[("authjs.session-token", base_max.as_str())]).unwrap().is_some());
        assert_eq!(read(&[("authjs.session-token", base_over.as_str())]), Err(SessionError::Malformed));
    }

    #[test]
    fn more_than_max_chunks_is_rejected() {
        let names: Vec<String> = (0..=MAX_CHUNKS).map(|i| format!("{B}.{i}")).collect();
        let pairs: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "x")).collect();
        assert_eq!(read(&pairs), Err(SessionError::Malformed));
        assert!(read(&pairs[..MAX_CHUNKS]).unwrap().is_some());
    }
}
