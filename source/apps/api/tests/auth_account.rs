//! `/api/ui/v1/account` + 2FA enrollment + own-password change, through the root boundary on a
//! real PostgreSQL database. Ignored by default:
//!
//!     NWUI_TEST_DATABASE_URL=postgres://user:pass@host:5432/postgres cargo test --test auth_account -- --include-ignored

mod auth_support;

use std::net::SocketAddr;

use auth_support::{TestDb, UserRow, audit_rows, insert_user, test_db};
use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use nwui_api::auth::{password, principal, session, totp};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const ORIGIN: &str = "http://admin.example.test";
const PASSWORD: &str = "Passw0rd-account";

fn app(db: &TestDb) -> Router {
    nwui_api::app::with_request_context(nwui_api::auth::router().with_state(db.state.clone()), None)
}

fn cookie(id: &str, sv: i64, requires_2fa: bool) -> String {
    let now = principal::now_unix_ms() / 1000;
    let claims = json!({"sub": id, "id": id, "sv": sv, "totpEnabled": false, "requires2fa": requires_2fa});
    let token = session::encode(claims.as_object().unwrap(), auth_support::SECRET, session::COOKIE_NAME_PLAIN, now, 3600).unwrap();
    format!("authjs.session-token={token}")
}

struct Reply {
    status: StatusCode,
    set_cookies: Vec<String>,
    body: Value,
}

async fn call(app: &Router, method: &str, path: &str, cookie: &str, form: Option<&str>) -> Reply {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "admin.example.test")
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

async fn me(db: &TestDb) -> (bool, Option<String>, Option<Value>, Option<i64>, i32) {
    let row = db.client.query_one("SELECT totp_enabled, totp_secret, backup_codes, totp_last_step, session_version FROM users WHERE id = 'u1'", &[]).await.unwrap();
    (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4))
}

