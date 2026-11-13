//! TOTP and recovery codes, compatible with `packages/auth/src/totp.ts` (otpauth 9.5.1).
//!
//! Normative references (standards matrix ID OTP-1):
//! - RFC 4226 §5.3 HOTP dynamic truncation; R6: the shared secret MUST be at least 128 bits
//!   (160 RECOMMENDED). New secrets are 160 bits; shorter stored secrets are refused.
//! - RFC 6238 §4: T = floor((unix − 0) / 30), HMAC-SHA-1, 6 digits (values mod 10^6, i.e. the last
//!   six digits of the RFC's 8-digit Appendix B vectors).
//! - RFC 6238 §5.2: the verifier MUST NOT accept a second use of an OTP after a successful
//!   validation. [`verify`] returns the matched absolute time step; the caller consumes it
//!   atomically (`users.totp_last_step`, accept only a strictly greater step).
//! - Window (project policy, unchanged from TS): the current step, then −1, then +1 (otpauth order).
//!
//! Recovery codes are not an RFC feature: `xxxxx-xxxxx` of 40 random bits, bcrypt-hashed after
//! normalisation, consumed once.

pub const DIGITS: usize = 6;
pub const PERIOD_SECS: u64 = 30;
pub const WINDOW: i64 = 1;
/// RFC 4226 R6 minimum (128 bits) and the size we generate (160 bits).
pub const MIN_SECRET_BYTES: usize = 16;
pub const SECRET_BYTES: usize = 20;
pub const BACKUP_CODE_COUNT: usize = 8;

/// A successful match: the absolute time step and its offset from the current step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TotpMatch {
    pub step: u64,
    pub delta: i64,
}

const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// RFC 4648 base32 as otpauth 9.5.1 `base32Decode` reads it: U+0020 removed, trailing `=`
/// stripped, upper-cased; any other character outside the alphabet is an error; leftover bits
/// are dropped.
pub fn decode_secret(base32: &str) -> Option<Vec<u8>> {
    let spaced = base32.replace(' ', "");
    let normalized = spaced.trim_end_matches('=').to_uppercase();
    let mut out = Vec::with_capacity(normalized.len() * 5 / 8);
    let (mut value, mut bits) = (0u32, 0u32);
    for c in normalized.bytes() {
        let index = BASE32_ALPHABET.iter().position(|&a| a == c)? as u32;
        value = (value << 5 | index) & 0xffff;
        bits += 5;
        if bits >= 8 {
            out.push((value >> (bits - 8)) as u8);
            bits -= 8;
        }
    }
    Some(out)
}

