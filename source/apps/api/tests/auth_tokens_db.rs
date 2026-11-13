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
    insert_user(&db.client, UserRow { id: "boss", email: "boss@example.test", role: "admin", ..Default::default() }, None).await;
    insert_user(&db.client, UserRow { id: "ed", email: "ed@example.test", role: "editor", ..Default::default() }, None).await;
    insert_user(&db.client, UserRow { id: "vi", email: "vi@example.test", role: "viewer", ..Default::default() }, None).await;
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
