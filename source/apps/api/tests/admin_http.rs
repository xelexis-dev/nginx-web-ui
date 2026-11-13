//! HTTP tests for `admin::router()` through the root boundary, on a real PostgreSQL database.
//!
//!     NWUI_TEST_DATABASE_URL=postgres://user:pass@host:5432/postgres cargo test --test admin_http -- --include-ignored

mod auth_support;

use std::net::SocketAddr;

use auth_support::{TestDb, UserRow, audit_rows, insert_user, test_db};
use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use nwui_api::auth::session;
use serde_json::{Value, json};
use tower::ServiceExt as _;

const ORIGIN: &str = "http://admin.example.test";

fn app(db: &TestDb) -> Router {
    nwui_api::app::with_request_context(nwui_api::admin::router().with_state(db.state.clone()), None)
}

/// A signed-in session that passed this app's second factor when it had one.
fn cookie_for(id: &str, sv: i64, requires_2fa: bool) -> String {
    cookie_with(id, sv, requires_2fa, true)
}

fn cookie_with(id: &str, sv: i64, requires_2fa: bool, mfa_verified: bool) -> String {
    let now = nwui_api::auth::principal::now_unix_ms() / 1000;
    let claims = json!({"sub": id, "id": id, "sv": sv, "requires2fa": requires_2fa, "mfaVerified": mfa_verified});
    let token = session::encode(claims.as_object().unwrap(), auth_support::SECRET, session::COOKIE_NAME_PLAIN, now, 3600).unwrap();
    format!("authjs.session-token={token}")
}

async fn call(app: &Router, method: &str, path: &str, cookie: &str, form: Option<&str>) -> (StatusCode, Value) {
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
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() })
}

