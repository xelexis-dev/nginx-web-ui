//! HTTP tests for `auth::router()` through the root request-context/Origin boundary, on a real
//! PostgreSQL database. Ignored by default:
//!
//!     NWUI_TEST_DATABASE_URL=postgres://user:pass@host:5432/postgres cargo test --test auth_http -- --include-ignored

mod auth_support;

use std::net::SocketAddr;

use auth_support::{TestDb, UserRow, audit_rows, insert_user, test_db};
use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use nwui_api::auth::{password, session};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const ORIGIN: &str = "http://admin.example.test";
const PASSWORD: &str = "Passw0rd-compat";

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

fn form(path: &str) -> axum::http::request::Builder {
    request("POST", path).header("origin", ORIGIN).header("content-type", "application/x-www-form-urlencoded")
}

struct Reply {
    status: StatusCode,
    set_cookies: Vec<String>,
    www_authenticate: Option<String>,
    body: Value,
}

async fn send(app: &Router, request: Request<Body>) -> Reply {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let set_cookies = response.headers().get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_owned()).collect();
    let www_authenticate = response.headers().get("www-authenticate").map(|v| v.to_str().unwrap().to_owned());
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let body = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
    Reply { status, set_cookies, www_authenticate, body }
}

async fn post_form(app: &Router, path: &str, body: &str, cookie: Option<&str>) -> Reply {
    let mut builder = form(path);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    send(app, builder.body(Body::from(body.to_owned())).unwrap()).await
}

async fn user_with_password(db: &TestDb, row: UserRow<'_>) {
    insert_user(&db.client, row, Some(&password::hash_password(PASSWORD).unwrap())).await;
}

fn login_body(email: &str, password: &str) -> String {
    format!("email={}&password={}&totp=", urlencode(email), urlencode(password))
}

fn urlencode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// `name=value` of the first Set-Cookie that stores a value.
fn stored_cookie(set_cookies: &[String]) -> String {
    set_cookies.iter().find(|c| !c.contains("Max-Age=0")).unwrap().split(';').next().unwrap().to_owned()
}

// ─────────────────────────────── session / options ───────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn session_view_is_the_contract_shape_and_never_sets_cookies() {
    let db = test_db().await;
    let app = app(&db);
    let reply = send(&app, request("GET", "/api/ui/v1/session").body(Body::empty()).unwrap()).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.www_authenticate.as_deref(), Some("Session realm=\"nginx-web-ui\""));

    insert_user(&db.client, UserRow::default(), None).await;
    let now = nwui_api::auth::principal::now_unix_ms() / 1000;
    let claims = json!({"sub": "u1", "id": "u1", "name": "홍길동", "role": "admin", "sv": 3, "totpEnabled": false, "requires2fa": true});
    let token = session::encode(claims.as_object().unwrap(), auth_support::SECRET, session::COOKIE_NAME_PLAIN, now, 3600).unwrap();
    let reply = send(&app, request("GET", "/api/ui/v1/session").header("cookie", format!("authjs.session-token={token}")).body(Body::empty()).unwrap()).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    let expires = chrono::DateTime::from_timestamp(now + 3600, 0).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    assert_eq!(reply.body, json!({
        "user": {"id": "u1", "email": "u1@example.test", "name": "홍길동", "image": null, "role": "editor", "sv": 3, "totpEnabled": false, "requires2fa": true, "verificationRequired": false},
        "expires": expires,
    }));
    assert!(reply.set_cookies.is_empty(), "SSR reads this; cookies are set only on browser form responses");
}

fn token_with(remaining_secs: i64, sv: i64) -> (String, i64) {
    let now = nwui_api::auth::principal::now_unix_ms() / 1000;
    let claims = json!({"sub": "u1", "id": "u1", "role": "editor", "sv": sv, "totpEnabled": false, "requires2fa": false});
    // Issued so that `remaining_secs` of the 8 h remain.
    let issued = now - (session::SESSION_MAX_AGE_SECS - remaining_secs);
    let token = session::encode(claims.as_object().unwrap(), auth_support::SECRET, session::COOKIE_NAME_PLAIN, issued, session::SESSION_MAX_AGE_SECS).unwrap();
    (token, issued + session::SESSION_MAX_AGE_SECS)
}

