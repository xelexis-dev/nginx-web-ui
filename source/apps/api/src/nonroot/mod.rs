//! Root preparation precedes the storage commit; activation alone publishes live files.
mod record;
use record::{LogMode,Receipt,PrepareRecord,latest_record};
pub use record::prepared_from_journal;
#[cfg(test)]
pub(crate) use record::fixture_prepare_records;
pub mod access_probe;
mod handles;
pub mod pre_exec;
pub mod selection;
mod privilege;
mod startup;
mod trusted;
pub(crate) use trusted::{Admitted,admit_nginx};
pub use trusted::admit_guard;
pub use startup::startup_value;
pub(crate) use startup::{envdir_value,runner_value};
use crate::{config::RuntimeConfig, db_upgrade::key_file::{self, KeyPolicy, ValidatedKeyFile}, publisher, upgrade::{Journal, KeyBinding, Outcome, PrivatePg, Step, StepError, StepState, StorageLocks, StorageSelection}};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io, os::{fd::AsRawFd, unix::fs::MetadataExt}, path::{Path, PathBuf}};

pub const PRIVILEGE_SPLIT: bool = true;

const RUNTIME_PRIVATE: &str = "/run/nwui-api-private";
const RUNTIME_KEY: &str = "/run/nwui-api-private/data_key";
const RECEIPT: &str = "/run/nwui-api-private/receipt.json";
const RUNTIME_TMP: &str = "/run/nwui-api";
const PUBLISHER_RUNTIME: &str = "/run/nwui-publisher";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Prepared { pub payload: Option<publisher::boot::StagedPayload>, pub key_receipt: Option<KeyReceipt> }
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyReceipt { pub selection_epoch: u64, pub key_sha256: String }

impl Receipt {
    fn new(sel: &StorageSelection, key: &[u8], attempt_id: &str) -> Result<Self,StepError> {
        Self::with_identity(sel,key,attempt_id,crate::upgrade::boot_identity::current())
    }
    fn with_identity(sel: &StorageSelection, key: &[u8], attempt_id: &str, identity: Result<String,StepError>) -> Result<Self,StepError> {
        let boot_identity=identity.map_err(|_|error("privilege_prepare_failed","컨테이너 시작 정보를 확인하지 못했습니다."))?;
        let KeyBinding::Verified { pair_id } = &sel.key_binding else { return Err(error("key_handle_failed","저장소 키의 결합을 확인하지 못했습니다.")); };
        Ok(Self { version: 1, key: KeyReceipt { selection_epoch: sel.epoch, key_sha256: hex::encode(Sha256::digest(key)) },
            pair_id: pair_id.clone(), pgdata: sel.pgdata.clone(), boot_identity, active: false, attempt_id: attempt_id.to_owned() })
    }
    fn validate(&self, sel: &StorageSelection, active: bool) -> io::Result<()> {
        let identity=crate::upgrade::boot_identity::current().map_err(|_|handles::invalid("container start identity unavailable"))?;
        self.validate_binding(sel,active,&identity)
    }
    fn validate_binding(&self, sel: &StorageSelection, active: bool, identity:&str) -> io::Result<()> {
        let KeyBinding::Verified { pair_id } = &sel.key_binding else { return Err(handles::invalid("storage key is not bound")); };
        if self.version != 1 || self.key.selection_epoch != sel.epoch || &self.pair_id != pair_id || self.pgdata != sel.pgdata
            || self.boot_identity != identity
            || self.active != active || (active && sel.outcome != Outcome::Pg17) {
            return Err(handles::invalid("runtime key receipt does not match the selected storage"));
        }
        Ok(())
    }
    fn validate_key(&self, key: &[u8]) -> io::Result<()> {
        if hex::encode(Sha256::digest(key)) != self.key.key_sha256 { return Err(handles::invalid("selected key changed")); }
        Ok(())
    }
    fn write(&self) -> io::Result<()> { self.write_at(Path::new(RECEIPT)) }
    fn write_at(&self, path: &Path) -> io::Result<()> {
        handles::atomic_write(path, &serde_json::to_vec(self).map_err(|_| handles::invalid("invalid runtime receipt"))?, privilege::API_GID, 0o440)
    }
    fn for_activation(existing: Option<Self>, candidate: &StorageSelection, committed: &StorageSelection, expected: &KeyReceipt, key: &[u8], attempt: &str) -> Result<Self,StepError> {
        let provisional = Self::new(candidate,key,attempt)?;
        let mut activated = Self::new(committed,key,attempt)?;
        if candidate.version != 1 || committed.version != 1 || committed.outcome != Outcome::Pg17
            || provisional.pair_id != activated.pair_id || provisional.key.selection_epoch != expected.selection_epoch
            || provisional.key.key_sha256 != expected.key_sha256 { return Err(error("key_handle_failed","활성화 대상이 준비된 키 핸들과 다릅니다.")); }
        if let Some(mut receipt) = existing {
            if receipt.attempt_id != attempt || receipt.key.key_sha256 != expected.key_sha256 { return Err(error("key_handle_failed","활성화 기록이 다른 시도 또는 키에 속합니다.")); }
            // A trusted receipt may survive /run across a host restart; validate its binding anew.
            receipt.boot_identity = activated.boot_identity.clone();
            io_error(receipt.validate_binding(if receipt.active {committed} else {candidate}, receipt.active, &activated.boot_identity),"key_handle_failed")?;
        }
        activated.active = true;
        Ok(activated)
    }
}

fn chown_then_chmod(chown: io::Result<()>, chmod: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    if let Err(error) = chown {
        if !matches!(error.raw_os_error(),Some(libc::EPERM | libc::EINVAL | libc::ENOTSUP)) { return Err(error); }
    }
    chmod()
}
fn log_metadata(path:&Path,dir: &fs::File, uid: u32, gid: u32, mode: u32,shared_default:bool) -> io::Result<()> {
    log_metadata_with(path,dir,uid,gid,mode,shared_default,|| {
        if unsafe { libc::fchmod(dir.as_raw_fd(),mode) } == -1 {return Err(io::Error::last_os_error());}
        Ok(())
    })
}
fn log_metadata_matches(path:&Path,metadata:&fs::Metadata,opened:&fs::Metadata,expected:(u32,u32,u32),shared_default:bool,kind:Option<crate::db_upgrade::shared_folder::SharedKind>)->bool{
    let(uid,gid,mode)=expected;
    metadata.is_dir()&&(metadata.dev(),metadata.ino())==(opened.dev(),opened.ino())&&metadata.mode()&0o7777==mode
        &&((metadata.uid(),metadata.gid())==(uid,gid)||shared_default&&uid==0&&path==Path::new("/var/log/nginx")&&mode==0o750
            &&(opened.uid(),opened.gid())==(0,0)&&(metadata.uid(),metadata.gid())==(0,0)
            &&kind==Some(crate::db_upgrade::shared_folder::SharedKind::OrbStackMac))
}
fn log_metadata_with(path:&Path,dir:&fs::File,uid:u32,gid:u32,mode:u32,shared_default:bool,chmod:impl FnOnce()->io::Result<()>)->io::Result<()> {
    let chown = if unsafe { libc::fchown(dir.as_raw_fd(),uid,gid) } == -1 { Err(io::Error::last_os_error()) } else {Ok(())};
    chown_then_chmod(chown, || {
        chmod()?;
        let metadata=fs::symlink_metadata(path)?;
        let opened=dir.metadata()?;
        if !log_metadata_matches(path,&metadata,&opened,(uid,gid,mode),shared_default,crate::db_upgrade::shared_folder::kind(dir)) {
            return Err(handles::invalid("log directory owner or mode was not enforced"));
        }
        dir.sync_all()
    })
}

