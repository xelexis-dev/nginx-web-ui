//! `account.regenerateBackupCodes` through the root boundary on a real PostgreSQL database.
//! Ignored by default:
//!
//!     NWUI_TEST_DATABASE_URL=postgres://user:pass@host:5432/postgres cargo test --test auth_backup_regenerate -- --include-ignored
//!
//! New backup codes replace every old one, once, for a session that proved the second factor and
//! re-authenticated just now: with the current password, or — an OAuth-only account has none —
//! with a current authenticator code. What these tests hold:
//! - no session, a revoked or disabled one, or one that never proved the factor (an OIDC sign-in
//!   not yet verified, a session from before 2FA was enabled elsewhere) regenerates nothing;
//! - a missing or wrong proof changes nothing, and wrong proofs count like every re-authentication;
//! - success invalidates every old code at once, returns the eight new ones in the response only,
//!   leaves the authenticator untouched and writes an audit row without any code;
//! - eligibility and the codes are checked again under the row lock: a change that lands while the
//!   request waits for the row is never overwritten, and its codes are never returned.

mod auth_support;

use std::net::SocketAddr;
use std::time::Duration;

use auth_support::{TestDb, UserRow, audit_rows, insert_user, test_db};
use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use nwui_api::auth::{password, principal, session, totp};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const ORIGIN: &str = "http://admin.example.test";
const PASSWORD: &str = "Passw0rd-regenerate";
const REGENERATE: &str = "/api/ui/v1/forms/account.regenerateBackupCodes";
const OLD: [&str; 2] = ["a1b2c-3d4e5", "fffff-00000"];
const CHANGED: &str = "인증 정보가 방금 바뀌었습니다. 다시 시도하세요.";
const NOT_ENROLLED: &str = "2단계 인증이 설정되어 있지 않습니다.";
const CODE_MISSING: &str = "인증 앱의 6자리 코드를 입력하세요.";
const CODE_WRONG: &str = "코드가 올바르지 않습니다. 인증 앱의 6자리 코드를 다시 입력하세요.";

fn app(db: &TestDb) -> Router {
    nwui_api::app::with_request_context(nwui_api::auth::router().with_state(db.state.clone()), None)
}

/// A session cookie for `id` at session version 3 with these extra claims.
fn session_cookie(id: &str, extra: Value) -> String {
    let now = principal::now_unix_ms() / 1000;
    let mut claims = json!({"sub": id, "id": id, "sv": 3, "totpEnabled": true, "requires2fa": false});
    claims.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    let token = session::encode(claims.as_object().unwrap(), auth_support::SECRET, session::COOKIE_NAME_PLAIN, now, 3600).unwrap();
    format!("{}={token}", session::COOKIE_NAME_PLAIN)
}

/// A password session that proved the factor (a two-step sign-in).
fn verified(id: &str) -> String {
    session_cookie(id, json!({"mfaVerified": true}))
}

struct Reply {
    status: StatusCode,
    set_cookies: Vec<String>,
    body: Value,
}

impl Reply {
    fn codes(&self) -> Vec<String> {
        serde_json::from_value(self.body["result"]["backupCodes"].clone()).unwrap_or_default()
    }
    fn field(&self, name: &str) -> &str {
        self.body["error"]["fieldErrors"][name].as_str().unwrap_or("")
    }
    fn error(&self) -> (&str, &str) {
        (self.body["error"]["code"].as_str().unwrap_or(""), self.body["error"]["message"].as_str().unwrap_or(""))
    }
}

async fn call(app: &Router, method: &str, path: &str, cookie: &str, form: Option<&str>) -> Reply {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "admin.example.test")
        .header("x-real-ip", "203.0.113.30")
        .header("cookie", cookie)
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));
    if form.is_some() {
        builder = builder.header("origin", ORIGIN).header("content-type", "application/x-www-form-urlencoded");
    }
    let response = app.clone().oneshot(builder.body(Body::from(form.unwrap_or("").to_owned())).unwrap()).await.unwrap();
    let status = response.status();
    let set_cookies = response.headers().get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_owned()).collect();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    Reply { status, set_cookies, body: if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() } }
}

