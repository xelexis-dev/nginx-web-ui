//! C1: credential exclusivity, live authority, route census and token lifecycle at use.
mod auth_support;
use auth_support::{TestDb, UserRow, insert_user};
use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{HeaderMap, Request, header},
    response::Response,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use nwui_api::auth::{
    principal::{self, TwoFactorGate, Use},
    rbac::Action,
    route_policy, user_token,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, net::SocketAddr, time::Duration};
use tower::ServiceExt;

fn credential() -> String {
    format!("nwu_u_{}", URL_SAFE_NO_PAD.encode([7; 32]))
}
fn headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        header::AUTHORIZATION,
        format!("Bearer {}", credential()).parse().unwrap(),
    );
    h
}
async fn fixture(scopes: &[&str]) -> TestDb {
    fixture_with(scopes, &[]).await
}
async fn fixture_with(scopes: &[&str], extra: &[(&str, &str)]) -> TestDb {
    let db = auth_support::test_db_with(extra).await;
    insert_user(
        &db.client,
        UserRow {
            role: "admin",
            totp_enabled: true,
            ..Default::default()
        },
        None,
    )
    .await;
    db.client.execute("INSERT INTO access_tokens(id,name,kind,owner_user_id,token_hash,token_suffix,scopes) VALUES('pat','automation','user','u1',$1,'test',$2)",&[&nwui_api::auth::token_digest(&credential()),&json!(scopes)]).await.unwrap();
    db
}
async fn user(db: &TestDb) -> principal::UserPrincipal {
    principal::require_user(
        &db.state,
        &headers(),
        &auth_support::ctx(),
        None,
        TwoFactorGate::Enforce,
        Use::SessionOrToken,
    )
    .await
    .unwrap()
}
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    auth: Option<&str>,
    cookie: Option<&str>,
    extra: &[(&str, &str)],
) -> Response {
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "ui.example.test")
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));
    if let Some(auth) = auth {
        r = r.header(header::AUTHORIZATION, auth);
    }
    if let Some(cookie) = cookie {
        r = r.header(header::COOKIE, cookie);
    }
    for &(k, v) in extra {
        r = r.header(k, v);
    }
    app.clone()
        .oneshot(r.body(Body::empty()).unwrap())
        .await
        .unwrap()
}
async fn body(response: Response) -> Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap(),
    )
    .unwrap()
}
fn document() -> Value {
    serde_json::from_str(include_str!("../../../packages/contracts/openapi.json")).unwrap()
}

#[test]
fn strict_header_parsing_and_fetch_metadata() {
    let valid = format!("bEaReR {}", credential());
    let mut h = HeaderMap::new();
    h.insert(header::AUTHORIZATION, valid.parse().unwrap());
    assert_eq!(user_token::parse_header(&h).unwrap(), credential());
    for bad in [
        format!("Bearer {},ignored", credential()),
        format!("Basic {}", credential()),
        format!("Bearer {} ", credential()),
        format!("Bearer  {}", credential()),
        credential().replace("nwu_u_", "Bearer nwu_s_"),
        "Bearer gateway".into(),
        format!("Bearer nwu_u_{}", "_".repeat(43)),
        format!("Bearer nwu_u_{}=", URL_SAFE_NO_PAD.encode([7; 32])),
    ] {
        h.insert(header::AUTHORIZATION, bad.parse().unwrap());
        assert_eq!(
            user_token::parse_header(&h).unwrap_err().code,
            "invalid_token"
        );
    }
    h = headers();
    h.append(header::AUTHORIZATION, valid.parse().unwrap());
    assert!(user_token::parse_header(&h).is_err());
    for value in ["cross-site", "same-site", "invalid"] {
        h.insert("sec-fetch-site", value.parse().unwrap());
        assert!(user_token::check_fetch_metadata(&h).is_err());
    }
    for value in ["none", "same-origin"] {
        h.insert("sec-fetch-site", value.parse().unwrap());
        assert!(user_token::check_fetch_metadata(&h).is_ok());
    }
    assert!(user_token::parse_header(&HeaderMap::new()).is_err());
}

#[test]
fn every_openapi_operation_and_view_has_exactly_one_explicit_class() {
    let doc = document();
    let mut seen = std::collections::HashSet::new();
    for route in route_policy::ROUTES {
        assert!(
            seen.insert((route.method, route.path)),
            "duplicate class {} {}",
            route.method,
            route.path
        );
    }
    for (path, item) in doc["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap().iter().filter(|(m, _)| {
            matches!(
                m.as_str(),
                "get" | "post" | "patch" | "delete" | "put" | "options" | "head"
            )
        }) {
            let route = route_policy::ROUTES
                .iter()
                .find(|r| r.path == path && r.method.eq_ignore_ascii_case(method))
                .unwrap_or_else(|| panic!("unclassified {method} {path}"));
            assert_eq!(op["operationId"], route.operation_id);
            if route.usage == Use::SessionOrToken {
                assert_eq!(
                    op["security"],
                    json!([{"browserSession":[]},{"userToken":[]}])
                );
                if route.operation_id != "readBrowserView" {
                    assert_eq!(op["x-nwui-token-scope"], json!(route.actions));
                }
                for code in ["401", "403", "429"] {
                    assert!(op["responses"][code].is_object());
                }
            } else {
                assert!(
                    !op["security"]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|v| v.get("userToken").is_some()))
                );
            }
        }
    }
    assert_eq!(
        route_policy::VIEWS.len(),
        doc["x-view-registry"].as_object().unwrap().len()
    );
    for &(path, actions) in route_policy::VIEWS {
        assert_eq!(
            doc["x-view-registry"][path]["x-nwui-token-scope"],
            json!(actions)
        );
    }
    assert!(route_policy::route("POST", "/api/ui/v1/forms/unknown.futureOperation").is_none());
    for path in [
        "/account",
        "/admin/users",
        "/unknown",
        "/reverse-proxy/x/not-a-view",
    ] {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("path", path)
            .finish();
        assert_eq!(
            route_policy::view_actions(Some(&query)).unwrap_err().code,
            "session_required_for_action"
        );
    }
}

