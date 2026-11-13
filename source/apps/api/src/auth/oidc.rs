//! OpenID Connect sign-in and account linking (Auth.js `type: "oidc"` providers from
//! `packages/auth/src/auth.ts`, `oauth-link.ts`, `link-ticket.ts`; `app/login/oauth-actions.ts`,
//! `app/(app)/account/link-actions.ts`).
//!
//! Authorization code flow (OIDC Core 1.0 §3.1) with discovery (OIDC Discovery 1.0 §4).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;

use crate::state::AppState;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpReply {
    pub status: u16,
    pub body: Vec<u8>,
}

/// The only way this module talks to an identity provider — the seam tests replace.
pub trait OidcTransport: Send + Sync + 'static {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<HttpReply, String>>;
    /// POST `application/x-www-form-urlencoded` with an `Authorization` header value.
    fn post_form<'a>(&'a self, url: &'a str, authorization: &'a str, body: &'a str) -> BoxFuture<'a, Result<HttpReply, String>>;
}

pub type Transport = Arc<dyn OidcTransport>;

/// Largest provider response read (discovery, JWKS, token): all are a few KiB.
const REPLY_MAX: usize = 1024 * 1024;

/// reqwest with redirects off (a discovery or token answer must come from the URL we chose), no
/// proxy (like Node `fetch`), a 10 s timeout and a bounded body.
struct ReqwestTransport(reqwest::Client);

impl ReqwestTransport {
    async fn read(response: Result<reqwest::Response, reqwest::Error>) -> Result<HttpReply, String> {
        // Only the host ever reaches the layers under reqwest; drop the URL (query may carry a code).
        let mut response = response.map_err(|e| e.without_url().to_string())?;
        let status = response.status().as_u16();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url().to_string())? {
            if body.len() + chunk.len() > REPLY_MAX {
                return Err("응답이 너무 큽니다.".into());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpReply { status, body })
    }
}

impl OidcTransport for ReqwestTransport {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<HttpReply, String>> {
        Box::pin(async move { Self::read(self.0.get(url).header("accept", "application/json").send().await).await })
    }

    fn post_form<'a>(&'a self, url: &'a str, authorization: &'a str, body: &'a str) -> BoxFuture<'a, Result<HttpReply, String>> {
        Box::pin(async move {
            let sent = self
                .0
                .post(url)
                .header("accept", "application/json")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("authorization", authorization)
                .body(body.to_owned())
                .send()
                .await;
            Self::read(sent).await
        })
    }
}

