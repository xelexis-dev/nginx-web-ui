//! HTTP tests for the SMTP settings screen and the mail-recipient test button, on a real
//! PostgreSQL database with a local fake SMTP server.
//!
//!     NWUI_TEST_DATABASE_URL=postgres://user:pass@host:5432/postgres cargo test --test admin_smtp -- --include-ignored

mod outbound_support;
mod auth_support;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use auth_support::{TestDb, UserRow, audit_rows, insert_user, test_db};
use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use nwui_api::auth::session;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tower::ServiceExt as _;

const FORMS: &str = "/api/ui/v1/forms";

fn app(db: &TestDb) -> Router {
    nwui_api::app::with_request_context(nwui_api::admin::router().with_state(db.state.clone()), None)
}

async fn boss(db: &TestDb) -> String {
    insert_user(&db.client, UserRow { id: "boss", email: "boss@example.test", role: "admin", totp_enabled: true, ..Default::default() }, Some("x")).await;
    let now = nwui_api::auth::principal::now_unix_ms() / 1000;
    let claims = json!({"sub": "boss", "id": "boss", "sv": 3, "requires2fa": true, "mfaVerified": true});
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
        builder = builder.header("origin", "http://admin.example.test").header("content-type", "application/x-www-form-urlencoded");
    }
    let response = app.clone().oneshot(builder.body(Body::from(form.unwrap_or("").to_owned())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() })
}

/// A plain-text SMTP server that accepts everything and records the lines it saw.
async fn fake_smtp() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let records = seen.clone();
    tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let records = records.clone();
            tokio::spawn(async move {
                let (read, mut write) = socket.into_split();
                let mut lines = BufReader::new(read).lines();
                write.write_all(b"220 local.test ESMTP\r\n").await.unwrap();
                let mut data = false;
                while let Ok(Some(line)) = lines.next_line().await {
                    records.lock().unwrap().push(line.clone());
                    let reply = if data {
                        if line == "." {
                            data = false;
                            Some("250 accepted\r\n")
                        } else {
                            None
                        }
                    } else if line.starts_with("EHLO") {
                        Some("250-local.test\r\n250 8BITMIME\r\n")
                    } else if line == "DATA" {
                        data = true;
                        Some("354 continue\r\n")
                    } else if line == "QUIT" {
                        Some("221 bye\r\n")
                    } else {
                        Some("250 OK\r\n")
                    };
                    if let Some(reply) = reply
                        && write.write_all(reply.as_bytes()).await.is_err()
                    {
                        break;
                    }
                    if line == "QUIT" {
                        break;
                    }
                }
            });
        }
    });
    (port, seen)
}

fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind((outbound_support::peer_ip(),0)).unwrap();
    listener.local_addr().unwrap().port()
}

fn smtp_form(port: u16, extra: &str) -> String {
    format!("host={}&port={port}&fromEmail=from%40example.test&fromName=%EC%95%8C%EB%A6%BC{extra}",outbound_support::peer_host())
}

