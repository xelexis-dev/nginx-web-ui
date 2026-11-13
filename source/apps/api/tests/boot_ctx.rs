//! Runner boot steps use an explicit private Unix-socket pool, not AppState.
use nwui_api::{config::RuntimeConfig, crypto::SecretBox, runtime_boot::{self, BootCtx}};

#[test]
fn p0b_core_keeps_base_listener_reservations_without_requiring_fresh_secrets() {
    runtime_boot::preflight_ports(3001,3000,5432).unwrap();
    for port in [80,443,8080,3000,5432] {
        let error=runtime_boot::preflight_ports(port,3000,5432).unwrap_err();
        assert_eq!(error.code,"api_port_conflict");
        assert!(error.detail.unwrap().contains(&format!("API_PORT={port} conflicts")));
    }
}

#[test]
fn environment_preflight_needs_no_database_or_state() {
    let mut config = RuntimeConfig::read(|name| match name {
        "DATABASE_URL" => Some("postgres://postgres@localhost/postgres?host=/run/n2-test".into()),
        "AUTH_SECRET" => Some("fixture-only".into()), _ => None,
    }).unwrap();
    runtime_boot::preflight_api_port_env(&config).unwrap();
    config.listen.set_port(config.web_port);
    let error = runtime_boot::preflight_api_port_env(&config).unwrap_err();
    assert_eq!(error.code, "api_port_conflict");
    assert!(error.detail.unwrap().contains("API_PORT="));
}

#[tokio::test]
#[ignore = "requires disposable NWUI_N2_PRIVATE_DATABASE_URL with a Unix socket"]
async fn runner_cores_use_the_private_pool_and_preserve_idempotence() {
    let Ok(url)=std::env::var("NWUI_N2_PRIVATE_DATABASE_URL") else {
        eprintln!("private-pool qualification requires NWUI_N2_PRIVATE_DATABASE_URL");
        return;
    };
    let pg: tokio_postgres::Config = url.parse().unwrap();
    assert!(pg.get_hosts().iter().all(|host| matches!(host, tokio_postgres::config::Host::Unix(_))));
    let (admin, connection) = pg.connect(tokio_postgres::NoTls).await.unwrap(); tokio::spawn(connection);
    let name = format!("n2_boot_{}", uuid::Uuid::new_v4().simple());
    admin.batch_execute(&format!("CREATE DATABASE {name}")).await.unwrap();
    let mut selected = pg.clone(); selected.dbname(&name);
    let manager = deadpool_postgres::Manager::new(selected.clone(),tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager).max_size(1).build().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut config = RuntimeConfig::read(|key| match key {
        "DATABASE_URL" => Some("postgres://postgres@localhost/postgres?host=/run/n2-test".into()),
        "AUTH_SECRET" => Some("fixture-only".into()), _ => None,
    }).unwrap();
    config.migrations_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");
    config.target_dns.descriptor_path = temp.path().join("target-dns.json");
    let secrets = SecretBox::from_key([13;32]);
    let ctx = BootCtx {config:&config, secrets:&secrets, pool:&pool};
    assert!(runtime_boot::run_migrations(&ctx).await.unwrap() > 0);
    assert_eq!(runtime_boot::run_migrations(&ctx).await.unwrap(), 0);
    runtime_boot::preflight_api_port_rows(&ctx).await.unwrap();
    let id = runtime_boot::ensure_instance(&ctx).await.unwrap();
    assert_eq!(runtime_boot::ensure_instance(&ctx).await.unwrap(), id);
    assert_eq!(runtime_boot::seed_admin(&ctx,None,None).await.unwrap(),nwui_api::auth::setup::SeedOutcome::Skipped);
    assert_eq!(runtime_boot::seed_admin(&ctx,Some(" ADMIN@example.com "),Some("fixture-password-only")).await.unwrap(),nwui_api::auth::setup::SeedOutcome::Seeded);
    assert_eq!(runtime_boot::seed_admin(&ctx,Some("second@example.com"),Some("another-fixture")).await.unwrap(),nwui_api::auth::setup::SeedOutcome::Skipped);
    let descriptor = runtime_boot::dns_preflight(&ctx).await.unwrap();
    assert_eq!(descriptor.available, serde_json::from_slice::<nwui_api::dns_guard::Descriptor>(&std::fs::read(&config.target_dns.descriptor_path).unwrap()).unwrap().available);
    assert!(runtime_boot::backfill(&ctx,None).await.unwrap().is_none());
    let client = pool.get().await.unwrap();
    client.execute("INSERT INTO certificates(id,domains,key_pem) VALUES('old','[\"old.test\"]','plain fixture')",&[]).await.unwrap();
    client.execute("INSERT INTO nginx_servers(id,server_name,listen,enabled) VALUES('disabled','old.test',$1,false)",&[&i32::from(config.listen.port())]).await.unwrap();
    drop(client);
    let error = runtime_boot::preflight_api_port_rows(&ctx).await.unwrap_err();
    assert_eq!(error.code,"api_port_conflict"); assert!(error.detail.unwrap().contains("disabled"));
    let result = runtime_boot::backfill(&ctx,Some("1")).await.unwrap().unwrap();
    assert_eq!(result.cert_keys,1); assert_eq!(result.failed,0);
    assert_eq!(runtime_boot::backfill(&ctx,Some("1")).await.unwrap().unwrap().cert_keys,0);
    let client = pool.get().await.unwrap();
    let sealed:String = client.query_one("SELECT key_pem FROM certificates WHERE id='old'",&[]).await.unwrap().get(0);
    assert_eq!(secrets.open(Some(&sealed)).as_deref(),Some("plain fixture")); drop(client);
    let mut invalid = config.clone(); invalid.migrations_dir = temp.path().join("missing-migrations");
    let invalid_ctx = BootCtx {config:&invalid,secrets:&secrets,pool:&pool};
    let error=runtime_boot::run_migrations(&invalid_ctx).await.unwrap_err();
    assert_eq!(error.code,"migration_failed");assert!(error.detail.unwrap().contains("run_migrations: read migration journal"));
    pool.close();
    admin.batch_execute(&format!("DROP DATABASE {name} WITH (FORCE)")).await.unwrap();
}