pub fn default_transport() -> Transport {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("an OIDC HTTP client with default TLS roots");
    Arc::new(ReqwestTransport(client))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OidcError {
    /// The provider could not be reached or answered with something unusable.
    Unavailable(String),
    /// A check failed (state, nonce, signature, claims…).
    Invalid(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    pub signing_algs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityClaims {
    pub subject: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
}

/// oauth4webapi's default `clockTolerance` (what Auth.js validated with).
pub const CLOCK_TOLERANCE_SECS: i64 = 30;

/// Asymmetric JWS algorithms only (RFC 7518 §3.1): `none` and HMAC keyed with a public value are
/// the classic ID-token forgeries.
const ASYMMETRIC_ALGS: [&str; 10] = ["RS256", "RS384", "RS512", "PS256", "PS384", "PS512", "ES256", "ES384", "ES512", "EdDSA"];

fn invalid(message: &str) -> OidcError {
    OidcError::Invalid(message.to_owned())
}

fn unavailable(message: impl Into<String>) -> OidcError {
    OidcError::Unavailable(message.into())
}

fn b64url(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// 32 random octets, base64url — state, nonce and the PKCE verifier (RFC 7636 §4.1: 43 chars).
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("operating system randomness");
    b64url(&bytes)
}

/// RFC 7636 §4.2 `S256`.
pub fn pkce_challenge(verifier: &str) -> String {
    use sha2::Digest as _;
    b64url(&sha2::Sha256::digest(verifier.as_bytes()))
}

fn https(url: &str, what: &str) -> Result<String, OidcError> {
    match url::Url::parse(url) {
        Ok(parsed) if parsed.scheme() == "https" => Ok(url.to_owned()),
        _ => Err(unavailable(format!("로그인 서버가 알려준 {what} 주소가 https 가 아닙니다."))),
    }
}

/// OIDC Discovery 1.0 §4: `<issuer without trailing slash>/.well-known/openid-configuration`, and
/// the document's `issuer` must be the one asked for (§4.3). A lone trailing-slash difference is
/// tolerated — the discovered value is then what `iss` is compared with.
pub async fn discover(transport: &dyn OidcTransport, issuer: &str) -> Result<Discovery, OidcError> {
    let url = format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/'));
    let reply = transport.get(&url).await.map_err(|e| unavailable(format!("로그인 서버에 연결하지 못했습니다 — {e}")))?;
    if reply.status != 200 {
        return Err(unavailable(format!("로그인 서버 설정을 읽지 못했습니다 (HTTP {}).", reply.status)));
    }
    let doc: serde_json::Value = serde_json::from_slice(&reply.body).map_err(|_| unavailable("로그인 서버 설정이 JSON 이 아닙니다."))?;
    let field = |key: &str| doc.get(key).and_then(serde_json::Value::as_str).map(str::to_owned);
    let discovered = field("issuer").ok_or_else(|| unavailable("로그인 서버 설정에 issuer 가 없습니다."))?;
    if discovered.trim_end_matches('/') != issuer.trim_end_matches('/') {
        return Err(unavailable(format!("로그인 서버의 issuer({discovered})가 등록한 주소와 다릅니다.")));
    }
    let endpoint = |key: &str, what: &str| field(key).ok_or_else(|| unavailable(format!("로그인 서버 설정에 {key} 가 없습니다."))).and_then(|u| https(&u, what));
    let signing_algs = match doc.get("id_token_signing_alg_values_supported") {
        Some(serde_json::Value::Array(items)) => items.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
        _ => Vec::new(),
    };
    Ok(Discovery {
        issuer: discovered,
        authorization_endpoint: endpoint("authorization_endpoint", "인증")?,
        token_endpoint: endpoint("token_endpoint", "토큰")?,
        jwks_uri: endpoint("jwks_uri", "서명 키")?,
        signing_algs,
    })
}

/// A verifier for `alg` from one JWK, or `None` when the key does not fit (type, use, alg).
fn verifier_for(alg: &str, jwk: &josekit::jwk::Jwk, header_has_kid: bool) -> Option<Box<dyn josekit::jws::JwsVerifier>> {
    use josekit::jws::{ES256, ES384, ES512, EdDSA, PS256, PS384, PS512, RS256, RS384, RS512};
    // A verifier built from a JWK with `kid` demands the same `kid` in the header; a token
    // without one may be checked against every candidate key.
    macro_rules! build {
        ($algorithm:expr) => {{
            let mut verifier = $algorithm.verifier_from_jwk(jwk).ok()?;
            if !header_has_kid {
                verifier.remove_key_id();
            }
            Box::new(verifier) as Box<dyn josekit::jws::JwsVerifier>
        }};
    }
    Some(match alg {
        "RS256" => build!(RS256),
        "RS384" => build!(RS384),
        "RS512" => build!(RS512),
        "PS256" => build!(PS256),
        "PS384" => build!(PS384),
        "PS512" => build!(PS512),
        "ES256" => build!(ES256),
        "ES384" => build!(ES384),
        "ES512" => build!(ES512),
        "EdDSA" => build!(EdDSA),
        _ => return None,
    })
}

/// RFC 7519 §2 NumericDate: absent → `None`; a JSON number, possibly fractional (floored), within
/// `i64` seconds; anything else is malformed. `u64` beyond `i64::MAX` saturates (the far future).
fn numeric_date(claims: &serde_json::Map<String, serde_json::Value>, key: &str) -> Result<Option<i64>, OidcError> {
    let Some(value) = claims.get(key) else { return Ok(None) };
    let malformed = || invalid("ID 토큰의 시각 클레임 형식이 올바르지 않습니다.");
    let serde_json::Value::Number(number) = value else { return Err(malformed()) };
    if let Some(seconds) = number.as_i64() {
        return Ok(Some(seconds));
    }
    if number.as_u64().is_some() {
        return Ok(Some(i64::MAX));
    }
    match number.as_f64() {
        // `i64::MAX as f64` rounds up to 2^63, which is itself out of range: compare strictly.
        Some(f) if f.is_finite() && f >= i64::MIN as f64 && f < i64::MAX as f64 => Ok(Some(f.floor() as i64)),
        _ => Err(malformed()),
    }
}

/// OIDC Core §3.1.3.7 on an ID token from the token endpoint. Step 6 would allow trusting TLS
/// instead of the signature for this channel (Auth.js did); the signature is checked anyway.
pub fn validate_id_token(id_token: &str, jwks: &[u8], discovery: &Discovery, client_id: &str, nonce: &str, now: i64) -> Result<IdentityClaims, OidcError> {
    let header = josekit::jwt::decode_header(id_token).map_err(|_| invalid("ID 토큰 형식이 올바르지 않습니다."))?;
    let alg = header.claim("alg").and_then(serde_json::Value::as_str).ok_or_else(|| invalid("ID 토큰에 alg 가 없습니다."))?;
    // §3.1.3.7 step 7: RS256 unless the provider advertises others.
    let advertised = if discovery.signing_algs.is_empty() { vec!["RS256".to_owned()] } else { discovery.signing_algs.clone() };
    if !ASYMMETRIC_ALGS.contains(&alg) || !advertised.iter().any(|a| a == alg) {
        return Err(invalid("ID 토큰의 서명 알고리즘을 받아들일 수 없습니다."));
    }
    let keys = josekit::jwk::JwkSet::from_bytes(jwks).map_err(|_| unavailable("로그인 서버의 서명 키(JWKS)를 읽지 못했습니다."))?;
    let kid = header.claim("kid").and_then(serde_json::Value::as_str);
    let candidates = match kid {
        Some(kid) => keys.get(kid),
        None => keys.keys(),
    };
    // The JWS layer only (signature, alg, kid, crit); the claims are read strictly below rather
    // than through josekit's JWT layer, whose own claim typing would decide some of it for us.
    let payload = candidates
        .into_iter()
        .filter_map(|jwk| verifier_for(alg, jwk, kid.is_some()))
        .find_map(|verifier| josekit::jws::deserialize_compact(id_token, &*verifier).ok())
        .map(|(payload, _)| payload)
        .ok_or_else(|| invalid("ID 토큰 서명이 올바르지 않습니다."))?;
    let claims: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&payload).map_err(|_| invalid("ID 토큰 내용이 JSON 객체가 아닙니다."))?;
    let claims = &claims;
    let text = |key: &str| claims.get(key).and_then(serde_json::Value::as_str);

    // Steps 2–5: issuer, audience, authorized party — a present claim of the wrong type is
    // malformed, never "absent".
    if text("iss") != Some(discovery.issuer.as_str()) {
        return Err(invalid("ID 토큰의 iss 가 로그인 서버와 다릅니다."));
    }
    let malformed_aud = || invalid("ID 토큰의 aud 형식이 올바르지 않습니다.");
    let audiences: Vec<&str> = match claims.get("aud") {
        Some(serde_json::Value::String(aud)) => vec![aud.as_str()],
        Some(serde_json::Value::Array(items)) => items.iter().map(|v| v.as_str().ok_or_else(malformed_aud)).collect::<Result<_, _>>()?,
        _ => return Err(malformed_aud()),
    };
    if !audiences.contains(&client_id) {
        return Err(invalid("ID 토큰이 이 클라이언트를 위한 것이 아닙니다."));
    }
    match claims.get("azp") {
        Some(serde_json::Value::String(azp)) if azp == client_id => {}
        Some(_) => return Err(invalid("ID 토큰의 azp 가 이 클라이언트가 아닙니다.")),
        None if audiences.len() > 1 => return Err(invalid("대상이 여럿인 ID 토큰에 azp 가 없습니다.")),
        None => {}
    }
    // Steps 9–10 and RFC 7519 §4.1.5: expiry, issue time, not-before, with tolerance. Saturating
    // arithmetic: an extreme NumericDate is a date, not an overflow.
    let exp = numeric_date(claims, "exp")?.ok_or_else(|| invalid("ID 토큰에 exp 가 없습니다."))?;
    if now.saturating_sub(CLOCK_TOLERANCE_SECS) >= exp {
        return Err(invalid("ID 토큰이 만료되었습니다."));
    }
    let iat = numeric_date(claims, "iat")?.ok_or_else(|| invalid("ID 토큰에 iat 가 없습니다."))?;
    if iat > now.saturating_add(CLOCK_TOLERANCE_SECS) {
        return Err(invalid("ID 토큰의 발급 시각이 미래입니다."));
    }
    if numeric_date(claims, "nbf")?.is_some_and(|nbf| nbf > now.saturating_add(CLOCK_TOLERANCE_SECS)) {
        return Err(invalid("ID 토큰이 아직 유효하지 않습니다(nbf)."));
    }
    // Step 11: the nonce we sent (constant time — it is a secret of this transaction).
    use subtle::ConstantTimeEq as _;
    let sent = text("nonce").unwrap_or("");
    if sent.is_empty() || !bool::from(sent.as_bytes().ct_eq(nonce.as_bytes())) {
        return Err(invalid("ID 토큰의 nonce 가 이 로그인 요청과 다릅니다."));
    }
    // §2: sub is required, locally unique, at most 255 ASCII characters (printable: it becomes a
    // stored identity key).
    let subject = text("sub")
        .filter(|s| !s.is_empty() && s.len() <= 255 && s.bytes().all(|b| b.is_ascii_graphic() || b == b' '))
        .ok_or_else(|| invalid("ID 토큰의 sub 가 올바르지 않습니다."))?;
    let non_empty = |key: &str| text(key).filter(|v| !v.is_empty()).map(str::to_owned);
    Ok(IdentityClaims {
        subject: subject.to_owned(),
        display_name: non_empty("name").or_else(|| non_empty("preferred_username")),
        email: if claims.get("email_verified").and_then(serde_json::Value::as_bool) == Some(true) { non_empty("email") } else { None },
    })
}

// ─────────────────────────────── Transaction ───────────────────────────────

/// One authorization request's secrets, kept in the browser between start and callback as an
/// Auth.js-style JWE (`dir` + A256CBC-HS512, key derived from AUTH_SECRET with the cookie name
/// as salt) — authenticated, so a forged cookie cannot pick the state, the nonce or whose
/// account an identity is linked to (what the TS link ticket's HMAC was for).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Transaction {
    provider: String,
    state: String,
    nonce: String,
    verifier: String,
    redirect_uri: String,
    /// Linking: the signed-in user the identity is for.
    link_user_id: Option<String>,
}

/// Long enough to sign in at the provider, short enough not to linger (Auth.js: 15 min).
pub const TX_TTL_SECS: i64 = 600;
pub const TX_COOKIE_PLAIN: &str = "nwui-oauth-tx";
pub const TX_COOKIE_SECURE: &str = "__Host-nwui-oauth-tx";
pub const CALLBACK_PATH: &str = "/api/auth/callback";

/// Like pending MFA, pending OIDC requests are bounded and lost on API restart.
#[derive(Default)]
pub struct PendingTransactions(std::sync::Mutex<std::collections::VecDeque<(String, i64)>>);

impl PendingTransactions {
    fn insert(&self, id: String, now: i64) {
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        pending.retain(|(key, expiry)| *expiry > now && key != &id);
        if pending.len() >= 10_000 { pending.pop_front(); }
        pending.push_back((id, now + TX_TTL_SECS));
    }

    fn consume(&self, id: &str, now: i64) -> bool {
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        pending.iter().position(|(key, _)| key == id)
            .and_then(|index| pending.remove(index)).is_some_and(|(_, expiry)| expiry > now)
    }
}

pub fn tx_cookie_name(secure: bool) -> &'static str {
    if secure { TX_COOKIE_SECURE } else { TX_COOKIE_PLAIN }
}

