//! OIDC sign-in, linking and unlinking through the root boundary, on a real PostgreSQL database
//! with an in-process identity provider behind the `OidcTransport` seam.
//!
//!     NWUI_TEST_DATABASE_URL=postgres://user:pass@host:5432/postgres cargo test --test auth_oidc -- --include-ignored

mod auth_support;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use auth_support::{TestDb, UserRow, audit_rows, insert_user, test_db};
use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode, header};
use base64::Engine as _;
use josekit::jws::{JwsHeader, RS256};
use josekit::jwt::{self, JwtPayload};
use nwui_api::auth::oidc::{self, BoxFuture, HttpReply, OidcTransport};
use nwui_api::auth::session;
use serde_json::{Value, json};
use tower::ServiceExt as _;

const ISSUER: &str = "https://idp.test";

struct Grant {
    sub: String,
    nonce: String,
    challenge: String,
    redirect_uri: String,
}

struct FakeIdp {
    signer: josekit::jws::alg::rsassa::RsassaJwsSigner,
    jwks: Vec<u8>,
    discovery: Mutex<Value>,
    discovery_calls: Mutex<usize>,
    email_verified: Mutex<Option<Value>>,
    grants: Mutex<HashMap<String, Grant>>,
    token_calls: Mutex<usize>,
    token_barrier: Mutex<Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>>,
}

impl FakeIdp {
    fn new() -> Arc<Self> {
        let pair = RS256.generate_key_pair(2048).unwrap();
        let mut private = pair.to_jwk_private_key();
        private.set_key_id("k1");
        let mut public = pair.to_jwk_public_key();
        public.set_key_id("k1");
        Arc::new(Self {
            signer: RS256.signer_from_jwk(&private).unwrap(),
            jwks: serde_json::to_vec(&json!({"keys": [serde_json::to_value(public.as_ref()).unwrap()]})).unwrap(),
            discovery: Mutex::new(json!({
                "issuer": ISSUER,
                "authorization_endpoint": "https://idp.test/auth?tenant=t1",
                "token_endpoint": "https://idp.test/token",
                "jwks_uri": "https://idp.test/jwks",
                "id_token_signing_alg_values_supported": ["RS256"],
            })),
            discovery_calls: Mutex::new(0),
            email_verified: Mutex::new(Some(json!(true))),
            grants: Mutex::new(HashMap::new()),
            token_calls: Mutex::new(0),
            token_barrier: Mutex::new(None),
        })
    }

    fn reply(status: u16, body: Value) -> Result<HttpReply, String> {
        Ok(HttpReply { status, body: serde_json::to_vec(&body).unwrap() })
    }
}

fn form_fields(body: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(body.as_bytes()).into_owned().collect()
}

impl OidcTransport for FakeIdp {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<HttpReply, String>> {
        Box::pin(async move {
            match url {
                "https://idp.test/.well-known/openid-configuration" => { *self.discovery_calls.lock().unwrap() += 1; Self::reply(200, self.discovery.lock().unwrap().clone()) },
                "https://idp.test/jwks" => Ok(HttpReply { status: 200, body: self.jwks.clone() }),
                _ => Self::reply(404, json!({})),
            }
        })
    }

    fn post_form<'a>(&'a self, url: &'a str, authorization: &'a str, body: &'a str) -> BoxFuture<'a, Result<HttpReply, String>> {
        Box::pin(async move {
            *self.token_calls.lock().unwrap() += 1;
            assert_eq!(url, "https://idp.test/token");
            // RFC 6749 §2.3.1: form-encoded, then base64.
            let expected = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("cid:s%26c%3Dret"));
            if authorization != expected {
                return Self::reply(401, json!({"error": "invalid_client"}));
            }
            let fields = form_fields(body);
            let Some(grant) = fields.get("code").and_then(|code| self.grants.lock().unwrap().remove(code)) else {
                return Self::reply(400, json!({"error": "invalid_grant"}));
            };
            if fields.get("grant_type").map(String::as_str) != Some("authorization_code")
                || fields.get("redirect_uri") != Some(&grant.redirect_uri)
                || fields.get("code_verifier").map(|v| oidc::pkce_challenge(v)) != Some(grant.challenge.clone())
            {
                return Self::reply(400, json!({"error": "invalid_grant"}));
            }
            let barrier = self.token_barrier.lock().unwrap().clone();
            if let Some((entered, resume)) = barrier {
                entered.notify_one();
                resume.notified().await;
            }
            let now = nwui_api::auth::principal::now_unix_ms() / 1000;
            let mut claims = json!({"iss": ISSUER, "aud": "cid", "sub": grant.sub, "nonce": grant.nonce, "iat": now, "exp": now + 300, "name": "Kim", "email": "kim@idp.test"});
            if let Some(verified) = self.email_verified.lock().unwrap().clone() { claims["email_verified"] = verified; }
            let mut header = JwsHeader::new();
            header.set_token_type("JWT");
            let id_token = jwt::encode_with_signer(&JwtPayload::from_map(claims.as_object().unwrap().clone()).unwrap(), &header, &self.signer).unwrap();
            Self::reply(200, json!({"access_token": "at", "token_type": "Bearer", "id_token": id_token}))
        })
    }
}

