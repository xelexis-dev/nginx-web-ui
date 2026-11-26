//! Passkey API trust boundaries and transactions, exercised against a disposable migrated DB.
mod auth_support;
use std::net::SocketAddr;
use std::time::Duration;

use auth_support::{TestDb, UserRow, audit_rows, insert_user, test_db};
use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use nwui_api::auth::{
    passkey_ceremony::{Ceremony, Registration},
    password, principal, session,
};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const PASSWORD: &str = "Passw0rd-passkey";
const PREFIX: &str = "/api/ui/v1/forms/";

fn app(db: &TestDb) -> Router {
    nwui_api::server::router(db.state.clone())
}
fn cookie(id: &str, version: i32, mfa: bool) -> String {
    let claims = json!({"id":id,"sv":version,"mfaVerified":mfa});
    let token = session::encode(
        claims.as_object().unwrap(),
        auth_support::SECRET,
        session::COOKIE_NAME_PLAIN,
        principal::now_unix_ms() / 1000,
        3600,
    )
    .unwrap();
    format!("{}={token}", session::COOKIE_NAME_PLAIN)
}
struct Reply {
    status: StatusCode,
    body: Value,
    cookies: Vec<String>,
}
impl Reply {
    fn cookie(&self, name: &str) -> String {
        self.cookies
            .iter()
            .find(|s| s.starts_with(&format!("{name}=")))
            .map(|s| s.split(';').next().unwrap().to_owned())
            .unwrap_or_default()
    }
}
async fn call(app: &Router, method: &str, path: &str, cookie: &str, form: Option<&str>) -> Reply {
    call_host(app, method, path, cookie, form, "localhost").await
}
async fn call_host(
    app: &Router,
    method: &str,
    path: &str,
    cookie: &str,
    form: Option<&str>,
    host: &str,
) -> Reply {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", host)
        .header("cookie", cookie)
        .header("x-real-ip", "203.0.113.42")
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
    if form.is_some() {
        builder = builder
            .header("origin", format!("http://{host}"))
            .header("content-type", "application/x-www-form-urlencoded");
    }
    let response = app
        .clone()
        .oneshot(
            builder
                .body(Body::from(form.unwrap_or("").to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let cookies = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    Reply {
        status,
        cookies,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}
async fn command(app: &Router, name: &str, cookie: &str, form: &str) -> Reply {
    call(app, "POST", &format!("{PREFIX}{name}"), cookie, Some(form)).await
}
fn form(fields: &[(&str, &str)]) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in fields {
        s.append_pair(k, v);
    }
    s.finish()
}
async fn user(db: &TestDb, id: &str, role: &str, hash: Option<&str>) {
    insert_user(
        &db.client,
        UserRow {
            id,
            email: &format!("{id}@example.test"),
            role,
            ..Default::default()
        },
        hash,
    )
    .await;
}
async fn stored(db: &TestDb, user: &str, id: &str) {
    db.client.execute("INSERT INTO user_passkeys(id,user_id,credential_id,public_key,algorithm,rp_id,origin,name,backup_eligible,backup_state) VALUES($1,$2,$3,$4,-7,'localhost','http://localhost',$1,true,false)",&[&id,&user,&id.as_bytes(),&&b"unused-in-list-test"[..]]).await.unwrap();
}
async fn sv(db: &TestDb, id: &str) -> i32 {
    db.client
        .query_one("SELECT session_version FROM users WHERE id=$1", &[&id])
        .await
        .unwrap()
        .get(0)
}
async fn count(db: &TestDb, id: &str) -> i64 {
    db.client
        .query_one(
            "SELECT count(*) FROM user_passkeys WHERE user_id=$1",
            &[&id],
        )
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn listing_is_owner_scoped_secret_free_and_available_even_on_an_insecure_address() {
    let db = test_db().await;
    user(&db, "owner", "editor", Some("unused")).await;
    user(&db, "other", "viewer", None).await;
    stored(&db, "owner", "one").await;
    stored(&db, "other", "two").await;
    let app = app(&db);
    assert_eq!(
        call(&app, "GET", "/api/ui/v1/account/passkeys", "", None)
            .await
            .status,
        401
    );
    let list = call(
        &app,
        "GET",
        "/api/ui/v1/account/passkeys",
        &cookie("owner", 3, false),
        None,
    )
    .await;
    assert_eq!(list.status, 200);
    assert_eq!(list.body["passkeys"].as_array().unwrap().len(), 1);
    assert_eq!(list.body["passkeys"][0]["id"], "one");
    assert_eq!(list.body["canRemove"], true);
    for secret in ["credentialId", "publicKey", "userHandle", "passwordHash"] {
        assert!(!list.body.to_string().contains(secret));
    }
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/ui/v1/account/passkeys/two",
            &cookie("owner", 3, false),
            None
        )
        .await
        .status,
        404
    );
    let unavailable = call_host(
        &app,
        "GET",
        "/api/ui/v1/account/passkeys",
        &cookie("owner", 3, false),
        None,
        "admin.example.test",
    )
    .await;
    assert_eq!(unavailable.body["availability"]["available"], false);
    assert_eq!(unavailable.body["passkeys"][0]["unavailableHere"], true);
    let options = call(&app, "GET", "/api/ui/v1/auth/options", "", None).await;
    assert_eq!(
        options.body["passkey"],
        json!({"available":true,"reason":null})
    );
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn registration_requires_password_verified_mfa_and_has_a_stable_handle_and_limit() {
    let db = test_db().await;
    let hash = password::hash_password(PASSWORD).unwrap();
    user(&db, "owner", "editor", Some(&hash)).await;
    user(&db, "oauth", "editor", None).await;
    db.client
        .execute(
            "UPDATE users SET passkey_user_handle=NULL WHERE id='owner'",
            &[],
        )
        .await
        .unwrap();
    let app = app(&db);
    let fields = form(&[("name", "My key"), ("currentPassword", PASSWORD)]);
    assert_eq!(
        command(&app, "account.passkeys.start", "", &fields)
            .await
            .status,
        401
    );
    assert_eq!(
        command(
            &app,
            "account.passkeys.start",
            &cookie("oauth", 3, false),
            &fields
        )
        .await
        .status,
        409
    );
    assert_eq!(
        command(
            &app,
            "account.passkeys.start",
            &cookie("owner", 3, false),
            "name=key&currentPassword=wrong"
        )
        .await
        .status,
        400
    );
    db.client
        .execute("UPDATE users SET totp_enabled=true WHERE id='owner'", &[])
        .await
        .unwrap();
    assert_eq!(
        command(
            &app,
            "account.passkeys.start",
            &cookie("owner", 3, false),
            &fields
        )
        .await
        .status,
        403
    );
    let first = command(
        &app,
        "account.passkeys.start",
        &cookie("owner", 3, true),
        &fields,
    )
    .await;
    assert_eq!(first.status, 200, "{}", first.body);
    let options = &first.body["result"]["publicKey"];
    assert_eq!(options["authenticatorSelection"]["residentKey"], "required");
    assert_eq!(
        options["authenticatorSelection"]["userVerification"],
        "required"
    );
    assert_eq!(
        options["pubKeyCredParams"],
        json!([{"type":"public-key","alg":-7},{"type":"public-key","alg":-257}])
    );
    let second = command(
        &app,
        "account.passkeys.start",
        &cookie("owner", 3, true),
        &fields,
    )
    .await;
    assert_eq!(
        options["user"]["id"],
        second.body["result"]["publicKey"]["user"]["id"]
    );
    assert_ne!(
        options["challenge"],
        second.body["result"]["publicKey"]["challenge"]
    );
    for i in 0..10 {
        stored(&db, "owner", &format!("key{i}")).await;
    }
    assert_eq!(
        command(
            &app,
            "account.passkeys.start",
            &cookie("owner", 3, true),
            &fields
        )
        .await
        .status,
        409
    );
    db.client
        .execute("DELETE FROM users WHERE id='owner'", &[])
        .await
        .unwrap();
    assert_eq!(count(&db, "owner").await, 0);
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn rename_and_remove_are_scoped_reauthenticated_versioned_and_audited() {
    let db = test_db().await;
    let hash = password::hash_password(PASSWORD).unwrap();
    user(&db, "owner", "editor", Some(&hash)).await;
    user(&db, "other", "editor", Some(&hash)).await;
    stored(&db, "owner", "one").await;
    stored(&db, "other", "two").await;
    let app = app(&db);
    let c = cookie("owner", 3, false);
    assert_eq!(
        command(&app, "account.passkeys.rename", &c, "id=two&name=Renamed")
            .await
            .status,
        404
    );
    assert_eq!(
        command(
            &app,
            "account.passkeys.rename",
            &c,
            &form(&[("id", "one"), ("name", &"x".repeat(65))])
        )
        .await
        .status,
        409
    );
    let renamed = command(&app, "account.passkeys.rename", &c, "id=one&name=Renamed").await;
    assert_eq!(renamed.status,200);
    assert_eq!(renamed.body["redirectTo"],"/account/passkeys/one?renamed=1");
    assert_eq!(sv(&db, "owner").await, 3);
    assert_eq!(audit_rows(&db.client, "user:passkey-rename").await.len(), 1);
    assert_eq!(
        command(
            &app,
            "account.passkeys.remove",
            &c,
            "id=one&currentPassword=wrong"
        )
        .await
        .status,
        400
    );
    assert_eq!(
        command(
            &app,
            "account.passkeys.remove",
            &c,
            &form(&[("id", "two"), ("currentPassword", PASSWORD)])
        )
        .await
        .status,
        404
    );
    let reply = command(
        &app,
        "account.passkeys.remove",
        &c,
        &form(&[("id", "one"), ("currentPassword", PASSWORD)]),
    )
    .await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body["redirectTo"], "/account?removed=1");
    assert_eq!(sv(&db, "owner").await, 4);
    assert_eq!(count(&db, "owner").await, 0);
    assert_eq!(count(&db, "other").await, 1);
    assert!(!reply.cookie(session::COOKIE_NAME_PLAIN).is_empty());
    assert_eq!(
        call(&app, "GET", "/api/ui/v1/account/passkeys", &c, None)
            .await
            .status,
        401
    );
    let audits = audit_rows(&db.client, "user:passkey-remove").await;
    assert_eq!(audits.len(), 1);
    assert_eq!(
        audits[0].get::<_, Option<String>>("target").as_deref(),
        Some("owner")
    );
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn admin_removal_requires_admin_password_and_supports_one_or_all() {
    let db = test_db().await;
    let hash = password::hash_password(PASSWORD).unwrap();
    user(&db, "admin", "admin", Some(&hash)).await;
    user(&db, "oauthadmin", "admin", None).await;
    user(&db, "target", "viewer", Some(&hash)).await;
    stored(&db, "target", "one").await;
    stored(&db, "target", "two").await;
    let app = app(&db);
    let list = call(
        &app,
        "GET",
        "/api/ui/v1/admin/users/target/passkeys",
        &cookie("oauthadmin", 3, false),
        None,
    )
    .await;
    assert_eq!(list.status, 200);
    assert_eq!(list.body["canRemove"], false);
    let form = form(&[
        ("userId", "target"),
        ("id", "one"),
        ("currentPassword", PASSWORD),
    ]);
    assert_eq!(
        command(
            &app,
            "admin.users.passkeys.remove",
            &cookie("target", 3, false),
            &form
        )
        .await
        .status,
        403
    );
    assert_eq!(
        command(
            &app,
            "admin.users.passkeys.remove",
            &cookie("oauthadmin", 3, false),
            &form
        )
        .await
        .status,
        409
    );
    for missing_user in ["aaa-missing", "zzz-missing"] {
        let missing = command(
            &app,
            "admin.users.passkeys.remove",
            &cookie("admin", 3, false),
            &format!("userId={missing_user}&all=1&currentPassword={PASSWORD}"),
        ).await;
        assert_eq!(missing.status, 404);
        assert_eq!(missing.body["error"]["code"], "not_found");
        assert_eq!(sv(&db, "admin").await, 3);
        assert!(audit_rows(&db.client, "user:passkey-remove").await.is_empty());
    }
    for missing in ["", "&id=", "&all=0", "&all=1&all=0"] {
        let refused = command(&app,"admin.users.passkeys.remove",&cookie("admin",3,false),&format!("userId=target&currentPassword={PASSWORD}{missing}")).await;
        assert_eq!(refused.status,409);
        assert_eq!(count(&db,"target").await,2);
        assert_eq!(sv(&db,"target").await,3);
    }
    let reply = command(
        &app,
        "admin.users.passkeys.remove",
        &cookie("admin", 3, false),
        &form,
    )
    .await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body["redirectTo"],"/admin/users/target?passkeysRemoved=1");
    assert_eq!(count(&db, "target").await, 1);
    assert_eq!(sv(&db, "target").await, 4);
    assert_eq!(sv(&db, "admin").await, 3);
    assert!(reply.cookies.is_empty());
    let all = command(&app,"admin.users.passkeys.remove",&cookie("admin",3,false),&format!("userId=target&all=1&currentPassword={PASSWORD}")).await;
    assert_eq!(all.status,200);
    assert_eq!(all.body["redirectTo"],"/admin/users/target?passkeysRemoved=1");
    assert_eq!(count(&db, "target").await, 0);
    assert_eq!(sv(&db, "target").await, 5);
    let rows = audit_rows(&db.client, "user:passkey-remove").await;
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].get::<_, Option<String>>("actor_id").as_deref(),
        Some("admin")
    );
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn starts_have_their_own_budget_and_failed_finishes_use_the_existing_ip_ceiling() {
    let db = test_db().await;
    let app = app(&db);
    for _ in 0..nwui_api::auth::throttle::MAX_PASSKEY_STARTS {
        let reply = command(&app, "login.passkeyStart", "", "").await;
        assert_eq!(reply.status, 200);
        assert_eq!(
            reply.body["result"]["publicKey"]["allowCredentials"],
            json!([])
        );
    }
    assert_eq!(
        command(&app, "login.passkeyStart", "", "").await.status,
        429
    );
    assert!(
        !db.state
            .runtime
            .login_throttle
            .is_ip_locked("203.0.113.42", principal::now_unix_ms())
    );
    for _ in 0..nwui_api::auth::throttle::MAX_IP_FAILURES {
        let reply = command(&app, "login.passkeyFinish", "", "credential=bad").await;
        assert_eq!(reply.body["redirectTo"], "/login?error=passkey");
        assert!(
            reply
                .cookies
                .iter()
                .any(|c| c.contains("nwui-passkey=;") && c.contains("Max-Age=0"))
        );
    }
    assert!(
        db.state
            .runtime
            .login_throttle
            .is_ip_locked("203.0.113.42", principal::now_unix_ms())
    );
    let log = std::fs::read_to_string(&db.auth_log).unwrap();
    assert!(log.contains("auth-failure"));
    assert!(log.contains("reason=bad_credentials"));
    let locked = audit_rows(&db.client, "auth:ip-locked").await;
    assert_eq!(locked.len(), 1);
    assert_eq!(locked[0].get::<_, Option<String>>("target").as_deref(), Some("203.0.113.42"));
    assert_eq!(locked[0].get::<_, Option<String>>("ip").as_deref(), Some("203.0.113.42"));
    command(&app, "login.passkeyFinish", "", "credential=bad").await;
    let log = std::fs::read_to_string(&db.auth_log).unwrap();
    assert!(log.lines().last().unwrap().ends_with("reason=ip_throttled"));
    assert_eq!(log.lines().filter(|line| line.ends_with("reason=bad_credentials")).count(), nwui_api::auth::throttle::MAX_IP_FAILURES);
    assert_eq!(audit_rows(&db.client, "auth:ip-locked").await.len(), 1);
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn failed_audit_rolls_back_removal_and_version_bump() {
    let db = test_db().await;
    let hash = password::hash_password(PASSWORD).unwrap();
    user(&db, "owner", "editor", Some(&hash)).await;
    stored(&db, "owner", "one").await;
    db.client.batch_execute("ALTER TABLE audit_logs ADD CONSTRAINT reject_passkey_audit CHECK (action <> 'user:passkey-remove')").await.unwrap();
    let reply = command(
        &app(&db),
        "account.passkeys.remove",
        &cookie("owner", 3, false),
        &format!("id=one&currentPassword={PASSWORD}"),
    )
    .await;
    assert_eq!(reply.status, 503);
    assert!(reply.cookies.is_empty());
    assert_eq!(count(&db, "owner").await, 1);
    assert_eq!(sv(&db, "owner").await, 3);
}

fn vector(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("tests/fixtures/passkey/{name}.json"));
    serde_json::from_slice(&std::fs::read(path).expect("WP1's committed Chromium vector")).unwrap()
}
fn bytes(value: &Value) -> Vec<u8> {
    URL_SAFE_NO_PAD.decode(value.as_str().unwrap()).unwrap()
}
fn fixture_host(f: &Value) -> &str {
    f["origin"]
        .as_str()
        .unwrap()
        .strip_prefix("http://")
        .unwrap()
}
fn entry(f: &Value, challenge: &Value, registration: Option<Registration>) -> Ceremony {
    Ceremony {
        rp_id: f["rpId"].as_str().unwrap().into(),
        origin: f["origin"].as_str().unwrap().into(),
        challenge: bytes(challenge).try_into().unwrap(),
        expires_ms: principal::now_unix_ms() + 300_000,
        registration,
    }
}
async fn enrollment(db: &TestDb, f: &Value, id: &str) -> String {
    let user = db
        .client
        .query_one(
            "SELECT password_hash,session_version FROM users WHERE id=$1",
            &[&id],
        )
        .await
        .unwrap();
    let handle = bytes(&f["userHandle"]);
    db.client
        .execute(
            "UPDATE users SET passkey_user_handle=$2 WHERE id=$1",
            &[&id, &handle],
        )
        .await
        .unwrap();
    let reg = Registration {
        user_id: id.into(),
        session_version: user.get(1),
        password_hash: user.get::<_, Option<String>>(0).unwrap(),
        user_handle: handle,
        name: "Browser fixture".into(),
        mfa_verified: true,
    };
    db.state
        .runtime
        .passkey_ceremonies
        .insert_fixture("registration", entry(f, &f["challenge"], Some(reg)));
    format!(
        "{}; nwui-passkey=registration",
        cookie(id, user.get(1), true)
    )
}
async fn enroll(db: &TestDb, f: &Value, id: &str) -> Reply {
    let c = enrollment(db, f, id).await;
    call_host(
        &app(db),
        "POST",
        &format!("{PREFIX}account.passkeys.finish"),
        &c,
        Some(&form(&[("credential", &f["registration"].to_string())])),
        fixture_host(f),
    )
    .await
}
fn signin_cookie(db: &TestDb, f: &Value, challenge: &Value, id: &str) -> String {
    db.state
        .runtime
        .passkey_ceremonies
        .insert_fixture(id, entry(f, challenge, None));
    format!("nwui-passkey={id}")
}
async fn finish(db: &TestDb, f: &Value, challenge: &Value, assertion: &Value, id: &str) -> Reply {
    let c = signin_cookie(db, f, challenge, id);
    call_host(
        &app(db),
        "POST",
        &format!("{PREFIX}login.passkeyFinish"),
        &c,
        Some(&form(&[("credential", &assertion.to_string())])),
        fixture_host(f),
    )
    .await
}
fn signed_in(reply: &Reply) -> bool {
    reply.body["redirectTo"] == "/" && !reply.cookie(session::COOKIE_NAME_PLAIN).is_empty()
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn chromium_es256_and_rs256_enroll_and_sign_in_without_password_or_second_factor() {
    for name in ["es256", "rs256"] {
        let db = test_db().await;
        let hash = password::hash_password(PASSWORD).unwrap();
        user(&db, "owner", "editor", Some(&hash)).await;
        db.client
            .execute("UPDATE users SET totp_enabled=true WHERE id='owner'", &[])
            .await
            .unwrap();
        let f = vector(name);
        let enrolled = enroll(&db, &f, "owner").await;
        assert_eq!(enrolled.status, 200, "{}", enrolled.body);
        assert!(enrolled.body["redirectTo"].as_str().unwrap().ends_with("?added=1"));
        assert_eq!(sv(&db, "owner").await, 4);
        let old = cookie("owner", 3, true);
        assert_eq!(
            call(&app(&db), "GET", "/api/ui/v1/account/passkeys", &old, None)
                .await
                .status,
            401
        );
        let pending = db
            .state
            .runtime
            .pending_logins
            .create(
                "owner",
                "email:owner@example.test",
                4,
                "factor".into(),
                principal::now_unix_ms(),
            )
            .unwrap();
        let c = signin_cookie(&db, &f, &f["assertionChallenge"], "assertion");
        let reply = call_host(
            &app(&db),
            "POST",
            &format!("{PREFIX}login.passkeyFinish"),
            &format!("{c}; nwui-2fa-challenge={pending}"),
            Some(&form(&[("credential", &f["assertion"].to_string())])),
            fixture_host(&f),
        )
        .await;
        assert!(signed_in(&reply), "{}", reply.body);
        assert!(
            db.state
                .runtime
                .pending_logins
                .get(&pending, principal::now_unix_ms())
                .is_none()
        );
        let cookies = session::request_cookies(&axum::http::HeaderMap::from_iter([(
            "cookie".parse().unwrap(),
            reply.cookie(session::COOKIE_NAME_PLAIN).parse().unwrap(),
        )]));
        let token = session::read_cookie(
            cookies.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            session::COOKIE_NAME_PLAIN,
        )
        .unwrap()
        .unwrap();
        let claims = session::decode(
            &token,
            &[auth_support::SECRET],
            session::COOKIE_NAME_PLAIN,
            principal::now_unix_ms() / 1000,
        )
        .unwrap();
        assert_eq!(claims["mfaVerified"], true);
        assert_eq!(claims["sv"], 4);
        let rows = audit_rows(&db.client, "login").await;
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get::<_, Option<Value>>("after").unwrap()["method"],
            "passkey"
        );
        let row=db.client.query_one("SELECT sign_count,last_used_at IS NOT NULL FROM user_passkeys WHERE user_id='owner'",&[]).await.unwrap();
        assert_eq!(row.get::<_, i64>(0), 2);
        assert!(row.get::<_, bool>(1));
        let replay = call_host(
            &app(&db),
            "POST",
            &format!("{PREFIX}login.passkeyFinish"),
            &c,
            Some(&form(&[("credential", &f["assertion"].to_string())])),
            fixture_host(&f),
        )
        .await;
        assert!(!signed_in(&replay));
    }
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn registration_finish_rechecks_version_password_handle_eligibility_and_limit() {
    let f = vector("es256");
    let hash = password::hash_password(PASSWORD).unwrap();
    for change in [
        "UPDATE users SET session_version=4 WHERE id='owner'",
        "UPDATE users SET password_hash=NULL WHERE id='owner'",
        "UPDATE users SET passkey_user_handle=NULL WHERE id='owner'",
        "UPDATE users SET enabled=false WHERE id='owner'",
        "UPDATE users SET approved=false WHERE id='owner'",
    ] {
        let db = test_db().await;
        user(&db, "owner", "editor", Some(&hash)).await;
        let c = enrollment(&db, &f, "owner").await;
        db.client.batch_execute(change).await.unwrap();
        let reply = call_host(
            &app(&db),
            "POST",
            &format!("{PREFIX}account.passkeys.finish"),
            &c,
            Some(&form(&[("credential", &f["registration"].to_string())])),
            fixture_host(&f),
        )
        .await;
        assert_ne!(reply.status, 200, "{change}");
        assert_eq!(count(&db, "owner").await, 0);
        assert!(audit_rows(&db.client, "user:passkey-add").await.is_empty());
    }
    let db = test_db().await;
    user(&db, "owner", "editor", Some(&hash)).await;
    let c = enrollment(&db, &f, "owner").await;
    for i in 0..10 {
        stored(&db, "owner", &format!("key{i}")).await;
    }
    let reply = call_host(
        &app(&db),
        "POST",
        &format!("{PREFIX}account.passkeys.finish"),
        &c,
        Some(&form(&[("credential", &f["registration"].to_string())])),
        fixture_host(&f),
    )
    .await;
    assert_eq!(reply.status, 409);
    assert_eq!(count(&db, "owner").await, 10);
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn duplicate_credential_is_global_and_registration_audit_failure_rolls_back() {
    let db = test_db().await;
    let hash = password::hash_password(PASSWORD).unwrap();
    user(&db, "owner", "editor", Some(&hash)).await;
    user(&db, "other", "editor", Some(&hash)).await;
    let f = vector("es256");
    assert_eq!(enroll(&db, &f, "owner").await.status, 200);
    // A user's handle is unique too; the credential's global uniqueness still holds with another handle.
    let mut another = f.clone();
    another["userHandle"] = json!(URL_SAFE_NO_PAD.encode([9u8; 32]));
    let reply = enroll(&db, &another, "other").await;
    assert_eq!(reply.status, 409);
    assert_eq!(sv(&db, "other").await, 3);
    assert_eq!(count(&db, "other").await, 0);
    db.client.batch_execute("DELETE FROM user_passkeys; ALTER TABLE audit_logs ADD CONSTRAINT reject_passkey_audit CHECK (action <> 'user:passkey-add') NOT VALID").await.unwrap();
    let reply = enroll(&db, &f, "owner").await;
    assert_eq!(reply.status, 503);
    assert!(reply.cookie(session::COOKIE_NAME_PLAIN).is_empty());
    assert!(reply.body.get("redirectTo").is_none());
    assert_eq!(sv(&db, "owner").await, 4);
    assert_eq!(count(&db, "owner").await, 0);
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn disabled_pending_oauth_only_and_wrong_signatures_do_not_issue_a_session() {
    let hash = password::hash_password(PASSWORD).unwrap();
    let f = vector("es256");
    for (change, redirect) in [
        (
            "UPDATE users SET enabled=false WHERE id='owner'",
            "/login?error=disabled",
        ),
        (
            "UPDATE users SET approved=false WHERE id='owner'",
            "/login?error=pending",
        ),
        (
            "UPDATE users SET password_hash=NULL WHERE id='owner'",
            "/login?error=passkey",
        ),
    ] {
        let db = test_db().await;
        user(&db, "owner", "editor", Some(&hash)).await;
        assert_eq!(enroll(&db, &f, "owner").await.status, 200);
        db.client.batch_execute(change).await.unwrap();
        let mut bad = f["assertion"].clone();
        let mut sig = bytes(&bad["response"]["signature"]);
        sig[0] ^= 1;
        bad["response"]["signature"] = json!(URL_SAFE_NO_PAD.encode(sig));
        let wrong = finish(&db, &f, &f["assertionChallenge"], &bad, "bad").await;
        assert_eq!(wrong.body["redirectTo"], "/login?error=passkey");
        let reply = finish(&db, &f, &f["assertionChallenge"], &f["assertion"], "valid").await;
        assert_eq!(reply.body["redirectTo"], redirect);
        assert!(reply.cookie(session::COOKIE_NAME_PLAIN).is_empty());
        let reason = match redirect {
            "/login?error=disabled" => "account_disabled",
            "/login?error=pending" => "account_pending",
            _ => "bad_credentials",
        };
        let log = std::fs::read_to_string(&db.auth_log).unwrap();
        assert!(log.lines().next().unwrap().ends_with("reason=bad_credentials"));
        assert!(log.lines().last().unwrap().ends_with(&format!("reason={reason}")));
        if reason != "bad_credentials" {
            for _ in 0..nwui_api::auth::throttle::MAX_FAILURES {
                let retry = finish(&db, &f, &f["assertionChallenge"], &f["assertion"], "retry").await;
                assert_eq!(retry.body["redirectTo"], redirect);
            }
            db.client.batch_execute("UPDATE users SET enabled=true, approved=true WHERE id='owner'").await.unwrap();
            assert!(signed_in(&finish(&db, &f, &f["assertionChallenge"], &f["assertion"], "enabled").await));
        } else {
            assert!(audit_rows(&db.client, "login").await.is_empty());
        }
        assert!(
            !db.state
                .runtime
                .login_throttle
                .is_account_locked("passkey:user:owner", principal::now_unix_ms())
        );
    }
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn current_version_is_used_and_policy_without_totp_confines_a_passkey_session_to_enrollment()
{
    let db = test_db().await;
    let hash = password::hash_password(PASSWORD).unwrap();
    user(&db, "owner", "editor", Some(&hash)).await;
    let f = vector("es256");
    assert_eq!(enroll(&db, &f, "owner").await.status, 200);
    let c = signin_cookie(&db, &f, &f["assertionChallenge"], "version");
    db.client.batch_execute("UPDATE users SET session_version=9 WHERE id='owner'; INSERT INTO app_settings(id,require_2fa_user) VALUES('singleton',true) ON CONFLICT(id) DO UPDATE SET require_2fa_user=true").await.unwrap();
    let reply = call_host(
        &app(&db),
        "POST",
        &format!("{PREFIX}login.passkeyFinish"),
        &c,
        Some(&form(&[("credential", &f["assertion"].to_string())])),
        fixture_host(&f),
    )
    .await;
    assert!(signed_in(&reply));
    let session = reply.cookie(session::COOKIE_NAME_PLAIN);
    let view = call(&app(&db), "GET", "/api/ui/v1/session", &session, None).await;
    assert_eq!(view.body["user"]["sv"], 9);
    assert_eq!(view.body["user"]["requires2fa"], true);
    assert_eq!(
        call(
            &app(&db),
            "GET",
            "/api/ui/v1/account/passkeys",
            &session,
            None
        )
        .await
        .status,
        403
    );
    assert_eq!(
        call(&app(&db), "GET", "/api/ui/v1/account/2fa", &session, None)
            .await
            .status,
        200
    );
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn signed_counter_six_races_under_the_credential_lock_and_accepts_only_one() {
    let db = test_db().await;
    let hash = password::hash_password(PASSWORD).unwrap();
    user(&db, "owner", "editor", Some(&hash)).await;
    let f = vector("es256");
    assert_eq!(enroll(&db, &f, "owner").await.status, 200);
    db.client
        .execute(
            "UPDATE user_passkeys SET sign_count=5 WHERE user_id='owner'",
            &[],
        )
        .await
        .unwrap();
    let a = &f["counterAssertions"][0];
    let b = &f["counterAssertions"][1];
    let c1 = signin_cookie(&db, &f, &a["challenge"], "race1");
    let c2 = signin_cookie(&db, &f, &b["challenge"], "race2");
    let form1 = form(&[("credential", &a["assertion"].to_string())]);
    let form2 = form(&[("credential", &b["assertion"].to_string())]);
    let app = app(&db);
    let path = format!("{PREFIX}login.passkeyFinish");
    let (a, b) = tokio::join!(
        call_host(&app, "POST", &path, &c1, Some(&form1), fixture_host(&f)),
        call_host(&app, "POST", &path, &c2, Some(&form2), fixture_host(&f))
    );
    assert_eq!(usize::from(signed_in(&a)) + usize::from(signed_in(&b)), 1);
    assert_eq!(audit_rows(&db.client, "login").await.len(), 1);
    assert_eq!(
        audit_rows(&db.client, "auth:passkey-counter").await.len(),
        1
    );
    assert_eq!(
        db.client
            .query_one(
                "SELECT sign_count FROM user_passkeys WHERE user_id='owner'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        6
    );
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn removed_credentials_and_expiry_while_waiting_for_a_user_lock_are_refused() {
    let db = test_db().await;
    let hash = password::hash_password(PASSWORD).unwrap();
    user(&db, "owner", "editor", Some(&hash)).await;
    let f = vector("es256");
    assert_eq!(enroll(&db, &f, "owner").await.status, 200);
    for removed in [false, true] {
        let mut blocker = db.state.db.get().await.unwrap();
        let tx = blocker.transaction().await.unwrap();
        tx.query_one("SELECT id FROM users WHERE id='owner' FOR UPDATE", &[])
            .await
            .unwrap();
        let mut e = entry(&f, &f["assertionChallenge"], None);
        if !removed {
            e.expires_ms = principal::now_unix_ms() + 5_000;
        }
        let expires_ms = e.expires_ms;
        db.state
            .runtime
            .passkey_ceremonies
            .insert_fixture("waiting", e);
        let app = app(&db);
        let body = form(&[("credential", &f["assertion"].to_string())]);
        let host = fixture_host(&f).to_owned();
        let waiting = tokio::spawn(async move {
            call_host(
                &app,
                "POST",
                &format!("{PREFIX}login.passkeyFinish"),
                "nwui-passkey=waiting",
                Some(&body),
                &host,
            )
            .await
        });
        let pid: i32 = tx
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .unwrap()
            .get(0);
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let blocked: bool = db
                    .client
                    .query_one(
                        "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)))",
                        &[&pid],
                    )
                    .await
                    .unwrap()
                    .get(0);
                if blocked {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("finish reached the user row lock before the mutation");
        if !removed {
            let remaining_ms = (expires_ms + 1 - principal::now_unix_ms()).max(0) as u64;
            tokio::time::sleep(Duration::from_millis(remaining_ms)).await;
        }
        if removed {
            tx.execute("DELETE FROM user_passkeys WHERE user_id='owner'", &[])
                .await
                .unwrap();
        }
        tx.commit().await.unwrap();
        assert!(!signed_in(&waiting.await.unwrap()));
    }
    assert!(audit_rows(&db.client, "login").await.is_empty());
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn login_audit_failure_rolls_back_counter_and_preserves_the_user_throttle_on_failure() {
    let db = test_db().await;
    let hash = password::hash_password(PASSWORD).unwrap();
    user(&db, "owner", "editor", Some(&hash)).await;
    let f = vector("es256");
    assert_eq!(enroll(&db, &f, "owner").await.status, 200);
    db.client
        .batch_execute(
            "ALTER TABLE audit_logs ADD CONSTRAINT reject_login CHECK(action <> 'login')",
        )
        .await
        .unwrap();
    assert!(!signed_in(
        &finish(&db, &f, &f["assertionChallenge"], &f["assertion"], "fail").await
    ));
    let row = db
        .client
        .query_one(
            "SELECT sign_count,last_used_at FROM user_passkeys WHERE user_id='owner'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, i64>(0), 1);
    assert!(
        row.get::<_, Option<chrono::DateTime<chrono::Utc>>>(1)
            .is_none()
    );
    db.client
        .batch_execute("ALTER TABLE audit_logs DROP CONSTRAINT reject_login")
        .await
        .unwrap();
    db.state
        .runtime
        .login_throttle
        .record_ip_failure("203.0.113.42", principal::now_unix_ms());
    assert!(signed_in(
        &finish(
            &db,
            &f,
            &f["assertionChallenge"],
            &f["assertion"],
            "success"
        )
        .await
    ));
    for _ in 0..9 {
        db.state
            .runtime
            .login_throttle
            .record_account_failure("passkey:user:owner", principal::now_unix_ms());
    }
    assert!(
        !db.state
            .runtime
            .login_throttle
            .is_account_locked("passkey:user:owner", principal::now_unix_ms()),
        "success cleared prior user failures"
    );
    for _ in 0..27 {
        db.state
            .runtime
            .login_throttle
            .record_ip_failure("203.0.113.42", principal::now_unix_ms());
    }
    assert!(
        !db.state
            .runtime
            .login_throttle
            .is_ip_locked("203.0.113.42", principal::now_unix_ms())
    );
    db.state
        .runtime
        .login_throttle
        .record_ip_failure("203.0.113.42", principal::now_unix_ms());
    assert!(
        db.state
            .runtime
            .login_throttle
            .is_ip_locked("203.0.113.42", principal::now_unix_ms()),
        "success did not forgive earlier address failures"
    );
}

/// These generated signatures exercise only the DB counter transaction. The verifier's positive
/// vectors and the enrollment/sign-in tests above use WP1's captured Chromium credentials.
#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn zero_counters_and_nonzero_replays_follow_the_rule_under_concurrent_finishes() {
    use aws_lc_rs::{
        rand::SystemRandom,
        signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair},
    };
    use sha2::Digest as _;
    use rcgen::PublicKeyData as _;
    let db = test_db().await;
    user(&db, "owner", "editor", Some("password-present")).await;
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let signer =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &key.serialize_der()).unwrap();
    let handle = vec![8u8; 32];
    let credential = vec![42u8; 32];
    db.client
        .execute(
            "UPDATE users SET passkey_user_handle=$1 WHERE id='owner'",
            &[&handle],
        )
        .await
        .unwrap();
    db.client.execute("INSERT INTO user_passkeys(id,user_id,credential_id,public_key,algorithm,rp_id,origin,name,backup_eligible,backup_state) VALUES('zero','owner',$1,$2,-7,'localhost','http://localhost','Synced key',true,true)",&[&credential,&key.subject_public_key_info()]).await.unwrap();
    let sign = |counter: u32, challenge: u8| {
        let challenge = vec![challenge; 32];
        let client=json!({"type":"webauthn.get","challenge":URL_SAFE_NO_PAD.encode(&challenge),"origin":"http://localhost","crossOrigin":false}).to_string();
        let mut auth_data = sha2::Sha256::digest(b"localhost").to_vec();
        auth_data.push(0x1d);
        auth_data.extend_from_slice(&counter.to_be_bytes());
        let mut message = auth_data.clone();
        message.extend_from_slice(&sha2::Sha256::digest(client.as_bytes()));
        let signature = signer.sign(&SystemRandom::new(), &message).unwrap();
        let assertion = json!({"type":"public-key","id":URL_SAFE_NO_PAD.encode(&credential),"rawId":URL_SAFE_NO_PAD.encode(&credential),"response":{"clientDataJSON":URL_SAFE_NO_PAD.encode(client),"authenticatorData":URL_SAFE_NO_PAD.encode(auth_data),"signature":URL_SAFE_NO_PAD.encode(signature.as_ref()),"userHandle":URL_SAFE_NO_PAD.encode(&handle)}});
        let f = json!({"rpId":"localhost","origin":"http://localhost"});
        (f, json!(URL_SAFE_NO_PAD.encode(challenge)), assertion)
    };
    for (stored, received, accept) in [
        (10i64, 0u32, false),
        (5, 5, false),
        (0, 0, true),
        (0, 1, true),
    ] {
        db.client
            .execute(
                "UPDATE user_passkeys SET sign_count=$1 WHERE id='zero'",
                &[&stored],
            )
            .await
            .unwrap();
        let (f, challenge, assertion) = sign(received, 1);
        assert_eq!(
            signed_in(&finish(&db, &f, &challenge, &assertion, "counter").await),
            accept,
            "{stored} -> {received}"
        );
        assert_eq!(
            db.client
                .query_one("SELECT sign_count FROM user_passkeys WHERE id='zero'", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            if accept { i64::from(received) } else { stored }
        );
    }
    db.client
        .execute("UPDATE user_passkeys SET sign_count=0 WHERE id='zero'", &[])
        .await
        .unwrap();
    let (f, challenge, a) = sign(0, 2);
    let (_, challenge_b, b) = sign(0, 3);
    let (a, b) = tokio::join!(
        finish(&db, &f, &challenge, &a, "zero-a"),
        finish(&db, &f, &challenge_b, &b, "zero-b")
    );
    assert!(
        signed_in(&a) && signed_in(&b),
        "two synced-key ceremonies at 0 -> 0 both succeed"
    );
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn account_and_admin_removal_share_the_registration_reauth_limit_on_the_actor() {
    let hash = password::hash_password(PASSWORD).unwrap();
    for admin in [false, true] {
        let db = test_db().await;
        user(
            &db,
            "actor",
            if admin { "admin" } else { "editor" },
            Some(&hash),
        )
        .await;
        let target = if admin { "target" } else { "actor" };
        if admin {
            user(&db, target, "viewer", Some(&hash)).await;
        }
        stored(&db, target, "one").await;
        if admin { db.client.execute("UPDATE users SET totp_enabled=true WHERE id=$1", &[&target]).await.unwrap(); }
        let app = app(&db);
        let c = cookie("actor", 3, false);
        let command_name = if admin {
            "admin.users.passkeys.remove"
        } else {
            "account.passkeys.remove"
        };
        for attempt in 0..nwui_api::auth::throttle::MAX_FAILURES {
            let reset = admin && attempt % 2 == 0;
            let reply = command(
                &app,
                if reset { "admin.users.resetTwoFactor" } else { command_name },
                &c,
                &form(&[
                    ("id", if reset { target } else { "one" }),
                    ("userId", target),
                    ("currentPassword", "wrong"),
                ]),
            )
            .await;
            assert_eq!(reply.status, 400, "{}", reply.body);
        }
        assert!(
            db.state
                .runtime
                .login_throttle
                .is_account_locked("reauth:user:actor", principal::now_unix_ms())
        );
        if admin {
            assert!(
                !db.state
                    .runtime
                    .login_throttle
                    .is_account_locked("reauth:user:target", principal::now_unix_ms())
            );
        }
        let correct = form(&[
            ("id", "one"),
            ("userId", target),
            ("currentPassword", PASSWORD),
        ]);
        assert_eq!(command(&app, command_name, &c, &correct).await.status, 429);
        if admin { assert_eq!(command(&app,"admin.users.resetTwoFactor",&c,&form(&[("id",target),("currentPassword",PASSWORD)])).await.status,429); }
        assert_eq!(
            command(
                &app,
                "account.passkeys.start",
                &c,
                &form(&[("name", "Key"), ("currentPassword", PASSWORD)])
            )
            .await
            .status,
            429,
            "registration uses the same actor counter"
        );
        assert_eq!(count(&db, target).await, 1);
        assert_eq!(sv(&db, target).await, 3);
        assert!(
            audit_rows(&db.client, "user:passkey-remove")
                .await
                .is_empty()
        );
    }
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn failed_assertions_limit_the_found_user_without_blocking_another_users_passkey() {
    let db = test_db().await;
    let hash = password::hash_password(PASSWORD).unwrap();
    user(&db,"owner","editor",Some(&hash)).await;
    user(&db,"other","editor",Some(&hash)).await;
    let f = vector("es256");
    assert_eq!(enroll(&db,&f,"owner").await.status,200);
    let mut bad = f["assertion"].clone();
    let mut signature = bytes(&bad["response"]["signature"]);
    signature[0] ^= 1;
    bad["response"]["signature"] = json!(URL_SAFE_NO_PAD.encode(signature));
    for _ in 0..nwui_api::auth::throttle::MAX_FAILURES {
        assert!(!signed_in(&finish(&db,&f,&f["assertionChallenge"],&bad,"bad-signature").await));
    }
    assert!(db.state.runtime.login_throttle.is_account_locked("passkey:user:owner",principal::now_unix_ms()));
    assert!(!db.state.runtime.login_throttle.is_ip_locked("203.0.113.42",principal::now_unix_ms()));
    assert!(!signed_in(&finish(&db,&f,&f["assertionChallenge"],&f["assertion"],"blocked-owner").await));
    let other = vector("rs256");
    assert_eq!(enroll(&db,&other,"other").await.status,200);
    assert!(signed_in(&finish(&db,&other,&other["assertionChallenge"],&other["assertion"],"other-user").await));
    let rows=audit_rows(&db.client,"login").await;
    assert_eq!(rows.len(),1);
    assert_eq!(rows[0].get::<_,Option<String>>("actor_id").as_deref(),Some("other"));
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn registration_finish_neither_uses_nor_changes_signin_throttles_or_failure_log() {
    use nwui_api::auth::throttle::{MAX_FAILURES, MAX_IP_FAILURES};
    let db = test_db().await;
    let hash = password::hash_password(PASSWORD).unwrap();
    user(&db, "owner", "editor", Some(&hash)).await;
    let f = vector("es256");
    let throttle = &db.state.runtime.login_throttle;
    for _ in 0..MAX_FAILURES - 1 {
        throttle.begin_account("passkey:user:owner", principal::now_unix_ms()).unwrap().keep();
    }
    for _ in 0..MAX_IP_FAILURES - 1 {
        throttle.begin_ip("203.0.113.42", principal::now_unix_ms()).unwrap().keep();
    }
    for _ in 0..MAX_FAILURES {
        let c = enrollment(&db, &f, "owner").await;
        let refused = call_host(&app(&db), "POST", &format!("{PREFIX}account.passkeys.finish"), &c, Some("credential=bad"), fixture_host(&f)).await;
        assert_eq!(refused.status, 409);
        assert!(refused.cookies.iter().any(|c| c.contains("nwui-passkey=;") && c.contains("Max-Age=0")));
    }
    assert!(!throttle.is_account_locked("passkey:user:owner", principal::now_unix_ms()));
    assert!(!throttle.is_ip_locked("203.0.113.42", principal::now_unix_ms()));
    assert!(!db.auth_log.exists());
    assert_eq!(count(&db, "owner").await, 0);
    assert_eq!(sv(&db, "owner").await, 3);
    throttle.begin_account("passkey:user:owner", principal::now_unix_ms()).unwrap().keep();
    throttle.begin_ip("203.0.113.42", principal::now_unix_ms()).unwrap().keep();
    assert_eq!(enroll(&db, &f, "owner").await.status, 200, "sign-in locks cannot block authenticated enrollment");
    assert!(throttle.is_account_locked("passkey:user:owner", principal::now_unix_ms()), "enrollment does not forgive failed sign-ins");
    assert!(throttle.is_ip_locked("203.0.113.42", principal::now_unix_ms()));
    assert!(!db.auth_log.exists());
    assert!(audit_rows(&db.client, "auth:ip-locked").await.is_empty());
}

async fn two_factor_owner(db: &TestDb) {
    let hash = password::hash_password(PASSWORD).unwrap();
    user(db, "owner", "editor", Some(&hash)).await;
    let secret = db.state.secrets.seal(&nwui_api::auth::totp::generate_secret()).unwrap();
    db.client.execute("UPDATE users SET totp_enabled=true,totp_secret=$1,backup_codes=$2,totp_last_step=123,passkey_user_handle=$3 WHERE id='owner'", &[&secret, &json!(["backup-before"]), &vec![42_u8;32]]).await.unwrap();
}
async fn disable_state(db: &TestDb) -> Value {
    let row = db.client.query_one("SELECT totp_enabled,totp_secret,backup_codes,totp_last_step,session_version,passkey_user_handle FROM users WHERE id='owner'", &[]).await.unwrap();
    json!({"enabled":row.get::<_,bool>(0),"secret":row.get::<_,Option<String>>(1),"backups":row.get::<_,Option<Value>>(2),"step":row.get::<_,Option<i64>>(3),"sv":row.get::<_,i32>(4),"handle":row.get::<_,Option<Vec<u8>>>(5),"passkeys":count(db,"owner").await})
}
async fn disable(db: &TestDb, version: i32) -> Reply {
    command(&app(db), "account.disableTwoFactor", &cookie("owner", version, true), &form(&[("currentPassword", PASSWORD)])).await
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn two_factor_disable_removes_every_own_passkey_with_one_version_bump() {
    let db = test_db().await;
    two_factor_owner(&db).await;
    for id in ["one", "two", "other-origin"] { stored(&db,"owner",id).await; }
    db.client.execute("UPDATE user_passkeys SET origin='https://other.example.test',rp_id='other.example.test' WHERE id='other-origin'",&[]).await.unwrap();
    user(&db,"other","editor",Some("unused")).await;
    stored(&db,"other","other-key").await;
    let before = disable_state(&db).await;
    let reply = disable(&db,3).await;
    assert_eq!(reply.status,200);
    assert_eq!(reply.body["redirectTo"],"/account/2fa");
    let after = disable_state(&db).await;
    assert_eq!(after["passkeys"],0);
    assert_eq!(count(&db,"other").await,1);
    assert_eq!(after["sv"],4);
    assert_eq!(after["enabled"],false);
    for key in ["secret","backups","step"] { assert!(after[key].is_null()); }
    assert_eq!(after["handle"],before["handle"]);
    let disabled = audit_rows(&db.client,"user:2fa-disable").await;
    assert_eq!(disabled.len(),1);
    assert_eq!(disabled[0].get::<_,Option<String>>("actor_id").as_deref(),Some("owner"));
    assert!(disabled[0].get::<_,Option<String>>("target").is_none());
    assert!(disabled[0].get::<_,Option<Value>>("after").is_none());
    let removed = audit_rows(&db.client,"user:passkey-remove").await;
    assert_eq!(removed.len(),1);
    assert_eq!(removed[0].get::<_,Option<String>>("actor_id").as_deref(),Some("owner"));
    assert_eq!(removed[0].get::<_,Option<String>>("target").as_deref(),Some("owner"));
    assert_eq!(removed[0].get::<_,Option<Value>>("after"),Some(json!({"passkeyId":null,"count":3,"reason":"2fa-disable"})));
    let c = reply.cookie(session::COOKIE_NAME_PLAIN);
    let claims = session::decode(c.split_once('=').unwrap().1,&[auth_support::SECRET],session::COOKIE_NAME_PLAIN,principal::now_unix_ms()/1000).unwrap();
    assert_eq!(claims["sv"],4);
    assert_eq!(claims["totpEnabled"],false);
    assert_eq!(claims["mfaVerified"],false);
    assert_eq!(call(&app(&db),"GET","/api/ui/v1/account",&cookie("owner",3,true),None).await.status,401);
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn two_factor_disable_without_passkeys_writes_no_passkey_audit() {
    let db=test_db().await; two_factor_owner(&db).await;
    assert_eq!(disable(&db,3).await.status,200);
    assert_eq!(sv(&db,"owner").await,4);
    assert!(audit_rows(&db.client,"user:passkey-remove").await.is_empty());
    assert_eq!(audit_rows(&db.client,"user:2fa-disable").await.len(),1);
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn two_factor_disable_refuses_an_account_without_totp_and_keeps_its_passkeys() {
    let db=test_db().await; two_factor_owner(&db).await; stored(&db,"owner","one").await;
    db.client.execute("UPDATE users SET totp_enabled=false WHERE id='owner'",&[]).await.unwrap();
    let before=disable_state(&db).await;
    let reply=disable(&db,3).await;
    assert_eq!(reply.status,409);
    assert_eq!(reply.body["error"]["message"],"2단계 인증이 설정되어 있지 않습니다.");
    assert_eq!(disable_state(&db).await,before);
    assert!(audit_rows(&db.client,"user:passkey-remove").await.is_empty());
    assert!(audit_rows(&db.client,"user:2fa-disable").await.is_empty());
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn two_factor_disable_wrong_password_keeps_passkeys_and_totp() {
    let db=test_db().await; two_factor_owner(&db).await; stored(&db,"owner","one").await;
    let before=disable_state(&db).await;
    let reply=command(&app(&db),"account.disableTwoFactor",&cookie("owner",3,true),"currentPassword=wrong").await;
    assert_eq!(reply.status,400);
    assert_eq!(disable_state(&db).await,before);
    assert!(audit_rows(&db.client,"user:passkey-remove").await.is_empty());
    assert!(audit_rows(&db.client,"user:2fa-disable").await.is_empty());
}

async fn barrier_request(app: Router, action: &str, cookie: String, body: String, host: &str) -> tokio::task::JoinHandle<Reply> {
    let ready=std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let started=ready.clone(); let path=format!("{PREFIX}{action}"); let host=host.to_owned();
    let request=tokio::spawn(async move { started.wait().await; call_host(&app,"POST",&path,&cookie,Some(&body),&host).await });
    ready.wait().await;
    request
}
async fn wait_for_blocked_requests(db: &TestDb, blocker_pid: i32, count: i64) {
    tokio::time::timeout(Duration::from_secs(20),async {
        loop {
            let waiting:i64=db.client.query_one("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND pid<>$1 AND cardinality(pg_blocking_pids(pid))>0",&[&blocker_pid]).await.unwrap().get(0);
            if waiting>=count { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("requests must reach the user row lock before mutation; no timing sleeps");
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn two_factor_disable_rechecks_the_account_under_the_user_lock() {
    for version_change in [true,false] {
        let db=test_db().await; two_factor_owner(&db).await; stored(&db,"owner","one").await;
        let mut before=disable_state(&db).await;
        let mut client=db.state.db.get().await.unwrap(); let tx=client.transaction().await.unwrap();
        tx.query_one("SELECT id FROM users WHERE id='owner' FOR UPDATE",&[]).await.unwrap();
        let pid=tx.query_one("SELECT pg_backend_pid()",&[]).await.unwrap().get(0);
        let waiting=barrier_request(app(&db),"account.disableTwoFactor",cookie("owner",3,true),form(&[("currentPassword",PASSWORD)]),"localhost").await;
        wait_for_blocked_requests(&db,pid,1).await;
        if version_change {
            tx.execute("UPDATE users SET session_version=session_version+1 WHERE id='owner'",&[]).await.unwrap();
            before["sv"]=4.into();
        } else {
            let hash=password::hash_password("Different-passw0rd").unwrap();
            tx.execute("UPDATE users SET password_hash=$1 WHERE id='owner'",&[&hash]).await.unwrap();
        }
        tx.commit().await.unwrap();
        let reply=waiting.await.unwrap(); assert_eq!(reply.status,409); assert!(reply.cookies.is_empty());
        assert_eq!(disable_state(&db).await,before);
        assert!(audit_rows(&db.client,"user:passkey-remove").await.is_empty());
        assert!(audit_rows(&db.client,"user:2fa-disable").await.is_empty());
    }
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn two_factor_disable_audit_failure_rolls_back_passkeys_totp_and_version() {
    for action in ["user:passkey-remove","user:2fa-disable"] {
        let db=test_db().await; two_factor_owner(&db).await; stored(&db,"owner","one").await;
        let before=disable_state(&db).await;
        db.client.batch_execute(&format!("ALTER TABLE audit_logs ADD CONSTRAINT reject_disable_audit CHECK (action <> '{action}')")).await.unwrap();
        let reply=disable(&db,3).await; assert_eq!(reply.status,503); assert!(reply.cookies.is_empty());
        assert_eq!(disable_state(&db).await,before);
        assert!(audit_rows(&db.client,"user:passkey-remove").await.is_empty());
        assert!(audit_rows(&db.client,"user:2fa-disable").await.is_empty());
    }
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn registration_started_before_two_factor_disable_cannot_finish() {
    for old_session in [false,true] {
        let db=test_db().await; two_factor_owner(&db).await;
        let f=vector("es256"); let pending=enrollment(&db,&f,"owner").await;
        let reply=disable(&db,3).await; assert_eq!(reply.status,200);
        let c=if old_session {pending} else {format!("{}; nwui-passkey=registration",reply.cookie(session::COOKIE_NAME_PLAIN))};
        let finish=call_host(&app(&db),"POST",&format!("{PREFIX}account.passkeys.finish"),&c,Some(&form(&[("credential",&f["registration"].to_string())])),fixture_host(&f)).await;
        assert_eq!(finish.status,if old_session {401} else {409});
        assert_eq!(count(&db,"owner").await,0);
        assert!(audit_rows(&db.client,"user:passkey-add").await.is_empty());
    }
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn passkey_signin_started_before_two_factor_disable_is_refused() {
    let db=test_db().await; two_factor_owner(&db).await;
    let f=vector("es256"); assert_eq!(enroll(&db,&f,"owner").await.status,200);
    let c=signin_cookie(&db,&f,&f["assertionChallenge"],"before-disable");
    assert_eq!(disable(&db,4).await.status,200);
    let reply=call_host(&app(&db),"POST",&format!("{PREFIX}login.passkeyFinish"),&c,Some(&form(&[("credential",&f["assertion"].to_string())])),fixture_host(&f)).await;
    assert!(!signed_in(&reply)); assert_eq!(reply.body["redirectTo"],"/login?error=passkey");
    assert!(audit_rows(&db.client,"login").await.is_empty());
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn disable_2fa_does_not_starve_one_connection_pool() {
    let db=auth_support::test_db_with(&[("API_DB_POOL_SIZE","1")]).await;
    two_factor_owner(&db).await; stored(&db,"owner","one").await;
    let reply=tokio::time::timeout(Duration::from_secs(30),disable(&db,3)).await.expect("disable must not borrow a second pool client for password or audit");
    assert_eq!(reply.status,200); assert_eq!(count(&db,"owner").await,0);
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn two_factor_disable_and_registration_finish_follow_the_user_lock() {
    let db=test_db().await; two_factor_owner(&db).await; stored(&db,"owner","one").await;
    let f=vector("es256"); let pending=enrollment(&db,&f,"owner").await;
    let mut client=db.state.db.get().await.unwrap(); let tx=client.transaction().await.unwrap();
    tx.query_one("SELECT id FROM users WHERE id='owner' FOR UPDATE",&[]).await.unwrap();
    let pid=tx.query_one("SELECT pg_backend_pid()",&[]).await.unwrap().get(0);
    let disabling=barrier_request(app(&db),"account.disableTwoFactor",cookie("owner",3,true),form(&[("currentPassword",PASSWORD)]),"localhost").await;
    wait_for_blocked_requests(&db,pid,1).await;
    let finishing=barrier_request(app(&db),"account.passkeys.finish",pending,form(&[("credential",&f["registration"].to_string())]),fixture_host(&f)).await;
    wait_for_blocked_requests(&db,pid,2).await; tx.commit().await.unwrap();
    assert_eq!(disabling.await.unwrap().status,200);
    assert_eq!(finishing.await.unwrap().status,409);
    assert_eq!(count(&db,"owner").await,0);
    assert!(audit_rows(&db.client,"user:passkey-add").await.is_empty());
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn two_factor_disable_and_passkey_signin_follow_the_user_lock() {
    let db=test_db().await; two_factor_owner(&db).await; let f=vector("es256");
    assert_eq!(enroll(&db,&f,"owner").await.status,200);
    let c=signin_cookie(&db,&f,&f["assertionChallenge"],"waiting-disable");
    let mut client=db.state.db.get().await.unwrap(); let tx=client.transaction().await.unwrap();
    tx.query_one("SELECT id FROM users WHERE id='owner' FOR UPDATE",&[]).await.unwrap();
    let pid=tx.query_one("SELECT pg_backend_pid()",&[]).await.unwrap().get(0);
    let disabling=barrier_request(app(&db),"account.disableTwoFactor",cookie("owner",4,true),form(&[("currentPassword",PASSWORD)]),"localhost").await;
    wait_for_blocked_requests(&db,pid,1).await;
    let signing=barrier_request(app(&db),"login.passkeyFinish",c,form(&[("credential",&f["assertion"].to_string())]),fixture_host(&f)).await;
    wait_for_blocked_requests(&db,pid,2).await; tx.commit().await.unwrap();
    assert_eq!(disabling.await.unwrap().status,200);
    assert!(!signed_in(&signing.await.unwrap()));
    assert_eq!(count(&db,"owner").await,0);
    assert!(audit_rows(&db.client,"login").await.is_empty());
}

async fn reset_admin(db: &TestDb) {
    let hash = password::hash_password(PASSWORD).unwrap();
    user(db, "admin", "admin", Some(&hash)).await;
}
async fn reset_factor(db: &TestDb) -> Reply {
    command(&app(db), "admin.users.resetTwoFactor", &cookie("admin", 3, true), &form(&[("id", "owner"), ("currentPassword", PASSWORD)])).await
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn admin_two_factor_reset_clears_factors_once_without_changing_actor_or_cookie() {
    let db = test_db().await; two_factor_owner(&db).await; reset_admin(&db).await;
    stored(&db,"owner","one").await; stored(&db,"owner","two").await; stored(&db,"admin","admin-key").await;
    let before = disable_state(&db).await;
    let reply = reset_factor(&db).await;
    assert_eq!(reply.status,200); assert_eq!(reply.body["redirectTo"],"/admin/users/owner?twoFactorReset=1"); assert!(reply.cookies.is_empty());
    let after=disable_state(&db).await;
    assert_eq!(after["enabled"],false); assert_eq!(after["sv"],4); assert_eq!(after["passkeys"],0); assert_eq!(after["handle"],before["handle"]);
    for key in ["secret","backups","step"] { assert!(after[key].is_null()); }
    assert_eq!(sv(&db,"admin").await,3); assert_eq!(count(&db,"admin").await,1);
    for (action,before,after) in [("user:passkey-remove",None, json!({"passkeyId":null,"count":2,"reason":"2fa-reset"})),("user:2fa-reset",Some(json!({"totpEnabled":true})),json!({"totpEnabled":false}))] {
        let rows=audit_rows(&db.client,action).await; assert_eq!(rows.len(),1);
        assert_eq!(rows[0].get::<_,Option<String>>("actor_id").as_deref(),Some("admin"));
        assert_eq!(rows[0].get::<_,Option<String>>("target").as_deref(),Some("owner"));
        assert_eq!(rows[0].get::<_,Option<Value>>("before"),before);
        assert_eq!(rows[0].get::<_,Option<Value>>("after"),Some(after));
        assert_eq!(rows[0].get::<_,Option<String>>("ip").as_deref(),Some("203.0.113.42"));
    }
    assert_eq!(call(&app(&db),"GET","/api/ui/v1/account",&cookie("owner",3,true),None).await.status,401);
    assert_eq!(call(&app(&db),"GET","/api/ui/v1/account",&cookie("admin",3,true),None).await.status,200);
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn admin_two_factor_reset_without_passkeys_has_no_removal_audit_and_allows_disabled_targets() {
    let db=test_db().await; two_factor_owner(&db).await; reset_admin(&db).await;
    db.client.execute("UPDATE users SET enabled=false WHERE id='owner'",&[]).await.unwrap();
    assert_eq!(reset_factor(&db).await.status,200);
    assert_eq!(sv(&db,"owner").await,4);
    assert!(audit_rows(&db.client,"user:passkey-remove").await.is_empty());
    assert_eq!(audit_rows(&db.client,"user:2fa-reset").await.len(),1);
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn admin_two_factor_reset_refusals_leave_factors_and_audits_unchanged() {
    for (case,status) in [("off",409),("self",409),("empty-id",404),("unknown-id",404),("empty-password",400),("wrong-password",400),("oidc-admin",409),("viewer",403),("editor",403),("unverified",403),("unapproved",409)] {
        let db=test_db().await; two_factor_owner(&db).await; reset_admin(&db).await; stored(&db,"owner","one").await;
        match case {
            "off" => { db.client.execute("UPDATE users SET totp_enabled=false WHERE id='owner'",&[]).await.unwrap(); }
            "unapproved" => { db.client.execute("UPDATE users SET totp_enabled=false, approved=false WHERE id='owner'",&[]).await.unwrap(); }
            "oidc-admin" => { db.client.execute("UPDATE users SET password_hash=NULL WHERE id='admin'",&[]).await.unwrap(); }
            "viewer" | "editor" => { db.client.execute("UPDATE users SET role=$1::text::role WHERE id='admin'",&[&case]).await.unwrap(); }
            "unverified" => { db.client.execute("UPDATE users SET totp_enabled=true WHERE id='admin'",&[]).await.unwrap(); }
            _ => {}
        }
        let before=disable_state(&db).await;
        let id=match case { "self"=>"admin", "empty-id"=>"", "unknown-id"=>"missing", _=>"owner" };
        let password=match case { "empty-password"=>"", "wrong-password"=>"wrong", _=>PASSWORD };
        let reply=command(&app(&db),"admin.users.resetTwoFactor",&cookie("admin",3,case!="unverified"),&form(&[("id",id),("currentPassword",password)])).await;
        assert_eq!(reply.status,status,"{case}: {}",reply.body); assert!(reply.cookies.is_empty());
        assert_eq!(disable_state(&db).await,before,"{case}");
        assert!(audit_rows(&db.client,"user:2fa-reset").await.is_empty()); assert!(audit_rows(&db.client,"user:passkey-remove").await.is_empty());
    }
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn admin_two_factor_reset_rolls_back_when_either_audit_fails() {
    for action in ["user:passkey-remove","user:2fa-reset"] {
        let db=test_db().await; two_factor_owner(&db).await; reset_admin(&db).await; stored(&db,"owner","one").await;
        let before=disable_state(&db).await;
        db.client.batch_execute(&format!("ALTER TABLE audit_logs ADD CONSTRAINT reject_reset_audit CHECK (action <> '{action}')")).await.unwrap();
        let reply=reset_factor(&db).await; assert_eq!(reply.status,503); assert!(reply.cookies.is_empty());
        assert_eq!(disable_state(&db).await,before); assert_eq!(sv(&db,"admin").await,3);
        assert!(audit_rows(&db.client,"user:passkey-remove").await.is_empty()); assert!(audit_rows(&db.client,"user:2fa-reset").await.is_empty());
    }
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn admin_two_factor_reset_does_not_starve_one_connection_pool() {
    let db=auth_support::test_db_with(&[("API_DB_POOL_SIZE","1")]).await;
    two_factor_owner(&db).await; reset_admin(&db).await; stored(&db,"owner","one").await;
    assert_eq!(tokio::time::timeout(Duration::from_secs(30),reset_factor(&db)).await.expect("reset must use its transaction client for every audit").status,200);
    assert_eq!(count(&db,"owner").await,0);
}

async fn reset_disable_race(reset_first: bool) {
    let db=test_db().await; two_factor_owner(&db).await; reset_admin(&db).await; stored(&db,"owner","one").await;
    let mut client=db.state.db.get().await.unwrap(); let tx=client.transaction().await.unwrap();
    tx.query_one("SELECT id FROM users WHERE id='owner' FOR UPDATE",&[]).await.unwrap();
    let pid=tx.query_one("SELECT pg_backend_pid()",&[]).await.unwrap().get(0);
    let (first,second)=if reset_first {("admin.users.resetTwoFactor","account.disableTwoFactor")} else {("account.disableTwoFactor","admin.users.resetTwoFactor")};
    let credentials=|action:&str| if action.starts_with("admin.") {(cookie("admin",3,true),form(&[("id","owner"),("currentPassword",PASSWORD)]))} else {(cookie("owner",3,true),form(&[("currentPassword",PASSWORD)]))};
    let (c,b)=credentials(first); let a=barrier_request(app(&db),first,c,b,"localhost").await;
    wait_for_blocked_requests(&db,pid,1).await;
    let (c,b)=credentials(second); let b=barrier_request(app(&db),second,c,b,"localhost").await;
    wait_for_blocked_requests(&db,pid,2).await; tx.commit().await.unwrap();
    assert_eq!(a.await.unwrap().status,200); let refused=b.await.unwrap(); assert_eq!(refused.status,409); assert!(refused.cookies.is_empty());
    assert_eq!(sv(&db,"owner").await,4); assert_eq!(count(&db,"owner").await,0);
    assert_eq!(audit_rows(&db.client,"user:2fa-reset").await.len(),usize::from(reset_first));
    assert_eq!(audit_rows(&db.client,"user:2fa-disable").await.len(),usize::from(!reset_first));
}
#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn admin_two_factor_reset_wins_before_self_disable() { reset_disable_race(true).await; }
#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn self_disable_wins_before_admin_two_factor_reset() { reset_disable_race(false).await; }

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn admin_two_factor_reset_blocks_inflight_registration_finish() {
    let db=test_db().await; two_factor_owner(&db).await; reset_admin(&db).await;
    let f=vector("es256"); let pending=enrollment(&db,&f,"owner").await;
    let mut client=db.state.db.get().await.unwrap(); let tx=client.transaction().await.unwrap();
    tx.query_one("SELECT id FROM users WHERE id='owner' FOR UPDATE",&[]).await.unwrap();
    let pid=tx.query_one("SELECT pg_backend_pid()",&[]).await.unwrap().get(0);
    let resetting=barrier_request(app(&db),"admin.users.resetTwoFactor",cookie("admin",3,true),form(&[("id","owner"),("currentPassword",PASSWORD)]),"localhost").await;
    wait_for_blocked_requests(&db,pid,1).await;
    let finishing=barrier_request(app(&db),"account.passkeys.finish",pending,form(&[("credential",&f["registration"].to_string())]),fixture_host(&f)).await;
    wait_for_blocked_requests(&db,pid,2).await; tx.commit().await.unwrap();
    assert_eq!(resetting.await.unwrap().status,200); assert_eq!(finishing.await.unwrap().status,409);
    assert_eq!(count(&db,"owner").await,0); assert!(audit_rows(&db.client,"user:passkey-add").await.is_empty());
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn admin_two_factor_reset_blocks_inflight_signed_passkey_signin() {
    let db=test_db().await; two_factor_owner(&db).await; reset_admin(&db).await;
    let f=vector("es256"); assert_eq!(enroll(&db,&f,"owner").await.status,200);
    let c=signin_cookie(&db,&f,&f["assertionChallenge"],"admin-reset");
    let mut client=db.state.db.get().await.unwrap(); let tx=client.transaction().await.unwrap();
    tx.query_one("SELECT id FROM users WHERE id='owner' FOR UPDATE",&[]).await.unwrap();
    let pid=tx.query_one("SELECT pg_backend_pid()",&[]).await.unwrap().get(0);
    let resetting=barrier_request(app(&db),"admin.users.resetTwoFactor",cookie("admin",3,true),form(&[("id","owner"),("currentPassword",PASSWORD)]),"localhost").await;
    wait_for_blocked_requests(&db,pid,1).await;
    let signing=barrier_request(app(&db),"login.passkeyFinish",c,form(&[("credential",&f["assertion"].to_string())]),fixture_host(&f)).await;
    wait_for_blocked_requests(&db,pid,2).await; tx.commit().await.unwrap();
    assert_eq!(resetting.await.unwrap().status,200); assert!(!signed_in(&signing.await.unwrap()));
    assert_eq!(count(&db,"owner").await,0); assert!(audit_rows(&db.client,"login").await.is_empty());
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn admin_two_factor_reset_blocks_inflight_backup_regeneration() {
    let db=test_db().await; two_factor_owner(&db).await; reset_admin(&db).await; stored(&db,"owner","one").await;
    let mut client=db.state.db.get().await.unwrap(); let tx=client.transaction().await.unwrap();
    tx.query_one("SELECT id FROM users WHERE id='owner' FOR UPDATE",&[]).await.unwrap();
    let pid=tx.query_one("SELECT pg_backend_pid()",&[]).await.unwrap().get(0);
    let resetting=barrier_request(app(&db),"admin.users.resetTwoFactor",cookie("admin",3,true),form(&[("id","owner"),("currentPassword",PASSWORD)]),"localhost").await;
    wait_for_blocked_requests(&db,pid,1).await;
    let regenerating=barrier_request(app(&db),"account.regenerateBackupCodes",cookie("owner",3,true),form(&[("currentPassword",PASSWORD)]),"localhost").await;
    wait_for_blocked_requests(&db,pid,2).await; tx.commit().await.unwrap();
    assert_eq!(resetting.await.unwrap().status,200); assert_eq!(regenerating.await.unwrap().status,401);
    let after=disable_state(&db).await; assert_eq!(after["sv"],4); assert!(after["backups"].is_null());
    assert!(audit_rows(&db.client,"user:2fa-backup-regenerate").await.is_empty());
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn reciprocal_admin_two_factor_resets_use_sorted_user_locks() {
    let db=test_db().await;
    let database:String=db.client.query_one("SELECT current_database()",&[]).await.unwrap().get(0);
    db.client.batch_execute(&format!("ALTER DATABASE {database} SET statement_timeout='15s'")).await.unwrap();
    db.state.db.get().await.unwrap().batch_execute("SET statement_timeout='15s'").await.unwrap();
    two_factor_owner(&db).await; reset_admin(&db).await;
    db.client.batch_execute("UPDATE users SET role='admin',totp_enabled=true").await.unwrap();
    let mut client=db.state.db.get().await.unwrap(); let tx=client.transaction().await.unwrap();
    tx.query_one("SELECT id FROM users WHERE id='admin' FOR UPDATE",&[]).await.unwrap();
    let pid=tx.query_one("SELECT pg_backend_pid()",&[]).await.unwrap().get(0);
    let first=barrier_request(app(&db),"admin.users.resetTwoFactor",cookie("admin",3,true),form(&[("id","owner"),("currentPassword",PASSWORD)]),"localhost").await;
    wait_for_blocked_requests(&db,pid,1).await;
    let second=barrier_request(app(&db),"admin.users.resetTwoFactor",cookie("owner",3,true),form(&[("id","admin"),("currentPassword",PASSWORD)]),"localhost").await;
    wait_for_blocked_requests(&db,pid,2).await; tx.commit().await.unwrap();
    let replies=tokio::time::timeout(Duration::from_secs(20),async { (first.await.unwrap(),second.await.unwrap()) }).await.expect("sorted locks must not deadlock");
    assert_eq!(replies.0.status,200); assert_eq!(replies.1.status,409);
    assert_eq!(sv(&db,"admin").await,3); assert_eq!(sv(&db,"owner").await,4);
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn admin_two_factor_reset_blocks_pending_login_finalisation() {
    use nwui_api::auth::pending_login::{self, Pending};
    let db=test_db().await; two_factor_owner(&db).await; reset_admin(&db).await;
    let secret:Option<String>=db.client.query_one("SELECT totp_secret FROM users WHERE id='owner'",&[]).await.unwrap().get(0);
    let pending=Pending {user_id:"owner".into(),throttle_key:"owner@example.test".into(),session_version:3,factor:pending_login::factor_digest(secret.as_deref()),expires_ms:principal::now_unix_ms()+300_000,attempts:0,finished:true};
    let mut client=db.state.db.get().await.unwrap(); let tx=client.transaction().await.unwrap();
    tx.query_one("SELECT id FROM users WHERE id='owner' FOR UPDATE",&[]).await.unwrap();
    let pid=tx.query_one("SELECT pg_backend_pid()",&[]).await.unwrap().get(0);
    let resetting=barrier_request(app(&db),"admin.users.resetTwoFactor",cookie("admin",3,true),form(&[("id","owner"),("currentPassword",PASSWORD)]),"localhost").await;
    wait_for_blocked_requests(&db,pid,1).await;
    let barrier=std::sync::Arc::new(tokio::sync::Barrier::new(2)); let ready=barrier.clone(); let state=db.state.clone();
    let signing=tokio::spawn(async move {ready.wait().await;pending_login::complete_login(&state,&pending,&auth_support::ctx(),&principal::now_unix_ms).await});
    barrier.wait().await; wait_for_blocked_requests(&db,pid,2).await; tx.commit().await.unwrap();
    assert_eq!(resetting.await.unwrap().status,200); assert!(signing.await.unwrap().unwrap().is_none());
    assert!(audit_rows(&db.client,"login").await.is_empty()); assert_eq!(sv(&db,"owner").await,4);
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn delayed_two_factor_enable_cannot_restore_factor_after_reset() {
    let db=test_db().await;
    user(&db,"owner","editor",Some("unused")).await;
    stored(&db,"owner","old-key").await;
    let secret=nwui_api::auth::totp::generate_secret();
    let code=nwui_api::auth::totp::code_at(&nwui_api::auth::totp::decode_secret(&secret).unwrap(),principal::now_unix_ms() as u64);
    let mut client=db.state.db.get().await.unwrap(); let tx=client.transaction().await.unwrap();
    tx.query_one("SELECT id FROM users WHERE id='owner' FOR UPDATE",&[]).await.unwrap();
    let pid=tx.query_one("SELECT pg_backend_pid()",&[]).await.unwrap().get(0);
    let enabling=barrier_request(app(&db),"account.enableTwoFactor",cookie("owner",3,false),form(&[("secret",&secret),("code",&code)]),"localhost").await;
    wait_for_blocked_requests(&db,pid,1).await;
    // A is admitted and waiting at UPDATE. B enrolls then resets under its held row lock.
    let sealed=db.state.secrets.seal(&secret).unwrap();
    tx.execute("UPDATE users SET totp_enabled=true,totp_secret=$1,session_version=session_version+1 WHERE id='owner'",&[&sealed]).await.unwrap();
    tx.execute("DELETE FROM user_passkeys WHERE user_id='owner'",&[]).await.unwrap();
    tx.execute("UPDATE users SET totp_enabled=false,totp_secret=NULL,backup_codes=NULL,totp_last_step=NULL,session_version=session_version+1 WHERE id='owner'",&[]).await.unwrap();
    tx.commit().await.unwrap();
    let reply=enabling.await.unwrap(); assert_eq!(reply.status,409); assert!(reply.cookies.is_empty());
    assert_eq!(reply.body["error"]["message"],"인증 정보가 바뀌었거나 요청이 만료되었습니다. 다시 시작하세요.");
    let after=disable_state(&db).await; assert_eq!(after["sv"],5); assert_eq!(after["enabled"],false); assert!(after["secret"].is_null());
    assert!(audit_rows(&db.client,"user:2fa-enable").await.is_empty());
}

async fn seed_owner_token(db: &TestDb) {
    db.client.batch_execute("INSERT INTO access_tokens(id,kind,owner_user_id,name,token_hash,token_suffix,scopes) VALUES('owner-token','user','owner','fixture','fixture-hash','test','[]')").await.unwrap();
}
async fn owner_token_revoked(db: &TestDb) -> bool {
    db.client.query_one("SELECT revoked_at IS NOT NULL FROM access_tokens WHERE id='owner-token'",&[]).await.unwrap().get(0)
}
#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn factor_changes_revoke_tokens_in_the_same_transaction() {
    for admin in [false,true] {
        for fail in [true,false] {
            let db=test_db().await; two_factor_owner(&db).await; reset_admin(&db).await;
            seed_owner_token(&db).await;
            let before=disable_state(&db).await;
            if fail { db.client.batch_execute("ALTER TABLE audit_logs ADD CONSTRAINT reject_token_revoke CHECK(action <> 'token:revoke')").await.unwrap(); }
            let reply=if admin { reset_factor(&db).await } else { command(&app(&db),"account.disableTwoFactor",&cookie("owner",3,true),&form(&[("currentPassword",PASSWORD)])).await };
            assert_eq!(reply.status,if fail {503} else {200},"{}",reply.body);
            assert_eq!(owner_token_revoked(&db).await,!fail);
            assert_eq!(audit_rows(&db.client,"token:revoke").await.len(),if fail {0} else {1});
            if fail { assert_eq!(disable_state(&db).await,before); }
        }
    }
}
#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn not_enrolled_factor_reset_keeps_tokens() {
    let db=test_db().await; two_factor_owner(&db).await; reset_admin(&db).await; seed_owner_token(&db).await;
    db.client.execute("UPDATE users SET totp_enabled=false WHERE id='owner'",&[]).await.unwrap();
    assert_eq!(reset_factor(&db).await.status,409);
    assert!(!owner_token_revoked(&db).await);
    assert!(audit_rows(&db.client,"token:revoke").await.is_empty());
}