impl Transaction {
    fn claims(&self) -> super::session::Claims {
        let mut claims = super::session::Claims::new();
        for (key, value) in [("provider", &self.provider), ("state", &self.state), ("nonce", &self.nonce), ("verifier", &self.verifier), ("redirectUri", &self.redirect_uri)] {
            claims.insert(key.into(), value.clone().into());
        }
        if let Some(user) = &self.link_user_id {
            claims.insert("linkUserId".into(), user.clone().into());
        }
        claims
    }

    fn from_claims(claims: &super::session::Claims) -> Option<Self> {
        let text = |key: &str| claims.get(key).and_then(serde_json::Value::as_str).filter(|v| !v.is_empty()).map(str::to_owned);
        Some(Self {
            provider: text("provider")?,
            state: text("state")?,
            nonce: text("nonce")?,
            verifier: text("verifier")?,
            redirect_uri: text("redirectUri")?,
            link_user_id: text("linkUserId"),
        })
    }
}

/// Secure transaction cookies are host-only; the Auth.js session cookie keeps its compatible name.
fn tx_cookie(secure: bool, value: &str, max_age: i64) -> String {
    let name = tx_cookie_name(secure);
    let path = if secure { "/" } else { CALLBACK_PATH };
    format!("{name}={value}; Path={path}; Max-Age={max_age}; HttpOnly; SameSite=Lax{}", if secure { "; Secure" } else { "" })
}

// ──────────────────────────────── Provider ────────────────────────────────

struct Provider {
    id: String,
    issuer: String,
    client_id: String,
    client_secret: String,
    scope: String,
}

/// Enabled, with a client secret that opens (the login screen hides the others too).
async fn load_provider(state: &AppState, id: &str) -> Result<Option<Provider>, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let row = client
        .query_opt("SELECT id, issuer, client_id, client_secret_enc, scope FROM oauth_providers WHERE id = $1 AND enabled", &[&id])
        .await
        .map_err(|_| db_unavailable())?;
    Ok(row.and_then(|row| {
        let client_secret = state.secrets.open(Some(row.get::<_, &str>("client_secret_enc")))?;
        Some(Provider {
            id: row.get("id"),
            issuer: row.get("issuer"),
            client_id: row.get("client_id"),
            client_secret,
            scope: crate::admin::oauth::normalize_scope(row.get::<_, &str>("scope")),
        })
    }))
}

/// The callback the provider was registered with: the public base URL when set (what the admin
/// screen shows), else this request's external origin.
async fn redirect_uri(state: &AppState, ctx: &RequestContext, provider_id: &str) -> Result<String, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let base: Option<String> = client
        .query_opt("SELECT public_base_url FROM app_settings WHERE id = 'current'", &[])
        .await
        .map_err(|_| db_unavailable())?
        .and_then(|row| row.get(0));
    let base = base.filter(|b| !b.is_empty()).unwrap_or_else(|| ctx.external_origin.clone());
    Ok(format!("{}{CALLBACK_PATH}/{provider_id}", base.trim_end_matches('/')))
}

/// OIDC Core §3.1.2.1 authentication request (RFC 6749 §4.1.1 + RFC 7636 §4.3).
fn authorization_url(discovery: &Discovery, provider: &Provider, tx: &Transaction) -> Result<String, OidcError> {
    let mut url = url::Url::parse(&discovery.authorization_endpoint).map_err(|_| unavailable("인증 주소가 올바르지 않습니다."))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &provider.client_id)
        .append_pair("redirect_uri", &tx.redirect_uri)
        .append_pair("scope", &provider.scope)
        .append_pair("state", &tx.state)
        .append_pair("nonce", &tx.nonce)
        .append_pair("code_challenge", &pkce_challenge(&tx.verifier))
        .append_pair("code_challenge_method", "S256");
    Ok(url.into())
}

