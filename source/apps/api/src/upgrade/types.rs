use serde::{Deserialize, Serialize};
use std::{ffi::OsStr, os::fd::OwnedFd, path::{Path, PathBuf}};

#[derive(Debug, Clone, thiserror::Error)]
#[error("{code}: {message}")]
pub struct StepError {
    pub code: &'static str,
    pub message: &'static str,
    pub detail: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorClass { Transient, Deterministic }
pub fn class_of(code: &str) -> ErrorClass {
    match code {
        "storage_busy" | "insufficient_space" | "insufficient_inodes" | "reserve_exhausted"
        | "journal_write_failed" | "interrupted" | "activation_timeout" | "custody_unavailable" | "io_error" => ErrorClass::Transient,
        _ => ErrorClass::Deterministic,
    }
}
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome { Upgrading, Pg17, RefusedPreflight, RefusedFresh, RefusedPrivilege,
    Restored, RestoreIncomplete, ActivationFailed, Postponed }
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum Branch { Upgrade, Fresh, Ready }
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum KeyBinding { Unverified, Verified { #[serde(rename="pairId")] pair_id: String } }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeySource { Canonical, Staged { generation: String } }
pub(crate) fn canonical_generation(value:&str)->bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id|id.hyphenated().to_string()==value)
}
impl KeySource {
    pub fn path(&self, app_root: &Path, trusted_startup_override: Option<&OsStr>) -> PathBuf {
        match self {
            Self::Canonical => crate::db_upgrade::paths::resolve_data_key_file(app_root, trusted_startup_override),
            Self::Staged { generation } => {
                // Journal readers validate this internal UUID before constructing a staged source.
                assert!(canonical_generation(generation), "invalid staged data-key generation");
                app_root.join(".nwui-auto/gen").join(generation).join("data_key")
            }
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StorageSelection {
    pub version: u8,
    pub epoch: u64,
    pub outcome: Outcome,
    pub branch: Branch,
    pub pgdata: PathBuf,
    pub backup_id: Option<String>,
    pub schema_profile: String,
    pub key_binding: KeyBinding,
}
pub fn read_selection() -> Result<StorageSelection, StepError> {
    read_selection_at(std::path::Path::new("/run/nwui-storage.json"))
}
fn selection_error() -> StepError {
    StepError { code: "pair_mismatch", message: "저장소 선택 정보를 확인하지 못했습니다.", detail: None }
}
fn validate_selection(sel: &StorageSelection) -> Result<(), StepError> {
    if sel.version != 1 || sel.epoch == 0 || sel.schema_profile.is_empty() || sel.schema_profile.len() > 128
        || sel.backup_id.as_ref().is_some_and(|id| uuid::Uuid::parse_str(id).is_err())
        || matches!(&sel.key_binding, KeyBinding::Verified { pair_id } if uuid::Uuid::parse_str(pair_id).is_err())
        || sel.outcome == Outcome::Pg17 && sel.pgdata != std::path::Path::new(crate::db_upgrade::paths::PGDATA) {
        return Err(selection_error());
    }
    Ok(())
}
pub(crate) fn read_selection_at(path: &std::path::Path) -> Result<StorageSelection, StepError> {
    use std::{fs::OpenOptions, io::Read, os::unix::fs::{MetadataExt, OpenOptionsExt}};
    crate::db_upgrade::files::real_path(path).map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { StepError { code: "postponed", message: "저장소 준비가 끝나지 않았습니다.", detail: None } } else { selection_error() })?;
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).map_err(|_| selection_error())?;
    let meta = file.metadata().map_err(|_| selection_error())?;
    if !meta.is_file() || meta.uid() != 0 || meta.mode() & 0o7777 != 0o644 || meta.nlink() != 1 || meta.len() == 0 || meta.len() > 65536 { return Err(selection_error()); }
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes).map_err(|_| selection_error())?;
    if bytes.len() > 65536 { return Err(selection_error()); }
    let sel: StorageSelection = serde_json::from_slice(&bytes).map_err(|_| selection_error())?;
    validate_selection(&sel)?;
    Ok(sel)
}
pub(crate) fn write_selection_at(path: &std::path::Path, sel: &StorageSelection) -> Result<(), StepError> {
    use std::{fs::{File, OpenOptions}, io::Write, os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt}};
    validate_selection(sel)?;
    let bytes = serde_json::to_vec(sel).map_err(|_| selection_error())?;
    if bytes.len() > 65536 { return Err(selection_error()); }
    let parent = path.parent().ok_or_else(selection_error)?;
    crate::db_upgrade::files::real_path(parent).map_err(|_| selection_error())?;
    let meta = std::fs::symlink_metadata(parent).map_err(|_| selection_error())?;
    if !meta.is_dir() || meta.uid() != 0 || meta.mode() & 0o022 != 0 { return Err(selection_error()); }
    let tmp = parent.join(format!(".nwui-storage-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).custom_flags(libc::O_NOFOLLOW).mode(0o644).open(&tmp)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
        file.write_all(&bytes)?; file.sync_all()?;
        std::fs::rename(&tmp, path)?; File::open(parent)?.sync_all()
    })();
    let _ = std::fs::remove_file(&tmp);
    result.map_err(|_| StepError { code: "io_error", message: "저장소 선택 정보를 기록하지 못했습니다.", detail: None })
}