async fn setup(db: &TestDb) -> String {
    insert_user(&db.client, UserRow { id: "boss", email: "boss@example.test", role: "admin", totp_enabled: true, ..Default::default() }, Some("x")).await;
    insert_user(&db.client, UserRow { id: "ed", email: "ed@example.test", role: "editor", ..Default::default() }, None).await;
    cookie_for("boss", 3, true)
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn admin_routes_accept_only_an_enrolled_admin_session() {
    let db = test_db().await;
    let app = app(&db);
    setup(&db).await;

    let (status, _) = call(&app, "GET", "/api/ui/v1/admin/users", "", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = call(&app, "GET", "/api/ui/v1/admin/users", &cookie_for("ed", 3, false), None).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::FORBIDDEN, Some("forbidden")));
    assert_eq!(audit_rows(&db.client, "rbac:denied").await.len(), 1);
    // An admin who must still enroll 2FA gets nothing but enrollment.
    insert_user(&db.client, UserRow { id: "new", email: "new@example.test", role: "admin", ..Default::default() }, None).await;
    let (status, body) = call(&app, "GET", "/api/ui/v1/admin/users", &cookie_for("new", 3, true), None).await;
    assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::FORBIDDEN, Some("two_factor_required")));
    // An enrolled admin whose session has not passed this app's code (an OAuth sign-in, a cookie
    // from before the claim) gets nothing but verification either.
    let (status, body) = call(&app, "GET", "/api/ui/v1/admin/users", &cookie_with("boss", 3, true, false), None).await;
    assert_eq!((status, body["error"]["message"].as_str()), (StatusCode::FORBIDDEN, Some("2단계 인증 코드를 먼저 확인해야 합니다.")));
    // A gateway Bearer token is never an admin session.
    let request = Request::builder().uri("/api/ui/v1/admin/users").header("host", "admin.example.test").header("authorization", "Bearer x")
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000)))).body(Body::empty()).unwrap();
    assert_eq!(app.clone().oneshot(request).await.unwrap().status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn users_views_and_commands() {
    let db = test_db().await;
    let app = app(&db);
    let boss = setup(&db).await;

    let (status, list) = call(&app, "GET", "/api/ui/v1/admin/users", &boss, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().iter().map(|u| u["id"].as_str().unwrap()).collect::<Vec<_>>(), ["boss", "ed"]);
    let (status, detail) = call(&app, "GET", "/api/ui/v1/admin/users/ed", &boss, None).await;
    assert_eq!((status, detail["role"].as_str()), (StatusCode::OK, Some("editor")));
    let (status, _) = call(&app, "GET", "/api/ui/v1/admin/users/missing", &boss, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.users.create", &boss, Some("email=bad&password=Passw0rd-x&confirmPassword=Passw0rd-x&role=viewer")).await;
    assert_eq!((status, body["error"]["fieldErrors"]["email"].as_str()), (StatusCode::BAD_REQUEST, Some("올바른 이메일 형식을 입력하세요.")));
    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.users.create", &boss, Some("email=v%40example.test&password=Passw0rd-x&confirmPassword=Passw0rd-x&role=viewer")).await;
    assert_eq!(status, StatusCode::OK);
    let redirect = body["redirectTo"].as_str().unwrap();
    assert!(redirect.starts_with("/admin/users/") && redirect.len() > "/admin/users/".len(), "{body}");

    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.users.updateRole", &boss, Some("id=ed&role=viewer")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"redirectTo": "/admin/users/ed"})));
    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.users.resetPassword", &boss, Some("id=ed&password=Passw0rd-x&confirmPassword=Passw0rd-x")).await;
    assert_eq!((status, body["error"]["message"].as_str()), (StatusCode::CONFLICT, Some("이 계정은 OAuth 로그인만 사용합니다 — 비밀번호를 설정할 수 없습니다.")));
    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.users.toggle", &boss, Some("id=ed")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"result": {"ok": true}, "refresh": true})));
    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.users.delete", &boss, Some("id=boss")).await;
    assert_eq!((status, body["error"]["message"].as_str()), (StatusCode::CONFLICT, Some("본인 계정은 삭제할 수 없습니다.")));
    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.users.delete", &boss, Some("id=ed")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"redirectTo": "/admin/users"})));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn security_view_and_commands() {
    let db = test_db().await;
    let app = app(&db);
    let boss = setup(&db).await;
    db.client.execute("INSERT INTO gateway_tokens (id, name, token_hash, role) VALUES ('g', 'gw', 'h', 'viewer')", &[]).await.unwrap();

    let (status, view) = call(&app, "GET", "/api/ui/v1/admin/security", &boss, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["policy"], json!({"require2faAdmin": false, "require2faUser": false, "require2faOauth": false}));
    assert_eq!(view["gateways"][0]["id"], "g");
    assert_eq!(view["gateways"][0]["legacyOrphaned"], false);

    // Reject mistyped switches atomically; explicit false and absent remain off.
    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.security.updateTwoFactorPolicy", &boss, Some("require2faUser=true&require2faOauth=on")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"]["fieldErrors"]["require2faOauth"].is_string());
    let (_, view) = call(&app, "GET", "/api/ui/v1/admin/security", &boss, None).await;
    assert_eq!(view["policy"], json!({"require2faAdmin": false, "require2faUser": false, "require2faOauth": false}));
    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.security.updateTwoFactorPolicy", &boss, Some("require2faUser=true&require2faOauth=false")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"result": {"ok": true}, "refresh": true})));
    let (_, view) = call(&app, "GET", "/api/ui/v1/admin/security", &boss, None).await;
    assert_eq!(view["policy"], json!({"require2faAdmin": false, "require2faUser": true, "require2faOauth": false}));

    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.security.revokeGateway", &boss, Some("id=g")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"result": {"ok": true}, "refresh": true})));
    let (status, _) = call(&app, "POST", "/api/ui/v1/forms/admin.security.revokeGateway", &boss, Some("id=missing")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn server_tokens_are_admin_only_and_pair() {
    let db = test_db().await;
    let app = app(&db);
    let boss = setup(&db).await;

    let (status, list) = call(&app, "GET", "/api/ui/v1/admin/tokens", &boss, None).await;
    assert_eq!((status, list["tokens"].clone()), (StatusCode::OK, json!([])));
    assert_eq!(list["issuableScopes"], json!(["config:read", "config:write", "nginx:reload", "user:manage", "node:pair"]));

    let (status, refused) = call(&app, "POST", "/api/ui/v1/forms/admin.tokens.issue", &boss, Some("name=gw&scopes=node%3Apair&expiry=never")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(refused["error"]["fieldErrors"]["expiresAt"].is_string());
    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.tokens.issue", &boss, Some("name=gw&scopes=node%3Apair&expiry=90d")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["ok"], true, "IssueTokenState.ok");
    let token = body["result"]["token"].as_str().unwrap().to_owned();
    let id = body["result"]["id"].as_str().unwrap().to_owned();
    assert!(token.starts_with("nwu_s_"));
    nwui_api::auth::principal::authenticate_pairing(&db.state, &token, &auth_support::ctx(), nwui_api::auth::principal::now_unix_ms()).await.unwrap();

    let (status, detail) = call(&app, "GET", &format!("/api/ui/v1/admin/tokens/{id}"), &boss, None).await;
    assert_eq!((status, detail["name"].as_str()), (StatusCode::OK, Some("gw")));
    // An editor never reaches the server-token surface.
    let (status, _) = call(&app, "GET", "/api/ui/v1/admin/tokens", &cookie_for("ed", 3, false), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.tokens.revoke", &boss, Some(&format!("id={id}"))).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"result": {"ok": true}, "refresh": true})));
    let (status, _) = call(&app, "POST", "/api/ui/v1/forms/admin.tokens.revoke", &boss, Some(&format!("id={id}"))).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn oauth_provider_views_and_commands() {
    let db = test_db().await;
    let app = app(&db);
    let boss = setup(&db).await;
    let create = "id=kc&name=Keycloak&issuer=https%3A%2F%2Fidp.example.test&clientId=c&clientSecret=s&scope=&enabled=true&sortOrder=0";

    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.oauth.create", &boss, Some("id=kc&issuer=http%3A%2F%2Fx")).await;
    assert_eq!((status, body["error"]["fieldErrors"]["issuer"].as_str()), (StatusCode::BAD_REQUEST, Some("로그인 서버 주소는 https 여야 합니다.")));
    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.oauth.create", &boss, Some(create)).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"redirectTo": "/admin/oauth/kc"})));

    let (status, list) = call(&app, "GET", "/api/ui/v1/admin/oauth", &boss, None).await;
    assert_eq!((status, list[0]["id"].as_str(), list[0]["scope"].as_str()), (StatusCode::OK, Some("kc"), Some("openid email profile")));
    let (status, detail) = call(&app, "GET", "/api/ui/v1/admin/oauth/kc", &boss, None).await;
    assert_eq!((status, detail["callbackUrl"].as_str()), (StatusCode::OK, Some("/api/auth/callback/kc")));
    assert!(!detail.to_string().contains("enc:v1:"));

    let update = create.replace("clientSecret=s", "clientSecret=__keep__").replace("name=Keycloak", "name=KC");
    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.oauth.update", &boss, Some(&update)).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"redirectTo": "/admin/oauth/kc"})));
    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.oauth.toggle", &boss, Some("id=kc")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"redirectTo": "/admin/oauth"})));
    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.oauth.delete", &boss, Some("id=kc")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"redirectTo": "/admin/oauth"})));
    let (status, _) = call(&app, "GET", "/api/ui/v1/admin/oauth/kc", &boss, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn general_settings_view_and_commands() {
    let db = test_db().await;
    let app = app(&db);
    let boss = setup(&db).await;
    let path = "/api/ui/v1/admin/settings/general";

    let (status, view) = call(&app, "GET", path, &boss, None).await;
    assert_eq!((status, view), (StatusCode::OK, json!({"publicBaseUrl": null, "metricsRetentionDays": null, "metricsExcludedHosts": null})));

    let url = "/api/ui/v1/forms/admin.settings.updatePublicBaseUrl";
    let (status, body) = call(&app, "POST", url, &boss, Some("publicBaseUrl=ftp%3A%2F%2Fx")).await;
    assert_eq!((status, body["error"]["fieldErrors"]["publicBaseUrl"].as_str()), (StatusCode::BAD_REQUEST, Some("http:// 또는 https:// 주소만 사용할 수 있습니다.")));
    let (_, body) = call(&app, "POST", url, &boss, Some("publicBaseUrl=not+a+url")).await;
    assert_eq!(body["error"]["fieldErrors"]["publicBaseUrl"], "URL 형식이 올바르지 않습니다.");
    let (status, body) = call(&app, "POST", url, &boss, Some("publicBaseUrl=+https%3A%2F%2Fnginx.example.test%2F%2F+")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"result": {"ok": true, "message": "앱 주소를 저장했습니다."}, "refresh": true})));
    let (_, view) = call(&app, "GET", path, &boss, None).await;
    assert_eq!(view["publicBaseUrl"], "https://nginx.example.test");
    let (_, body) = call(&app, "POST", url, &boss, Some("publicBaseUrl=")).await;
    assert_eq!(body["result"]["message"], "앱 주소를 지웠습니다.");
    assert_eq!(audit_rows(&db.client, "settings:public-base-url").await.len(), 2);

    let metrics = "/api/ui/v1/forms/admin.settings.updateMetrics";
    for bad in ["0", "3651", "1.5", "1e2", "x"] {
        let (status, body) = call(&app, "POST", metrics, &boss, Some(&format!("metricsRetentionDays={bad}"))).await;
        assert_eq!((status, body["error"]["fieldErrors"]["metricsRetentionDays"].as_str()), (StatusCode::BAD_REQUEST, Some("1 이상 3650 이하의 정수여야 합니다(비우면 무제한).")), "{bad}");
    }
    let (_, body) = call(&app, "POST", metrics, &boss, Some("metricsExcludedHosts=ok.example%0Abad+host")).await;
    assert_eq!(body["error"]["fieldErrors"]["metricsExcludedHosts"], "도메인 형식이 아닙니다: bad host");
    let (status, body) = call(&app, "POST", metrics, &boss, Some("metricsRetentionDays=+90+&metricsExcludedHosts=Admin.Example.TEST%2C+%0A+b.example%0A")).await;
    assert_eq!((status, body["result"]["message"].as_str()), (StatusCode::OK, Some("보존 기간을 90일로 설정했습니다.")));
    let (_, view) = call(&app, "GET", path, &boss, None).await;
    assert_eq!((view["metricsRetentionDays"].clone(), view["metricsExcludedHosts"].clone()), (json!(90), json!("admin.example.test\nb.example")));
    let (_, body) = call(&app, "POST", metrics, &boss, Some("metricsRetentionDays=&metricsExcludedHosts=")).await;
    assert_eq!(body["result"]["message"], "보존 기간을 무제한으로 설정했습니다.");
    let audit = audit_rows(&db.client, "settings:metrics").await;
    assert_eq!(audit.last().unwrap().get::<_, Option<Value>>("after"), Some(json!({"metricsRetentionDays": null, "metricsExcludedHosts": null})));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn secrets_health_classifies_every_sealed_column() {
    let db = test_db().await;
    let app = app(&db);
    let boss = setup(&db).await;
    let ok = db.state.secrets.seal("v").unwrap();
    let other_key = nwui_api::crypto::SecretBox::from_key([9u8; 32]).seal("v").unwrap();
    db.client.batch_execute(&format!("
        INSERT INTO oauth_providers (id, name, issuer, client_id, client_secret_enc) VALUES ('kc', 'KC', 'https://i.test', 'c', '{ok}');
        INSERT INTO smtp_settings (id, host, port, from_email, password_enc) VALUES ('current', 'smtp.example.test', 587, 'a@b.c', '{other_key}');
        INSERT INTO webhook_targets (id, name, format, url_enc, headers_enc, events) VALUES ('w', 'hook', 'generic', 'https://plain.example.test', '{other_key}', '[]');
        UPDATE users SET totp_secret = 'JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP' WHERE id = 'boss';
    ")).await.unwrap();

    let (status, view) = call(&app, "GET", "/api/ui/v1/admin/secrets", &boss, None).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view, json!([
        {"kind": "oauth", "label": "KC", "state": "sealed-ok", "href": "/admin/oauth/kc/edit"},
        {"kind": "smtp", "label": "smtp.example.test", "state": "sealed-broken", "href": "/admin/settings/smtp"},
        {"kind": "webhook", "label": "hook", "state": "sealed-broken", "href": "/admin/notifications/webhooks/w/edit"},
        {"kind": "totp", "label": "boss@example.test", "state": "plaintext", "href": "/admin/users/boss"},
    ]), "a broken header outranks a plaintext URL; only 2FA users are listed");
    assert!(!view.to_string().contains("JBSWY3DP"), "never the values");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn pending_queue_views_and_commands() {
    let db = test_db().await;
    let app = app(&db);
    let boss = setup(&db).await;
    insert_user(&db.client, UserRow { id: "p", email: "p@example.test", role: "viewer", approved: false, ..Default::default() }, None).await;

    let (status, queue) = call(&app, "GET", "/api/ui/v1/admin/pending", &boss, None).await;
    assert_eq!((status, queue.as_array().unwrap().len()), (StatusCode::OK, 1));
    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.pending.approve", &boss, Some("id=p&role=editor")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"redirectTo": "/admin/pending"})));
    let (_, queue) = call(&app, "GET", "/api/ui/v1/admin/pending", &boss, None).await;
    assert_eq!(queue, json!([]));
    let (status, body) = call(&app, "POST", "/api/ui/v1/forms/admin.pending.reject", &boss, Some("id=p")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"redirectTo": "/admin/pending"})));
    assert_eq!(audit_rows(&db.client, "user:reject").await.len(), 0, "an approved account is not rejected");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn audit_s5_issuer_change_refuses_existing_identity_links() {
    let db = test_db().await;
    let boss = setup(&db).await;
    let secret = db.state.secrets.seal("provider-secret").unwrap();
    db.client.execute("INSERT INTO oauth_providers(id,name,issuer,client_id,client_secret_enc) VALUES('kc','KC','https://old.example.test','c',$1)", &[&secret]).await.unwrap();
    db.client.batch_execute("INSERT INTO oauth_accounts(provider_id,subject,user_id) VALUES('kc','42','boss')").await.unwrap();
    let base = "id=kc&name=Renamed&clientId=c&clientSecret=__keep__&scope=openid&enabled=true&sortOrder=0";
    let (status, value) = call(&app(&db), "POST", "/api/ui/v1/forms/admin.oauth.update", &boss, Some(&format!("{base}&issuer=https%3A%2F%2Fnew.example.test"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a new issuer must not inherit old subject links: {value}");
    assert!(value["error"]["fieldErrors"]["issuer"].is_string());
    assert_eq!(db.client.query_one("SELECT issuer FROM oauth_providers WHERE id='kc'", &[]).await.unwrap().get::<_, String>(0), "https://old.example.test");
    assert_eq!(db.client.query_one("SELECT count(*) FROM oauth_accounts WHERE provider_id='kc'", &[]).await.unwrap().get::<_, i64>(0), 1);
    assert_eq!(call(&app(&db), "POST", "/api/ui/v1/forms/admin.oauth.update", &boss, Some(&format!("{base}&issuer=https%3A%2F%2Fold.example.test"))).await.0, StatusCode::OK);
}

#[tokio::test]
#[ignore = "real DB"]
async fn q3r1_self_reset_accepts_recent_authentication_but_requires_reauth_otherwise() {
    let db=test_db().await;let app=app(&db);setup(&db).await;
    db.client.execute("UPDATE users SET password_hash=$1 WHERE id='boss'",&[&nwui_api::auth::password::hash_password("Old-Passw0rd!").unwrap()]).await.unwrap();
    let now=nwui_api::auth::principal::now_unix_ms()/1000;
    for (age,allowed) in [(601,false),(30,true)] {
        let claims=json!({"id":"boss","sv":3,"mfaVerified":true,"authTime":now-age});
        let token=session::encode(claims.as_object().unwrap(),auth_support::SECRET,session::COOKIE_NAME_PLAIN,now,3600).unwrap();
        let (status,body)=call(&app,"POST","/api/ui/v1/forms/admin.users.resetPassword",&format!("authjs.session-token={token}"),Some("id=boss&password=New-Passw0rd%21&confirmPassword=New-Passw0rd%21")).await;
        assert_eq!(status==StatusCode::OK,allowed,"age {age}: {body}");
        if !allowed {assert!(body.to_string().contains("인증") || body.to_string().contains("로그인"));}
    }
}
