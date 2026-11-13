//! Real-PostgreSQL tests for `admin::users`. Ignored by default:
//!
//!     NWUI_TEST_DATABASE_URL=postgres://user:pass@host:5432/postgres cargo test --test admin_db -- --include-ignored

mod auth_support;

use auth_support::{TestDb, UserRow, audit_rows, ctx, insert_user, test_db};
use nwui_api::admin::AdminError;
use nwui_api::admin::users::{self, NewUser};
use nwui_api::auth::password;
use nwui_api::auth::principal::SessionPrincipal;
use nwui_api::contracts::Role;
use serde_json::{Value, json};

const PASSWORD: &str = "Passw0rd-admin";

fn actor(id: &str) -> SessionPrincipal {
    SessionPrincipal {
        user_id: id.into(), email: None, name: None, role: Role::Admin, session_version: 0,
        totp_enabled: true, requires_2fa: false, mfa_verified: true, expires_at: 0,
    }
}

fn refused(message: &str) -> AdminError {
    AdminError::Refused(message.to_owned())
}

async fn admin(db: &TestDb, id: &str) {
    insert_user(&db.client, UserRow { id, email: &format!("{id}@example.test"), role: "admin", ..Default::default() }, Some("$2b$12$placeholderplaceholderplaceholderplaceholderplace")).await;
}

async fn user_state(db: &TestDb, id: &str) -> (String, bool, i32) {
    let row = db.client.query_one("SELECT role::text, enabled, session_version FROM users WHERE id = $1", &[&id]).await.unwrap();
    (row.get(0), row.get(1), row.get(2))
}

// ─────────────────────────────── views ───────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn list_detail_and_pending_views_never_carry_the_hash() {
    let db = test_db().await;
    db.client.batch_execute("
        INSERT INTO users (id, email, password_hash, role, created_at) VALUES ('a', 'a@example.test', 'HASH-A', 'admin', '2026-01-01T00:00:00Z');
        INSERT INTO users (id, email, role, approved, created_at) VALUES ('o', NULL, 'viewer', false, '2026-01-02T00:00:00Z');
        INSERT INTO users (id, email, password_hash, role, enabled, approved, created_at) VALUES ('r', 'r@example.test', NULL, 'viewer', false, false, '2026-01-03T00:00:00Z');
        INSERT INTO oauth_providers (id, name, issuer, client_id, client_secret_enc) VALUES ('kc', 'Keycloak', 'https://idp.example.test', 'cid', 'enc:v1:x'), ('gh', 'GitHub', 'https://gh.example.test', 'cid', 'enc:v1:x');
        INSERT INTO oauth_accounts (provider_id, subject, user_id, display_name, email, linked_at) VALUES ('kc', 's1', 'o', '오어스 사용자', 'o@idp.test', '2026-01-02T00:00:00Z'), ('gh', 's2', 'o', NULL, NULL, '2026-01-04T00:00:00Z'), ('gh', 's3', 'a', 'Alice', NULL, '2026-01-05T00:00:00Z');
    ").await.unwrap();

    db.client.execute("UPDATE oauth_accounts SET email_verified=true WHERE provider_id='kc' AND subject='s1'", &[]).await.unwrap();
    let list = serde_json::to_value(users::list(&db.state).await.unwrap()).unwrap();
    assert_eq!(list, json!([
        {"id": "a", "email": "a@example.test", "displayName": "Alice", "role": "admin", "enabled": true, "approved": true, "hasPassword": true, "providers": ["GitHub"], "createdAt": "2026-01-01T00:00:00.000Z"},
        {"id": "o", "email": "o@idp.test", "displayName": "오어스 사용자", "role": "viewer", "enabled": true, "approved": false, "hasPassword": false, "providers": ["Keycloak", "GitHub"], "createdAt": "2026-01-02T00:00:00.000Z"},
        {"id": "r", "email": "r@example.test", "displayName": null, "role": "viewer", "enabled": false, "approved": false, "hasPassword": false, "providers": [], "createdAt": "2026-01-03T00:00:00.000Z"},
    ]));
    assert!(!list.to_string().contains("HASH-A"));

    let detail = serde_json::to_value(users::detail(&db.state, "a").await.unwrap()).unwrap();
    assert_eq!(detail, json!({"totpEnabled": false, "id": "a", "email": "a@example.test", "role": "admin", "enabled": true, "approved": true, "hasPassword": true, "createdAt": "2026-01-01T00:00:00.000Z"}));
    assert_eq!(users::detail(&db.state, "missing").await.unwrap_err(), AdminError::NotFound);

    // Rejected (disabled) accounts leave the queue but stay in the list.
    let pending = serde_json::to_value(users::pending(&db.state).await.unwrap()).unwrap();
    assert_eq!(pending, json!([{"id": "o", "email": "o@idp.test", "displayName": "오어스 사용자", "provider": "Keycloak", "createdAt": "2026-01-02T00:00:00.000Z"}]));
}