impl LogMode {
    fn read(path: &Path) -> io::Result<Self> {
        let meta = handles::directory(path)?.metadata()?;
        Ok(Self { path: path.into(), uid: meta.uid(), gid: meta.gid(), mode: meta.mode() & 0o7777 })
    }
    fn restore(&self) -> io::Result<()> {
        let dir = handles::directory(&self.path)?;
        let current = dir.metadata()?;
        if (current.uid(), current.gid(), current.mode() & 0o7777) == (self.uid,self.gid,self.mode) { return Ok(()); }
        log_metadata(&self.path,&dir,self.uid,self.gid,self.mode,false)
    }
}
pub(crate) fn error(code: &'static str, message: &'static str) -> StepError { StepError { code, message, detail: None } }
fn io_error<T>(result: io::Result<T>, code: &'static str) -> Result<T, StepError> {
    result.map_err(|_| error(code, match code {
        "log_dir_unsafe" => "호스트 로그 폴더의 쓰기 권한을 root로 제한하세요.",
        "auth_log_failed" => "인증 로그의 안전한 쓰기 핸들을 준비하지 못했습니다.",
        "key_handle_failed" => "선택한 저장소의 암호화 키 핸들을 준비하지 못했습니다.",
        _ => "API 전용 접근 권한을 준비하지 못했습니다.",
    }))
}

fn require_verified(sel: &StorageSelection) -> Result<(), StepError> {
    if matches!(sel.key_binding, KeyBinding::Unverified) { return Err(error("key_unverified", "저장소 암호화 키의 일치 여부를 확인하지 못했습니다.")); }
    Ok(())
}

pub(super) fn config() -> anyhow::Result<RuntimeConfig> { RuntimeConfig::read(runner_value) }

pub(super) fn canonical_key(config: &RuntimeConfig) -> Result<PathBuf,StepError> {
    if !config.data_key_file.is_absolute() { return Err(error("key_path_invalid", "암호화 키 경로는 비어 있지 않은 절대 경로여야 합니다.")); }
    Ok(crate::db_upgrade::paths::resolve_data_key_file(Path::new(crate::db_upgrade::paths::APPROOT), Some(config.data_key_file.as_os_str())))
}
async fn validated_key(path: PathBuf, policy: KeyPolicy) -> Result<ValidatedKeyFile,StepError> {
    crate::control::internal_targets::blocking(move ||key_file::read_key_file(&path,policy)).await
        .map_err(|_|error("key_handle_failed", "키 파일 검증 작업을 완료하지 못했습니다."))?
        .map_err(|_|error("key_handle_failed", "선택한 저장소의 암호화 키를 검증하지 못했습니다."))
}

fn default_log_paths() -> [PathBuf;2] { ["/var/log/nginx".into(), "/var/log/nginx-web-ui".into()] }
fn publisher_directory() -> PathBuf { Path::new(crate::db_upgrade::paths::APPROOT).join(".nwui-publisher") }

fn prepare_publisher_directory(path: &Path, create: bool) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() && meta.uid() == 0 && meta.mode() & 0o7777 == 0o700 => Ok(()),
        Ok(_) => Err(handles::invalid("unsafe publisher bootstrap directory")),
        Err(error) if error.kind() == io::ErrorKind::NotFound && create => handles::ensure_directory(path,0,0,0o700).map(|_|()),
        Err(error) => Err(error),
    }
}
fn owned_bootstrap(path: &Path, created: bool) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() && meta.uid() == 0 && meta.mode() & 0o7777 == 0o700 => {},
        Ok(_) => return Err(handles::invalid("unsafe publisher bootstrap")),
        Err(error) => return Err(error),
    }
    if !created {
        match fs::symlink_metadata(path.join(".manifest.init")) {
            Ok(_) => return Err(handles::invalid("publisher bootstrap temp is not owned by this attempt")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
    }
    Ok(created)
}

fn log_paths(config: &RuntimeConfig) -> Result<Vec<PathBuf>, StepError> {
    let mut paths = default_log_paths().to_vec();
    for file in [&config.auth_log_file, &config.nginx_access_log, &config.nginx_error_log] {
        let parent = file.parent().ok_or_else(|| error("log_dir_unsafe", "로그 경로에 상위 폴더가 없습니다."))?;
        if !paths.iter().any(|p| p == parent) { paths.push(parent.into()); }
    }
    Ok(paths)
}

fn prepare_handles(config: &RuntimeConfig, receipt: &Receipt, identity: &privilege::Identity, key: &[u8], create_publisher: bool) -> Result<fs::File, StepError> {
    io_error(handles::ensure_directory(Path::new(RUNTIME_PRIVATE), 0, privilege::API_GID, 0o750), "key_handle_failed")?;
    io_error(handles::ensure_directory(Path::new(RUNTIME_TMP), privilege::API_UID, privilege::API_GID, 0o700), "privilege_prepare_failed")?;
    io_error(handles::ensure_directory(Path::new(PUBLISHER_RUNTIME), 0, privilege::API_GID, 0o750), "privilege_prepare_failed")?;
    io_error(prepare_publisher_directory(&publisher_directory(), create_publisher), "privilege_prepare_failed")?;
    io_error(receipt.validate_key(key), "key_handle_failed")?;
    io_error(handles::atomic_write(Path::new(RUNTIME_KEY), &key, privilege::API_GID, 0o440), "key_handle_failed")?;
    io_error(receipt.write(), "key_handle_failed")?;
    let nginx_gid = identity.groups[0];
    for path in default_log_paths() {
        let dir = io_error(handles::directory(&path), "log_dir_unsafe")?;
        let (gid,mode) = if path == Path::new("/var/log/nginx") { (nginx_gid,0o750) } else { (0,0o755) };
        io_error(log_metadata(&path,&dir,0,gid,mode,true), "log_dir_unsafe")?;
    }
    io_error(handles::open_auth_log(&config.auth_log_file, &identity.groups), "auth_log_failed")
}

struct ProbeFiles(Vec<PathBuf>);
impl Drop for ProbeFiles {
    fn drop(&mut self) { for path in &self.0 { let _ = fs::remove_file(path); } }
}

fn storage_probe_paths(mut storage:Vec<PathBuf>,strict:Vec<PathBuf>,shared:impl Fn(&Path)->bool)->Vec<PathBuf>{
    storage.retain(|path|!shared(path));storage.extend(strict);storage
}
async fn probe(config: &RuntimeConfig, auth_log: &fs::File, identity: privilege::Identity) -> Result<(), StepError> {
    let mut protected = include_str!("../../../../docker/nonroot/protected-paths").lines().filter(|line| !line.is_empty() && !line.starts_with('#')).map(PathBuf::from).collect::<Vec<_>>();
    protected.push(config.certs_dir.clone());
    let mut strict=vec![config.nginx_conf_dir.clone(), config.nginx_stream_dir.clone(), RUNTIME_PRIVATE.into(), PUBLISHER_RUNTIME.into()];
    if let Some(parent)=config.nginx_main_conf.parent(){strict.push(parent.into());}
    let protected=storage_probe_paths(protected,strict,crate::db_upgrade::shared_folder::nearest_is_shared);
    let log_paths = storage_probe_paths(log_paths(config)?,vec![],crate::db_upgrade::shared_folder::path_is_shared);
    let mut files = ProbeFiles(Vec::new());
    let entries = &mut files.0;
    // Never rename or unlink real log files, even when the denial probe fails.
    for dir in log_paths {
        let path = dir.join(format!(".nwui-probe-{}", uuid::Uuid::new_v4()));
        if let Err(error) = handles::create_log_probe(&path, &identity.groups) {
            return io_error(Err(error), "log_dir_unsafe");
        }
        entries.push(path);
    }
    let log_entries = entries.clone();
    for entry in &log_entries {entries.push(io_error(access_probe::log_destination(entry),"log_dir_unsafe")?);}
    let conf_probe = config.nginx_conf_dir.join(format!(".nwui-access-probe-{}", uuid::Uuid::new_v4()));
    if let Err(error) = handles::create_log_probe(&conf_probe, &[]) {
        return io_error(Err(error), "isolation_unavailable");
    }
    entries.push(conf_probe.clone());
    let replacement = conf_probe.with_file_name(format!("{}-replacement", conf_probe.file_name().unwrap().to_string_lossy()));
    io_error(handles::create_log_probe(&replacement, &[]), "isolation_unavailable")?;
    entries.push(replacement);
    // Track the rename destination too, so cancellation cannot strand a moved sibling.
    entries.push(conf_probe.with_file_name(format!("{}-renamed", conf_probe.file_name().unwrap().to_string_lossy())));
    let mut plan = access_probe::Plan { conf_probe, key: RUNTIME_KEY.into(), writable: RUNTIME_TMP.into(), protected,
        log_entries, root_pid: std::process::id() as i32, publisher_socket: "/run/nwui-publisher/control.sock".into(), node_probe: false, log_probe:true };
    let mut failure_code="isolation_unavailable";
    let result=match access_probe::execute(&plan,auth_log,identity.clone()).await {
        Ok(true)=>{
            plan.log_probe=false;
            match access_probe::execute(&plan,auth_log,identity.clone()).await {
                Ok(true)=>{
                    plan.node_probe=true;
                    access_probe::execute(&plan,auth_log,identity).await.and_then(|passed|
                        if passed{Ok(())}else{Err(handles::invalid("actual-UID isolation probe failed"))})
                },
                Ok(false)=>Err(handles::invalid("actual-UID isolation probe failed")),
                Err(error)=>Err(error),
            }
        },
        Ok(false)=>{failure_code="log_dir_unsafe";Err(handles::invalid("actual-UID log directory probe failed"))},
        Err(error)=>Err(error),
    };
    let mut cleanup_error = None;
    for path in entries.iter() {
        if let Err(error) = fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound { cleanup_error.get_or_insert(error); }
        }
    }
    if let Some(error) = cleanup_error { return io_error(Err(error), failure_code); }
    entries.clear();
    io_error(result, failure_code)
}