pub struct StorageLocks { lock_a: OwnedFd, lock_d: Option<OwnedFd> }
impl StorageLocks {
    pub fn acquire(db: &std::path::Path, app: &std::path::Path) -> Result<Self, StepError> {
        use crate::db_upgrade::{engine, layout};
        let map = |error: crate::db_upgrade::error::Error| StepError { code: error.code, message: error.message, detail: error.detail.map(str::to_owned) };
        let roots = layout::Roots { db: db.into(), app: app.into(), architecture: engine::architecture().map_err(map)?.into() };
        layout::validate_roots(&roots).map_err(map)?;
        let held = layout::lock_roots(&roots).map_err(map)?;
        layout::validate_roots(&roots).map_err(map)?;
        let mut files = held.into_files();
        let app = files.pop().expect("lock_roots holds the app directory");
        let db = files.pop().expect("lock_roots holds the DB directory");
        Ok(Self { lock_a: app.into(), lock_d: Some(db.into()) })
    }
    /// Root-only runner adapter: custody descriptors are closed, never explicitly unlocked.
    pub(crate) fn from_custody(db:&std::path::Path,app:&std::path::Path,lock_a:OwnedFd,lock_d:Option<OwnedFd>)->Result<Self,StepError> {
        let locks=Self {lock_a,lock_d};
        if !locks.matches_app(app) || locks.lock_d.as_ref().is_some_and(|fd|!Self::same_root(fd,db)) {return Err(StepError {code:"storage_busy",message:"보관된 저장소 잠금의 대상을 확인하지 못했습니다.",detail:None});}Ok(locks)
    }
    pub(crate) fn a_fd(&self)->std::os::fd::BorrowedFd<'_> {use std::os::fd::AsFd;self.lock_a.as_fd()}
    pub(crate) fn d_fd(&self)->Option<std::os::fd::BorrowedFd<'_>> {use std::os::fd::AsFd;self.lock_d.as_ref().map(|fd|fd.as_fd())}
    /// The fdholder's D copy must already be deleted before this close.
    pub(crate) fn release_d(&mut self) {drop(self.lock_d.take());}
    pub fn holds_a(&self) -> bool { use std::os::fd::AsRawFd; self.lock_a.as_raw_fd() >= 0 }
    pub fn holds_d(&self) -> bool { self.lock_d.is_some() }
    pub(crate) fn db_entry_exists(&self, name: &std::ffi::CStr) -> std::io::Result<bool> {
        use std::os::fd::AsRawFd;
        let fd=self.lock_d.as_ref().ok_or_else(|| std::io::Error::other("DB lock not held"))?;
        let mut metadata=std::mem::MaybeUninit::<libc::stat>::uninit();
        let result=unsafe { libc::fstatat(fd.as_raw_fd(), name.as_ptr(), metadata.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) };
        if result==0 { return Ok(true); }
        let error=std::io::Error::last_os_error();
        if error.kind()==std::io::ErrorKind::NotFound { Ok(false) } else { Err(error) }
    }
    pub(crate) fn matches_app(&self, app: &std::path::Path) -> bool { Self::same_root(&self.lock_a, app) }
    fn same_root(fd: &OwnedFd, path: &std::path::Path) -> bool {
        use std::{fs::File, os::unix::fs::MetadataExt};
        let metadata = || -> std::io::Result<bool> {
            let held = File::from(fd.try_clone()?).metadata()?;
            let actual = std::fs::symlink_metadata(path)?;
            Ok(actual.is_dir() && held.dev() == actual.dev() && held.ino() == actual.ino())
        };
        metadata().unwrap_or(false)
    }
    pub fn matches_roots(&self, db: &std::path::Path, app: &std::path::Path) -> bool {
        self.matches_app(app) && self.lock_d.as_ref().is_some_and(|fd| Self::same_root(fd, db))
    }
}