fn form_encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// RFC 6749 §4.1.3 with `client_secret_basic` (§2.3.1: both parts form-encoded before base64),
/// then the ID token checked with the provider's current JWKS.
async fn exchange_and_validate(transport: &dyn OidcTransport, discovery: &Discovery, provider: &Provider, tx: &Transaction, code: &str, now: i64) -> Result<IdentityClaims, OidcError> {
    use base64::Engine as _;
    let credentials = format!("{}:{}", form_encode(&provider.client_id), form_encode(&provider.client_secret));
    let authorization = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(credentials));
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", code)
        .append_pair("redirect_uri", &tx.redirect_uri)
        .append_pair("code_verifier", &tx.verifier)
        .finish();
    let reply = transport.post_form(&discovery.token_endpoint, &authorization, &body).await.map_err(|e| unavailable(format!("토큰을 받지 못했습니다 — {e}")))?;
    if reply.status >= 500 {
        return Err(unavailable(format!("토큰을 받지 못했습니다 (HTTP {}).", reply.status)));
    }
    if reply.status != 200 {
        return Err(invalid("로그인 서버가 인가 코드를 받아들이지 않았습니다."));
    }
    let tokens: serde_json::Value = serde_json::from_slice(&reply.body).map_err(|_| unavailable("토큰 응답이 JSON 이 아닙니다."))?;
    let id_token = tokens.get("id_token").and_then(serde_json::Value::as_str).ok_or_else(|| invalid("토큰 응답에 ID 토큰이 없습니다."))?;
    let jwks = transport.get(&discovery.jwks_uri).await.map_err(|e| unavailable(format!("서명 키를 받지 못했습니다 — {e}")))?;
    if jwks.status != 200 {
        return Err(unavailable(format!("서명 키를 받지 못했습니다 (HTTP {}).", jwks.status)));
    }
    validate_id_token(id_token, &jwks.body, discovery, &provider.client_id, &tx.nonce, now)
}

// ─────────────────────────────── Identities ───────────────────────────────

fn db_error<E>(_: E) -> ApiError {
    db_unavailable()
}

enum SignIn {
    Session(AuthorizedUser),
    /// A first-seen identity became a pending viewer account (TS `resolveOAuthUser`).
    Pending,
    Disabled,
}

// Bind the verified identity to the still-current issuer while its link is resolved or written.
async fn lock_verified_issuer(tx: &tokio_postgres::Transaction<'_>, provider: &str, issuer: &str) -> Result<(), ApiError> {
    let current = tx.query_opt("SELECT issuer FROM oauth_providers WHERE id=$1 AND enabled FOR SHARE", &[&provider]).await.map_err(db_error)?;
    if !current.is_some_and(|row| row.get::<_, &str>(0) == issuer) {
        return Err(ApiError::new(StatusCode::CONFLICT, "provider_changed", "로그인 서버 설정이 바뀌었습니다. 다시 시도하세요."));
    }
    Ok(())
}

/// Identity is `(provider, subject)` — never the email (`oauth-link.ts` rule 1: matching by
/// email would let a provider that does not verify emails take over an account).
async fn sign_in_identity(state: &AppState, provider: &str, issuer: &str, identity: &IdentityClaims, ctx: &RequestContext) -> Result<SignIn, ApiError> {
    // Twice at most: a concurrent first sign-in of the same identity wins the insert race.
    for _ in 0..2 {
        let mut client = state.db.get().await.map_err(db_error)?;
        let tx = client.transaction().await.map_err(db_error)?;
        lock_verified_issuer(&tx, provider, issuer).await?;
        let row = tx
            .query_opt(
                "SELECT u.id, u.email, u.role::text AS role, u.session_version, u.totp_enabled, u.enabled, u.approved \
                 FROM oauth_accounts a JOIN users u ON u.id = a.user_id WHERE a.provider_id = $1 AND a.subject = $2",
                &[&provider, &identity.subject],
            )
            .await
            .map_err(db_error)?;
        if let Some(row) = row {
            tx.execute("UPDATE oauth_accounts SET email = COALESCE($3, email), email_verified = $4 WHERE provider_id = $1 AND subject = $2", &[&provider, &identity.subject, &identity.email, &identity.email.is_some()]).await.map_err(db_error)?;
            if !row.get::<_, bool>("enabled") {
                tx.commit().await.map_err(db_error)?;
                return Ok(SignIn::Disabled);
            }
            if !row.get::<_, bool>("approved") {
                tx.commit().await.map_err(db_error)?;
                return Ok(SignIn::Pending);
            }
            let policy = tx.query_opt("SELECT require_2fa_oauth, require_2fa_admin, require_2fa_user FROM app_settings WHERE id = 'current'", &[]).await.map_err(db_error)?;
            let role = parse_role(row.get::<_, &str>("role")).ok_or_else(ApiError::internal)?;
            let (oauth_switch, role_policy) = policy.map_or((false, false), |p| {
                (p.get::<_, bool>(0), if role == Role::Admin { p.get::<_, bool>(1) } else { p.get::<_, bool>(2) })
            });
            let user = AuthorizedUser {
                id: row.get("id"),
                email: row.get("email"),
                role,
                session_version: row.get("session_version"),
                totp_enabled: row.get("totp_enabled"),
                // Policy only: the OAuth switch or the role's policy, as for a password sign-in.
                // 1.17.x used the switch alone, so a taken-over IdP account of an admin under the
                // admin policy was a full session without the app's code. A TOTP the user
                // enrolled themselves is asked through `externalLogin` (see `principal.rs`).
                requires_2fa: oauth_switch || role_policy,
                external_login: true,
                // The provider's login is not this app's factor: when the switch is on and the
                // user is enrolled, the session is confined to `account.verifyTwoFactor` (TS
                // issued a full session here — the switch only ever forced enrollment).
                mfa_verified: false,
            };
            tx.commit().await.map_err(db_error)?;
            return Ok(SignIn::Session(user));
        }
        // Rule 2: a first-seen identity gets an account, viewer, awaiting approval.
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute("INSERT INTO users (id, email, password_hash, role, approved) VALUES ($1, NULL, NULL, 'viewer', false)", &[&id]).await.map_err(db_error)?;
        let inserted = tx
            .execute(
                "INSERT INTO oauth_accounts (provider_id, subject, user_id, display_name, email, email_verified) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
                &[&provider, &identity.subject, &id, &identity.display_name, &identity.email, &identity.email.is_some()],
            )
            .await
            .map_err(db_error)?;
        if inserted == 0 {
            continue; // rolled back on drop; the other sign-in's account is read next time
        }
        let entry = AuditEntry { actor_id: Some(id.clone()), action: "account:oauth-signup".into(), target: Some(id.clone()), after: Some(serde_json::json!({"provider": provider})), ip: ctx.ip.clone(), ..AuditEntry::default() };
        audit::write(&*tx, &entry).await.map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        return Ok(SignIn::Pending);
    }
    Err(db_unavailable())
}

