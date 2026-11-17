#[path="../src/db_upgrade/settings.rs"] mod settings;
#[path="../src/db_upgrade/state.rs"] mod state;
#[path="../src/db_upgrade/catalog.rs"] mod catalog;
pub use nwui_api::crypto;
pub use nwui_api::db_upgrade::cluster;
#[path="../src/db_upgrade/error.rs"] mod error;
#[path="../src/db_upgrade/inspect.rs"] mod inspect;
#[test]
fn nested_sealed_values_require_the_existing_key_and_are_never_returned() {
    let key=crypto::SecretBox::from_key([4;32]);let value=key.seal("private fixture").unwrap();
    let json=serde_json::json!({"nested":[{"stored":value}]});
    assert!(inspect::check_sealed(&json,Some(&key)).unwrap());
    assert_eq!(inspect::check_sealed(&json,None).unwrap_err().code,"pair_mismatch");
    assert_eq!(inspect::check_sealed(&json,Some(&crypto::SecretBox::from_key([5;32]))).unwrap_err().code,"pair_mismatch");
    assert!(!inspect::check_sealed(&serde_json::json!({"legacy":"plain"}),None).unwrap());
}
#[test]
fn quoting_identifiers_is_literal_even_for_untrusted_database_names() {
    assert_eq!(inspect::quote("has\"quote"),"\"has\"\"quote\"");
}
mod auth_support;
async fn legacy_db()->auth_support::TestDb {
    // The offline source profile is 1.19.11, before 2.x functions/standalone sequences.
    let history=tempfile::tempdir().unwrap();std::fs::create_dir(history.path().join("meta")).unwrap();
    let current=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");
    let mut journal:serde_json::Value=serde_json::from_slice(&std::fs::read(current.join("meta/_journal.json")).unwrap()).unwrap();
    let entries=journal["entries"].as_array_mut().unwrap();entries.retain(|entry|entry["idx"].as_u64().unwrap()<=27);
    assert_eq!(entries.last().unwrap()["tag"],"0027_notification_delivery_queue");
    for entry in entries {let name=format!("{}.sql",entry["tag"].as_str().unwrap());std::fs::copy(current.join(&name),history.path().join(name)).unwrap();}
    std::fs::write(history.path().join("meta/_journal.json"),serde_json::to_vec(&journal).unwrap()).unwrap();
    auth_support::test_db_with_migrations(&[],history.path()).await
}
#[tokio::test]
#[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
async fn real_migrated_data_baseline_checks_encryption_rows_sequences_and_extra_objects() {
    let t=legacy_db().await;
    let empty=inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap();
    assert!(!empty.has_encrypted);assert!(empty.tables.contains_key("drizzle.__drizzle_migrations"));
    auth_support::insert_user(&t.client,auth_support::UserRow::default(),None).await;
    let sealed=t.state.secrets.seal("synthetic-totp").unwrap();
    t.client.execute("UPDATE users SET totp_secret=$1 WHERE id='u1'",&[&sealed]).await.unwrap();
    let baseline=inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap();
    assert!(baseline.has_encrypted);assert_eq!(baseline.tables["public.users"].rows,1);
    assert_eq!(baseline,inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap());
    assert_eq!(inspect::snapshot_database(&t.client,None,true).await.unwrap_err().code,"pair_mismatch");
    assert_eq!(inspect::snapshot_database(&t.client,Some(&crypto::SecretBox::from_key([7;32])),true).await.unwrap_err().code,"pair_mismatch");
    t.client.batch_execute("UPDATE users SET totp_secret=repeat('x',16777217) WHERE id='u1'").await.unwrap();
    assert_eq!(inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap_err().code,"unsupported_layout");
    t.client.execute("UPDATE users SET totp_secret=$1 WHERE id='u1'",&[&sealed]).await.unwrap();
    t.client.batch_execute("CREATE TABLE public.foreign_data(id int)").await.unwrap();
    assert_eq!(inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap_err().code,"unsupported_layout");
    t.client.batch_execute("DROP TABLE public.foreign_data; CREATE SEQUENCE public.unowned_sequence").await.unwrap();
    assert_eq!(inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap_err().code,"unsupported_layout");
    t.client.batch_execute("DROP SEQUENCE public.unowned_sequence; CREATE FUNCTION public.foreign_function() RETURNS integer LANGUAGE sql AS 'SELECT 1'").await.unwrap();
    assert_eq!(inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap_err().code,"unsupported_layout");
    t.client.batch_execute("DROP FUNCTION public.foreign_function(); CREATE TYPE public.foreign_enum AS ENUM('foreign')").await.unwrap();
    assert_eq!(inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap_err().code,"unsupported_layout");
    t.client.batch_execute("DROP TYPE public.foreign_enum; CREATE DOMAIN public.foreign_domain AS text").await.unwrap();
    assert_eq!(inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap_err().code,"unsupported_layout");
}

#[tokio::test]
#[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
async fn system_schema_and_uncovered_catalog_objects_refuse_without_changing_data() {
    let t=legacy_db().await;
    let baseline=inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap();
    let mut accepted=Vec::new();
    for (create,drop) in [
        ("CREATE FUNCTION pg_catalog.review_foreign() RETURNS integer LANGUAGE sql AS 'SELECT 1'","DROP FUNCTION pg_catalog.review_foreign()"),
        ("CREATE TABLE information_schema.review_foreign(value text)","DROP TABLE information_schema.review_foreign"),
        ("CREATE COLLATION public.review_foreign FROM pg_catalog.\"C\"","DROP COLLATION public.review_foreign"),
        ("CREATE ACCESS METHOD review_foreign TYPE TABLE HANDLER pg_catalog.heap_tableam_handler","DROP ACCESS METHOD review_foreign"),
        ("CREATE TEXT SEARCH CONFIGURATION public.review_foreign (COPY=pg_catalog.simple)","DROP TEXT SEARCH CONFIGURATION public.review_foreign"),
        ("ALTER DEFAULT PRIVILEGES GRANT SELECT ON TABLES TO PUBLIC","ALTER DEFAULT PRIVILEGES REVOKE SELECT ON TABLES FROM PUBLIC"),
    ] {
        t.client.batch_execute(create).await.unwrap();
        match inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await {
            Ok(_)=>accepted.push(create),Err(error)=>assert_eq!(error.code,"unsupported_layout"),
        }
        t.client.batch_execute(drop).await.unwrap();
        assert_eq!(baseline,inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap(),"inspection mutated application data");
    }
    assert!(accepted.is_empty(),"uncovered objects accepted: {accepted:?}");
}

