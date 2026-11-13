//! Real-PostgreSQL tests for `auth::{principal, login, setup}`. Ignored by default:
//!
//!     NWUI_TEST_DATABASE_URL=postgres://user:pass@host:5432/postgres cargo test --test auth_db -- --include-ignored
//!
//! The per-test database and drizzle-exact migrations come from `auth_support`.

mod auth_support;

use auth_support::{TestDb, UserRow, audit_rows, ctx, insert_user, test_db, SECRET};
use axum::http::{HeaderMap, StatusCode};
use nwui_api::auth::principal::{self, TwoFactorGate};
use nwui_api::auth::rbac::Action;
use nwui_api::auth::throttle::MAX_IP_FAILURES;
use nwui_api::auth::{session, token_digest};
use nwui_api::contracts::Role;
use nwui_api::request_context::RequestContext;
use serde_json::{Value, json};

pub const NOW: i64 = 1_800_000_000;

pub fn cookie_headers(claims: Value) -> HeaderMap {
    cookie_headers_at(claims, NOW)
}

fn cookie_headers_at(claims: Value, now: i64) -> HeaderMap {
    let token = session::encode(claims.as_object().unwrap(), SECRET, session::COOKIE_NAME_PLAIN, now, session::SESSION_MAX_AGE_SECS).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("cookie", format!("{}={token}", session::COOKIE_NAME_PLAIN).parse().unwrap());
    headers
}

fn claims_for(id: &str, sv: Value, requires_2fa: bool) -> Value {
    json!({"sub": id, "id": id, "email": "stale@example.test", "name": "표시 이름", "role": "admin", "sv": sv, "totpEnabled": true, "requires2fa": requires_2fa})
}