async fn stored(db: &TestDb) -> tokio_postgres::Row {
    db.client.query_one("SELECT enabled, host, port, username, password_enc, from_name, reject_unauthorized FROM smtp_settings WHERE id = 'current'", &[]).await.unwrap()
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn the_view_never_carries_the_password_and_says_whether_mail_can_go() {
    let db = test_db().await;
    let app = app(&db);
    let boss = boss(&db).await;
    let view = "/api/ui/v1/admin/settings/smtp";

    let (status, body) = call(&app, "GET", view, &boss, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({
            "config": {"enabled": false, "host": "", "port": 587, "secure": false, "username": "", "fromEmail": "", "fromName": "", "rejectUnauthorized": true},
            "hasStoredPassword": false,
            "readiness": {"ok": false, "reason": "not-configured", "message": "SMTP가 설정되지 않았습니다. 관리자 패널 → 사이트 설정 → SMTP 설정에서 설정하세요."},
        })
    );
    let sealed = db.state.secrets.seal("SECRET-PW").unwrap();
    db.client
        .execute("INSERT INTO smtp_settings (id, enabled, host, port, username, password_enc, from_email) VALUES ('current', false, 'mail.test', 25, 'u', $1, 'f@example.test')", &[&sealed])
        .await
        .unwrap();
    let (_, body) = call(&app, "GET", view, &boss, None).await;
    assert_eq!((&body["hasStoredPassword"], &body["readiness"]["reason"]), (&json!(true), &json!("disabled")));
    assert!(!body.to_string().contains("SECRET") && !body.to_string().contains("enc:v1"));
    db.client.execute("UPDATE smtp_settings SET enabled = true", &[]).await.unwrap();
    assert_eq!(call(&app, "GET", view, &boss, None).await.1["readiness"], json!({"ok": true}));
    let foreign = nwui_api::crypto::SecretBox::from_key([5u8; 32]).seal("x").unwrap();
    db.client.execute("UPDATE smtp_settings SET password_enc = $1", &[&foreign]).await.unwrap();
    assert_eq!(call(&app, "GET", view, &boss, None).await.1["readiness"]["reason"], "password-unreadable");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn saving_validates_verifies_enabled_settings_and_seals_the_password() {
    let db = test_db().await;
    let app = app(&db);
    let boss = boss(&db).await;
    let update = format!("{FORMS}/admin.smtp.update");

    let (status, body) = call(&app, "POST", &update, &boss, Some("host=+&port=0x19&fromEmail=nope")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["fieldErrors"], json!({"host": "SMTP 호스트를 입력하세요.", "port": "1-65535 사이의 포트를 입력하세요.", "fromEmail": "발신 주소 형식이 올바르지 않습니다."}));
    let (_, body) = call(&app, "POST", &update, &boss, Some("host=h&port=70000&fromEmail=")).await;
    assert_eq!((body["error"]["fieldErrors"]["port"].as_str(), body["error"]["fieldErrors"]["fromEmail"].as_str()), (Some("1-65535 사이의 포트를 입력하세요."), Some("발신 주소를 입력하세요.")));

    // Disabled settings are kept without a connection check.
    let dead = closed_port();
    let (status, body) = call(&app, "POST", &update, &boss, Some(&smtp_form(dead, "&username=u&password=first"))).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"result": {"ok": true, "message": "저장했습니다."}, "refresh": true})));
    let row = stored(&db).await;
    let sealed: Option<String> = row.get("password_enc");
    assert_eq!(db.state.secrets.open(sealed.as_deref()).as_deref(), Some("first"));
    assert_eq!((row.get::<_, bool>("enabled"), row.get::<_, Option<String>>("from_name").as_deref(), row.get::<_, bool>("reject_unauthorized")), (false, Some("알림"), true));
    let audit = &audit_rows(&db.client, "settings:smtp").await[0];
    assert_eq!(audit.get::<_, Value>("after"), json!({"enabled": false, "host": outbound_support::peer_host(), "port": dead, "fromEmail": "from@example.test", "passwordChanged": true}));
    assert!(!audit.get::<_, Value>("after").to_string().contains("first"));

    // Enabled settings must connect first; a failure saves nothing.
    let (status, body) = call(&app, "POST", &update, &boss, Some(&smtp_form(dead, "&enabled=true&password=__keep__"))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"]["message"].as_str().unwrap().starts_with(&format!("{}:{dead} 연결·인증에 실패해 저장하지 않았습니다 — ",outbound_support::peer_host())), "{body}");
    assert!(!stored(&db).await.get::<_, bool>("enabled"));

    let (port, _) = fake_smtp().await;
    // Keeping the password for another server would hand it to that server: re-enter it.
    let (status, body) = call(&app, "POST", &update, &boss, Some(&smtp_form(port, "&enabled=true&password=__keep__").replace(&outbound_support::peer_host(), "localhost"))).await;
    assert_eq!((status, body["error"]["fieldErrors"]["password"].as_str()), (StatusCode::BAD_REQUEST, Some("다른 서버를 쓰려면 비밀번호를 다시 입력하세요. 저장된 비밀번호는 저장된 서버로만 보냅니다.")));
    // Nor may disabled settings be parked for another server with the stored password.
    let (status, body) = call(&app, "POST", &update, &boss, Some(&smtp_form(port, "&password=__keep__").replace(&outbound_support::peer_host(), "localhost"))).await;
    assert_eq!((status, body["error"]["fieldErrors"]["password"].as_str()), (StatusCode::BAD_REQUEST, Some("다른 서버를 쓰려면 비밀번호를 다시 입력하세요. 저장된 비밀번호는 저장된 서버로만 보냅니다.")));
    assert_eq!(stored(&db).await.get::<_, String>("host"), outbound_support::peer_host());
    // Same host, no username: verified against the fake server, then saved; the password stays.
    let (status, _) = call(&app, "POST", &update, &boss, Some(&smtp_form(port, "&enabled=true&password=__keep__&rejectUnauthorized=false"))).await;
    assert_eq!(status, StatusCode::OK);
    let row = stored(&db).await;
    assert_eq!((row.get::<_, bool>("enabled"), row.get::<_, i32>("port"), row.get::<_, Option<String>>("password_enc")), (true, i32::from(port), sealed.clone()));
    assert!(!row.get::<_, bool>("reject_unauthorized"));
    assert_eq!(audit_rows(&db.client, "settings:smtp").await.len(), 2);
    // An absent password field keeps it too; an empty one removes it.
    call(&app, "POST", &update, &boss, Some(&smtp_form(port, ""))).await;
    assert_eq!(stored(&db).await.get::<_, Option<String>>("password_enc"), sealed);
    call(&app, "POST", &update, &boss, Some(&smtp_form(port, "&password="))).await;
    assert_eq!(stored(&db).await.get::<_, Option<String>>("password_enc"), None);
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn the_test_button_checks_the_connection_and_sends_when_asked() {
    let db = auth_support::test_db_with(&[("API_DB_POOL_SIZE", "1")]).await;
    let app = app(&db);
    let boss = boss(&db).await;
    let test = format!("{FORMS}/admin.smtp.test");

    let (status, body) = call(&app, "POST", &test, &boss, Some("host=&fromEmail=")).await;
    assert_eq!((status, body["error"]["message"].as_str()), (StatusCode::BAD_REQUEST, Some("SMTP가 설정되어 있지 않습니다. SMTP 호스트와 발신 주소를 먼저 입력한 뒤 다시 시도하세요.")));
    assert!(audit_rows(&db.client, "settings:smtp-test").await.is_empty());
    let dead = closed_port();
    let (status, body) = call(&app, "POST", &test, &boss, Some(&smtp_form(dead, "&username=test-user&password=synthetic-smtp-password"))).await;
    assert_eq!((status, body["result"]["ok"].as_bool()), (StatusCode::OK, Some(false)));
    assert!(body["result"]["error"].as_str().unwrap().starts_with(&format!("{}:{dead} 에 연결하지 못했습니다 — ",outbound_support::peer_host())));

    let rows = audit_rows(&db.client, "settings:smtp-test").await;
    assert_eq!(rows.len(), 1, "a failed SMTP connection attempt must be audited");
    assert_eq!(rows[0].get::<_, Option<String>>("actor_id").as_deref(), Some("boss"));
    assert_eq!(rows[0].get::<_, Option<String>>("ip").as_deref(), Some("127.0.0.1"));
    let after: Value = rows[0].get("after");
    assert_eq!(after["host"], outbound_support::peer_host());
    assert_eq!(after["port"], dead);
    assert!(!after.to_string().contains("synthetic-smtp-password"));
    assert!(after.get("password").is_none());
    let (port, seen) = fake_smtp().await;
    let (status, body) = call(&app, "POST", &test, &boss, Some(&smtp_form(port, "&rejectUnauthorized=false"))).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"result": {"ok": true, "message": "연결·인증에 성공했습니다. (수신 주소를 넣으면 실제 발송까지 시험합니다)"}})));
    let (_, body) = call(&app, "POST", &test, &boss, Some(&smtp_form(port, "&testTo=ops%40example.test"))).await;
    assert_eq!(body, json!({"result": {"ok": true, "message": "ops@example.test 로 테스트 메일을 보냈습니다."}}));
    let lines = seen.lock().unwrap().join("\n");
    // The Korean subject travels RFC 2047-encoded; the recipient and a subject line are what show.
    assert!(lines.contains("RCPT TO:<ops@example.test>") && lines.contains("Subject: "), "{lines}");
    assert!(db.client.query_opt("SELECT 1 FROM smtp_settings", &[]).await.unwrap().is_none(), "testing saves no SMTP settings");
    let rows = audit_rows(&db.client, "settings:smtp-test").await;
    assert_eq!(rows.len(), 3, "verify and send each audit exactly one attempt");
    assert!(rows.iter().any(|r| r.get::<_, Value>("after")["to"] == "ops@example.test"));
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_mail_recipient_test_sends_the_sample_through_the_saved_settings() {
    let db = auth_support::test_db_with(&[("API_DB_POOL_SIZE", "1")]).await;
    let app = app(&db);
    let boss = boss(&db).await;
    let test = format!("{FORMS}/admin.mailRecipients.test");

    let (status, body) = call(&app, "POST", &test, &boss, Some("address=nope")).await;
    assert_eq!((status, body["error"]["fieldErrors"]["address"].as_str()), (StatusCode::BAD_REQUEST, Some("먼저 올바른 수신 주소를 입력한 뒤 다시 시도하세요.")));
    let (status, body) = call(&app, "POST", &test, &boss, Some("address=ops%40example.test")).await;
    assert_eq!((status, body["error"]["message"].as_str()), (StatusCode::CONFLICT, Some("메일을 보낼 수 없습니다 — 관리자 패널 → 사이트 설정 → SMTP 설정을 먼저 마치세요.")));

    assert!(audit_rows(&db.client, "settings:mail-recipient-test").await.is_empty());
    let (port, seen) = fake_smtp().await;
    db.client
        .execute("INSERT INTO smtp_settings (id, enabled, host, port, from_email, reject_unauthorized) VALUES ('current', true, $2, $1, 'from@example.test', false)", &[&i32::from(port),&outbound_support::peer_host()])
        .await
        .unwrap();
    db.client.execute("INSERT INTO mail_recipients (id, address, events) VALUES ('r', 'ops@example.test', '[\"cert.issued\"]')", &[]).await.unwrap();
    let (status, body) = call(&app, "POST", &test, &boss, Some("address=+ops%40example.test+")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"result": {"ok": true, "message": "ops@example.test 로 테스트 메일을 보냈습니다."}})));
    assert!(seen.lock().unwrap().iter().any(|l| l == "RCPT TO:<ops@example.test>"));
    let untouched = db.client.query_one("SELECT last_attempt_at IS NULL FROM mail_recipients WHERE id = 'r'", &[]).await.unwrap();
    assert!(untouched.get::<_, bool>(0), "a test is not the recipient's last attempt");
    let rows = audit_rows(&db.client, "settings:mail-recipient-test").await;
    assert_eq!(rows.len(), 1, "the sample send must be audited");
    assert_eq!(rows[0].get::<_, Option<String>>("actor_id").as_deref(), Some("boss"));
    assert_eq!(rows[0].get::<_, Option<String>>("ip").as_deref(), Some("127.0.0.1"));
    assert_eq!(rows[0].get::<_, Value>("after")["address"], "ops@example.test");
    db.client.execute("UPDATE smtp_settings SET port=$1", &[&i32::from(closed_port())]).await.unwrap();
    let (status, body) = call(&app, "POST", &test, &boss, Some("address=ops%40example.test")).await;
    assert_eq!((status, body["result"]["ok"].as_bool()), (StatusCode::OK, Some(false)));
    assert_eq!(audit_rows(&db.client, "settings:mail-recipient-test").await.len(), 2, "failed sends must also be audited");
}

