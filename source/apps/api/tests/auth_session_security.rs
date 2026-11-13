//! Logout revocation and the local second factor of a session, through the root boundary on a
//! real PostgreSQL database.
//!
//!     NWUI_TEST_DATABASE_URL=postgres://user:pass@host:5432/postgres cargo test --test auth_session_security -- --include-ignored

mod auth_support;

use std::net::SocketAddr;

use auth_support::{TestDb, UserRow, audit_rows, insert_user, test_db};
use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode, header};
use nwui_api::auth::{session, totp};
use serde_json::{Value, json};
use tower::ServiceExt as _;

fn app(db: &TestDb) -> Router {
    nwui_api::app::with_request_context(nwui_api::auth::router().with_state(db.state.clone()), None)
}

fn now() -> i64 {
    nwui_api::auth::principal::now_unix_ms() / 1000
}

fn cookie(claims: Value) -> String {
    let token = session::encode(claims.as_object().unwrap(), auth_support::SECRET, session::COOKIE_NAME_PLAIN, now(), 3600).unwrap();
    format!("{}={token}", session::COOKIE_NAME_PLAIN)
}

struct Reply {
    status: StatusCode,
    body: Value,
    /// The session cookie this response set, if any (`name=value`).
    session: Option<String>,
    /// The two-step sign-in's pending cookie, when one was stored.
    pending: Option<String>,
}

async fn send(app: &Router, method: &str, path: &str, cookie: &str, form: Option<&str>) -> Reply {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "admin.example.test")
        .header("cookie", cookie)
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));
    if method == "POST" {
        builder = builder.header("origin", "http://admin.example.test").header("content-type", "application/x-www-form-urlencoded");
    }
    let response = app.clone().oneshot(builder.body(Body::from(form.unwrap_or("").to_owned())).unwrap()).await.unwrap();
    let status = response.status();
    let set_cookies: Vec<String> = response.headers().get_all(header::SET_COOKIE).iter().map(|v| v.to_str().unwrap().to_owned()).collect();
    let stored = |name: &str| {
        set_cookies
            .iter()
            .find(|v| v.starts_with(&format!("{name}=")) && !v.contains("Max-Age=0") && !v.contains("Expires=Thu, 01 Jan 1970"))
            .map(|v| v.split(';').next().unwrap().to_owned())
    };
    let session = stored(session::COOKIE_NAME_PLAIN);
    let pending = stored(nwui_api::auth::pending_login::COOKIE_NAME_PLAIN);
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let body = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
    Reply { status, body, session, pending }
}

fn claims_of(cookie: &str) -> serde_json::Map<String, Value> {
    let token = cookie.split_once('=').unwrap().1;
    session::decode(token, &[auth_support::SECRET], session::COOKIE_NAME_PLAIN, now()).unwrap()
}

async fn sv(db: &TestDb, id: &str) -> i32 {
    db.client.query_one("SELECT session_version FROM users WHERE id = $1", &[&id]).await.unwrap().get(0)
}

/// Enrolls `id` with a fresh secret and two backup codes; returns (secret bytes, backup codes).
async fn enroll(db: &TestDb, id: &str) -> (Vec<u8>, Vec<String>) {
    let secret = totp::generate_secret();
    let codes = totp::generate_backup_codes(2);
    let hashes = totp::hash_backup_codes(&codes).unwrap();
    let sealed = db.state.secrets.seal(&secret).unwrap();
    db.client
        .execute("UPDATE users SET totp_enabled = true, totp_secret = $2, backup_codes = $3 WHERE id = $1", &[&id, &sealed, &json!(hashes)])
        .await
        .unwrap();
    (totp::decode_secret(&secret).unwrap(), codes)
}

fn current_code(secret: &[u8]) -> String {
    totp::code_at(secret, nwui_api::auth::principal::now_unix_ms() as u64)
}