async fn regenerate(app: &Router, cookie: &str, form: &str) -> Reply {
    call(app, "POST", REGENERATE, cookie, Some(form)).await
}

fn enc(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn password_form() -> String {
    format!("currentPassword={}", enc(PASSWORD))
}

/// An enrolled user — with a password, or OAuth-only — holding the two `OLD` backup codes. Returns
/// the base32 TOTP secret.
async fn enrolled(db: &TestDb, id: &str, email: &str, with_password: bool) -> String {
    let hash = with_password.then(|| password::hash_password(PASSWORD).unwrap());
    insert_user(&db.client, UserRow { id, email, totp_enabled: true, ..Default::default() }, hash.as_deref()).await;
    let secret = totp::generate_secret();
    let hashes = totp::hash_backup_codes(&OLD.map(str::to_owned)).unwrap();
    db.client
        .execute("UPDATE users SET totp_secret = $2, backup_codes = $3 WHERE id = $1", &[&id, &db.state.secrets.seal(&secret).unwrap(), &json!(hashes)])
        .await
        .unwrap();
    secret
}

fn code_now(secret: &str) -> String {
    totp::code_at(&totp::decode_secret(secret).unwrap(), principal::now_unix_ms() as u64)
}

fn wrong_code(secret: &str) -> &'static str {
    let bytes = totp::decode_secret(secret).unwrap();
    let now = principal::now_unix_ms() as u64;
    let window: Vec<String> = [now - 30_000, now, now + 30_000].iter().map(|t| totp::code_at(&bytes, *t)).collect();
    ["000000", "111111", "222222", "333333"].into_iter().find(|c| !window.iter().any(|w| w == c)).unwrap()
}

struct Stored {
    hashes: Vec<String>,
    secret: Option<String>,
    last_step: Option<i64>,
    session_version: i32,
}

async fn stored(db: &TestDb, id: &str) -> Stored {
    let row = db
        .client
        .query_one("SELECT backup_codes, totp_secret, totp_last_step, session_version FROM users WHERE id = $1", &[&id])
        .await
        .unwrap();
    let codes: Option<Value> = row.get(0);
    Stored {
        hashes: codes.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default(),
        secret: row.get(1),
        last_step: row.get(2),
        session_version: row.get(3),
    }
}

fn accepts(hashes: &[String], code: &str) -> bool {
    totp::find_backup_code(code, hashes).is_some()
}

async fn assert_unchanged(db: &TestDb, id: &str, context: &str) {
    let now = stored(db, id).await;
    assert_eq!(now.hashes.len(), OLD.len(), "{context}: the old codes are all still stored");
    assert!(OLD.iter().all(|c| accepts(&now.hashes, c)), "{context}: every old code still works");
    assert!(audit_rows(&db.client, "user:2fa-backup-regenerate").await.is_empty(), "{context}: nothing audited");
}

