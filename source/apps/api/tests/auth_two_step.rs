//! Two-step credentials sign-in through `auth::router()` and the root request-context/Origin
//! boundary, on a real PostgreSQL database. Ignored by default:
//!
//!     NWUI_TEST_DATABASE_URL=postgres://user:pass@host:5432/postgres cargo test --test auth_two_step -- --include-ignored
//!
//! A password that matches an account with its own TOTP gets no session: only a short-lived pending
//! cookie that `/login/2fa` completes with `login.verifyTwoFactor`. What these tests hold:
//! - the pending cookie is never a session, and no `login` audit or throttle reset happens before
//!   the second factor;
//! - the second factor is checked against the account as it is now (session version, enabled,
//!   approved, the enrolled factor itself), once, even when submitted concurrently;
//! - failed codes count against the account, across as many password restarts as an attacker
//!   likes, and a challenge dies after its own attempt budget;
//! - the authenticator screen and the backup-code screen each take only their own kind of code,
//!   never spend the other kind, and share one attempt budget.

mod auth_support;

use std::net::SocketAddr;

use auth_support::{SECRET, TestDb, UserRow, audit_rows, ctx, insert_user, test_db};
use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use nwui_api::auth::principal::now_unix_ms;
use nwui_api::auth::pending_login::{self, Pending};
use nwui_api::auth::{password, session, totp};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const ORIGIN: &str = "http://admin.example.test";
const PASSWORD: &str = "Passw0rd-compat";
const LOGIN: &str = "/api/ui/v1/forms/login.loginAction";
const VERIFY: &str = "/api/ui/v1/forms/login.verifyTwoFactor";
const CANCEL: &str = "/api/ui/v1/forms/login.cancelTwoFactor";
const CHALLENGE: &str = "nwui-2fa-challenge";
const EXPIRED: &str = "/login?error=TwoFactorExpired";
const INVALID: &str = "/login/2fa?error=invalid";
const BACKUP_INVALID: &str = "/login/2fa/backup?error=invalid";

fn app(db: &TestDb) -> Router {
    nwui_api::app::with_request_context(nwui_api::auth::router().with_state(db.state.clone()), None)
}

/// A request as nginx forwards it: loopback peer, Host, X-Real-IP.
fn request(method: &str, path: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "admin.example.test")
        .header("x-real-ip", "203.0.113.20")
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))))
}

struct Reply {
    status: StatusCode,
    set_cookies: Vec<String>,
    body: Value,
}

impl Reply {
    fn redirect(&self) -> &str {
        self.body["redirectTo"].as_str().unwrap_or("")
    }
    /// `name=value` of the Set-Cookie that stores `name` (not an expiry), if any.
    fn stored(&self, name: &str) -> Option<String> {
        self.set_cookies
            .iter()
            .find(|c| c.starts_with(&format!("{name}=")) && !c.contains("Max-Age=0"))
            .map(|c| c.split(';').next().unwrap().to_owned())
    }
    fn expires(&self, name: &str) -> bool {
        self.set_cookies.iter().any(|c| c.starts_with(&format!("{name}=")) && c.contains("Max-Age=0"))
    }
    fn sets_session(&self) -> bool {
        self.stored(session::COOKIE_NAME_PLAIN).is_some() || self.stored(session::COOKIE_NAME_SECURE).is_some()
    }
}

async fn send(app: &Router, request: Request<Body>) -> Reply {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let set_cookies = response.headers().get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_owned()).collect();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let body = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
    Reply { status, set_cookies, body }
}