#[test]
fn every_router_registration_is_in_the_census() {
    // Inspect registrations rather than trusting the contract: adding even an unpublished route must fail.
    fn sources(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources(&path, files)
            } else if path.extension().is_some_and(|e| e == "rs") {
                files.push(path)
            }
        }
    }
    let literal=regex::Regex::new(r#"\.route\(\s*"([^"]+)"\s*,\s*((?:get|post|put|patch|delete|head|options)\([^\n]*|[\s\S]*?)\)"#).unwrap();
    let method = regex::Regex::new(r"\b(get|post|put|patch|delete|head|options)\(").unwrap();
    let dynamic = regex::Regex::new(r#"\.route\(\s*(&format!\("[^"]+"\)|&base)"#).unwrap();
    let allowed = [
        "&base",
        "&format!(\"{base}/{{id}}\")",
        "&format!(\"{base}/{{id}}/toggle\")",
        "&format!(\"/api/ui/v1/forms/{id}\")",
        "&format!(\"/api/ui/v1/forms/{command}\")",
        "&format!(\"/api/v1/certificates/{{id}}/{operation}\")",
    ];
    let mut files = vec![];
    sources(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    for file in files {
        let raw = std::fs::read_to_string(&file).unwrap();
        if file.file_name().unwrap().to_string_lossy().ends_with("_tests.rs") { continue; }
        let registrations=raw.split("#[cfg(test)]").next().unwrap();
        let text = registrations
            .lines()
            .map(|l| l.split("//").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        let count = text.matches(".route(").count();
        let captures: Vec<_> = literal.captures_iter(&text).collect();
        let dynamics: Vec<_> = dynamic.captures_iter(&text).collect();
        assert_eq!(
            count,
            captures.len() + dynamics.len(),
            "unrecognized registration in {}",
            file.display()
        );
        for c in captures {
            let path = &c[1];
            // Method chains end at the next comma/semicolon; their methods all need a class.
            let start = c.get(2).unwrap().start();
            let tail = &text[start..];
            let end = tail
                .find(';')
                .unwrap_or(tail.len())
                .min(tail.find(".route(").unwrap_or(tail.len()));
            for m in method.captures_iter(&tail[..end]) {
                assert!(
                    route_policy::ROUTES
                        .iter()
                        .any(|r| r.path == path && r.method.eq_ignore_ascii_case(&m[1])),
                    "unclassified {} {path} in {}",
                    &m[1],
                    file.display()
                );
            }
        }
        for c in dynamics {
            assert!(
                allowed.contains(&&c[1]),
                "new dynamic route {} in {}",
                &c[1],
                file.display()
            );
        }
    }
    for ids in [
        nwui_api::control::routes::FORM_IDS,
        nwui_api::control::operations::FORM_IDS,
        nwui_api::certificates::routes::FORM_IDS,
        nwui_api::nodes::routes::FORM_IDS,
        nwui_api::nodes::relay::FORM_IDS,
    ] {
        for id in ids {
            assert!(
                route_policy::route("POST", &format!("/api/ui/v1/forms/{id}")).is_some(),
                "{id}"
            );
        }
    }
    for plural in ["servers", "upstreams", "streams"] {
        for (suffix, methods) in [
            ("", &["GET", "POST"][..]),
            ("/{id}", &["GET", "PATCH", "DELETE"][..]),
        ] {
            for method in methods {
                assert!(
                    route_policy::route(method, &format!("/api/v1/{plural}{suffix}")).is_some()
                );
            }
        }
    }
    for operation in [
        "retry",
        "renew",
        "renew-dns",
        "dns-challenge",
        "cancel",
        "replace",
    ] {
        assert!(
            route_policy::route("POST", &format!("/api/v1/certificates/x/{operation}")).is_some()
        );
    }
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn live_role_scope_and_node_intersection_matrix_and_view_flags() {
    let db = fixture(&[]).await;
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id) VALUES('node','Node','http://127.0.0.1:1','sealed','admin','instance')",&[]).await.unwrap();
    let sets: &[&[&str]] = &[
        &["config:read"],
        &["config:read", "config:write"],
        &["config:read", "nginx:reload"],
        &["config:read", "config:write", "nginx:reload"],
        &["config:read", "config:write", "nginx:reload", "user:manage"],
        &["node:pair", "unknown"],
    ];
    for (role, allowed) in [
        ("viewer", &[Action::ConfigRead][..]),
        (
            "editor",
            &[Action::ConfigRead, Action::ConfigWrite, Action::NginxReload][..],
        ),
        ("admin", &Action::ALL[..]),
    ] {
        db.client
            .execute(
                "UPDATE users SET role=$1::text::role WHERE id='u1'",
                &[&role],
            )
            .await
            .unwrap();
        for scopes in sets {
            db.client
                .execute(
                    "UPDATE access_tokens SET scopes=$1 WHERE id='pat'",
                    &[&json!(scopes)],
                )
                .await
                .unwrap();
            let actor = user(&db).await;
            for action in Action::ALL {
                assert_eq!(
                    actor.can(action),
                    allowed.contains(&action) && scopes.contains(&action.as_str()),
                    "{role} {scopes:?} {action:?}"
                );
            }
            for route in route_policy::ROUTES
                .iter()
                .filter(|r| r.usage == Use::SessionOrToken)
            {
                let expected = route
                    .actions
                    .iter()
                    .all(|a| allowed.contains(a) && scopes.contains(&a.as_str()));
                assert_eq!(
                    route.actions.iter().all(|&a| actor.can(a)),
                    expected,
                    "{}",
                    route.operation_id
                );
            }
            if actor.can(Action::ConfigRead) {
                let flags = nwui_api::control::views::view(
                    &db.state,
                    &actor,
                    "/reverse-proxy",
                    &BTreeMap::new(),
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(flags["canWrite"], actor.can(Action::ConfigWrite));
                assert_eq!(flags["canApply"], actor.can(Action::NginxReload));
            }
            for (node_role, node_allowed) in [
                ("viewer", &[Action::ConfigRead][..]),
                (
                    "editor",
                    &[Action::ConfigRead, Action::ConfigWrite, Action::NginxReload][..],
                ),
                ("admin", &Action::ALL[..]),
            ] {
                db.client
                    .execute(
                        "UPDATE nodes SET token_role=$1::text::role WHERE id='node'",
                        &[&node_role],
                    )
                    .await
                    .unwrap();
                for action in Action::ALL {
                    assert_eq!(
                        nwui_api::nodes::service::resolve(&db.state, &actor, "node", action)
                            .await
                            .is_ok(),
                        actor.can(action) && node_allowed.contains(&action),
                        "{role} {scopes:?} {node_role} {action:?}"
                    );
                }
            }
        }
    }
    // A stored scope cannot be enlarged by a promotion; a downgrade applies on the very next guard.
    db.client
        .execute(
            "UPDATE access_tokens SET scopes='[\"config:read\",\"config:write\"]' WHERE id='pat'",
            &[],
        )
        .await
        .unwrap();
    for role in ["admin", "viewer", "admin"] {
        db.client
            .execute(
                "UPDATE users SET role=$1::text::role WHERE id='u1'",
                &[&role],
            )
            .await
            .unwrap();
        let actor = user(&db).await;
        assert_eq!(actor.can(Action::ConfigWrite), role == "admin");
        assert!(!actor.can(Action::UserManage));
        assert!(!actor.can(Action::NginxReload));
    }
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn invalid_lifecycle_states_have_identical_errors_and_factor_is_live() {
    let db = fixture(&["config:read"]).await;
    let app = nwui_api::server::router(db.state.clone());
    let auth = format!("Bearer {}", credential());
    let mut reference = None;
    for sql in [
        "UPDATE access_tokens SET revoked_at=now() WHERE id='pat'",
        "UPDATE access_tokens SET expires_at=now() WHERE id='pat'",
        "UPDATE users SET enabled=false WHERE id='u1'",
        "UPDATE users SET approved=false WHERE id='u1'",
    ] {
        db.client.batch_execute("UPDATE access_tokens SET revoked_at=NULL,expires_at=NULL; UPDATE users SET enabled=true,approved=true").await.unwrap();
        db.client.batch_execute(sql).await.unwrap();
        let response = call(
            &app,
            "GET",
            "/api/ui/v1/notifications",
            Some(&auth),
            None,
            &[],
        )
        .await;
        assert_eq!(response.status(), 401);
        assert_eq!(
            response.headers()[header::WWW_AUTHENTICATE],
            "Bearer realm=\"nginx-web-ui\", error=\"invalid_token\""
        );
        assert!(!response.headers().contains_key(header::SET_COOKIE));
        let value = body(response).await;
        if let Some(expected) = &reference {
            assert_eq!(&value, expected)
        } else {
            reference = Some(value)
        }
    }
    db.client.batch_execute("UPDATE access_tokens SET revoked_at=NULL,expires_at=NULL; UPDATE users SET enabled=true,approved=true,totp_enabled=false").await.unwrap();
    let response = call(
        &app,
        "GET",
        "/api/ui/v1/notifications",
        Some(&auth),
        None,
        &[],
    )
    .await;
    assert_eq!(response.status(), 403);
    assert_eq!(
        body(response).await,
        json!({"error":{"code":"two_factor_required","message":"토큰 주인이 2단계 인증을 등록해야 토큰을 쓸 수 있습니다."}})
    );
    db.client
        .batch_execute("UPDATE users SET totp_enabled=true; DELETE FROM users WHERE id='u1'")
        .await
        .unwrap();
    assert_eq!(
        body(
            call(
                &app,
                "GET",
                "/api/ui/v1/notifications",
                Some(&auth),
                None,
                &[]
            )
            .await
        )
        .await,
        reference.unwrap()
    );
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn bearer_is_exclusive_and_cookie_csrf_stays_in_force() {
    let db = fixture(&["config:read"]).await;
    let app = nwui_api::server::router(db.state.clone());
    let auth = format!("Bearer {}", credential());
    let claims = json!({"id":"u1","sv":3});
    let cookie = nwui_api::auth::session::encode(
        claims.as_object().unwrap(),
        auth_support::SECRET,
        nwui_api::auth::session::COOKIE_NAME_PLAIN,
        principal::now_unix_ms() / 1000,
        3600,
    )
    .unwrap();
    let cookie = format!("authjs.session-token={cookie}");
    for (method, path) in [
        ("GET", "/api/ui/v1/notifications"),
        ("POST", "/api/ui/v1/notifications/seen"),
    ] {
        let response = call(
            &app,
            method,
            path,
            Some(&auth),
            Some("authjs.session-token=broken"),
            &[],
        )
        .await;
        assert_eq!(response.status(), 200);
        assert!(!response.headers().contains_key(header::SET_COOKIE));
        let bad = call(
            &app,
            method,
            path,
            Some("Bearer nwu_u_invalid"),
            Some(&cookie),
            &[],
        )
        .await;
        assert_eq!(bad.status(), 401);
        assert!(!bad.headers().contains_key(header::SET_COOKIE));
    }
    for bad in [
        credential().replace("nwu_u_", "Bearer nwu_s_"),
        "Bearer oidc-credential".into(),
        "Basic ignored".into(),
    ] {
        assert_eq!(
            call(
                &app,
                "GET",
                "/api/ui/v1/notifications",
                Some(&bad),
                Some(&cookie),
                &[]
            )
            .await
            .status(),
            200
        );
    }
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/ui/v1/notifications/seen",
            None,
            Some(&cookie),
            &[]
        )
        .await
        .status(),
        403
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/ui/v1/notifications/seen",
            None,
            Some(&cookie),
            &[("origin", "http://ui.example.test")]
        )
        .await
        .status(),
        200
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/ui/v1/notifications/seen",
            Some(&auth),
            None,
            &[("sec-fetch-site", "cross-site")]
        )
        .await
        .status(),
        403
    );
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/api/ui/v1/notifications?access_token={}", credential()),
            None,
            None,
            &[]
        )
        .await
        .status(),
        401
    );
    let request = Request::builder()
        .method("POST")
        .uri("/api/ui/v1/notifications/seen")
        .header("host", "ui.example.test")
        .header("origin", "http://ui.example.test")
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))))
        .body(Body::from(format!("token={}", credential())))
        .unwrap();
    assert_eq!(app.oneshot(request).await.unwrap().status(), 401);
}

