//! Managed-file ownership and atomic replacement. Unmanaged files are never removed.
use crate::config::RuntimeConfig;
#[cfg(test)]
use crate::crypto::SecretBox;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use tokio::{fs, io::AsyncWriteExt};

pub use crate::control::model::RenderedConfig;

// Intentionally no Debug: this structure contains private key material.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CertificateMaterial {
    pub id: String,
    pub cert_pem: String,
    pub key_pem: String,
}

#[derive(Clone, Copy)]
pub enum ManagedKind {
    Conf,
    Stream,
}
impl ManagedKind {
    pub fn matches(self, name: &str) -> bool {
        let prefix = match self {
            Self::Conf => "server-",
            Self::Stream => "stream-",
        };
        (matches!(self, Self::Conf) && name == "upstreams.conf")
            || (name.starts_with(prefix)
                && name.ends_with(".conf")
                && name.len() > prefix.len() + 5)
    }
}

pub fn safe_id(id: &str) -> Result<&str> {
    ensure!(
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
        "Invalid managed resource id"
    );
    Ok(id)
}
pub fn cert_file_paths(config:&RuntimeConfig,id:&str) -> Result<(PathBuf,PathBuf)> {
    Ok(crate::control::model::cert_file_paths(config,id)?)
}
fn certificate_directory(name: &str) -> bool {
    uuid::Uuid::parse_str(name).is_ok() && name.len() == 36
}

/// Only call before writers start, or under the global apply lock. Exact UUID
/// names identify our interrupted atomic writes; unrelated files stay untouched.
#[cfg(test)]
async fn clean_temporary_in(directory: &Path) -> Result<()> {
    if !fs::try_exists(directory).await? { return Ok(()); }
    real_directory(directory).await?;
    let mut entries = fs::read_dir(directory).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let owned = name.strip_prefix(".nwui-").and_then(|n| n.strip_suffix(".tmp"))
            .is_some_and(certificate_directory);
        if owned && !entry.file_type().await?.is_dir() {
            // unlink never traverses a symlink target.
            fs::remove_file(entry.path()).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub async fn clean_interrupted_writes(config: &RuntimeConfig) -> Result<()> {
    clean_temporary_in(&config.nginx_conf_dir).await?;
    clean_temporary_in(&config.nginx_stream_dir).await?;
    if !fs::try_exists(&config.certs_dir).await? { return Ok(()); }
    real_directory(&config.certs_dir).await?;
    let mut entries = fs::read_dir(&config.certs_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_name().to_str().is_some_and(certificate_directory)
            && entry.file_type().await?.is_dir() {
            clean_temporary_in(&entry.path()).await?;
        }
    }
    Ok(())
}

/// Refuse symlink traversal for owned files/directories. The container owns these
/// directories; an unexpected link is an error, never a reason to touch its target.
pub(crate) async fn real_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path).await {
        Ok(meta) => ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "Expected a real directory: {}",
            path.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::create_dir_all(path).await?,
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

pub async fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path.parent().context("Managed file has no parent")?;
    real_directory(parent).await?;
    if let Ok(meta) = fs::symlink_metadata(path).await {
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "Expected a regular managed file: {}",
            path.display()
        );
        // An apply writes every managed file but usually changes one: each rewrite costs a
        // temporary file, an fsync and a rename. Identical content with the same mode is left
        // alone (a drifted mode, e.g. on a private key, is still rewritten).
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, MetadataExt};
            if meta.uid() == 0 && meta.gid() == 0 && meta.nlink() == 1 && meta.permissions().mode() & 0o777 == mode
                && meta.len() == bytes.len() as u64
                && fs::read(path).await.is_ok_and(|current| current == bytes)
            {
                return Ok(());
            }
        }
    }
    let temporary = parent.join(format!(".nwui-{}.tmp", uuid::Uuid::new_v4()));
    let result = async {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(mode);
        let mut file = options.open(&temporary).await?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            if unsafe {libc::fchown(file.as_raw_fd(),0,0)} != 0 {
                return Err(std::io::Error::last_os_error()).context("Could not set root ownership on managed output");
            }
        }
        file.write_all(bytes).await?;
        file.sync_all().await?;
        drop(file);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temporary, std::fs::Permissions::from_mode(mode)).await?;
        }
        fs::rename(&temporary, path).await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(&temporary).await;
    }
    result.with_context(|| format!("Could not atomically write {}", path.display()))
}