enum Link {
    Linked,
    OwnedByOther,
}

/// `linkOAuthAccount`: linking what is already mine is harmless; someone else's is refused.
async fn link_identity(state: &AppState, user_id: &str, provider: &str, issuer: &str, identity: &IdentityClaims, ctx: &RequestContext) -> Result<Link, ApiError> {
    let mut client = state.db.get().await.map_err(db_error)?;
    let tx = client.transaction().await.map_err(db_error)?;
    lock_verified_issuer(&tx, provider, issuer).await?;
    let inserted = tx
        .execute(
            "INSERT INTO oauth_accounts (provider_id, subject, user_id, display_name, email, email_verified) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
            &[&provider, &identity.subject, &user_id, &identity.display_name, &identity.email, &identity.email.is_some()],
        )
        .await
        .map_err(db_error)?;
    if inserted == 0 {
        let owner: Option<String> = tx
            .query_opt("SELECT user_id FROM oauth_accounts WHERE provider_id = $1 AND subject = $2", &[&provider, &identity.subject])
            .await
            .map_err(db_error)?
            .map(|row| row.get(0));
        if owner.as_deref() != Some(user_id) { return Ok(Link::OwnedByOther); }
        tx.execute("UPDATE oauth_accounts SET email = COALESCE($3, email), email_verified = $4 WHERE provider_id = $1 AND subject = $2", &[&provider, &identity.subject, &identity.email, &identity.email.is_some()]).await.map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        return Ok(Link::Linked);
    }
    let entry = AuditEntry { actor_id: Some(user_id.to_owned()), action: "account:oauth-link".into(), target: Some(provider.to_owned()), ip: ctx.ip.clone(), ..AuditEntry::default() };
    audit::write(&*tx, &entry).await.map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Link::Linked)
}

pub const LAST_LOGIN_METHOD: &str = "마지막 남은 로그인 방법은 해제할 수 없습니다.";

/// `unlinkOAuthAccount` rule 3, under a lock on the user so two concurrent unlinks cannot both
/// pass the check and strand the account. Nothing linked for that provider → nothing.
pub async fn unlink(state: &AppState, user_id: &str, provider: &str, ctx: &RequestContext) -> Result<(), ApiError> {
    let mut client = state.db.get().await.map_err(db_error)?;
    let tx = client.transaction().await.map_err(db_error)?;
    let Some(user) = tx.query_opt("SELECT password_hash IS NOT NULL FROM users WHERE id = $1 FOR UPDATE", &[&user_id]).await.map_err(db_error)? else {
        return Ok(());
    };
    let counts = tx
        .query_one("SELECT count(*) FILTER (WHERE provider_id = $2), count(*) FROM oauth_accounts WHERE user_id = $1", &[&user_id, &provider])
        .await
        .map_err(db_error)?;
    let (this, total): (i64, i64) = (counts.get(0), counts.get(1));
    if this == 0 {
        return Ok(());
    }
    if !user.get::<_, bool>(0) && total - this == 0 {
        return Err(ApiError::new(axum::http::StatusCode::CONFLICT, "conflict", LAST_LOGIN_METHOD));
    }
    tx.execute("DELETE FROM oauth_accounts WHERE user_id = $1 AND provider_id = $2", &[&user_id, &provider]).await.map_err(db_error)?;
    let entry = AuditEntry { actor_id: Some(user_id.to_owned()), action: "account:oauth-unlink".into(), target: Some(provider.to_owned()), ip: ctx.ip.clone(), ..AuditEntry::default() };
    audit::write(&*tx, &entry).await.map_err(db_error)?;
    tx.commit().await.map_err(db_error)
}

// ───────────────────────────────── Routes ─────────────────────────────────

use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json};

use super::audit::{self, AuditEntry};
use super::login::AuthorizedUser;
use super::principal::{TwoFactorGate, db_unavailable, now_unix_ms, require_session};
use super::rbac::parse_role;
use crate::contracts::Role;
use super::session;
use crate::contracts::FormResult;
use crate::error::ApiError;
use crate::http_policy::read_form;
use crate::request_context::RequestContext;

pub fn router(transport: Transport) -> Router<AppState> {
    Router::new()
        .route("/api/ui/v1/forms/login.oauthSignIn", post(sign_in_start))
        .route("/api/ui/v1/forms/account.startLink", post(link_start))
        .route("/api/ui/v1/forms/account.unlink", post(unlink_route))
        .route("/api/auth/callback/{id}", get(callback))
        .layer(Extension(transport))
}

fn append_cookies(mut response: Response, cookies: &[String]) -> Response {
    for cookie in cookies {
        match HeaderValue::from_str(cookie) {
            Ok(value) => {
                response.headers_mut().append(header::SET_COOKIE, value);
            }
            Err(_) => return ApiError::internal().into_response(),
        }
    }
    response
}

/// Starts an authorization request: `{redirectTo: <absolute provider URL>}` + the transaction
/// cookie. The browser must navigate there (a cross-origin full navigation).
async fn begin(state: &AppState, transport: &dyn OidcTransport, ctx: &RequestContext, provider_id: &str, link_user_id: Option<String>) -> Result<Response, ApiError> {
    let provider = load_provider(state, provider_id).await?.ok_or_else(ApiError::not_found)?;
    // Callers may be unauthenticated: they learn that the provider failed, not what it or the
    // network said (error strings, the issuer it announced). The detail goes to the server log.
    let discovery = discover(transport, &provider.issuer).await.map_err(|error| {
        let (OidcError::Unavailable(detail) | OidcError::Invalid(detail)) = error;
        eprintln!("[oidc] discovery failed for provider {}: {detail}", provider.id);
        ApiError::new(StatusCode::BAD_GATEWAY, "upstream_failed", "로그인 서버에 연결하지 못했습니다. 잠시 후 다시 시도하거나 관리자에게 알리세요.")
    })?;
    let tx = Transaction {
        provider: provider.id.clone(),
        state: random_token(),
        nonce: random_token(),
        verifier: random_token(),
        redirect_uri: redirect_uri(state, ctx, &provider.id).await?,
        link_user_id,
    };
    let to = authorization_url(&discovery, &provider, &tx).map_err(|_| ApiError::new(StatusCode::BAD_GATEWAY, "upstream_failed", "인증 주소가 올바르지 않습니다."))?;
    let secret = state.config.auth_secrets.first().ok_or_else(ApiError::internal)?;
    let now = now_unix_ms() / 1000;
    let sealed = session::encode(&tx.claims(), secret, tx_cookie_name(ctx.secure), now, TX_TTL_SECS).map_err(|_| ApiError::internal())?;
    state.runtime.pending_oidc.insert(tx.state.clone(), now);
    let body = FormResult { redirect_to: Some(to), ..FormResult::default() };
    Ok(append_cookies(Json(body).into_response(), &[tx_cookie(ctx.secure, &sealed, TX_TTL_SECS)]))
}