async fn database_snapshot(db: &TestDb) -> BTreeMap<String, Value> {
    let mut snapshot = BTreeMap::new();
    for row in db
        .client
        .query(
            "SELECT tablename FROM pg_tables WHERE schemaname='public' ORDER BY tablename",
            &[],
        )
        .await
        .unwrap()
    {
        let table: String = row.get(0);
        let escaped = table.replace('"', "\"\"");
        let rows:Value=db.client.query_one(&format!("SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM \"{escaped}\" t"),&[]).await.unwrap().get(0);
        snapshot.insert(table, rows);
    }
    snapshot
}
#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn every_session_only_user_operation_refuses_a_valid_token_without_side_effects() {
    let db = fixture(&["config:read", "config:write", "nginx:reload", "user:manage"]).await;
    let before = database_snapshot(&db).await;
    let app = nwui_api::server::router(db.state.clone());
    let auth = format!("Bearer {}", credential());
    for route in route_policy::ROUTES
        .iter()
        .filter(|r| r.user_surface && r.usage == Use::SessionOnly)
    {
        let path = route.path.replace("{id}", "u1");
        let response = call(&app, route.method, &path, Some(&auth), None, &[]).await;
        assert_eq!(response.status(), 403, "{} {}", route.method, route.path);
        assert!(!response.headers().contains_key(header::SET_COOKIE));
        assert_eq!(
            body(response).await["error"]["code"],
            "session_required_for_action"
        );
    }
    let unpublished=Router::new().route("/api/future-alias",axum::routing::post(||async {axum::http::StatusCode::IM_A_TEAPOT}));
    let unpublished=nwui_api::app::finish_router(nwui_api::app::with_user_request_context(unpublished,db.state.clone()),"2.1.0");
    assert_eq!(call(&unpublished,"POST","/api/future-alias",Some(&auth),None,&[]).await.status(),403);
    assert_eq!(database_snapshot(&db).await, before);
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn last_use_is_throttled_success_only_and_scope_denial_has_typed_provenance() {
    let db = fixture(&["config:read"]).await;
    let app = nwui_api::server::router(db.state.clone());
    let auth = format!("Bearer {}", credential());
    for path in [
        "/api/ui/v1/forms/apply.applyConfig",
        "/api/ui/v1/forms/admin.users.toggle",
        "/api/ui/v1/forms/history.rollback",
    ] {
        let response = call(&app, "POST", path, Some(&auth), None, &[]).await;
        assert_eq!(response.status(), 403);
        assert_eq!(body(response).await["error"]["code"], "insufficient_scope");
    }
    assert!(
        db.client
            .query_one(
                "SELECT last_used_at IS NULL FROM access_tokens WHERE id='pat'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
    let entries = db
        .client
        .query("SELECT via FROM audit_logs WHERE action='rbac:denied'", &[])
        .await
        .unwrap();
    assert_eq!(entries.len(), 3);
    for row in entries {
        assert_eq!(
            row.get::<_, Value>(0),
            json!({"kind":"token","id":"pat","name":"automation"})
        );
    }
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/ui/v1/notifications",
            Some(&auth),
            None,
            &[]
        )
        .await
        .status(),
        200
    );
    let first: chrono::DateTime<chrono::Utc> = db
        .client
        .query_one("SELECT last_used_at FROM access_tokens WHERE id='pat'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/ui/v1/notifications",
            Some(&auth),
            None,
            &[]
        )
        .await
        .status(),
        200
    );
    assert_eq!(
        db.client
            .query_one("SELECT last_used_at FROM access_tokens WHERE id='pat'", &[])
            .await
            .unwrap()
            .get::<_, chrono::DateTime<chrono::Utc>>(0),
        first
    );
    db.client
        .execute(
            "UPDATE access_tokens SET last_used_at=now()-interval '61 seconds' WHERE id='pat'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/ui/v1/notifications",
            Some(&auth),
            None,
            &[]
        )
        .await
        .status(),
        200
    );
    assert!(
        db.client
            .query_one(
                "SELECT last_used_at>now()-interval '5 seconds' FROM access_tokens WHERE id='pat'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn bad_bearer_uses_ip_throttle_and_valid_token_budgets_have_retry_after() {
    let db = fixture(&["config:read"]).await;
    let app = nwui_api::server::router(db.state.clone());
    let auth = format!("Bearer {}", credential());
    for _ in 0..30 {
        assert_eq!(
            call(
                &app,
                "GET",
                "/api/ui/v1/notifications",
                Some("Bearer nwu_u_invalid"),
                None,
                &[]
            )
            .await
            .status(),
            401
        );
    }
    let blocked = call(
        &app,
        "GET",
        "/api/ui/v1/notifications",
        Some("Bearer nwu_u_invalid"),
        None,
        &[],
    )
    .await;
    assert_eq!(blocked.status(), 429);
    assert!(blocked.headers().contains_key(header::RETRY_AFTER));
    // A different trusted client address is unaffected by guesses from the first one.
    let request = Request::builder()
        .uri("/api/ui/v1/notifications")
        .header("host", "ui.example.test")
        .header("authorization", &auth)
        .extension(ConnectInfo(SocketAddr::from(([198, 51, 100, 8], 12345))))
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.oneshot(request).await.unwrap().status(), 200);
    for _ in 0..60 {
        db.state
            .runtime
            .user_token_limits
            .admit("pat", "u1", true, false)
            .unwrap();
    }
    let response = call(
        &nwui_api::server::router(db.state.clone()),
        "POST",
        "/api/ui/v1/notifications/seen",
        Some(&auth),
        None,
        &[("x-real-ip", "198.51.100.9")],
    )
    .await;
    // The peer above is loopback (trusted proxy); x-real-ip supplies the independent address.
    assert_eq!(response.status(), 429);
    assert!(response.headers().contains_key(header::RETRY_AFTER));
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn token_stream_revocation_expiry_and_database_failure_close_the_lifetime() {
    let db = fixture(&["config:read"]).await;
    let stopped = nwui_api::auth::stream_guard::token_lifetime(
        &db.state,
        headers(),
        auth_support::ctx(),
        vec![Action::ConfigRead],
        None,
    );
    let task = tokio::spawn(stopped);
    db.client
        .execute(
            "UPDATE access_tokens SET revoked_at=now() WHERE id='pat'",
            &[],
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(32), task)
        .await
        .expect("revoked token must close within a revalidation period")
        .unwrap();
    db.client
        .execute(
            "UPDATE access_tokens SET revoked_at=NULL WHERE id='pat'",
            &[],
        )
        .await
        .unwrap();
    let stopped = nwui_api::auth::stream_guard::token_lifetime(
        &db.state,
        headers(),
        auth_support::ctx(),
        vec![Action::ConfigRead],
        Some(principal::now_unix_ms() + 150),
    );
    tokio::time::timeout(Duration::from_secs(1), stopped)
        .await
        .expect("expiry is exact, not the next 30 second poll");
    db.state.db.close();
    let stopped = nwui_api::auth::stream_guard::token_lifetime(
        &db.state,
        headers(),
        auth_support::ctx(),
        vec![Action::ConfigRead],
        None,
    );
    tokio::time::timeout(Duration::from_secs(32), stopped)
        .await
        .expect("database failure closes token streams");
    assert_eq!(
        principal::require_user(
            &db.state,
            &headers(),
            &auth_support::ctx(),
            None,
            TwoFactorGate::Enforce,
            Use::SessionOrToken
        )
        .await
        .unwrap_err()
        .status,
        503
    );
}

#[tokio::test]
#[ignore = "real PostgreSQL"]
async fn stream_response_owns_and_releases_its_concurrency_permit() {
    let db = fixture(&["config:read"]).await;
    let app = nwui_api::server::router(db.state.clone());
    let auth = format!("Bearer {}", credential());
    let mut streams = Vec::new();
    for _ in 0..3 {
        let response = call(
            &app,
            "GET",
            "/api/ui/v1/logs/stream",
            Some(&auth),
            None,
            &[],
        )
        .await;
        assert_eq!(response.status(), 200);
        streams.push(response);
    }
    let refused = call(
        &app,
        "GET",
        "/api/ui/v1/logs/stream",
        Some(&auth),
        None,
        &[],
    )
    .await;
    assert_eq!(refused.status(), 429);
    assert!(refused.headers().contains_key(header::RETRY_AFTER));
    drop(streams.pop());
    let reopened = call(
        &app,
        "GET",
        "/api/ui/v1/logs/stream",
        Some(&auth),
        None,
        &[],
    )
    .await;
    assert_eq!(reopened.status(), 200);
    drop(reopened);
    drop(streams);
    let reopened = call(
        &app,
        "GET",
        "/api/ui/v1/logs/stream",
        Some(&auth),
        None,
        &[],
    )
    .await;
    assert_eq!(reopened.status(), 200);
}

#[tokio::test]
#[ignore="real PostgreSQL"]
async fn view_paths_require_their_full_action_set_and_never_open_unknown_views() {
    let db=fixture(&["config:write"]).await;
    let app=nwui_api::server::router(db.state.clone());let auth=format!("Bearer {}",credential());
    for scopes in [json!(["config:write"]),json!(["config:read"]),json!(["config:read","config:write"])] {
        db.client.execute("UPDATE access_tokens SET scopes=$1 WHERE id='pat'",&[&scopes]).await.unwrap();
        let response=call(&app,"GET","/api/ui/v1/views?path=%2Freverse-proxy%2Fnew",Some(&auth),None,&[]).await;
        assert_eq!(response.status(),if scopes.as_array().unwrap().len()==2 {200} else {403});
    }
    for query in ["path=%2Faccount","path=%2Fadmin%2Fusers","path=%2Ffuture-page","path=%2Freverse-proxy&path=%2Faccount"] {
        let response=call(&app,"GET",&format!("/api/ui/v1/views?{query}"),Some(&auth),None,&[]).await;assert!([400,403].contains(&response.status().as_u16()));
    }
}

#[tokio::test]
#[ignore="real PostgreSQL"]
async fn explicit_admin_business_actions_accept_tokens_and_preserve_actor_and_protections() {
    let db=fixture(&["user:manage"]).await;
    insert_user(&db.client,UserRow {id:"other",email:"other@example.test",role:"editor",..Default::default()},None).await;
    let app=nwui_api::server::router(db.state.clone());let auth=format!("Bearer {}",credential());
    for (operation,form) in [("admin.users.toggle","id=other"),("admin.settings.updateMetrics","metricsRetentionDays=14"),("nodes.refreshStatuses","")] {
        let request=Request::builder().method("POST").uri(format!("/api/ui/v1/forms/{operation}"))
            .header("host","ui.example.test").header("authorization",&auth).header("content-type","application/x-www-form-urlencoded")
            .extension(ConnectInfo(SocketAddr::from(([127,0,0,1],12345)))).body(Body::from(form)).unwrap();
        let response=app.clone().oneshot(request).await.unwrap();assert_eq!(response.status(),200,"{operation}");assert!(!response.headers().contains_key(header::SET_COOKIE));
    }
    assert!(!db.client.query_one("SELECT enabled FROM users WHERE id='other'",&[]).await.unwrap().get::<_,bool>(0));
    assert_eq!(db.client.query_one("SELECT metrics_retention_days FROM app_settings WHERE id='current'",&[]).await.unwrap().get::<_,i32>(0),14);
    for action in ["user:disable","settings:metrics"] {
        let via:Value=db.client.query_one("SELECT via FROM audit_logs WHERE action=$1",&[&action]).await.unwrap().get(0);
        assert_eq!(via,json!({"kind":"token","id":"pat","name":"automation"}));
    }
    f2_refused(&db,&app,"admin.users.toggle",&[("id","u1")], f2_row("user:disable","u1","admin",None,&[],"admin_target")).await;
    assert!(db.client.query_one("SELECT enabled FROM users WHERE id='u1'",&[]).await.unwrap().get::<_,bool>(0));
}

#[tokio::test]
#[ignore="real PostgreSQL"]
async fn relayed_token_stream_closes_on_node_lookup_database_error() {
    use tokio::io::{AsyncReadExt,AsyncWriteExt};
    use http_body_util::BodyExt;
    let db=fixture(&["config:read"]).await;
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
    let version=db.state.config.version.clone();
    let upstream=tokio::spawn(async move {
        let (mut socket,_)=listener.accept().await.unwrap();let mut request=Vec::new();let mut chunk=[0;4096];
        while !request.windows(4).any(|b|b==b"\r\n\r\n") {let n=socket.read(&mut chunk).await.unwrap();assert!(n>0);request.extend_from_slice(&chunk[..n]);assert!(request.len()<16384);}
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nX-Node-Version: {version}\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n:hi\n\n\r\n").as_bytes()).await.unwrap();
        let mut byte=[0];socket.read(&mut byte).await.unwrap()
    });
    let sealed=db.state.secrets.seal("node-stream-fixture").unwrap();
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id,last_version) VALUES('node','Node',$1,$2,'admin','stream-node',$3)",&[&format!("http://{address}"),&sealed,&db.state.config.version]).await.unwrap();
    let app=nwui_api::server::router(db.state.clone());let auth=format!("Bearer {}",credential());
    let response=call(&app,"GET","/api/nodes/node/logs/stream",Some(&auth),None,&[]).await;assert_eq!(response.status(),200);
    let mut stream=response.into_body();
    auth_support::expect_log_source(&mut stream, b":hi\n\n").await;
    // User authentication still succeeds; only the node-authority query now fails.
    db.client.batch_execute("ALTER TABLE nodes RENAME COLUMN token_enc TO unavailable_token_enc").await.unwrap();assert!(user(&db).await.can(Action::ConfigRead));
    tokio::time::timeout(Duration::from_secs(35),async {while stream.frame().await.is_some(){}}).await.expect("node lookup DB failure must close the token stream");
    drop(stream);db.state.runtime.shutdown.send_replace(true);
    assert_eq!(tokio::time::timeout(Duration::from_secs(2),upstream).await.unwrap().unwrap(),0);
}

