use crate::{config::RuntimeConfig, control::model::{GuardReport,SkippedRecord}, crypto::SecretBox, upgrade::{PrivatePg, StepError}};
use serde::{Deserialize, Serialize};

pub use super::store::PublisherStore;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StagedPayload { pub id: String, pub manifest_sha256: String, pub skipped: Vec<SkippedRecord> }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum BootMode { Conversion, Ready }

pub enum PgSource<'a> { Private(&'a PrivatePg), Pool(&'a deadpool_postgres::Pool) }
impl PgSource<'_> {
    fn pool(self, config: &RuntimeConfig) -> Result<deadpool_postgres::Pool, StepError> {
        match self {
            Self::Private(private) => private.pool(config),
            Self::Pool(pool) => Ok(pool.clone()),
        }
    }
}
pub async fn prepare_payload(config: &RuntimeConfig, secrets: &SecretBox, pg: PgSource<'_>, store: &PublisherStore, mode:BootMode) -> Result<StagedPayload, StepError> {
    let admitted=crate::nonroot::admit_nginx(config).await?;
    super::prepare::prepare(config.clone(),secrets,pg.pool(config)?,store.clone(),&admitted,mode).await
}
pub async fn publish_live(config: &RuntimeConfig, store: &PublisherStore, id: &str) -> Result<PublishReport, StepError> {
    let admitted=crate::nonroot::admit_nginx(config).await?;
    super::publication::publish(config.clone(),store.clone(),id.into(),admitted).await
}
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PublishReport { pub files_written: usize, pub validation_output: String }
pub async fn guard(config: &RuntimeConfig, _store: &PublisherStore) -> Result<GuardReport, StepError> {
    let _store_lock=_store.lock(std::time::Duration::from_secs(30)).await.map_err(|error|StepError{code:"storage_busy",message:"발행기 저장소를 다른 작업에서 사용 중입니다.",detail:Some(error.to_string())})?;
    let _files = crate::publisher::nginx_lock::acquire(config).await.map_err(|error| StepError {
        code: "storage_busy", message: "nginx 설정 파일을 다른 작업에서 사용 중입니다.",
        detail: Some(error.to_string().chars().take(512).collect()),
    })?;
    super::recovery::before_start_for_guard(config,_store).await.map_err(|error|StepError {
        code:"restore_incomplete",message:"nginx의 이전 설정 복원을 완료하지 못했습니다.",detail:Some(format!("{error:#}").chars().take(1024).collect()),
    })?;
    crate::publisher::bootstrap::guard_durable(config, &config.nginx_main_conf, &config.nginx_quarantine_dir,_store,Some(unsafe{libc::getppid()}as u32))
        .await.map_err(|error| {
            if let Some(step)=error.downcast_ref::<StepError>() {return StepError{code:step.code,message:step.message,detail:step.detail.clone()};}
            StepError {code:"guard_failed",message:"nginx 관리 설정 검사를 완료하지 못했습니다.",detail:Some(error.to_string().chars().take(512).collect())}
        })
}

#[cfg(test)]
mod source_tests {
    use super::*;

    fn config() -> RuntimeConfig {
        RuntimeConfig::read(|name| match name {
            "DATABASE_URL" => Some("postgres://nginx:fixture-only@remote.invalid:6543/foreign".into()),
            "AUTH_SECRET" => Some("publisher-source-fixture".into()),
            "API_DB_POOL_SIZE" => Some("64".into()),
            _ => None,
        }).unwrap()
    }

    #[tokio::test]
    async fn private_boot_source_uses_the_shared_bounded_pool_without_connecting() {
        let config = config();
        let private = PrivatePg {
            socket_dir: "/fixture/private-socket".into(), port: 50433,
            dbname: "nginx_web_ui".into(), role: "nginx".into(), major: 17,
        };
        let pool = PgSource::Private(&private).pool(&config).unwrap();
        assert_eq!(pool.status().max_size, 2);
        assert_eq!(pool.status().size, 0);
        let mut invalid = private;
        invalid.role = "postgres".into();
        assert_eq!(PgSource::Private(&invalid).pool(&config).err().unwrap().detail.as_deref(), Some("private_pool"));
    }

    #[tokio::test]
    async fn existing_boot_pool_is_reused_instead_of_rebuilt_from_runtime_url() {
        let mut config = config();
        let manager = deadpool_postgres::Manager::new(
            "postgres://fixture@127.0.0.1/fixture".parse().unwrap(), tokio_postgres::NoTls,
        );
        let existing = deadpool_postgres::Pool::builder(manager).max_size(3).build().unwrap();
        config.database_url = "not a PostgreSQL URL".into();
        let selected = PgSource::Pool(&existing).pool(&config).unwrap();
        selected.resize(1);
        assert_eq!(existing.status().max_size, 1);
        assert_eq!(selected.status().size, 0);
    }
}