// ─────────────────────────────── create ───────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn create_validates_like_ts_and_audits_without_the_password() {
    let db = test_db().await;
    admin(&db, "boss").await;
    let boss = actor("boss");
    let ctx = ctx();
    let input = |email, pw, confirm, role| NewUser { email, password: pw, confirm_password: confirm, role };

    let AdminError::Invalid(errors) = users::create(&db.state, &boss, &ctx, input("bad", "short", "x", "root")).await.unwrap_err() else { panic!() };
    assert_eq!(errors, [
        ("confirmPassword", "비밀번호가 일치하지 않습니다."), ("email", "올바른 이메일 형식을 입력하세요."),
        ("password", "비밀번호는 최소 8자 이상이어야 합니다."), ("role", "역할을 선택하세요."),
    ].into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect());

    let id = users::create(&db.state, &boss, &ctx, input(" New@Example.TEST ", PASSWORD, PASSWORD, "editor")).await.unwrap();
    let row = db.client.query_one("SELECT email, role::text, password_hash FROM users WHERE id = $1", &[&id]).await.unwrap();
    assert_eq!((row.get::<_, String>(0).as_str(), row.get::<_, String>(1).as_str()), ("new@example.test", "editor"));
    assert!(password::verify_password(PASSWORD, &row.get::<_, String>(2)));
    let audit = audit_rows(&db.client, "user:create").await;
    assert_eq!(audit[0].get::<_, Option<String>>("actor_id").as_deref(), Some("boss"));
    assert_eq!(audit[0].get::<_, Option<Value>>("after"), Some(json!({"email": "new@example.test", "role": "editor"})));

    assert_eq!(
        users::create(&db.state, &boss, &ctx, input("new@example.test", PASSWORD, PASSWORD, "viewer")).await.unwrap_err(),
        AdminError::Invalid([("email".to_owned(), "이미 사용 중인 이메일입니다: new@example.test".to_owned())].into())
    );
}

// ─────────────────────────────── role ───────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn role_changes_bump_the_session_version_and_keep_one_admin() {
    let db = test_db().await;
    admin(&db, "boss").await;
    insert_user(&db.client, UserRow { id: "ed", email: "ed@example.test", role: "editor", sv: 7, ..Default::default() }, None).await;
    let (boss, ctx) = (actor("boss"), ctx());

    assert_eq!(users::update_role(&db.state, &boss, &ctx, "boss", "viewer").await.unwrap_err(), refused("본인 계정의 역할은 변경할 수 없습니다."));
    assert_eq!(users::update_role(&db.state, &boss, &ctx, "ed", "root").await.unwrap_err(), AdminError::Invalid([("role".to_owned(), "역할을 선택하세요.".to_owned())].into()));
    assert_eq!(users::update_role(&db.state, &boss, &ctx, "", "viewer").await.unwrap_err(), refused("잘못된 요청입니다."));
    assert_eq!(users::update_role(&db.state, &boss, &ctx, "nobody", "viewer").await.unwrap_err(), AdminError::NotFound);

    users::update_role(&db.state, &boss, &ctx, "ed", "admin").await.unwrap();
    assert_eq!(user_state(&db, "ed").await, ("admin".into(), true, 8));
    let audit = audit_rows(&db.client, "user:update").await;
    assert_eq!((audit[0].get::<_, Option<Value>>("before"), audit[0].get::<_, Option<Value>>("after")), (Some(json!({"role": "editor"})), Some(json!({"role": "admin"}))));

    // Two admins now; "ed" demoting "boss" leaves one, then nobody may demote "ed".
    users::update_role(&db.state, &actor("ed"), &ctx, "boss", "editor").await.unwrap();
    insert_user(&db.client, UserRow { id: "third", email: "third@example.test", role: "admin", enabled: false, ..Default::default() }, None).await;
    assert_eq!(users::update_role(&db.state, &actor("third"), &ctx, "ed", "viewer").await.unwrap_err(), refused("마지막 남은 활성 admin의 역할은 변경할 수 없습니다."), "a disabled admin does not count");
}

/// The TS race, made deterministic: another demotion has passed its guard and holds its row but
/// has not committed. The guard here must wait for it and count again — not act on the old count.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn the_last_admin_guard_waits_for_a_concurrent_demotion() {
    let mut db = test_db().await;
    admin(&db, "a").await;
    admin(&db, "b").await;

    let concurrent = db.client.transaction().await.unwrap();
    concurrent.execute("UPDATE users SET role = 'viewer' WHERE id = 'b'", &[]).await.unwrap();

    let state = db.state.clone();
    let second = tokio::spawn(async move { users::update_role(&state, &actor("b"), &ctx(), "a", "viewer").await });
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(!second.is_finished(), "the guard must block on the admin rows the other change holds");
    concurrent.commit().await.unwrap();

    assert_eq!(second.await.unwrap().unwrap_err(), refused("마지막 남은 활성 admin의 역할은 변경할 수 없습니다."));
    let admins: i64 = db.client.query_one("SELECT count(*) FROM users WHERE role = 'admin' AND enabled", &[]).await.unwrap().get(0);
    assert_eq!(admins, 1);
}