#[tokio::test]
#[ignore="real PostgreSQL"]
async fn authenticated_policy_refusals_also_consume_the_token_budget() {
    let db=fixture(&["config:read"]).await;let app=nwui_api::server::router(db.state.clone());let auth=format!("Bearer {}",credential());
    for _ in 0..60 {assert_eq!(call(&app,"POST","/api/ui/v1/forms/account.changePassword",Some(&auth),None,&[]).await.status(),403);}
    let refused=call(&app,"POST","/api/ui/v1/forms/account.changePassword",Some(&auth),None,&[]).await;
    assert_eq!(refused.status(),429);assert!(refused.headers().contains_key(header::RETRY_AFTER));
    assert!(db.client.query_one("SELECT last_used_at IS NULL FROM access_tokens WHERE id='pat'",&[]).await.unwrap().get::<_,bool>(0));
}

fn review_cookie() -> String {
    let claims=json!({"id":"u1","sv":3,"mfaVerified":true});
    let value=nwui_api::auth::session::encode(claims.as_object().unwrap(),auth_support::SECRET,nwui_api::auth::session::COOKIE_NAME_PLAIN,principal::now_unix_ms()/1000,3600).unwrap();
    format!("authjs.session-token={value}")
}
async fn review_post(app: &Router, operation: &str, fields: &[(&str,&str)], token: bool) -> Response {
    let encoded=url::form_urlencoded::Serializer::new(String::new()).extend_pairs(fields.iter().copied()).finish();
    let mut request=Request::builder().method("POST").uri(format!("/api/ui/v1/forms/{operation}"))
        .header("host","ui.example.test").header("content-type","application/x-www-form-urlencoded")
        .extension(ConnectInfo(SocketAddr::from(([127,0,0,1],12345))));
    if token {request=request.header("authorization",format!("Bearer {}",credential()));}
    else {request=request.header("cookie",review_cookie()).header("origin","http://ui.example.test");}
    tokio::time::timeout(Duration::from_secs(30), app.clone().oneshot(request.body(Body::from(encoded)).unwrap())).await.expect("user change must finish after releasing row locks").unwrap()
}
async fn review_scopes(db: &TestDb, scopes: &[&str]) {
    db.client.execute("UPDATE access_tokens SET scopes=$1 WHERE id='pat'",&[&json!(scopes)]).await.unwrap();
}
const REVIEW_FULL: &[&str]=&["config:read","config:write","nginx:reload","user:manage"];

