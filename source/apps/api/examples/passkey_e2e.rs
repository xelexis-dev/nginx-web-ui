//! Disposable real-HTTP harness for browser passkey end-to-end checks. Never use an operator DB.
#[path = "../tests/auth_support/mod.rs"]
mod auth_support;

#[tokio::main]
async fn main() {
    let db = auth_support::test_db().await;
    let hash = nwui_api::auth::password::hash_password("Passw0rd-passkey-e2e").unwrap();
    auth_support::insert_user(
        &db.client,
        auth_support::UserRow {
            role: "admin",
            ..Default::default()
        },
        Some(&hash),
    )
    .await;
    auth_support::insert_user(
        &db.client,
        auth_support::UserRow { id: "u2", email: "u2@example.test", role: "admin", ..Default::default() },
        Some(&hash),
    ).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3111")
        .await
        .unwrap();
    eprintln!("passkey-e2e ready: 127.0.0.1:3111");
    axum::serve(
        listener,
        nwui_api::server::router(db.state.clone())
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
}