struct Harness {
    db: TestDb,
    idp: Arc<FakeIdp>,
    app: Router,
}

async fn harness() -> Harness {
    let db = test_db().await;
    let secret = db.state.secrets.seal("s&c=ret").unwrap();
    db.client
        .execute("INSERT INTO oauth_providers (id, name, issuer, client_id, client_secret_enc) VALUES ('kc', 'Keycloak', $1, 'cid', $2)", &[&ISSUER, &secret])
        .await
        .unwrap();
    let idp = FakeIdp::new();
    let app = nwui_api::app::with_request_context(oidc::router(idp.clone()).with_state(db.state.clone()), None);
    Harness { db, idp, app }
}

fn session_cookie(id: &str) -> String {
    let now = nwui_api::auth::principal::now_unix_ms() / 1000;
    let claims = json!({"sub": id, "id": id, "sv": 3, "requires2fa": false});
    let token = session::encode(claims.as_object().unwrap(), auth_support::SECRET, session::COOKIE_NAME_PLAIN, now, 3600).unwrap();
    format!("authjs.session-token={token}")
}

struct Reply {
    status: StatusCode,
    location: Option<String>,
    cookies: Vec<String>,
    body: Value,
}

impl Reply {
    fn cookie(&self, name: &str) -> Option<String> {
        self.cookies.iter().find_map(|c| c.strip_prefix(&format!("{name}=")).map(|rest| rest.split(';').next().unwrap().to_owned()))
    }
}

async fn send(app: &Router, method: &str, path: &str, cookie: &str, form: Option<&str>) -> Reply {
    send_at(app,method,path,cookie,form,"203.0.113.20").await
}

async fn send_at(app: &Router, method: &str, path: &str, cookie: &str, form: Option<&str>, ip: &str) -> Reply {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "admin.example.test")
        .header("x-real-ip", ip)
        .header("cookie", cookie)
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));
    if form.is_some() {
        builder = builder.header("origin", "http://admin.example.test").header("content-type", "application/x-www-form-urlencoded");
    }
    let response = app.clone().oneshot(builder.body(Body::from(form.unwrap_or("").to_owned())).unwrap()).await.unwrap();
    let status = response.status();
    let location = response.headers().get(header::LOCATION).map(|v| v.to_str().unwrap().to_owned());
    let cookies = response.headers().get_all(header::SET_COOKIE).iter().map(|v| v.to_str().unwrap().to_owned()).collect();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let body = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
    Reply { status, location, cookies, body }
}

/// Starts a sign-in (or a link with `session`), lets the provider grant `sub`, and returns the
/// callback path and the cookie header to send with it.
async fn authorize(h: &Harness, command: &str, session: &str, sub: &str) -> (String, String) {
    let start = send(&h.app, "POST", &format!("/api/ui/v1/forms/{command}"), session, Some("providerId=kc")).await;
    assert_eq!(start.status, StatusCode::OK, "{}", start.body);
    let to = url::Url::parse(start.body["redirectTo"].as_str().unwrap()).unwrap();
    let query: HashMap<String, String> = to.query_pairs().into_owned().collect();
    let tx = start.cookie(oidc::TX_COOKIE_PLAIN).unwrap();
    h.idp.grants.lock().unwrap().insert(
        format!("code-{sub}"),
        Grant { sub: sub.into(), nonce: query["nonce"].clone(), challenge: query["code_challenge"].clone(), redirect_uri: query["redirect_uri"].clone() },
    );
    let callback = format!("/api/auth/callback/kc?code=code-{sub}&state={}", query["state"]);
    let cookie = if session.is_empty() { format!("{}={tx}", oidc::TX_COOKIE_PLAIN) } else { format!("{session}; {}={tx}", oidc::TX_COOKIE_PLAIN) };
    (callback, cookie)
}