fn claims_of(set_cookie: &str) -> serde_json::Map<String, Value> {
    let token = set_cookie.split(';').next().unwrap().strip_prefix("authjs.session-token=").unwrap();
    session::decode(token, &[auth_support::SECRET], session::COOKIE_NAME_PLAIN, principal::now_unix_ms() / 1000).unwrap()
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn account_view_lists_enabled_providers_with_my_links() {
    let db = test_db().await;
    let app = app(&db);
    insert_user(&db.client, UserRow::default(), Some("hash")).await;
    db.client.batch_execute("
        INSERT INTO oauth_providers (id, name, issuer, client_id, client_secret_enc, sort_order) VALUES ('b', 'Beta', 'https://b.test', 'c', 'x', 1), ('a', 'Alpha', 'https://a.test', 'c', 'x', 1);
        INSERT INTO oauth_providers (id, name, issuer, client_id, client_secret_enc, enabled) VALUES ('off', 'Off', 'https://o.test', 'c', 'x', false);
        INSERT INTO oauth_accounts (provider_id, subject, user_id, linked_at) VALUES ('b', 's', 'u1', '2026-03-01T00:00:00Z');
    ").await.unwrap();
    let reply = call(&app, "GET", "/api/ui/v1/account", &cookie("u1", 3, false), None).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, json!({
        "email": "u1@example.test", "hasPassword": true, "totpEnabled": false,
        "logins": [{"providerId": "a", "name": "Alpha", "linkedAt": null}, {"providerId": "b", "name": "Beta", "linkedAt": "2026-03-01T00:00:00.000Z"}],
    }));
    // Required-but-unenrolled users are confined to enrollment.
    let reply = call(&app, "GET", "/api/ui/v1/account", &cookie("u1", 3, true), None).await;
    assert_eq!((reply.status, reply.body["error"]["code"].as_str()), (StatusCode::FORBIDDEN, Some("two_factor_required")));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn two_factor_enrollment_round_trip() {
    let db = test_db().await;
    let app = app(&db);
    insert_user(&db.client, UserRow::default(), Some(&password::hash_password(PASSWORD).unwrap())).await;
    // A user the policy confines to enrollment can still reach every enrollment step.
    let session = cookie("u1", 3, true);

    let reply = call(&app, "GET", "/api/ui/v1/account/2fa", &session, None).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body["enabled"], false);
    let secret = reply.body["pending"]["secret"].as_str().unwrap().to_owned();
    assert_eq!(reply.body["pending"]["otpauthUri"], totp::otpauth_uri(&secret, "u1@example.test"));

    let reply = call(&app, "POST", "/api/ui/v1/forms/account.enableTwoFactor", &session, Some(&format!("secret={secret}&code=000000"))).await;
    assert_eq!((reply.status, reply.body["error"]["message"].as_str()), (StatusCode::CONFLICT, Some("코드가 올바르지 않습니다. 인증 앱의 6자리 코드를 다시 입력하세요.")));
    let reply = call(&app, "POST", "/api/ui/v1/forms/account.enableTwoFactor", &session, Some("secret=&code=123456")).await;
    assert_eq!(reply.body["error"]["message"], "설정 세션이 만료되었습니다. 페이지를 새로고침해 다시 시도하세요.");
    // A self-chosen 80-bit secret is refused (RFC 4226 R6) even with a matching code.
    let weak = "JBSWY3DPEHPK3PXP";
    let weak_code = totp::code_at(&totp::decode_secret(weak).unwrap(), principal::now_unix_ms() as u64);
    let reply = call(&app, "POST", "/api/ui/v1/forms/account.enableTwoFactor", &session, Some(&format!("secret={weak}&code={weak_code}"))).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);

    let now = principal::now_unix_ms();
    let code = totp::code_at(&totp::decode_secret(&secret).unwrap(), now as u64);
    let reply = call(&app, "POST", "/api/ui/v1/forms/account.enableTwoFactor", &session, Some(&format!("secret={secret}&code={code}"))).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["result"]["ok"], true, "TwoFactorState.ok");
    let codes: Vec<String> = serde_json::from_value(reply.body["result"]["backupCodes"].clone()).unwrap();
    assert_eq!(codes.len(), 8);
    let (enabled, stored_secret, backup, last_step, _) = me(&db).await;
    assert!(enabled);
    assert_eq!(db.state.secrets.open(stored_secret.as_deref()).as_deref(), Some(secret.as_str()), "sealed at rest");
    assert_eq!(backup.unwrap().as_array().unwrap().len(), 8);
    assert!(last_step.is_some(), "the enrollment code's step is consumed: it cannot sign in again");
    assert_eq!(audit_rows(&db.client, "user:2fa-enable").await.len(), 1);
    // The accepted code is this session's proof: enable re-issues the cookie with it.
    let enabled_cookie = &reply.set_cookies[0];
    assert_eq!((&claims_of(enabled_cookie)["totpEnabled"], &claims_of(enabled_cookie)["mfaVerified"]), (&json!(true), &json!(true)));
    let proved = enabled_cookie.split(';').next().unwrap().to_owned();
    assert_eq!(call(&app, "GET", "/api/ui/v1/account", &session, None).await.status, StatusCode::UNAUTHORIZED);

    // Acknowledging the codes refreshes the cookie's claim (TS unstable_update), keeping the proof.
    let reply = call(&app, "POST", "/api/ui/v1/forms/account.finishTwoFactorSetup", &proved, Some("")).await;
    assert_eq!(reply.body, json!({"redirectTo": "/account"}));
    assert_eq!((&claims_of(&reply.set_cookies[0])["totpEnabled"], &claims_of(&reply.set_cookies[0])["mfaVerified"]), (&json!(true), &json!(true)));
    let reply = call(&app, "GET", "/api/ui/v1/account/2fa", &proved, None).await;
    assert_eq!(reply.body, json!({"enabled": true, "pending": null, "verificationRequired": false}));
    assert_eq!(call(&app, "GET", "/api/ui/v1/account", &proved, None).await.status, StatusCode::OK);

    // Disabling needs a verified session, then the password.
    assert_eq!(call(&app, "POST", "/api/ui/v1/forms/account.disableTwoFactor", &session, Some(&format!("currentPassword={PASSWORD}"))).await.status, StatusCode::UNAUTHORIZED);
    let reply = call(&app, "POST", "/api/ui/v1/forms/account.disableTwoFactor", &proved, Some("currentPassword=wrong")).await;
    assert_eq!(reply.body["error"]["fieldErrors"]["currentPassword"], "현재 비밀번호가 올바르지 않습니다.");
    let reply = call(&app, "POST", "/api/ui/v1/forms/account.disableTwoFactor", &proved, Some(&format!("currentPassword={PASSWORD}"))).await;
    assert_eq!(reply.body, json!({"redirectTo": "/account/2fa"}));
    assert_eq!((&claims_of(&reply.set_cookies[0])["totpEnabled"], &claims_of(&reply.set_cookies[0])["mfaVerified"]), (&json!(false), &json!(false)));
    let (enabled_after, secret_after, backup_after, step_after, _) = me(&db).await;
    assert!(!enabled_after);
    assert_eq!((secret_after, backup_after, step_after), (None, None, None), "re-enrollment starts clean");
    assert_eq!(audit_rows(&db.client, "user:2fa-disable").await.len(), 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn changing_the_own_password_ends_every_session() {
    let db = test_db().await;
    let app = app(&db);
    insert_user(&db.client, UserRow::default(), Some(&password::hash_password(PASSWORD).unwrap())).await;
    insert_user(&db.client, UserRow { id: "oa", email: "oa@example.test", ..Default::default() }, None).await;
    let session = cookie("u1", 3, false);
    let path = "/api/ui/v1/forms/account.changePassword";

    let reply = call(&app, "POST", path, &session, Some("currentPassword=&newPassword=short&confirmPassword=x")).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.body["error"]["fieldErrors"], json!({
        "currentPassword": "현재 비밀번호를 입력하세요.", "newPassword": "비밀번호는 최소 8자 이상이어야 합니다.", "confirmPassword": "새 비밀번호가 일치하지 않습니다.",
    }));
    let reply = call(&app, "POST", path, &session, Some("currentPassword=wrong-pass1&newPassword=New-passw0rd&confirmPassword=New-passw0rd")).await;
    assert_eq!(reply.body["error"]["fieldErrors"]["currentPassword"], "현재 비밀번호가 올바르지 않습니다.");
    let reply = call(&app, "POST", path, &cookie("oa", 3, false), Some("currentPassword=x&newPassword=New-passw0rd&confirmPassword=New-passw0rd")).await;
    assert_eq!(reply.body["error"]["message"], "이 계정은 비밀번호를 사용하지 않습니다.");

    let reply = call(&app, "POST", path, &session, Some(&format!("currentPassword={PASSWORD}&newPassword=New-passw0rd&confirmPassword=New-passw0rd"))).await;
    assert_eq!((reply.status, reply.body.clone()), (StatusCode::OK, json!({"redirectTo": "/login"})));
    assert!(reply.set_cookies.iter().all(|c| c.contains("Max-Age=0")) && !reply.set_cookies.is_empty(), "this device is signed out");
    assert_eq!(me(&db).await.4, 4, "every session ends");
    let hash: String = db.client.query_one("SELECT password_hash FROM users WHERE id = 'u1'", &[]).await.unwrap().get(0);
    assert!(password::verify_password("New-passw0rd", &hash));
    assert_eq!(audit_rows(&db.client, "user:change-own-password").await.len(), 1);
    // The old cookie no longer works.
    assert_eq!(call(&app, "GET", "/api/ui/v1/account", &session, None).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn current_password_checks_stop_guessing_at_the_cap() {
    // A stolen session must not guess the password through re-authentication without limit
    // (15-minute window, 10 per account, shared by both forms that ask for it).
    let db = test_db().await;
    let app = app(&db);
    insert_user(&db.client, UserRow { totp_enabled: true, ..UserRow::default() }, Some(&password::hash_password(PASSWORD).unwrap())).await;
    let secret = db.state.secrets.seal(&totp::generate_secret()).unwrap();
    db.client.execute("UPDATE users SET totp_secret=$1 WHERE id='u1'", &[&secret]).await.unwrap();
    let mut claims = claims_of(&cookie("u1", 3, false));
    claims.insert("mfaVerified".into(), json!(true));
    let token = session::encode(&claims, auth_support::SECRET, session::COOKIE_NAME_PLAIN, principal::now_unix_ms() / 1000, 3600).unwrap();
    let session = format!("{}={token}", session::COOKIE_NAME_PLAIN);
    let change = "/api/ui/v1/forms/account.changePassword";
    let wrong = "currentPassword=wrong-1&newPassword=NewPassw0rd-9&confirmPassword=NewPassw0rd-9";
    for _ in 0..10 {
        assert_eq!(call(&app, "POST", change, &session, Some(wrong)).await.status, StatusCode::BAD_REQUEST);
    }
    let right = format!("currentPassword={PASSWORD}&newPassword=NewPassw0rd-9&confirmPassword=NewPassw0rd-9");
    let locked = call(&app, "POST", change, &session, Some(&right)).await;
    assert_eq!((locked.status, locked.body["error"]["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited")));
    let disable = call(&app, "POST", "/api/ui/v1/forms/account.disableTwoFactor", &session, Some(&format!("currentPassword={PASSWORD}"))).await;
    assert_eq!(disable.status, StatusCode::TOO_MANY_REQUESTS, "the same window guards 2FA removal");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_s2_policy_off_stale_session_cannot_disable_an_enrolled_factor() {
    let db = test_db().await;
    let hash = password::hash_password(PASSWORD).unwrap();
    insert_user(&db.client, UserRow { totp_enabled: true, ..UserRow::default() }, Some(&hash)).await;
    let sealed = db.state.secrets.seal(&totp::generate_secret()).unwrap();
    db.client.execute("UPDATE users SET totp_secret=$1 WHERE id='u1'", &[&sealed]).await.unwrap();
    db.client.batch_execute("UPDATE app_settings SET require_2fa_admin=false, require_2fa_user=false").await.unwrap();
    let reply = call(&app(&db), "POST", "/api/ui/v1/forms/account.disableTwoFactor", &cookie("u1", 3, false), Some(&format!("currentPassword={PASSWORD}"))).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "password alone must not remove the enrolled factor: {}", reply.body);
    assert_eq!(reply.body["error"]["code"], "two_factor_required");
    assert!(me(&db).await.0);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_s3_login_email_cannot_lock_factor_or_reauthentication_keys() {
    let db = test_db().await;
    for key in ["user:u1", "reauth:user:u1"] {
        for _ in 0..nwui_api::auth::throttle::MAX_FAILURES {
            let reply = call(&app(&db), "POST", "/api/ui/v1/forms/login.loginAction", "", Some(&format!("email={key}&password=wrong"))).await;
            assert_eq!(reply.body["redirectTo"], "/login?error=CredentialsSignin");
        }
        assert!(db.state.runtime.login_throttle.begin_account(key, principal::now_unix_ms()).is_some(), "an attacker-controlled email locked internal key {key}");
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_s4_oversized_email_counts_only_ip_and_never_creates_a_large_audit_target() {
    let db = test_db().await;
    let email = "é".repeat(161);
    let encoded = email.bytes().map(|b| format!("%{b:02X}")).collect::<String>();
    for _ in 0..nwui_api::auth::throttle::MAX_IP_FAILURES {
        call(&app(&db), "POST", "/api/ui/v1/forms/login.loginAction", "", Some(&format!("email={encoded}&password=wrong"))).await;
    }
    let throttle = &db.state.runtime.login_throttle;
    for key in [email.clone(), format!("email:{email}")] {
        assert!(throttle.begin_account(&key, principal::now_unix_ms()).is_some(), "oversized email must not enter account counters");
    }
    assert!(throttle.begin_ip("127.0.0.1", principal::now_unix_ms()).is_none(), "oversized attempts still count against the IP ceiling");
    assert!(audit_rows(&db.client, "auth:login-locked").await.is_empty());
    for row in db.client.query("SELECT target FROM audit_logs WHERE target IS NOT NULL", &[]).await.unwrap() {
        assert!(row.get::<_, String>(0).len() <= 320);
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_i11_login_uses_the_same_email_casing_as_account_creation() {
    let db = test_db().await;
    let hash = password::hash_password(PASSWORD).unwrap();
    insert_user(&db.client, UserRow::default(), Some(&hash)).await;
    let reply = call(&app(&db), "POST", "/api/ui/v1/forms/login.loginAction", "", Some(&format!("email=U1%40EXAMPLE.TEST&password={PASSWORD}"))).await;
    assert_eq!(reply.body["redirectTo"], "/");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_m1_stored_mixed_case_email_accepts_both_login_casings() {
    let db = test_db().await;
    let hash = password::hash_password(PASSWORD).unwrap();
    insert_user(&db.client, UserRow { email: "Admin@Example.Test", ..UserRow::default() }, Some(&hash)).await;
    for email in ["Admin@Example.Test", "admin@example.test", "  ADMIN@EXAMPLE.TEST  "] {
        let reply = call(&app(&db), "POST", "/api/ui/v1/forms/login.loginAction", "", Some(&format!("email={email}&password={PASSWORD}"))).await;
        assert_eq!(reply.body["redirectTo"], "/", "legacy stored email must remain usable: {email}");
        assert_eq!(claims_of(&reply.set_cookies[0])["id"], "u1");
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_m1_case_only_duplicates_prefer_exact_and_refuse_ambiguous_input() {
    use nwui_api::auth::login::{authorize, Credentials, LoginOutcome};
    let db = test_db().await;
    let hash = password::hash_password(PASSWORD).unwrap();
    let other_password = "Other-passw0rd";
    let other_hash = password::hash_password(other_password).unwrap();
    insert_user(&db.client, UserRow { email: "Admin@Example.Test", ..UserRow::default() }, Some(&hash)).await;
    insert_user(&db.client, UserRow { id: "u2", email: "admin@example.test", ..UserRow::default() }, Some(&other_hash)).await;
    for (email, password, expected_id) in [("Admin@Example.Test", PASSWORD, "u1"), (" admin@example.test ", other_password, "u2")] {
        let outcome = authorize(&db.state, Credentials { email, password }, principal::now_unix_ms()).await.unwrap();
        assert!(matches!(outcome, LoginOutcome::Success(ref user) if user.id == expected_id), "exact trimmed case must select its own account: {outcome:?}");
    }
    for (email, password) in [("ADMIN@EXAMPLE.TEST", PASSWORD), ("ADMIN@EXAMPLE.TEST", other_password), ("Admin@Example.Test", other_password), ("admin@example.test", PASSWORD)] {
        assert_eq!(authorize(&db.state, Credentials { email, password }, principal::now_unix_ms()).await.unwrap(), LoginOutcome::Rejected, "ambiguous case/wrong account password must never select another row");
    }
}


#[tokio::test]
#[ignore = "real DB"]
async fn q3_w7_factor_changes_end_other_sessions_and_reissue_this_one() {
    let db = test_db().await;
    insert_user(&db.client, UserRow::default(), Some(&password::hash_password(PASSWORD).unwrap())).await;
    let app = app(&db);
    let old = cookie("u1", 3, false);
    let secret = totp::generate_secret();
    let code = totp::code_at(&totp::decode_secret(&secret).unwrap(), principal::now_unix_ms() as u64);
    let enabled = call(&app,"POST","/api/ui/v1/forms/account.enableTwoFactor",&old,Some(&format!("secret={secret}&code={code}"))).await;
    assert_eq!(enabled.status, StatusCode::OK);
    assert_eq!(me(&db).await.4, 4);
    assert_eq!(claims_of(&enabled.set_cookies[0])["sv"], 4);
    assert_eq!(call(&app,"GET","/api/ui/v1/account",&old,None).await.status, StatusCode::UNAUTHORIZED);
    let current=enabled.set_cookies[0].split(';').next().unwrap();
    let disabled=call(&app,"POST","/api/ui/v1/forms/account.disableTwoFactor",current,Some(&format!("currentPassword={PASSWORD}"))).await;
    assert_eq!(disabled.status, StatusCode::OK);
    assert_eq!(me(&db).await.4, 5);
    assert_eq!(claims_of(&disabled.set_cookies[0])["sv"], 5);
    assert_eq!(call(&app,"GET","/api/ui/v1/account",current,None).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&app,"GET","/api/ui/v1/account",disabled.set_cookies[0].split(';').next().unwrap(),None).await.status, StatusCode::OK);
}