#[derive(Clone, Debug)]
pub struct PrivatePg { pub socket_dir: PathBuf, pub port: u16, pub dbname: String, pub role: String, pub major: u8 }
impl PrivatePg {
    fn pool_error() -> StepError {
        StepError {code:"boot_step_failed",message:"비공개 PostgreSQL 연결 설정을 확인하지 못했습니다.",detail:Some("private_pool".into())}
    }
    fn pool_config(&self,config:&crate::config::RuntimeConfig)->Result<tokio_postgres::Config,StepError> {
        if !self.socket_dir.is_absolute() || self.port==0 || !matches!(self.major,15|17)
            || [&self.dbname,&self.role].iter().any(|value|value.is_empty() || value.contains('\0')) {return Err(Self::pool_error());}
        let credentials=config.database_url.parse::<tokio_postgres::Config>().map_err(|_|Self::pool_error())?;
        if credentials.get_user()!=Some(self.role.as_str()) {return Err(Self::pool_error());}
        // Rebuild rather than append a host: no URL hostaddr, port, database or startup options survive.
        let mut pg=tokio_postgres::Config::new();
        pg.host_path(&self.socket_dir).port(self.port).dbname(&self.dbname).user(&self.role)
            .connect_timeout(std::time::Duration::from_secs(5)).options("-c search_path=public");
        if let Some(password)=credentials.get_password() {pg.password(password);}
        Ok(pg)
    }
    pub fn pool(&self,config:&crate::config::RuntimeConfig)->Result<deadpool_postgres::Pool,StepError> {
        use deadpool_postgres::{Manager,ManagerConfig,Pool,RecyclingMethod,Runtime};
        use std::time::Duration;
        let manager=Manager::from_config(self.pool_config(config)?,tokio_postgres::NoTls,ManagerConfig {recycling_method:RecyclingMethod::Fast});
        Pool::builder(manager).max_size(2).runtime(Runtime::Tokio1)
            .wait_timeout(Some(Duration::from_secs(5))).create_timeout(Some(Duration::from_secs(5)))
            .recycle_timeout(Some(Duration::from_secs(2))).build().map_err(|_|Self::pool_error())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UpgradeStatus {
    pub version: u8, pub updated_at: String, pub outcome: Outcome, pub branch: Branch,
    pub step: StatusStep, pub failure: Option<StatusFailure>, pub space: Option<StatusSpace>,
    pub locations: Vec<String>, pub backup: Option<RetainedCopy>, pub discarded: Vec<RetainedCopy>,
    pub cleanup_pending: bool, pub retry: StatusRetry,
    #[serde(default,skip_serializing_if="Vec::is_empty")]
    pub warnings: Vec<StorageWarning>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StorageWarning {pub code:String,pub message:String,pub scopes:Vec<String>}
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StatusStep { pub id: super::Step, pub index: u32, pub total: u32, pub started_at: String, pub bytes_done: u64, pub bytes_total: u64 }
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StatusFailure { pub code: String, pub class: ErrorClass, pub message: String, pub next_action: String }
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StatusSpace { pub required_bytes: u64, pub available_bytes: u64, pub inodes_required: u64, pub inodes_available: u64 }
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RetainedCopy { pub id: String, pub bytes: u64, pub remove_command: String }
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StatusRetry { pub next_eligible_at: Option<String>, pub reason: Option<String> }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn n0_selection_and_status_keep_the_frozen_wire_shapes() {
        let value = json!({"version":1,"epoch":7,"outcome":"pg17","branch":"ready",
            "pgdata":"/var/lib/postgresql/data/17/data","backupId":null,
            "schemaProfile":"2.0.0","keyBinding":{"state":"unverified"}});
        let selection: StorageSelection = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(selection).unwrap(), value);
        let value = json!({"version":1,"updatedAt":"2026-10-27T00:00:00Z",
            "outcome":"upgrading","branch":"upgrade",
            "step":{"id":"p1-backup","index":3,"total":11,"startedAt":"2026-10-27T00:00:00Z","bytesDone":0,"bytesTotal":0},
            "failure":null,"space":null,"locations":[],"backup":null,"discarded":[],
            "cleanupPending":false,"retry":{"nextEligibleAt":null,"reason":null}});
        let status: UpgradeStatus = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(status).unwrap(), value);
        assert!(serde_json::from_value::<Outcome>(json!("restored-ish")).is_err());
    }

    #[test]
    fn n0_key_source_keeps_canonical_and_staged_paths_distinct() {
        let root=Path::new("/isolated/app");
        assert_eq!(KeySource::Canonical.path(root, None), root.join("secrets/data_key"));
        assert_eq!(KeySource::Canonical.path(root, Some(OsStr::new("/mounted/key"))), Path::new("/mounted/key"));
        let generation="3d521ab4-34ad-44e8-b7eb-882c0feff693";
        let staged=KeySource::Staged { generation:generation.into() };
        assert_eq!(staged.path(root, Some(OsStr::new("/mounted/key"))), root.join(".nwui-auto/gen").join(generation).join("data_key"));
    }
    #[test]
    #[should_panic(expected="invalid staged data-key generation")]
    fn n0_key_source_never_returns_a_traversal_path() {
        KeySource::Staged { generation:"../../outside".into() }.path(Path::new("/isolated/app"), None);
    }

