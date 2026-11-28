//! Real-PostgreSQL tests for `auth::tokens` (access tokens). Ignored by default:
//!
//!     NWUI_TEST_DATABASE_URL=postgres://user:pass@host:5432/postgres cargo test --test auth_tokens_db -- --include-ignored

mod auth_support;

use auth_support::{TestDb, UserRow, audit_rows, ctx, insert_user, test_db};
use nwui_api::auth::principal::{self, SessionPrincipal};
use nwui_api::auth::token_digest;
use nwui_api::auth::tokens::{self, IssueFormValue, TokenError, TokenKind, TokenScope, TokenStatus};
use nwui_api::contracts::Role;
use serde_json::{Value, json};

fn actor(id: &str, role: Role) -> SessionPrincipal {
    SessionPrincipal {
        user_id: id.into(), email: Some(format!("{id}@example.test")), name: None, role, session_version: 3,
        totp_enabled: true, requires_2fa: false, mfa_verified: true, expires_at: 0,
    }
}

fn value(name: &str, scopes: &[&str], expires_at_ms: Option<i64>) -> IssueFormValue {
    IssueFormValue { name: name.into(), scopes: scopes.iter().map(|s| (*s).to_owned()).collect(), expires_at_ms }
}

fn rejected(message: &str) -> TokenError {
    TokenError::Rejected(message.to_owned())
}