async fn f2_audit_rows(db: &TestDb) -> BTreeMap<String, Value> {
    db.client.query("SELECT id,to_jsonb(a) FROM audit_logs a ORDER BY id",&[]).await.unwrap().into_iter().map(|r|(r.get(0),r.get(1))).collect()
}
fn f2_delta(before: &BTreeMap<String,Value>, after: &BTreeMap<String,Value>) -> Vec<(String,Value)> {
    for (id,row) in before { assert_eq!(after.get(id),Some(row),"existing audit row changed"); }
    after.iter().filter(|(id,_)|!before.contains_key(*id)).map(|(id,row)| {
        assert!(uuid::Uuid::parse_str(id).is_ok());
        let mut row=row.clone();assert!(row["created_at"].is_string());
        row.as_object_mut().unwrap().remove("id");row.as_object_mut().unwrap().remove("created_at");
        (id.clone(),row)
    }).collect()
}
fn f2_row(operation: &str, id: &str, current: &str, resulting: Option<&str>, missing: &[&str], reason: &str) -> Value {
    json!({"action":"rbac:denied","target":operation,"actor_id":"u1","actor_label":"u1@example.test","ip":"127.0.0.1","via":{"kind":"token","id":"pat","name":"automation"},"before":{"operation":operation,"userId":id,"role":"admin","targetRole":current,"resultingRole":resulting,"missingActions":missing,"reason":reason},"after":null})
}
async fn f2_forbidden(response: Response) {
    assert_eq!(response.status(),403);assert!(!response.headers().contains_key(header::SET_COOKIE));
    assert_eq!(body(response).await,json!({"error":{"code":"forbidden","message":"이 동작을 수행할 권한이 없습니다."}}));
}
async fn f2_refused(db: &TestDb, app: &Router, op: &str, fields: &[(&str,&str)], expected: Value) -> String {
    let before=f2_audit_rows(db).await;let mut data=database_snapshot(db).await;data.remove("audit_logs");
    f2_forbidden(review_post(app,op,fields,true).await).await;
    let delta=f2_delta(&before,&f2_audit_rows(db).await);assert_eq!(delta.len(),1,"exactly one new audit row across all actions");assert_eq!(delta[0].1,expected);
    let mut after=database_snapshot(db).await;after.remove("audit_logs");assert_eq!(after,data,"all business tables unchanged");
    tokio::time::timeout(Duration::from_secs(10),db.client.execute("UPDATE users SET email=email WHERE id=$1 OR id='u1'",&[&expected["before"]["userId"].as_str().unwrap()])).await.expect("target and actor locks released").unwrap();
    delta[0].0.clone()
}
async fn f2_allowed(db: &TestDb, app: &Router, op: &str, fields: &[(&str,&str)], token: bool) -> Response {
    let before=f2_audit_rows(db).await;let response=review_post(app,op,fields,token).await;
    let delta=f2_delta(&before,&f2_audit_rows(db).await);
    assert!(delta.iter().all(|(_,row)|row["action"]!="rbac:denied"));
    if response.status()==200 { assert!(delta.iter().any(|(_,row)|match op {
        "admin.users.updateRole"=>row["action"]=="user:update", "admin.users.toggle"=>row["action"]=="user:enable"||row["action"]=="user:disable", "admin.pending.approve"=>row["action"]=="user:approve", "admin.pending.reject"=>row["action"]=="user:reject", _=>false
    }),"allowed change retains success audit"); } response
}

