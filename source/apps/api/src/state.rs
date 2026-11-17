use crate::auth::password::PasswordHasher;
use crate::{config::RuntimeConfig, crypto::SecretBox};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
pub struct AppState {
    pub db: Pool,
    pub config: Arc<RuntimeConfig>,
    pub secrets: Arc<SecretBox>,
    pub runtime: Arc<RuntimeState>,
    pub passwords: PasswordHasher,
    pub node_http: crate::nodes::client::NodeHttp,
    pub webhook_http: crate::notify::send::HttpClient,
}

pub struct RuntimeState {
    pub(crate) storage_selection: std::sync::OnceLock<crate::upgrade::StorageSelection>,
    /// All materialization, apply and rollback operations share this owner. Operations that
    /// touch nginx's files also take `control::nginx_lock`, which the restart guard shares.
    pub apply_lock: tokio::sync::Mutex<()>,
    /// Login and node pairing share one address throttle across all handlers.
    pub user_token_limits: crate::auth::token_limits::TokenLimits,
    pub login_throttle: crate::auth::throttle::LoginThrottle,
    /// Password sign-ins waiting for the second factor (`/login/2fa`).
    pub pending_logins: crate::auth::pending_login::PendingLogins,
    pub passkey_ceremonies: crate::auth::passkey_ceremony::Ceremonies,
    pub pending_oidc: crate::auth::oidc::PendingTransactions,
    pub shutdown: tokio::sync::watch::Sender<bool>,
    pub certificate_jobs: crate::certificates::jobs::JobRuntime,
    pub observability_started: std::sync::atomic::AtomicBool,
    pub notifications_started: std::sync::atomic::AtomicBool,
    pub metrics_cache: tokio::sync::Mutex<crate::observability::query::MetricsCache>,
    pub metrics_parse_lock: tokio::sync::Mutex<()>,
    pub publisher: crate::publisher_client::Client,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            storage_selection: Default::default(),
            apply_lock: tokio::sync::Mutex::new(()),
            login_throttle: Default::default(),
            user_token_limits: Default::default(),
            pending_logins: Default::default(),
            passkey_ceremonies: Default::default(),
            pending_oidc: Default::default(),
            shutdown: tokio::sync::watch::channel(false).0,
            certificate_jobs: Default::default(),
            observability_started: Default::default(),
            notifications_started: Default::default(),
            metrics_cache: Default::default(),
            metrics_parse_lock: Default::default(),
            publisher: Default::default(),
        }
    }
}

impl AppState {
    pub fn new(config: RuntimeConfig) -> anyhow::Result<Self> {
        // Validate the database URL before reading the runner-prepared key.
        config.database_url.parse::<tokio_postgres::Config>()?;
        let secrets=SecretBox::load_existing(&config.data_key_file).map_err(|error|match &error {
            crate::crypto::CryptoError::Io{source,..} if source.kind()==std::io::ErrorKind::NotFound=>anyhow::Error::from(crate::upgrade::StepError{
                code:"key_missing",message:"선택된 저장소의 암호화 키가 없습니다. 기존 키를 복원한 뒤 다시 시작하세요.",detail:None,
            }),
            _=>error.into(),
        })?;
        Self::new_with_secrets(config,secrets)
    }

    pub fn new_with_secrets(config: RuntimeConfig, secrets: SecretBox) -> anyhow::Result<Self> {
        let pg = config.database_url.parse::<tokio_postgres::Config>()?;
        let manager = Manager::from_config(
            pg,
            tokio_postgres::NoTls,
            ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            },
        );
        let db = Pool::builder(manager)
            .max_size(config.database_pool_size)
            .runtime(deadpool_postgres::Runtime::Tokio1)
            .wait_timeout(Some(Duration::from_secs(5)))
            .create_timeout(Some(Duration::from_secs(5)))
            .recycle_timeout(Some(Duration::from_secs(2)))
            .build()?;
        let secrets = Arc::new(secrets);
        let passwords = PasswordHasher::new(config.password_concurrency);
        let node_http = crate::nodes::client::NodeHttp::new(&config.version,&config)?;
        let webhook_http = crate::notify::send::client(&config);
        Ok(Self {
            db,
            secrets,
            passwords,
            node_http,
            webhook_http,
            config: Arc::new(config),
            runtime: Arc::new(RuntimeState::default()),
        })
    }
}

#[cfg(test)]
mod key_lifecycle_tests {
    use super::*;
    fn config(key:&std::path::Path)->RuntimeConfig {
        RuntimeConfig::read(|name|match name {
            "DATABASE_URL"=>Some("postgres://fixture@127.0.0.1/test".into()),
            "AUTH_SECRET"=>Some("state-key-test-only".into()),"DATA_KEY_FILE"=>Some(key.display().to_string()),_=>None,
        }).unwrap()
    }
    #[test]
    fn app_state_refuses_a_missing_key_without_creating_its_file_or_directory() {
        let temp=tempfile::tempdir().unwrap();let key=temp.path().join("missing-parent/data_key");
        let error=AppState::new(config(&key)).err().expect("AppState must never create a canonical key");
        let error=error.downcast_ref::<crate::upgrade::StepError>().expect("typed startup refusal");
        assert_eq!(error.code,"key_missing");assert_eq!(error.message,"선택된 저장소의 암호화 키가 없습니다. 기존 키를 복원한 뒤 다시 시작하세요.");
        assert!(!key.parent().unwrap().exists());
    }
    #[test]
    fn app_state_loads_the_prepared_key_without_rewriting_its_bytes() {
        let temp=tempfile::tempdir().unwrap();let key=temp.path().join("data_key");let bytes=format!("\n{}\n","AB".repeat(32));std::fs::write(&key,&bytes).unwrap();
        AppState::new(config(&key)).unwrap();assert_eq!(std::fs::read(&key).unwrap(),bytes.as_bytes());
    }
}