pub async fn prepare(sel: &StorageSelection, locks: &StorageLocks, pg: &PrivatePg, j: &mut Journal, mode:publisher::boot::BootMode) -> Result<Prepared, StepError> {
    require_verified(sel)?;
    if !locks.holds_a() || !locks.holds_d() { return Err(error("storage_busy", "저장소 잠금이 필요합니다.")); }
    let identity = io_error(privilege::Identity::read(), "privilege_prepare_failed")?;
    let config = config().map_err(|_| error("privilege_prepare_failed", "루트 시작 설정을 읽지 못했습니다."))?;
    let admitted=admit_nginx(&config).await?;
    if admitted.conf_mismatch {
        static WARNED:std::sync::Once=std::sync::Once::new();
        WARNED.call_once(||eprintln!("[nonroot] NGINX_CONF differs from nginx's compiled --conf-path; nginx uses {}",admitted.conf_path.display()));
    }
    let canonical = canonical_key(&config)?;
    let log_modes = default_log_paths().iter().map(|path| io_error(LogMode::read(path), "log_dir_unsafe")).collect::<Result<Vec<_>, _>>()?;
    let previous_receipt = match handles::read_private_json(Path::new(RECEIPT)) {
        Ok(receipt) => Some(receipt),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return io_error(Err(error), "key_handle_failed"),
    };
    let publisher_created = match fs::symlink_metadata(publisher_directory()) {
        Ok(_) => j.load().records.last().is_some_and(|record| record.step == Step::S4Prepare)
            && latest_record(&j.load().records).is_some_and(|record| record.publisher_created),
        Err(error) if error.kind() == io::ErrorKind::NotFound => true,
        Err(error) => return io_error(Err(error), "privilege_prepare_failed"),
    };
    io_error(owned_bootstrap(&publisher_directory(), publisher_created), "verification_failed")?;
    let source_kind = j.key_source()?;
    let source = source_kind.path(Path::new(crate::db_upgrade::paths::APPROOT), Some(config.data_key_file.as_os_str()));
    let policy = key_file::policy_for(&source_kind,Path::new(crate::db_upgrade::paths::APPROOT),&canonical);
    let material = validated_key(source,policy).await?;
    let key = material.bytes;
    let secrets = material.secrets;
    let receipt = Receipt::new(sel, &key, &j.load().attempt_id)?;
    let intent = PrepareRecord { prepared: None, log_modes: log_modes.clone(), previous_receipt: previous_receipt.clone(), publisher_created, candidate: Some(sel.clone()) };
    j.record(Step::S4Prepare, StepState::Intent, Some(serde_json::to_value(intent).unwrap()))?;
    let auth_log = prepare_handles(&config, &receipt, &identity, &key, true)?;
    probe(&config, &auth_log, identity).await?;
    let store = publisher::PublisherStore::open(&config)?;
    let payload = publisher::boot::prepare_payload(&config, &secrets, publisher::boot::PgSource::Private(pg), &store, mode).await?;
    let prepared = Prepared { payload: Some(payload), key_receipt: Some(receipt.key) };
    j.record(Step::S4Prepare, StepState::Done, Some(serde_json::to_value(PrepareRecord { prepared: Some(prepared.clone()), log_modes, previous_receipt, publisher_created, candidate: Some(sel.clone()) }).unwrap()))?;
    Ok(prepared)
}