#[tokio::test]
#[ignore="real PostgreSQL"]
async fn review_f1_proxy_authorization_keeps_cookie_and_refresh() {
    let db=fixture(&["config:read"]).await;let app=nwui_api::server::router(db.state.clone());let cookie=review_cookie();
    for auth in ["Basic b3V0ZXI6cHJveHk=","Bearer outer-sso","Bearer nwu_s_outer"] {
        assert_eq!(call(&app,"GET","/api/ui/v1/account",Some(auth),Some(&cookie),&[]).await.status(),200,"{auth}");
        let refreshed=call(&app,"POST","/api/ui/v1/session/refresh",Some(auth),Some(&cookie),&[("origin","http://ui.example.test")]).await;
        assert_eq!(refreshed.status(),200);assert!(refreshed.headers().contains_key(header::SET_COOKIE));
    }
}
#[tokio::test]
#[ignore="real PostgreSQL"]
async fn review_f1_outer_headers_do_not_lock_password_signin() {
    let db=fixture(&["config:read"]).await;let app=nwui_api::server::router(db.state.clone());let cookie=review_cookie();
    let password="Review-passw0rd!";
    insert_user(&db.client,UserRow {id:"signin",email:"signin@example.test",..Default::default()},Some(&nwui_api::auth::password::hash_password(password).unwrap())).await;
    for _ in 0..31 {
        for auth in ["Basic b3V0ZXI6cHJveHk=","Bearer outer-sso","Bearer nwu_s_outer"] {
            let _=call(&app,"GET","/api/ui/v1/account",Some(auth),Some(&cookie),&[]).await;
        }
    }
    let now=principal::now_unix_ms();
    assert!(!db.state.runtime.login_throttle.is_ip_locked("127.0.0.1",now));
    // One real bad user token must be the 30th failure, proving the outer headers added zero.
    for _ in 0..29 {db.state.runtime.login_throttle.record_ip_failure("198.51.100.22",now);}
    let request=Request::builder().uri("/api/ui/v1/account").header("host","ui.example.test").header("authorization","bEaReR nwu_u_bad")
        .header("x-real-ip","198.51.100.22").extension(ConnectInfo(SocketAddr::from(([127,0,0,1],12345)))).body(Body::empty()).unwrap();
    assert_eq!(app.clone().oneshot(request).await.unwrap().status(),401);
    assert!(db.state.runtime.login_throttle.is_ip_locked("198.51.100.22",principal::now_unix_ms()));
    let login=url::form_urlencoded::Serializer::new(String::new()).append_pair("email","signin@example.test").append_pair("password",password).finish();
    let request=Request::builder().method("POST").uri("/api/ui/v1/forms/login.loginAction").header("host","ui.example.test")
        .header("origin","http://ui.example.test").header("authorization","Basic b3V0ZXI6cHJveHk=").header("content-type","application/x-www-form-urlencoded")
        .extension(ConnectInfo(SocketAddr::from(([127,0,0,1],12345)))).body(Body::from(login)).unwrap();
    let response=app.oneshot(request).await.unwrap();assert_eq!(response.status(),200);assert!(response.headers().contains_key(header::SET_COOKIE));
    assert_eq!(body(response).await["redirectTo"],"/");
}
#[tokio::test]
#[ignore="real PostgreSQL"]
async fn review_f2_role_updates_stay_within_token_authority() {
    let db=fixture(&["config:read","user:manage"]).await;let app=nwui_api::server::router(db.state.clone());
    insert_user(&db.client,UserRow {id:"target",email:"target@example.test",role:"viewer",..Default::default()},None).await;
    f2_refused(&db,&app,"admin.users.updateRole",&[("id","target"),("role","editor")],f2_row("user:update","target","viewer",Some("editor"),&["config:write","nginx:reload"],"exceeds_token")).await;
    assert_eq!(db.client.query_one("SELECT role::text FROM users WHERE id='target'",&[]).await.unwrap().get::<_,String>(0),"viewer");
    review_scopes(&db,REVIEW_FULL).await;
    assert_eq!(f2_allowed(&db,&app,"admin.users.updateRole",&[("id","target"),("role","editor")],true).await.status(),200);
    f2_refused(&db,&app,"admin.users.updateRole",&[("id","target"),("role","admin")],f2_row("user:update","target","editor",Some("admin"),&[],"admin_grant")).await;
    assert_eq!(f2_allowed(&db,&app,"admin.users.updateRole",&[("id","target"),("role","admin")],false).await.status(),200);
    f2_refused(&db,&app,"admin.users.updateRole",&[("id","target"),("role","viewer")],f2_row("user:update","target","admin",Some("viewer"),&[],"admin_target")).await;
}
#[tokio::test]
#[ignore="real PostgreSQL"]
async fn review_f2_approval_and_enable_cannot_mint_a_stronger_session() {
    let db=fixture(&["config:read","user:manage"]).await;let app=nwui_api::server::router(db.state.clone());
    for (id,approved) in [("pending",false),("disabled",true)] {
        insert_user(&db.client,UserRow {id,email:if approved {"disabled@example.test"}else{"pending@example.test"},role:"editor",approved,enabled:false,..Default::default()},None).await;
    }
    f2_refused(&db,&app,"admin.pending.approve",&[("id","pending"),("role","editor")],f2_row("user:approve","pending","editor",Some("editor"),&["config:write","nginx:reload"],"exceeds_token")).await;
    f2_refused(&db,&app,"admin.users.toggle",&[("id","disabled")],f2_row("user:enable","disabled","editor",Some("editor"),&["config:write","nginx:reload"],"exceeds_token")).await;
    review_scopes(&db,REVIEW_FULL).await;
    f2_refused(&db,&app,"admin.pending.approve",&[("id","pending"),("role","admin")],f2_row("user:approve","pending","editor",Some("admin"),&[],"admin_grant")).await;
    assert_eq!(f2_allowed(&db,&app,"admin.pending.approve",&[("id","pending"),("role","editor")],true).await.status(),200);
    assert_eq!(f2_allowed(&db,&app,"admin.users.toggle",&[("id","disabled")],true).await.status(),200);
    // A narrowed token may still disable or reject a non-admin target.
    review_scopes(&db,&["config:read","user:manage"]).await;
    assert_eq!(f2_allowed(&db,&app,"admin.users.toggle",&[("id","disabled")],true).await.status(),200);
    db.client.execute("UPDATE users SET approved=false,enabled=true WHERE id='pending'",&[]).await.unwrap();
    assert_eq!(f2_allowed(&db,&app,"admin.pending.reject",&[("id","pending")],true).await.status(),200);
    assert!(!db.client.query_one("SELECT enabled FROM users WHERE id='pending'",&[]).await.unwrap().get::<_,bool>(0));
    // The cookie path retains the original authority to approve as admin.
    assert_eq!(f2_allowed(&db,&app,"admin.pending.approve",&[("id","pending"),("role","admin")],false).await.status(),200);
}
#[tokio::test]
#[ignore="real PostgreSQL"]
async fn review_f2_admin_targets_are_untouchable_by_tokens() {
    let db=fixture(REVIEW_FULL).await;let app=nwui_api::server::router(db.state.clone());
    for (id,approved,enabled) in [("active-admin",true,true),("disabled-admin",true,false),("pending-admin",false,true)] {
        insert_user(&db.client,UserRow {id,email:if id=="active-admin" {"active@example.test"}else if approved {"disabled@example.test"}else{"pending@example.test"},role:"admin",approved,enabled,..Default::default()},None).await;
    }
    let mut before=database_snapshot(&db).await;before.remove("audit_logs");
    for (operation,fields,action,id,resulting) in [
        ("admin.users.updateRole",vec![("id","active-admin"),("role","viewer")],"user:update","active-admin",Some("viewer")),
        ("admin.users.toggle",vec![("id","active-admin")],"user:disable","active-admin",None),
        ("admin.users.toggle",vec![("id","disabled-admin")],"user:enable","disabled-admin",Some("admin")),
        ("admin.pending.approve",vec![("id","pending-admin"),("role","viewer")],"user:approve","pending-admin",Some("viewer")),
        ("admin.pending.reject",vec![("id","pending-admin")],"user:reject","pending-admin",None),
    ] {f2_refused(&db,&app,operation,&fields,f2_row(action,id,"admin",resulting,&[],"admin_target")).await;}
    let mut after=database_snapshot(&db).await;after.remove("audit_logs");assert_eq!(after,before);
    assert_eq!(f2_allowed(&db,&app,"admin.users.toggle",&[("id","disabled-admin")],false).await.status(),200);
}
#[tokio::test]
#[ignore="real PostgreSQL"]
async fn review_f3_local_replace_needs_write_and_reload() {
    let db=fixture(&["nginx:reload"]).await;let app=nwui_api::server::router(db.state.clone());
    let initial=rcgen::generate_simple_self_signed(vec!["initial.example.test".into()]).unwrap();
    let replacement=rcgen::generate_simple_self_signed(vec!["replacement.example.test".into()]).unwrap();
    let key=db.state.secrets.seal(&initial.signing_key.serialize_pem()).unwrap();
    db.client.execute("INSERT INTO certificates(id,domains,status,source,cert_pem,key_pem) VALUES('c',$1,'issued','manual',$2,$3)",&[&json!(["initial.example.test"]),&initial.cert.pem(),&key]).await.unwrap();
    let pem=replacement.cert.pem();let key_pem=replacement.signing_key.serialize_pem();let fields=[("id","c"),("certPem",pem.as_str()),("keyPem",key_pem.as_str())];
    assert_eq!(review_post(&app,"certificates.replaceCertificate",&fields,true).await.status(),403);
    assert_eq!(db.client.query_one("SELECT cert_pem FROM certificates WHERE id='c'",&[]).await.unwrap().get::<_,String>(0),initial.cert.pem());
    review_scopes(&db,&["config:write","nginx:reload"]).await;
    assert_eq!(review_post(&app,"certificates.replaceCertificate",&fields,true).await.status(),200);
    assert_eq!(db.client.query_one("SELECT cert_pem FROM certificates WHERE id='c'",&[]).await.unwrap().get::<_,String>(0),pem.trim());
    let actor=user(&db).await;
    for id in ["form_certificates_renewCertificate","form_certificates_renewDnsCertificate"] {
        let policy=route_policy::ROUTES.iter().find(|r|r.operation_id==id).unwrap();assert_eq!(policy.actions,&[Action::NginxReload]);assert!(policy.actions.iter().all(|a|actor.can(*a)));
    }
}
#[tokio::test]
#[ignore="real PostgreSQL"]
async fn review_f3_remote_replace_needs_write_and_reload() {
    use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
    let db=fixture(&["nginx:reload"]).await;let calls=Arc::new(AtomicUsize::new(0));let observed=calls.clone();let version=db.state.config.version.clone();
    let remote=Router::new().route("/api/v1/certificates/c/replace",axum::routing::post(move || {let calls=observed.clone();let version=version.clone();async move {calls.fetch_add(1,Ordering::SeqCst);([("x-node-version",version)],axum::Json(json!({"id":"c","uncovered":[],"applyError":null})))}}));
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();let server=tokio::spawn(async move {axum::serve(listener,remote).await.unwrap()});
    let key=db.state.secrets.seal("remote-review-fixture").unwrap();
    db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id,last_version) VALUES('node','Node',$1,$2,'admin','review-node',$3)",&[&format!("http://{address}"),&key,&db.state.config.version]).await.unwrap();
    let app=nwui_api::server::router(db.state.clone());let fields=[("_apiArgs","[\"node\"]"),("id","c"),("certPem","fixture"),("keyPem","fixture")];
    assert_eq!(review_post(&app,"node.replaceNodeCertificate",&fields,true).await.status(),403);assert_eq!(calls.load(Ordering::SeqCst),0);
    review_scopes(&db,&["config:write","nginx:reload"]).await;
    let response=review_post(&app,"node.replaceNodeCertificate",&fields,true).await;assert_eq!(response.status(),200);assert_eq!(calls.load(Ordering::SeqCst),1);
    server.abort();
}
#[tokio::test]
#[ignore="real PostgreSQL"]
async fn review_n3_secret_sending_tests_require_a_session() {
    let db=fixture(REVIEW_FULL).await;let app=nwui_api::server::router(db.state.clone());
    let mut responses=Vec::new();
    for operation in ["admin.smtp.test","admin.webhooks.test"] {
        let response=review_post(&app,operation,&[],true).await;
        responses.push((response.status().as_u16(),body(response).await));
    }
    assert_eq!(responses.iter().map(|r|r.0).collect::<Vec<_>>(),vec![403,403]);
    for (_,value) in responses {assert_eq!(value["error"]["code"],"session_required_for_action");}
}