    #[test]
    fn n0_error_classes_keep_the_frozen_transient_list() {
        for code in ["storage_busy","insufficient_space","insufficient_inodes","reserve_exhausted",
            "journal_write_failed","interrupted","activation_timeout","custody_unavailable","io_error"] {
            assert_eq!(class_of(code), ErrorClass::Transient, "{code}");
        }
        for code in ["unsupported_layout","pair_mismatch","key_missing","storage_read_only",
            "log_dir_unsafe","isolation_unavailable","collation_unsupported","ledger_unknown",
            "object_unsupported","pg_upgrade_check_failed","rehearsal_failed","migration_failed","database_newer",
            "verification_failed","backup_verification_failed","restore_incomplete",
            "privilege_prepare_failed","key_handle_failed","auth_log_failed","render_failed",
            "nginx_test_failed","publish_failed","api_port_conflict","fresh_failed","postponed","not_implemented","guard_failed","source_too_old"] {
            assert_eq!(class_of(code), ErrorClass::Deterministic, "{code}");
        }
    }

    #[test]
    fn w1_missing_selection_refuses_without_reading_a_real_runtime_path() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(read_selection_at(&t.path().join("missing")).unwrap_err().code, "postponed");
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;
    use crate::db_upgrade::{files, fresh, key_binding, layout, pair::Pair};
    use std::{fs, os::unix::fs::PermissionsExt};

    fn fixture() -> (tempfile::TempDir, layout::Roots, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let roots = layout::Roots { db:t.path().join("db"), app:t.path().join("app"),
            architecture:crate::db_upgrade::engine::architecture().unwrap().into() };
        let env = t.path().join("env");
        for p in [&roots.db, &roots.app, &env] {
            fs::create_dir(p).unwrap(); fs::set_permissions(p, fs::Permissions::from_mode(0o700)).unwrap();
        }
        (t, roots, env)
    }

    #[test]
    fn w1_locked_prepare_and_secrets_do_not_reacquire_or_accept_other_roots() {
        let (_t, roots, env) = fixture();
        let locks = StorageLocks::acquire(&roots.db, &roots.app).unwrap();
        assert!(locks.holds_a() && locks.holds_d());
        assert_eq!(layout::prepare(&roots, &|_|Ok("123456".into())).unwrap_err().code, "storage_busy");
        layout::prepare_locked(&locks, &roots, &|_|Ok("123456".into())).unwrap();
        fresh::secrets_locked(&locks, &roots, &fresh::Inputs::default(), &env).unwrap();
        let before = files::snapshot(&roots.app).unwrap();
        fresh::secrets_locked(&locks, &roots, &fresh::Inputs::default(), &env).unwrap();
        assert_eq!(files::snapshot(&roots.app).unwrap(), before);
        let (_other, other, other_env) = fixture();
        assert_eq!(layout::prepare_locked(&locks, &other, &|_|Ok("123456".into())).unwrap_err().code, "storage_busy");
        assert_eq!(fresh::secrets_locked(&locks, &other, &fresh::Inputs::default(), &other_env).unwrap_err().code, "storage_busy");
        assert!(fs::read_dir(other.db).unwrap().next().is_none());
        assert!(fs::read_dir(other.app).unwrap().next().is_none());
    }