// ─────────────────────────────── password ───────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn password_reset_rehashes_revokes_and_refuses_oauth_only_accounts() {
    let db = test_db().await;
    admin(&db, "boss").await;
    insert_user(&db.client, UserRow { id: "ed", email: "ed@example.test", sv: 2, ..Default::default() }, Some(&password::hash_password("Old-passw0rd").unwrap())).await;
    insert_user(&db.client, UserRow { id: "oauth", email: "oauth@example.test", ..Default::default() }, None).await;
    let (boss, ctx) = (actor("boss"), ctx());

    let AdminError::Invalid(errors) = users::reset_password(&db.state, &boss, &ctx, "ed", "abcdefgh", "other", None).await.unwrap_err() else { panic!() };
    assert_eq!(errors.len(), 2);
    assert_eq!(users::reset_password(&db.state, &boss, &ctx, "oauth", PASSWORD, PASSWORD, None).await.unwrap_err(), refused("이 계정은 OAuth 로그인만 사용합니다 — 비밀번호를 설정할 수 없습니다."));
    users::reset_password(&db.state, &boss, &ctx, "ed", PASSWORD, PASSWORD, None).await.unwrap();
    let row = db.client.query_one("SELECT password_hash, session_version FROM users WHERE id = 'ed'", &[]).await.unwrap();
    assert!(password::verify_password(PASSWORD, &row.get::<_, String>(0)));
    assert_eq!(row.get::<_, i32>(1), 3);
    assert_eq!(audit_rows(&db.client, "user:reset-password").await[0].get::<_, Option<Value>>("after"), Some(json!({"passwordReset": true})));
}

// ─────────────────────────────── delete / toggle ───────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn delete_and_disable_guard_self_and_the_last_admin() {
    let db = test_db().await;
    admin(&db, "boss").await;
    insert_user(&db.client, UserRow { id: "ed", email: "ed@example.test", sv: 4, ..Default::default() }, None).await;
    let (boss, ctx) = (actor("boss"), ctx());

    assert_eq!(users::delete(&db.state, &boss, &ctx, "boss").await.unwrap_err(), refused("본인 계정은 삭제할 수 없습니다."));
    assert_eq!(users::toggle(&db.state, &boss, &ctx, "boss").await.unwrap_err(), refused("본인 계정은 비활성화할 수 없습니다."));
    assert_eq!(users::delete(&db.state, &actor("ed"), &ctx, "boss").await.unwrap_err(), refused("마지막 남은 활성 admin은 삭제할 수 없습니다."));
    assert_eq!(users::toggle(&db.state, &actor("ed"), &ctx, "boss").await.unwrap_err(), refused("마지막 남은 활성 admin은 비활성화할 수 없습니다."));

    assert!(!users::toggle(&db.state, &boss, &ctx, "ed").await.unwrap());
    assert_eq!(user_state(&db, "ed").await, ("editor".into(), false, 5), "disabling revokes sessions");
    assert!(users::toggle(&db.state, &boss, &ctx, "ed").await.unwrap());
    assert_eq!(user_state(&db, "ed").await, ("editor".into(), true, 5), "enabling keeps the version");
    let disable = audit_rows(&db.client, "user:disable").await;
    assert_eq!((disable[0].get::<_, Option<Value>>("before"), disable[0].get::<_, Option<Value>>("after")), (Some(json!({"enabled": true})), Some(json!({"enabled": false}))));
    assert_eq!(audit_rows(&db.client, "user:enable").await.len(), 1);

    users::delete(&db.state, &boss, &ctx, "ed").await.unwrap();
    assert_eq!(users::detail(&db.state, "ed").await.unwrap_err(), AdminError::NotFound);
    assert_eq!(audit_rows(&db.client, "user:delete").await[0].get::<_, Option<Value>>("before"), Some(json!({"email": "ed@example.test", "role": "editor"})));
    assert_eq!(users::delete(&db.state, &boss, &ctx, "ed").await.unwrap_err(), AdminError::NotFound);
}

// ─────────────────────────────── security ───────────────────────────────

use nwui_api::admin::security::{self, TwoFactorPolicy};

fn policy(admin: bool, user: bool, oauth: bool) -> TwoFactorPolicy {
    TwoFactorPolicy { require2fa_admin: admin, require2fa_user: user, require2fa_oauth: oauth }
}