pub async fn managed_files(dir: &Path, kind: ManagedKind) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut names = match fs::read_dir(dir).await {
        Ok(names) => names,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e.into()),
    };
    let mut files = BTreeMap::new();
    while let Some(entry) = names.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !kind.matches(&name) {
            continue;
        }
        ensure!(
            entry.file_type().await?.is_file(),
            "Managed config is not a regular file: {name}"
        );
        files.insert(name, fs::read(entry.path()).await?);
    }
    Ok(files)
}

pub async fn replace_managed(
    dir: &Path,
    kind: ManagedKind,
    desired: &BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    for name in desired.keys() {
        ensure!(
            kind.matches(name) && !name.contains(['/', '\\', '\0']),
            "Invalid managed config filename"
        );
    }
    real_directory(dir).await?;
    let previous = managed_files(dir, kind).await?;
    for (name, bytes) in desired {
        atomic_write(&dir.join(name), bytes, 0o644).await?;
    }
    for name in previous.keys().filter(|name| !desired.contains_key(*name)) {
        fs::remove_file(dir.join(name)).await?;
    }
    Ok(())
}

pub struct FileBackup {
    pub conf: BTreeMap<String, Vec<u8>>,
    pub stream: BTreeMap<String, Vec<u8>>,
    // Plaintext private keys exist only in memory and on their private volume.
    certificates: BTreeMap<String, BTreeMap<String, Vec<u8>>>,
}
impl FileBackup {
    pub async fn capture(config: &RuntimeConfig) -> Result<Self> {
        let conf = managed_files(&config.nginx_conf_dir, ManagedKind::Conf).await?;
        let stream = managed_files(&config.nginx_stream_dir, ManagedKind::Stream).await?;
        let mut certificates = BTreeMap::new();
        let mut entries = match fs::read_dir(&config.certs_dir).await {
            Ok(entries) => Some(entries),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        if let Some(entries) = &mut entries {
            while let Some(entry) = entries.next_entry().await? {
                let id = entry.file_name().to_string_lossy().into_owned();
                if !certificate_directory(&id) {
                    continue;
                }
                ensure!(
                    entry.file_type().await?.is_dir(),
                    "Certificate path is not a real directory"
                );
                let mut files = BTreeMap::new();
                for name in ["fullchain.pem", "privkey.pem"] {
                    let path = entry.path().join(name);
                    match fs::symlink_metadata(&path).await {
                        Ok(meta) => {
                            ensure!(
                                meta.is_file() && !meta.file_type().is_symlink(),
                                "Certificate material is not a regular file"
                            );
                            files.insert(name.into(), fs::read(path).await?);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e.into()),
                    }
                }
                certificates.insert(id, files);
            }
        }
        Ok(Self {
            conf,
            stream,
            certificates,
        })
    }

    pub async fn restore(&self, config: &RuntimeConfig) -> Result<()> {
        // Attempt every restoration even if one directory has become unwritable.
        let mut failures = Vec::new();
        if let Err(e) = replace_managed(&config.nginx_conf_dir, ManagedKind::Conf, &self.conf).await
        {
            failures.push(e.to_string());
        }
        if let Err(e) =
            replace_managed(&config.nginx_stream_dir, ManagedKind::Stream, &self.stream).await
        {
            failures.push(e.to_string());
        }
        if let Err(e) = self.restore_certificates(config).await {
            failures.push(e.to_string());
        }
        if !failures.is_empty() {
            bail!("File rollback failed: {}", failures.join("; "));
        }
        Ok(())
    }
    async fn restore_certificates(&self, config: &RuntimeConfig) -> Result<()> {
        real_directory(&config.certs_dir).await?;
        clean_certificates(
            &config.certs_dir,
            &self.certificates.keys().cloned().collect(),
        )
        .await?;
        for (id, files) in &self.certificates {
            let dir = config.certs_dir.join(id);
            private_directory(&dir).await?;
            for name in ["fullchain.pem", "privkey.pem"] {
                if let Some(bytes) = files.get(name) {
                    atomic_write(
                        &dir.join(name),
                        bytes,
                        if name == "privkey.pem" { 0o600 } else { 0o644 },
                    )
                    .await?;
                } else {
                    remove_if_exists(&dir.join(name)).await?;
                }
            }
        }
        Ok(())
    }
}
async fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
async fn private_directory(path: &Path) -> Result<()> {
    real_directory(path).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
    }
    Ok(())
}
async fn clean_certificates(root: &Path, keep: &BTreeSet<String>) -> Result<()> {
    let mut entries = fs::read_dir(root).await?;
    while let Some(entry) = entries.next_entry().await? {
        let id = entry.file_name().to_string_lossy().into_owned();
        if !certificate_directory(&id) || keep.contains(&id) {
            continue;
        }
        ensure!(
            entry.file_type().await?.is_dir(),
            "Certificate path is not a real directory"
        );
        for name in ["fullchain.pem", "privkey.pem"] {
            remove_if_exists(&entry.path().join(name)).await?;
        }
        // Preserve unexpected user files; only remove an empty managed directory.
        match fs::remove_dir(entry.path()).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
pub async fn materialize_certificates(
    config: &RuntimeConfig,
    secrets: &SecretBox,
    certs: &[CertificateMaterial],
) -> Result<()> {
    clean_interrupted_writes(config).await?;
    // Decrypt every certificate before modifying any file. Broken sealed values
    // retain existing files, exactly as the previous materializer did.
    let mut opened = Vec::new();
    let mut keep = BTreeSet::new();
    for cert in certs {
        ensure!(certificate_directory(&cert.id), "Invalid certificate id");
        keep.insert(cert.id.clone());
        match secrets.open_maybe_plain(Some(&cert.key_pem)) {
            Some(key) if !key.is_empty() => opened.push((cert, key)),
            _ => eprintln!(
                "[certs] {}: private key could not be opened; keeping existing files",
                cert.id
            ),
        }
    }
    private_directory(&config.certs_dir).await?;
    for (cert, key) in opened {
        let (cert_path, key_path) = cert_file_paths(config, &cert.id)?;
        private_directory(cert_path.parent().unwrap()).await?;
        atomic_write(&cert_path, cert.cert_pem.as_bytes(), 0o644).await?;
        atomic_write(&key_path, key.as_bytes(), 0o600).await?;
    }
    clean_certificates(&config.certs_dir, &keep).await
}

pub async fn write_rendered(config: &RuntimeConfig, rendered: &RenderedConfig) -> Result<()> {
    replace_managed(&config.nginx_conf_dir, ManagedKind::Conf, &rendered.conf).await?;
    replace_managed(
        &config.nginx_stream_dir,
        ManagedKind::Stream,
        &rendered.stream,
    )
    .await
}

/// Read references from the live files, including consumers absent from the saved model.
pub(crate) fn certificate_consumers(
    config: &RuntimeConfig,
    rendered: &RenderedConfig,
) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let mut consumers = BTreeMap::<String, BTreeSet<String>>::new();
    for (name, bytes) in rendered.conf.iter().chain(&rendered.stream) {
        let text = std::str::from_utf8(bytes).context("Invalid live config encoding")?;
        for statement in super::guard_targets::statements(text) {
            if !matches!(statement.words.first().map(String::as_str), Some("ssl_certificate" | "ssl_certificate_key")) {
                continue;
            }
            ensure!(statement.words.len() == 2, "Invalid live certificate reference");
            let path = Path::new(&statement.words[1]);
            let Ok(relative) = path.strip_prefix(&config.certs_dir) else { continue };
            let mut parts = relative.components();
            let id = parts.next().and_then(|p| p.as_os_str().to_str()).context("Invalid live certificate id")?;
            ensure!(certificate_directory(id), "Invalid live certificate id");
            let (cert, key) = cert_file_paths(config, id)?;
            ensure!(path == cert || path == key, "Invalid live certificate path");
            consumers.entry(id.to_owned()).or_default().insert(name.clone());
        }
    }
    Ok(consumers)
}

/// Capture once before the intent; recovery never learns a generation from live files.
pub(crate) async fn capture_payload_files(config: &RuntimeConfig) -> Result<BTreeMap<String,Vec<u8>>> {
    let backup = FileBackup::capture(config).await?;
    let mut files = BTreeMap::new();
    for (prefix, entries) in [("conf",backup.conf),("stream",backup.stream)] {
        files.extend(entries.into_iter().map(|(name,bytes)|(format!("{prefix}/{name}"),bytes)));
    }
    for (id, entries) in backup.certificates {
        files.extend(entries.into_iter().map(|(name,bytes)|(format!("certs/{id}/{name}"),bytes)));
    }
    let runtime = config.nginx_conf_dir.join("00-runtime-upstreams.conf");
    match fs::symlink_metadata(&runtime).await {
        Ok(meta) => {
            ensure!(meta.is_file() && !meta.file_type().is_symlink(), "Invalid runtime upstream file");
            files.insert("conf/00-runtime-upstreams.conf".into(),fs::read(&runtime).await?);
        }
        Err(error) if error.kind()==std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(error.into()),
    }
    for name in files.keys() {crate::publisher::validate_payload_path(name)?;}
    Ok(files)
}

/// Input is re-verified by the store before this complete, idempotent restoration.
pub(crate) async fn restore_payload_files(config: &RuntimeConfig, files: &BTreeMap<String,Vec<u8>>) -> Result<()> {
    let mut backup = FileBackup {conf:BTreeMap::new(),stream:BTreeMap::new(),certificates:BTreeMap::new()};
    let mut runtime = None;
    for (name,bytes) in files {
        crate::publisher::validate_payload_path(name)?;
        let parts:Vec<_> = name.split('/').collect();
        match parts.as_slice() {
            ["conf","00-runtime-upstreams.conf"] => runtime=Some(bytes),
            ["conf",name] => {backup.conf.insert((*name).into(),bytes.clone());},
            ["stream",name] => {backup.stream.insert((*name).into(),bytes.clone());},
            ["certs",id,name] => {backup.certificates.entry((*id).into()).or_default().insert((*name).into(),bytes.clone());},
            _ => unreachable!("payload_path checked the shape"),
        }
    }
    let mut errors=Vec::new();
    if let Err(error)=backup.restore(config).await {errors.push(error.to_string());}
    let runtime_path=config.nginx_conf_dir.join("00-runtime-upstreams.conf");
    let restored=match runtime {
        Some(bytes)=>atomic_write(&runtime_path,bytes,0o644).await,
        None=>remove_if_exists(&runtime_path).await,
    };
    if let Err(error)=restored {errors.push(error.to_string());}
    // File fsync precedes rename; directory fsync makes rename and removal durable.
    let roots=vec![(config.nginx_conf_dir.clone(),false),(config.nginx_stream_dir.clone(),false),(config.certs_dir.clone(),true)];
    let synced=tokio::task::spawn_blocking(move || -> Result<()> {
        let mut errors=Vec::new();
        for (root,certificates) in roots {
            let result=(|| -> Result<()> {
                if root.exists() {
                    crate::db_upgrade::files::real_path(&root)?;
                    if certificates {
                        for entry in std::fs::read_dir(&root)? {
                            let entry=entry?;
                            if entry.file_type()?.is_dir() && entry.file_name().to_str().is_some_and(certificate_directory) {
                                std::fs::File::open(entry.path())?.sync_all()?;
                            }
                        }
                    }
                    std::fs::File::open(&root)?.sync_all()?;
                    if let Some(parent)=root.parent() {std::fs::File::open(parent)?.sync_all()?;}
                }
                Ok(())
            })();
            if let Err(error)=result {errors.push(error.to_string());}
        }
        ensure!(errors.is_empty(),"Could not sync live payload: {}",errors.join("; "));
        Ok(())
    }).await;
    match synced {Ok(Ok(()))=>{},Ok(Err(error))=>errors.push(error.to_string()),Err(error)=>errors.push(error.to_string())}
    ensure!(errors.is_empty(),"Payload restoration failed: {}",errors.join("; "));
    Ok(())
}

#[cfg(all(test, unix))]
mod durable_files_tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt,PermissionsExt};

    fn config(root:&Path) -> RuntimeConfig {
        let mut config=RuntimeConfig::read(|key| match key {
            "DATABASE_URL"=>Some("postgres://fixture@127.0.0.1/fixture".into()),
            "AUTH_SECRET"=>Some("durable-files-fixture".into()),_=>None,
        }).unwrap();
        config.nginx_conf_dir=root.join("conf");config.nginx_stream_dir=root.join("stream");config.certs_dir=root.join("certs");
        config
    }

    #[tokio::test]
    async fn same_content_still_replaces_api_owned_or_hardlinked_managed_inodes() {
        let temp=tempfile::tempdir().unwrap();
        let path=temp.path().join("server-owned.conf");
        std::fs::write(&path,b"same bytes").unwrap();
        let cpath=std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        assert_eq!(unsafe {libc::chown(cpath.as_ptr(),10001,10001)},0);
        atomic_write(&path,b"same bytes",0o644).await.unwrap();
        let meta=std::fs::metadata(&path).unwrap();
        assert_eq!((meta.uid(),meta.gid(),meta.nlink(),meta.permissions().mode()&0o777),(0,0,1,0o644));
        let link=temp.path().join("old-linked-inode");
        std::fs::hard_link(&path,&link).unwrap();
        atomic_write(&path,b"same bytes",0o644).await.unwrap();
        assert_ne!(std::fs::metadata(&path).unwrap().ino(),std::fs::metadata(&link).unwrap().ino());
        assert_eq!(std::fs::read(link).unwrap(),b"same bytes");
    }

    #[tokio::test]
    async fn durable_previous_restores_every_managed_byte_and_preserves_unmanaged_files() {
        let temp=tempfile::tempdir().unwrap();let config=config(temp.path());
        let id=uuid::Uuid::new_v4().to_string();
        let previous=BTreeMap::from([
            ("conf/upstreams.conf".into(),b"upstream old {}".to_vec()),
            ("conf/server-old.conf".into(),b"old http".to_vec()),
            ("conf/00-runtime-upstreams.conf".into(),b"old runtime".to_vec()),
            ("stream/stream-old.conf".into(),b"old stream".to_vec()),
            (format!("certs/{id}/fullchain.pem"),b"old cert".to_vec()),
            (format!("certs/{id}/privkey.pem"),b"old key".to_vec()),
        ]);
        restore_payload_files(&config,&previous).await.unwrap();
        atomic_write(&config.nginx_conf_dir.join("user.conf"),b"unmanaged",0o644).await.unwrap();
        let saved=capture_payload_files(&config).await.unwrap();assert_eq!(saved,previous);
        let different=BTreeMap::from([("conf/server-new.conf".into(),b"new http".to_vec())]);
        restore_payload_files(&config,&different).await.unwrap();
        restore_payload_files(&config,&saved).await.unwrap();
        assert_eq!(capture_payload_files(&config).await.unwrap(),previous);
        assert!(!config.nginx_conf_dir.join("server-new.conf").exists());
        assert_eq!(std::fs::read(config.nginx_conf_dir.join("user.conf")).unwrap(),b"unmanaged");
        assert!(restore_payload_files(&config,&BTreeMap::from([("conf/../outside".into(),vec![])])).await.is_err());
    }
}

#[cfg(test)]
#[path="g13_files_kernel_tests.rs"]
mod g13_kernel;
