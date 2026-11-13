//! Cross-language golden vectors (TS ↔ Rust). Ignored by default because they need the fixture
//! directory containing the TypeScript reference vectors:
//!
//!     NWUI_COMPAT_FIXTURES=<dir> cargo test -- --ignored compat_
//!
//! `compat_rust_to_ts_vectors` writes `<dir>/out/rust-vectors.json`, which
//! `verify-rust-output.mjs` then checks with the installed TypeScript code.

use std::path::PathBuf;

use serde_json::{Value, json};

use super::{password, session, token_digest, totp};
use crate::crypto::SecretBox;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(std::env::var("NWUI_COMPAT_FIXTURES").expect("set NWUI_COMPAT_FIXTURES to the golden fixture directory"))
}

fn ts() -> Value {
    let raw = std::fs::read_to_string(fixtures_dir().join("ts-vectors.json")).expect("ts-vectors.json");
    serde_json::from_str(&raw).expect("valid JSON")
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("expected string, got {v}"))
}

/// The instant every TS → Rust time check is judged at: the generator's recorded reference time,
/// never the wall clock, so a fixture gives the same verdict years later (a 1-hour `nbf` token
/// stays "not yet valid", 10-year sessions never expire under the test).
fn reference_time(fixture: &Value) -> i64 {
    fixture["meta"]["referenceTimeSec"].as_i64().expect("meta.referenceTimeSec — regenerate the fixtures")
}

/// Wall clock, only for values freshly made here for TS to verify right away.
fn wall_clock() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

fn key_from_hex(hex_key: &str) -> SecretBox {
    let bytes: [u8; 32] = hex::decode(hex_key).unwrap().try_into().unwrap();
    SecretBox::from_key(bytes)
}

fn cookie_pairs(v: &Value) -> Vec<(String, String)> {
    v.as_array().unwrap().iter().map(|c| (s(&c["name"]).to_owned(), s(&c["value"]).to_owned())).collect()
}

fn read(pairs: &[(String, String)], name: &str) -> Result<Option<String>, session::SessionError> {
    session::read_cookie(pairs.iter().map(|(n, v)| (n.as_str(), v.as_str())), name)
}

#[test]
#[ignore = "golden vectors: set NWUI_COMPAT_FIXTURES"]
fn compat_hkdf_and_kid_match_auth_js() {
    for v in ts()["hkdf"].as_array().unwrap().iter().filter(|v| v["length"] == 64) {
        let key = session::derive_key(s(&v["ikm"]), s(&v["salt"]));
        assert_eq!(hex::encode(key), s(&v["okmHex"]), "salt {}", v["salt"]);
        assert_eq!(session::key_id(&key), s(&v["kid"]));
    }
}

#[test]
#[ignore = "golden vectors: set NWUI_COMPAT_FIXTURES"]
fn compat_ts_sessions_decode_with_identical_claims() {
    let fixture = ts();
    let now = reference_time(&fixture);
    for v in fixture["session"]["valid"].as_array().unwrap() {
        let secrets: Vec<&str> = v["secrets"].as_array().unwrap().iter().map(s).collect();
        let claims = session::decode(s(&v["token"]), &secrets, s(&v["salt"]), now).unwrap_or_else(|e| panic!("{}: {e}", v["id"]));
        assert_eq!(Value::Object(claims), v["claims"], "{}", v["id"]);
    }
}

#[test]
#[ignore = "golden vectors: set NWUI_COMPAT_FIXTURES"]
fn compat_ts_sessions_rejected_where_required() {
    let fixture = ts();
    let now = reference_time(&fixture);
    let session_fx = &fixture["session"];
    let secret = s(&session_fx["secrets"]["current"]);
    let plain = s(&session_fx["cookieNames"]["plain"]);
    for v in session_fx["crossSalt"].as_array().unwrap() {
        assert!(session::decode(s(&v["token"]), &[secret], s(&v["readSalt"]), now).is_err(), "{}", v["id"]);
    }
    let check = |key: &str, want: session::SessionError| {
        let got = session::decode(s(&session_fx[key]["token"]), &[secret], plain, now);
        assert_eq!(got, Err(want), "{key}");
    };
    check("nbfFuture", session::SessionError::NotYetValid);
    // Application policy: A256CBC-HS512 only (Auth.js itself accepts the A256GCM form).
    assert_eq!(s(&session_fx["a256gcm"]["authjs"]), "accepted");
    check("a256gcm", session::SessionError::UnsupportedAlgorithm);
    check("a128cbc", session::SessionError::UnsupportedAlgorithm);
    check("nonNumericExp", session::SessionError::InvalidClaim("exp"));
}