pub async fn discard(_p: Prepared, j: &mut Journal) -> Result<(), StepError> {
    let Some(record) = latest_record(&j.load().records) else {
        return if prepared_from_journal(j).is_none() { Ok(()) } else { Err(error("verification_failed", "권한 준비 복구 기록이 손상되었습니다.")) };
    };
    let current: Option<Receipt> = match handles::read_private_json(Path::new(RECEIPT)) {
        Ok(receipt) => Some(receipt),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return io_error(Err(error), "key_handle_failed"),
    };
    if current.as_ref().is_some_and(|receipt| receipt.attempt_id != j.load().attempt_id)
        && serde_json::to_value(&current).unwrap() != serde_json::to_value(&record.previous_receipt).unwrap() { return Ok(()); }
    // Complete N1's owned bootstrap while the journal still says S4-prepare.
    if io_error(owned_bootstrap(&publisher_directory(), record.publisher_created), "verification_failed")? {
        publisher::PublisherStore::open_root()?;
    }
    for mode in record.log_modes { io_error(mode.restore(), "log_dir_unsafe")?; }
    discard_runtime_handles(record.previous_receipt, Path::new(RUNTIME_PRIVATE)).await?;
    if record.publisher_created {
        match fs::remove_dir(publisher_directory()) {
            Ok(()) => io_error(handles::directory(Path::new(crate::db_upgrade::paths::APPROOT)).and_then(|dir|dir.sync_all()), "privilege_prepare_failed")?,
            Err(error) if error.kind() == io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ENOTEMPTY) => {},
            Err(error) => return io_error(Err(error), "privilege_prepare_failed"),
        }
    }
    Ok(())
}

async fn discard_runtime_handles(previous_receipt: Option<Receipt>, runtime_private: &Path) -> Result<(), StepError> {
    let directory = match handles::parent(&runtime_private.join("receipt.json")) {
        Ok((directory,_)) => directory,
        // /run may disappear before preparation creates any handles; phase 1 recreates them.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return io_error(Err(error), "key_handle_failed"),
    };
    if let Some(previous) = previous_receipt {
        let path = crate::db_upgrade::paths::startup_data_key_file(Path::new(crate::db_upgrade::paths::APPROOT))
            .map_err(|e|StepError {code:e.code,message:e.message,detail:None})?;
        let policy = key_file::canonical_policy(Path::new(crate::db_upgrade::paths::APPROOT),&path);
        let key = validated_key(path,policy).await?.bytes;
        io_error(previous.validate_key(&key), "key_handle_failed")?;
        io_error(handles::atomic_write(&runtime_private.join("data_key"), &key, privilege::API_GID, 0o440), "key_handle_failed")?;
        io_error(previous.write_at(&runtime_private.join("receipt.json")), "key_handle_failed")?;
    } else {
        // Only runtime copies from this attempt, never canonical storage, are removed.
        for path in [runtime_private.join("receipt.json"), runtime_private.join("data_key")] {
            match fs::remove_file(path) { Ok(()) => {}, Err(error) if error.kind() == io::ErrorKind::NotFound => {}, Err(error) => return io_error(Err(error), "key_handle_failed") }
        }
        io_error(directory.sync_all(), "key_handle_failed")?;
    }
    Ok(())
}

pub async fn activate(sel: &StorageSelection, p: &Prepared, j: &mut Journal) -> Result<(), StepError> {
    require_verified(sel)?;
    let journaled = prepared_from_journal(j).ok_or_else(|| error("verification_failed", "기록된 준비 설정이 없습니다."))?;
    if serde_json::to_value(&journaled).unwrap() != serde_json::to_value(p).unwrap() {
        return Err(error("verification_failed", "준비된 설정이 복구 기록과 다릅니다."));
    }
    let expected = p.key_receipt.as_ref().ok_or_else(|| error("key_handle_failed", "준비된 키 핸들이 없습니다."))?;
    let payload = p.payload.as_ref().ok_or_else(|| error("verification_failed", "준비된 설정이 없습니다."))?;
    let record = latest_record(&j.load().records).ok_or_else(|| error("verification_failed", "준비 복구 기록이 없습니다."))?;
    let candidate = record.candidate.ok_or_else(|| error("verification_failed", "준비된 저장소 선택 기록이 없습니다."))?;
    let previous = match handles::read_private_json(Path::new(RECEIPT)) {
        Ok(receipt) => Some(receipt), Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return io_error(Err(error), "key_handle_failed"),
    };
    let config = config().map_err(|_| error("privilege_prepare_failed", "루트 시작 설정을 읽지 못했습니다."))?;
    let identity = io_error(privilege::Identity::read(), "privilege_prepare_failed")?;
    let path = canonical_key(&config)?;
    let policy = key_file::canonical_policy(Path::new(crate::db_upgrade::paths::APPROOT),&path);
    let canonical = validated_key(path,policy).await?.bytes;
    let receipt = Receipt::for_activation(previous, &candidate, sel, expected, &canonical, &j.load().attempt_id)?;
    let provisional = Receipt::new(&candidate, &canonical, &j.load().attempt_id)?;
    j.record(Step::S4Activate, StepState::Intent, None)?;
    // /run is disposable: journal identity plus validated canonical bytes re-create its handles.
    let log = prepare_handles(&config, &provisional, &identity, &canonical, false)?;
    probe(&config, &log, identity).await?;
    let copy = io_error(handles::read_runtime_key(Path::new(RUNTIME_KEY)), "key_handle_failed")?;
    if canonical != copy { return Err(error("key_handle_failed", "런타임 키 복사본이 정식 키와 다릅니다.")); }
    let store = publisher::PublisherStore::open(&config)?;
    publisher::boot::publish_live(&config, &store, &payload.id).await?;
    io_error(receipt.write(), "key_handle_failed")?;
    j.record(Step::S4Activate, StepState::Done, None)
}

#[cfg(test)]
mod tests {
    #[test]
    fn probe_siblings_are_cleaned_on_success_and_failure() {
        let temp = tempfile::tempdir().unwrap();
        for fail in [false, true] {
            let path = temp.path().join(".nwui-access-probe-owned");
            let result = (|| -> std::io::Result<()> {
                let mut files = super::ProbeFiles(Vec::new());
                std::fs::write(&path, b"probe")?;
                files.0.push(path.clone());
                if fail { return Err(std::io::Error::other("probe refused")); }
                Ok(())
            })();
            assert_eq!(result.is_err(), fail);
            assert!(!path.exists());
        }
    }