#[tokio::test]
#[ignore="real PostgreSQL"]
async fn review_f1_proxy_authorization_preserves_recent_self_reset() {
    for auth in ["Basic b3V0ZXI6cHJveHk=","Bearer outer-sso","Bearer nwu_s_outer"] {
        let db=fixture(&["config:read"]).await;
        db.client.execute("UPDATE users SET password_hash=$1 WHERE id='u1'",&[&nwui_api::auth::password::hash_password("Old-Passw0rd!").unwrap()]).await.unwrap();
        let now=principal::now_unix_ms()/1000;
        let claims=json!({"id":"u1","sv":3,"mfaVerified":true,"authTime":now-30});
        let cookie=nwui_api::auth::session::encode(claims.as_object().unwrap(),auth_support::SECRET,nwui_api::auth::session::COOKIE_NAME_PLAIN,now,3600).unwrap();
        let request=Request::builder().method("POST").uri("/api/ui/v1/forms/admin.users.resetPassword")
            .header("host","ui.example.test").header("origin","http://ui.example.test")
            .header("authorization",auth).header("cookie",format!("authjs.session-token={cookie}"))
            .header("content-type","application/x-www-form-urlencoded")
            .extension(ConnectInfo(SocketAddr::from(([127,0,0,1],12345))))
            .body(Body::from("id=u1&password=New-Passw0rd%21&confirmPassword=New-Passw0rd%21")).unwrap();
        let response=nwui_api::server::router(db.state.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(),200,"{auth}: {}",body(response).await);
        assert!(nwui_api::auth::password::verify_password("New-Passw0rd!", &db.client.query_one("SELECT password_hash FROM users WHERE id='u1'",&[]).await.unwrap().get::<_,String>(0)));
    }
}