fn tx_cleared(reply: &Reply) -> bool {
    reply.cookies.iter().any(|c| c.starts_with(&format!("{}=;", oidc::TX_COOKIE_PLAIN)) && c.contains("Max-Age=0") && c.contains("Path=/api/auth/callback"))
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn the_authorization_request_carries_state_nonce_and_pkce() {
    let h = harness().await;
    let start = send(&h.app, "POST", "/api/ui/v1/forms/login.oauthSignIn", "", Some("providerId=kc")).await;
    assert_eq!(start.status, StatusCode::OK);
    let to = url::Url::parse(start.body["redirectTo"].as_str().unwrap()).unwrap();
    assert_eq!((to.scheme(), to.host_str(), to.path()), ("https", Some("idp.test"), "/auth"));
    let q: HashMap<String, String> = to.query_pairs().into_owned().collect();
    assert_eq!(q["tenant"], "t1", "the endpoint's own query is kept");
    assert_eq!((q["response_type"].as_str(), q["client_id"].as_str(), q["scope"].as_str()), ("code", "cid", "openid email profile"));
    assert_eq!((q["redirect_uri"].as_str(), q["code_challenge_method"].as_str()), ("http://admin.example.test/api/auth/callback/kc", "S256"));
    assert!(q["state"].len() >= 43 && q["nonce"].len() >= 43 && q["code_challenge"].len() == 43);
    let set = start.cookies.iter().find(|c| c.starts_with("nwui-oauth-tx=")).unwrap();
    assert!(set.contains("Path=/api/auth/callback") && set.contains("HttpOnly") && set.contains("SameSite=Lax") && set.contains("Max-Age=600"), "{set}");
    let raw = start.cookie(oidc::TX_COOKIE_PLAIN).unwrap();
    assert!(!raw.contains(&q["nonce"]) && !raw.contains(&q["state"]), "the transaction is encrypted");

    // The public base URL, when set, is the registered callback.
    h.db.client.execute("INSERT INTO app_settings (id, public_base_url) VALUES ('current', 'https://nginx.example.test/') ON CONFLICT (id) DO UPDATE SET public_base_url = EXCLUDED.public_base_url", &[]).await.unwrap();
    let start = send(&h.app, "POST", "/api/ui/v1/forms/login.oauthSignIn", "", Some("providerId=kc")).await;
    let to = url::Url::parse(start.body["redirectTo"].as_str().unwrap()).unwrap();
    assert!(to.query_pairs().any(|(k, v)| k == "redirect_uri" && v == "https://nginx.example.test/api/auth/callback/kc"));

    assert_eq!(send(&h.app, "POST", "/api/ui/v1/forms/login.oauthSignIn", "", Some("providerId=nope")).await.status, StatusCode::NOT_FOUND);
    h.db.client.execute("UPDATE oauth_providers SET enabled = false", &[]).await.unwrap();
    assert_eq!(send(&h.app, "POST", "/api/ui/v1/forms/login.oauthSignIn", "", Some("providerId=kc")).await.status, StatusCode::NOT_FOUND);
    h.db.client.execute("UPDATE oauth_providers SET enabled = true", &[]).await.unwrap();
    h.idp.discovery.lock().unwrap()["issuer"] = json!("https://evil.test");
    let failed = send(&h.app, "POST", "/api/ui/v1/forms/login.oauthSignIn", "", Some("providerId=kc")).await;
    assert_eq!((failed.status, failed.body["error"]["code"].as_str()), (StatusCode::BAD_GATEWAY, Some("upstream_failed")));
    assert!(failed.cookies.is_empty());
    // Unauthenticated callers learn that the provider failed, not what it or the network said.
    assert!(!failed.body.to_string().contains("evil.test"), "{}", failed.body);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_new_identity_waits_for_approval_then_signs_in() {
    let h = harness().await;
    let (callback, cookie) = authorize(&h, "login.oauthSignIn", "", "alice").await;
    let reply = send(&h.app, "GET", &callback, &cookie, None).await;
    assert_eq!((reply.status, reply.location.as_deref()), (StatusCode::FOUND, Some("/login?pending=1")));
    assert!(tx_cleared(&reply) && reply.cookie(session::COOKIE_NAME_PLAIN).is_none(), "no session before approval");
    let user = h.db.client.query_one("SELECT u.id, u.role::text, u.approved, u.email, a.display_name, a.email FROM oauth_accounts a JOIN users u ON u.id = a.user_id WHERE a.provider_id = 'kc' AND a.subject = 'alice'", &[]).await.unwrap();
    let id: String = user.get(0);
    assert_eq!((user.get::<_, String>(1).as_str(), user.get::<_, bool>(2), user.get::<_, Option<String>>(3)), ("viewer", false, None), "never matched or created by email");
    assert_eq!((user.get::<_, Option<String>>(4).as_deref(), user.get::<_, Option<String>>(5).as_deref()), (Some("Kim"), Some("kim@idp.test")));
    assert_eq!(audit_rows(&h.db.client, "account:oauth-signup").await.len(), 1);

    // Replaying the same callback: the code is single-use at the provider.
    let replay = send(&h.app, "GET", &callback, &cookie, None).await;
    assert_eq!(replay.location.as_deref(), Some(oidc::LOGIN_FAILED));
    // Signing in again before approval still opens nothing.
    let (callback, cookie) = authorize(&h, "login.oauthSignIn", "", "alice").await;
    let reply = send(&h.app, "GET", &callback, &cookie, None).await;
    assert_eq!(reply.location.as_deref(), Some("/login?pending=1"));
    assert!(reply.cookie(session::COOKIE_NAME_PLAIN).is_none());
    assert_eq!(audit_rows(&h.db.client, "account:oauth-signup").await.len(), 1, "no second account");

    h.db.client.execute("UPDATE users SET approved = true WHERE id = $1", &[&id]).await.unwrap();
    h.db.client.execute("INSERT INTO app_settings (id, require_2fa_oauth) VALUES ('current', true) ON CONFLICT (id) DO UPDATE SET require_2fa_oauth = true", &[]).await.unwrap();
    let (callback, cookie) = authorize(&h, "login.oauthSignIn", "", "alice").await;
    let reply = send(&h.app, "GET", &callback, &cookie, None).await;
    assert_eq!(reply.location.as_deref(), Some("/"));
    assert!(tx_cleared(&reply));
    let token = reply.cookie(session::COOKIE_NAME_PLAIN).unwrap();
    let now = nwui_api::auth::principal::now_unix_ms() / 1000;
    let claims = session::decode(&token, &[auth_support::SECRET], session::COOKIE_NAME_PLAIN, now).unwrap();
    assert_eq!((claims["id"].as_str(), claims["role"].as_str(), &claims["requires2fa"]), (Some(id.as_str()), Some("viewer"), &json!(true)), "the OAuth 2FA switch, not the role");
    assert_eq!(h.db.client.query_one("SELECT count(*) FROM users", &[]).await.unwrap().get::<_, i64>(0), 1, "the same identity is the same account");
    assert_eq!(audit_rows(&h.db.client, "login").await[0].get::<_, Option<String>>("actor_id").as_deref(), Some(id.as_str()));

    h.db.client.execute("UPDATE users SET enabled = false WHERE id = $1", &[&id]).await.unwrap();
    let (callback, cookie) = authorize(&h, "login.oauthSignIn", "", "alice").await;
    let reply = send(&h.app, "GET", &callback, &cookie, None).await;
    assert_eq!(reply.location.as_deref(), Some("/login?error=disabled"));
    assert!(reply.cookie(session::COOKIE_NAME_PLAIN).is_none());
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn an_enrolled_oauth_user_must_still_pass_this_apps_second_factor_when_required() {
    let h = harness().await;
    insert_user(&h.db.client, UserRow { id: "u", email: "u@example.test", totp_enabled: true, ..Default::default() }, None).await;
    h.db.client.execute("INSERT INTO oauth_accounts (provider_id, subject, user_id) VALUES ('kc', 'alice', 'u')", &[]).await.unwrap();
    h.db.client.execute("INSERT INTO app_settings (id, require_2fa_oauth) VALUES ('current', true)", &[]).await.unwrap();
    let (callback, cookie) = authorize(&h, "login.oauthSignIn", "", "alice").await;
    let reply = send(&h.app, "GET", &callback, &cookie, None).await;
    assert_eq!(reply.location.as_deref(), Some("/"));
    let token = reply.cookie(session::COOKIE_NAME_PLAIN).unwrap();
    let now = nwui_api::auth::principal::now_unix_ms() / 1000;
    let claims = session::decode(&token, &[auth_support::SECRET], session::COOKIE_NAME_PLAIN, now).unwrap();
    assert_eq!((&claims["requires2fa"], claims.get("mfaVerified")), (&json!(true), Some(&json!(false))), "the provider's login is not this app's factor");
    let guarded = nwui_api::auth::principal::require_user(
        &h.db.state,
        &{
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(header::COOKIE, format!("{}={token}", session::COOKIE_NAME_PLAIN).parse().unwrap());
            headers
        },
        &auth_support::ctx(),
        None,
        nwui_api::auth::principal::TwoFactorGate::Enforce, nwui_api::auth::principal::Use::SessionOnly)
    .await;
    assert_eq!(guarded.unwrap_err().code, "two_factor_required");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn callbacks_that_do_not_match_their_request_are_refused_before_the_token_call() {
    let h = harness().await;
    // A second provider at the same issuer: a transaction for one must not complete at the other
    // (mix-up, RFC 9700 §4.4).
    let secret = h.db.state.secrets.seal("s&c=ret").unwrap();
    h.db.client
        .execute("INSERT INTO oauth_providers (id, name, issuer, client_id, client_secret_enc) VALUES ('other', 'Other', $1, 'cid', $2)", &[&ISSUER, &secret])
        .await
        .unwrap();
    let (callback, cookie) = authorize(&h, "login.oauthSignIn", "", "alice").await;
    for (name, path, cookie) in [
        ("no cookie", callback.clone(), String::new()),
        ("forged cookie", callback.clone(), format!("{}=x.y.z.w.v", oidc::TX_COOKIE_PLAIN)),
        ("other state", callback.replace("state=", "state=x"), cookie.clone()),
        ("no code", callback.replace("code=code-alice&", ""), cookie.clone()),
        ("other provider", callback.replace("/kc?", "/other?"), cookie.clone()),
    ] {
        let reply = send(&h.app, "GET", &path, &cookie, None).await;
        assert_eq!((reply.status, reply.location.as_deref()), (StatusCode::FOUND, Some(oidc::LOGIN_FAILED)), "{name}");
        assert!(tx_cleared(&reply), "{name}");
    }
    assert_eq!(*h.idp.token_calls.lock().unwrap(), 0);
    let (_, cookie) = authorize(&h, "login.oauthSignIn", "", "alice").await;
    let denied = send(&h.app, "GET", "/api/auth/callback/kc?error=access_denied&state=x", &cookie, None).await;
    assert_eq!(denied.location.as_deref(), Some(oidc::LOGIN_DENIED));
    // A new authorization still works; the cancelled request cannot be reused.
    let (callback, cookie) = authorize(&h, "login.oauthSignIn", "", "alice").await;
    assert_eq!(send(&h.app, "GET", &callback, &cookie, None).await.location.as_deref(), Some("/login?pending=1"));
    assert_eq!(*h.idp.token_calls.lock().unwrap(), 1);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn linking_attaches_an_identity_to_the_signed_in_user_only() {
    let h = harness().await;
    insert_user(&h.db.client, UserRow { id: "u", email: "u@example.test", ..Default::default() }, Some("x")).await;
    insert_user(&h.db.client, UserRow { id: "v", email: "v@example.test", ..Default::default() }, Some("x")).await;
    let (u, v) = (session_cookie("u"), session_cookie("v"));

    assert_eq!(send(&h.app, "POST", "/api/ui/v1/forms/account.startLink", "", Some("providerId=kc")).await.status, StatusCode::UNAUTHORIZED);
    let (callback, cookie) = authorize(&h, "account.startLink", &u, "bob").await;
    let reply = send(&h.app, "GET", &callback, &cookie, None).await;
    assert_eq!(reply.location.as_deref(), Some("/account?linked=1"));
    assert!(tx_cleared(&reply) && reply.cookie(session::COOKIE_NAME_PLAIN).is_none(), "the session stays as it was");
    let owner: String = h.db.client.query_one("SELECT user_id FROM oauth_accounts WHERE provider_id = 'kc' AND subject = 'bob'", &[]).await.unwrap().get(0);
    assert_eq!(owner, "u");
    assert_eq!(audit_rows(&h.db.client, "account:oauth-link").await.len(), 1);
    *h.idp.email_verified.lock().unwrap() = Some(json!(false));
    let (callback, cookie) = authorize(&h, "account.startLink", &u, "bob").await;
    assert_eq!(send(&h.app, "GET", &callback, &cookie, None).await.location.as_deref(), Some("/account?linked=1"), "linking what is mine again is harmless");
    assert!(!h.db.client.query_one("SELECT email_verified FROM oauth_accounts WHERE provider_id='kc' AND subject='bob'",&[]).await.unwrap().get::<_,bool>(0));
    *h.idp.email_verified.lock().unwrap() = Some(json!(true));
    let (callback,cookie)=authorize(&h,"account.startLink",&u,"bob").await;
    assert_eq!(send(&h.app,"GET",&callback,&cookie,None).await.location.as_deref(),Some("/account?linked=1"));
    assert!(h.db.client.query_one("SELECT email_verified FROM oauth_accounts WHERE provider_id='kc' AND subject='bob'",&[]).await.unwrap().get::<_,bool>(0));
    assert_eq!(audit_rows(&h.db.client, "account:oauth-link").await.len(), 1);

    let (callback, cookie) = authorize(&h, "account.startLink", &v, "bob").await;
    assert_eq!(send(&h.app, "GET", &callback, &cookie, None).await.location.as_deref(), Some(oidc::LINK_TAKEN));
    // The link intent is for the user who started it: no session, or another one, is refused.
    let (callback, cookie) = authorize(&h, "account.startLink", &v, "carol").await;
    let tx_only = cookie.split("; ").find(|c| c.starts_with(oidc::TX_COOKIE_PLAIN)).unwrap().to_owned();
    assert_eq!(send(&h.app, "GET", &callback, &tx_only, None).await.location.as_deref(), Some(oidc::LINK_FAILED));
    let (callback, cookie) = authorize(&h, "account.startLink", &v, "carol").await;
    let swapped = cookie.replace(&v, &u);
    assert_eq!(send(&h.app, "GET", &callback, &swapped, None).await.location.as_deref(), Some(oidc::LINK_FAILED));
    assert_eq!(h.db.client.query_one("SELECT count(*) FROM oauth_accounts WHERE subject = 'carol'", &[]).await.unwrap().get::<_, i64>(0), 0);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn unlinking_never_removes_the_last_way_in() {
    let h = harness().await;
    h.db.client.execute("INSERT INTO oauth_providers (id, name, issuer, client_id, client_secret_enc) VALUES ('gh', 'Other', 'https://other.test', 'cid', 'x')", &[]).await.unwrap();
    insert_user(&h.db.client, UserRow { id: "pw", email: "pw@example.test", ..Default::default() }, Some("x")).await;
    insert_user(&h.db.client, UserRow { id: "only", email: "only@example.test", ..Default::default() }, None).await;
    h.db.client
        .batch_execute("INSERT INTO oauth_accounts (provider_id, subject, user_id) VALUES ('kc', 'a', 'pw'), ('kc', 'b', 'only'), ('gh', 'c', 'only')")
        .await
        .unwrap();
    let unlink = "/api/ui/v1/forms/account.unlink";

    let reply = send(&h.app, "POST", unlink, &session_cookie("pw"), Some("providerId=kc")).await;
    assert_eq!((reply.status, &reply.body), (StatusCode::OK, &json!({"redirectTo": "/account?unlinked=1"})));
    assert_eq!(audit_rows(&h.db.client, "account:oauth-unlink").await.len(), 1);

    let only = session_cookie("only");
    assert_eq!(send(&h.app, "POST", unlink, &only, Some("providerId=kc")).await.status, StatusCode::OK, "another provider remains");
    let reply = send(&h.app, "POST", unlink, &only, Some("providerId=gh")).await;
    assert_eq!((reply.status, reply.body["error"]["message"].as_str()), (StatusCode::CONFLICT, Some(oidc::LAST_LOGIN_METHOD)));
    assert_eq!(h.db.client.query_one("SELECT count(*) FROM oauth_accounts WHERE user_id = 'only'", &[]).await.unwrap().get::<_, i64>(0), 1);
    let reply = send(&h.app, "POST", unlink, &only, Some("providerId=kc")).await;
    assert_eq!(reply.status, StatusCode::OK, "nothing linked for that provider is nothing to refuse");
    assert_eq!(audit_rows(&h.db.client, "account:oauth-unlink").await.len(), 2);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn concurrent_unlinks_cannot_strand_an_account() {
    let h = harness().await;
    h.db.client.execute("INSERT INTO oauth_providers (id, name, issuer, client_id, client_secret_enc) VALUES ('gh', 'Other', 'https://other.test', 'cid', 'x')", &[]).await.unwrap();
    for round in 0..5 {
        let user = format!("only{round}");
        insert_user(&h.db.client, UserRow { id: &user, email: &format!("{user}@example.test"), ..Default::default() }, None).await;
        h.db.client
            .execute("INSERT INTO oauth_accounts (provider_id, subject, user_id) VALUES ('kc', $1, $1), ('gh', $1, $1)", &[&user])
            .await
            .unwrap();
        let cookie = session_cookie(&user);
        let (a, b) = tokio::join!(
            send(&h.app, "POST", "/api/ui/v1/forms/account.unlink", &cookie, Some("providerId=kc")),
            send(&h.app, "POST", "/api/ui/v1/forms/account.unlink", &cookie, Some("providerId=gh")),
        );
        let left: i64 = h.db.client.query_one("SELECT count(*) FROM oauth_accounts WHERE user_id = $1", &[&user]).await.unwrap().get(0);
        assert_eq!(left, 1, "round {round}: {:?} {:?}", a.status, b.status);
    }
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn oauth_sign_in_follows_the_role_policy_and_the_users_own_enrollment() {
    // The provider's login is not this app's factor. Besides the OAuth switch, the role's 2FA
    // policy and a TOTP the user enrolled themselves also apply, as they do for a password
    // sign-in — otherwise a taken-over IdP account is a full admin session.
    let h = harness().await;
    insert_user(&h.db.client, UserRow { id: "admin", email: "admin@example.test", role: "admin", ..Default::default() }, None).await;
    insert_user(&h.db.client, UserRow { id: "enrolled", email: "enrolled@example.test", totp_enabled: true, ..Default::default() }, None).await;
    insert_user(&h.db.client, UserRow { id: "plain", email: "plain@example.test", ..Default::default() }, None).await;
    for (subject, user) in [("a", "admin"), ("e", "enrolled"), ("p", "plain")] {
        h.db.client.execute("INSERT INTO oauth_accounts (provider_id, subject, user_id) VALUES ('kc', $1, $2)", &[&subject, &user]).await.unwrap();
    }
    h.db.client.execute("INSERT INTO app_settings (id, require_2fa_oauth, require_2fa_admin, require_2fa_user) VALUES ('current', false, true, false)", &[]).await.unwrap();
    let claims_for = |subject: &'static str| {
        let h = &h;
        async move {
            let (callback, cookie) = authorize(h, "login.oauthSignIn", "", subject).await;
            let reply = send(&h.app, "GET", &callback, &cookie, None).await;
            assert_eq!(reply.location.as_deref(), Some("/"), "{subject}");
            let token = reply.cookie(session::COOKIE_NAME_PLAIN).unwrap();
            let now = nwui_api::auth::principal::now_unix_ms() / 1000;
            session::decode(&token, &[auth_support::SECRET], session::COOKIE_NAME_PLAIN, now).unwrap()
        }
    };
    let guard = |claims: serde_json::Map<String, serde_json::Value>| {
        let h = &h;
        async move {
            let now = nwui_api::auth::principal::now_unix_ms() / 1000;
            let token = session::encode(&claims, auth_support::SECRET, session::COOKIE_NAME_PLAIN, now, 3600).unwrap();
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(header::COOKIE, format!("{}={token}", session::COOKIE_NAME_PLAIN).parse().unwrap());
            nwui_api::auth::principal::require_user(&h.db.state, &headers, &auth_support::ctx(), None, nwui_api::auth::principal::TwoFactorGate::Enforce, nwui_api::auth::principal::Use::SessionOnly)
                .await
                .map(|_| ())
                .map_err(|e| e.code)
        }
    };
    assert_eq!(claims_for("a").await["requires2fa"], json!(true), "the admin role policy applies to OAuth too");

    let enrolled = claims_for("e").await;
    assert_eq!(enrolled["requires2fa"], json!(false), "requires2fa carries policy only");
    assert_eq!(guard(enrolled.clone()).await, Err("two_factor_required".into()), "an enrolled user proves the code");
    // Disabling 2FA later must not trap the same session in forced enrollment.
    h.db.client.execute("UPDATE users SET totp_enabled = false WHERE id = 'enrolled'", &[]).await.unwrap();
    assert_eq!(guard(enrolled).await, Ok(()));

    let plain = claims_for("p").await;
    assert_eq!(plain["requires2fa"], json!(false));
    assert_eq!(guard(plain).await, Ok(()), "no policy and not enrolled: nothing to ask");
}

async fn issuer_change_during_exchange_is_refused(link: bool) {
    let h = harness().await;
    if link { insert_user(&h.db.client, UserRow::default(), Some("hash")).await; }
    let session = if link { session_cookie("u1") } else { String::new() };
    let (callback, cookie) = authorize(&h, if link { "account.startLink" } else { "login.oauthSignIn" }, &session, "old-subject").await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    *h.idp.token_barrier.lock().unwrap() = Some((entered.clone(), resume.clone()));
    let app = h.app.clone();
    let pending = tokio::spawn(async move { send(&app, "GET", &callback, &cookie, None).await });
    tokio::time::timeout(std::time::Duration::from_secs(3), entered.notified()).await.unwrap();
    h.db.client.execute("UPDATE oauth_providers SET issuer='https://new.example.test' WHERE id='kc'", &[]).await.unwrap();
    resume.notify_one();
    let reply = pending.await.unwrap();
    assert_eq!(reply.location.as_deref(), Some(if link { oidc::LINK_FAILED } else { oidc::LOGIN_FAILED }), "a callback verified against the old issuer must not attach or resolve an identity under the new one");
    assert!(tx_cleared(&reply));
    assert_eq!(h.db.client.query_one("SELECT count(*) FROM oauth_accounts", &[]).await.unwrap().get::<_, i64>(0), 0);
    assert_eq!(h.db.client.query_one("SELECT count(*) FROM users", &[]).await.unwrap().get::<_, i64>(0), if link { 1 } else { 0 });
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_s5_inflight_signin_cannot_cross_an_issuer_change() {
    issuer_change_during_exchange_is_refused(false).await;
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_s5_inflight_link_cannot_cross_an_issuer_change() {
    issuer_change_during_exchange_is_refused(true).await;
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn anonymous_oidc_discovery_is_bounded_per_ip() {
    let h = harness().await;
    let mut jobs = tokio::task::JoinSet::new();
    for _ in 0..35 {
        let app = h.app.clone();
        jobs.spawn(async move { send(&app,"POST","/api/ui/v1/forms/login.oauthSignIn","",Some("providerId=kc")).await.status });
    }
    let mut limited = 0;
    while let Some(result) = jobs.join_next().await { if result.unwrap() == StatusCode::TOO_MANY_REQUESTS { limited += 1; } }
    assert_eq!(limited, 5);
    assert_eq!(*h.idp.discovery_calls.lock().unwrap(), 30);
    assert_eq!(send_at(&h.app,"POST","/api/ui/v1/forms/login.oauthSignIn","",Some("providerId=kc"),"203.0.113.21").await.status,StatusCode::OK);
    assert!(h.db.state.runtime.login_throttle.begin_ip("203.0.113.20",nwui_api::auth::principal::now_unix_ms()).is_some(),"OIDC must not consume password attempts");
    h.idp.discovery.lock().unwrap()["issuer"] = json!("https://wrong.test");
    for _ in 0..30 {
        assert_eq!(send_at(&h.app,"POST","/api/ui/v1/forms/login.oauthSignIn","",Some("providerId=kc"),"203.0.113.22").await.status,StatusCode::BAD_GATEWAY);
    }
    assert_eq!(send_at(&h.app,"POST","/api/ui/v1/forms/login.oauthSignIn","",Some("providerId=kc"),"203.0.113.22").await.status,StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(*h.idp.discovery_calls.lock().unwrap(),61);
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn oidc_email_display_tracks_verified_callbacks() {
    let h = harness().await;
    for verified in [false, true, false] {
        *h.idp.email_verified.lock().unwrap() = Some(json!(verified));
        let (path,cookie) = authorize(&h,"login.oauthSignIn","","verified-sub").await;
        let reply = send(&h.app,"GET",&path,&cookie,None).await;
        assert_eq!(reply.status, StatusCode::FOUND);
        let users = nwui_api::admin::users::list(&h.db.state).await.unwrap();
        assert_eq!(users.len(),1);
        assert_eq!(users[0].email.as_deref(), if verified {Some("kim@idp.test")} else {None});
        assert!(!users[0].approved);
    }
    let row=h.db.client.query_one("SELECT u.email, a.email, a.email_verified FROM users u JOIN oauth_accounts a ON a.user_id=u.id",&[]).await.unwrap();
    assert!(row.get::<_,Option<String>>(0).is_none());
    assert_eq!(row.get::<_,Option<String>>(1).as_deref(),Some("kim@idp.test"));
    assert!(!row.get::<_,bool>(2));
}


#[tokio::test]
#[ignore = "real DB"]
async fn q3_i2_first_callback_consumes_even_a_failed_transaction() {
    let h=harness().await;
    let (callback,cookie)=authorize(&h,"login.oauthSignIn","","replay").await;
    let cancelled=send(&h.app,"GET","/api/auth/callback/kc?error=access_denied",&cookie,None).await;
    assert!(tx_cleared(&cancelled));
    let replay=send(&h.app,"GET",&callback,&cookie,None).await;
    assert_eq!(replay.location.as_deref(),Some(oidc::LOGIN_FAILED));
    assert!(h.db.client.query_opt("SELECT id FROM users WHERE email='replay@example.test'",&[]).await.unwrap().is_none());
}

#[tokio::test]
#[ignore = "real DB"]
async fn q3_w14_secure_transaction_cookie_is_host_only() {
    let h=harness().await;
    let req=Request::builder().method("POST").uri("/api/ui/v1/forms/login.oauthSignIn")
        .header("host","admin.example.test").header("x-forwarded-proto","https")
        .header("origin","https://admin.example.test").header("content-type","application/x-www-form-urlencoded")
        .extension(ConnectInfo(SocketAddr::from(([127,0,0,1],40000))))
        .body(Body::from("providerId=kc")).unwrap();
    let response=h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(),StatusCode::OK);
    let cookie=response.headers().get("set-cookie").unwrap().to_str().unwrap();
    assert!(cookie.starts_with("__Host-nwui-oauth-tx="),"{cookie}");
    assert!(cookie.contains("Path=/;") && cookie.contains("; Secure") && !cookie.contains("Domain="));
}
