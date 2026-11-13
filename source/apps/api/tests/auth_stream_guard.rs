//! Long-lived responses (log streams) authorize at connect time; the stream guard ends them when
//! the session that opened them stops being valid.
mod auth_support;

use std::time::Duration;

use auth_support::{UserRow, insert_user, test_db};
use axum::http::{HeaderMap, header};
use nwui_api::auth::{principal, session, stream_guard};
use serde_json::json;

fn session_headers(id: &str, sv: i64) -> HeaderMap {
    let now = principal::now_unix_ms() / 1000;
    let claims = json!({"sub": id, "id": id, "sv": sv, "requires2fa": false});
    let token = session::encode(claims.as_object().unwrap(), auth_support::SECRET, session::COOKIE_NAME_PLAIN, now, 3600).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(header::COOKIE, format!("{}={token}", session::COOKIE_NAME_PLAIN).parse().unwrap());
    headers
}

async fn fires_within(rx: &mut tokio::sync::watch::Receiver<bool>, limit: Duration) -> bool {
    tokio::time::timeout(limit, rx.wait_for(|v| *v)).await.is_ok_and(|r| r.is_ok())
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_stream_ends_when_its_session_is_revoked_and_not_before() {
    let db = test_db().await;
    insert_user(&db.client, UserRow::default(), None).await;
    let mut rx = stream_guard::session_bound_shutdown(&db.state, session_headers("u1", 3), auth_support::ctx(), None, Duration::from_millis(100));
    assert!(!fires_within(&mut rx, Duration::from_millis(400)).await, "a valid session keeps streaming");
    // Sign-out elsewhere, a disable or a role change all move sessionVersion on.
    db.client.execute("UPDATE users SET session_version = session_version + 1 WHERE id = 'u1'", &[]).await.unwrap();
    assert!(fires_within(&mut rx, Duration::from_secs(2)).await, "a revoked session ends the stream");
}

#[tokio::test]
#[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
async fn a_stream_still_ends_on_process_shutdown() {
    let db = test_db().await;
    insert_user(&db.client, UserRow::default(), None).await;
    let mut rx = stream_guard::session_bound_shutdown(&db.state, session_headers("u1", 3), auth_support::ctx(), None, Duration::from_secs(60));
    db.state.runtime.shutdown.send_replace(true);
    assert!(fires_within(&mut rx, Duration::from_secs(2)).await);
}