// ─────────────────────────────── session ───────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn session_takes_role_and_totp_from_the_db_row() {
    let db = test_db().await;
    insert_user(&db.client, UserRow { role: "viewer", ..Default::default() }, None).await;
    let principal = principal::authenticate_session(&db.state, &cookie_headers(claims_for("u1", json!(3), false)), &ctx(), NOW)
        .await
        .unwrap()
        .expect("valid session");
    assert_eq!(principal.role, Role::Viewer, "the token said admin; the row wins");
    assert!(!principal.totp_enabled, "the token said true; the row wins");
    assert_eq!(principal.email.as_deref(), Some("u1@example.test"));
    assert_eq!(principal.name.as_deref(), Some("표시 이름"));
    assert_eq!((principal.session_version, principal.expires_at), (3, NOW + session::SESSION_MAX_AGE_SECS));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn revoked_disabled_unapproved_or_deleted_accounts_have_no_session() {
    let db = test_db().await;
    insert_user(&db.client, UserRow::default(), None).await;
    insert_user(&db.client, UserRow { id: "off", email: "off@example.test", enabled: false, ..Default::default() }, None).await;
    insert_user(&db.client, UserRow { id: "wait", email: "wait@example.test", approved: false, ..Default::default() }, None).await;
    let state = &db.state;
    let none = |claims| async move { principal::authenticate_session(state, &cookie_headers(claims), &ctx(), NOW).await.unwrap() };
    assert!(none(claims_for("u1", json!(2), false)).await.is_none(), "sessionVersion bumped since sign-in");
    assert!(none(claims_for("u1", json!("3"), false)).await.is_none(), "a non-numeric sv never matches");
    assert!(none(claims_for("off", json!(3), false)).await.is_none(), "disabled");
    assert!(none(claims_for("wait", json!(3), false)).await.is_none(), "not approved");
    assert!(none(claims_for("gone", json!(3), false)).await.is_none(), "deleted");
    assert!(none(json!({"sub": "u1", "sv": 3})).await.is_none(), "no id claim");
    // A missing sv claim is 0, as in TS (`sv ?? 0`).
    insert_user(&db.client, UserRow { id: "zero", email: "zero@example.test", sv: 0, ..Default::default() }, None).await;
    assert!(none(json!({"id": "zero"})).await.is_some());
    // No cookie, garbage cookie, expired cookie.
    assert!(principal::authenticate_session(&db.state, &HeaderMap::new(), &ctx(), NOW).await.unwrap().is_none());
    let mut garbage = HeaderMap::new();
    garbage.insert("cookie", "authjs.session-token=not.a.jwe.at.all".parse().unwrap());
    assert!(principal::authenticate_session(&db.state, &garbage, &ctx(), NOW).await.unwrap().is_none());
    let expired_at = NOW + session::SESSION_MAX_AGE_SECS + session::CLOCK_TOLERANCE_SECS;
    assert!(principal::authenticate_session(&db.state, &cookie_headers(claims_for("u1", json!(3), false)), &ctx(), expired_at).await.unwrap().is_none());
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn secure_requests_read_only_the_secure_cookie_name() {
    let db = test_db().await;
    insert_user(&db.client, UserRow::default(), None).await;
    let secure_ctx = RequestContext { secure: true, external_origin: "https://admin.example.test".into(), ..ctx() };
    // A plain-name cookie is not the session of an https request.
    assert!(principal::authenticate_session(&db.state, &cookie_headers(claims_for("u1", json!(3), false)), &secure_ctx, NOW).await.unwrap().is_none());
    let token = session::encode(claims_for("u1", json!(3), false).as_object().unwrap(), SECRET, session::COOKIE_NAME_SECURE, NOW, 60).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("cookie", format!("{}={token}", session::COOKIE_NAME_SECURE).parse().unwrap());
    assert!(principal::authenticate_session(&db.state, &headers, &secure_ctx, NOW).await.unwrap().is_some());
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn require_session_enforces_2fa_enrollment_and_rbac_with_audit() {
    use axum::response::IntoResponse;
    let db = test_db().await;
    insert_user(&db.client, UserRow { role: "viewer", ..Default::default() }, None).await;
    let ctx = ctx();
    // require_session reads the real clock; its cookie must use that clock too.
    let now = principal::now_unix_ms() / 1000;

    let err = principal::require_session(&db.state, &HeaderMap::new(), &ctx, None, TwoFactorGate::Enforce).await.unwrap_err();
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()["www-authenticate"], "Session realm=\"nginx-web-ui\"", "never a Bearer challenge on the session surface");

    // Required 2FA not enrolled (DB totp_enabled=false although the token claims true).
    let must_enroll = cookie_headers_at(claims_for("u1", json!(3), true), now);
    let err = principal::require_session(&db.state, &must_enroll, &ctx, None, TwoFactorGate::Enforce).await.unwrap_err();
    assert_eq!((err.status, err.code), (StatusCode::FORBIDDEN, "two_factor_required"));
    assert!(principal::require_session(&db.state, &must_enroll, &ctx, None, TwoFactorGate::AllowEnrollment).await.is_ok());

    let viewer = cookie_headers_at(claims_for("u1", json!(3), false), now);
    assert!(principal::require_session(&db.state, &viewer, &ctx, Some(Action::ConfigRead), TwoFactorGate::Enforce).await.is_ok());
    assert!(audit_rows(&db.client, "rbac:denied").await.is_empty());
    let err = principal::require_session(&db.state, &viewer, &ctx, Some(Action::ConfigWrite), TwoFactorGate::Enforce).await.unwrap_err();
    assert_eq!((err.status, err.code), (StatusCode::FORBIDDEN, "forbidden"));
    let rows = audit_rows(&db.client, "rbac:denied").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, Option<String>>("actor_id").as_deref(), Some("u1"));
    assert_eq!(rows[0].get::<_, Option<String>>("target").as_deref(), Some("config:write"));
    assert_eq!(rows[0].get::<_, Option<Value>>("before"), Some(json!({"role": "viewer", "requiredAction": "config:write"})));
    assert_eq!(rows[0].get::<_, Option<String>>("ip").as_deref(), Some("198.51.100.7"));
}

// ─────────────────────────────── gateway ───────────────────────────────

async fn insert_gateway(client: &tokio_postgres::Client, id: &str, token: &str, role: &str, revoked: bool) {
    auth_support::gateway_issuer(client).await;
    client
        .execute(
            "INSERT INTO gateway_tokens (id, name, token_hash, role, revoked_at,created_by_user_id) VALUES ($1, $2, $3, $4::text::role, CASE WHEN $5 THEN now() END,'q3-gateway-issuer')",
            &[&id, &format!("gw-{id}"), &token_digest(token), &role, &revoked],
        )
        .await
        .unwrap();
}

fn gateway_headers(version: Option<&str>, authorization: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(v) = version {
        headers.insert("x-gateway-version", v.parse().unwrap());
    }
    if let Some(a) = authorization {
        headers.insert("authorization", a.parse().unwrap());
    }
    headers
}