// ───────────────────────────── who may regenerate ─────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn regeneration_needs_a_session_that_proved_the_second_factor() {
    let db = test_db().await;
    enrolled(&db, "u1", "u1@example.test", true).await;
    let app = app(&db);
    let form = password_form();

    let none = regenerate(&app, "", &form).await;
    assert_eq!(none.status, StatusCode::UNAUTHORIZED, "no session");
    let revoked = regenerate(&app, &session_cookie("u1", json!({"sv": 2, "mfaVerified": true})), &form).await;
    assert_eq!(revoked.status, StatusCode::UNAUTHORIZED, "a revoked session version");
    // Signed in before 2FA was enabled on another device, under a policy that does not require it:
    // the gate lets it through, but it never proved the factor.
    let earlier = regenerate(&app, &session_cookie("u1", json!({})), &form).await;
    assert_eq!((earlier.status, earlier.error().0), (StatusCode::FORBIDDEN, "two_factor_required"), "{:?}", earlier.body);
    let policy = regenerate(&app, &session_cookie("u1", json!({"requires2fa": true})), &form).await;
    assert_eq!((policy.status, policy.error().0), (StatusCode::FORBIDDEN, "two_factor_required"));
    let oidc = regenerate(&app, &session_cookie("u1", json!({"externalLogin": true})), &form).await;
    assert_eq!((oidc.status, oidc.error().0), (StatusCode::FORBIDDEN, "two_factor_required"), "an OIDC sign-in not yet verified");
    assert_unchanged(&db, "u1", "refused sessions").await;

    db.client.execute("UPDATE users SET enabled = false WHERE id = 'u1'", &[]).await.unwrap();
    let disabled = regenerate(&app, &verified("u1"), &form).await;
    assert_eq!(disabled.status, StatusCode::UNAUTHORIZED, "a disabled account");
    assert_unchanged(&db, "u1", "disabled").await;
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn regeneration_needs_an_enrolled_second_factor() {
    let db = test_db().await;
    insert_user(&db.client, UserRow::default(), Some(&password::hash_password(PASSWORD).unwrap())).await;
    let app = app(&db);
    let reply = regenerate(&app, &session_cookie("u1", json!({"totpEnabled": false})), &password_form()).await;
    assert_eq!((reply.status, reply.error().1), (StatusCode::CONFLICT, NOT_ENROLLED), "{:?}", reply.body);
    assert_eq!(stored(&db, "u1").await.hashes.len(), 0);
    assert!(audit_rows(&db.client, "user:2fa-backup-regenerate").await.is_empty());
}