async fn keep_save_race(initially_stored: bool, next_host: &str) {
    use std::time::Duration;
    let db = test_db().await;
    let app = app(&db);
    let boss = boss(&db).await;
    let update = format!("{FORMS}/admin.smtp.update");
    if initially_stored {
        assert_eq!(call(&app, "POST", &update, &boss, Some(&smtp_form(25, "&password=A"))).await.0, StatusCode::OK);
    }
    let listener = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    let smtp = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (read, mut write) = socket.into_split();
        let mut lines = BufReader::new(read).lines();
        write.write_all(b"220 local.test ESMTP\r\n").await.unwrap();
        assert!(lines.next_line().await.unwrap().unwrap().starts_with("EHLO"));
        entered.send(()).unwrap();
        resume.await.unwrap();
        write.write_all(b"250 local.test\r\n").await.unwrap();
        while let Ok(Some(line)) = lines.next_line().await {
            let reply = if line == "QUIT" { "221 bye\r\n" } else { "250 OK\r\n" };
            write.write_all(reply.as_bytes()).await.unwrap();
            if line == "QUIT" { break; }
        }
    });
    let x_app = app.clone();
    let x_boss = boss.clone();
    let x_update = update.clone();
    let x = tokio::spawn(async move {
        call(&x_app, "POST", &x_update, &x_boss, Some(&smtp_form(port, "&enabled=true&password=__keep__"))).await
    });
    tokio::time::timeout(Duration::from_secs(2), waiting).await.unwrap().unwrap();
    let b = smtp_form(25, "&password=B").replace(&outbound_support::peer_host(), next_host);
    assert_eq!(tokio::time::timeout(Duration::from_secs(2), call(&app, "POST", &update, &boss, Some(&b))).await.expect("SMTP I/O must not hold the save transaction").0, StatusCode::OK);
    release.send(()).unwrap();
    let (status, body) = x.await.unwrap();
    let row = stored(&db).await;
    assert_eq!(row.get::<_, String>("host"), next_host, "B's host must survive the stale Keep save");
    let password: Option<String> = row.get("password_enc");
    assert_eq!(db.state.secrets.open(password.as_deref()).as_deref(), Some("B"));
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"]["message"].as_str().unwrap().contains("새로고침"));
    assert_eq!(audit_rows(&db.client, "settings:smtp").await.len(), if initially_stored { 2 } else { 1 });
    smtp.await.unwrap();
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn keep_save_rejects_a_concurrent_host_and_password_change() {
    keep_save_race(true, "localhost").await;
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn keep_save_rejects_a_concurrent_password_change_on_the_same_host() {
    keep_save_race(true, &outbound_support::peer_host()).await;
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn concurrent_first_keep_and_set_saves_never_adopt_an_unseen_password() {
    keep_save_race(false, "localhost").await;
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn test_send_audits_before_external_io_without_holding_the_pool() {
    use std::time::Duration;
    let db = auth_support::test_db_with(&[("API_DB_POOL_SIZE", "1")]).await;
    let app = app(&db);
    let cookie = boss(&db).await;
    let listener = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    let smtp = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        entered.send(()).unwrap();
        resume.await.unwrap();
        let (read, mut write) = socket.into_split();
        write.write_all(b"220 local.test ESMTP\r\n").await.unwrap();
        let mut lines = BufReader::new(read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let reply = if line == "QUIT" { "221 bye\r\n" } else { "250 OK\r\n" };
            write.write_all(reply.as_bytes()).await.unwrap();
            if line == "QUIT" { break; }
        }
    });
    let request = tokio::spawn(async move {
        call(&app, "POST", &format!("{FORMS}/admin.smtp.test"), &cookie, Some(&smtp_form(port, ""))).await
    });
    tokio::time::timeout(Duration::from_secs(3), waiting).await.unwrap().unwrap();
    let recorded_before_connect = audit_rows(&db.client, "settings:smtp-test").await.len();
    let lease_free = matches!(tokio::time::timeout(Duration::from_secs(1), db.state.db.get()).await, Ok(Ok(_)));
    release.send(()).unwrap();
    let (status, body) = request.await.unwrap();
    smtp.await.unwrap();
    assert_eq!(recorded_before_connect, 1, "audit must commit before external I/O starts");
    assert!(lease_free, "the SMTP wait must not hold the only pool lease");
    assert_eq!((status, body["result"]["ok"].as_bool()), (StatusCode::OK, Some(true)));
}