async fn last_used(client: &tokio_postgres::Client, table: &str, id: &str) -> (bool, Option<String>) {
    let row = client
        .query_one(&format!("SELECT last_used_at IS NOT NULL, last_used_ip FROM {table} WHERE id = $1"), &[&id])
        .await
        .unwrap();
    (row.get(0), row.get(1))
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn gateway_checks_version_before_the_token() {
    let db = test_db().await;
    let version = db.state.config.version.clone();
    let err = principal::require_gateway(&db.state, &gateway_headers(Some("0.0.1"), None), &ctx(), &[]).await.unwrap_err();
    assert_eq!((err.status, err.code), (StatusCode::CONFLICT, "version_mismatch"));
    let err = principal::require_gateway(&db.state, &gateway_headers(Some(&version), None), &ctx(), &[]).await.unwrap_err();
    assert_eq!((err.status, err.message.as_str()), (StatusCode::UNAUTHORIZED, "인증 토큰이 없습니다."));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn unknown_and_revoked_gateway_tokens_are_indistinguishable() {
    use axum::response::IntoResponse;
    let db = test_db().await;
    let version = db.state.config.version.clone();
    insert_gateway(&db.client, "revoked", "tok-revoked", "admin", true).await;
    let mut bodies = Vec::new();
    for token in ["tok-unknown", "tok-revoked"] {
        let err = principal::require_gateway(&db.state, &gateway_headers(Some(&version), Some(&format!("Bearer {token}"))), &ctx(), &[]).await.unwrap_err();
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()["www-authenticate"], "Bearer realm=\"nginx-web-ui\"");
        bodies.push(axum::body::to_bytes(response.into_body(), 1 << 16).await.unwrap());
    }
    assert_eq!(bodies[0], bodies[1], "byte-identical 401s");
    assert_eq!(last_used(&db.client, "gateway_tokens", "revoked").await, (false, None));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn gateway_records_use_before_rbac_and_checks_actions_in_order() {
    let db = test_db().await;
    let version = db.state.config.version.clone();
    insert_gateway(&db.client, "viewer", "tok-viewer", "viewer", false).await;
    let headers = gateway_headers(Some(&version), Some("bearer tok-viewer"));

    let err = principal::require_gateway(&db.state, &headers, &ctx(), &[Action::ConfigWrite, Action::NginxReload]).await.unwrap_err();
    assert_eq!((err.status, err.code), (StatusCode::FORBIDDEN, "forbidden"));
    assert_eq!(err.message, "이 토큰의 등급(viewer)으로는 할 수 없는 동작입니다.");
    assert_eq!(last_used(&db.client, "gateway_tokens", "viewer").await, (true, Some("198.51.100.7".into())), "a 403 still records use");

    let gateway = principal::require_gateway(&db.state, &headers, &ctx(), &[]).await.unwrap();
    assert_eq!((gateway.token_id.as_str(), gateway.name.as_str(), gateway.role), ("viewer", "gw-viewer", Role::Viewer));
    assert_eq!(gateway.actor(&ctx()).label.as_deref(), Some("gateway:gw-viewer"));
    assert!(principal::require_gateway(&db.state, &headers, &ctx(), &[Action::ConfigRead]).await.is_ok());

    insert_gateway(&db.client, "editor", "tok-editor", "editor", false).await;
    let editor = gateway_headers(Some(&version), Some("Bearer tok-editor"));
    assert!(principal::require_gateway(&db.state, &editor, &ctx(), &[Action::ConfigWrite, Action::NginxReload]).await.is_ok());
    let err = principal::require_gateway(&db.state, &editor, &ctx(), &[Action::ConfigWrite, Action::UserManage]).await.unwrap_err();
    assert_eq!(err.message, "이 토큰의 등급(editor)으로는 할 수 없는 동작입니다.");
}

// ─────────────────────────────── login ───────────────────────────────

use nwui_api::auth::login::{self, Credentials, LoginOutcome};
use nwui_api::auth::setup::{self, SetupInput, SetupOutcome};
use nwui_api::auth::{password, totp as otp};

const PASSWORD: &str = "Passw0rd-compat";

fn creds<'a>(email: &'a str, password: &'a str) -> Credentials<'a> {
    Credentials { email, password }
}

async fn user_with_password(db: &TestDb, row: UserRow<'_>) {
    let hash = password::hash_password(PASSWORD).unwrap();
    insert_user(&db.client, row, Some(&hash)).await;
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn login_success_derives_requires_2fa_from_the_role_policy() {
    let db = test_db().await;
    user_with_password(&db, UserRow { role: "editor", sv: 5, ..Default::default() }).await;
    user_with_password(&db, UserRow { id: "adm", email: "adm@example.test", role: "admin", ..Default::default() }).await;
    db.client.execute("INSERT INTO app_settings (id, require_2fa_admin, require_2fa_user) VALUES ('current', false, true)", &[]).await.unwrap();

    let LoginOutcome::Success(user) = login::authorize(&db.state, creds("u1@example.test", PASSWORD), principal::now_unix_ms()).await.unwrap() else {
        panic!("expected success")
    };
    assert_eq!((user.id.as_str(), user.role, user.session_version, user.totp_enabled, user.requires_2fa), ("u1", Role::Editor, 5, false, true));
    let mut claims = user.session_claims();
    let auth_time = claims.remove("authTime").unwrap().as_i64().unwrap();
    assert!((auth_time - principal::now_unix_ms() / 1000).abs() <= 2);
    assert_eq!(serde_json::Value::Object(claims), json!({"sub": "u1", "id": "u1", "email": "u1@example.test", "role": "editor", "sv": 5, "totpEnabled": false, "requires2fa": true, "mfaVerified": false}));
    let LoginOutcome::Success(admin) = login::authorize(&db.state, creds("adm@example.test", PASSWORD), principal::now_unix_ms()).await.unwrap() else {
        panic!("expected success")
    };
    assert!(!admin.requires_2fa, "admins follow require_2fa_admin");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn login_failures_are_one_outcome_and_the_account_locks() {
    let db = test_db().await;
    user_with_password(&db, UserRow::default()).await;
    insert_user(&db.client, UserRow { id: "oauth", email: "oauth@example.test", ..Default::default() }, None).await;
    let now = principal::now_unix_ms();
    let throttle = &db.state.runtime.login_throttle;

    for (email, pw) in [("nobody@example.test", PASSWORD), ("oauth@example.test", PASSWORD), ("u1@example.test", "wrong-Passw0rd")] {
        assert_eq!(login::authorize(&db.state, creds(email, pw), now).await.unwrap(), LoginOutcome::Rejected, "{email}");
    }
    // Empty input is refused without counting.
    assert_eq!(login::authorize(&db.state, creds("", PASSWORD), now).await.unwrap(), LoginOutcome::Rejected);
    assert_eq!(login::authorize(&db.state, creds("u1@example.test", ""), now).await.unwrap(), LoginOutcome::Rejected);

    for _ in 0..nwui_api::auth::throttle::MAX_FAILURES - 1 - 1 {
        throttle.record_account_failure(&nwui_api::auth::throttle::email_key("u1@example.test"), now);
    }
    assert!(!throttle.is_account_locked(&nwui_api::auth::throttle::email_key("u1@example.test"), now), "1 recorded wrong password + 8 = 9");
    assert!(matches!(login::authorize(&db.state, creds("u1@example.test", PASSWORD), now).await.unwrap(), LoginOutcome::Success(_)));
    assert!(!throttle.is_account_locked(&nwui_api::auth::throttle::email_key("u1@example.test"), now), "success clears the account window");
    for _ in 0..nwui_api::auth::throttle::MAX_FAILURES {
        throttle.record_account_failure(&nwui_api::auth::throttle::email_key("u1@example.test"), now);
    }
    assert_eq!(login::authorize(&db.state, creds("u1@example.test", PASSWORD), now).await.unwrap(), LoginOutcome::Rejected, "locked even with the right password");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn disabled_and_pending_are_reported_only_after_the_password_matches() {
    let db = test_db().await;
    user_with_password(&db, UserRow { id: "off", email: "off@example.test", enabled: false, ..Default::default() }).await;
    user_with_password(&db, UserRow { id: "wait", email: "wait@example.test", approved: false, ..Default::default() }).await;
    let now = principal::now_unix_ms();
    assert_eq!(login::authorize(&db.state, creds("off@example.test", "wrong-Passw0rd"), now).await.unwrap(), LoginOutcome::Rejected);
    assert_eq!(login::authorize(&db.state, creds("off@example.test", PASSWORD), now).await.unwrap(), LoginOutcome::Disabled);
    assert_eq!(login::authorize(&db.state, creds("wait@example.test", PASSWORD), now).await.unwrap(), LoginOutcome::Pending);
    // 1 (wrong password) + 8 = 9 stays unlocked only if Disabled added nothing; one more locks it.
    let throttle = &db.state.runtime.login_throttle;
    for _ in 0..nwui_api::auth::throttle::MAX_FAILURES - 2 {
        throttle.record_account_failure(&nwui_api::auth::throttle::email_key("off@example.test"), now);
    }
    assert!(!throttle.is_account_locked(&nwui_api::auth::throttle::email_key("off@example.test"), now), "Disabled must not count as a failure");
    throttle.record_account_failure(&nwui_api::auth::throttle::email_key("off@example.test"), now);
    assert!(throttle.is_account_locked(&nwui_api::auth::throttle::email_key("off@example.test"), now), "the wrong password did count");
}

async fn enable_totp(db: &TestDb, id: &str, stored_secret: &str) {
    db.client
        .execute("UPDATE users SET totp_enabled = true, totp_secret = $2 WHERE id = $1", &[&id, &stored_secret])
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_password_for_a_totp_account_is_a_second_factor_step_not_a_sign_in() {
    let db = test_db().await;
    user_with_password(&db, UserRow::default()).await;
    let stored = db.state.secrets.seal(&otp::generate_secret()).unwrap();
    enable_totp(&db, "u1", &stored).await;
    let now = principal::now_unix_ms();
    let throttle = &db.state.runtime.login_throttle;
    throttle.record_account_failure(&nwui_api::auth::throttle::email_key("u1@example.test"), now);
    let outcome = login::authorize(&db.state, creds("u1@example.test", PASSWORD), now).await.unwrap();
    assert_eq!(
        outcome,
        LoginOutcome::SecondFactor(login::PendingFactor {
            user_id: "u1".into(),
            session_version: 3,
            factor: nwui_api::auth::pending_login::factor_digest(Some(&stored)),
        })
    );
    // The right password is not a failure, but it clears nothing before the factor: the one
    // earlier failure plus MAX - 1 more is the limit only if the earlier one was kept.
    for _ in 0..nwui_api::auth::throttle::MAX_FAILURES - 1 {
        throttle.record_account_failure(&nwui_api::auth::throttle::email_key("u1@example.test"), now);
    }
    assert!(throttle.is_account_locked(&nwui_api::auth::throttle::email_key("u1@example.test"), now), "the earlier failure still counts");
    let step: Option<i64> = db.client.query_one("SELECT totp_last_step FROM users WHERE id = 'u1'", &[]).await.unwrap().get(0);
    assert_eq!(step, None);
}

async fn factor(db: &TestDb) -> login::StoredFactor {
    let row = db.client.query_one("SELECT totp_secret, backup_codes FROM users WHERE id = 'u1'", &[]).await.unwrap();
    login::StoredFactor { totp_secret: row.get(0), backup_codes: row.get(1) }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn totp_codes_are_accepted_once_and_never_for_an_earlier_step() {
    let db = test_db().await;
    user_with_password(&db, UserRow::default()).await;
    let secret = otp::generate_secret();
    enable_totp(&db, "u1", &db.state.secrets.seal(&secret).unwrap()).await;
    let bytes = otp::decode_secret(&secret).unwrap();
    let now = 1_900_000_000_000i64; // explicit clock: step = now / 30 s
    let code = |offset_ms: i64| otp::code_at(&bytes, (now + offset_ms) as u64);
    let accept = |code: String| {
        let db = &db;
        async move { login::accept_second_factor(&db.state, "u1", factor(db).await, login::FactorKind::Either, &code, now).await.unwrap() }
    };

    assert!(!accept(String::new()).await, "code missing");
    let wrong = if code(0) == "000000" { "111111" } else { "000000" };
    assert!(!accept(wrong.to_owned()).await);
    assert!(accept(code(0)).await);
    assert!(!accept(code(0)).await, "RFC 6238 §5.2 replay");
    assert!(!accept(code(-30_000)).await, "earlier step");
    assert!(accept(code(30_000)).await, "next step is fine");
    let step: Option<i64> = db.client.query_one("SELECT totp_last_step FROM users WHERE id = 'u1'", &[]).await.unwrap().get(0);
    assert_eq!(step, Some(now / 30_000 + 1));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn legacy_plaintext_totp_secrets_still_verify() {
    let db = test_db().await;
    user_with_password(&db, UserRow::default()).await;
    let secret = otp::generate_secret();
    enable_totp(&db, "u1", &secret).await;
    let now = 1_900_000_000_000i64;
    let code = otp::code_at(&otp::decode_secret(&secret).unwrap(), now as u64);
    assert!(login::accept_second_factor(&db.state, "u1", factor(&db).await, login::FactorKind::Either, &code, now).await.unwrap());
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn backup_codes_are_single_use_even_when_raced() {
    let db = test_db().await;
    user_with_password(&db, UserRow::default()).await;
    enable_totp(&db, "u1", &db.state.secrets.seal(&otp::generate_secret()).unwrap()).await;
    let codes = vec!["a1b2c-3d4e5".to_owned(), "fffff-00000".to_owned()];
    let hashes = otp::hash_backup_codes(&codes).unwrap();
    db.client.execute("UPDATE users SET backup_codes = $2 WHERE id = $1", &[&"u1", &json!(hashes)]).await.unwrap();
    let now = principal::now_unix_ms();

    let attempt = || async { login::accept_second_factor(&db.state, "u1", factor(&db).await, login::FactorKind::Either, "A1B2C3D4E5", now).await.unwrap() };
    let (a, b) = tokio::join!(attempt(), attempt());
    assert_eq!([a, b].iter().filter(|ok| **ok).count(), 1, "one code, one use");
    let stored: Value = db.client.query_one("SELECT backup_codes FROM users WHERE id = 'u1'", &[]).await.unwrap().get(0);
    assert_eq!(stored, json!([hashes[1]]));
    assert!(!attempt().await);
    assert!(login::accept_second_factor(&db.state, "u1", factor(&db).await, login::FactorKind::Either, "fffff-00000", now).await.unwrap());
}

/// Sign-ins wait for a bcrypt slot (API_PASSWORD_CONCURRENCY). While they wait they must not
/// hold a pool connection, or a burst of logins starves every other API route.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn waiting_for_bcrypt_does_not_hold_a_database_connection() {
    let db = auth_support::test_db_with(&[("API_DB_POOL_SIZE", "1"), ("API_PASSWORD_CONCURRENCY", "1")]).await;
    user_with_password(&db, UserRow::default()).await;

    // Occupy the only bcrypt slot until released.
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let hasher = db.state.passwords.clone();
    let blocker = tokio::spawn(async move {
        hasher
            .run(move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
            .await
    });
    started_rx.await.unwrap();

    let state = db.state.clone();
    let login = tokio::spawn(async move { login::authorize(&state, creds("u1@example.test", PASSWORD), principal::now_unix_ms()).await });
    // Let the sign-in load its row and queue for bcrypt.
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    let other_route = tokio::time::timeout(std::time::Duration::from_secs(3), db.state.db.get()).await;
    let got_connection = matches!(other_route, Ok(Ok(_)));
    drop(other_route);
    release_tx.send(()).unwrap();
    blocker.await.unwrap().unwrap();
    assert!(got_connection, "a queued bcrypt held the only pool connection");
    assert!(matches!(login.await.unwrap().unwrap(), LoginOutcome::Success(_)));
}

// ─────────────────────────────── seed ───────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_m1_seed_admin_canonicalizes_email_and_keeps_first_run_rules() {
    let db = test_db().await;
    // Both values are required; otherwise the web /setup flow creates the first admin.
    for (email, pw) in [(None, Some(PASSWORD)), (Some("seed@example.test"), None), (Some(""), Some(PASSWORD))] {
        assert_eq!(setup::seed_admin(&db.state, email, pw).await.unwrap(), setup::SeedOutcome::Skipped);
    }
    assert!(setup::is_first_run(&db.state).await.unwrap());
    // New env seeds use the same canonical email as setup/create; the password is unchanged.
    assert_eq!(setup::seed_admin(&db.state, Some("Seed@Example.test"), Some(PASSWORD)).await.unwrap(), setup::SeedOutcome::Seeded);
    let row = db.client.query_one("SELECT email, role::text, password_hash FROM users", &[]).await.unwrap();
    assert_eq!((row.get::<_, String>(0).as_str(), row.get::<_, String>(1).as_str()), ("seed@example.test", "admin"));
    assert!(password::verify_password(PASSWORD, &row.get::<_, String>(2)));
    // Idempotent on every boot.
    assert_eq!(setup::seed_admin(&db.state, Some("other@example.test"), Some(PASSWORD)).await.unwrap(), setup::SeedOutcome::Skipped);
    let count: i64 = db.client.query_one("SELECT count(*) FROM users", &[]).await.unwrap().get(0);
    assert_eq!(count, 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn seed_and_web_setup_racing_create_one_admin() {
    let db = test_db().await;
    let ctx = setup_ctx();
    let (seeded, created) = tokio::join!(
        setup::seed_admin(&db.state, Some("seed@example.test"), Some(PASSWORD)),
        setup::create_first_admin(&db.state, setup_input("web@example.test", PASSWORD, PASSWORD), &ctx),
    );
    let seeded = seeded.unwrap() == setup::SeedOutcome::Seeded;
    let created = matches!(created.unwrap(), SetupOutcome::Created { .. });
    assert!(seeded ^ created, "exactly one of them creates the first admin (seeded={seeded}, created={created})");
    let count: i64 = db.client.query_one("SELECT count(*) FROM users", &[]).await.unwrap().get(0);
    assert_eq!(count, 1);
}

// ─────────────────────────────── setup ───────────────────────────────

fn setup_ctx() -> RequestContext {
    RequestContext { external_origin: "http://127.0.0.1:8080".into(), ..ctx() }
}

fn setup_input<'a>(email: &'a str, password: &'a str, confirm: &'a str) -> SetupInput<'a> {
    SetupInput { email, password, confirm_password: confirm }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn first_admin_is_created_once_with_an_audit_row() {
    let db = test_db().await;
    assert!(setup::is_first_run(&db.state).await.unwrap());
    let outcome = setup::create_first_admin(&db.state, setup_input("  Admin@Example.TEST ", PASSWORD, PASSWORD), &setup_ctx()).await.unwrap();
    let SetupOutcome::Created { user_id } = outcome else { panic!("{outcome:?}") };
    assert!(!setup::is_first_run(&db.state).await.unwrap());
    let row = db.client.query_one("SELECT email, role::text, password_hash FROM users WHERE id = $1", &[&user_id]).await.unwrap();
    assert_eq!((row.get::<_, String>(0).as_str(), row.get::<_, String>(1).as_str()), ("admin@example.test", "admin"));
    assert!(password::verify_password(PASSWORD, &row.get::<_, String>(2)));
    let audit = audit_rows(&db.client, "admin:first-setup").await;
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].get::<_, Option<String>>("actor_id").as_deref(), Some(user_id.as_str()));
    assert_eq!(audit[0].get::<_, Option<Value>>("after"), Some(json!({"email": "admin@example.test", "role": "admin"})));
    assert_eq!(
        setup::create_first_admin(&db.state, setup_input("second@example.test", PASSWORD, PASSWORD), &setup_ctx()).await.unwrap(),
        SetupOutcome::AlreadyInitialized
    );
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn setup_validation_uses_the_ts_messages_and_writes_nothing() {
    let db = test_db().await;
    let SetupOutcome::Invalid(errors) = setup::create_first_admin(&db.state, setup_input("not-an-email", "short", "other"), &setup_ctx()).await.unwrap() else {
        panic!("expected validation errors")
    };
    assert_eq!(errors.get("email").map(String::as_str), Some("올바른 이메일 형식을 입력하세요."));
    assert_eq!(errors.get("password").map(String::as_str), Some("비밀번호는 최소 8자 이상이어야 합니다."));
    assert_eq!(errors.get("confirmPassword").map(String::as_str), Some("비밀번호가 일치하지 않습니다."));
    assert!(setup::is_first_run(&db.state).await.unwrap());
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn concurrent_first_setups_create_exactly_one_admin() {
    let db = test_db().await;
    let ctx = setup_ctx();
    let (a, b) = tokio::join!(
        setup::create_first_admin(&db.state, setup_input("one@example.test", PASSWORD, PASSWORD), &ctx),
        setup::create_first_admin(&db.state, setup_input("two@example.test", PASSWORD, PASSWORD), &ctx),
    );
    let outcomes = [a.unwrap(), b.unwrap()];
    assert_eq!(outcomes.iter().filter(|o| matches!(o, SetupOutcome::Created { .. })).count(), 1, "{outcomes:?}");
    assert!(outcomes.contains(&SetupOutcome::AlreadyInitialized));
    let count: i64 = db.client.query_one("SELECT count(*) FROM users", &[]).await.unwrap().get(0);
    assert_eq!(count, 1);
}

// ─────────────────────────────── pairing ───────────────────────────────

async fn insert_access_token(client: &tokio_postgres::Client, id: &str, token: &str, kind: &str, scopes: Value, expired: bool, revoked: bool) {
    let owner: Option<&str> = (kind == "user").then_some("owner");
    client
        .execute(
            "INSERT INTO access_tokens (id, kind, owner_user_id, name, token_hash, token_suffix, scopes, expires_at, revoked_at, created_by_user_id) \
             VALUES ($1, $2::text::token_kind, $3, $4, $5, 'abcd', $6, CASE WHEN $7 THEN now() - interval '1 minute' END, CASE WHEN $8 THEN now() END, 'owner')",
            &[&id, &kind, &owner, &format!("name-{id}"), &token_digest(token), &scopes, &expired, &revoked],
        )
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn pairing_rejections_follow_the_ts_mapping() {
    let db = test_db().await;
    insert_user(&db.client, UserRow { id: "owner", email: "owner@example.test", role: "admin", ..Default::default() }, None).await;
    insert_access_token(&db.client, "ok", "nwu_s_ok", "server", json!(["node:pair"]), false, false).await;
    insert_access_token(&db.client, "revoked", "nwu_s_revoked", "server", json!(["node:pair"]), false, true).await;
    insert_access_token(&db.client, "expired", "nwu_s_expired", "server", json!(["node:pair"]), true, false).await;
    insert_access_token(&db.client, "both", "nwu_s_both", "server", json!(["node:pair"]), true, true).await;
    insert_access_token(&db.client, "user", "nwu_u_user", "user", json!(["config:read"]), false, false).await;
    insert_access_token(&db.client, "noscope", "nwu_s_noscope", "server", json!(["config:read"]), false, false).await;
    let throttle = &db.state.runtime.login_throttle;
    let now_ms = principal::now_unix_ms();
    let ctx = ctx();

    let mut unauthorized = Vec::new();
    for token in ["nwu_s_missing", "nwu_s_revoked", "nwu_s_expired", "nwu_s_both"] {
        let err = principal::authenticate_pairing(&db.state, token, &ctx, now_ms).await.unwrap_err();
        unauthorized.push((err.status, err.code, err.message));
    }
    assert!(unauthorized.iter().all(|e| *e == (StatusCode::UNAUTHORIZED, "unauthorized", "토큰이 유효하지 않거나 해지되었습니다.".to_owned())), "{unauthorized:?}");

    let err = principal::authenticate_pairing(&db.state, "nwu_u_user", &ctx, now_ms).await.unwrap_err();
    assert_eq!((err.status, err.message.as_str()), (StatusCode::FORBIDDEN, "사용자 토큰으로는 노드를 연결할 수 없습니다 — 이 노드의 관리자 화면에서 발급한 서버 토큰이 필요합니다."));
    let err = principal::authenticate_pairing(&db.state, "nwu_s_noscope", &ctx, now_ms).await.unwrap_err();
    assert_eq!((err.status, err.message.as_str()), (StatusCode::FORBIDDEN, "이 토큰에는 노드 연결(node:pair) 권한이 없습니다."));

    // Four 401s counted on the shared throttle, the two 403s not: 25 more failures do not lock
    // the address, 26 do.
    for _ in 0..MAX_IP_FAILURES - 4 - 1 {
        throttle.record_ip_failure("198.51.100.7", now_ms);
    }
    assert!(principal::check_pairing_ip_lock(&db.state, &ctx, now_ms).is_ok());
    throttle.record_ip_failure("198.51.100.7", now_ms);
    assert!(principal::check_pairing_ip_lock(&db.state, &ctx, now_ms).is_err());

    let grant = principal::authenticate_pairing(&db.state, "nwu_s_ok", &ctx, now_ms).await.unwrap();
    assert_eq!((grant.access_token_id.as_str(), grant.created_by_user_id.as_deref()), ("ok", Some("owner")));
    assert_eq!(last_used(&db.client, "access_tokens", "ok").await, (true, Some("198.51.100.7".into())));
    assert_eq!(last_used(&db.client, "access_tokens", "noscope").await, (false, None));
}