// ───────────────────────────── re-authentication ─────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_missing_or_wrong_password_changes_nothing_and_counts() {
    let db = test_db().await;
    let secret = enrolled(&db, "u1", "u1@example.test", true).await;
    let app = app(&db);
    let session = verified("u1");

    let empty = regenerate(&app, &session, "currentPassword=").await;
    assert_eq!((empty.status, empty.field("currentPassword")), (StatusCode::BAD_REQUEST, "현재 비밀번호를 입력하세요."));
    // An account with a password re-authenticates with it: a code does not stand in.
    let code_only = regenerate(&app, &session, &format!("code={}", code_now(&secret))).await;
    assert_eq!((code_only.status, code_only.field("currentPassword")), (StatusCode::BAD_REQUEST, "현재 비밀번호를 입력하세요."));
    assert_eq!(stored(&db, "u1").await.last_step, None, "the code was not looked at");
    let wrong = regenerate(&app, &session, "currentPassword=wrong-password").await;
    assert_eq!((wrong.status, wrong.field("currentPassword")), (StatusCode::BAD_REQUEST, "현재 비밀번호가 올바르지 않습니다."));
    assert_unchanged(&db, "u1", "wrong password").await;

    // Ten wrong passwords in the window (this one included) lock every re-authentication.
    for _ in 1..10 {
        assert_eq!(regenerate(&app, &session, "currentPassword=wrong-password").await.status, StatusCode::BAD_REQUEST);
    }
    let locked = regenerate(&app, &session, &password_form()).await;
    assert_eq!((locked.status, locked.error().0), (StatusCode::TOO_MANY_REQUESTS, "rate_limited"));
    let change = call(&app, "POST", "/api/ui/v1/forms/account.changePassword", &session, Some(&format!("currentPassword={}&newPassword=NewPassw0rd-9&confirmPassword=NewPassw0rd-9", enc(PASSWORD)))).await;
    assert_eq!(change.status, StatusCode::TOO_MANY_REQUESTS, "the same window as the other re-authentications");
    assert_unchanged(&db, "u1", "locked").await;
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn an_oauth_only_account_proves_a_current_authenticator_code_instead() {
    let db = test_db().await;
    let secret = enrolled(&db, "oa", "oa@example.test", false).await;
    let app = app(&db);
    // An OIDC sign-in that has been verified with this app's factor.
    let session = session_cookie("oa", json!({"externalLogin": true, "mfaVerified": true}));

    let no_code = regenerate(&app, &session, "currentPassword=anything").await;
    assert_eq!((no_code.status, no_code.field("code")), (StatusCode::BAD_REQUEST, CODE_MISSING), "{:?}", no_code.body);
    let wrong = regenerate(&app, &session, &format!("code={}", wrong_code(&secret))).await;
    assert_eq!((wrong.status, wrong.field("code")), (StatusCode::BAD_REQUEST, CODE_WRONG));
    // Only the authenticator: a backup code is not looked at, so it is not spent either.
    let backup = regenerate(&app, &session, &format!("code={}", OLD[0])).await;
    assert_eq!((backup.status, backup.field("code")), (StatusCode::BAD_REQUEST, CODE_WRONG));
    assert_unchanged(&db, "oa", "wrong or backup code").await;

    let code = code_now(&secret);
    let reply = regenerate(&app, &session, &format!("code={code}")).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    let after = stored(&db, "oa").await;
    assert!(after.last_step.is_some(), "the authenticator code is used like any sign-in code");
    assert!(!OLD.iter().any(|c| accepts(&after.hashes, c)));
    assert!(reply.codes().iter().all(|c| accepts(&after.hashes, c)));
    let replay = regenerate(&app, &session, &format!("code={code}")).await;
    assert_eq!((replay.status, replay.field("code")), (StatusCode::BAD_REQUEST, CODE_WRONG), "the same code twice");
    assert_eq!(stored(&db, "oa").await.hashes, after.hashes, "a replay changes nothing");
    assert_eq!(audit_rows(&db.client, "user:2fa-backup-regenerate").await.len(), 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn wrong_codes_are_throttled_like_account_verification() {
    let db = test_db().await;
    let secret = enrolled(&db, "oa", "oa@example.test", false).await;
    let app = app(&db);
    let session = session_cookie("oa", json!({"externalLogin": true, "mfaVerified": true}));
    for _ in 0..nwui_api::auth::throttle::MAX_FAILURES {
        assert_eq!(regenerate(&app, &session, &format!("code={}", wrong_code(&secret))).await.status, StatusCode::BAD_REQUEST);
    }
    let locked = regenerate(&app, &session, &format!("code={}", code_now(&secret))).await;
    assert_eq!((locked.status, locked.error().0), (StatusCode::TOO_MANY_REQUESTS, "rate_limited"));
    assert_eq!(stored(&db, "oa").await.last_step, None, "a locked request does not use the code");
    assert_unchanged(&db, "oa", "locked").await;
}

// ───────────────────────────── the new codes ─────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn regeneration_replaces_every_backup_code_and_leaves_the_authenticator() {
    let db = test_db().await;
    let secret = enrolled(&db, "u1", "u1@example.test", true).await;
    let app = app(&db);
    let session = verified("u1");
    let before = stored(&db, "u1").await;

    let reply = regenerate(&app, &session, &password_form()).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["result"]["ok"], json!(true), "TwoFactorState.ok");
    let codes = reply.codes();
    assert_eq!(codes.len(), totp::BACKUP_CODE_COUNT);
    assert!(codes.iter().all(|c| c.len() == 11 && totp::looks_like_backup_code(c)), "{codes:?}");
    let mut distinct = codes.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), codes.len());
    assert!(reply.set_cookies.is_empty(), "the session is not re-issued: {:?}", reply.set_cookies);

    let after = stored(&db, "u1").await;
    assert_eq!(after.hashes.len(), codes.len());
    assert!(!OLD.iter().any(|c| accepts(&after.hashes, c)), "every old code stopped working");
    assert!(codes.iter().all(|c| accepts(&after.hashes, c)), "every new code works");
    assert_eq!((after.secret, after.last_step, after.session_version), (before.secret, before.last_step, before.session_version), "the authenticator and the sessions are untouched");

    let audits = audit_rows(&db.client, "user:2fa-backup-regenerate").await;
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].get::<_, Option<String>>("actor_id").as_deref(), Some("u1"));
    let written = format!(
        "{:?} {:?} {:?} {:?}",
        audits[0].get::<_, Option<String>>("target"),
        audits[0].get::<_, Option<Value>>("before"),
        audits[0].get::<_, Option<Value>>("after"),
        audits[0].get::<_, Option<String>>("actor_label"),
    );
    let bare = |c: &str| c.replace('-', "");
    assert!(!codes.iter().chain(OLD.iter().map(|c| *c).map(str::to_owned).collect::<Vec<_>>().iter()).any(|c| written.contains(c.as_str()) || written.contains(&bare(c))), "no code in the audit row: {written}");
    assert!(!after.hashes.iter().any(|h| written.contains(h.as_str())), "no hash in the audit row");
    assert_eq!(call(&app, "GET", "/api/ui/v1/account", &session, None).await.status, StatusCode::OK, "this session goes on");

    // At sign-in: an old code is refused, a new one completes the step, and so does the authenticator.
    let old = sign_in_with(&app, "backup", OLD[0]).await;
    assert_eq!(old, "/login/2fa/backup?error=invalid");
    assert_eq!(sign_in_with(&app, "backup", &codes[3]).await, "/");
    assert_eq!(sign_in_with(&app, "totp", &code_now(&secret)).await, "/");
}