async fn people(db: &TestDb) {
    insert_user(&db.client, UserRow { id: "boss", email: "boss@example.test", role: "admin", totp_enabled: true, ..Default::default() }, None).await;
    insert_user(&db.client, UserRow { id: "ed", email: "ed@example.test", role: "editor", totp_enabled: true, ..Default::default() }, None).await;
    insert_user(&db.client, UserRow { id: "vi", email: "vi@example.test", role: "viewer", totp_enabled: true, ..Default::default() }, None).await;
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn user_tokens_store_only_a_hash_and_audit_without_the_plaintext() {
    let db = test_db().await;
    people(&db).await;
    let ed = actor("ed", Role::Editor);
    let expires = 1_900_000_000_000i64;
    let issued = tokens::issue(&db.state, &ed, &ctx(), TokenKind::User, value("CI 배포용", &["config:read", "nginx:reload"], Some(expires))).await.unwrap();
    assert!(issued.token.starts_with("nwu_u_") && issued.token.len() == 6 + 43, "{}", issued.token);
    assert!(issued.token[6..].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));

    let row = db.client.query_one("SELECT kind::text, owner_user_id, token_hash, token_suffix, scopes, created_by_user_id, expires_at IS NOT NULL FROM access_tokens WHERE id = $1", &[&issued.id]).await.unwrap();
    assert_eq!(row.get::<_, String>(0), "user");
    assert_eq!(row.get::<_, Option<String>>(1).as_deref(), Some("ed"));
    assert_eq!(row.get::<_, String>(2), token_digest(&issued.token));
    assert_eq!(row.get::<_, String>(3), &issued.token[issued.token.len() - 4..]);
    assert_eq!(row.get::<_, Value>(4), json!(["config:read", "nginx:reload"]));
    assert_eq!(row.get::<_, Option<String>>(5).as_deref(), Some("ed"));

    let audit = audit_rows(&db.client, "token:issue").await;
    assert_eq!(audit[0].get::<_, Option<String>>("actor_id").as_deref(), Some("ed"));
    assert_eq!(audit[0].get::<_, Option<String>>("actor_label").as_deref(), Some("ed@example.test"));
    assert_eq!(audit[0].get::<_, Option<Value>>("after"), Some(json!({"kind": "user", "name": "CI 배포용", "scopes": ["config:read", "nginx:reload"], "expiresAt": "2030-03-17T17:46:40.000Z"})));
    let everything: String = db.client.query_one("SELECT string_agg(audit_logs::text, ' ') FROM audit_logs", &[]).await.unwrap().get(0);
    assert!(!everything.contains(&issued.token[6..]), "the plaintext never reaches the audit table");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn scopes_are_bounded_by_vocabulary_kind_and_role() {
    let db = test_db().await;
    people(&db).await;
    let (ed, vi) = (actor("ed", Role::Editor), actor("vi", Role::Viewer));
    let issue = |who: &SessionPrincipal, kind, scopes: &[&str]| {
        let v = value("t", scopes, None);
        let who = who.clone();
        let state = db.state.clone();
        async move { tokens::issue(&state, &who, &ctx(), kind, v).await }
    };
    assert_eq!(issue(&ed, TokenKind::User, &["bogus"]).await.unwrap_err(), rejected("알 수 없는 권한입니다: bogus"));
    assert_eq!(issue(&ed, TokenKind::User, &["config:read", "user:manage"]).await.unwrap_err(), rejected("이 토큰에 줄 수 없는 권한입니다: user:manage"));
    assert_eq!(issue(&ed, TokenKind::User, &["node:pair"]).await.unwrap_err(), rejected("이 토큰에 줄 수 없는 권한입니다: node:pair"));
    assert_eq!(issue(&vi, TokenKind::User, &["config:write"]).await.unwrap_err(), rejected("이 토큰에 줄 수 없는 권한입니다: config:write"));
    assert_eq!(issue(&ed, TokenKind::Server, &["node:pair"]).await.unwrap_err(), rejected("서버 토큰은 관리자만 발급할 수 있습니다."));
    let count: i64 = db.client.query_one("SELECT count(*) FROM access_tokens", &[]).await.unwrap().get(0);
    assert_eq!(count, 0);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn server_tokens_are_ownerless_audited_by_kind_only_and_pair() {
    let db = test_db().await;
    people(&db).await;
    let boss = actor("boss", Role::Admin);
    let issued = tokens::issue(&db.state, &boss, &ctx(), TokenKind::Server, value("gw.example 연결용", &["node:pair"], Some(principal::now_unix_ms() + 86_400_000))).await.unwrap();
    assert!(issued.token.starts_with("nwu_s_"));
    let owner: Option<String> = db.client.query_one("SELECT owner_user_id FROM access_tokens WHERE id = $1", &[&issued.id]).await.unwrap().get(0);
    assert_eq!(owner, None);
    assert_eq!(audit_rows(&db.client, "token:issue").await[0].get::<_, Option<Value>>("after"), Some(json!({"kind": "server"})), "viewers can read the audit log");

    // The token does what it is for: pairing accepts it.
    let grant = principal::authenticate_pairing(&db.state, &issued.token, &ctx(), principal::now_unix_ms()).await.unwrap();
    assert_eq!((grant.access_token_id, grant.created_by_user_id.as_deref()), (issued.id, Some("boss")));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn lists_are_scoped_masked_and_carry_a_status() {
    let db = test_db().await;
    people(&db).await;
    let (boss, ed) = (actor("boss", Role::Admin), actor("ed", Role::Editor));
    let now = principal::now_unix_ms();
    let old = tokens::issue(&db.state, &ed, &ctx(), TokenKind::User, value("old", &["config:read"], Some(now + 60_000))).await.unwrap();
    db.client.execute("UPDATE access_tokens SET created_at = now() - interval '1 day', expires_at = now() - interval '1 minute' WHERE id = $1", &[&old.id]).await.unwrap();
    let mine = tokens::issue(&db.state, &ed, &ctx(), TokenKind::User, value("mine", &["config:read"], None)).await.unwrap();
    tokens::issue(&db.state, &boss, &ctx(), TokenKind::User, value("boss's", &["config:read"], None)).await.unwrap();
    tokens::issue(&db.state, &boss, &ctx(), TokenKind::Server, value("server", &["node:pair"], Some(principal::now_unix_ms() + 86_400_000))).await.unwrap();

    let own = tokens::list(&db.state, &ed, &TokenScope::User { owner_user_id: "ed".into() }, now).await.unwrap();
    assert_eq!(own.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["mine", "old"], "own tokens only, newest first");
    assert_eq!(own[0].masked, format!("nwu_u_…{}", &mine.token[mine.token.len() - 4..]));
    assert_eq!((own[0].status, own[1].status), (TokenStatus::Active, TokenStatus::Expired));
    assert!(!serde_json::to_string(&own).unwrap().contains(&token_digest(&mine.token)));

    assert_eq!(tokens::list(&db.state, &ed, &TokenScope::Server, now).await.unwrap_err(), rejected("서버 토큰 목록은 관리자만 볼 수 있습니다."));
    let server = tokens::list(&db.state, &boss, &TokenScope::Server, now).await.unwrap();
    assert_eq!(server.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["server"]);
    assert!(server[0].masked.starts_with("nwu_s_…"));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn list_views_keep_expired_tokens_and_details_keep_revoked_tokens() {
    let db = test_db().await;
    people(&db).await;
    let (boss, ed) = (actor("boss", Role::Admin), actor("ed", Role::Editor));
    for (actor, scope, kind, permission) in [
        (&ed, TokenScope::User { owner_user_id: "ed".into() }, TokenKind::User, "config:read"),
        (&boss, TokenScope::Server, TokenKind::Server, "node:pair"),
    ] {
        for (name, expired, revoked) in [
            ("active", false, false),
            ("expired", true, false),
            ("revoked", false, true),
            ("expired and revoked", true, true),
        ] {
            let expires_at = principal::now_unix_ms() + 86_400_000;
            let issued = tokens::issue(&db.state, actor, &ctx(), kind, value(name, &[permission], Some(expires_at))).await.unwrap();
            if expired {
                db.client.execute(
                    "UPDATE access_tokens SET created_at = now() - interval '1 day', expires_at = now() - interval '1 minute' WHERE id = $1",
                    &[&issued.id],
                ).await.unwrap();
            }
            if revoked {
                tokens::revoke(&db.state, actor, &ctx(), &scope, &issued.id).await.unwrap();
                let detail = tokens::detail_view(&db.state, actor, &scope, &issued.id).await.unwrap();
                assert_eq!(detail["id"], issued.id);
                assert_eq!(detail["status"], "revoked");
            }
        }
        let list = tokens::list_view(&db.state, actor, &scope).await.unwrap();
        let mut shown: Vec<_> = list["tokens"].as_array().unwrap().iter()
            .map(|row| (row["name"].as_str().unwrap(), row["status"].as_str().unwrap()))
            .collect();
        shown.sort_unstable();
        assert_eq!(shown, [("active", "active"), ("expired", "expired")]);
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn revoke_is_scoped_once_and_never_says_whose() {
    let db = test_db().await;
    people(&db).await;
    let (boss, ed) = (actor("boss", Role::Admin), actor("ed", Role::Editor));
    let bosses = tokens::issue(&db.state, &boss, &ctx(), TokenKind::User, value("b", &["config:read"], None)).await.unwrap();
    let eds = tokens::issue(&db.state, &ed, &ctx(), TokenKind::User, value("e", &["config:read"], None)).await.unwrap();
    let server = tokens::issue(&db.state, &boss, &ctx(), TokenKind::Server, value("s", &["node:pair"], Some(principal::now_unix_ms() + 86_400_000))).await.unwrap();
    let ed_scope = TokenScope::User { owner_user_id: "ed".into() };

    assert_eq!(tokens::revoke(&db.state, &ed, &ctx(), &ed_scope, &bosses.id).await.unwrap_err(), TokenError::NothingToRevoke, "someone else's");
    assert_eq!(tokens::revoke(&db.state, &ed, &ctx(), &ed_scope, "no-such-id").await.unwrap_err(), TokenError::NothingToRevoke, "missing");
    assert_eq!(tokens::revoke(&db.state, &ed, &ctx(), &TokenScope::Server, &server.id).await.unwrap_err(), rejected("서버 토큰은 관리자만 해지할 수 있습니다."));
    assert_eq!(tokens::revoke(&db.state, &boss, &ctx(), &TokenScope::Server, &eds.id).await.unwrap_err(), TokenError::NothingToRevoke, "a user token is not in the server scope");

    tokens::revoke(&db.state, &ed, &ctx(), &ed_scope, &eds.id).await.unwrap();
    let first: String = db.client.query_one("SELECT revoked_at::text FROM access_tokens WHERE id = $1", &[&eds.id]).await.unwrap().get(0);
    assert_eq!(tokens::revoke(&db.state, &ed, &ctx(), &ed_scope, &eds.id).await.unwrap_err(), TokenError::NothingToRevoke, "already revoked");
    let again: String = db.client.query_one("SELECT revoked_at::text FROM access_tokens WHERE id = $1", &[&eds.id]).await.unwrap().get(0);
    assert_eq!(first, again, "the first revocation time is kept");
    let audit = audit_rows(&db.client, "token:revoke").await;
    assert_eq!(audit.len(), 1);
    assert!(audit[0].get::<_, Option<Value>>("after").unwrap()["revokedAt"].as_str().unwrap().ends_with('Z'));

    tokens::revoke(&db.state, &boss, &ctx(), &TokenScope::Server, &server.id).await.unwrap();
    let err = principal::authenticate_pairing(&db.state, &server.token, &ctx(), principal::now_unix_ms()).await.unwrap_err();
    assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED, "a revoked server token no longer pairs");
}


#[tokio::test]
#[ignore = "real DB"]
async fn q3_w9_new_server_tokens_require_an_expiry() {
    let db=test_db().await; people(&db).await;
    let result=tokens::issue(&db.state,&actor("boss",Role::Admin),&ctx(),TokenKind::Server,value("pair", &["node:pair"],None)).await;
    assert!(result.is_err(),"new pairing tokens must expire");
    assert!(tokens::issue(&db.state,&actor("ed",Role::Editor),&ctx(),TokenKind::User,value("user", &["config:read"],None)).await.is_ok());
}

#[tokio::test]
#[ignore = "real DB"]
async fn issue_requires_current_enrollment_and_session_proof() {
    let db = test_db().await;
    people(&db).await;
    let mut ed = actor("ed", Role::Editor);
    ed.mfa_verified = false;
    assert_eq!(tokens::issue(&db.state, &ed, &ctx(), TokenKind::User, value("unverified", &["config:read"], None)).await.unwrap_err(), TokenError::TwoFactorVerificationRequired);
    db.client.execute("UPDATE users SET totp_enabled=false WHERE id='ed'", &[]).await.unwrap();
    ed.mfa_verified = true;
    assert_eq!(tokens::issue(&db.state, &ed, &ctx(), TokenKind::User, value("unenrolled", &["config:read"], None)).await.unwrap_err(), TokenError::TwoFactorRequired);
    assert_eq!(db.client.query_one("SELECT count(*) FROM access_tokens", &[]).await.unwrap().get::<_, i64>(0), 0);
}

#[tokio::test]
#[ignore = "real DB"]
async fn issue_waits_for_user_lock_and_rechecks_security_changes() {
    for change in ["enabled=false", "session_version=session_version+1", "role='viewer'", "totp_enabled=false", "approved=false"] {
        let db = test_db().await;
        people(&db).await;
        let mut locker = db.state.db.get().await.unwrap();
        let tx = locker.transaction().await.unwrap();
        tx.query_one("SELECT id FROM users WHERE id='ed' FOR UPDATE", &[]).await.unwrap();
        let state = db.state.clone();
        let mut issuing = tokio::spawn(async move { tokens::issue(&state, &actor("ed", Role::Editor), &ctx(), TokenKind::User, value("raced", &["config:write"], None)).await });
        assert!(tokio::time::timeout(std::time::Duration::from_millis(80), &mut issuing).await.is_err(), "issue must wait for user lock");
        tx.execute(&format!("UPDATE users SET {change} WHERE id='ed'"), &[]).await.unwrap();
        tx.commit().await.unwrap();
        assert!(issuing.await.unwrap().is_err(), "{change}");
        assert_eq!(db.client.query_one("SELECT count(*) FROM access_tokens", &[]).await.unwrap().get::<_, i64>(0), 0);
    }
}

#[tokio::test]
#[ignore = "real DB"]
async fn admin_security_changes_revoke_atomically_and_never_revive_tokens() {
    use nwui_api::admin::users;
    for reason in ["password-reset", "account-disabled", "pending-reject"] {
        for fail_audit in [true, false] {
            let db = test_db().await;
            people(&db).await;
            db.client.execute("UPDATE users SET password_hash='old-hash' WHERE id='ed'", &[]).await.unwrap();
            let token = tokens::issue(&db.state, &actor("ed", Role::Editor), &ctx(), TokenKind::User, value("retire", &["config:read"], None)).await.unwrap();
            if reason == "pending-reject" { db.client.execute("UPDATE users SET approved=false WHERE id='ed'", &[]).await.unwrap(); }
            if fail_audit {
                db.client.batch_execute("CREATE FUNCTION fail_token_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='token:revoke' THEN RAISE EXCEPTION 'forced token audit failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER fail_token_audit BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION fail_token_audit()").await.unwrap();
            }
            let boss = actor("boss", Role::Admin);
            let result = match reason {
                "password-reset" => users::reset_password(&db.state, &boss, &ctx(), "ed", "NewPassw0rd!", "NewPassw0rd!", None).await,
                "account-disabled" => users::toggle(&db.state, &boss.clone().into(), &ctx(), "ed").await.map(|_| ()),
                _ => users::reject(&db.state, &boss.clone().into(), &ctx(), "ed").await,
            };
            assert_eq!(result.is_err(), fail_audit, "{reason}");
            let row = db.client.query_one("SELECT revoked_at IS NOT NULL FROM access_tokens WHERE id=$1", &[&token.id]).await.unwrap();
            assert_eq!(row.get::<_, bool>(0), !fail_audit, "{reason}");
            let row = db.client.query_one("SELECT enabled,session_version,password_hash FROM users WHERE id='ed'", &[]).await.unwrap();
            if fail_audit {
                assert!(row.get::<_, bool>(0));
                assert_eq!(row.get::<_, i32>(1), 3);
                assert_eq!(row.get::<_, String>(2), "old-hash");
                assert!(audit_rows(&db.client, "token:revoke").await.is_empty());
            } else {
                let audit = audit_rows(&db.client, "token:revoke").await;
                assert_eq!(audit.len(), 1);
                assert_eq!(audit[0].get::<_, Option<Value>>("after"), Some(json!({"reason":reason,"count":1})));
                if reason == "account-disabled" { users::toggle(&db.state, &boss.clone().into(), &ctx(), "ed").await.unwrap(); }
                db.client.execute("UPDATE users SET totp_enabled=true WHERE id='ed'", &[]).await.unwrap();
                assert!(db.client.query_one("SELECT revoked_at IS NOT NULL FROM access_tokens WHERE id=$1", &[&token.id]).await.unwrap().get::<_, bool>(0));
            }
        }
    }
}

#[tokio::test]
#[ignore = "real DB"]
async fn repeated_security_revocation_has_no_extra_audit_and_preserves_other_token_kinds() {
    let db=test_db().await; people(&db).await;
    let ed=actor("ed",Role::Editor); let boss=actor("boss",Role::Admin);
    let user=tokens::issue(&db.state,&ed,&ctx(),TokenKind::User,value("user",&["config:read"],None)).await.unwrap();
    let server=tokens::issue(&db.state,&boss,&ctx(),TokenKind::Server,value("server",&["node:pair"],Some(principal::now_unix_ms()+60_000))).await.unwrap();
    db.client.batch_execute("INSERT INTO gateway_tokens(id,name,token_hash,role,created_by_user_id) VALUES('gateway','fixture','gateway-hash','editor','ed')").await.unwrap();
    let mut client=db.state.db.get().await.unwrap();let tx=client.transaction().await.unwrap();
    tx.query_one("SELECT id FROM users WHERE id='ed' FOR UPDATE",&[]).await.unwrap();
    assert_eq!(tokens::revoke_user_tokens(&tx,"ed","2fa-disable",&ed.actor(&ctx())).await.unwrap(),1);
    assert_eq!(tokens::revoke_user_tokens(&tx,"ed","2fa-disable",&ed.actor(&ctx())).await.unwrap(),0);
    tx.commit().await.unwrap();
    assert_eq!(audit_rows(&db.client,"token:revoke").await.len(),1);
    assert!(db.client.query_one("SELECT revoked_at IS NOT NULL FROM access_tokens WHERE id=$1",&[&user.id]).await.unwrap().get::<_,bool>(0));
    assert!(db.client.query_one("SELECT revoked_at IS NULL FROM access_tokens WHERE id=$1",&[&server.id]).await.unwrap().get::<_,bool>(0));
    assert!(db.client.query_one("SELECT revoked_at IS NULL FROM gateway_tokens WHERE id='gateway'",&[]).await.unwrap().get::<_,bool>(0));
}

#[test]
fn provenance_protocol_is_typed_and_round_trips_without_relaxing_unknown_fields() {
    let value=json!({"kind":"token","id":"id","name":"<name>"});
    let via: nwui_api::contracts::Via=serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(via).unwrap(),value);
    let actor=json!({"userId":null,"label":"owner@example.test","ip":null,"via":value});
    let decoded:nwui_api::publisher::protocol::Actor=serde_json::from_value(actor.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(),actor);
    assert!(serde_json::from_value::<nwui_api::contracts::Via>(json!({"kind":"token","id":"id","name":"name","extra":true})).is_err());
    assert!(serde_json::from_value::<nwui_api::publisher::protocol::Actor>(json!({"userId":null,"label":null,"ip":null,"extra":true})).is_err());
}

#[tokio::test]
#[ignore = "real DB"]
async fn issue_does_not_return_plaintext_or_persist_a_token_when_audit_fails() {
    let db=test_db().await; people(&db).await;
    db.client.batch_execute("ALTER TABLE audit_logs ADD CONSTRAINT reject_token_issue CHECK(action <> 'token:issue')").await.unwrap();
    assert_eq!(tokens::issue(&db.state,&actor("ed",Role::Editor),&ctx(),TokenKind::User,value("not-committed",&["config:read"],None)).await.unwrap_err(),TokenError::Unavailable);
    assert_eq!(db.client.query_one("SELECT count(*) FROM access_tokens",&[]).await.unwrap().get::<_,i64>(0),0);
}