async fn versions(db: &TestDb) -> Vec<(String, i32)> {
    db.client.query("SELECT id, session_version FROM users ORDER BY id", &[]).await.unwrap().iter().map(|r| (r.get(0), r.get(1))).collect()
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn policy_changes_end_only_the_affected_groups_sessions() {
    let db = test_db().await;
    admin(&db, "boss").await; // sv 3
    insert_user(&db.client, UserRow { id: "ed", email: "ed@example.test", ..Default::default() }, None).await;
    insert_user(&db.client, UserRow { id: "oa", email: "oa@example.test", role: "viewer", ..Default::default() }, None).await;
    db.client.batch_execute("INSERT INTO oauth_providers (id, name, issuer, client_id, client_secret_enc) VALUES ('kc', 'KC', 'https://idp.example.test', 'c', 'enc:v1:x');
        INSERT INTO oauth_accounts (provider_id, subject, user_id) VALUES ('kc', 's', 'oa')").await.unwrap();
    let (boss, ctx) = (actor("boss"), ctx());

    // No settings row yet: everything off.
    assert_eq!(security::view(&db.state).await.unwrap().policy, policy(false, false, false));

    security::update_policy(&db.state, &boss, &ctx, policy(true, false, false)).await.unwrap();
    assert_eq!(versions(&db).await, [("boss".into(), 4), ("ed".into(), 3), ("oa".into(), 3)], "admins only");
    security::update_policy(&db.state, &boss, &ctx, policy(true, true, false)).await.unwrap();
    assert_eq!(versions(&db).await, [("boss".into(), 4), ("ed".into(), 4), ("oa".into(), 4)], "non-admins only");
    security::update_policy(&db.state, &boss, &ctx, policy(true, true, true)).await.unwrap();
    assert_eq!(versions(&db).await, [("boss".into(), 4), ("ed".into(), 4), ("oa".into(), 5)], "linked accounts only");
    security::update_policy(&db.state, &boss, &ctx, policy(true, true, true)).await.unwrap();
    assert_eq!(versions(&db).await, [("boss".into(), 4), ("ed".into(), 4), ("oa".into(), 5)], "no change, no revocation");

    assert_eq!(security::view(&db.state).await.unwrap().policy, policy(true, true, true));
    let actor_id: Option<String> = db.client.query_one("SELECT actor_id FROM app_settings WHERE id = 'current'", &[]).await.unwrap().get(0);
    assert_eq!(actor_id.as_deref(), Some("boss"));
    let audit = audit_rows(&db.client, "settings:2fa-policy").await;
    assert_eq!(audit.len(), 4);
    assert_eq!(audit[0].get::<_, Option<Value>>("before"), Some(json!({"require2faAdmin": false, "require2faUser": false, "require2faOauth": false})));
    assert_eq!(audit[0].get::<_, Option<Value>>("after"), Some(json!({"require2faAdmin": true, "require2faUser": false, "require2faOauth": false})));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn gateways_are_listed_newest_first_and_revoked_once() {
    let db = test_db().await;
    admin(&db, "boss").await;
    db.client.batch_execute("
        INSERT INTO gateway_tokens (id, name, token_hash, role, created_at, last_used_at, last_used_ip) VALUES ('g1', 'old-gw', 'HASH-1', 'viewer', '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z', '203.0.113.1');
        INSERT INTO gateway_tokens (id, name, token_hash, role, created_at) VALUES ('g2', 'new-gw', 'HASH-2', 'admin', '2026-02-01T00:00:00Z');
    ").await.unwrap();
    let (boss, ctx) = (actor("boss"), ctx());

    let view = serde_json::to_value(security::view(&db.state).await.unwrap()).unwrap();
    assert_eq!(view["gateways"], json!([
        {"id": "g2", "name": "new-gw", "role": "admin", "createdAt": "2026-02-01T00:00:00.000Z", "lastUsedAt": null, "lastUsedIp": null, "revokedAt": null, "legacyOrphaned": false},
        {"id": "g1", "name": "old-gw", "role": "viewer", "createdAt": "2026-01-01T00:00:00.000Z", "lastUsedAt": "2026-01-02T00:00:00.000Z", "lastUsedIp": "203.0.113.1", "revokedAt": null, "legacyOrphaned": false},
    ]));
    assert!(!view.to_string().contains("HASH-"));

    assert_eq!(security::revoke_gateway(&db.state, &boss, &ctx, "").await.unwrap_err(), refused("잘못된 요청입니다."));
    assert_eq!(security::revoke_gateway(&db.state, &boss, &ctx, "nope").await.unwrap_err(), AdminError::NotFound);
    security::revoke_gateway(&db.state, &boss, &ctx, "g1").await.unwrap();
    security::revoke_gateway(&db.state, &boss, &ctx, "g1").await.unwrap();
    let audit = audit_rows(&db.client, "node:gateway:revoke").await;
    assert_eq!(audit.len(), 1, "a second revoke has nothing to do");
    assert_eq!(audit[0].get::<_, Option<Value>>("before"), Some(json!({"name": "old-gw", "role": "viewer"})));
    let view = security::view(&db.state).await.unwrap();
    assert!(view.gateways.iter().find(|g| g.id == "g1").unwrap().revoked_at.is_some());
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn legacy_gateway_orphans_are_reported_without_changing_access_or_revocation() {
    let db = test_db().await;
    admin(&db, "issuer").await;
    admin(&db, "deleted").await;
    db.client.batch_execute("INSERT INTO gateway_tokens (id,name,token_hash,role,legacy_issuer,created_by_user_id) VALUES
        ('eligible','eligible','SECRET-eligible','viewer',true,'issuer'),
        ('missing','missing','SECRET-missing','viewer',true,NULL),
        ('deleted','deleted','SECRET-deleted','viewer',true,'deleted'),
        ('strict','strict','SECRET-strict','viewer',false,NULL),
        ('revoked','revoked','SECRET-revoked','viewer',true,NULL);
        UPDATE gateway_tokens SET revoked_at=now() WHERE id='revoked';
        DELETE FROM users WHERE id='deleted';").await.unwrap();
    let before = db.client.query("SELECT id,legacy_issuer,revoked_at FROM gateway_tokens ORDER BY id", &[]).await.unwrap();
    for (change, eligible) in [
        ("UPDATE users SET enabled=true, approved=true, role='admin' WHERE id='issuer'", true),
        ("UPDATE users SET enabled=false WHERE id='issuer'", false),
        ("UPDATE users SET enabled=true, approved=false WHERE id='issuer'", false),
        ("UPDATE users SET approved=true, role='editor' WHERE id='issuer'", false),
    ] {
        db.client.batch_execute(change).await.unwrap();
        let view = serde_json::to_value(security::view(&db.state).await.unwrap()).unwrap();
        for row in view["gateways"].as_array().unwrap() {
            let expected = match row["id"].as_str().unwrap() {
                "eligible" => !eligible,
                "strict" => false,
                _ => true,
            };
            assert_eq!(row["legacyOrphaned"], json!(expected), "{change}: {row}");
        }
        assert!(!view.to_string().contains("SECRET-"));
    }
    let after = db.client.query("SELECT id,legacy_issuer,revoked_at FROM gateway_tokens ORDER BY id", &[]).await.unwrap();
    for (a,b) in before.iter().zip(after.iter()) {
        assert_eq!(a.get::<_,String>(0), b.get::<_,String>(0));
        assert_eq!(a.get::<_,bool>(1), b.get::<_,bool>(1));
        assert_eq!(a.get::<_,Option<chrono::DateTime<chrono::Utc>>>(2), b.get::<_,Option<chrono::DateTime<chrono::Utc>>>(2));
    }
}

// ─────────────────────────────── oauth providers ───────────────────────────────

use nwui_api::admin::oauth;
use nwui_api::contracts::FormInput;

fn form(pairs: &[(&str, &str)]) -> FormInput {
    let mut input = FormInput::default();
    for (k, v) in pairs {
        input.fields.entry((*k).to_owned()).or_default().push((*v).to_owned());
    }
    input
}

const PROVIDER: &[(&str, &str)] = &[
    ("id", "kc"), ("name", " Keycloak "), ("issuer", "https://idp.example.test/realms/main"), ("clientId", "nginx-web-ui"),
    ("clientSecret", "s3cr3t"), ("scope", "email profile email"), ("enabled", "true"), ("sortOrder", " 2 "),
];

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn provider_create_validates_seals_and_never_audits_the_secret() {
    let db = test_db().await;
    admin(&db, "boss").await;
    let (boss, ctx) = (actor("boss"), ctx());

    let AdminError::Invalid(errors) = oauth::create(&db.state, &boss, &ctx, &form(&[("id", "Bad_Id"), ("issuer", "http://idp.example.test"), ("sortOrder", "1e2")])).await.unwrap_err() else { panic!() };
    assert_eq!(errors["id"], "공급자 ID 는 영소문자로 시작하고 소문자·숫자·하이픈만 쓸 수 있습니다(최대 64자): \"Bad_Id\"");
    assert_eq!(errors["issuer"], "로그인 서버 주소는 https 여야 합니다.");
    assert_eq!(errors["name"], "표시 이름을 입력하세요.");
    assert_eq!(errors["clientId"], "클라이언트 ID 를 입력하세요.");
    assert_eq!(errors["clientSecret"], "클라이언트 비밀키를 입력하세요.");
    assert_eq!(errors["sortOrder"], "정렬 순서는 정수여야 합니다.");

    assert_eq!(oauth::create(&db.state, &boss, &ctx, &form(PROVIDER)).await.unwrap(), "kc");
    let row = db.client.query_one("SELECT name, scope, enabled, sort_order, client_secret_enc, actor_id FROM oauth_providers WHERE id = 'kc'", &[]).await.unwrap();
    assert_eq!((row.get::<_, String>(0).as_str(), row.get::<_, String>(1).as_str(), row.get::<_, bool>(2), row.get::<_, i32>(3)), ("Keycloak", "openid email profile", true, 2));
    let sealed: String = row.get(4);
    assert!(sealed.starts_with("enc:v1:"));
    assert_eq!(db.state.secrets.open(Some(&sealed)).as_deref(), Some("s3cr3t"));
    assert_eq!(row.get::<_, Option<String>>(5).as_deref(), Some("boss"));
    let audit = audit_rows(&db.client, "oauth:provider-create").await;
    assert_eq!(audit[0].get::<_, Option<Value>>("after"), Some(json!({"name": "Keycloak", "issuer": "https://idp.example.test/realms/main", "enabled": true})));
    let everything: String = db.client.query_one("SELECT string_agg(audit_logs::text, ' ') FROM audit_logs", &[]).await.unwrap().get(0);
    assert!(!everything.contains("s3cr3t"));

    let AdminError::Invalid(errors) = oauth::create(&db.state, &boss, &ctx, &form(PROVIDER)).await.unwrap_err() else { panic!() };
    assert_eq!(errors["id"], "이미 같은 ID 의 공급자가 있습니다.");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn provider_update_keeps_or_replaces_the_secret() {
    let db = test_db().await;
    admin(&db, "boss").await;
    let (boss, ctx) = (actor("boss"), ctx());
    oauth::create(&db.state, &boss, &ctx, &form(PROVIDER)).await.unwrap();
    async fn sealed(db: &TestDb) -> String {
        db.client.query_one("SELECT client_secret_enc FROM oauth_providers WHERE id = 'kc'", &[]).await.unwrap().get(0)
    }
    let before = sealed(&db).await;

    let mut keep: Vec<(&str, &str)> = PROVIDER.to_vec();
    keep.retain(|(k, _)| *k != "clientSecret" && *k != "name");
    keep.extend([("clientSecret", oauth::KEEP_SECRET), ("name", "KC 새이름")]);
    assert_eq!(oauth::update(&db.state, &boss, &ctx, &form(&keep)).await.unwrap(), "kc");
    assert_eq!(sealed(&db).await, before, "__keep__ leaves the sealed value untouched");
    let audit = audit_rows(&db.client, "oauth:provider-update").await;
    assert_eq!(audit[0].get::<_, Option<Value>>("after").unwrap()["secretChanged"], false);

    let mut change: Vec<(&str, &str)> = PROVIDER.to_vec();
    change.retain(|(k, _)| *k != "clientSecret");
    change.push(("clientSecret", "rotated"));
    oauth::update(&db.state, &boss, &ctx, &form(&change)).await.unwrap();
    assert_eq!(db.state.secrets.open(Some(&sealed(&db).await)).as_deref(), Some("rotated"));

    let mut empty: Vec<(&str, &str)> = PROVIDER.to_vec();
    empty.retain(|(k, _)| *k != "clientSecret");
    let AdminError::Invalid(errors) = oauth::update(&db.state, &boss, &ctx, &form(&empty)).await.unwrap_err() else { panic!() };
    assert_eq!(errors["clientSecret"], "클라이언트 비밀키를 입력하세요.", "an empty secret is not 'keep'");
    let mut missing = PROVIDER.to_vec();
    missing.retain(|(k, _)| *k != "id");
    missing.push(("id", "nope"));
    assert_eq!(oauth::update(&db.state, &boss, &ctx, &form(&missing)).await.unwrap_err(), AdminError::NotFound);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn provider_views_toggle_and_delete() {
    let db = test_db().await;
    admin(&db, "boss").await;
    insert_user(&db.client, UserRow { id: "o", email: "o@example.test", ..Default::default() }, None).await;
    let (boss, ctx) = (actor("boss"), ctx());
    oauth::create(&db.state, &boss, &ctx, &form(PROVIDER)).await.unwrap();
    let mut second: Vec<(&str, &str)> = PROVIDER.to_vec();
    second.retain(|(k, _)| !["id", "name", "sortOrder"].contains(k));
    second.extend([("id", "aa"), ("name", "Alpha"), ("sortOrder", "2")]);
    oauth::create(&db.state, &boss, &ctx, &form(&second)).await.unwrap();
    db.client.execute("INSERT INTO oauth_accounts (provider_id, subject, user_id) VALUES ('kc', 's', 'o')", &[]).await.unwrap();

    let list = serde_json::to_value(oauth::list(&db.state).await.unwrap()).unwrap();
    assert_eq!(list.as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect::<Vec<_>>(), ["aa", "kc"], "sort order, then name");
    assert_eq!(list[1]["linkedUsers"], 1);
    assert!(!list.to_string().contains("enc:v1:"));

    let detail = serde_json::to_value(oauth::detail(&db.state, "kc").await.unwrap()).unwrap();
    assert_eq!(detail["callbackUrl"], "/api/auth/callback/kc");
    db.client.execute("INSERT INTO app_settings (id, public_base_url) VALUES ('current', 'https://nginx.example.test/')", &[]).await.unwrap();
    let detail = serde_json::to_value(oauth::detail(&db.state, "kc").await.unwrap()).unwrap();
    assert_eq!((detail["callbackUrl"].as_str(), detail["clientId"].as_str()), (Some("https://nginx.example.test/api/auth/callback/kc"), Some("nginx-web-ui")));
    assert_eq!(oauth::detail(&db.state, "nope").await.unwrap_err(), AdminError::NotFound);

    oauth::toggle(&db.state, &boss, &ctx, "kc").await.unwrap();
    assert_eq!(oauth::detail(&db.state, "kc").await.unwrap().provider.enabled, false);
    assert_eq!(audit_rows(&db.client, "oauth:provider-toggle").await[0].get::<_, Option<Value>>("after"), Some(json!({"enabled": false})));
    oauth::toggle(&db.state, &boss, &ctx, "nope").await.unwrap();

    oauth::delete(&db.state, &boss, &ctx, "kc").await.unwrap();
    assert_eq!(audit_rows(&db.client, "oauth:provider-delete").await[0].get::<_, Option<Value>>("before"), Some(json!({"name": "Keycloak", "issuer": "https://idp.example.test/realms/main", "linkedUsers": 1})));
    let links: i64 = db.client.query_one("SELECT count(*) FROM oauth_accounts", &[]).await.unwrap().get(0);
    assert_eq!(links, 0, "links cascade");
    assert!(users::detail(&db.state, "o").await.is_ok(), "the person stays");
    oauth::delete(&db.state, &boss, &ctx, "kc").await.unwrap();
}

// ─────────────────────────────── approval ───────────────────────────────

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn approval_is_once_and_reject_only_touches_pending_accounts() {
    let db = test_db().await;
    admin(&db, "boss").await;
    insert_user(&db.client, UserRow { id: "p1", email: "p1@example.test", role: "viewer", approved: false, ..Default::default() }, None).await;
    insert_user(&db.client, UserRow { id: "p2", email: "p2@example.test", role: "viewer", approved: false, ..Default::default() }, None).await;
    let (boss, ctx) = (actor("boss"), ctx());

    users::approve(&db.state, &boss, &ctx, "p1", "editor").await.unwrap();
    let row = db.client.query_one("SELECT approved, role::text FROM users WHERE id = 'p1'", &[]).await.unwrap();
    assert_eq!((row.get::<_, bool>(0), row.get::<_, String>(1).as_str()), (true, "editor"));
    let audit = audit_rows(&db.client, "user:approve").await;
    assert_eq!(audit[0].get::<_, Option<Value>>("before"), Some(json!({"approved": false, "role": "viewer"})));
    assert_eq!(audit[0].get::<_, Option<Value>>("after"), Some(json!({"approved": true, "role": "editor"})));
    // Resubmitting approval does not turn into a role change.
    users::approve(&db.state, &boss, &ctx, "p1", "admin").await.unwrap();
    assert_eq!(user_state(&db, "p1").await.0, "editor");
    assert_eq!(audit_rows(&db.client, "user:approve").await.len(), 1);
    // Unknown role → viewer; unknown id → nothing.
    users::approve(&db.state, &boss, &ctx, "p2", "root").await.unwrap();
    assert_eq!(user_state(&db, "p2").await.0, "viewer");
    users::approve(&db.state, &boss, &ctx, "nobody", "viewer").await.unwrap();

    insert_user(&db.client, UserRow { id: "p3", email: "p3@example.test", approved: false, ..Default::default() }, None).await;
    users::reject(&db.state, &boss, &ctx, "p3").await.unwrap();
    assert_eq!(user_state(&db, "p3").await.1, false);
    assert_eq!(audit_rows(&db.client, "user:reject").await[0].get::<_, Option<Value>>("after"), Some(json!({"enabled": false})));
    // An approved, active account (here the only admin) is not "rejected".
    users::reject(&db.state, &boss, &ctx, "boss").await.unwrap();
    assert_eq!(user_state(&db, "boss").await, ("admin".into(), true, 3));
    assert_eq!(audit_rows(&db.client, "user:reject").await.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_i5_pending_admin_cannot_justify_removing_the_last_approved_admin() {
    let mut db = test_db().await;
    admin(&db, "a").await;
    admin(&db, "b").await;
    insert_user(&db.client, UserRow { id: "pending", email: "pending@example.test", role: "admin", approved: false, ..Default::default() }, None).await;
    let concurrent = db.client.transaction().await.unwrap();
    concurrent.execute("UPDATE users SET role='viewer' WHERE id='b'", &[]).await.unwrap();
    let state = db.state.clone();
    let second = tokio::spawn(async move { users::update_role(&state, &actor("b"), &ctx(), "a", "viewer").await });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!second.is_finished());
    concurrent.commit().await.unwrap();
    assert!(second.await.unwrap().is_err(), "a pending admin cannot sign in and must not satisfy the last-admin guard");
    assert_eq!(db.client.query_one("SELECT count(*) FROM users WHERE role='admin' AND enabled AND approved", &[]).await.unwrap().get::<_, i64>(0), 1);
    users::delete(&db.state, &actor("a"), &ctx(), "pending").await.unwrap();
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn legacy_oidc_email_is_hidden_until_verified() {
    let db = test_db().await;
    db.client.batch_execute("INSERT INTO users(id,email,role,approved) VALUES('legacy',NULL,'viewer',false);
        INSERT INTO oauth_providers(id,name,issuer,client_id,client_secret_enc) VALUES('legacy','Legacy','https://idp.test','cid','sealed');
        INSERT INTO oauth_accounts(provider_id,subject,user_id,email) VALUES('legacy','sub','legacy','unverified@idp.test');").await.unwrap();
    assert!(users::list(&db.state).await.unwrap()[0].email.is_none());
    assert!(users::pending(&db.state).await.unwrap()[0].email.is_none());
    assert_eq!(db.client.query_one("SELECT email FROM oauth_accounts WHERE user_id='legacy'", &[]).await.unwrap().get::<_,String>(0), "unverified@idp.test");
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn migration_0030_keeps_11911_identity_rows_and_masks_their_email() {
    let current = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");
    let mut journal: Value = serde_json::from_slice(&std::fs::read(current.join("meta/_journal.json")).unwrap()).unwrap();
    let current_count = journal["entries"].as_array().unwrap().len();
    // v1.19.11 ends at 0027; its committed SQL history is reused byte-for-byte.
    journal["entries"].as_array_mut().unwrap().truncate(28);
    assert_eq!(journal["entries"][27]["tag"], "0027_notification_delivery_queue");
    let legacy = tempfile::tempdir().unwrap();
    std::fs::create_dir(legacy.path().join("meta")).unwrap();
    std::fs::write(legacy.path().join("meta/_journal.json"), serde_json::to_vec(&journal).unwrap()).unwrap();
    for entry in journal["entries"].as_array().unwrap() {
        let name = format!("{}.sql", entry["tag"].as_str().unwrap());
        std::fs::copy(current.join(&name), legacy.path().join(&name)).unwrap();
    }
    let db = auth_support::test_db_with(&[("MIGRATIONS_DIR", legacy.path().to_str().unwrap())]).await;
    assert!(!db.client.query_one("SELECT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_name='oauth_accounts' AND column_name='email_verified')", &[]).await.unwrap().get::<_,bool>(0));
    db.client.batch_execute("INSERT INTO users(id,email,role,approved) VALUES('legacy',NULL,'viewer',false);
        INSERT INTO oauth_providers(id,name,issuer,client_id,client_secret_enc) VALUES('legacy','Legacy','https://idp.test','cid','sealed');
        INSERT INTO oauth_accounts(provider_id,subject,user_id,display_name,email) VALUES('legacy','sub','legacy','Legacy user','stored@idp.test');").await.unwrap();
    let snapshot = "SELECT jsonb_build_object('account',to_jsonb(a)-'email_verified','user',to_jsonb(u),'provider',to_jsonb(p)) FROM oauth_accounts a JOIN users u ON u.id=a.user_id JOIN oauth_providers p ON p.id=a.provider_id";
    let before: Value = db.client.query_one(snapshot,&[]).await.unwrap().get(0);
    assert_eq!(nwui_api::migrations::run(&db.state.db,&current).await.unwrap(),current_count-28);
    let mut after: Value = db.client.query_one(snapshot,&[]).await.unwrap().get(0);
    assert_eq!(after["user"].as_object_mut().unwrap().remove("passkey_user_handle"),Some(Value::Null));
    assert_eq!(after,before);
    assert!(!db.client.query_one("SELECT email_verified FROM oauth_accounts",&[]).await.unwrap().get::<_,bool>(0));
    assert!(users::list(&db.state).await.unwrap()[0].email.is_none());
    assert!(users::pending(&db.state).await.unwrap()[0].email.is_none());
    assert!(users::detail(&db.state,"legacy").await.unwrap().email.is_none());
    assert_eq!(nwui_api::migrations::run(&db.state.db,&current).await.unwrap(),0);
    db.client.execute("UPDATE oauth_accounts SET email_verified=true",&[]).await.unwrap();
    assert_eq!(users::list(&db.state).await.unwrap()[0].email.as_deref(),Some("stored@idp.test"));
}


#[tokio::test]
#[ignore = "real DB"]
async fn q3_i3_self_password_reset_is_refused_without_mutation() {
    let db=test_db().await; admin(&db,"boss").await;
    let before: String=db.client.query_one("SELECT password_hash FROM users WHERE id='boss'",&[]).await.unwrap().get(0);
    let error=users::reset_password(&db.state,&actor("boss"),&ctx(),"boss",PASSWORD,PASSWORD,None).await.unwrap_err();
    assert!(matches!(error,AdminError::Refused(ref text) if text.contains("로그인")));
    let after: String=db.client.query_one("SELECT password_hash FROM users WHERE id='boss'",&[]).await.unwrap().get(0);
    assert_eq!(before,after);
}
