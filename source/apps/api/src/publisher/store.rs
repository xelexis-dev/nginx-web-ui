//! Root-private generations and their durable manifest.
use crate::{config::RuntimeConfig, upgrade::StepError};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs::{self, File, OpenOptions}, io::{Read, Write}, os::unix::fs::{MetadataExt, OpenOptionsExt}, path::{Path, PathBuf}};

#[derive(Clone)]
pub struct PublisherStore {
    pub(super) root: PathBuf,
    #[cfg(test)]
    pub(super) capacity_probe:Option<super::capacity::Probe>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all="camelCase")]
pub(super) struct StoreManifest {
    pub version: u8,
    pub current: Option<String>,
    pub previous: Option<String>,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all="kebab-case")]
pub(super) enum Origin { Publisher, LegacyEvidence }

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields,rename_all="camelCase")]
pub(super) struct FileDigest { pub sha256:String, pub bytes:u64 }

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields,rename_all="camelCase")]
pub(super) struct PayloadManifest {
    pub version:u8,
    pub id:String,
    pub origin:Origin,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub derived_from:Option<String>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub reason:Option<String>,
    pub model_digest:Option<String>,
    pub certificate_digests:BTreeMap<String,String>,
    pub files:BTreeMap<String,FileDigest>,
}

// No Debug: the files include private keys.
pub(super) struct Payload {
    pub manifest:PayloadManifest,
    pub files:BTreeMap<String,Vec<u8>>,
    pub manifest_sha256:String,
}

pub(super) fn digest(bytes:&[u8]) -> String {
    use sha2::{Digest,Sha256};
    hex::encode(Sha256::digest(bytes))
}

pub(crate) fn payload_path(name:&str) -> Result<()> {
    ensure!(!name.contains(['\\','\0']),"Invalid payload path");
    let parts:Vec<_>=name.split('/').collect();
    ensure!(parts.iter().all(|part|!part.is_empty() && !matches!(*part,"."|"..")),"Invalid payload path");
    match parts.as_slice() {
        ["conf",name]=>ensure!(crate::publisher::files::ManagedKind::Conf.matches(name) || *name=="00-runtime-upstreams.conf","Unowned HTTP file"),
        ["stream",name]=>ensure!(crate::publisher::files::ManagedKind::Stream.matches(name),"Unowned stream file"),
        ["certs",id,name]=>{uuid(id)?;ensure!(matches!(*name,"fullchain.pem"|"privkey.pem"),"Unowned certificate file");},
        _=>anyhow::bail!("Invalid payload path"),
    }
    Ok(())
}

fn private_bytes(path:&Path) -> Result<Vec<u8>> {
    crate::db_upgrade::files::real_path(path.parent().context("Missing payload parent")?)?;
    let mut file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC).open(path)?;
    let meta=file.metadata()?;
    ensure!(meta.is_file() && meta.uid()==0 && meta.nlink()==1 && meta.mode()&0o7777==0o600,"Unsafe publisher file");
    let mut bytes=Vec::new();file.read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64==meta.len(),"Publisher file changed while reading");
    Ok(bytes)
}