    #[test]
    fn w1_locked_fresh_classifier_admits_only_its_valid_journal_and_resumes_secrets() {
        let (_t,roots,env)=fixture();let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();
        let _journal=crate::upgrade::Journal::begin(&roots.app,&locks,Branch::Fresh,crate::upgrade::journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();
        assert!(matches!(layout::classify_locked(&locks,&roots,&|_|Ok("123456".into())).unwrap(),layout::Decision::Empty));
        assert!(layout::classify(&roots,&|_|Ok("123456".into())).is_err());
        fresh::secrets_locked(&locks,&roots,&fresh::Inputs::default(),&env).unwrap();
        let layout::Decision::Fresh(pair)=layout::classify_locked(&locks,&roots,&|_|Ok("123456".into())).unwrap()else{panic!("fresh intent was lost")};assert_eq!(pair.fresh_step.as_deref(),Some("initdb"));
        let before=files::snapshot(&roots.app).unwrap();fresh::secrets_locked(&locks,&roots,&fresh::Inputs::default(),&env).unwrap();assert_eq!(files::snapshot(&roots.app).unwrap(),before);
    }
    #[test]
    fn w1_locked_classifier_never_treats_a_missing_ready_database_or_bad_journal_as_fresh() {
        for branch in [Branch::Ready,Branch::Upgrade,Branch::Fresh] {
            let (_t,roots,_env)=fixture();let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();
            let _journal={let fixture_journal=crate::upgrade::Journal::begin(&roots.app,&locks,branch,crate::upgrade::journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};
            if branch==Branch::Fresh {fs::write(roots.app.join(".nwui-auto/journal-v1.json"),b"{}").unwrap();}
            let before=files::snapshot(&roots.app).unwrap();assert!(layout::classify_locked(&locks,&roots,&|_|Ok("123456".into())).is_err());assert_eq!(files::snapshot(&roots.app).unwrap(),before);assert!(fs::read_dir(&roots.db).unwrap().next().is_none());
        }
    }
    #[test]
    fn w1_custody_copy_keeps_the_app_lock_after_the_runner_handle_closes() {
        let (_t, roots, _env) = fixture();
        let locks = StorageLocks::acquire(&roots.db, &roots.app).unwrap();
        let custody = locks.lock_a.try_clone().unwrap();
        drop(locks);
        assert_eq!(StorageLocks::acquire(&roots.db, &roots.app).err().unwrap().code, "storage_busy");
        drop(custody);
        // Concurrent test forks may retain CLOEXEC copies until their exec completes.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if StorageLocks::acquire(&roots.db, &roots.app).is_ok() { break; }
            assert!(std::time::Instant::now() < deadline, "custody never released the app lock");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[tokio::test]
    async fn w1_locked_key_binding_updates_only_the_held_pair_without_relocking() {
        let (_t, roots, env) = fixture();
        let locks = StorageLocks::acquire(&roots.db, &roots.app).unwrap();
        fresh::secrets_locked(&locks, &roots, &fresh::Inputs::default(), &env).unwrap();
        fs::create_dir_all(roots.db.join("17/data/global")).unwrap();
        fs::write(roots.db.join("17/data/PG_VERSION"), "17").unwrap();
        fs::write(roots.db.join("17/data/global/pg_control"), "fixture control").unwrap();
        let mut pair: Pair = files::read_json(&roots.app.join(".nwui/pair-v1.json")).unwrap();
        pair.ready = true; pair.fresh_step = None; pair.system_identifier = Some("123456".into());
        for root in [&roots.db, &roots.app] { files::atomic_json(&root.join(".nwui/pair-v1.json"), &pair).unwrap(); }
        let key = crate::crypto::SecretBox::load_or_create(&roots.app.join("secrets/data_key")).unwrap();
        let before = fs::read(roots.app.join("secrets/data_key")).unwrap();
        assert_eq!(key_binding::mark_key_required(&roots, &|_|Ok("123456".into()), &key).unwrap_err().code, "storage_busy");
        let _journal=crate::upgrade::Journal::begin(&roots.app,&locks,Branch::Ready,crate::upgrade::journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();
        key_binding::mark_key_required_locked(&locks, &roots, &|_|Ok("123456".into()), &key).unwrap();
        let db: Pair = files::read_json(&roots.db.join(".nwui/pair-v1.json")).unwrap();
        let app: Pair = files::read_json(&roots.app.join(".nwui/pair-v1.json")).unwrap();
        assert_eq!(db, app); assert!(db.encryption_key_required);
        assert_eq!(fs::read(roots.app.join("secrets/data_key")).unwrap(), before);
        let (_other, other, _) = fixture();
        assert_eq!(fresh::provision_locked(&locks, &other).await.unwrap_err().code, "storage_busy");
        assert!(fs::read_dir(other.db).unwrap().next().is_none());
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use std::{fs, os::unix::fs::{PermissionsExt, symlink}};
    fn selection() -> StorageSelection {
        StorageSelection { version:1, epoch:7, outcome:Outcome::Pg17, branch:Branch::Ready,
            pgdata:crate::db_upgrade::paths::PGDATA.into(), backup_id:None, schema_profile:"2.0.0".into(),
            key_binding:KeyBinding::Verified { pair_id:uuid::Uuid::new_v4().to_string() } }
    }
    #[test]
    fn w1_selection_is_published_with_the_trusted_mode_and_validated_on_read() {
        let t=tempfile::tempdir().unwrap(); let path=t.path().join("storage.json");
        write_selection_at(&path, &selection()).unwrap();
        assert_eq!(read_selection_at(&path).unwrap().outcome, Outcome::Pg17);
        let before=fs::read(&path).unwrap();
        let mut invalid=selection(); invalid.pgdata="/tmp/other".into();
        assert!(write_selection_at(&path, &invalid).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(t.path()).unwrap().count(), 1);
    }
    #[test]
    fn w1_selection_refuses_unsafe_files_and_unknown_descriptors() {
        for bad in ["mode","owner","hardlink","symlink","directory","too-large","empty","version","epoch","pgdata","pair","outcome"] {
            let t=tempfile::tempdir().unwrap(); let path=t.path().join("storage.json");
            write_selection_at(&path, &selection()).unwrap();
            match bad {
                "mode" => fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap(),
                "owner" => std::os::unix::fs::chown(&path, Some(1000), None).unwrap(),
                "hardlink" => fs::hard_link(&path, t.path().join("alias")).unwrap(),
                "symlink" => { let original=t.path().join("original"); fs::rename(&path, &original).unwrap(); symlink(original, &path).unwrap(); },
                "directory" => { fs::remove_file(&path).unwrap(); fs::create_dir(&path).unwrap(); },
                "too-large" => fs::write(&path, vec![b' ';65537]).unwrap(),
                "empty" => fs::write(&path, "").unwrap(),
                _ => { let mut value=serde_json::to_value(selection()).unwrap();
                    match bad {
                        "version" => value["version"]=serde_json::json!(2),
                        "epoch" => value["epoch"]=serde_json::json!(0),
                        "pgdata" => value["pgdata"]=serde_json::json!("/tmp/foreign"),
                        "pair" => value["keyBinding"]["pairId"]=serde_json::json!("foreign"),
                        _ => value["outcome"]=serde_json::json!("unknown"),
                    }
                    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
                }
            }
            assert!(read_selection_at(&path).is_err(), "{bad}");
        }
    }
}

#[cfg(test)]
mod key_override_tests {
    use super::*;
    use crate::{crypto::SecretBox, db_upgrade::{files, fresh, key_binding, layout, migration, pair::Pair}};
    use std::{fs, os::unix::fs::PermissionsExt};
    #[test]
    fn w1_empty_raw_key_override_refuses_before_storage_mutation() {
        const CHILD:&str="NWUI_W1_EMPTY_KEY_OVERRIDE_FIXTURE";
        if let Some(root)=std::env::var_os(CHILD) {
            let root=PathBuf::from(root);
            let roots=layout::Roots {db:root.join("db"),app:root.join("app"),architecture:"arm64".into()};
            assert_eq!(crate::nonroot::startup_value("DATA_KEY_FILE").as_deref(),Some(""));
            assert_eq!(crate::db_upgrade::paths::startup_data_key_file(&roots.app).unwrap_err().code,"key_path_invalid");
            assert_eq!(fresh::secrets(&roots,&fresh::Inputs::default(),&root.join("env")).unwrap_err().code,"key_path_invalid");
            assert_eq!(migration::key(&roots.app).err().unwrap().code,"key_path_invalid");
            for path in [&roots.db,&roots.app,&root.join("env")] {assert_eq!(fs::read_dir(path).unwrap().count(),0);}
            return;
        }
        let t=tempfile::tempdir().unwrap();
        for name in ["db","app","env"] {let path=t.path().join(name);fs::create_dir(&path).unwrap();}
        let output=std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","upgrade::types::key_override_tests::w1_empty_raw_key_override_refuses_before_storage_mutation","--nocapture"])
            .env_clear().env(CHILD,t.path()).env("DATA_KEY_FILE","").output().unwrap();
        assert!(output.status.success(),"child failed: {}",String::from_utf8_lossy(&output.stderr));
    }
    #[test]
    fn w1_all_key_readers_honor_the_same_startup_override_without_a_default_key() {
        const CHILD:&str="NWUI_W1_KEY_OVERRIDE_FIXTURE";
        if let Some(root)=std::env::var_os(CHILD) {
            let root=PathBuf::from(root);
            let roots=layout::Roots { db:root.join("db"), app:root.join("app"), architecture:crate::db_upgrade::engine::architecture().unwrap().into() };
            fresh::secrets(&roots,&fresh::Inputs::default(),&root.join("env")).unwrap();
            fs::create_dir_all(roots.db.join("17/data/global")).unwrap();
            fs::write(roots.db.join("17/data/PG_VERSION"),"17").unwrap();
            fs::write(roots.db.join("17/data/global/pg_control"),"fixture control").unwrap();
            let mut pair:Pair=files::read_json(&roots.app.join(".nwui/pair-v1.json")).unwrap();
            pair.ready=true;pair.fresh_step=None;pair.system_identifier=Some("123456".into());
            for p in [&roots.db,&roots.app] { files::atomic_json(&p.join(".nwui/pair-v1.json"),&pair).unwrap(); }
            let key=migration::key(&roots.app).unwrap().expect("the configured key must not be treated as absent");
            key_binding::mark_key_required(&roots,&|_|Ok("123456".into()),&key).unwrap();
            let mut interrupted:Pair=files::read_json(&roots.app.join(".nwui/pair-v1.json")).unwrap();
            interrupted.encryption_key_required=false;
            files::atomic_json(&roots.app.join(".nwui/pair-v1.json"),&interrupted).unwrap();
            let layout::Decision::Ready(p)=layout::classify(&roots,&|_|Ok("123456".into())).unwrap() else {panic!("ready pair expected")};
            assert!(p.encryption_key_required);
            key_binding::mark_key_required(&roots,&|_|Ok("123456".into()),&key).unwrap();
            assert!(!roots.app.join("secrets/data_key").exists());
            return;
        }
        let t=tempfile::tempdir().unwrap();
        for name in ["db","app","env"] {let p=t.path().join(name);fs::create_dir(&p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let key=t.path().join("custom.key");SecretBox::load_or_create(&key).unwrap();fs::set_permissions(&key,fs::Permissions::from_mode(0o444)).unwrap();
        use sha2::{Digest,Sha256};let before=Sha256::digest(fs::read(&key).unwrap());
        // A child isolates the trusted startup environment from parallel tests.
        let output=std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","upgrade::types::key_override_tests::w1_all_key_readers_honor_the_same_startup_override_without_a_default_key","--nocapture"])
            .env_clear().env(CHILD,t.path()).env("DATA_KEY_FILE",&key).output().unwrap();
        assert!(output.status.success(),"child failed: {}",String::from_utf8_lossy(&output.stderr));
        assert_eq!(Sha256::digest(fs::read(&key).unwrap()),before);
        let pair:Pair=files::read_json(&t.path().join("app/.nwui/pair-v1.json")).unwrap();
        assert!(pair.encryption_key_required);assert!(!t.path().join("app/secrets/data_key").exists());
    }
}

#[cfg(test)]
mod private_pool_tests {
    use super::*;
    fn config(url:&str)->crate::config::RuntimeConfig {
        crate::config::RuntimeConfig::read(|name|match name {"DATABASE_URL"=>Some(url.into()),"AUTH_SECRET"=>Some("private-pool-fixture-only".into()),_=>None}).unwrap()
    }
    fn pg()->PrivatePg {PrivatePg {socket_dir:"/fixture/private-socket".into(),port:50433,dbname:"nginx_web_ui".into(),role:"nginx".into(),major:17}}
    #[test]
    fn w1_private_pool_config_uses_only_the_private_socket_and_application_credentials() {
        use tokio_postgres::config::Host;
        let cfg=config("postgres://nginx:fixture%3Apassword@remote.invalid:6543/foreign?hostaddr=203.0.113.20&options=-c%20search_path%3Dforeign");
        let plan=pg().pool_config(&cfg).unwrap();
        assert_eq!(plan.get_hosts(),&[Host::Unix(PathBuf::from("/fixture/private-socket"))]);
        assert!(plan.get_hostaddrs().is_empty());assert_eq!(plan.get_ports(),&[50433]);
        assert_eq!(plan.get_dbname(),Some("nginx_web_ui"));assert_eq!(plan.get_user(),Some("nginx"));
        assert_eq!(plan.get_password(),Some(b"fixture:password".as_slice()));
        assert_eq!(plan.get_options(),Some("-c search_path=public"));
    }
    #[tokio::test]
    async fn w1_private_pool_is_bounded_to_two_and_does_not_connect_during_construction() {
        let mut cfg=config("postgres://nginx:fixture@remote.invalid:6543/foreign");cfg.database_pool_size=64;
        let pool=pg().pool(&cfg).unwrap();assert_eq!(pool.status().max_size,2);assert_eq!(pool.status().size,0);
    }
    #[tokio::test]
    async fn w1_private_connect_never_uses_the_url_tcp_host_and_sanitizes_connection_failure() {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let port=listener.local_addr().unwrap().port();
        let cfg=config(&format!("postgres://nginx:fixture-secret@127.0.0.1:{port}/foreign"));let t=tempfile::tempdir().unwrap();let mut private=pg();private.socket_dir=t.path().to_owned();
        let pool=private.pool(&cfg).unwrap();let error=pool.get().await.err().unwrap();assert!(!format!("{error:?}").contains("fixture-secret"));
        assert!(tokio::time::timeout(std::time::Duration::from_millis(30),listener.accept()).await.is_err());
    }
    #[test]
    fn w1_private_pool_rejects_nonprivate_or_mismatched_settings_without_secret_detail() {
        let cfg=config("postgres://nginx:fixture-secret@remote.invalid/foreign");
        for case in ["relative","zero-port","major","empty-role","empty-db","role-mismatch"] {
            let mut private=pg();match case {"relative"=>private.socket_dir="127.0.0.1".into(),"zero-port"=>private.port=0,"major"=>private.major=18,"empty-role"=>private.role.clear(),"empty-db"=>private.dbname.clear(),_=>private.role="postgres".into()};
            let error=private.pool_config(&cfg).unwrap_err();assert_eq!(error.code,"boot_step_failed");assert_eq!(error.detail.as_deref(),Some("private_pool"));
            assert!(!format!("{error:?}").contains("fixture-secret"));
        }
        let mut invalid=cfg;invalid.database_url="invalid connection string containing fixture-secret".into();
        let error=pg().pool_config(&invalid).unwrap_err();assert_eq!(error.detail.as_deref(),Some("private_pool"));assert!(!format!("{error:?}").contains("fixture-secret"));
    }
}

#[cfg(test)]
mod serde_contract_tests {
    use super::*;
    use super::super::journal::{Step,StepState,MovePhase,Handoff,JournalV1};
    use serde::de::DeserializeOwned;
    use serde_json::{json,Value};
    fn literal<T:DeserializeOwned+Serialize>(value:Value) {
        let parsed:T=serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(),value);
    }
    #[test]
    fn n0_serde_contract_all_unit_variants_use_the_frozen_spellings() {
        for v in ["upgrading","pg17","refused-preflight","refused-fresh","refused-privilege","restored","restore-incomplete","activation-failed","postponed"] {literal::<Outcome>(json!(v));}
        for v in ["upgrade","fresh","ready"] {literal::<Branch>(json!(v));}
        for v in ["transient","deterministic"] {literal::<ErrorClass>(json!(v));}
        for v in ["p0","p0b","f1","y1","y2","y3","p1-backup","p2-preflight","p3-convert","p4-target","s4-prepare","p5-publish","p5-guard","p5-commit","s4-activate","handoff","phase2-monitor","p6-cleanup","r-restore","cmd-restore-pg15","cmd-remove-backup","cmd-remove-discarded"] {literal::<Step>(json!(v));}
        for v in ["set-aside","restore","target","guard","cleanup"] {literal::<MovePhase>(json!(v));}
        for v in ["none","intent","done"] {literal::<Handoff>(json!(v));}
    }
    #[test]
    fn n0_serde_contract_every_key_binding_variant_uses_pair_id_camel_case() {
        literal::<KeyBinding>(json!({"state":"unverified"}));
        literal::<KeyBinding>(json!({"state":"verified","pairId":"3d521ab4-34ad-44e8-b7eb-882c0feff693"}));
        assert!(serde_json::from_value::<KeyBinding>(json!({"state":"verified","pair_id":"wrong-wire"})).is_err());
    }
    #[test]
    fn n0_serde_contract_selection_contains_the_literal_verified_binding() {
        for binding in [json!({"state":"unverified"}),json!({"state":"verified","pairId":"3d521ab4-34ad-44e8-b7eb-882c0feff693"})] {
            literal::<StorageSelection>(json!({"version":1,"epoch":7,"outcome":"pg17","branch":"ready","pgdata":"/var/lib/postgresql/data/17/data","backupId":"3d521ab4-34ad-44e8-b7eb-882c0feff693","schemaProfile":"2.0.0","keyBinding":binding}));
        }
    }
    #[test]
    fn n0_serde_contract_journal_covers_every_struct_variant_and_nested_field() {
        for state in [json!({"kind":"intent"}),json!({"kind":"done"}),json!({"kind":"failed","code":"io_error"})] {
            literal::<StepState>(state.clone());
            literal::<JournalV1>(json!({"version":1,"attemptId":"3d521ab4-34ad-44e8-b7eb-882c0feff693","branch":"upgrade","candidate":{"revision":"0".repeat(40),"pinsSha256":"1".repeat(64),"migrationsSha256":"2".repeat(64)},"records":[{"seq":1,"step":"p1-backup","state":state,"at":"2026-10-27T00:00:00Z","data":null}],"moves":[{"seq":7,"phase":"restore","entry":"base","from":"BK","to":"L","state":state}],"handoff":"none","outcome":"upgrading","retry":{"consumedLabels":["rehearsal"],"interruptions":0,"backoffUntil":null}}));
        }
    }
    #[test]
    fn n0_serde_contract_status_covers_populated_and_null_nested_fields() {
        for class in ["transient","deterministic"] {
            literal::<UpgradeStatus>(json!({"version":1,"updatedAt":"2026-10-27T00:00:00Z","outcome":"upgrading","branch":"upgrade","step":{"id":"p1-backup","index":3,"total":11,"startedAt":"2026-10-27T00:00:00Z","bytesDone":0,"bytesTotal":0},"failure":{"code":"insufficient_space","class":class,"message":"공간을 확인하세요.","nextAction":"여유 공간을 확보하세요."},"space":{"requiredBytes":0,"availableBytes":0,"inodesRequired":0,"inodesAvailable":0},"locations":["/var/lib/postgresql/data/.nwui-pg15-backup/fixture"],"backup":{"id":"fixture","bytes":0,"removeCommand":"nwui-db-upgrade remove-pg15-backup --confirm fixture"},"discarded":[{"id":"fixture","bytes":0,"removeCommand":"nwui-db-upgrade remove-discarded --confirm fixture"}],"cleanupPending":false,"retry":{"nextEligibleAt":"2026-10-27T00:00:00Z","reason":"interrupted"}}));
        }
        literal::<UpgradeStatus>(json!({"version":1,"updatedAt":"2026-10-27T00:00:00Z","outcome":"pg17","branch":"ready","step":{"id":"p6-cleanup","index":11,"total":11,"startedAt":"2026-10-27T00:00:00Z","bytesDone":0,"bytesTotal":0},"failure":null,"space":null,"locations":[],"backup":null,"discarded":[],"cleanupPending":false,"retry":{"nextEligibleAt":null,"reason":null}}));
    }
}