    #[tokio::test]
    async fn probe_siblings_are_cleaned_when_cancelled() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".nwui-access-probe-cancelled");
        let owned = path.clone();
        let (ready, started) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            std::fs::write(&owned, b"probe").unwrap();
            let _files = super::ProbeFiles(vec![owned]);
            ready.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        started.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!path.exists());
    }

    use super::*;
    #[test]
    fn privilege_split_is_on() { assert!(PRIVILEGE_SPLIT); }
    use crate::upgrade::Branch;
    use std::os::unix::fs::PermissionsExt;

    fn selection() -> StorageSelection {
        StorageSelection { version: 1, epoch: 7, outcome: Outcome::Pg17, branch: Branch::Ready,
            pgdata: "/var/lib/postgresql/data/17/data".into(), backup_id: None, schema_profile: "current".into(),
            key_binding: KeyBinding::Verified { pair_id: "fixture-pair".into() } }
    }
    // Fixed image paths are exercised only in an explicitly disposable root container.
    async fn g50_discard_fixture(previous: bool) {
        assert_eq!(std::env::var("NWUI_G50_DISPOSABLE_ROOT").as_deref(),Ok("1"));
        assert_eq!(unsafe{libc::geteuid()},0);
        for name in ["AUTH_SECRET","DATABASE_URL","DATA_KEY_FILE"] {
            assert!(std::env::var_os(name).is_none());
            assert!(startup::envdir_value(name).is_none());
        }
        let app=Path::new(crate::db_upgrade::paths::APPROOT);
        let runtime=Path::new(RUNTIME_PRIVATE);
        assert!(!app.exists() && !runtime.exists(),"disposable fixture paths must start absent");
        fs::create_dir(app).unwrap();fs::set_permissions(app,fs::Permissions::from_mode(0o700)).unwrap();
        struct Cleanup;
        impl Drop for Cleanup {fn drop(&mut self){let _=fs::remove_dir_all(crate::db_upgrade::paths::APPROOT);let _=fs::remove_dir_all(RUNTIME_PRIVATE);}}
        let _cleanup=Cleanup;
        let db=tempfile::tempdir_in("/run").unwrap();
        let locks=StorageLocks::acquire(db.path(),app).unwrap();
        let mut j=Journal::begin(app,&locks,Branch::Ready,crate::upgrade::journal::Candidate{revision:"1".repeat(40),pins_sha256:"2".repeat(64),migrations_sha256:"3".repeat(64)}).unwrap();
        let key="AB".repeat(32).into_bytes();
        let receipt=if previous {
            fs::create_dir(app.join("secrets")).unwrap();
            fs::set_permissions(app.join("secrets"),fs::Permissions::from_mode(0o700)).unwrap();
            fs::write(app.join("secrets/data_key"),&key).unwrap();
            fs::set_permissions(app.join("secrets/data_key"),fs::Permissions::from_mode(0o600)).unwrap();
            handles::ensure_directory(runtime,0,privilege::API_GID,0o750).unwrap();
            Some(Receipt::new(&selection(),&key,"previous-attempt").unwrap())
        }else{None};
        let previous_bytes=receipt.as_ref().map(|r|serde_json::to_vec(r).unwrap());
        let record=PrepareRecord{prepared:None,log_modes:Vec::new(),previous_receipt:receipt,publisher_created:true,candidate:None};
        j.record(Step::S4Prepare,StepState::Intent,Some(serde_json::to_value(record).unwrap())).unwrap();
        fs::create_dir(publisher_directory()).unwrap();
        fs::set_permissions(publisher_directory(),fs::Permissions::from_mode(0o700)).unwrap();
        crate::upgrade::runner::discard_privileges(&mut j).await.unwrap();
        assert!(j.load().records.iter().any(|r|r.step==Step::P5Publish && r.state==StepState::Done && r.data.as_ref().is_some_and(|d|d["authorityEvent"]=="privilege-discarded")));
        assert!(!publisher_directory().join(".manifest.init").exists());
        let manifest:serde_json::Value=serde_json::from_slice(&fs::read(publisher_directory().join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest,serde_json::json!({"version":1,"current":null,"previous":null}));
        if let Some(bytes)=previous_bytes {
            assert_eq!(fs::read(RECEIPT).unwrap(),bytes);assert_eq!(fs::read(RUNTIME_KEY).unwrap(),key);
            let metadata=fs::symlink_metadata(RUNTIME_KEY).unwrap();assert_eq!((metadata.uid(),metadata.gid(),metadata.mode()&0o7777),(0,10001,0o440));
        }else{assert!(!runtime.exists());}
    }
    #[tokio::test]
    #[ignore = "requires NWUI_G50_DISPOSABLE_ROOT=1 in an empty disposable root container"]
    async fn g50_discard_bootstraps_without_credentials_and_journals_discarded() {if std::env::var("NWUI_G50_DISPOSABLE_ROOT").as_deref()!=Ok("1"){return;}g50_discard_fixture(false).await;}
    #[tokio::test]
    #[ignore = "requires NWUI_G50_DISPOSABLE_ROOT=1 in an empty disposable root container"]
    async fn g50_discard_restores_previous_receipt_without_credentials() {if std::env::var("NWUI_G50_DISPOSABLE_ROOT").as_deref()!=Ok("1"){return;}g50_discard_fixture(true).await;}
    fn g47_partial_intent(previous: bool) -> PrepareRecord {
        let mut records=fixture_prepare_records(&selection(),"fixture-attempt",1,false);
        assert_eq!(records[0].state,StepState::Intent);
        if !previous { records[0].data.as_mut().unwrap()["previousReceipt"]=serde_json::Value::Null; }
        let record=latest_record(&records).unwrap();assert!(record.prepared.is_none());record
    }
    #[tokio::test]
    async fn g47_partial_intent_with_absent_runtime_directory_discards_idempotently() {
        let temp=tempfile::tempdir_in("/run").unwrap();let path=temp.path().join("absent");
        for _ in 0..2 { discard_runtime_handles(g47_partial_intent(false).previous_receipt,&path).await.unwrap(); }
        assert!(!path.exists());assert_eq!(fs::read_dir(temp.path()).unwrap().count(),0);
    }
    #[tokio::test]
    async fn g47_previous_receipt_with_absent_runtime_directory_is_not_restored() {
        let temp=tempfile::tempdir_in("/run").unwrap();let path=temp.path().join("absent");
        for _ in 0..2 { discard_runtime_handles(g47_partial_intent(true).previous_receipt,&path).await.unwrap(); }
        assert!(!path.exists());assert_eq!(fs::read_dir(temp.path()).unwrap().count(),0);
    }
    #[tokio::test]
    async fn g47_unsafe_runtime_directories_refuse_before_handle_removal() {
        for case in ["symlink","foreign-owner","writable"] {
            let temp=tempfile::tempdir_in("/run").unwrap();let real=temp.path().join("real");fs::create_dir(&real).unwrap();
            fs::write(real.join("data_key"),b"preserved").unwrap();
            let path=if case=="symlink" { let path=temp.path().join("linked");std::os::unix::fs::symlink(&real,&path).unwrap();path } else {real.clone()};
            if case=="foreign-owner" {std::os::unix::fs::chown(&real,Some(501),Some(501)).unwrap();}
            if case=="writable" {fs::set_permissions(&real,fs::Permissions::from_mode(0o777)).unwrap();}
            assert_eq!(discard_runtime_handles(None,&path).await.unwrap_err().code,"key_handle_failed","{case}");
            assert_eq!(fs::read(real.join("data_key")).unwrap(),b"preserved","{case}");
        }
    }
    #[tokio::test]
    async fn g47_non_notfound_directory_open_error_stays_key_handle_failed() {
        let temp=tempfile::tempdir_in("/run").unwrap();let path=temp.path().join("not-directory");fs::write(&path,b"preserved").unwrap();
        assert_eq!(handles::directory(&path).unwrap_err().raw_os_error(),Some(libc::ENOTDIR));
        assert_eq!(discard_runtime_handles(None,&path).await.unwrap_err().code,"key_handle_failed");
        assert_eq!(fs::read(path).unwrap(),b"preserved");
    }
    #[tokio::test]
    async fn g47_existing_runtime_handles_are_removed_and_siblings_preserved() {
        let temp=tempfile::tempdir_in("/run").unwrap();let path=temp.path().join("private");fs::create_dir(&path).unwrap();
        std::os::unix::fs::chown(&path,Some(0),Some(privilege::API_GID)).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o750)).unwrap();
        for name in ["data_key","receipt.json","sibling"] {fs::write(path.join(name),b"preserved").unwrap();}
        discard_runtime_handles(None,&path).await.unwrap();discard_runtime_handles(None,&path).await.unwrap();
        assert!(!path.join("data_key").exists());assert!(!path.join("receipt.json").exists());assert_eq!(fs::read(path.join("sibling")).unwrap(),b"preserved");
    }
    #[tokio::test]
    async fn g47_runtime_handle_remove_errors_remain_strict() {
        let temp=tempfile::tempdir_in("/run").unwrap();let path=temp.path().join("private");fs::create_dir(&path).unwrap();fs::create_dir(path.join("receipt.json")).unwrap();
        assert_eq!(discard_runtime_handles(None,&path).await.unwrap_err().code,"key_handle_failed");assert!(path.join("receipt.json").is_dir());
    }
    #[test]
    fn activation_cannot_create_a_missing_publisher_store() {
        let temp = tempfile::tempdir().unwrap(); let path = temp.path().join("publisher");
        assert!(prepare_publisher_directory(&path,false).is_err());
        assert!(!path.exists());
        prepare_publisher_directory(&path,true).unwrap();
        prepare_publisher_directory(&path,false).unwrap();
    }
    #[test]
    fn unsupported_log_chown_still_enforces_the_mode_for_the_uid_probe() {
        for code in [libc::EPERM,libc::EINVAL,libc::ENOTSUP] {
            let changed = std::cell::Cell::new(false);
            chown_then_chmod(Err(io::Error::from_raw_os_error(code)),|| { changed.set(true); Ok(()) }).unwrap();
            assert!(changed.get());
        }
        let changed = std::cell::Cell::new(false);
        assert!(chown_then_chmod(Err(io::Error::from_raw_os_error(libc::EIO)),|| { changed.set(true); Ok(()) }).is_err());
        assert!(!changed.get());
        assert!(chown_then_chmod(Err(io::Error::from_raw_os_error(libc::EPERM)),||Err(io::Error::from_raw_os_error(libc::EACCES))).is_err());
    }
    #[test]
    fn ineffective_chmod_refuses_log_dir_before_any_later_step() {
        let temp=tempfile::tempdir_in("/run").unwrap();
        fs::set_permissions(temp.path(),fs::Permissions::from_mode(0o777)).unwrap();
        let dir=handles::directory(temp.path()).unwrap();let later=std::cell::Cell::new(false);
        let result=io_error(log_metadata_with(temp.path(),&dir,0,0,0o755,false,||Ok(())),"log_dir_unsafe").map(|_|later.set(true));
        assert_eq!(result.unwrap_err().code,"log_dir_unsafe");
        assert!(!later.get(),"no later step may run after ineffective chmod");
        assert_eq!(fs::symlink_metadata(temp.path()).unwrap().mode()&0o7777,0o777);
    }
    #[test]
    fn log_metadata_requires_exact_identity_owner_group_and_mode_without_following_links() {
        for case in ["owner","group","mode","symlink","replacement"] {
            let temp=tempfile::tempdir_in("/run").unwrap();let path=temp.path().join("log");fs::create_dir(&path).unwrap();
            let dir=handles::directory(&path).unwrap();
            let result=log_metadata_with(&path,&dir,0,0,0o755,false,|| {
                assert_eq!(unsafe{libc::fchmod(dir.as_raw_fd(),0o755)},0);
                match case {
                    "owner"=>assert_eq!(unsafe{libc::fchown(dir.as_raw_fd(),501,0)},0),
                    "group"=>assert_eq!(unsafe{libc::fchown(dir.as_raw_fd(),0,10001)},0),
                    "mode"=>assert_eq!(unsafe{libc::fchmod(dir.as_raw_fd(),0o775)},0),
                    _=>{let old=temp.path().join("old");fs::rename(&path,&old).unwrap();if case=="symlink"{std::os::unix::fs::symlink(&old,&path).unwrap();}else{fs::create_dir(&path).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o755)).unwrap();}},
                }
                Ok(())
            });
            assert_eq!(io_error(result,"log_dir_unsafe").unwrap_err().code,"log_dir_unsafe","{case}");
        }
        let temp=tempfile::tempdir_in("/run").unwrap();let dir=handles::directory(temp.path()).unwrap();log_metadata(temp.path(),&dir,0,0,0o755,false).unwrap();
        assert_eq!(fs::symlink_metadata(temp.path()).unwrap().mode()&0o7777,0o755);
    }
    #[test]
    fn empty_canonical_key_path_is_refused_without_relying_on_runner_order() {
        let config = RuntimeConfig::read(|key|match key {
            "AUTH_SECRET"=>Some("fixture-only".into()), "DATABASE_URL"=>Some("postgres://postgres@127.0.0.1/postgres".into()),
            "DATA_KEY_FILE"=>Some(String::new()), _=>None,
        }).unwrap();
        assert_eq!(canonical_key(&config).unwrap_err().code,"key_path_invalid");
    }
    #[test]
    fn unverified_candidate_is_named_before_any_prepare_intent() {
        let mut selection = selection(); selection.key_binding = KeyBinding::Unverified;
        assert_eq!(require_verified(&selection).unwrap_err().code, "key_unverified");
    }
    #[test]
    fn bootstrap_temp_requires_this_attempts_creation_record_and_is_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("store");
        assert!(!owned_bootstrap(&store,true).unwrap());
        fs::create_dir(&store).unwrap(); fs::set_permissions(&store,fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(store.join(".manifest.init"),b"N1-owned-temp").unwrap();
        assert!(owned_bootstrap(&store,false).is_err());
        assert_eq!(fs::read(store.join(".manifest.init")).unwrap(),b"N1-owned-temp");
        assert!(owned_bootstrap(&store,true).unwrap());
        fs::set_permissions(&store,fs::Permissions::from_mode(0o755)).unwrap();
        assert!(owned_bootstrap(&store,true).is_err());
    }
    #[test]
    fn host_log_preimage_preserves_nonroot_owner_for_rollback() {
        let temp = tempfile::tempdir_in("/run").unwrap();
        let path = std::ffi::CString::new(temp.path().as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::chown(path.as_ptr(),501,501) }, 0);
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o777)).unwrap();
        let previous = LogMode::read(temp.path()).unwrap();
        assert_eq!((previous.uid, previous.gid, previous.mode), (501,501,0o777));
        let directory = handles::directory(temp.path()).unwrap();
        assert_eq!(unsafe { libc::fchown(directory.as_raw_fd(),0,0) },0);
        assert_eq!(unsafe { libc::fchmod(directory.as_raw_fd(),0o755) },0);
        previous.restore().unwrap();
        let restored = fs::metadata(temp.path()).unwrap();
        assert_eq!((restored.uid(),restored.gid(),restored.mode() & 0o7777),(501,501,0o777));
    }
    #[test]
    fn only_default_log_directories_are_enforced_and_override_parents_stay_unchanged() {
        let temp = tempfile::tempdir_in("/run").unwrap();
        let override_dir = temp.path().join("custom-log"); fs::create_dir(&override_dir).unwrap();
        let name = std::ffi::CString::new(override_dir.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::chown(name.as_ptr(),501,501) }, 0);
        fs::set_permissions(&override_dir, fs::Permissions::from_mode(0o750)).unwrap();
        let canonical = temp.path().join("key"); fs::write(&canonical, "a".repeat(64)).unwrap();
        fs::set_permissions(&canonical, fs::Permissions::from_mode(0o600)).unwrap();
        let config = RuntimeConfig::read(|key| match key {
            "AUTH_SECRET" => Some("fixture-only".into()), "DATABASE_URL" => Some("postgres://postgres@127.0.0.1/postgres".into()),
            "DATA_KEY_FILE" => Some(canonical.display().to_string()),
            "AUTH_LOG" => Some(override_dir.join("auth.log").display().to_string()),
            "NGINX_ACCESS_LOG" => Some(override_dir.join("access.log").display().to_string()),
            "NGINX_ERROR_LOG" => Some(override_dir.join("error.log").display().to_string()), _ => None,
        }).unwrap();
        fs::create_dir_all("/var/lib/nginx-web-ui").unwrap();
        for path in default_log_paths() {
            fs::create_dir_all(&path).unwrap();
            let directory = handles::directory(&path).unwrap();
            assert_eq!(unsafe { libc::fchown(directory.as_raw_fd(),501,501) },0);
            assert_eq!(unsafe { libc::fchmod(directory.as_raw_fd(),0o777) },0);
        }
        let receipt = Receipt::new(&selection(), &fs::read(&canonical).unwrap(), "fixture").unwrap();
        let identity = privilege::Identity::for_test_groups(vec![101,4]);
        prepare_handles(&config, &receipt, &identity, &fs::read(&canonical).unwrap(), true).unwrap();
        let after = fs::metadata(&override_dir).unwrap();
        assert_eq!((after.uid(),after.gid(),after.mode() & 0o7777), (501,501,0o750));
        let nginx = fs::metadata("/var/log/nginx").unwrap();
        assert_eq!((nginx.uid(),nginx.gid(),nginx.mode() & 0o7777),(0,101,0o750));
        let app = fs::metadata("/var/log/nginx-web-ui").unwrap();
        assert_eq!((app.uid(),app.gid(),app.mode() & 0o7777),(0,0,0o755));
        let publisher = fs::metadata("/var/lib/nginx-web-ui/.nwui-publisher").unwrap();
        assert_eq!((publisher.uid(), publisher.mode() & 0o7777), (0,0o700));
    }
    #[test]
    fn journal_intent_recovers_preimages_and_never_falls_back_past_a_new_attempt() {
        use crate::upgrade::journal::Record;
        let intent = PrepareRecord { prepared: None, log_modes: vec![LogMode { path: "/var/log/nginx".into(), uid: 0, gid: 999, mode: 0o777 }], previous_receipt: None, publisher_created: false, candidate: None };
        let record = Record { seq: 1, step: Step::S4Prepare, state: StepState::Intent, at: "fixture".into(), data: Some(serde_json::to_value(intent).unwrap()) };
        let data = record.data.as_ref().unwrap();
        assert!(data.get("logModes").is_some());
        assert!(data.get("previousReceipt").is_some());
        let recovered = latest_record(&[record.clone()]).unwrap();
        assert!(recovered.prepared.is_none());
        assert_eq!(recovered.log_modes[0].mode, 0o777);
        let mut malformed = record.clone();
        malformed.seq = 2;
        malformed.data = Some(serde_json::json!({"wrong":"shape"}));
        assert!(latest_record(&[record.clone(), malformed]).is_none());
        let mut done = record.clone(); done.state = StepState::Done;
        done.data.as_mut().unwrap()["prepared"] = serde_json::json!({"payload":{"id":"generation","manifestSha256":"digest","skipped":[]}, "keyReceipt":{"selectionEpoch":7,"keySha256":"hash"}});
        assert_eq!(latest_record(&[record, done]).unwrap().prepared.unwrap().payload.unwrap().id, "generation");
    }
    #[test]
    fn missing_boot_identity_refuses_privilege_prepare_without_writing_a_receipt() {
        let temp=tempfile::tempdir().unwrap();let path=temp.path().join("receipt.json");
        let result=Receipt::with_identity(&selection(),&[1;32],"attempt",Err(error("boot_step_failed","fixture")));
        match result {
            Ok(receipt)=>{fs::write(&path,serde_json::to_vec(&receipt).unwrap()).unwrap();panic!("missing identity accepted");},
            Err(error)=>assert_eq!(error.code,"privilege_prepare_failed"),
        }
        assert!(!path.exists());
    }

    #[test]
    fn receipt_uses_shared_container_boot_identity_and_refuses_old_boot_id_shape() {
        let receipt=Receipt::new(&selection(),&[1;32],"attempt").unwrap();
        let mut value=serde_json::to_value(receipt).unwrap();
        let identity=crate::upgrade::boot_identity::current().unwrap();
        assert_eq!(value["bootIdentity"],identity);
        assert!(value.get("bootId").is_none());
        value.as_object_mut().unwrap().remove("bootIdentity");
        value["bootId"]=serde_json::json!("old host-only boot id");
        assert!(serde_json::from_value::<Receipt>(value).is_err());
    }

    #[test]
    fn runtime_receipt_binds_key_selection_pair_boot_and_activation() {
        let selection = selection();
        let receipt = Receipt::new(&selection, &[1;32], "attempt").unwrap();
        receipt.validate(&selection, false).unwrap();
        assert!(receipt.validate(&selection, true).is_err());
        receipt.validate_key(&[1;32]).unwrap();
        assert!(receipt.validate_key(&[2;32]).is_err());
        let mut active = receipt.clone(); active.active = true;
        active.validate(&selection, true).unwrap();
        let mut changed = selection.clone(); changed.epoch += 1;
        assert!(active.validate(&changed, true).is_err());
        changed = selection.clone(); changed.pgdata = "/other".into();
        assert!(active.validate(&changed, true).is_err());
        changed = selection.clone(); changed.key_binding = KeyBinding::Verified { pair_id: "other".into() };
        assert!(active.validate(&changed, true).is_err());
        changed = selection.clone(); changed.outcome = Outcome::Upgrading;
        assert!(active.validate(&changed, true).is_err());
        active.boot_identity = "other".into();
        assert!(active.validate(&selection, true).is_err());
    }
    #[test]
    fn activation_recreates_missing_runtime_receipt_from_canonical_and_journal_digest() {
        let mut candidate = selection(); candidate.pgdata = "/candidate/work".into(); candidate.outcome = Outcome::Upgrading;
        let committed = selection(); let key = vec![b'a';64];
        let original = Receipt::new(&candidate,&key,"attempt").unwrap();
        let active = Receipt::for_activation(None,&candidate,&committed,&original.key,&key,"attempt").unwrap();
        active.validate(&committed,true).unwrap();
        assert!(Receipt::for_activation(None,&candidate,&committed,&original.key,&vec![b'b';64],"attempt").is_err());
    }
    #[test]
    fn activation_rebinds_candidate_path_and_epoch_and_resumes_after_receipt_write() {
        let mut candidate = selection(); candidate.pgdata = "/candidate/work".into(); candidate.outcome = Outcome::Upgrading;
        let mut committed = selection(); committed.epoch += 1;
        let key = vec![b'a';64]; let provisional = Receipt::new(&candidate,&key,"attempt").unwrap();
        let active = Receipt::for_activation(Some(provisional.clone()),&candidate,&committed,&provisional.key,&key,"attempt").unwrap();
        active.validate(&committed,true).unwrap();
        let again = Receipt::for_activation(Some(active.clone()),&candidate,&committed,&provisional.key,&key,"attempt").unwrap();
        again.validate(&committed,true).unwrap();
        assert!(Receipt::for_activation(Some(active),&candidate,&committed,&provisional.key,&key,"another-attempt").is_err());
        committed.key_binding = KeyBinding::Verified { pair_id: "another-pair".into() };
        assert!(Receipt::for_activation(None,&candidate,&committed,&provisional.key,&key,"attempt").is_err());
    }
    #[test]
    fn log_mode_has_a_recoverable_preimage_before_enforcement() {
        let temp = tempfile::tempdir_in("/run").unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o777)).unwrap();
        let before = LogMode::read(temp.path()).unwrap();
        handles::ensure_directory(temp.path(), 0, 0, 0o755).unwrap();
        assert_eq!(fs::metadata(temp.path()).unwrap().mode() & 0o7777, 0o755);
        let durable: LogMode = serde_json::from_slice(&serde_json::to_vec(&before).unwrap()).unwrap();
        durable.restore().unwrap();
        assert_eq!(fs::metadata(temp.path()).unwrap().mode() & 0o7777, 0o777);
    }
    #[test]
    fn atomic_key_copy_does_not_change_canonical_key_and_receipt_stays_private() {
        let temp = tempfile::tempdir().unwrap();
        let canonical = temp.path().join("canonical");
        fs::write(&canonical, "a".repeat(64)).unwrap();
        fs::set_permissions(&canonical, fs::Permissions::from_mode(0o600)).unwrap();
        let dir = temp.path().join("handles");
        handles::ensure_directory(&dir, 0, privilege::API_GID, 0o750).unwrap();
        let target = dir.join("data_key");
        handles::atomic_write(&target, &handles::read_key(&canonical).unwrap(), privilege::API_GID, 0o440).unwrap();
        assert_eq!(fs::read(&canonical).unwrap(), fs::read(&target).unwrap());
        let metadata = fs::metadata(&target).unwrap();
        assert_eq!((metadata.uid(), metadata.gid(), metadata.mode() & 0o7777, metadata.nlink()), (0, privilege::API_GID, 0o440, 1));
        assert_eq!(handles::read_runtime_key(&target).unwrap(), fs::read(&canonical).unwrap());
        assert!(handles::read_key(&target).is_err());
        let receipt = dir.join("receipt.json");
        handles::atomic_write(&receipt, br#"{"epoch":7}"#, privilege::API_GID, 0o440).unwrap();
        let value: serde_json::Value = handles::read_private_json(&receipt).unwrap();
        assert_eq!(value["epoch"], 7);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
    }

    #[test]
    fn runtime_receipt_refuses_unknown_fields_at_both_levels() {
        let receipt=Receipt::new(&selection(),&[1;32],"attempt").unwrap();
        for nested in [false,true] {
            let mut value=serde_json::to_value(&receipt).unwrap();
            if nested{value["key"]["unknown"]=serde_json::json!(true);}else{value["unknown"]=serde_json::json!(true);}
            assert!(serde_json::from_value::<Receipt>(value).is_err());
        }
    }

}