async fn post(app: &Router, path: &str, body: &str, cookie: Option<&str>) -> Reply {
    let mut builder = request("POST", path).header("origin", ORIGIN).header("content-type", "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    send(app, builder.body(Body::from(body.to_owned())).unwrap()).await
}

async fn get(app: &Router, path: &str, cookie: Option<&str>) -> Reply {
    let mut builder = request("GET", path);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    send(app, builder.body(Body::empty()).unwrap()).await
}

fn enc(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn login_body(email: &str, password: &str, totp: &str) -> String {
    format!("email={}&password={}&totp={}", enc(email), enc(password), enc(totp))
}

fn code_body(code: &str) -> String {
    format!("totp={}", enc(code))
}

/// What a factor screen posts: `/login/2fa` sends `totp`, `/login/2fa/backup` sends `backup`.
fn method_body(method: &str, code: &str) -> String {
    format!("method={}&totp={}", enc(method), enc(code))
}

/// An editor with a password and an enrolled TOTP (sealed, as enrollment stores it). Returns the
/// base32 secret, to compute codes with.
async fn totp_user(db: &TestDb, id: &str, email: &str) -> String {
    insert_user(&db.client, UserRow { id, email, totp_enabled: true, ..Default::default() }, Some(&password::hash_password(PASSWORD).unwrap())).await;
    let secret = totp::generate_secret();
    db.client
        .execute("UPDATE users SET totp_secret = $2 WHERE id = $1", &[&id, &db.state.secrets.seal(&secret).unwrap()])
        .await
        .unwrap();
    secret
}

fn code_now(secret: &str) -> String {
    totp::code_at(&totp::decode_secret(secret).unwrap(), now_unix_ms() as u64)
}

fn wrong_code(secret: &str) -> &'static str {
    // Different from any code in the ±1 step window around now.
    let bytes = totp::decode_secret(secret).unwrap();
    let now = now_unix_ms() as u64;
    let window: Vec<String> = [now - 30_000, now, now + 30_000].iter().map(|t| totp::code_at(&bytes, *t)).collect();
    ["000000", "111111", "222222", "333333"].into_iter().find(|c| !window.iter().any(|w| w == c)).unwrap()
}

async fn set_backup_codes(db: &TestDb, id: &str, codes: &[&str]) {
    let hashes = totp::hash_backup_codes(&codes.iter().map(|c| (*c).to_owned()).collect::<Vec<_>>()).unwrap();
    db.client.execute("UPDATE users SET backup_codes = $2 WHERE id = $1", &[&id, &json!(hashes)]).await.unwrap();
}

async fn backup_codes_left(db: &TestDb, id: &str) -> usize {
    let left: Value = db.client.query_one("SELECT backup_codes FROM users WHERE id = $1", &[&id]).await.unwrap().get(0);
    left.as_array().map_or(0, Vec::len)
}

/// The last authenticator time step used; `None` until a TOTP code has been accepted.
async fn totp_step(db: &TestDb, id: &str) -> Option<i64> {
    db.client.query_one("SELECT totp_last_step FROM users WHERE id = $1", &[&id]).await.unwrap().get(0)
}

/// The password step of a TOTP account: returns the pending cookie (`name=value`).
async fn password_step(app: &Router, email: &str) -> String {
    let reply = post(app, LOGIN, &login_body(email, PASSWORD, ""), None).await;
    assert_eq!(reply.redirect(), "/login/2fa", "{:?}", reply.body);
    reply.stored(CHALLENGE).expect("a pending cookie")
}

async fn pending(app: &Router, cookie: Option<&str>) -> bool {
    let reply = get(app, "/api/ui/v1/auth/options", cookie).await;
    assert_eq!(reply.status, StatusCode::OK);
    reply.body["twoFactorPending"].as_bool().unwrap_or(false)
}

// ───────────────────────────── the password step ─────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_password_for_a_totp_account_sets_only_a_pending_cookie() {
    let db = test_db().await;
    totp_user(&db, "u1", "u1@example.test").await;
    let app = app(&db);

    let reply = post(&app, LOGIN, &login_body("u1@example.test", PASSWORD, ""), None).await;
    assert_eq!((reply.status, reply.redirect()), (StatusCode::OK, "/login/2fa"));
    assert!(!reply.sets_session(), "no session before the second factor: {:?}", reply.set_cookies);
    let set = reply.set_cookies.iter().find(|c| c.starts_with(&format!("{CHALLENGE}="))).expect("pending cookie");
    for attribute in ["Path=/", "HttpOnly", "SameSite=Lax", "Max-Age=300"] {
        assert!(set.contains(attribute), "{attribute} missing: {set}");
    }
    assert!(!set.contains("Secure"), "plain http: {set}");
    let cookie = reply.stored(CHALLENGE).unwrap();
    assert!(cookie.len() > CHALLENGE.len() + 40, "an opaque random value: {cookie}");

    assert!(audit_rows(&db.client, "login").await.is_empty(), "the login is audited only after the second factor");
    let session = get(&app, "/api/ui/v1/session", Some(&cookie)).await;
    assert_eq!(session.status, StatusCode::UNAUTHORIZED, "the pending cookie is not a session");
    assert!(pending(&app, Some(&cookie)).await);
    assert!(!pending(&app, None).await);
    assert!(!pending(&app, Some(&format!("{CHALLENGE}=not-a-challenge"))).await);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn the_password_step_expires_a_session_this_browser_already_holds() {
    // Otherwise SSR would still see the old session and skip the factor screen.
    let db = test_db().await;
    totp_user(&db, "u1", "u1@example.test").await;
    insert_user(&db.client, UserRow { id: "u2", email: "u2@example.test", ..Default::default() }, None).await;
    let app = app(&db);
    let claims = json!({"sub": "u2", "id": "u2", "role": "editor", "sv": 3, "totpEnabled": false, "requires2fa": false});
    let old = session::encode(claims.as_object().unwrap(), SECRET, session::COOKIE_NAME_PLAIN, now_unix_ms() / 1000, session::SESSION_MAX_AGE_SECS).unwrap();
    let cookie = format!("{0}={old}; {0}.0=stale; {0}.1=stale", session::COOKIE_NAME_PLAIN);
    let reply = post(&app, LOGIN, &login_body("u1@example.test", PASSWORD, ""), Some(&cookie)).await;
    assert_eq!(reply.redirect(), "/login/2fa");
    assert!(!reply.sets_session());
    for name in [session::COOKIE_NAME_PLAIN.to_owned(), format!("{}.0", session::COOKIE_NAME_PLAIN), format!("{}.1", session::COOKIE_NAME_PLAIN)] {
        assert!(reply.expires(&name), "{name} must be expired: {:?}", reply.set_cookies);
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn duplicate_pending_cookies_fail_closed() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    let app = app(&db);
    let challenge = password_step(&app, "u1@example.test").await;
    for cookie in [format!("{challenge}; {CHALLENGE}=other"), format!("{CHALLENGE}=other; {challenge}"), format!("{challenge}; {challenge}")] {
        assert!(!pending(&app, Some(&cookie)).await, "{cookie}");
        let reply = post(&app, VERIFY, &code_body(&code_now(&secret)), Some(&cookie)).await;
        assert_eq!(reply.redirect(), EXPIRED, "{cookie}");
        assert!(!reply.sets_session(), "{cookie}");
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_code_sent_with_the_password_does_not_skip_the_second_step() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    let app = app(&db);
    let reply = post(&app, LOGIN, &login_body("u1@example.test", PASSWORD, &code_now(&secret)), None).await;
    assert_eq!(reply.redirect(), "/login/2fa");
    assert!(!reply.sets_session());
    let step: Option<i64> = db.client.query_one("SELECT totp_last_step FROM users WHERE id = 'u1'", &[]).await.unwrap().get(0);
    assert_eq!(step, None, "the code was not even consumed");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn an_account_without_totp_signs_in_in_one_step_as_before() {
    let db = test_db().await;
    insert_user(&db.client, UserRow::default(), Some(&password::hash_password(PASSWORD).unwrap())).await;
    let app = app(&db);
    let reply = post(&app, LOGIN, &login_body("u1@example.test", PASSWORD, ""), None).await;
    assert_eq!(reply.redirect(), "/");
    assert!(reply.sets_session());
    assert!(reply.stored(CHALLENGE).is_none());
    assert_eq!(audit_rows(&db.client, "login").await.len(), 1);
}

// ───────────────────────────── the factor step ─────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_valid_code_issues_the_session_once_and_retires_the_challenge() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    set_backup_codes(&db, "u1", &["a1b2c-3d4e5"]).await;
    let app = app(&db);
    let challenge = password_step(&app, "u1@example.test").await;

    let reply = post(&app, VERIFY, &code_body(&format!(" {} ", code_now(&secret))), Some(&challenge)).await;
    assert_eq!((reply.status, reply.redirect()), (StatusCode::OK, "/"));
    assert!(reply.expires(CHALLENGE), "the pending cookie is cleared: {:?}", reply.set_cookies);
    let session = reply.stored(session::COOKIE_NAME_PLAIN).expect("session cookie");
    let view = get(&app, "/api/ui/v1/session", Some(&session)).await;
    assert_eq!(view.status, StatusCode::OK);
    assert_eq!(view.body["user"]["totpEnabled"], json!(true));
    assert_eq!(view.body["user"]["verificationRequired"], json!(false), "this session proved the factor");
    let logins = audit_rows(&db.client, "login").await;
    assert_eq!(logins.len(), 1);
    assert_eq!(logins[0].get::<_, Option<String>>("actor_id").as_deref(), Some("u1"));

    // The same challenge never completes again, not even with another valid factor.
    let again = post(&app, VERIFY, &code_body("a1b2c-3d4e5"), Some(&challenge)).await;
    assert_eq!(again.redirect(), EXPIRED);
    assert!(!again.sets_session());
    assert!(!pending(&app, Some(&challenge)).await);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_backup_code_completes_the_step_and_is_spent() {
    let db = test_db().await;
    totp_user(&db, "u1", "u1@example.test").await;
    set_backup_codes(&db, "u1", &["a1b2c-3d4e5", "fffff-00000"]).await;
    let app = app(&db);
    let challenge = password_step(&app, "u1@example.test").await;
    let reply = post(&app, VERIFY, &code_body("A1B2C3D4E5"), Some(&challenge)).await;
    assert_eq!(reply.redirect(), "/");
    assert!(reply.sets_session());
    let left: Value = db.client.query_one("SELECT backup_codes FROM users WHERE id = 'u1'", &[]).await.unwrap().get(0);
    assert_eq!(left.as_array().unwrap().len(), 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_wrong_code_stays_on_the_factor_screen_until_the_challenge_budget_is_spent() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    let app = app(&db);
    let challenge = password_step(&app, "u1@example.test").await;
    let wrong = wrong_code(&secret);
    for n in 1..5 {
        let reply = post(&app, VERIFY, &code_body(wrong), Some(&challenge)).await;
        assert_eq!(reply.redirect(), INVALID, "attempt {n}");
        assert!(!reply.sets_session());
        assert!(pending(&app, Some(&challenge)).await, "still pending after {n}");
    }
    let fifth = post(&app, VERIFY, &code_body(wrong), Some(&challenge)).await;
    assert_eq!(fifth.redirect(), EXPIRED, "the fifth wrong code ends the challenge");
    assert!(fifth.expires(CHALLENGE));
    let late = post(&app, VERIFY, &code_body(&code_now(&secret)), Some(&challenge)).await;
    assert_eq!(late.redirect(), EXPIRED, "a right code on a spent challenge does nothing");
    assert!(!late.sets_session());
    assert!(audit_rows(&db.client, "login").await.is_empty());
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn restarting_with_the_password_does_not_reset_the_account_limit() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    let app = app(&db);
    let wrong = wrong_code(&secret);
    // Two challenges, five wrong codes each: ten failures on the account, its limit.
    for _ in 0..2 {
        let challenge = password_step(&app, "u1@example.test").await;
        for _ in 0..5 {
            post(&app, VERIFY, &code_body(wrong), Some(&challenge)).await;
        }
    }
    // The right password is now refused like any failure, and no challenge is started.
    let reply = post(&app, LOGIN, &login_body("u1@example.test", PASSWORD, ""), None).await;
    assert_eq!(reply.redirect(), "/login?error=CredentialsSignin");
    assert!(reply.stored(CHALLENGE).is_none());
    assert!(!reply.sets_session());
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_new_password_step_replaces_the_previous_challenge() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    let app = app(&db);
    let first = password_step(&app, "u1@example.test").await;
    let second = password_step(&app, "u1@example.test").await;
    assert_ne!(first, second);
    assert!(!pending(&app, Some(&first)).await, "one challenge per account");
    assert_eq!(post(&app, VERIFY, &code_body(&code_now(&secret)), Some(&first)).await.redirect(), EXPIRED);
    assert_eq!(post(&app, VERIFY, &code_body(&code_now(&secret)), Some(&second)).await.redirect(), "/");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn concurrent_distinct_backup_codes_make_one_session() {
    let db = test_db().await;
    totp_user(&db, "u1", "u1@example.test").await;
    set_backup_codes(&db, "u1", &["a1b2c-3d4e5", "fffff-00000"]).await;
    let app = app(&db);
    let challenge = password_step(&app, "u1@example.test").await;
    let (first, second) = (code_body("a1b2c-3d4e5"), code_body("fffff-00000"));
    let (a, b) = tokio::join!(post(&app, VERIFY, &first, Some(&challenge)), post(&app, VERIFY, &second, Some(&challenge)));
    let sessions = [&a, &b].iter().filter(|r| r.sets_session()).count();
    assert_eq!(sessions, 1, "one challenge, one session: {:?} / {:?}", a.body, b.body);
    let loser = if a.sets_session() { &b } else { &a };
    assert_eq!(loser.redirect(), EXPIRED);
    assert_eq!(audit_rows(&db.client, "login").await.len(), 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn the_account_is_checked_again_before_the_session() {
    // Each change between the two steps must end the pending sign-in.
    let changes: [(&str, &str); 5] = [
        ("session version moved on (sign-out elsewhere, password change)", "UPDATE users SET session_version = session_version + 1 WHERE id = 'u1'"),
        ("account disabled", "UPDATE users SET enabled = false WHERE id = 'u1'"),
        ("approval withdrawn", "UPDATE users SET approved = false WHERE id = 'u1'"),
        ("2FA turned off", "UPDATE users SET totp_enabled = false WHERE id = 'u1'"),
        ("factor replaced", "UPDATE users SET totp_secret = 'JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP' WHERE id = 'u1'"),
    ];
    for (what, sql) in changes {
        let db = test_db().await;
        let secret = totp_user(&db, "u1", "u1@example.test").await;
        let app = app(&db);
        let challenge = password_step(&app, "u1@example.test").await;
        db.client.batch_execute(sql).await.unwrap();
        assert!(!pending(&app, Some(&challenge)).await, "{what}: options must not claim a pending sign-in");
        let reply = post(&app, VERIFY, &code_body(&code_now(&secret)), Some(&challenge)).await;
        assert_eq!(reply.redirect(), EXPIRED, "{what}");
        assert!(!reply.sets_session(), "{what}");
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn cancel_ends_the_pending_sign_in() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    let app = app(&db);
    let challenge = password_step(&app, "u1@example.test").await;
    let reply = post(&app, CANCEL, "", Some(&challenge)).await;
    assert_eq!(reply.redirect(), "/login");
    assert!(reply.expires(CHALLENGE));
    assert!(!pending(&app, Some(&challenge)).await);
    assert_eq!(post(&app, VERIFY, &code_body(&code_now(&secret)), Some(&challenge)).await.redirect(), EXPIRED);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn no_or_unknown_pending_cookie_validates_nothing() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    let app = app(&db);
    for cookie in [None, Some(format!("{CHALLENGE}=forged")), Some(format!("{CHALLENGE}="))] {
        let reply = post(&app, VERIFY, &code_body(&code_now(&secret)), cookie.as_deref()).await;
        assert_eq!(reply.redirect(), EXPIRED, "{cookie:?}");
        assert!(!reply.sets_session(), "{cookie:?}");
    }
    // A session cookie is not a pending cookie either.
    let reply = post(&app, VERIFY, &code_body(&code_now(&secret)), Some("authjs.session-token=x")).await;
    assert_eq!(reply.redirect(), EXPIRED);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn the_factor_commands_require_the_origin_like_every_form() {
    let db = test_db().await;
    let app = app(&db);
    for path in [VERIFY, CANCEL] {
        let request = request("POST", path).header("content-type", "application/x-www-form-urlencoded").body(Body::from("totp=123456")).unwrap();
        assert_eq!(send(&app, request).await.status, StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn https_uses_the_host_prefixed_secure_pending_cookie() {
    let db = test_db().await;
    totp_user(&db, "u1", "u1@example.test").await;
    let app = app(&db);
    let request = request("POST", LOGIN)
        .header("origin", "https://admin.example.test")
        .header("x-forwarded-proto", "https")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(login_body("u1@example.test", PASSWORD, "")))
        .unwrap();
    let reply = send(&app, request).await;
    assert_eq!(reply.redirect(), "/login/2fa", "{:?}", reply.body);
    let set = reply.set_cookies.iter().find(|c| c.starts_with("__Host-nwui-2fa-challenge=")).expect("__Host- cookie");
    assert!(set.contains("; Secure") && set.contains("Path=/") && !set.contains("Domain="), "{set}");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn finalisation_refuses_a_pending_sign_in_that_expired_while_waiting() {
    // The expiry is read after the row lock, just before the audit: a slow database cannot carry an
    // expired pending sign-in into a session. Deterministic clock.
    let db = test_db().await;
    totp_user(&db, "u1", "u1@example.test").await;
    let stored: Option<String> = db.client.query_one("SELECT totp_secret FROM users WHERE id = 'u1'", &[]).await.unwrap().get(0);
    let pending = Pending {
        user_id: "u1".into(),
        throttle_key: nwui_api::auth::throttle::email_key("u1@example.test"),
        session_version: 3,
        factor: pending_login::factor_digest(stored.as_deref()),
        expires_ms: 1_000_000,
        attempts: 0,
        finished: true,
    };
    let at_expiry = pending_login::complete_login(&db.state, &pending, &ctx(), &|| 1_000_000).await.unwrap();
    assert!(at_expiry.is_none(), "expired at the instant of the check");
    assert!(audit_rows(&db.client, "login").await.is_empty(), "nothing written for a refused sign-in");
    let user = pending_login::complete_login(&db.state, &pending, &ctx(), &|| 999_999).await.unwrap().expect("still inside its lifetime");
    assert!(user.mfa_verified && user.totp_enabled);
    assert_eq!((user.id.as_str(), user.session_version), ("u1", 3));
    assert_eq!(audit_rows(&db.client, "login").await.len(), 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_lock_at_the_factor_step_ends_the_sign_in_without_blaming_the_password() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    let app = app(&db);
    let challenge = password_step(&app, "u1@example.test").await;
    let throttle = &db.state.runtime.login_throttle;
    for _ in 0..nwui_api::auth::throttle::MAX_FAILURES {
        throttle.record_account_failure(&nwui_api::auth::throttle::login_key(&nwui_api::auth::throttle::email_key("u1@example.test"), Some("203.0.113.20")), now_unix_ms());
    }
    let reply = post(&app, VERIFY, &code_body(&code_now(&secret)), Some(&challenge)).await;
    assert_eq!(reply.redirect(), EXPIRED, "not /login?error=CredentialsSignin: the password was right");
    assert!(!reply.sets_session());
    assert!(!pending(&app, Some(&challenge)).await);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn any_new_password_submission_ends_the_browsers_pending_sign_in() {
    let db = test_db().await;
    let secret = totp_user(&db, "a", "a@example.test").await;
    insert_user(&db.client, UserRow { id: "b", email: "b@example.test", ..Default::default() }, Some(&password::hash_password(PASSWORD).unwrap())).await;
    let app = app(&db);

    // A is pending; B (no TOTP) signs in in this browser: A's sign-in is over.
    let a = password_step(&app, "a@example.test").await;
    let b = post(&app, LOGIN, &login_body("b@example.test", PASSWORD, ""), Some(&a)).await;
    assert_eq!(b.redirect(), "/");
    assert!(b.sets_session());
    assert!(b.expires(CHALLENGE), "the pending cookie is expired: {:?}", b.set_cookies);
    assert!(!pending(&app, Some(&a)).await, "a copy of A's cookie is dead too");
    assert_eq!(post(&app, VERIFY, &code_body(&code_now(&secret)), Some(&a)).await.redirect(), EXPIRED);

    // A failed password submission ends it as well.
    let a = password_step(&app, "a@example.test").await;
    let wrong = post(&app, LOGIN, &login_body("a@example.test", "wrong-password", ""), Some(&a)).await;
    assert_eq!(wrong.redirect(), "/login?error=CredentialsSignin");
    assert!(wrong.expires(CHALLENGE));
    assert!(!pending(&app, Some(&a)).await);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_pending_sign_in_for_another_account_replaces_the_old_one_and_its_cookie_wins() {
    let db = test_db().await;
    totp_user(&db, "a", "a@example.test").await;
    let c_secret = totp_user(&db, "c", "c@example.test").await;
    let app = app(&db);
    let a = password_step(&app, "a@example.test").await;
    let reply = post(&app, LOGIN, &login_body("c@example.test", PASSWORD, ""), Some(&a)).await;
    assert_eq!(reply.redirect(), "/login/2fa");
    let c = reply.stored(CHALLENGE).expect("C's pending cookie");
    assert!(!reply.expires(CHALLENGE), "no expiry line may override the new cookie: {:?}", reply.set_cookies);
    assert!(!pending(&app, Some(&a)).await);
    assert!(pending(&app, Some(&c)).await);
    assert_eq!(post(&app, VERIFY, &code_body(&code_now(&c_secret)), Some(&c)).await.redirect(), "/");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn sign_out_ends_a_pending_sign_in_even_without_a_session() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    let app = app(&db);
    let challenge = password_step(&app, "u1@example.test").await;
    let reply = post(&app, "/api/ui/v1/forms/session.signOut", "", Some(&challenge)).await;
    assert_eq!(reply.redirect(), "/login");
    assert!(reply.expires(CHALLENGE), "{:?}", reply.set_cookies);
    assert!(!pending(&app, Some(&challenge)).await);
    assert_eq!(post(&app, VERIFY, &code_body(&code_now(&secret)), Some(&challenge)).await.redirect(), EXPIRED);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_pending_sign_in_does_not_starve_a_one_connection_pool() {
    // auth/options must not hold one pool connection while the pending check borrows another:
    // with API_DB_POOL_SIZE=1 that request would wait for itself.
    let db = auth_support::test_db_with(&[("API_DB_POOL_SIZE", "1")]).await;
    totp_user(&db, "u1", "u1@example.test").await;
    let app = app(&db);
    let challenge = password_step(&app, "u1@example.test").await;
    let reply = tokio::time::timeout(std::time::Duration::from_secs(10), get(&app, "/api/ui/v1/auth/options", Some(&challenge)))
        .await
        .expect("auth/options waited for a connection it was holding itself");
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["twoFactorPending"], json!(true));
}

// ───────────────────────────── the two factor screens ─────────────────────────────
// `/login/2fa` posts `method=totp` and `/login/2fa/backup` posts `method=backup`. A page from
// before the split posts no method and keeps the old field that took either kind.

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn the_authenticator_screen_takes_exactly_six_digits_and_never_spends_a_backup_code() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    set_backup_codes(&db, "u1", &["a1b2c-3d4e5"]).await;
    let app = app(&db);
    let challenge = password_step(&app, "u1@example.test").await;

    let code = code_now(&secret);
    let spaced = format!("{} {}", &code[..3], &code[3..]);
    for refused in ["a1b2c-3d4e5", "A1B2C3D4E5", spaced.as_str()] {
        let reply = post(&app, VERIFY, &method_body("totp", refused), Some(&challenge)).await;
        assert_eq!(reply.redirect(), INVALID, "{refused}");
        assert!(!reply.sets_session(), "{refused}");
    }
    assert_eq!(backup_codes_left(&db, "u1").await, 1, "the backup code was not spent");
    assert_eq!(totp_step(&db, "u1").await, None);
    assert!(pending(&app, Some(&challenge)).await);

    // Surrounding whitespace is trimmed like every form field; the six digits are the code.
    let reply = post(&app, VERIFY, &method_body("totp", &format!(" {code} ")), Some(&challenge)).await;
    assert_eq!(reply.redirect(), "/");
    assert!(reply.sets_session());
    assert_eq!(backup_codes_left(&db, "u1").await, 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn the_backup_screen_takes_only_a_backup_code_and_stays_there_when_it_is_wrong() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    set_backup_codes(&db, "u1", &["a1b2c-3d4e5", "fffff-00000"]).await;
    let app = app(&db);
    let challenge = password_step(&app, "u1@example.test").await;

    for refused in [code_now(&secret), "00000-00000".to_owned()] {
        let reply = post(&app, VERIFY, &method_body("backup", &refused), Some(&challenge)).await;
        assert_eq!(reply.redirect(), BACKUP_INVALID, "{refused}");
        assert!(!reply.sets_session(), "{refused}");
    }
    assert_eq!(totp_step(&db, "u1").await, None, "the authenticator code was not used");

    // The backup forms accepted before the split still are: any case, with or without the dash.
    let reply = post(&app, VERIFY, &method_body("backup", " A1B2C3D4E5 "), Some(&challenge)).await;
    assert_eq!(reply.redirect(), "/");
    assert!(reply.sets_session());
    assert_eq!(backup_codes_left(&db, "u1").await, 1);
    assert_eq!(totp_step(&db, "u1").await, None);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_code_refused_on_the_other_screen_still_works_on_its_own() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    set_backup_codes(&db, "u1", &["a1b2c-3d4e5"]).await;
    let app = app(&db);
    let code = code_now(&secret);

    let challenge = password_step(&app, "u1@example.test").await;
    assert_eq!(post(&app, VERIFY, &method_body("backup", &code), Some(&challenge)).await.redirect(), BACKUP_INVALID);
    assert_eq!(post(&app, VERIFY, &method_body("totp", "a1b2c-3d4e5"), Some(&challenge)).await.redirect(), INVALID);
    let reply = post(&app, VERIFY, &method_body("totp", &code), Some(&challenge)).await;
    assert_eq!(reply.redirect(), "/", "the authenticator code was not recorded as used on the backup screen");
    assert!(reply.sets_session());

    let challenge = password_step(&app, "u1@example.test").await;
    let reply = post(&app, VERIFY, &method_body("backup", "a1b2c-3d4e5"), Some(&challenge)).await;
    assert_eq!(reply.redirect(), "/", "the backup code was not spent on the authenticator screen");
    assert_eq!(backup_codes_left(&db, "u1").await, 0);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn one_attempt_budget_covers_both_screens() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    set_backup_codes(&db, "u1", &["a1b2c-3d4e5"]).await;
    let app = app(&db);
    let challenge = password_step(&app, "u1@example.test").await;
    let wrong = wrong_code(&secret);

    let switching = [("totp", wrong, INVALID), ("backup", "00000-00000", BACKUP_INVALID), ("totp", wrong, INVALID), ("backup", "00000-00000", BACKUP_INVALID)];
    for (n, (method, code, back)) in switching.into_iter().enumerate() {
        let reply = post(&app, VERIFY, &method_body(method, code), Some(&challenge)).await;
        assert_eq!(reply.redirect(), back, "attempt {}", n + 1);
        assert!(pending(&app, Some(&challenge)).await, "still pending after {}", n + 1);
    }
    let fifth = post(&app, VERIFY, &method_body("backup", "11111-11111"), Some(&challenge)).await;
    assert_eq!(fifth.redirect(), EXPIRED, "the fifth wrong code ends it, whichever screen sent it");
    assert!(fifth.expires(CHALLENGE));
    for (method, code) in [("totp", code_now(&secret)), ("backup", "a1b2c-3d4e5".to_owned())] {
        let late = post(&app, VERIFY, &method_body(method, &code), Some(&challenge)).await;
        assert_eq!(late.redirect(), EXPIRED, "{method}");
        assert!(!late.sets_session());
    }
    assert_eq!(backup_codes_left(&db, "u1").await, 1);
    assert_eq!(totp_step(&db, "u1").await, None);
    assert!(audit_rows(&db.client, "login").await.is_empty());
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn an_unrecognised_method_is_a_counted_failure_whose_code_is_never_looked_at() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    set_backup_codes(&db, "u1", &["a1b2c-3d4e5"]).await;
    let app = app(&db);

    // Empty, another value or case, or the field twice in either order. Each round sends a right
    // code — a TOTP code, then a backup code — and five of them spend a challenge; the two rounds
    // are the account's ten failures.
    for code in [code_now(&secret), "a1b2c-3d4e5".to_owned()] {
        let challenge = password_step(&app, "u1@example.test").await;
        let code = enc(&code);
        let bodies = [
            format!("method=&totp={code}"),
            format!("method=TOTP&totp={code}"),
            format!("method=sms&totp={code}"),
            format!("method=totp&method=backup&totp={code}"),
            format!("method=backup&method=totp&totp={code}"),
        ];
        for (n, body) in bodies.iter().enumerate() {
            let reply = post(&app, VERIFY, body, Some(&challenge)).await;
            assert!(!reply.sets_session(), "{body}");
            assert_eq!(reply.redirect(), if n < 4 { INVALID } else { EXPIRED }, "{body}");
        }
    }
    assert_eq!(totp_step(&db, "u1").await, None, "no code was evaluated");
    assert_eq!(backup_codes_left(&db, "u1").await, 1);
    let reply = post(&app, LOGIN, &login_body("u1@example.test", PASSWORD, ""), None).await;
    assert_eq!(reply.redirect(), "/login?error=CredentialsSignin", "the failures were kept on the account");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_page_from_before_the_split_still_takes_either_code_without_a_method() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    set_backup_codes(&db, "u1", &["a1b2c-3d4e5"]).await;
    let app = app(&db);

    let challenge = password_step(&app, "u1@example.test").await;
    assert_eq!(post(&app, VERIFY, &code_body("00000-00000"), Some(&challenge)).await.redirect(), INVALID, "a wrong code of either form");
    assert_eq!(post(&app, VERIFY, &code_body("a1b2c-3d4e5"), Some(&challenge)).await.redirect(), "/");
    let challenge = password_step(&app, "u1@example.test").await;
    assert_eq!(post(&app, VERIFY, &code_body(&code_now(&secret)), Some(&challenge)).await.redirect(), "/");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_lock_on_the_backup_screen_ends_the_sign_in_like_on_the_other() {
    let db = test_db().await;
    totp_user(&db, "u1", "u1@example.test").await;
    set_backup_codes(&db, "u1", &["a1b2c-3d4e5"]).await;
    let app = app(&db);
    let challenge = password_step(&app, "u1@example.test").await;
    for _ in 0..nwui_api::auth::throttle::MAX_FAILURES {
        db.state.runtime.login_throttle.record_account_failure(&nwui_api::auth::throttle::login_key(&nwui_api::auth::throttle::email_key("u1@example.test"), Some("203.0.113.20")), now_unix_ms());
    }
    let reply = post(&app, VERIFY, &method_body("backup", "a1b2c-3d4e5"), Some(&challenge)).await;
    assert_eq!(reply.redirect(), EXPIRED, "not the backup screen's invalid");
    assert!(!reply.sets_session());
    assert_eq!(backup_codes_left(&db, "u1").await, 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_s3_pending_factor_lock_is_audited_in_the_email_namespace() {
    let db = test_db().await;
    totp_user(&db, "u1", "u1@example.test").await;
    let app = app(&db);
    for _ in 0..nwui_api::auth::throttle::MAX_FAILURES {
        let challenge = password_step(&app, "u1@example.test").await;
        let reply = post(&app, VERIFY, &code_body("not-a-code"), Some(&challenge)).await;
        assert!(!reply.sets_session());
    }
    let rows = audit_rows(&db.client, "auth:login-locked").await;
    assert_eq!(rows.len(), 1, "factor failures must report the same credential counter as password failures");
    assert_eq!(rows[0].get::<_, Option<String>>("target").as_deref(), Some("u1@example.test"));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn failed_second_factors_do_not_lock_other_client_addresses() {
    let db = test_db().await;
    let secret = totp_user(&db, "u1", "u1@example.test").await;
    let app = app(&db);
    async fn from(app: &Router, ip: &str, path: &str, body: String, cookie: Option<&str>) -> Reply {
        let mut builder = request("POST", path).header("origin", ORIGIN).header("content-type", "application/x-www-form-urlencoded");
        builder.headers_mut().unwrap().insert("x-real-ip", ip.parse().unwrap());
        if let Some(cookie) = cookie { builder = builder.header("cookie", cookie); }
        send(app, builder.body(Body::from(body)).unwrap()).await
    }
    let attacker = "203.0.113.51";
    let mut failures = 0;
    while failures < nwui_api::auth::throttle::MAX_FAILURES {
        // Address changes between password and factor: the factor request owns its counter.
        let start = from(&app, "203.0.113.50", LOGIN, login_body("u1@example.test", PASSWORD, ""), None).await;
        assert_eq!(start.redirect(), "/login/2fa");
        let cookie = start.stored(CHALLENGE).unwrap();
        for _ in 0..pending_login::MAX_ATTEMPTS {
            let reply = from(&app, attacker, VERIFY, code_body(wrong_code(&secret)), Some(&cookie)).await;
            assert!(!reply.sets_session());
            failures += 1;
            if failures == nwui_api::auth::throttle::MAX_FAILURES { break; }
        }
    }
    let start = from(&app, "203.0.113.52", LOGIN, login_body("u1@example.test", PASSWORD, ""), None).await;
    assert_eq!(start.redirect(), "/login/2fa", "second-factor failures at another address must not block this password step");
    let cookie = start.stored(CHALLENGE).unwrap();
    let complete = from(&app, "203.0.113.52", VERIFY, code_body(&code_now(&secret)), Some(&cookie)).await;
    assert_eq!(complete.redirect(), "/");
    assert!(complete.sets_session());
    let locked = from(&app, attacker, LOGIN, login_body("u1@example.test", PASSWORD, ""), None).await;
    assert_eq!(locked.redirect(), "/login?error=CredentialsSignin", "success elsewhere must not forgive this address");
    let notices = audit_rows(&db.client, "auth:login-locked").await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].get::<_, Option<String>>("ip").as_deref(), Some(attacker));
    assert_eq!(notices[0].get::<_, Option<String>>("target").as_deref(), Some("u1@example.test"));
}
