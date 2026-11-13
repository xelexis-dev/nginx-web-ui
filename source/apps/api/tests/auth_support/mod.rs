//! Shared real-PostgreSQL harness for the auth integration tests (`auth_db`, `auth_http`).
//!
//! Each test gets its own database migrated by the product migrator `nwui_api::migrations::run`
//! (drizzle-exact, root-owned) over `packages/db/drizzle`.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use nwui_api::config::RuntimeConfig;
use nwui_api::request_context::RequestContext;
use nwui_api::state::AppState;
use nwui_api::control::model::RenderedConfig;
use tokio_postgres::NoTls;

pub const SECRET: &str = "auth-db-test-secret-not-for-production";

pub struct TestDb {
    pub state: AppState,
    pub client: tokio_postgres::Client,
    pub auth_log: PathBuf,
    _target_dns_dir: tempfile::TempDir,
}

fn migrations_dir() -> PathBuf {
    std::env::var("MIGRATIONS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle"))
}

async fn connect(url: &str) -> tokio_postgres::Client {
    let (client, connection) = tokio_postgres::connect(url, NoTls)
        .await
        .expect("connect to test postgres");
    tokio::spawn(connection);
    client
}

pub async fn test_db() -> TestDb {
    test_db_with(&[]).await
}

/// `extra` overrides environment keys (e.g. `API_DB_POOL_SIZE` or a legacy `MIGRATIONS_DIR`).
pub async fn test_db_with(extra: &[(&str, &str)]) -> TestDb {
    let directory = extra.iter().find(|(key, _)| *key == "MIGRATIONS_DIR")
        .map_or_else(migrations_dir, |(_, value)| PathBuf::from(value));
    test_db_with_migrations(extra, &directory).await
}

pub async fn test_db_with_migrations(extra: &[(&str, &str)], directory: &Path) -> TestDb {
    let admin_url = std::env::var("NWUI_TEST_DATABASE_URL").expect("NWUI_TEST_DATABASE_URL");
    let name = format!("nwui_t_{}", uuid::Uuid::new_v4().simple());
    connect(&admin_url)
        .await
        .batch_execute(&format!("CREATE DATABASE {name}"))
        .await
        .unwrap();
    let (base, _) = admin_url.rsplit_once('/').unwrap();
    let url = format!("{base}/{name}");

    let scratch = std::env::temp_dir().join(&name);
    std::fs::create_dir_all(&scratch).unwrap();
    let key_file = scratch.join("data_key");
    let auth_log = scratch.join("auth.log");
    let target_dns_dir = tempfile::tempdir().unwrap();
    let mut config = RuntimeConfig::read(|key| match key {
        _ if extra.iter().any(|(k, _)| *k == key) => extra
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| (*v).to_owned()),
        "DATABASE_URL" => Some(url.clone()),
        "AUTH_SECRET" => Some(SECRET.to_owned()),
        "DATA_KEY_FILE" => Some(key_file.display().to_string()),
        "AUTH_LOG" => Some(auth_log.display().to_string()),
        _ => None,
    })
    .unwrap();
    config.target_dns.descriptor_path = target_dns_dir.path().join("target-dns.json");
    std::fs::write(&config.data_key_file,"A5".repeat(32)).unwrap();let state = AppState::new(config).unwrap();
    // The product migrator (drizzle-exact; includes 0025 users.totp_last_step).
    nwui_api::migrations::run(&state.db, directory)
        .await
        .expect("migrations");
    TestDb {
        state,
        client: connect(&url).await,
        auth_log,
        _target_dns_dir: target_dns_dir,
    }
}

pub fn ctx() -> RequestContext {
    RequestContext {
        ip: Some("198.51.100.7".into()),
        external_origin: "http://admin.example.test".into(),
        secure: false,
        origin_configured: false,
    }
}

pub struct UserRow<'a> {
    pub id: &'a str,
    pub email: &'a str,
    pub role: &'a str,
    pub sv: i32,
    pub enabled: bool,
    pub approved: bool,
    pub totp_enabled: bool,
}

impl Default for UserRow<'_> {
    fn default() -> Self {
        Self {
            id: "u1",
            email: "u1@example.test",
            role: "editor",
            sv: 3,
            enabled: true,
            approved: true,
            totp_enabled: false,
        }
    }
}

pub async fn insert_user(
    client: &tokio_postgres::Client,
    row: UserRow<'_>,
    password_hash: Option<&str>,
) {
    client
        .execute(
            "INSERT INTO users (id, email, password_hash, role, session_version, enabled, approved, totp_enabled) \
             VALUES ($1, $2, $3, $4::text::role, $5, $6, $7, $8)",
            &[&row.id, &row.email, &password_hash, &row.role, &row.sv, &row.enabled, &row.approved, &row.totp_enabled],
        )
        .await
        .unwrap();
}

pub async fn audit_rows(client: &tokio_postgres::Client, action: &str) -> Vec<tokio_postgres::Row> {
    client
        .query("SELECT actor_id, actor_label, target, before, after, ip FROM audit_logs WHERE action = $1", &[&action])
        .await
        .unwrap()
}

#[cfg(unix)]
pub mod nginx;

/// A gateway fixture delegates a real active administrator, like the pairing route.
pub async fn gateway_issuer(client: &tokio_postgres::Client) {
    client.execute("INSERT INTO users(id,email,role,enabled,approved) VALUES('q3-gateway-issuer','q3-gateway-issuer@example.test','admin',true,true) ON CONFLICT(id) DO NOTHING", &[]).await.unwrap();
}

#[cfg(unix)]
pub mod restart_guard;