#[cfg(test)]mod desktop_shared_plan_tests{
 use super::*;
 #[test]fn desktop_shared_storage_split_keeps_configuration_overrides_and_runtime_strict(){
  let app=PathBuf::from("/var/lib/nginx-web-ui");let conf=app.join("custom-conf");let runtime=PathBuf::from(RUNTIME_PRIVATE);
  let result=storage_probe_paths(vec![app.clone(),app.join("certs"),"/var/lib/postgresql/data".into()],vec![conf.clone(),runtime.clone()],|p|p.starts_with(&app));
  assert_eq!(result,vec![PathBuf::from("/var/lib/postgresql/data"),conf,runtime]);
  assert!(storage_probe_paths(vec![app.clone()],vec![app.clone()],|_|true).contains(&app));
  let logs=vec![PathBuf::from("/var/log/nginx"),PathBuf::from("/var/log/nginx-web-ui")];
  assert_eq!(storage_probe_paths(logs.clone(),vec![],|p|p==logs[1]),vec![logs[0].clone()]);
 }
}

#[cfg(all(test,target_os="linux"))]mod orbstack_log_tests{
 use super::*;
 use crate::db_upgrade::shared_folder::SharedKind;
 use std::os::unix::fs::PermissionsExt;
 #[test]fn projected_nginx_log_owner_is_limited_to_default_preparation(){
  if unsafe{libc::geteuid()}!=0{return;}
  let t=tempfile::tempdir().unwrap();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o750)).unwrap();let m=fs::metadata(t.path()).unwrap();let path=Path::new("/var/log/nginx");
  assert!(log_metadata_matches(path,&m,&m,(0,999,0o750),true,Some(SharedKind::OrbStackMac)));
  for kind in [None,Some(SharedKind::DockerDesktop)]{assert!(!log_metadata_matches(path,&m,&m,(0,999,0o750),true,kind));}
  assert!(!log_metadata_matches(path,&m,&m,(0,999,0o750),false,Some(SharedKind::OrbStackMac)));
  assert!(!log_metadata_matches(Path::new("/run"),&m,&m,(0,999,0o750),true,Some(SharedKind::OrbStackMac)));
  assert!(!log_metadata_matches(path,&m,&m,(100,999,0o750),true,Some(SharedKind::OrbStackMac)));
  assert!(!log_metadata_matches(path,&m,&m,(0,999,0o755),true,Some(SharedKind::OrbStackMac)));
  let other=tempfile::tempdir().unwrap();assert!(!log_metadata_matches(path,&m,&fs::metadata(other.path()).unwrap(),(0,999,0o750),true,Some(SharedKind::OrbStackMac)));
  std::os::unix::fs::chown(t.path(),Some(0),Some(12345)).unwrap();let m=fs::metadata(t.path()).unwrap();assert!(!log_metadata_matches(path,&m,&m,(0,999,0o750),true,Some(SharedKind::OrbStackMac)));
 }
}