#[test]
#[ignore = "golden vectors: set NWUI_COMPAT_FIXTURES"]
fn compat_auth_js_chunked_cookies_reassemble() {
    let fixture = ts();
    let now = reference_time(&fixture);
    let session_fx = &fixture["session"];
    let secret = s(&session_fx["secrets"]["current"]);
    let plain = s(&session_fx["cookieNames"]["plain"]);
    assert_eq!(session_fx["chunked"]["authjsReadsBack"], true, "Auth.js itself must read its own chunks back");
    for key in ["chunked", "mixed", "unrelatedPrefix"] {
        let pairs = cookie_pairs(&session_fx[key]["cookies"]);
        let token = read(&pairs, plain).unwrap().unwrap_or_else(|| panic!("{key}: no cookie"));
        let claims = session::decode(&token, &[secret], plain, now).unwrap();
        assert_eq!(claims["email"], session_fx[key]["expectedEmail"], "{key}");
    }
}

#[test]
#[ignore = "golden vectors: set NWUI_COMPAT_FIXTURES"]
fn compat_ts_sealed_values_open() {
    let fixture = ts();
    let seal = &fixture["seal"];
    let secret_box = key_from_hex(s(&seal["keyHex"]));
    let wrong = key_from_hex(s(&seal["wrongKeyHex"]));
    for v in seal["vectors"].as_array().unwrap() {
        let sealed = s(&v["sealed"]);
        assert_eq!(secret_box.open(Some(sealed)).as_deref(), Some(s(&v["plaintext"])), "{}", v["label"]);
        assert_eq!(secret_box.open_maybe_plain(Some(sealed)).as_deref(), Some(s(&v["plaintext"])));
        assert_eq!(wrong.open(Some(sealed)), None);
    }
    for v in seal["legacyPlain"].as_array().unwrap() {
        assert_eq!(secret_box.open_maybe_plain(Some(s(&v["stored"]))).as_deref(), v["openMaybePlain"].as_str(), "{}", v["label"]);
        assert_eq!(secret_box.open(Some(s(&v["stored"]))).as_deref(), v["open"].as_str());
    }
    for v in seal["malformed"].as_array().unwrap() {
        assert_eq!(secret_box.open_maybe_plain(Some(s(&v["stored"]))).as_deref(), v["openMaybePlain"].as_str(), "{}", v["label"]);
    }
    let dir = std::env::temp_dir().join(format!("nwui-compat-keys-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for v in seal["keyFiles"].as_array().unwrap() {
        let file = dir.join(s(&v["label"]));
        std::fs::write(&file, s(&v["content"])).unwrap();
        assert_eq!(SecretBox::load_or_create(&file).is_ok(), v["ok"].as_bool().unwrap(), "{}", v["label"]);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[ignore = "golden vectors: set NWUI_COMPAT_FIXTURES"]
fn compat_bcryptjs_hashes_verify_identically() {
    let fixture = ts();
    for v in fixture["password"]["hashes"].as_array().unwrap() {
        for check in v["checks"].as_array().unwrap() {
            let got = password::verify_password(s(&check["candidate"]), s(&v["hash"]));
            assert_eq!(got, check["expect"].as_bool().unwrap(), "{} candidate {:?}", v["label"], check["candidate"]);
        }
    }
    for v in fixture["password"]["policy"].as_array().unwrap() {
        assert_eq!(password::validate_password(s(&v["input"])).as_deref(), v["message"].as_str(), "{}", v["input"]);
    }
}

#[test]
#[ignore = "golden vectors: set NWUI_COMPAT_FIXTURES"]
fn compat_backup_codes_match() {
    let fixture = ts();
    let backup = &fixture["backup"];
    let hashes: Vec<String> = backup["hashes"].as_array().unwrap().iter().map(|h| s(h).to_owned()).collect();
    for v in backup["looksLike"].as_array().unwrap() {
        assert_eq!(totp::looks_like_backup_code(s(&v["input"])), v["expect"].as_bool().unwrap(), "{}", v["input"]);
    }
    for v in backup["consume"].as_array().unwrap() {
        assert_eq!(totp::find_backup_code(s(&v["input"]), &hashes), v["consumedIndex"].as_u64().map(|i| i as usize), "{}", v["input"]);
    }
}

#[test]
#[ignore = "golden vectors: set NWUI_COMPAT_FIXTURES"]
fn compat_totp_codes_and_window_match_otpauth() {
    let fixture = ts();
    let totp_fx = &fixture["totp"];
    let secret_of = |label: &str| {
        let b32 = totp_fx["secrets"].as_array().unwrap().iter().find(|v| v["label"] == label).unwrap();
        s(&b32["base32"]).to_owned()
    };
    for v in totp_fx["codes"].as_array().unwrap() {
        let bytes = totp::decode_secret(&secret_of(s(&v["secret"]))).unwrap();
        assert_eq!(totp::code_at(&bytes, v["timestampMs"].as_u64().unwrap()), s(&v["code"]), "{} @{}", v["secret"], v["timestampMs"]);
    }
    for v in totp_fx["validate"].as_array().unwrap() {
        let b32 = secret_of(s(&v["secret"]));
        let got = totp::verify(&b32, s(&v["token"]), v["timestampMs"].as_u64().unwrap()).map(|m| m.delta);
        let short = totp::decode_secret(&b32).unwrap().len() < totp::MIN_SECRET_BYTES;
        // Policy deviation (RFC 4226 R6): a secret under 128 bits is refused even where otpauth accepts it.
        let want = if short { None } else { v["expectDelta"].as_i64() };
        assert_eq!(got, want, "{} offset {}", v["secret"], v["tokenFromOffsetMs"]);
    }
    for v in totp_fx["normalize"].as_array().unwrap() {
        let cleaned = totp::normalize_token(s(&v["input"]));
        assert_eq!(cleaned, s(&v["cleaned"]), "{:?}", v["input"]);
        let shape = cleaned.len() == 6 && cleaned.bytes().all(|b| b.is_ascii_digit());
        assert_eq!(shape, v["shapeOk"].as_bool().unwrap());
    }
}

#[test]
#[ignore = "golden vectors: set NWUI_COMPAT_FIXTURES"]
fn compat_token_digests_match() {
    for v in ts()["tokens"].as_array().unwrap() {
        assert_eq!(token_digest(s(&v["token"])), s(&v["digest"]));
    }
}

/// Rust → TS: values made here must be accepted by the TS code (`verify-rust-output.mjs`).
#[test]
#[ignore = "golden vectors: set NWUI_COMPAT_FIXTURES"]
fn compat_rust_to_ts_vectors() {
    let fixture = ts();
    let secret_box = key_from_hex(s(&fixture["seal"]["keyHex"]));
    let seal: Vec<Value> = fixture["seal"]["vectors"].as_array().unwrap().iter()
        .map(|v| json!({"label": v["label"], "plaintext": v["plaintext"], "sealed": secret_box.seal(s(&v["plaintext"])).unwrap()}))
        .collect();

    let passwords: Vec<Value> = fixture["password"]["hashes"].as_array().unwrap().iter()
        .map(|v| json!({"label": v["label"], "password": v["password"], "hash": password::hash_password(s(&v["password"])).unwrap()}))
        .collect();

    let session_fx = &fixture["session"];
    let secret = s(&session_fx["secrets"]["current"]);
    let claims = session_fx["valid"][0]["claims"].as_object().unwrap().clone();
    let issued_at = wall_clock();
    let sessions: Vec<Value> = ["plain", "secure"].iter().map(|kind| {
        let name = s(&session_fx["cookieNames"][*kind]);
        let token = session::encode(&claims, secret, name, issued_at, session::SESSION_MAX_AGE_SECS).unwrap();
        json!({"cookieName": name, "token": token, "iat": issued_at, "exp": issued_at + session::SESSION_MAX_AGE_SECS})
    }).collect();
    // A token over one cookie's size, split by `chunk_cookie`, for Auth.js to reassemble.
    let plain = s(&session_fx["cookieNames"]["plain"]);
    let mut big_claims = claims.clone();
    big_claims.insert("pad".into(), "x".repeat(6000).into());
    let big = session::encode(&big_claims, secret, plain, issued_at, session::SESSION_MAX_AGE_SECS).unwrap();
    let chunked: Vec<Value> = session::chunk_cookie(plain, &big).into_iter()
        .map(|(name, value)| json!({"name": name, "value": value}))
        .collect();
    assert!(chunked.len() > 1, "the padded token must need chunking");

    let rfc = totp::decode_secret("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").unwrap();
    let totp_codes: Vec<Value> = [59_000u64, 1_700_000_000_000, 2_000_000_000_000].iter()
        .map(|ts| json!({"base32": "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", "timestampMs": ts, "code": totp::code_at(&rfc, *ts)}))
        .collect();
    let generated_secret = totp::generate_secret();

    let codes = totp::generate_backup_codes(3);
    let backup_hashes = totp::hash_backup_codes(&codes).unwrap();

    let tokens: Vec<Value> = fixture["tokens"].as_array().unwrap().iter()
        .map(|v| json!({"token": v["token"], "digest": token_digest(s(&v["token"]))}))
        .collect();

    let out = json!({
        "seal": seal, "passwords": passwords, "sessions": sessions, "claims": claims, "chunkedSession": chunked,
        "totp": {"codes": totp_codes, "generatedSecret": generated_secret},
        "backup": {"codes": codes, "hashes": backup_hashes}, "tokens": tokens,
    });
    let dir = fixtures_dir().join("out");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rust-vectors.json"), serde_json::to_string_pretty(&out).unwrap()).unwrap();
}