/// A full two-step sign-in of u1 with this factor; the factor step's redirect.
async fn sign_in_with(app: &Router, method: &str, code: &str) -> String {
    let login = call(app, "POST", "/api/ui/v1/forms/login.loginAction", "", Some(&format!("email=u1%40example.test&password={}", enc(PASSWORD)))).await;
    assert_eq!(login.body["redirectTo"], "/login/2fa", "{:?}", login.body);
    let pending = login.set_cookies.iter().find(|c| c.starts_with("nwui-2fa-challenge=")).expect("pending cookie").split(';').next().unwrap().to_owned();
    let reply = call(app, "POST", "/api/ui/v1/forms/login.verifyTwoFactor", &pending, Some(&format!("method={method}&totp={}", enc(code)))).await;
    reply.body["redirectTo"].as_str().unwrap_or("").to_owned()
}

// ───────────────────────────── under the row lock ─────────────────────────────

/// Runs a regeneration while this test holds u1's row, applies `change` in the holding
/// transaction once the request is waiting for the row, then releases it.
async fn regenerate_while_the_row_changes(db: &TestDb, app: &Router, change: &str) -> Reply {
    db.client.batch_execute("BEGIN; SELECT 1 FROM users WHERE id = 'u1' FOR UPDATE").await.unwrap();
    let request = {
        let app = app.clone();
        let session = verified("u1");
        let form = password_form();
        tokio::spawn(async move { regenerate(&app, &session, &form).await })
    };
    let waiting = async {
        loop {
            let row = db
                .client
                .query_one(
                    "SELECT pg_stat_clear_snapshot()::text, count(*) FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock'",
                    &[],
                )
                .await
                .unwrap();
            if row.get::<_, i64>(1) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let reached = tokio::time::timeout(Duration::from_secs(20), waiting).await;
    db.client.batch_execute(&format!("{change}; COMMIT")).await.unwrap();
    reached.expect("the regeneration never waited for the row: it does not take the row lock");
    request.await.unwrap()
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn factors_that_change_while_waiting_for_the_row_are_not_overwritten() {
    // What the re-authentication was checked against, and the codes it would replace, must still be
    // there under the row lock. Otherwise nothing is written and no codes are returned: the user
    // would save a set that was never stored, or one proved against a factor that no longer exists.
    let other_codes = json!(totp::hash_backup_codes(&["00000-11111".to_owned()]).unwrap()).to_string();
    let other_password = password::hash_password("Other-passw0rd-1").unwrap();
    let cases = [
        ("a sign-in spent a backup code", "backup_codes"),
        ("2FA was turned off and on again", "totp_secret"),
        ("the password was reset without signing out", "password_hash"),
    ];
    for (what, column) in cases {
        let db = test_db().await;
        enrolled(&db, "u1", "u1@example.test", true).await;
        let app = app(&db);
        let change = match column {
            "backup_codes" => format!("UPDATE users SET backup_codes = '{}'::jsonb WHERE id = 'u1'", json!([stored(&db, "u1").await.hashes[1]])),
            "totp_secret" => {
                let secret = db.state.secrets.seal(&totp::generate_secret()).unwrap();
                format!("UPDATE users SET totp_secret = '{secret}', backup_codes = '{other_codes}'::jsonb WHERE id = 'u1'")
            }
            _ => format!("UPDATE users SET password_hash = '{other_password}' WHERE id = 'u1'"),
        };
        let expected: Option<String> = {
            let before = format!("SELECT {column}::text FROM users WHERE id = 'u1'");
            db.client.query_one(&before, &[]).await.unwrap().get(0)
        };
        let reply = regenerate_while_the_row_changes(&db, &app, &change).await;
        assert_eq!((reply.status, reply.error().1), (StatusCode::CONFLICT, CHANGED), "{what}: {:?}", reply.body);
        assert!(reply.codes().is_empty(), "{what}");
        let now: Option<String> = db.client.query_one(&format!("SELECT {column}::text FROM users WHERE id = 'u1'"), &[]).await.unwrap().get(0);
        assert_ne!(now, expected, "{what}: the concurrent change stands");
        assert!(audit_rows(&db.client, "user:2fa-backup-regenerate").await.is_empty(), "{what}");
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn eligibility_that_ends_while_waiting_for_the_row_regenerates_nothing() {
    let cases = [
        ("UPDATE users SET session_version = session_version + 1 WHERE id = 'u1'", StatusCode::UNAUTHORIZED, "signed out everywhere"),
        ("UPDATE users SET enabled = false WHERE id = 'u1'", StatusCode::UNAUTHORIZED, "disabled"),
        ("UPDATE users SET totp_enabled = false WHERE id = 'u1'", StatusCode::CONFLICT, "2FA turned off"),
    ];
    for (change, status, what) in cases {
        let db = test_db().await;
        enrolled(&db, "u1", "u1@example.test", true).await;
        let app = app(&db);
        let reply = regenerate_while_the_row_changes(&db, &app, change).await;
        assert_eq!(reply.status, status, "{what}: {:?}", reply.body);
        assert!(reply.codes().is_empty(), "{what}");
        assert_unchanged(&db, "u1", what).await;
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn regeneration_does_not_starve_a_one_connection_pool() {
    // No connection may be held while re-authentication, hashing or the transaction borrow another.
    let db = auth_support::test_db_with(&[("API_DB_POOL_SIZE", "1")]).await;
    enrolled(&db, "u1", "u1@example.test", true).await;
    let app = app(&db);
    let reply = tokio::time::timeout(Duration::from_secs(30), regenerate(&app, &verified("u1"), &password_form()))
        .await
        .expect("the regeneration waited for a connection it was holding itself");
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn regeneration_requires_the_origin_like_every_form() {
    let db = test_db().await;
    enrolled(&db, "u1", "u1@example.test", true).await;
    let app = app(&db);
    let request = Request::builder()
        .method("POST")
        .uri(REGENERATE)
        .header("host", "admin.example.test")
        .header("cookie", verified("u1"))
        .header("content-type", "application/x-www-form-urlencoded")
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))))
        .body(Body::from(password_form()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_unchanged(&db, "u1", "no Origin").await;
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn the_codes_are_not_replaced_when_the_audit_row_cannot_be_written() {
    // The audit row belongs to the same transaction: without it, nothing changes and no codes are
    // returned.
    let db = test_db().await;
    enrolled(&db, "u1", "u1@example.test", true).await;
    db.client
        .batch_execute(
            "CREATE FUNCTION refuse_regenerate_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'audit refused'; END $$;
             CREATE TRIGGER refuse_regenerate_audit BEFORE INSERT ON audit_logs FOR EACH ROW
             WHEN (NEW.action = 'user:2fa-backup-regenerate') EXECUTE FUNCTION refuse_regenerate_audit();",
        )
        .await
        .unwrap();
    let app = app(&db);
    let reply = regenerate(&app, &verified("u1"), &password_form()).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE, "{:?}", reply.body);
    assert!(reply.codes().is_empty());
    assert_unchanged(&db, "u1", "audit refused").await;
}