#[tokio::test]
#[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
async fn empty_temporary_namespaces_pass_but_live_temporary_objects_refuse() {
    let t=legacy_db().await;
    let before=inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap();
    let database:String=t.client.query_one("SELECT current_database()",&[]).await.unwrap().get(0);
    let mut config=std::env::var("NWUI_TEST_DATABASE_URL").unwrap().parse::<tokio_postgres::Config>().unwrap();config.dbname(&database);
    let (other,connection)=config.connect(tokio_postgres::NoTls).await.unwrap();let driver=tokio::spawn(connection);
    other.batch_execute("CREATE TEMP TABLE review_temporary(value text)").await.unwrap();
    assert_eq!(inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap_err().code,"unsupported_layout");
    other.batch_execute("DROP TABLE review_temporary").await.unwrap();drop(other);driver.abort();
    assert_eq!(before,inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap());
}

#[tokio::test]
#[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
async fn internal_fk_trigger_names_are_not_identity_but_enabled_state_is() {
    let t=legacy_db().await;
    let before=inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap();
    let row=t.client.query_one("SELECT n.nspname,c.relname,t.tgname FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE t.tgisinternal AND n.nspname='public' LIMIT 1",&[]).await.unwrap();
    let table=format!("{}.{}",inspect::quote(row.get(0)),inspect::quote(row.get(1)));let original:String=row.get(2);
    t.client.batch_execute(&format!("ALTER TRIGGER {} ON {table} RENAME TO review_renamed",inspect::quote(&original))).await.unwrap();
    assert_eq!(before,inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap());
    t.client.batch_execute(&format!("ALTER TABLE {table} DISABLE TRIGGER review_renamed")).await.unwrap();
    let changed=inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap();
    assert_eq!(before.tables,changed.tables);assert_eq!(before.sequences,changed.sequences);assert_ne!(before.schema_sha256,changed.schema_sha256);
    t.client.batch_execute(&format!("ALTER TABLE {table} ENABLE TRIGGER review_renamed; ALTER TRIGGER review_renamed ON {table} RENAME TO {}",inspect::quote(&original))).await.unwrap();
    assert_eq!(before,inspect::snapshot_database(&t.client,Some(&t.state.secrets),true).await.unwrap());
}

#[tokio::test]
#[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
async fn catalog_reads_version_independent_locale_and_hashed_role_identity() {
    let t=auth_support::test_db().await;
    let (databases,roles)=inspect::read_catalog(&t.client).await.unwrap();
    let db=databases.iter().find(|d|d.name=="template0").unwrap();
    assert!(!db.encoding.is_empty());assert!(matches!(db.provider.as_str(),"c"|"i"));
    let postgres=roles.iter().find(|r|r.name=="postgres").unwrap();
    assert_eq!(postgres.oid,"10");assert!(postgres.login);assert!(postgres.elevated);
    if let Some(hash)=&postgres.password_sha256{assert!(state::sha256(hash));}
    assert!(catalog::validate(&databases,&roles,false).is_err(),"the CI cluster's extra test databases must never pass the product profile");
}
#[tokio::test]
#[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
async fn private_inspector_does_not_load_database_configured_session_libraries() {
    let t=auth_support::test_db().await;
    let name:String=t.client.query_one("SELECT current_database()",&[]).await.unwrap().get(0);
    t.client.batch_execute(&format!("ALTER DATABASE {} SET session_preload_libraries='nwui_missing_probe_library'",inspect::quote(&name))).await.unwrap();
    let url=std::env::var("NWUI_TEST_DATABASE_URL").unwrap();
    let mut unsafe_config=url.parse::<tokio_postgres::Config>().unwrap();unsafe_config.dbname(&name);
    assert!(unsafe_config.connect(tokio_postgres::NoTls).await.is_err());
    let private=inspect::connection_config(std::path::Path::new("/private/socket"),&name,"postgres",None);
    let mut safe_config=url.parse::<tokio_postgres::Config>().unwrap();safe_config.dbname(&name).options(private.get_options().unwrap());
    let (client,connection)=safe_config.connect(tokio_postgres::NoTls).await.unwrap();let driver=tokio::spawn(connection);
    assert_eq!(client.query_one("SELECT 1",&[]).await.unwrap().get::<_,i32>(0),1);
    client.batch_execute(&format!("ALTER DATABASE {} RESET session_preload_libraries",inspect::quote(&name))).await.unwrap();driver.abort();
}
#[test]
fn only_the_superuser_inspection_connection_overrides_su_preload_settings() {
    let root=inspect::connection_config(std::path::Path::new("/private/socket"),"postgres","postgres",None);
    let application=inspect::connection_config(std::path::Path::new("/private/socket"),"nginx_web_ui","nginx",Some("test-only"));
    assert!(root.get_options().unwrap().contains("session_preload_libraries="));
    assert!(!application.get_options().unwrap().contains("session_preload_libraries="));
}