/// `oauthSignInAction`.
async fn sign_in_start(State(state): State<AppState>, Extension(transport): Extension<Transport>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Response, ApiError> {
    let form = read_form(request).await?;
    let provider = crate::crypto::js_trim(form.get("providerId").unwrap_or(""));
    let key = format!("oauth-signin:{}", ctx.ip.as_deref().unwrap_or("unknown"));
    let attempt = state.runtime.login_throttle.begin_ip(&key, now_unix_ms()).ok_or_else(|| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "로그인 요청이 너무 많습니다. 잠시 후 다시 시도하세요."))?;
    attempt.keep();
    begin(&state, &*transport, &ctx, provider, None).await
}

/// `startLink`: the signed-in (and 2FA-satisfied) user links another login method.
async fn link_start(State(state): State<AppState>, Extension(transport): Extension<Transport>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Response, ApiError> {
    let actor = require_session(&state, request.headers(), &ctx, None, TwoFactorGate::Enforce).await?;
    let form = read_form(request).await?;
    let provider = crate::crypto::js_trim(form.get("providerId").unwrap_or(""));
    begin(&state, &*transport, &ctx, provider, Some(actor.user_id)).await
}

async fn unlink_route(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>, request: Request) -> Result<Json<FormResult>, ApiError> {
    let actor = require_session(&state, request.headers(), &ctx, None, TwoFactorGate::Enforce).await?;
    let form = read_form(request).await?;
    let provider = crate::crypto::js_trim(form.get("providerId").unwrap_or(""));
    unlink(&state, &actor.user_id, provider, &ctx).await?;
    Ok(Json(FormResult { redirect_to: Some("/account?unlinked=1".into()), ..FormResult::default() }))
}

pub const LOGIN_FAILED: &str = "/login?error=OAuthCallbackError";
pub const LOGIN_DENIED: &str = "/login?error=AccessDenied";
pub const LINK_FAILED: &str = "/account?error=link-failed";
pub const LINK_TAKEN: &str = "/account?error=already-linked";

fn found(location: &str, cookies: &[String]) -> Response {
    let response = (StatusCode::FOUND, [(header::LOCATION, location.to_owned())]).into_response();
    append_cookies(response, cookies)
}

#[derive(serde::Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// The provider sends the browser back here (path registered at the provider; it must not
/// change). Every outcome is a redirect that also expires the transaction cookie — one use only.
async fn callback(
    State(state): State<AppState>,
    Extension(transport): Extension<Transport>,
    Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let clear = tx_cookie(ctx.secure, "", 0);
    let now = now_unix_ms() / 1000;
    let name = tx_cookie_name(ctx.secure);
    let secrets: Vec<&str> = state.config.auth_secrets.iter().map(String::as_str).collect();
    let tx = session::request_cookies(&headers)
        .into_iter()
        .find(|(cookie, _)| cookie == name)
        .and_then(|(_, value)| session::decode(&value, &secrets, name, now).ok())
        .and_then(|claims| Transaction::from_claims(&claims))
        .filter(|tx| state.runtime.pending_oidc.consume(&tx.state, now))
        .filter(|tx| tx.provider == id);
    let Some(tx) = tx else { return found(LOGIN_FAILED, &[clear]) };
    let failed = if tx.link_user_id.is_some() { LINK_FAILED } else { LOGIN_FAILED };

    // RFC 6749 §4.1.2.1: the provider refused (the user cancelled, usually).
    if query.error.is_some() {
        return found(if tx.link_user_id.is_some() { LINK_FAILED } else { LOGIN_DENIED }, &[clear]);
    }
    // RFC 6749 §10.12 / RFC 9700 §4.7: the state binds this response to our request.
    use subtle::ConstantTimeEq as _;
    let state_ok = query.state.as_deref().is_some_and(|s| bool::from(s.as_bytes().ct_eq(tx.state.as_bytes())));
    let Some(code) = query.code.filter(|c| !c.is_empty() && state_ok) else { return found(failed, &[clear]) };

    let outcome = async {
        let provider = load_provider(&state, &id).await.ok().flatten()?;
        let discovery = discover(&*transport, &provider.issuer).await.ok()?;
        let identity = exchange_and_validate(&*transport, &discovery, &provider, &tx, &code, now).await.ok()?;
        Some((identity, provider.issuer))
    }
    .await;
    let Some((identity, verified_issuer)) = outcome else { return found(failed, &[clear]) };

    if let Some(user_id) = &tx.link_user_id {
        // The link is for the user who started it, who must still be the one signed in here.
        let current = super::principal::authenticate_session(&state, &headers, &ctx, now).await.ok().flatten();
        if current.as_ref().map(|p| p.user_id.as_str()) != Some(user_id.as_str()) {
            return found(LINK_FAILED, &[clear]);
        }
        return match link_identity(&state, user_id, &id, &verified_issuer, &identity, &ctx).await {
            Ok(Link::Linked) => found("/account?linked=1", &[clear]),
            Ok(Link::OwnedByOther) => found(LINK_TAKEN, &[clear]),
            Err(_) => found(LINK_FAILED, &[clear]),
        };
    }

    match sign_in_identity(&state, &id, &verified_issuer, &identity, &ctx).await {
        Ok(SignIn::Pending) => found("/login?pending=1", &[clear]),
        Ok(SignIn::Disabled) => found("/login?error=disabled", &[clear]),
        Ok(SignIn::Session(user)) => {
            let session_name = session::cookie_name(ctx.secure);
            let Some(secret) = state.config.auth_secrets.first() else { return found(LOGIN_FAILED, &[clear]) };
            let Ok(token) = session::encode(&user.session_claims(), secret, session_name, now, session::SESSION_MAX_AGE_SECS) else {
                return found(LOGIN_FAILED, &[clear]);
            };
            let present = session::request_cookies(&headers);
            let mut cookies = session::issue_cookies(session_name, &token, &present, ctx.secure, now + session::SESSION_MAX_AGE_SECS);
            cookies.push(clear);
            // Auth.js `events.signIn`: the login row is part of signing in.
            let entry = AuditEntry { actor_id: Some(user.id.clone()), action: "login".into(), ip: ctx.ip.clone(), ..AuditEntry::default() };
            let audited = match state.db.get().await {
                Ok(client) => audit::write(&**client, &entry).await.is_ok(),
                Err(_) => false,
            };
            if !audited {
                return found(LOGIN_FAILED, &[tx_cookie(ctx.secure, "", 0)]);
            }
            found("/", &cookies)
        }
        Err(_) => found(LOGIN_FAILED, &[clear]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use josekit::jws::{JwsHeader, RS256};
    use josekit::jwt::{self, JwtPayload};
    use serde_json::{Value, json};

    fn discovery() -> Discovery {
        Discovery {
            issuer: "https://idp.test".into(),
            authorization_endpoint: "https://idp.test/auth".into(),
            token_endpoint: "https://idp.test/token".into(),
            jwks_uri: "https://idp.test/jwks".into(),
            signing_algs: vec!["RS256".into()],
        }
    }

    struct Keys {
        signer: josekit::jws::alg::rsassa::RsassaJwsSigner,
        public: josekit::jwk::Jwk,
        jwks: Vec<u8>,
    }

    /// The same public key published under another `kid`.
    fn jwks_with_kid(keys: &Keys, kid: &str) -> Vec<u8> {
        let mut public = keys.public.clone();
        public.set_key_id(kid);
        serde_json::to_vec(&json!({"keys": [serde_json::to_value(public.as_ref()).unwrap()]})).unwrap()
    }

    fn keys(kid: &str) -> Keys {
        let pair = RS256.generate_key_pair(2048).unwrap();
        let mut private = pair.to_jwk_private_key();
        private.set_key_id(kid);
        let mut public = pair.to_jwk_public_key();
        public.set_key_id(kid);
        let jwks = serde_json::to_vec(&json!({"keys": [serde_json::to_value(public.as_ref()).unwrap()]})).unwrap();
        Keys { signer: RS256.signer_from_jwk(&private).unwrap(), public, jwks }
    }

    fn claims(now: i64) -> Value {
        json!({"iss": "https://idp.test", "aud": "cid", "sub": "s-1", "nonce": "n-1", "iat": now, "exp": now + 300, "name": "Kim", "email": "k@idp.test", "email_verified": true})
    }

    fn sign(keys: &Keys, claims: &Value, kid: Option<&str>) -> String {
        let mut header = JwsHeader::new();
        header.set_token_type("JWT");
        if let Some(kid) = kid {
            header.set_key_id(kid);
        }
        josekit::jws::serialize_compact(&serde_json::to_vec(claims).unwrap(), &header, &keys.signer).unwrap()
    }

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn pkce_matches_rfc7636_appendix_b() {
        assert_eq!(pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn a_valid_id_token_yields_the_identity() {
        let k = keys("k1");
        let token = sign(&k, &claims(NOW), Some("k1"));
        assert_eq!(
            validate_id_token(&token, &k.jwks, &discovery(), "cid", "n-1", NOW),
            Ok(IdentityClaims { subject: "s-1".into(), display_name: Some("Kim".into()), email: Some("k@idp.test".into()) })
        );
        // preferred_username stands in for a missing name; aud may be an array with azp.
        let mut c = claims(NOW);
        c.as_object_mut().unwrap().remove("name");
        c["preferred_username"] = json!("kim");
        c["aud"] = json!(["cid", "other"]);
        c["azp"] = json!("cid");
        let token = sign(&k, &c, None);
        assert_eq!(validate_id_token(&token, &k.jwks, &discovery(), "cid", "n-1", NOW).unwrap().display_name.as_deref(), Some("kim"));
    }

    #[test]
    fn oidc_email_requires_a_boolean_verification_claim() {
        let k = keys("k1");
        for verified in [None, Some(json!(false)), Some(json!(null)), Some(json!("true")), Some(json!(1)), Some(json!(true))] {
            let mut c = claims(NOW);
            c.as_object_mut().unwrap().remove("email_verified");
            if let Some(value) = &verified { c["email_verified"] = value.clone(); }
            let token = sign(&k, &c, Some("k1"));
            let identity = validate_id_token(&token, &k.jwks, &discovery(), "cid", "n-1", NOW).unwrap();
            assert_eq!(identity.subject, "s-1");
            assert_eq!(identity.email.is_some(), verified == Some(json!(true)), "{verified:?}");
        }
    }

    #[test]
    fn every_claim_check_refuses() {
        let k = keys("k1");
        type Change = Box<dyn Fn(&mut Value)>;
        let cases: Vec<(&str, Change)> = vec![
            ("iss", Box::new(|c| c["iss"] = json!("https://evil.test"))),
            ("aud", Box::new(|c| c["aud"] = json!("other"))),
            ("aud list", Box::new(|c| c["aud"] = json!(["other"]))),
            ("azp", Box::new(|c| { c["aud"] = json!(["cid", "other"]); c["azp"] = json!("other"); })),
            ("azp missing with many audiences", Box::new(|c| c["aud"] = json!(["cid", "other"]))),
            ("exp past", Box::new(|c| c["exp"] = json!(NOW - CLOCK_TOLERANCE_SECS - 1))),
            ("exp missing", Box::new(|c| { c.as_object_mut().unwrap().remove("exp"); })),
            ("iat future", Box::new(|c| c["iat"] = json!(NOW + CLOCK_TOLERANCE_SECS + 1))),
            ("iat missing", Box::new(|c| { c.as_object_mut().unwrap().remove("iat"); })),
            ("nonce", Box::new(|c| c["nonce"] = json!("n-2"))),
            ("nonce missing", Box::new(|c| { c.as_object_mut().unwrap().remove("nonce"); })),
            ("sub empty", Box::new(|c| c["sub"] = json!(""))),
            ("sub missing", Box::new(|c| { c.as_object_mut().unwrap().remove("sub"); })),
            ("sub not ASCII", Box::new(|c| c["sub"] = json!("ü-1"))),
            ("sub too long", Box::new(|c| c["sub"] = json!("a".repeat(256)))),
            ("exp not a number", Box::new(|c| c["exp"] = json!("99999999999"))),
            ("exp out of range", Box::new(|c| c["exp"] = json!(1e300))),
            ("iat not a number", Box::new(|c| c["iat"] = json!(true))),
            ("aud with a non-string member", Box::new(|c| c["aud"] = json!(["cid", 5]))),
            ("aud empty", Box::new(|c| c["aud"] = json!([]))),
            ("aud wrong type", Box::new(|c| c["aud"] = json!({"cid": true}))),
            ("azp wrong type", Box::new(|c| c["azp"] = json!(5))),
            ("nbf in the future", Box::new(|c| c["nbf"] = json!(NOW + CLOCK_TOLERANCE_SECS + 1))),
            ("nbf wrong type", Box::new(|c| c["nbf"] = json!("soon"))),
        ];
        for (name, change) in cases {
            let mut c = claims(NOW);
            change(&mut c);
            let token = sign(&k, &c, Some("k1"));
            assert!(matches!(validate_id_token(&token, &k.jwks, &discovery(), "cid", "n-1", NOW), Err(OidcError::Invalid(_))), "{name}");
        }
        // Tolerance edges are accepted.
        let mut c = claims(NOW);
        c["exp"] = json!(NOW - CLOCK_TOLERANCE_SECS + 1);
        c["iat"] = json!(NOW + CLOCK_TOLERANCE_SECS);
        c["nbf"] = json!(NOW + CLOCK_TOLERANCE_SECS);
        assert!(validate_id_token(&sign(&k, &c, Some("k1")), &k.jwks, &discovery(), "cid", "n-1", NOW).is_ok());
        // RFC 7519 NumericDate may be fractional; the far future is not an overflow.
        for exp in [json!(NOW as f64 + 300.5), json!(i64::MAX), json!(u64::MAX)] {
            let mut c = claims(NOW);
            c["exp"] = exp.clone();
            assert!(validate_id_token(&sign(&k, &c, Some("k1")), &k.jwks, &discovery(), "cid", "n-1", NOW).is_ok(), "exp {exp}");
        }
        let mut c = claims(i64::MIN / 2);
        c["iat"] = json!(i64::MIN);
        assert!(matches!(validate_id_token(&sign(&k, &c, Some("k1")), &k.jwks, &discovery(), "cid", "n-1", i64::MAX), Err(OidcError::Invalid(_))), "no overflow at the other end either");
    }

    #[test]
    fn signatures_keys_and_algorithms_are_checked() {
        let k = keys("k1");
        let other = keys("k1");
        let token = sign(&k, &claims(NOW), Some("k1"));
        let check = |token: &str, jwks: &[u8], d: &Discovery| validate_id_token(token, jwks, d, "cid", "n-1", NOW);
        assert!(matches!(check(&token, &other.jwks, &discovery()), Err(OidcError::Invalid(_))), "another key with the same kid");
        assert!(matches!(check(&token, &jwks_with_kid(&k, "k9"), &discovery()), Err(OidcError::Invalid(_))), "unknown kid");
        let mut tampered = token.clone();
        let flip = tampered.len() - 5;
        tampered.replace_range(flip..flip + 1, if &token[flip..flip + 1] == "A" { "B" } else { "A" });
        assert!(matches!(check(&tampered, &k.jwks, &discovery()), Err(OidcError::Invalid(_))), "tampered signature");
        let mut es_only = discovery();
        es_only.signing_algs = vec!["ES256".into()];
        assert!(matches!(check(&token, &k.jwks, &es_only), Err(OidcError::Invalid(_))), "alg not advertised");
        // alg=none and a symmetric alg keyed with the public key are never accepted.
        let unsecured = jwt::encode_unsecured(&JwtPayload::from_map(claims(NOW).as_object().unwrap().clone()).unwrap(), &JwsHeader::new()).unwrap();
        let mut any = discovery();
        any.signing_algs = vec!["none".into(), "HS256".into(), "RS256".into()];
        assert!(matches!(check(&unsecured, &k.jwks, &any), Err(OidcError::Invalid(_))), "alg none");
        let hs = josekit::jws::HS256.signer_from_bytes(&k.jwks).unwrap();
        let hs_token = jwt::encode_with_signer(&JwtPayload::from_map(claims(NOW).as_object().unwrap().clone()).unwrap(), &JwsHeader::new(), &hs).unwrap();
        assert!(matches!(check(&hs_token, &k.jwks, &any), Err(OidcError::Invalid(_))), "HS256");
        assert!(matches!(check("not.a.jwt", &k.jwks, &discovery()), Err(OidcError::Invalid(_))));
        assert!(matches!(check(&token, b"not json", &discovery()), Err(OidcError::Unavailable(_))), "a broken JWKS is the provider's problem");
    }
}

#[cfg(test)]
mod q3_transaction_tests {
    use super::*;

    #[test]
    fn issued_transactions_expire_are_bounded_and_are_consumed_once() {
        let pending = Arc::new(PendingTransactions::default());
        pending.insert("expired".into(), 0);
        assert!(!pending.consume("expired", TX_TTL_SECS));
        pending.insert("once".into(), 10);
        let workers: Vec<_> = (0..8).map(|_| {
            let pending = pending.clone();
            std::thread::spawn(move || pending.consume("once", 11) as usize)
        }).collect();
        assert_eq!(workers.into_iter().map(|worker| worker.join().unwrap()).sum::<usize>(), 1);
        for n in 0..10_000 { pending.insert(n.to_string(), 20); }
        pending.insert("overflow".into(), 20);
        assert!(pending.consume("overflow", 20));
        assert!(!pending.consume("0", 20));
        pending.insert("after-expiry".into(), 20 + TX_TTL_SECS);
        assert!(!PendingTransactions::default().consume("after-expiry", 20 + TX_TTL_SECS));
    }
    #[test]
    fn q3r1_bound_evicts_oldest_without_rejecting_a_new_login() {
        let pending = PendingTransactions::default();
        for n in 0..10_000 { pending.insert(n.to_string(), 20); }
        pending.insert("new".into(), 20);
        assert!(pending.consume("new", 21));
        assert!(!pending.consume("0", 21));
        assert!(pending.consume("1", 21));
    }

    #[test]
    fn expired_transactions_leave_room_before_a_live_oldest_request_is_evicted() {
        let pending = PendingTransactions::default();
        pending.insert("expired".into(), 0);
        for n in 1..10_000 { pending.insert(n.to_string(), 1); }
        pending.insert("new".into(), TX_TTL_SECS);
        assert!(!pending.consume("expired", TX_TTL_SECS));
        assert!(pending.consume("1", TX_TTL_SECS));
        assert!(pending.consume("new", TX_TTL_SECS));
    }

}