// ─────────────────────────────── logout ───────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn signing_out_ends_every_session_of_the_user() {
    let db = test_db().await;
    let app = app(&db);
    insert_user(&db.client, UserRow::default(), None).await;
    let claims = json!({"id": "u1", "sub": "u1", "sv": 3, "requires2fa": false});
    let (this_device, other_device) = (cookie(claims.clone()), cookie(claims));

    let out = send(&app, "POST", "/api/ui/v1/forms/session.signOut", &this_device, None).await;
    assert_eq!((out.status, &out.body), (StatusCode::OK, &json!({"redirectTo": "/login"})));
    assert_eq!(sv(&db, "u1").await, 4);
    assert_eq!(audit_rows(&db.client, "logout").await.len(), 1);
    for cookie in [&this_device, &other_device] {
        assert_eq!(send(&app, "GET", "/api/ui/v1/session", cookie, None).await.status, StatusCode::UNAUTHORIZED);
    }
    // A stale cookie revokes nothing more: its version is already gone.
    let again = send(&app, "POST", "/api/ui/v1/forms/session.signOut", &this_device, None).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(sv(&db, "u1").await, 4);
    assert_eq!(audit_rows(&db.client, "logout").await.len(), 1);
    // No cookie at all: just the redirect.
    assert_eq!(send(&app, "POST", "/api/ui/v1/forms/session.signOut", "", None).await.body, json!({"redirectTo": "/login"}));
}