fn exp_of(cookie: &str) -> i64 {
    let token = cookie.strip_prefix("authjs.session-token=").unwrap();
    let now = nwui_api::auth::principal::now_unix_ms() / 1000;
    session::decode(token, &[auth_support::SECRET], session::COOKIE_NAME_PLAIN, now).unwrap()["exp"].as_i64().unwrap()
}

fn iso(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn session_refresh_renews_only_below_four_hours_remaining() {
    let db = test_db().await;
    let app = app(&db);
    insert_user(&db.client, UserRow::default(), None).await;
    let path = "/api/ui/v1/session/refresh";

    // An unsafe browser method: the root boundary demands Origin.
    let (token, _) = token_with(3600, 3);
    let reply = send(&app, request("POST", path).header("cookie", format!("authjs.session-token={token}")).body(Body::empty()).unwrap()).await;
    assert_eq!((reply.status, reply.body["error"]["code"].as_str()), (StatusCode::FORBIDDEN, Some("invalid_origin")));

    let reply = post_form(&app, path, "", None).await;
    assert_eq!((reply.status, reply.www_authenticate.as_deref()), (StatusCode::UNAUTHORIZED, Some("Session realm=\"nginx-web-ui\"")));

    // 1 h left → re-issued for a full 8 h; the view reports the new expiry.
    let reply = post_form(&app, path, "", Some(&format!("authjs.session-token={token}"))).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    let renewed = stored_cookie(&reply.set_cookies);
    let now = nwui_api::auth::principal::now_unix_ms() / 1000;
    let new_exp = exp_of(&renewed);
    assert!((new_exp - (now + session::SESSION_MAX_AGE_SECS)).abs() <= 2, "{new_exp} vs {now}");
    assert_eq!(reply.body["expires"], iso(new_exp));
    assert_eq!(reply.body["user"]["id"], "u1");
    let reply = send(&app, request("GET", "/api/ui/v1/session").header("cookie", &renewed).body(Body::empty()).unwrap()).await;
    assert_eq!(reply.status, StatusCode::OK, "the renewed cookie is a valid session");

    // 5 h left → unchanged: no Set-Cookie, the current expiry.
    let (token, exp) = token_with(5 * 3600, 3);
    let reply = post_form(&app, path, "", Some(&format!("authjs.session-token={token}"))).await;
    assert_eq!((reply.status, reply.set_cookies.len()), (StatusCode::OK, 0));
    assert_eq!(reply.body["expires"], iso(exp));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn session_refresh_never_renews_a_revoked_session() {
    let db = test_db().await;
    let app = app(&db);
    insert_user(&db.client, UserRow { sv: 4, ..Default::default() }, None).await;
    let (token, _) = token_with(600, 3);
    let reply = post_form(&app, "/api/ui/v1/session/refresh", "", Some(&format!("authjs.session-token={token}"))).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.set_cookies.len(), 1, "the dead cookie is expired: {:?}", reply.set_cookies);
    assert!(reply.set_cookies[0].starts_with("authjs.session-token=;") && reply.set_cookies[0].contains("Max-Age=0"));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn auth_options_list_only_usable_enabled_providers() {
    let db = test_db().await;
    let app = app(&db);
    let reply = send(&app, request("GET", "/api/ui/v1/auth/options").body(Body::empty()).unwrap()).await;
    assert_eq!(reply.body, json!({"firstRun": true, "authenticated": false, "providers": [], "twoFactorPending": false, "passkey": {"available": false, "reason": "insecure_origin"}}));

    let sealed = db.state.secrets.seal("client-secret").unwrap();
    for (id, name, secret, enabled, sort) in [
        ("zeta", "Zeta SSO", sealed.as_str(), true, 0),
        ("alpha", "Alpha SSO", sealed.as_str(), true, 0),
        ("first", "Z First", sealed.as_str(), true, -1),
        ("broken", "Broken", "enc:v1:AAAA", true, 0),
        ("off", "Off", sealed.as_str(), false, 0),
    ] {
        db.client
            .execute(
                "INSERT INTO oauth_providers (id, name, issuer, client_id, client_secret_enc, enabled, sort_order) VALUES ($1, $2, 'https://idp.example.test', 'cid', $3, $4, $5)",
                &[&id, &name, &secret, &enabled, &sort],
            )
            .await
            .unwrap();
    }
    insert_user(&db.client, UserRow::default(), None).await;
    let reply = send(&app, request("GET", "/api/ui/v1/auth/options").body(Body::empty()).unwrap()).await;
    assert_eq!(reply.body, json!({"firstRun": false, "authenticated": false, "providers": [
        {"id": "first", "name": "Z First"}, {"id": "alpha", "name": "Alpha SSO"}, {"id": "zeta", "name": "Zeta SSO"},
    ], "twoFactorPending": false, "passkey": {"available": false, "reason": "insecure_origin"}}));
}

// ─────────────────────────────── account tokens ───────────────────────────────

fn cookie_of(id: &str, sv: i64, mfa_verified: bool) -> String {
    let now = nwui_api::auth::principal::now_unix_ms() / 1000;
    let claims = json!({"sub": id, "id": id, "sv": sv, "mfaVerified": mfa_verified});
    let token = session::encode(claims.as_object().unwrap(), auth_support::SECRET, session::COOKIE_NAME_PLAIN, now, 3600).unwrap();
    format!("authjs.session-token={token}")
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn account_tokens_issue_once_list_own_and_revoke() {
    let db = test_db().await;
    let app = app(&db);
    insert_user(&db.client, UserRow { id: "vi", email: "vi@example.test", role: "viewer", totp_enabled: true, ..Default::default() }, None).await;
    let vi = cookie_of("vi", 3, true);

    let reply = send(&app, request("GET", "/api/ui/v1/account/tokens").header("cookie", &vi).body(Body::empty()).unwrap()).await;
    assert_eq!((reply.status, reply.body.clone()), (StatusCode::OK, json!({"tokens": [], "issuableScopes": ["config:read"]})));

    let reply = post_form(&app, "/api/ui/v1/forms/account.tokens.issue", "name=&expiry=never", Some(&vi)).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.body["error"]["fieldErrors"]["scopes"], "권한을 하나 이상 선택하세요.");
    let reply = post_form(&app, "/api/ui/v1/forms/account.tokens.issue", "name=x&scopes=config%3Awrite&expiry=never", Some(&vi)).await;
    assert_eq!((reply.status, reply.body["error"]["fieldErrors"]["scopes"].as_str()), (StatusCode::BAD_REQUEST, Some("이 토큰에 줄 수 없는 권한입니다: config:write")));

    let reply = post_form(&app, "/api/ui/v1/forms/account.tokens.issue", "name=CI&scopes=config%3Aread&expiry=30d", Some(&vi)).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["result"]["ok"], true, "IssueTokenState.ok");
    let token = reply.body["result"]["token"].as_str().unwrap().to_owned();
    let id = reply.body["result"]["id"].as_str().unwrap().to_owned();
    assert!(token.starts_with("nwu_u_") && reply.body["result"]["name"] == "CI" && reply.body.get("redirectTo").is_none(), "no redirect: the plaintext rides this response only");

    let reply = send(&app, request("GET", &format!("/api/ui/v1/account/tokens/{id}")).header("cookie", &vi).body(Body::empty()).unwrap()).await;
    assert_eq!((reply.status, reply.body["status"].as_str()), (StatusCode::OK, Some("active")));
    assert!(!reply.body.to_string().contains(&token[6..]));

    // Another user's token id is indistinguishable from a missing one.
    insert_user(&db.client, UserRow { id: "ed", email: "ed@example.test", ..Default::default() }, None).await;
    let ed = cookie_of("ed", 3, false);
    let reply = send(&app, request("GET", &format!("/api/ui/v1/account/tokens/{id}")).header("cookie", &ed).body(Body::empty()).unwrap()).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let reply = post_form(&app, "/api/ui/v1/forms/account.tokens.revoke", &format!("id={id}"), Some(&ed)).await;
    assert_eq!((reply.status, reply.body["error"]["message"].as_str()), (StatusCode::CONFLICT, Some("해지할 항목이 없습니다 — 이미 해지되었거나 없는 토큰입니다.")));

    let reply = post_form(&app, "/api/ui/v1/forms/account.tokens.revoke", &format!("id={id}"), Some(&vi)).await;
    assert_eq!((reply.status, reply.body.clone()), (StatusCode::OK, json!({"result": {"ok": true}, "refresh": true})));
    let reply = send(&app, request("GET", "/api/ui/v1/account/tokens").header("cookie", &vi).body(Body::empty()).unwrap()).await;
    assert_eq!(reply.body["tokens"][0]["status"], "revoked");
    // Sign-in is required.
    let reply = send(&app, request("GET", "/api/ui/v1/account/tokens").body(Body::empty()).unwrap()).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}

// ─────────────────────────────── setup ───────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn first_admin_setup_form() {
    let db = auth_support::test_db_with(&[("AUTH_URL", ORIGIN)]).await;
    let app = nwui_api::app::with_request_context(nwui_api::auth::router().with_state(db.state.clone()), Some(ORIGIN.to_owned()));
    let path = "/api/ui/v1/forms/setup.createFirstAdmin";
    let body = format!("email=admin%40example.test&password={PASSWORD}&confirmPassword={PASSWORD}");

    // The root boundary refuses a cross-origin write before the handler runs.
    let reply = send(&app, request("POST", path).header("content-type", "application/x-www-form-urlencoded").body(Body::from(body.clone())).unwrap()).await;
    assert_eq!((reply.status, reply.body["error"]["code"].as_str()), (StatusCode::FORBIDDEN, Some("invalid_origin")));

    let reply = post_form(&app, path, "email=bad&password=x&confirmPassword=y", None).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.body["error"]["code"], "validation_failed");
    assert_eq!(reply.body["error"]["fieldErrors"]["email"], "올바른 이메일 형식을 입력하세요.");
    assert_eq!(reply.body["error"]["fieldErrors"]["confirmPassword"], "비밀번호가 일치하지 않습니다.");

    let reply = post_form(&app, path, &body, None).await;
    assert_eq!((reply.status, reply.body.clone()), (StatusCode::OK, json!({"redirectTo": "/login"})));
    assert!(reply.set_cookies.is_empty(), "setup does not sign the admin in");
    let audit = audit_rows(&db.client, "admin:first-setup").await;
    assert_eq!(audit[0].get::<_, Option<String>>("ip").as_deref(), Some("203.0.113.20"), "X-Real-IP from the trusted loopback proxy");

    let reply = post_form(&app, path, &body.replace("admin%40", "again%40"), None).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.body["error"]["message"], "이미 관리자 계정이 존재합니다. 로그인해 주세요.");
}