pub(super) fn new_private_file(path:&Path,bytes:&[u8]) -> Result<()> {
    let parent=path.parent().context("Missing payload parent")?;
    crate::db_upgrade::files::real_path(parent)?;
    let mut file=OpenOptions::new().write(true).create_new(true).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub(super) fn step_error(code: &'static str, error: impl std::fmt::Display) -> StepError {
    StepError { code, message:"nginx 설정 발행 저장소를 확인할 수 없습니다.", detail:Some(error.to_string().chars().take(512).collect()) }
}

pub(super) fn uuid(id: &str) -> Result<()> {
    ensure!(uuid::Uuid::parse_str(id)?.to_string() == id, "Invalid publisher UUID");
    Ok(())
}

impl PublisherStore {
    pub(super) fn at(root:PathBuf)->Self {Self{root,#[cfg(test)]capacity_probe:None}}

    pub fn open(_config: &RuntimeConfig) -> Result<Self, StepError> { Self::open_root() }

    pub(crate) fn open_root() -> Result<Self, StepError> {
        #[cfg(feature="test-support")]
        if let Some(root)=std::env::var_os("NWUI_TEST_PUBLISHER_ROOT") {
            return Self::open_at(Path::new(&root));
        }
        Self::open_at(Path::new(crate::db_upgrade::paths::APPROOT))
    }

    /// Read-only admission handle: never initialize or repair the publisher store.
    pub(super) fn trusted_directory()->Option<File>{
        let app=std::path::PathBuf::from(crate::db_upgrade::paths::APPROOT);
        #[cfg(feature="test-support")]
        let app=std::env::var_os("NWUI_TEST_PUBLISHER_ROOT").map(std::path::PathBuf::from).unwrap_or(app);
        let path=app.join(".nwui-publisher");
        crate::db_upgrade::files::real_path(&path).ok()?;
        std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC).open(path).ok()
    }

    #[cfg(feature="test-support")]
    pub(super) fn open_test_root(root:&Path) -> Result<Self,StepError> {
        Self::open_at(root)
    }

    fn open_at(app: &Path) -> Result<Self, StepError> {
        let store=Self::open_inner(app).map_err(|error| step_error("publish_failed", error))?;
        super::poison::check(&store).map_err(|error| step_error("publish_failed", error))?;
        store.lock_file().map_err(|error|step_error("publish_failed",error))?;
        Ok(store)
    }

    fn open_inner(app: &Path) -> Result<Self> {
        ensure!(unsafe { libc::geteuid() } == 0, "Publisher requires root");
        let root = app.join(".nwui-publisher");
        crate::db_upgrade::files::real_path(&root)?;
        let meta = fs::symlink_metadata(&root)?;
        ensure!(meta.is_dir() && meta.uid()==0 && meta.mode()&0o7777==0o700, "Unsafe publisher directory");
        let manifest_path = root.join("manifest.json");
        match fs::symlink_metadata(&manifest_path) {
            Ok(_) => {
                let manifest: StoreManifest = crate::db_upgrade::files::read_json(&manifest_path)?;
                ensure!(manifest.version==1, "Unsupported publisher manifest");
                for id in [&manifest.current, &manifest.previous].into_iter().flatten() { uuid(id)?; }
                File::open(&root)?.sync_all()?;
                return Ok(Self::at(root));
            }
            Err(error) if error.kind()==std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }
        // Shape is insufficient: only this attempt's S4-prepare can own a partial temp.
        let journal: crate::upgrade::journal::JournalV1 = crate::db_upgrade::files::read_json(&app.join(".nwui-auto/journal-v1.json"))?;
        ensure!(journal.version==1, "Invalid upgrade journal");
        uuid(&journal.attempt_id)?;
        ensure!(journal.records.last().is_some_and(|record| record.step==crate::upgrade::Step::S4Prepare), "Publisher bootstrap is outside S4-prepare");
        let record = journal.records.iter().rev().find(|record|
            record.step==crate::upgrade::Step::S4Prepare && matches!(record.state, crate::upgrade::StepState::Intent | crate::upgrade::StepState::Done))
            .context("Missing S4-prepare intent")?;
        ensure!(record.data.as_ref().and_then(|data|data.get("publisherCreated")).and_then(serde_json::Value::as_bool)==Some(true), "Publisher bootstrap is not journal-owned");
        let temporary = root.join(".manifest.init");
        for entry in fs::read_dir(&root)? {
            let entry=entry?;
            ensure!(entry.file_name()==".manifest.init", "Unexpected publisher bootstrap entry");
            let meta=fs::symlink_metadata(entry.path())?;
            ensure!(meta.is_file() && !meta.file_type().is_symlink() && meta.uid()==0 && meta.nlink()==1, "Unsafe publisher bootstrap temp");
        }
        if fs::symlink_metadata(&temporary).is_ok() {
            fs::remove_file(&temporary)?;
            File::open(&root)?.sync_all()?;
        }
        let attempt = (|| -> Result<()> {
            let mut file=OpenOptions::new().write(true).create_new(true).mode(0o600)
                .custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC).open(&temporary)?;
            file.write_all(&serde_json::to_vec(&StoreManifest { version:1, current:None, previous:None })?)?;
            file.sync_all()?;
            fs::rename(&temporary, &manifest_path)?;
            File::open(&root)?.sync_all()?;
            Ok(())
        })();
        if let Err(error)=attempt {
            match fs::remove_file(&temporary) {
                Ok(())=>File::open(&root)?.sync_all()?,
                Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},
                Err(e)=>return Err(e).context("Could not retire publisher bootstrap temp"),
            }
            return Err(error);
        }
        Ok(Self::at(root))
    }

    fn lock_file(&self)->Result<File> {
        let owner=OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC|libc::O_NONBLOCK).open(self.root.join(".mutator.lock"))?;
        let meta=owner.metadata()?;
        ensure!(meta.is_file() && meta.uid()==0 && meta.nlink()==1 && meta.mode()&0o7777==0o600,"Unsafe publisher store lock");
        owner.sync_all()?;File::open(&self.root)?.sync_all()?;Ok(owner)
    }
    pub(super) async fn lock(&self,wait:std::time::Duration)->Result<File> {
        let owner=self.lock_file()?;
        let deadline=std::time::Instant::now()+wait;
        loop {
            match owner.try_lock() {
                Ok(())=>{super::poison::check(self)?;return Ok(owner);},
                Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now()<deadline=>tokio::time::sleep(std::time::Duration::from_millis(100)).await,
                Err(error)=>return Err(error.into()),
            }
        }
    }

    pub(super) fn directory(&self, name: &str) -> Result<PathBuf> {
        ensure!(matches!(name, "payloads"|"ops"), "Invalid publisher directory");
        let path=self.root.join(name);
        crate::db_upgrade::files::private_dir(&path)?;
        Ok(path)
    }

    pub(super) fn manifest(&self) -> Result<StoreManifest> {
        Ok(crate::db_upgrade::files::read_json(&self.root.join("manifest.json"))?)
    }

    pub(super) fn select(&self, id: &str) -> Result<()> {
        uuid(id)?;
        self.payload(id)?;
        let mut manifest=self.manifest()?;
        if manifest.current.as_deref()!=Some(id) {
            manifest.previous=manifest.current.take();
            manifest.current=Some(id.into());
            crate::db_upgrade::files::atomic_json(&self.root.join("manifest.json"), &manifest)?;
        }
        Ok(())
    }

    /// Only first adoption captures live bytes; an existing durable generation is authoritative.
    pub(super) async fn ensure_previous(&self, config:&RuntimeConfig,operation:&super::journal::Operation) -> Result<String> {
        super::poison::check(self)?;
        if let Some(id)=self.manifest()?.current {
            self.payload(&id)?;
            return Ok(id);
        }
        let files=crate::publisher::files::capture_payload_files(config).await?;
        let id=uuid::Uuid::new_v4().to_string();
        let previous=files.clone();
        let budget=super::capacity::StageBudget::new(config,operation,&previous,!super::confirmation::uncertain(config).await);
        self.stage_with_budget(&id,Origin::LegacyEvidence,None,BTreeMap::new(),files,&budget)?;
        self.select(&id)?;
        Ok(id)
    }

    /// A journaled forward or restore intent must precede the complete live-set write.
    pub(super) async fn write_live(&self, config:&RuntimeConfig, operation:&super::journal::Operation) -> Result<()> {
        use super::journal::Phase;
        super::poison::check(self)?;
        let persisted=super::journal::Operation::load(self,&operation.id)?;
        ensure!(persisted.command_digest==operation.command_digest,"Operation identity changed");
        let id=match persisted.phase {
            Phase::LiveWriting=>{
                let previous=persisted.previous.as_deref().context("Missing durable previous payload")?;
                self.payload(previous)?;
                persisted.next.as_deref().context("Missing next payload")?
            }
            Phase::RestoreWriting=>persisted.restore_target.as_deref().context("Missing durable restore target")?,
            _=>anyhow::bail!("Live write has no durable write intent"),
        };
        let payload=self.payload(id)?;
        crate::publisher::confirmation::begin_uncertain(config).await?;
        crate::publisher::files::restore_payload_files(config,&payload.files).await?;
        ensure!(crate::publisher::files::capture_payload_files(config).await?==payload.files,"Live payload does not match its durable manifest");
        Ok(())
    }

    /// Recovery chooses the entire durable generation from the persisted track, never live bytes.
    pub(super) async fn write_recovery(&self, config:&RuntimeConfig, operation:&super::journal::Operation) -> Result<String> {
        super::poison::check(self)?;
        let persisted=super::journal::Operation::load(self,&operation.id)?;
        ensure!(persisted.command_digest==operation.command_digest,"Operation identity changed");
        let id=match super::recovery::plan(&persisted)? {
            super::recovery::Action::ResumeRestore(id) if persisted.phase==super::journal::Phase::RestoreWriting=>id,
            super::recovery::Action::KeepUncertain(id) if persisted.commit_unknown && persisted.uncertain=>id,
            super::recovery::Action::Finish{payload:id,..} if persisted.phase==super::journal::Phase::DbCommitted=>id,
            _=>anyhow::bail!("Recovery has no durable live-write intent"),
        };
        let payload=self.payload(&id)?;
        crate::publisher::confirmation::begin_uncertain(config).await?;
        crate::publisher::files::restore_payload_files(config,&payload.files).await?;
        ensure!(crate::publisher::files::capture_payload_files(config).await?==payload.files,"Recovered live payload does not match its durable manifest");
        Ok(id)
    }

    /// The caller persists the operation's next id before any live mutation.
    pub(super) fn stage_with_budget(&self,id:&str,origin:Origin,model_digest:Option<String>,certificate_digests:BTreeMap<String,String>,files:BTreeMap<String,Vec<u8>>,budget:&super::capacity::StageBudget<'_>)->Result<Payload> {
        self.stage_inner(id,origin,model_digest,certificate_digests,files,None,None,Some(budget)).map_err(super::capacity::stage_error)
    }
    fn admit_capacity(&self,id:&str,files:&BTreeMap<String,Vec<u8>>,manifest_bytes:usize,budget:&super::capacity::StageBudget<'_>)->Result<()> {
        let probe=||->Result<super::capacity::Available>{
            #[cfg(test)]
            if let Some(probe)=&self.capacity_probe{return probe();}
            let root=OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC).open(&self.root)?;
            super::capacity::available(&root)
        };
        let available=probe()?;
        if super::capacity::needed(self,id,files,manifest_bytes,budget,available.block_size)?.fits(available){return Ok(());}
        super::journal::collect_completed(self,budget.a8_clear);
        let available=probe()?;
        if super::capacity::needed(self,id,files,manifest_bytes,budget,available.block_size)?.fits(available){Ok(())}else{Err(super::capacity::full())}
    }
    #[cfg(test)]
    pub(super) fn stage(&self,id:&str,origin:Origin,model_digest:Option<String>,certificate_digests:BTreeMap<String,String>,files:BTreeMap<String,Vec<u8>>) -> Result<Payload> {
        self.stage_inner(id,origin,model_digest,certificate_digests,files,None,None,None)
    }

    #[cfg(test)]
    pub(super) fn stage_derived(&self,id:&str,source:&str,reason:&str,files:BTreeMap<String,Vec<u8>>)->Result<Payload> {
        uuid(source)?;ensure!(source!=id && !reason.is_empty() && reason.len()<=1024,"Invalid derivation provenance");
        let source_payload=self.payload(source)?;
        self.stage_inner(id,Origin::Publisher,source_payload.manifest.model_digest,source_payload.manifest.certificate_digests,files,Some(source.into()),Some(reason.into()),None)
    }
    pub(super) fn stage_derived_with_budget(&self,id:&str,source:&str,reason:&str,files:BTreeMap<String,Vec<u8>>,budget:&super::capacity::StageBudget<'_>) -> Result<Payload> {
        uuid(source)?;ensure!(source!=id && !reason.is_empty() && reason.len()<=1024,"Invalid derivation provenance");
        let source_payload=self.payload(source)?;
        self.stage_inner(id,Origin::Publisher,source_payload.manifest.model_digest,source_payload.manifest.certificate_digests,files,Some(source.into()),Some(reason.into()),Some(budget)).map_err(super::capacity::stage_error)
    }

    fn stage_inner(&self,id:&str,origin:Origin,model_digest:Option<String>,certificate_digests:BTreeMap<String,String>,files:BTreeMap<String,Vec<u8>>,derived_from:Option<String>,reason:Option<String>,budget:Option<&super::capacity::StageBudget<'_>>) -> Result<Payload> {
        uuid(id)?;
        for name in files.keys() {payload_path(name)?;}
        for (id,hash) in &certificate_digests {
            uuid(id)?;
            ensure!(hash.len()==64 && hash.bytes().all(|b|b.is_ascii_hexdigit()),"Invalid certificate digest");
        }
        if let Some(hash)=&model_digest {ensure!(hash.len()==64 && hash.bytes().all(|b|b.is_ascii_hexdigit()),"Invalid model digest");}
        let entries=files.iter().map(|(name,bytes)|(name.clone(),FileDigest{sha256:digest(bytes),bytes:bytes.len() as u64})).collect();
        let manifest=PayloadManifest{version:1,id:id.into(),origin,derived_from,reason,model_digest,certificate_digests,files:entries};
        let bytes=serde_json::to_vec(&manifest)?;
        #[cfg(not(test))]
        ensure!(budget.is_some(),"Publisher stage budget is required");
        if let Some(budget)=budget {self.admit_capacity(id,&files,bytes.len(),budget)?;}
        let parent=self.directory("payloads")?;
        let root=parent.join(id);
        ensure!(!root.try_exists()?,"Immutable payload already exists");
        crate::db_upgrade::files::private_dir(&root)?;
        for (name,bytes) in &files {
            let path=root.join(name);
            let relative=Path::new(name).parent().unwrap();
            let mut directory=root.clone();
            for component in relative.components() {
                directory.push(component);
                crate::db_upgrade::files::private_dir(&directory)?;
            }
            new_private_file(&path,bytes)?;
        }
        new_private_file(&root.join("manifest.json"),&bytes)?;
        File::open(&root)?.sync_all()?;
        File::open(&parent)?.sync_all()?;
        Ok(Payload {manifest,files,manifest_sha256:digest(&bytes)})
    }

    pub(super) fn payload(&self,id:&str) -> Result<Payload> {
        uuid(id)?;
        let root=self.directory("payloads")?.join(id);
        crate::db_upgrade::files::real_path(&root)?;
        let meta=fs::symlink_metadata(&root)?;
        ensure!(meta.is_dir() && meta.uid()==0 && meta.mode()&0o7777==0o700,"Unsafe payload directory");
        let bytes=private_bytes(&root.join("manifest.json"))?;
        let manifest:PayloadManifest=serde_json::from_slice(&bytes)?;
        ensure!(manifest.version==1 && manifest.id==id,"Invalid payload manifest");
        if let Some(source)=&manifest.derived_from {uuid(source)?;ensure!(source!=id && manifest.reason.as_ref().is_some_and(|reason|!reason.is_empty()&&reason.len()<=1024),"Invalid derivation provenance");}
        let mut files=BTreeMap::new();
        for (name,expected) in &manifest.files {
            payload_path(name)?;
            let bytes=private_bytes(&root.join(name))?;
            ensure!(bytes.len() as u64==expected.bytes && digest(&bytes)==expected.sha256,"Payload digest mismatch");
            files.insert(name.clone(),bytes);
        }
        Ok(Payload {manifest,files,manifest_sha256:digest(&bytes)})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs, os::unix::fs::PermissionsExt};

    fn fixture() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        for name in [".nwui-auto", ".nwui-publisher"] {
            let dir = temp.path().join(name);
            fs::create_dir(&dir).unwrap();
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let record = json!({"version":1,"attemptId":uuid::Uuid::new_v4().to_string(),"branch":"fresh",
            "candidate":{"revision":"0".repeat(40),"pinsSha256":"1".repeat(64),"migrationsSha256":"2".repeat(64)},
            "records":[{"seq":1,"step":"s4-prepare","state":{"kind":"intent"},"at":"2026-10-28T00:00:00Z","data":{"publisherCreated":true}}],
            "moves":[],"handoff":"none","outcome":"upgrading","retry":{"consumedLabels":[],"interruptions":0,"backoffUntil":null}});
        crate::db_upgrade::files::atomic_json(&temp.path().join(".nwui-auto/journal-v1.json"), &record).unwrap();
        temp
    }

    #[tokio::test]
    async fn n1_live_writes_require_durable_intent_and_restore_the_complete_previous_generation() {
        use crate::publisher::journal::{Admission,Operation,Phase};
        let temp=fixture();let store=PublisherStore::open_at(temp.path()).unwrap();
        let mut config=RuntimeConfig::read(|key|match key {
            "DATABASE_URL"=>Some("postgres://fixture@127.0.0.1/fixture".into()),"AUTH_SECRET"=>Some("store-live-fixture".into()),_=>None,
        }).unwrap();
        config.nginx_conf_dir=temp.path().join("live/conf");config.nginx_stream_dir=temp.path().join("live/stream");config.certs_dir=temp.path().join("live/certs");
        let previous=BTreeMap::from([("conf/server-old.conf".into(),b"old http".to_vec()),("stream/stream-old.conf".into(),b"old stream".to_vec())]);
        crate::publisher::files::restore_payload_files(&config,&previous).await.unwrap();
        let before=store.ensure_previous(&config,&super::super::journal::Operation::planned(&uuid::Uuid::new_v4().to_string(),&digest(&[]))).await.unwrap();
        assert!(matches!(store.payload(&before).unwrap().manifest.origin,Origin::LegacyEvidence));
        let next=uuid::Uuid::new_v4().to_string();
        let desired=BTreeMap::from([("conf/server-new.conf".into(),b"new http".to_vec())]);
        store.stage(&next,Origin::Publisher,Some("c".repeat(64)),BTreeMap::new(),desired.clone()).unwrap();
        let Admission::New(mut operation)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&"d".repeat(64)).unwrap() else {panic!()};
        operation.previous=Some(before.clone());operation.next=Some(next);
        assert!(store.write_live(&config,&operation).await.is_err());
        assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),previous);
        operation.advance(&store,Phase::PayloadStaged).unwrap();
        assert!(store.write_live(&config,&operation).await.is_err());
        operation.advance(&store,Phase::LiveWriting).unwrap();
        store.write_live(&config,&operation).await.unwrap();
        assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),desired);
        // No re-adoption from a mixed live set once a durable generation exists.
        std::fs::write(config.nginx_conf_dir.join("server-new.conf"),b"mixed").unwrap();
        assert_eq!(store.ensure_previous(&config,&super::super::journal::Operation::planned(&uuid::Uuid::new_v4().to_string(),&digest(&[]))).await.unwrap(),before);
        operation.begin_restore(&store,&before,"test-interruption").unwrap();
        assert!(store.write_live(&config,&operation).await.is_err());
        operation.advance(&store,Phase::RestoreWriting).unwrap();
        store.write_live(&config,&operation).await.unwrap();
        assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),previous);
    }

    #[test]
    fn n1_bootstrap_commits_a_manifest_before_returning() {
        let temp = fixture();
        let store = PublisherStore::open_at(temp.path()).unwrap();
        assert!(store.root.join("manifest.json").is_file());
        assert!(!store.root.join(".manifest.init").exists());
        let manifest: serde_json::Value = crate::db_upgrade::files::read_json(&store.root.join("manifest.json")).unwrap();
        assert_eq!(manifest["version"], 1);
        assert!(PublisherStore::open_at(temp.path()).is_ok());
    }

    #[test]
    fn n1_bootstrap_finishes_its_journal_owned_partial_temp() {
        let temp = fixture();
        let path = temp.path().join(".nwui-publisher/.manifest.init");
        fs::write(&path, b"partial manifest").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let store = PublisherStore::open_at(temp.path()).unwrap();
        assert!(store.root.join("manifest.json").is_file());
        assert!(!path.exists());
    }

    #[test]
    fn n1_bootstrap_refuses_and_preserves_an_unowned_temp_or_extra_entry() {
        let temp = fixture();
        let journal = temp.path().join(".nwui-auto/journal-v1.json");
        let mut record: serde_json::Value = crate::db_upgrade::files::read_json(&journal).unwrap();
        record["records"][0]["data"]["publisherCreated"] = json!(false);
        crate::db_upgrade::files::atomic_json(&journal, &record).unwrap();
        let path = temp.path().join(".nwui-publisher/.manifest.init");
        fs::write(&path, b"not ours").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(PublisherStore::open_at(temp.path()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"not ours");
        fs::write(temp.path().join(".nwui-publisher/foreign"), b"preserve").unwrap();
        assert!(PublisherStore::open_at(temp.path()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"not ours");
    }

    #[test]
    fn n1_payload_is_immutable_verified_and_selected_only_after_complete_staging() {
        let temp=fixture();
        let store=PublisherStore::open_at(temp.path()).unwrap();
        let id=uuid::Uuid::new_v4().to_string();
        let files=BTreeMap::from([("conf/server-test.conf".into(),b"server { listen 18460; }\n".to_vec())]);
        let payload=store.stage(&id,Origin::Publisher,Some("a".repeat(64)),BTreeMap::new(),files.clone()).unwrap();
        assert_eq!(payload.files,files);
        assert_eq!(payload.manifest_sha256,store.payload(&id).unwrap().manifest_sha256);
        assert!(store.manifest().unwrap().current.is_none());
        store.select(&id).unwrap();
        assert_eq!(store.manifest().unwrap().current.as_deref(),Some(id.as_str()));
        assert!(store.stage(&id,Origin::Publisher,None,BTreeMap::new(),files).is_err());
        fs::write(store.root.join("payloads").join(&id).join("conf/server-test.conf"),b"corrupt").unwrap();
        assert!(store.payload(&id).is_err());
    }

    #[test]
    fn n1_payload_refuses_paths_and_symlinked_material_without_following_them() {
        let temp=fixture();
        let store=PublisherStore::open_at(temp.path()).unwrap();
        for name in ["../outside","conf/../outside","conf/user.conf","certs/invalid/privkey.pem","conf//server-a.conf"] {
            assert!(store.stage(&uuid::Uuid::new_v4().to_string(),Origin::Publisher,None,BTreeMap::new(),BTreeMap::from([(name.into(),b"bad".to_vec())])).is_err());
        }
        assert!(!store.root.join("payloads").exists());
        let id=uuid::Uuid::new_v4().to_string();
        store.stage(&id,Origin::LegacyEvidence,None,BTreeMap::new(),BTreeMap::from([("conf/server-a.conf".into(),b"original".to_vec())])).unwrap();
        let path=store.root.join("payloads").join(&id).join("conf/server-a.conf");
        fs::remove_file(&path).unwrap();
        let outside=temp.path().join("outside");fs::write(&outside,b"preserve").unwrap();
        std::os::unix::fs::symlink(&outside,&path).unwrap();
        assert!(store.payload(&id).is_err());
        assert_eq!(fs::read(outside).unwrap(),b"preserve");
    }
}
