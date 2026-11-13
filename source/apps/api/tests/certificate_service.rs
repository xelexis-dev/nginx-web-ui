mod auth_support;
use nwui_api::{
    certificates::{service, store},
    contracts::Actor,
};
use serde_json::json;
fn actor() -> Actor {
    Actor {
        id: None,
        label: Some("gateway:test".into()),
        ip: None,
    }
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn queued_job_audit_survives_actor_deleted_before_completion() {
    let t = auth_support::test_db().await;
    let mut db = t.state.db.get().await.unwrap();
    let tx = db.transaction().await.unwrap();
    let deleted = Actor {
        id: Some("removed-user".into()),
        label: None,
        ip: None,
    };
    store::audit(
        &*tx,
        &deleted,
        "cert:issue-result",
        "cert",
        json!({"ok":true}),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let row = t
        .client
        .query_one(
            "SELECT actor_id,actor_label FROM audit_logs WHERE target='cert'",
            &[],
        )
        .await
        .unwrap();
    assert!(row.get::<_, Option<String>>(0).is_none());
    assert_eq!(
        row.get::<_, Option<String>>(1).as_deref(),
        Some("deleted-user:removed-user")
    );
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn create_atomically_enqueues_sealed_snapshot_and_cancel_is_durable() {
    let t = auth_support::test_db().await;
    let input = service::parse_input(
        &json!({"domains":["example.com"],"email":"admin@example.com","alias":"before"}),
    )
    .unwrap();
    let id = service::create(&t.state, &input, &actor()).await.unwrap();
    let job = t
        .client
        .query_one(
            "SELECT payload_enc,state FROM certificate_jobs WHERE certificate_id=$1",
            &[&id],
        )
        .await
        .unwrap();
    let enc: String = job.get(0);
    assert!(enc.starts_with("enc:v1:"));
    assert!(!enc.contains("PRIVATE KEY"));
    assert_eq!(job.get::<_, String>(1), "queued");
    assert!(
        service::update(&t.state, &id, &input, &actor())
            .await
            .is_err()
    );
    service::cancel(&t.state, &id, &actor()).await.unwrap();
    assert!(
        t.client
            .query_one(
                "SELECT cancel_requested FROM certificate_jobs WHERE certificate_id=$1",
                &[&id]
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
    assert_eq!(
        store::get(&t.client, &id, false).await.unwrap().status,
        "failed"
    );
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn metadata_update_preserves_live_material_and_omitted_alias() {
    let t = auth_support::test_db().await;
    t.client.execute("INSERT INTO certificates(id,domains,status,alias,cert_pem,key_pem) VALUES('c','[\"example.com\"]','issued','keep','old cert','old key')",&[]).await.unwrap();
    let input =
        service::parse_input(&json!({"domains":["www.example.com"],"email":"admin@example.com"}))
            .unwrap();
    service::update(&t.state, "c", &input, &actor())
        .await
        .unwrap();
    let cert = store::get(&t.client, "c", false).await.unwrap();
    assert_eq!(cert.alias.as_deref(), Some("keep"));
    assert_eq!(cert.cert_pem.as_deref(), Some("old cert"));
    assert_eq!(cert.status, "issued");
    let clear = service::parse_input(
        &json!({"domains":["www.example.com"],"email":"admin@example.com","alias":null}),
    )
    .unwrap();
    service::update(&t.state, "c", &clear, &actor())
        .await
        .unwrap();
    assert!(
        store::get(&t.client, "c", false)
            .await
            .unwrap()
            .alias
            .is_none()
    );
}
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn pem_import_replace_seals_private_key_and_linked_delete_is_rejected() {
    let t = auth_support::test_db().await;
    let material = rcgen::generate_simple_self_signed(vec!["example.com".into()]).unwrap();
    let input = service::CertificatePemInput {
        cert_pem: material.cert.pem(),
        key_pem: material.signing_key.serialize_pem(),
    };
    let id = service::import(&t.state, &input, &actor()).await.unwrap();
    let old = store::get(&t.client, &id, false).await.unwrap();
    assert_eq!(old.source, "manual");
    assert!(old.key_pem.as_deref().unwrap().starts_with("enc:v1:"));
    assert_eq!(
        t.state.secrets.open(old.key_pem.as_deref()).as_deref(),
        Some(input.key_pem.as_str())
    );
    let other = rcgen::generate_simple_self_signed(vec!["www.example.com".into()]).unwrap();
    let replacement = service::CertificatePemInput {
        cert_pem: other.cert.pem(),
        key_pem: other.signing_key.serialize_pem(),
    };
    let result = service::replace(&t.state, &id, &replacement, &actor())
        .await
        .unwrap();
    assert!(result["applyError"].is_null());
    assert_eq!(
        store::get(&t.client, &id, false).await.unwrap().names(),
        vec!["www.example.com"]
    );
    t.client.execute("INSERT INTO nginx_servers(id,server_name,certificate_id) VALUES('linked','www.example.com',$1)",&[&id]).await.unwrap();
    let error = service::remove(&t.state, &id, &actor()).await.unwrap_err();
    assert_eq!(error.status, axum::http::StatusCode::CONFLICT);
    t.client
        .execute("DELETE FROM nginx_servers WHERE id='linked'", &[])
        .await
        .unwrap();
    service::remove(&t.state, &id, &actor()).await.unwrap();
    assert!(
        t.client
            .query_opt("SELECT id FROM certificates WHERE id=$1", &[&id])
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn changing_dns_provider_clears_the_manual_order_without_losing_material() {
    use axum::{Json, Router};
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", socket.local_addr().unwrap());
    let app = Router::new().fallback(|| async {
        Json(json!({"success":true,"result":[{"id":"zone","name":"example.com"}]}))
    });
    let server = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
    let t = auth_support::test_db_with(&[("CF_API_BASE", &base)]).await;
    t.client.execute("INSERT INTO certificates(id,domains,status,challenge_type,dns_order,cert_pem,key_pem) VALUES('provider-change','[\"example.com\"]','issued','dns-01','{\"order\":{\"url\":\"https://ca.example/order/old\"}}','old cert','old key')", &[]).await.unwrap();
    let input=service::parse_input(&json!({"domains":["example.com"],"email":"admin@example.com","challengeType":"dns-01","dnsProvider":"cloudflare","dnsToken":"test-token"})).unwrap();
    service::update(&t.state, "provider-change", &input, &actor())
        .await
        .unwrap();
    let cert = store::get(&t.client, "provider-change", false)
        .await
        .unwrap();
    assert_eq!(cert.dns_provider.as_deref(), Some("cloudflare"));
    assert!(
        cert.dns_order.is_none(),
        "manual order must not strand automated renewals"
    );
    assert_eq!(cert.cert_pem.as_deref(), Some("old cert"));
    assert_eq!(cert.key_pem.as_deref(), Some("old key"));
    assert_eq!(cert.status, "issued");
    server.abort();
}

#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn provider_switch_from_awaiting_dns_allows_retry() {
    use axum::{Json, Router};
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", socket.local_addr().unwrap());
    let app = Router::new().fallback(|| async {
        Json(json!({"success":true,"result":[{"id":"zone","name":"example.com"}]}))
    });
    let server = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
    let t = auth_support::test_db_with(&[("CF_API_BASE", &base)]).await;
    t.client.execute("INSERT INTO certificates(id,domains,status,challenge_type,dns_order,cert_pem,key_pem) VALUES('provider-change','[\"example.com\"]','awaiting-dns','dns-01','{\"order\":{\"url\":\"https://ca.example/order/old\"}}','old cert','old key')", &[]).await.unwrap();
    let input=service::parse_input(&json!({"domains":["example.com"],"email":"admin@example.com","challengeType":"dns-01","dnsProvider":"cloudflare","dnsToken":"test-token"})).unwrap();
    service::update(&t.state, "provider-change", &input, &actor())
        .await
        .unwrap();
    let cert = store::get(&t.client, "provider-change", false)
        .await
        .unwrap();
    assert_eq!(cert.dns_provider.as_deref(), Some("cloudflare"));
    assert!(
        cert.dns_order.is_none(),
        "manual order must not strand automated renewals"
    );
    assert_eq!(cert.cert_pem.as_deref(), Some("old cert"));
    assert_eq!(cert.key_pem.as_deref(), Some("old key"));
    assert_eq!(cert.status, "failed");
    assert!(
        cert.last_error
            .as_deref()
            .is_some_and(|s| s.contains("설정"))
    );
    service::retry(&t.state, "provider-change", &actor())
        .await
        .unwrap();
    let queued:bool=t.client.query_one("SELECT EXISTS(SELECT 1 FROM certificate_jobs WHERE certificate_id='provider-change' AND kind='dns_auto' AND state='queued')",&[]).await.unwrap().get(0);
    assert!(queued);
    server.abort();
}