// ─────────────────────────────── login / sign-out ───────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn login_sets_the_session_cookie_that_the_session_route_accepts() {
    let db = test_db().await;
    let app = app(&db);
    user_with_password(&db, UserRow::default()).await;
    let stale = "authjs.session-token.0=old; authjs.session-token.1=old";
    let reply = post_form(&app, "/api/ui/v1/forms/login.loginAction", &login_body("u1@example.test", PASSWORD), Some(stale)).await;
    assert_eq!((reply.status, reply.body.clone()), (StatusCode::OK, json!({"redirectTo": "/"})));
    assert!(reply.set_cookies[0].starts_with("authjs.session-token="));
    assert!(reply.set_cookies[0].contains("; Path=/;") && reply.set_cookies[0].ends_with("; HttpOnly; SameSite=Lax"), "{:?}", reply.set_cookies);
    let expired: Vec<&str> = reply.set_cookies.iter().filter(|c| c.contains("Max-Age=0")).map(|c| c.split('=').next().unwrap()).collect();
    assert_eq!(expired, ["authjs.session-token.0", "authjs.session-token.1"], "stale chunks are expired");

    let audit = audit_rows(&db.client, "login").await;
    assert_eq!((audit.len(), audit[0].get::<_, Option<String>>("actor_id").as_deref()), (1, Some("u1")));

    let cookie = stored_cookie(&reply.set_cookies);
    let reply = send(&app, request("GET", "/api/ui/v1/session").header("cookie", &cookie).body(Body::empty()).unwrap()).await;
    assert_eq!((reply.status, reply.body["user"]["id"].as_str()), (StatusCode::OK, Some("u1")));

    let reply = post_form(&app, "/api/ui/v1/forms/session.signOut", "", Some(&cookie)).await;
    assert_eq!(reply.body, json!({"redirectTo": "/login"}));
    assert!(reply.set_cookies.len() == 1 && reply.set_cookies[0].starts_with("authjs.session-token=;") && reply.set_cookies[0].contains("Max-Age=0"));
    let audit = audit_rows(&db.client, "logout").await;
    assert_eq!(audit[0].get::<_, Option<String>>("actor_id").as_deref(), Some("u1"));

    // Signing out without a session writes no audit row (Auth.js signout has no token to report).
    let reply = post_form(&app, "/api/ui/v1/forms/session.signOut", "", None).await;
    assert_eq!((reply.body.clone(), reply.set_cookies.len()), (json!({"redirectTo": "/login"}), 0));
    assert_eq!(audit_rows(&db.client, "logout").await.len(), 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn https_requests_use_the_secure_cookie_name() {
    let db = test_db().await;
    let app = app(&db);
    user_with_password(&db, UserRow::default()).await;
    let request = request_https("/api/ui/v1/forms/login.loginAction").body(Body::from(login_body("u1@example.test", PASSWORD))).unwrap();
    let reply = send(&app, request).await;
    assert_eq!(reply.body, json!({"redirectTo": "/"}));
    assert!(reply.set_cookies[0].starts_with("__Secure-authjs.session-token="), "{:?}", reply.set_cookies);
    assert!(reply.set_cookies[0].ends_with("; Secure"));
}

fn request_https(path: &str) -> axum::http::request::Builder {
    request("POST", path)
        .header("x-forwarded-proto", "https")
        .header("origin", "https://admin.example.test")
        .header("content-type", "application/x-www-form-urlencoded")
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn login_failures_redirect_alike_and_feed_fail2ban() {
    let db = test_db().await;
    let app = app(&db);
    user_with_password(&db, UserRow::default()).await;
    user_with_password(&db, UserRow { id: "off", email: "off@example.test", enabled: false, ..Default::default() }).await;
    user_with_password(&db, UserRow { id: "wait", email: "wait@example.test", approved: false, ..Default::default() }).await;
    let path = "/api/ui/v1/forms/login.loginAction";

    for (email, pw, redirect) in [
        ("u1@example.test", "wrong-Passw0rd", "/login?error=CredentialsSignin"),
        ("nobody@example.test", PASSWORD, "/login?error=CredentialsSignin"),
        ("off@example.test", PASSWORD, "/login?error=disabled"),
        ("wait@example.test", PASSWORD, "/login?error=pending"),
    ] {
        let reply = post_form(&app, path, &login_body(email, pw), None).await;
        assert_eq!((reply.status, reply.body.clone()), (StatusCode::OK, json!({"redirectTo": redirect})), "{email}");
        assert!(reply.set_cookies.is_empty());
    }
    let log = std::fs::read_to_string(&db.auth_log).unwrap();
    let reasons: Vec<&str> = log.lines().map(|l| l.rsplit("reason=").next().unwrap()).collect();
    assert_eq!(reasons, ["bad_credentials", "bad_credentials", "account_disabled", "account_pending"]);
    assert!(log.lines().all(|l| l.contains(" nginx-web-ui auth-failure ip=203.0.113.20 ")), "{log}");
    assert!(!log.contains("@example.test"), "the email is never written");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn lockouts_are_audited_once_per_transition() {
    let db = test_db().await;
    let app = app(&db);
    user_with_password(&db, UserRow::default()).await;
    let path = "/api/ui/v1/forms/login.loginAction";
    let throttle = &db.state.runtime.login_throttle;
    let now = nwui_api::auth::principal::now_unix_ms();

    // Account: the 10th failure locks it and is audited; later ones are not.
    for _ in 0..nwui_api::auth::throttle::MAX_FAILURES - 1 {
        throttle.record_account_failure(&nwui_api::auth::throttle::login_key(&nwui_api::auth::throttle::email_key("u1@example.test"), Some("203.0.113.20")), now);
    }
    post_form(&app, path, &login_body("u1@example.test", "wrong-Passw0rd"), None).await;
    post_form(&app, path, &login_body("u1@example.test", "wrong-Passw0rd"), None).await;
    let locked = audit_rows(&db.client, "auth:login-locked").await;
    assert_eq!((locked.len(), locked[0].get::<_, Option<String>>("target").as_deref()), (1, Some("u1@example.test")));

    // Address: the failure that reaches the cap is audited once; afterwards even the right
    // password is refused before any work, logged as ip_throttled.
    for _ in 0..nwui_api::auth::throttle::MAX_IP_FAILURES {
        throttle.record_ip_failure("203.0.113.20", now);
    }
    let reply = post_form(&app, path, &login_body("other@example.test", PASSWORD), None).await;
    assert_eq!(reply.body, json!({"redirectTo": "/login?error=CredentialsSignin"}));
    assert!(std::fs::read_to_string(&db.auth_log).unwrap().lines().last().unwrap().ends_with("reason=ip_throttled"));
    assert_eq!(audit_rows(&db.client, "auth:ip-locked").await.len(), 0, "already locked before this attempt: no transition");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn the_failure_that_locks_an_address_is_audited() {
    let db = test_db().await;
    let app = app(&db);
    let throttle = &db.state.runtime.login_throttle;
    let now = nwui_api::auth::principal::now_unix_ms();
    for _ in 0..nwui_api::auth::throttle::MAX_IP_FAILURES - 1 {
        throttle.record_ip_failure("203.0.113.20", now);
    }
    post_form(&app, "/api/ui/v1/forms/login.loginAction", &login_body("x@example.test", "whatever1"), None).await;
    let rows = audit_rows(&db.client, "auth:ip-locked").await;
    assert_eq!((rows.len(), rows[0].get::<_, Option<String>>("target").as_deref()), (1, Some("203.0.113.20")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_concurrent_burst_of_wrong_passwords_locks_once_and_stops_at_the_caps() {
    // Each attempt is counted when it is admitted, not after bcrypt finishes. Otherwise a
    // burst passes the lock check before the first failure is recorded and every request
    // reaches the password check (15-minute cap: 10 per account, 30 per address).
    let db = test_db().await;
    let app = app(&db);
    user_with_password(&db, UserRow::default()).await;
    let mut burst = tokio::task::JoinSet::new();
    for _ in 0..40 {
        let app = app.clone();
        burst.spawn(async move {
            post_form(&app, "/api/ui/v1/forms/login.loginAction", &login_body("u1@example.test", "wrong-password"), None).await.status
        });
    }
    while let Some(status) = burst.join_next().await {
        assert_eq!(status.unwrap(), StatusCode::OK);
    }
    assert_eq!(audit_rows(&db.client, "auth:login-locked").await.len(), 1, "the account lock engages exactly once");
    assert_eq!(audit_rows(&db.client, "auth:ip-locked").await.len(), 1, "the address lock engages exactly once");
    // Locked: even the right password is refused for the rest of the window.
    let reply = post_form(&app, "/api/ui/v1/forms/login.loginAction", &login_body("u1@example.test", PASSWORD), None).await;
    assert_eq!(reply.body, json!({"redirectTo": "/login?error=CredentialsSignin"}));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn behind_a_tls_terminating_proxy_the_scheme_is_not_compared_unless_auth_url_is_set() {
    // A reverse proxy in front of the published port terminates TLS and forwards plain HTTP, so
    // this API derives http:// while the browser's Origin says https://. 1.17.x (Next.js Server
    // Actions) compared the host only; the same holds here unless AUTH_URL pins the origin.
    let db = test_db().await;
    user_with_password(&db, UserRow::default()).await;
    let login = |app: Router, origin: &'static str| async move {
        let request = request("POST", "/api/ui/v1/forms/login.loginAction")
            .header("origin", origin)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(login_body("u1@example.test", PASSWORD)))
            .unwrap();
        send(&app, request).await
    };
    let derived = app(&db);
    let reply = login(derived.clone(), "https://admin.example.test").await;
    assert_eq!((reply.status, reply.body), (StatusCode::OK, json!({"redirectTo": "/"})));
    assert_eq!(login(derived.clone(), "https://evil.example.test").await.status, StatusCode::FORBIDDEN);
    assert_eq!(login(derived, "https://admin.example.test:8443").await.status, StatusCode::FORBIDDEN);

    let pinned = nwui_api::app::with_request_context(
        nwui_api::auth::router().with_state(db.state.clone()),
        Some("https://admin.example.test".into()),
    );
    assert_eq!(login(pinned.clone(), "http://admin.example.test").await.status, StatusCode::FORBIDDEN);
    assert_eq!(login(pinned, "https://admin.example.test").await.status, StatusCode::OK);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn first_run_setup_rejects_dns_hosts_even_with_matching_origin() {
    for host in ["rebind.attacker.test", "127.0.0.1.attacker.test", "localhost.attacker.test"] {
        let db = test_db().await;
        let app = app(&db);
        let body = format!("email=admin%40example.test&password={PASSWORD}&confirmPassword={PASSWORD}");
        let req = Request::builder().method("POST").uri("/api/ui/v1/forms/setup.createFirstAdmin")
            .header("host", host).header("origin", format!("http://{host}"))
            .header("content-type", "application/x-www-form-urlencoded")
            .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))))
            .body(Body::from(body)).unwrap();
        let reply = send(&app, req).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "unconfigured DNS host {host} must not create an admin: {}", reply.body);
        assert!(reply.body["error"]["message"].as_str().unwrap().contains("AUTH_URL"));
        assert_eq!(db.client.query_one("SELECT count(*) FROM users", &[]).await.unwrap().get::<_, i64>(0), 0);
        assert!(audit_rows(&db.client, "admin:first-setup").await.is_empty());
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn first_run_setup_keeps_loopback_and_ip_literal_installation() {
    for host in ["localhost:8080", "127.0.0.1:8080", "192.0.2.45:8080", "[::1]:8080", "[2001:db8::10]:8080"] {
        let db = test_db().await;
        let app = app(&db);
        let body = format!("email=admin%40example.test&password={PASSWORD}&confirmPassword={PASSWORD}");
        let req = Request::builder().method("POST").uri("/api/ui/v1/forms/setup.createFirstAdmin")
            .header("host", host).header("origin", format!("http://{host}"))
            .header("content-type", "application/x-www-form-urlencoded")
            .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))))
            .body(Body::from(body)).unwrap();
        let reply = send(&app, req).await;
        assert_eq!(reply.status, StatusCode::OK, "{host}: {}", reply.body);
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn first_run_setup_accepts_configured_dns_and_keeps_initialized_flow() {
    let db = auth_support::test_db_with(&[("AUTH_URL", ORIGIN)]).await;
    let configured = nwui_api::app::with_request_context(nwui_api::auth::router().with_state(db.state.clone()), Some(ORIGIN.to_owned()));
    let body = format!("email=admin%40example.test&password={PASSWORD}&confirmPassword={PASSWORD}");
    assert_eq!(post_form(&configured, "/api/ui/v1/forms/setup.createFirstAdmin", &body, None).await.status, StatusCode::OK);
    let unconfigured = app(&db);
    let reply = post_form(&unconfigured, "/api/ui/v1/forms/setup.createFirstAdmin", &body, None).await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "initialized instances must keep their existing flow");
    assert_eq!(send(&unconfigured, request("GET", "/api/ui/v1/auth/options").body(Body::empty()).unwrap()).await.body["firstRun"], false);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn remote_login_failures_do_not_lock_the_admin_on_another_ip() {
    let db = test_db().await;
    user_with_password(&db, UserRow { role: "admin", ..Default::default() }).await;
    let app = app(&db);
    let from = |ip: &str, password: &str| Request::builder()
        .method("POST").uri("/api/ui/v1/forms/login.loginAction")
        .header("host", "admin.example.test").header("origin", ORIGIN)
        .header("content-type", "application/x-www-form-urlencoded")
        .extension(ConnectInfo(format!("{ip}:40000").parse::<SocketAddr>().unwrap()))
        .body(Body::from(login_body("u1@example.test", password))).unwrap();
    for _ in 0..nwui_api::auth::throttle::MAX_FAILURES {
        let reply = send(&app, from("203.0.113.51", "wrong-Passw0rd")).await;
        assert!(reply.body["redirectTo"].as_str().unwrap().starts_with("/login?error="));
    }
    let attacker = send(&app, from("203.0.113.51", PASSWORD)).await;
    assert!(attacker.body["redirectTo"].as_str().unwrap().starts_with("/login?error="));
    let admin = send(&app, from("203.0.113.52", PASSWORD)).await;
    assert_eq!(admin.body["redirectTo"], "/", "a remote guesser must not lock out another address: {:?}", admin.body);
    assert!(admin.set_cookies.iter().any(|c| c.starts_with("authjs.session-token=") && !c.contains("Max-Age=0")));
    let still_locked = send(&app, from("203.0.113.51", PASSWORD)).await;
    assert!(still_locked.body["redirectTo"].as_str().unwrap().starts_with("/login?error="), "success at B must not clear A");
}


#[tokio::test]
#[ignore = "real DB"]
async fn q3_w8_refresh_has_an_absolute_login_age() {
    let db=test_db().await; insert_user(&db.client,UserRow::default(),None).await;
    let app=app(&db); let now=nwui_api::auth::principal::now_unix_ms()/1000;
    for auth_time in [now-30*86400, now+3600] {
        let claims=json!({"sub":"u1","id":"u1","sv":3,"authTime":auth_time});
        let token=session::encode(claims.as_object().unwrap(),auth_support::SECRET,session::COOKIE_NAME_PLAIN,now,3600).unwrap();
        let reply=post_form(&app,"/api/ui/v1/session/refresh","",Some(&format!("authjs.session-token={token}"))).await;
        assert_eq!(reply.status,StatusCode::UNAUTHORIZED,"authTime {auth_time}");
        assert!(reply.set_cookies.iter().any(|c|c.contains("Max-Age=0")));
    }
}

#[tokio::test]
#[ignore = "real DB"]
async fn q3r1_legacy_session_starts_its_age_at_first_refresh_not_old_iat() {
    use josekit::jwe::{self, JweHeader, Dir};
    let db=test_db().await; insert_user(&db.client,UserRow::default(),None).await;
    let now=nwui_api::auth::principal::now_unix_ms()/1000;
    let claims=json!({"id":"u1","sub":"u1","sv":3,"iat":now-31*86400,"exp":now+5*3600});
    let key=session::derive_key(auth_support::SECRET,session::COOKIE_NAME_PLAIN);
    let mut header=JweHeader::new();header.set_content_encryption(session::CONTENT_ENCRYPTION);header.set_key_id(session::key_id(&key));
    let token=jwe::serialize_compact(&serde_json::to_vec(&claims).unwrap(),&header,&Dir.encrypter_from_bytes(key).unwrap()).unwrap();
    let reply=post_form(&app(&db),"/api/ui/v1/session/refresh","",Some(&format!("authjs.session-token={token}"))).await;
    assert_eq!(reply.status,StatusCode::OK,"{}",reply.body);
    let cookie=stored_cookie(&reply.set_cookies);
    let claims=session::decode(cookie.strip_prefix("authjs.session-token=").unwrap(),&[auth_support::SECRET],session::COOKIE_NAME_PLAIN,now).unwrap();
    assert!((claims["authTime"].as_i64().unwrap()-now).abs()<=2);
}


#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn account_token_issue_requires_enrolled_totp() {
    let db = test_db().await;
    insert_user(&db.client, UserRow::default(), None).await;
    let reply = post_form(&app(&db), "/api/ui/v1/forms/account.tokens.issue", "name=CI&scopes=config%3Aread&expiry=never", Some(&cookie_of("u1", 3, false))).await;
    assert_eq!((reply.status, reply.body["error"]["code"].as_str()), (StatusCode::FORBIDDEN, Some("two_factor_required")));
    assert_eq!(db.client.query_one("SELECT count(*) FROM access_tokens", &[]).await.unwrap().get::<_, i64>(0), 0);
    assert!(audit_rows(&db.client, "token:issue").await.is_empty());
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn account_token_names_reject_controls_without_changing_existing_rows() {
    let db = test_db().await;
    insert_user(&db.client, UserRow { totp_enabled: true, ..Default::default() }, None).await;
    let legacy = "legacy\u{7}name";
    db.client.execute("INSERT INTO access_tokens(id,kind,owner_user_id,name,token_hash,token_suffix,scopes) VALUES('legacy','user','u1',$1,'fixture-hash','test','[]')", &[&legacy]).await.unwrap();
    let app = app(&db);
    let cookie = cookie_of("u1", 3, true);
    for name in ["bell\u{7}", "\u{1b}[31mescape", "back\u{8}space", "\ntrimmed", "next\u{85}line"] {
        let body = url::form_urlencoded::Serializer::new(String::new()).append_pair("name", name).append_pair("scopes", "config:read").append_pair("expiry", "never").finish();
        let reply = post_form(&app, "/api/ui/v1/forms/account.tokens.issue", &body, Some(&cookie)).await;
        assert_eq!((reply.status, reply.body["error"]["fieldErrors"]["name"].as_str()), (StatusCode::BAD_REQUEST, Some("이름에 제어 문자를 쓸 수 없습니다.")));
    }
    assert_eq!(db.client.query_one("SELECT name FROM access_tokens WHERE id='legacy'", &[]).await.unwrap().get::<_, String>(0), legacy);
    assert_eq!(db.client.query_one("SELECT count(*) FROM access_tokens", &[]).await.unwrap().get::<_, i64>(0), 1);
    assert!(audit_rows(&db.client, "token:issue").await.is_empty());
}
