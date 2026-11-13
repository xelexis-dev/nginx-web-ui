use super::{
    apply::ConfigSnapshotData,
    files::{self, ManagedKind},
    guard_targets, internal_targets,
    process::NginxProcess,
    reserved_ports::{NGINX_BASE_LISTEN_PORTS, ReservedPorts},
};
use crate::config::RuntimeConfig;
use anyhow::{Context, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub use crate::control::model::ports_for_snapshot;
use crate::control::model::process_ports;

/// Apply-time starting point: the fixed nginx listeners and this instance's own processes
/// only. The renderer adds each row it accepts, so a disabled or skipped row never blocks a
/// row that is actually rendered. Saving still uses `ports_for_snapshot`, disabled rows
/// included, because enabling that row later would collide.
pub fn ports_for_render(config: &RuntimeConfig) -> ReservedPorts {
    ports_for_snapshot(config, &ConfigSnapshotData::default())
}

pub use crate::control::model::GuardReport;

fn listen_ports(content:&[u8],tcp_only:bool)->Vec<u16> {
    guard_targets::listen_ports(&String::from_utf8_lossy(content),tcp_only)
}

pub(super) async fn isolate_with_writer(
    config:&RuntimeConfig,root:&Path,label:&str,conf:&BTreeMap<String,Vec<u8>>,stream:&BTreeMap<String,Vec<u8>>,
    writer:Option<&mut super::derivation::Derivation>,
)->Result<Option<PathBuf>> {
    if conf.is_empty() && stream.is_empty() {
        return Ok(None);
    }
    let dir = root.join(format!(
        "{}-{}-{label}",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
        uuid::Uuid::new_v4()
    ));
    // First persist every recovery copy, then remove any live configuration.
    for (sub, files) in [("conf.d", conf), ("stream.d", stream)] {
        for (name, bytes) in files {
            files::atomic_write(&dir.join(sub).join(name), bytes, 0o600).await?;
        }
    }
    if let Some(writer)=writer {
        let mut live=super::files::RenderedConfig {
            conf:files::managed_files(&config.nginx_conf_dir,ManagedKind::Conf).await?,
            stream:files::managed_files(&config.nginx_stream_dir,ManagedKind::Stream).await?,
        };
        live.conf.retain(|name,_|!conf.contains_key(name));live.stream.retain(|name,_|!stream.contains_key(name));
        writer.write(config,&live,&format!("guard-{label}")).await?;
        return Ok(Some(dir));
    }
    for (source, files) in [
        (&config.nginx_conf_dir, conf),
        (&config.nginx_stream_dir, stream),
    ] {
        for name in files.keys() {
            if let Err(error) = tokio::fs::remove_file(source.join(name)).await {
                restore_isolated(config, conf, stream)
                    .await
                    .context("Quarantine failed and files could not be restored")?;
                return Err(error.into());
            }
        }
    }
    Ok(Some(dir))
}
async fn restore_isolated(
    config: &RuntimeConfig,
    conf: &BTreeMap<String, Vec<u8>>,
    stream: &BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    for (dir, files) in [
        (&config.nginx_conf_dir, conf),
        (&config.nginx_stream_dir, stream),
    ] {
        for (name, bytes) in files {
            files::atomic_write(&dir.join(name), bytes, 0o644).await?;
        }
    }
    Ok(())
}

async fn scan_internal_targets(
    config: RuntimeConfig,
    conf: BTreeMap<String, Vec<u8>>,
    stream: BTreeMap<String, Vec<u8>>,
    resolve: impl Fn(&str, u16) -> Vec<std::net::IpAddr> + Send + 'static,
) -> Result<(BTreeMap<String, Vec<u8>>, BTreeMap<String, Vec<u8>>, guard_targets::TargetScan,Vec<String>)> {
    Ok(internal_targets::blocking(move || {
        let mut scan = guard_targets::scan(&config, &conf, &stream, &resolve);
        let actions=super::unix_targets::extend_scan(&conf,&stream,&mut scan);
        (conf, stream, scan,super::unix_targets::notices(&actions))
    }).await?)
}

/// Names resolve again every time nginx starts, so a target that apply accepted can reach this
/// container's web, API or database port by now. Moves those files out the same way.
async fn isolate_internal_targets(
    config: &RuntimeConfig,
    quarantine: &Path,
    report: &mut GuardReport,
    mut writer:Option<&mut super::derivation::Derivation>,
) -> Result<Vec<String>> {
    let conf = files::managed_files(&config.nginx_conf_dir, ManagedKind::Conf).await?;
    let stream = files::managed_files(&config.nginx_stream_dir, ManagedKind::Stream).await?;
    let (conf, stream, scan,notices) = scan_internal_targets(config.clone(), conf, stream, internal_targets::system_resolve).await?;
    if scan.is_empty() {
        return Ok(notices);
    }
    let pick = |all: &BTreeMap<String, Vec<u8>>, names: &BTreeSet<String>| -> BTreeMap<String, Vec<u8>> {
        all.iter()
            .filter(|(name, _)| names.contains(*name))
            .map(|(name, bytes)| (name.clone(), bytes.clone()))
            .collect()
    };
    let (bad_conf, bad_stream) = (pick(&conf, &scan.conf), pick(&stream, &scan.stream));
    let Some(dir) = isolate_with_writer(config, quarantine, "internal-target", &bad_conf, &bad_stream,writer.as_deref_mut()).await?
    else {
        return Ok(notices);
    };
    // The original upstreams.conf is in the recovery directory; the other blocks go back live.
    if let Some(kept) = &scan.upstreams {
        let path = config.nginx_conf_dir.join(guard_targets::UPSTREAMS_FILE);
        let write=if let Some(writer)=writer.as_deref_mut() {
            let mut live=super::files::RenderedConfig {conf:files::managed_files(&config.nginx_conf_dir,ManagedKind::Conf).await?,stream:files::managed_files(&config.nginx_stream_dir,ManagedKind::Stream).await?};
            live.conf.insert(guard_targets::UPSTREAMS_FILE.into(),kept.clone());writer.write(config,&live,"guard-internal-target").await
        } else {files::atomic_write(&path,kept,0o644).await};
        if let Err(error) = write {
            if writer.is_some() {return Err(error);}
            restore_isolated(config, &bad_conf, &bad_stream)
                .await
                .context("Could not write upstreams.conf back and files could not be restored")?;
            return Err(error);
        }
    }
    // upstreams.conf itself is not out: only its removed blocks are, counted by name.
    report.internal_target_files =
        bad_conf.len() + bad_stream.len() - usize::from(scan.upstreams.is_some());
    report.removed_upstreams = scan.removed_upstreams;
    report.quarantine_dirs.push(dir);
    Ok(notices)
}

/// Run from the pre-nginx CLI/s6 guard, never as a live API request. No DB or
/// reload is involved; callers hold `nginx_lock`. HTTP takes priority over conflicting
/// TCP streams; UDP listeners remain independent. User-created nonmanaged files stay in place.
#[cfg(test)]
pub async fn guard_before_start(
    config: &RuntimeConfig,
    nginx_conf: &Path,
    quarantine: &Path,
) -> Result<GuardReport> {
    files::clean_interrupted_writes(config).await?;
    guard_managed_files(config, nginx_conf, quarantine).await
}

/// The same guard for an nginx restart while the API runs (`nwui-api guard` from
/// nginx/run). It leaves interrupted-write cleanup to boot: a temporary file may
/// belong to an apply that is still writing.
#[cfg(test)]
pub async fn guard_managed_files(
    config: &RuntimeConfig,
    nginx_conf: &Path,
    quarantine: &Path,
) -> Result<GuardReport> {
    guard_inner(config,nginx_conf,quarantine,None).await
}
pub(super) async fn guard_durable(config:&RuntimeConfig,nginx_conf:&Path,quarantine:&Path,store:&super::store::PublisherStore,guard_parent:Option<u32>)->Result<GuardReport> {
    let planned=super::journal::Operation::planned(&uuid::Uuid::new_v4().to_string(),&super::store::digest(&[]));
    store.ensure_previous(config,&planned).await?;
    let mut writer=super::derivation::Derivation::new(store)?;
    let attempt=async {
        let report=guard_inner(config,nginx_conf,quarantine,Some(&mut writer)).await?;
        writer.finish(config).await?;
        Ok::<_,anyhow::Error>(report)
    }.await;
    if attempt.is_err() {super::recovery::before_start_with_parent(config,store,guard_parent).await.map_err(|error|super::store::step_error("restore_incomplete",error))?;}
    attempt
}
async fn guard_inner(config:&RuntimeConfig,nginx_conf:&Path,quarantine:&Path,mut writer:Option<&mut super::derivation::Derivation>)->Result<GuardReport> {
    super::confirmation::begin_uncertain(config).await?;
    super::confirmation::publish(config,&uuid::Uuid::new_v4().to_string()).await?;
    let mut report = GuardReport::default();
    let conf = files::managed_files(&config.nginx_conf_dir, ManagedKind::Conf).await?;
    let stream = files::managed_files(&config.nginx_stream_dir, ManagedKind::Stream).await?;
    let mut processes = process_ports(config);
    let dns=crate::dns_guard::status(config);
    let dns_port=dns.available.then_some(dns.endpoint.port());
    if let Some(port)=dns_port { processes.push(port); }
    let mut http: BTreeSet<u16> = match tokio::fs::read(nginx_conf).await {
        Ok(content) => listen_ports(&content, false).into_iter().collect(),
        Err(e) => {
            eprintln!("[nginx-guard] could not read base listeners: {e}");
            BTreeSet::new()
        }
    };
    if http.is_empty() {
        eprintln!("[nginx-guard] no base listeners found; using image defaults");
        http.extend(NGINX_BASE_LISTEN_PORTS);
    }
    let mut bad_conf = BTreeMap::new();
    let mut bad_stream = BTreeMap::new();
    for (name, bytes) in &conf {
        if !name.starts_with("server-") {
            continue;
        }
        let ports = listen_ports(bytes, false);
        if ports.iter().any(|p| processes.contains(p) || *p == super::self_proxy::ADMIN_UI_PORT) {
            bad_conf.insert(name.clone(), bytes.clone());
        } else {
            http.extend(ports);
        }
    }
    for (name, bytes) in &stream {
        if listen_ports(bytes, true)
            .iter()
            .any(|p| processes.contains(p) || http.contains(p))
            || listen_ports(bytes, false).iter().any(|p|Some(*p)==dns_port)
        {
            bad_stream.insert(name.clone(), bytes.clone());
        }
    }
    if let Some(dir) = isolate_with_writer(config, quarantine, "reserved", &bad_conf, &bad_stream,writer.as_deref_mut()).await? {
        report.reserved_files = bad_conf.len() + bad_stream.len();
        report.quarantine_dirs.push(dir);
    }
    let isolation_notices=isolate_internal_targets(config, quarantine, &mut report,writer.as_deref_mut()).await?;
    let process = NginxProcess::new(&config.nginx_bin);
    let contained = if let Some(writer)=writer.as_deref_mut() {super::containment::contain_on_disk_durable(config,quarantine,writer).await?}else{super::containment::contain_on_disk(config, quarantine).await?};
    report.validation_output = contained.validation.output;
    report.quarantine_dirs.extend(contained.quarantine_dirs);
    for action in contained.actions {
        match action.unit {
            super::containment::Unit::ConfFile(name) | super::containment::Unit::StreamFile(name) => report.rejected_files.push(name),
            super::containment::Unit::UpstreamBlock(name) => report.rejected_upstreams.push(name),
        }
    }
    if contained.validation.ok {
        for notice in isolation_notices {report.validation_output.push_str(&format!("\n{notice}"));}
        return Ok(report);
    }
    let conf = files::managed_files(&config.nginx_conf_dir, ManagedKind::Conf).await?;
    let stream = files::managed_files(&config.nginx_stream_dir, ManagedKind::Stream).await?;
    if let Some(dir) = isolate_with_writer(config, quarantine, "invalid", &conf, &stream,writer.as_deref_mut()).await? {
        let second = if let Some(writer)=writer.as_deref_mut() {writer.validate(config).await}else{process.validate().await};
        report.validation_output = second.output;
        if second.ok {
            report.invalid_files = conf.len() + stream.len();
            report.quarantine_dirs.push(dir);
        } else {
            if writer.is_some() {anyhow::bail!("Base nginx validation failed after durable correction: {}",report.validation_output);}
            restore_isolated(config, &conf, &stream).await.context(
                "Base nginx validation still failed and managed files could not be restored",
            )?;
        }
    }
    for notice in isolation_notices {report.validation_output.push_str(&format!("\n{notice}"));}
    Ok(report)
}

#[cfg(test)]
mod scheduler_tests {
    #[tokio::test(flavor = "current_thread")]
    async fn boot_target_scan_does_not_park_the_async_executor() {
        use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
        let mut config = crate::config::RuntimeConfig::read(|key| match key {
            "DATABASE_URL" => Some("postgres://test:test@127.0.0.1/test".into()),
            "AUTH_SECRET" => Some("test-only".into()),
            "NWUI_CONTAINER" => Some("1".into()),
            _ => None,
        }).unwrap();
        let dir = tempfile::tempdir().unwrap();
        config.target_dns.descriptor_path = dir.path().join("target-dns.json");
        let conf = std::collections::BTreeMap::from([("server-slow.conf".into(), b"server { location / { proxy_pass http://slow.example:3001; } }".to_vec())]);
        let progressed = Arc::new(AtomicBool::new(false));
        let observer = progressed.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            ready.await.unwrap();
            observer.store(true, Ordering::SeqCst);
        });
        let started = std::sync::Mutex::new(Some(started));
        let responsive = progressed.clone();
        let (_, _, scan, _) = super::scan_internal_targets(config, conf, Default::default(), move |_, _| {
            if let Some(tx) = started.lock().unwrap().take() { tx.send(()).unwrap(); }
            // Wait for the async observer, with a bound so the broken implementation fails.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
            while !responsive.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert!(responsive.load(Ordering::SeqCst), "boot DNS scan blocked the async executor");
            vec!["127.0.0.1".parse().unwrap()]
        }).await.unwrap();
        task.await.unwrap();
        assert!(scan.conf.contains("server-slow.conf"), "offloading must retain internal-target rejection");
    }
}

#[cfg(test)]
#[path="g13_bootstrap_kernel_tests.rs"]
mod g13_kernel;