#[tokio::test]
#[ignore="real PostgreSQL"]
async fn review_f2_pool_one_persists_all_callers_and_audit_projection() {
    let db=fixture_with(REVIEW_FULL,&[("API_DB_POOL_SIZE","1")]).await;
    insert_user(&db.client,UserRow {id:"target",email:"target@example.test",role:"admin",approved:false,..Default::default()},None).await;
    let app=nwui_api::server::router(db.state.clone());
    for (op,fields,action,resulting) in [
        ("admin.users.updateRole",vec![("id","target"),("role","viewer")],"user:update",Some("viewer")),
        ("admin.users.toggle",vec![("id","target")],"user:disable",None),
        ("admin.pending.approve",vec![("id","target"),("role","viewer")],"user:approve",Some("viewer")),
        ("admin.pending.reject",vec![("id","target")],"user:reject",None),
    ] {
        let expected=f2_row(action,"target","admin",resulting,&[],"admin_target");
        let id=f2_refused(&db,&app,op,&fields,expected.clone()).await;
        let list=nwui_api::observability::query::audit_list(&db.state,&BTreeMap::from([("action".into(),"rbac:denied".into())])).await.unwrap();
        let item=list["items"].as_array().unwrap().iter().find(|r|r["id"]==id).unwrap();
        assert_eq!(item["target"],action);assert_eq!(item["via"],expected["via"]);assert_eq!(item["actorLabel"],"u1@example.test");assert!(item.get("before").is_none());
        let detail=nwui_api::observability::query::audit_detail(&db.state,&id).await.unwrap();
        assert_eq!(detail["log"]["target"],action);assert_eq!(detail["log"]["before"],expected["before"]);assert_eq!(detail["log"]["after"],Value::Null);assert_eq!(detail["log"]["via"],expected["via"]);
    }
    db.client.execute("UPDATE users SET enabled=false WHERE id='target'",&[]).await.unwrap();
    f2_refused(&db,&app,"admin.users.toggle",&[("id","target")],f2_row("user:enable","target","admin",Some("admin"),&[],"admin_target")).await;
}

#[tokio::test]
#[ignore="real PostgreSQL"]
async fn review_f2_audit_insert_failure_keeps_forbidden_and_all_tables_unchanged() {
    for extra in [vec![],vec![("API_DB_POOL_SIZE","1")]] {
        let db=fixture_with(REVIEW_FULL,&extra).await;
        insert_user(&db.client,UserRow {id:"target",email:"target@example.test",role:"admin",approved:false,..Default::default()},None).await;
        db.client.batch_execute("ALTER TABLE audit_logs ADD CONSTRAINT test_reject_denial CHECK(action <> 'rbac:denied')").await.unwrap();
        let app=nwui_api::server::router(db.state.clone());
        for (op,fields) in [
            ("admin.users.updateRole",vec![("id","target"),("role","viewer")]),("admin.users.toggle",vec![("id","target")]),
            ("admin.pending.approve",vec![("id","target"),("role","viewer")]),("admin.pending.reject",vec![("id","target")]),
        ] {
            let before=database_snapshot(&db).await;
            f2_forbidden(review_post(&app,op,&fields,true).await).await;
            assert_eq!(database_snapshot(&db).await,before,"audit failure must not change any table: {op}");
            tokio::time::timeout(Duration::from_secs(10),db.client.execute("UPDATE users SET email=email WHERE id IN ('u1','target')",&[])).await.expect("locks released even after audit failure").unwrap();
            assert_eq!(db.state.db.get().await.unwrap().query_one("SELECT 1",&[]).await.unwrap().get::<_,i32>(0),1,"same pooled connection remains usable");
        }
    }
}

#[tokio::test]
#[ignore="real PostgreSQL"]
async fn review_f2_general_denial_is_not_duplicated() {
    let db=fixture(&["config:read"]).await;
    insert_user(&db.client,UserRow {id:"target",email:"target@example.test",..Default::default()},None).await;
    let app=nwui_api::server::router(db.state.clone());let before=f2_audit_rows(&db).await;
    let response=review_post(&app,"admin.users.updateRole",&[("id","target"),("role","viewer")],true).await;
    assert_eq!(response.status(),403);assert!(!response.headers().contains_key(header::SET_COOKIE));assert_eq!(body(response).await["error"]["code"],"insufficient_scope");
    let delta=f2_delta(&before,&f2_audit_rows(&db).await);
    let mut expected=f2_row("user:manage","target","editor",Some("viewer"),&[],"exceeds_token");expected["before"]=json!({"role":"admin","requiredAction":"user:manage"});
    assert_eq!(delta.len(),1);assert_eq!(delta[0].1,expected);
}

#[tokio::test]
#[ignore="real PostgreSQL"]
async fn review_f2_guard_order_and_early_refusals_stay_unchanged() {
    let db=fixture(&["user:manage"]).await;let app=nwui_api::server::router(db.state.clone());
    insert_user(&db.client,UserRow {id:"target",email:"target@example.test",role:"admin",..Default::default()},None).await;
    // Admin-target reason wins even though viewer would require the missing config:read.
    f2_refused(&db,&app,"admin.users.updateRole",&[("id","target"),("role","viewer")],f2_row("user:update","target","admin",Some("viewer"),&["config:read"],"admin_target")).await;
    // The F2 guard still precedes approved/no-op checks and self-disable.
    f2_refused(&db,&app,"admin.pending.approve",&[("id","target"),("role","viewer")],f2_row("user:approve","target","admin",Some("viewer"),&["config:read"],"admin_target")).await;
    f2_refused(&db,&app,"admin.pending.reject",&[("id","target")],f2_row("user:reject","target","admin",None,&[],"admin_target")).await;
    f2_refused(&db,&app,"admin.users.toggle",&[("id","u1")],f2_row("user:disable","u1","admin",None,&[],"admin_target")).await;
    for (op,fields,status) in [
        ("admin.users.updateRole",vec![("id","target"),("role","invalid")],400),
        ("admin.users.updateRole",vec![("id","u1"),("role","viewer")],409),
        ("admin.users.updateRole",vec![("id","gone"),("role","viewer")],404),
        ("admin.pending.approve",vec![("id","gone"),("role","viewer")],200),
        ("admin.pending.reject",vec![("id","gone")],200),
    ] {
        let before=f2_audit_rows(&db).await;assert_eq!(review_post(&app,op,&fields,true).await.status(),status,"{op} {fields:?}");assert!(f2_delta(&before,&f2_audit_rows(&db).await).is_empty());
    }
}