fn encode_base32(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut value, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        value = (value << 8 | u32::from(byte)) & 0xffff;
        bits += 8;
        while bits >= 5 {
            out.push(BASE32_ALPHABET[((value >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(BASE32_ALPHABET[((value << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// A fresh 160-bit secret as unpadded upper-case base32 (what otpauth `Secret({size:20})` emits).
///
/// # Panics
/// If the operating system RNG fails (as `uuid::Uuid::new_v4` does).
pub fn generate_secret() -> String {
    let mut secret = [0u8; SECRET_BYTES];
    getrandom::fill(&mut secret).expect("the system random number generator failed");
    encode_base32(&secret)
}

/// The HOTP value for `counter` (RFC 4226): TOTP with a 1-second step evaluated at `counter`.
pub fn hotp(secret: &[u8], counter: u64) -> String {
    totp_rs::Builder::new()
        .with_algorithm(totp_rs::Algorithm::SHA1)
        .with_digits(DIGITS as u8)
        .with_step_duration(1)
        .with_secret(secret.to_vec())
        .build_noncompliant()
        .generate(counter)
        .to_string()
}

/// The TOTP value at `timestamp_ms` (RFC 6238, 30 s step).
pub fn code_at(secret: &[u8], timestamp_ms: u64) -> String {
    hotp(secret, timestamp_ms / 1000 / PERIOD_SECS)
}

/// `token.replace(/\s/g, "")` with the ECMAScript WhiteSpace and LineTerminator set.
pub fn normalize_token(token: &str) -> String {
    token.chars().filter(|&c| !crate::crypto::is_ecmascript_whitespace(c)).collect()
}

/// Verify `token` against a stored base32 secret at `now_ms`. `None` for a bad shape, a secret
/// shorter than 128 bits, or no match in the window.
pub fn verify(secret_base32: &str, token: &str, now_ms: u64) -> Option<TotpMatch> {
    use subtle::ConstantTimeEq as _;

    let token = normalize_token(token);
    if token.len() != DIGITS || !token.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let secret = decode_secret(secret_base32).filter(|s| s.len() >= MIN_SECRET_BYTES)?;
    let current = now_ms / 1000 / PERIOD_SECS;
    // otpauth order: current, then −1, +1 for each window step.
    std::iter::once(0).chain((1..=WINDOW).flat_map(|i| [-i, i])).find_map(|delta| {
        let step = current.checked_add_signed(delta)?;
        let expected = hotp(&secret, step);
        bool::from(expected.as_bytes().ct_eq(token.as_bytes())).then_some(TotpMatch { step, delta })
    })
}

pub const ISSUER: &str = "nginx-web-ui";

/// otpauth 9.5.1 `TOTP#toString()` for our fixed parameters (`packages/auth/src/totp.ts`
/// `totpAuthUri`), with `encodeURIComponent` on issuer and label.
pub fn otpauth_uri(secret_base32: &str, label: &str) -> String {
    use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
    // `encodeURIComponent` leaves A–Z a–z 0–9 and - _ . ! ~ * ' ( ) unescaped.
    const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-').remove(b'_').remove(b'.').remove(b'!').remove(b'~').remove(b'*').remove(b'\'').remove(b'(').remove(b')');
    let e = |s: &str| utf8_percent_encode(s, COMPONENT).to_string();
    format!(
        "otpauth://totp/{}:{}?issuer={}&secret={}&algorithm=SHA1&digits={DIGITS}&period={PERIOD_SECS}",
        e(ISSUER),
        e(label),
        e(ISSUER),
        e(secret_base32)
    )
}

/// `/^[0-9a-f]{5}-?[0-9a-f]{5}$/i` after an ECMAScript `trim()`.
pub fn looks_like_backup_code(value: &str) -> bool {
    let value = crate::crypto::js_trim(value).as_bytes();
    let hex = |part: &[u8]| part.iter().all(u8::is_ascii_hexdigit);
    match value.len() {
        10 => hex(value),
        11 => value[5] == b'-' && hex(&value[..5]) && hex(&value[6..]),
        _ => false,
    }
}

/// `trim().toLowerCase().replace(/-/g, "")`.
pub fn normalize_backup_code(code: &str) -> String {
    crate::crypto::js_trim(code).to_lowercase().replace('-', "")
}

/// `count` codes of 40 random bits as `xxxxx-xxxxx`.
///
/// # Panics
/// If the operating system RNG fails.
pub fn generate_backup_codes(count: usize) -> Vec<String> {
    (0..count)
        .map(|_| {
            let mut raw = [0u8; 5];
            getrandom::fill(&mut raw).expect("the system random number generator failed");
            let hex = hex::encode(raw);
            format!("{}-{}", &hex[..5], &hex[5..])
        })
        .collect()
}

/// Blocking (one bcrypt per code): run it on the blocking pool.
pub fn hash_backup_codes(codes: &[String]) -> Result<Vec<String>, super::password::PasswordError> {
    codes.iter().map(|code| super::password::hash_password(&normalize_backup_code(code))).collect()
}

/// Index of the first stored hash matching `code`; the caller removes it atomically.
/// Blocking (up to one bcrypt per stored hash).
pub fn find_backup_code(code: &str, hashes: &[String]) -> Option<usize> {
    let normalized = normalize_backup_code(code);
    hashes.iter().position(|hash| super::password::verify_password(&normalized, hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4226 Appendix D secret "12345678901234567890".
    const RFC_SECRET: &[u8] = b"12345678901234567890";

    #[test]
    fn hotp_matches_rfc4226_appendix_d() {
        let expected = ["755224", "287082", "359152", "969429", "338314", "254676", "287922", "162583", "399871", "520489"];
        for (counter, want) in expected.iter().enumerate() {
            assert_eq!(hotp(RFC_SECRET, counter as u64), *want, "counter {counter}");
        }
    }

    /// RFC 6238 Appendix B (SHA1) — last six digits of the 8-digit values.
    #[test]
    fn totp_matches_rfc6238_appendix_b_sha1() {
        for (unix, eight) in [(59u64, "94287082"), (1_111_111_109, "07081804"), (1_111_111_111, "14050471"),
                              (1_234_567_890, "89005924"), (2_000_000_000, "69279037"), (20_000_000_000, "65353130")] {
            assert_eq!(code_at(RFC_SECRET, unix * 1000), &eight[2..], "T={unix}");
        }
    }

    #[test]
    fn verify_returns_the_matched_step_within_the_window() {
        let b32 = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        let now = 1_700_000_000_000u64;
        let step = now / 1000 / PERIOD_SECS;
        for (offset, delta) in [(-30_000i64, -1i64), (0, 0), (30_000, 1)] {
            let token = code_at(RFC_SECRET, (now as i64 + offset) as u64);
            assert_eq!(verify(b32, &token, now), Some(TotpMatch { step: (step as i64 + delta) as u64, delta }));
        }
        for offset in [-60_000i64, 60_000] {
            let token = code_at(RFC_SECRET, (now as i64 + offset) as u64);
            assert_eq!(verify(b32, &token, now), None, "offset {offset}");
        }
    }

    #[test]
    fn verify_rejects_bad_shapes_and_short_secrets() {
        let now = 1_700_000_000_000u64;
        let good = code_at(RFC_SECRET, now);
        let b32 = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        for token in ["12345", "1234567", "12a456", "１２３４５６", ""] {
            assert_eq!(verify(b32, token, now), None, "{token}");
        }
        assert!(verify(b32, &format!(" {} {} ", &good[..3], &good[3..]), now).is_some());
        // 10-byte (80-bit) secret: below the RFC 4226 R6 minimum.
        assert_eq!(verify("JBSWY3DPEHPK3PXP", "000000", now), None);
    }

    #[test]
    fn secret_decoding_follows_otpauth_normalisation() {
        assert_eq!(decode_secret("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").unwrap(), RFC_SECRET);
        assert_eq!(decode_secret("gezd gnbv gy3t qojq gezd gnbv gy3t qojq").unwrap(), RFC_SECRET);
        assert_eq!(decode_secret("JBSWY3DPEHPK3PXP===="), decode_secret("jbswy3dpehpk3pxp"));
        assert_eq!(decode_secret("GEZD1"), None); // '1' is not in the RFC 4648 alphabet
    }

    #[test]
    fn generated_secrets_are_160_bit_base32() {
        let secret = generate_secret();
        assert_eq!(secret.len(), 32);
        assert_eq!(decode_secret(&secret).unwrap().len(), SECRET_BYTES);
        assert_ne!(secret, generate_secret());
    }

    #[test]
    fn token_normalisation_uses_the_ecmascript_whitespace_set() {
        assert_eq!(normalize_token(" 123\u{00a0}456\u{3000}\u{feff}\t"), "123456");
        // U+0085 (NEL) is not ECMAScript whitespace, so it stays.
        assert_eq!(normalize_token("123\u{0085}456"), "123\u{0085}456");
    }

    #[test]
    fn backup_code_shape_and_normalisation() {
        assert!(looks_like_backup_code("a1b2c-3d4e5"));
        assert!(looks_like_backup_code("A1B2C3D4E5"));
        assert!(looks_like_backup_code(" a1b2c-3d4e5 "));
        assert!(!looks_like_backup_code("a1b2c--3d4e5"));
        assert!(!looks_like_backup_code("a1b2c_3d4e5"));
        assert!(!looks_like_backup_code("g1b2c-3d4e5"));
        assert_eq!(normalize_backup_code("  A1B2C-3D4E5 "), "a1b2c3d4e5");
        let codes = generate_backup_codes(BACKUP_CODE_COUNT);
        assert_eq!(codes.len(), 8);
        assert!(codes.iter().all(|c| looks_like_backup_code(c) && c.len() == 11 && &c[5..6] == "-"));
    }

    /// The TS fixture (`ts-vectors.json` `totp.authUri`) and `encodeURIComponent`'s reserved set.
    #[test]
    fn otpauth_uri_matches_the_ts_library() {
        assert_eq!(
            otpauth_uri("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", "admin@example.test"),
            "otpauth://totp/nginx-web-ui:admin%40example.test?issuer=nginx-web-ui&secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&algorithm=SHA1&digits=6&period=30"
        );
        assert!(otpauth_uri("A", "a b/c:d?é!~*'()").contains(":a%20b%2Fc%3Ad%3F%C3%A9!~*'()?"));
    }

    #[test]
    fn backup_codes_are_found_once_by_normalised_value() {
        let codes = vec!["a1b2c-3d4e5".to_owned(), "fffff-00000".to_owned()];
        let hashes = hash_backup_codes(&codes).unwrap();
        assert_eq!(find_backup_code("FFFFF00000", &hashes), Some(1));
        assert_eq!(find_backup_code(" a1b2c-3d4e5 ", &hashes), Some(0));
        assert_eq!(find_backup_code("00000-00000", &hashes), None);
    }
}