// ───────────────────────────── local 2FA ─────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn an_unverified_session_of_an_enrolled_user_gets_nothing_but_verification() {
    let db = test_db().await;
    let app = app(&db);
    insert_user(&db.client, UserRow::default(), None).await;
    let (secret, backup) = enroll(&db, "u1").await;
    // Required by policy, enrolled, no local proof (an OAuth sign-in, or a cookie from before).
    let pending = cookie(json!({"id": "u1", "sub": "u1", "sv": 3, "requires2fa": true, "totpEnabled": true}));

    let denied = send(&app, "GET", "/api/ui/v1/account", &pending, None).await;
    assert_eq!((denied.status, denied.body["error"]["code"].as_str()), (StatusCode::FORBIDDEN, Some("two_factor_required")));
    let view = send(&app, "GET", "/api/ui/v1/session", &pending, None).await;
    assert_eq!((view.status, &view.body["user"]["verificationRequired"]), (StatusCode::OK, &json!(true)));
    let two_factor = send(&app, "GET", "/api/ui/v1/account/2fa", &pending, None).await;
    assert_eq!(two_factor.body, json!({"enabled": true, "pending": null, "verificationRequired": true}));
    // Finishing a setup is not a proof; disabling is not open to an unverified session.
    let finished = send(&app, "POST", "/api/ui/v1/forms/account.finishTwoFactorSetup", &pending, None).await;
    if let Some(reissued) = &finished.session {
        assert_ne!(claims_of(reissued).get("mfaVerified"), Some(&json!(true)));
        assert_eq!(send(&app, "GET", "/api/ui/v1/account", reissued, None).await.status, StatusCode::FORBIDDEN);
    }
    let disabled = send(&app, "POST", "/api/ui/v1/forms/account.disableTwoFactor", &pending, Some("currentPassword=x")).await;
    assert_eq!(disabled.status, StatusCode::FORBIDDEN);
    assert!(db.client.query_one("SELECT totp_enabled FROM users WHERE id = 'u1'", &[]).await.unwrap().get::<_, bool>(0));

    let verify = "/api/ui/v1/forms/account.verifyTwoFactor";
    let wrong = send(&app, "POST", verify, &pending, Some("code=000000")).await;
    assert_eq!((wrong.status, wrong.body["error"]["fieldErrors"]["code"].as_str()), (StatusCode::BAD_REQUEST, Some("코드가 올바르지 않습니다. 인증 앱의 6자리 코드나 백업 코드를 입력하세요.")));
    assert!(wrong.session.is_none());
    let code = current_code(&secret);
    let ok = send(&app, "POST", verify, &pending, Some(&format!("code={code}"))).await;
    assert_eq!((ok.status, &ok.body), (StatusCode::OK, &json!({"redirectTo": "/"})));
    let verified = ok.session.expect("a verified cookie");
    assert_eq!(claims_of(&verified)["mfaVerified"], json!(true));
    assert_eq!(send(&app, "GET", "/api/ui/v1/account", &verified, None).await.status, StatusCode::OK);
    assert_eq!(send(&app, "GET", "/api/ui/v1/session", &verified, None).await.body["user"]["verificationRequired"], json!(false));
    assert_eq!(audit_rows(&db.client, "user:2fa-verify").await.len(), 1);
    // The proof survives renewal.
    assert_eq!(claims_of(&verified)["requires2fa"], json!(true));

    // RFC 6238 §5.2: the same code does not verify a second session.
    let replay = send(&app, "POST", verify, &pending, Some(&format!("code={code}"))).await;
    assert_eq!(replay.status, StatusCode::BAD_REQUEST);
    // A backup code works once.
    let first = send(&app, "POST", verify, &pending, Some(&format!("code={}", backup[0]))).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(send(&app, "POST", verify, &pending, Some(&format!("code={}", backup[0]))).await.status, StatusCode::BAD_REQUEST);
    // A verified session may now disable (password re-check still applies: this user has none).
    let disabled = send(&app, "POST", "/api/ui/v1/forms/account.disableTwoFactor", &verified, Some("currentPassword=x")).await;
    assert_ne!(disabled.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn verification_is_throttled_and_only_asked_when_the_policy_requires_it() {
    let db = test_db().await;
    let app = app(&db);
    insert_user(&db.client, UserRow::default(), None).await;
    let (secret, _) = enroll(&db, "u1").await;
    // Not required by policy: an enrolled user without local proof is not challenged.
    let relaxed = cookie(json!({"id": "u1", "sub": "u1", "sv": 3, "requires2fa": false}));
    assert_eq!(send(&app, "GET", "/api/ui/v1/account", &relaxed, None).await.status, StatusCode::OK);
    assert_eq!(send(&app, "GET", "/api/ui/v1/session", &relaxed, None).await.body["user"]["verificationRequired"], json!(false));

    let pending = cookie(json!({"id": "u1", "sub": "u1", "sv": 3, "requires2fa": true}));
    let verify = "/api/ui/v1/forms/account.verifyTwoFactor";
    for _ in 0..10 {
        assert_eq!(send(&app, "POST", verify, &pending, Some("code=000000")).await.status, StatusCode::BAD_REQUEST);
    }
    let locked = send(&app, "POST", verify, &pending, Some(&format!("code={}", current_code(&secret)))).await;
    assert_eq!((locked.status, locked.body["error"]["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited")));
    assert!(locked.session.is_none());
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn enrolling_and_signing_in_with_a_code_carry_the_proof() {
    let db = test_db().await;
    let app = app(&db);
    insert_user(&db.client, UserRow::default(), Some(&nwui_api::auth::password::hash_password("Passw0rd-x").unwrap())).await;
    db.client.execute("INSERT INTO app_settings (id, require_2fa_user) VALUES ('current', true)", &[]).await.unwrap();

    // Enrollment: the confirming code is the proof for this session.
    let enrolling = cookie(json!({"id": "u1", "sub": "u1", "sv": 3, "requires2fa": true}));
    assert_eq!(send(&app, "GET", "/api/ui/v1/account", &enrolling, None).await.status, StatusCode::FORBIDDEN);
    let pending = send(&app, "GET", "/api/ui/v1/account/2fa", &enrolling, None).await;
    assert_eq!(pending.body["verificationRequired"], json!(false));
    let secret = pending.body["pending"]["secret"].as_str().unwrap().to_owned();
    let code = current_code(&totp::decode_secret(&secret).unwrap());
    let enabled = send(&app, "POST", "/api/ui/v1/forms/account.enableTwoFactor", &enrolling, Some(&format!("secret={secret}&code={code}"))).await;
    assert_eq!(enabled.status, StatusCode::OK);
    assert_eq!(enabled.body["result"]["backupCodes"].as_array().map(Vec::len), Some(8));
    let proved = enabled.session.expect("enable re-issues the cookie with the proof");
    assert_eq!((&claims_of(&proved)["mfaVerified"], &claims_of(&proved)["totpEnabled"]), (&json!(true), &json!(true)));
    assert_eq!(send(&app, "GET", "/api/ui/v1/account", &proved, None).await.status, StatusCode::OK);

    // A password sign-in proves the code on its own step, and that session carries the proof.
    let login = send(&app, "POST", "/api/ui/v1/forms/login.loginAction", "", Some("email=u1%40example.test&password=Passw0rd-x")).await;
    assert_eq!(login.body, json!({"redirectTo": "/login/2fa"}));
    assert!(login.session.is_none(), "no session before the code");
    let challenge = login.pending.expect("a pending cookie");
    let next = totp::code_at(&totp::decode_secret(&secret).unwrap(), (nwui_api::auth::principal::now_unix_ms() + 30_000) as u64);
    let verified = send(&app, "POST", "/api/ui/v1/forms/login.verifyTwoFactor", &challenge, Some(&format!("totp={next}"))).await;
    assert_eq!(verified.body, json!({"redirectTo": "/"}));
    let signed_in = verified.session.expect("a session cookie");
    assert_eq!(claims_of(&signed_in)["mfaVerified"], json!(true));
    assert_eq!(send(&app, "GET", "/api/ui/v1/account", &signed_in, None).await.status, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_concurrent_burst_of_wrong_codes_reaches_the_check_at_most_ten_times() {
    // Counted on admission: a burst cannot pass the lock check before the first failure lands.
    let db = test_db().await;
    let app = app(&db);
    insert_user(&db.client, UserRow::default(), None).await;
    enroll(&db, "u1").await;
    let pending = cookie(json!({"id": "u1", "sub": "u1", "sv": 3, "requires2fa": true}));
    let mut burst = tokio::task::JoinSet::new();
    for _ in 0..40 {
        let (app, pending) = (app.clone(), pending.clone());
        burst.spawn(async move {
            send(&app, "POST", "/api/ui/v1/forms/account.verifyTwoFactor", &pending, Some("code=000000")).await.status
        });
    }
    let mut checked = 0;
    while let Some(status) = burst.join_next().await {
        match status.unwrap() {
            StatusCode::BAD_REQUEST => checked += 1,
            StatusCode::TOO_MANY_REQUESTS => {}
            other => panic!("unexpected status {other}"),
        }
    }
    assert_eq!(checked, 10, "only admitted attempts reach the code check");
}

#[test]
fn q3_w8_reencoding_preserves_the_original_authentication_time() {
    let claims=json!({"sub":"user","iat":100,"authTime":100});
    let encoded=session::encode(claims.as_object().unwrap(),auth_support::SECRET,session::COOKIE_NAME_PLAIN,200,3600).unwrap();
    let decoded=session::decode(&encoded,&[auth_support::SECRET],session::COOKIE_NAME_PLAIN,200).unwrap();
    assert_eq!(decoded["authTime"],100);
    let encoded=session::encode(&decoded,auth_support::SECRET,session::COOKIE_NAME_PLAIN,300,3600).unwrap();
    let decoded=session::decode(&encoded,&[auth_support::SECRET],session::COOKIE_NAME_PLAIN,300).unwrap();
    assert_eq!(decoded["authTime"],100);
    assert_eq!(decoded["iat"],300);
}

#[test]
fn q3_w8_account_cookie_reissues_cannot_bypass_the_absolute_age() {
    let now=1_900_000_000;
    let claims=json!({"sub":"user","authTime":now-30*86400});
    let encoded=session::encode(claims.as_object().unwrap(),auth_support::SECRET,session::COOKIE_NAME_PLAIN,now,3600).unwrap();
    assert!(session::decode(&encoded,&[auth_support::SECRET],session::COOKIE_NAME_PLAIN,now).is_err());
}
