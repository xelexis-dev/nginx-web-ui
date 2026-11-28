//! Test-only publisher fixtures; no direct writer-module imports.
#![allow(dead_code)]
use nwui_api::{config::RuntimeConfig, control::model::{self, CertificateRef, ConfigSnapshotData, RenderOutcome}, state::AppState};
use std::{path::Path, os::unix::fs::PermissionsExt};

pub async fn isolated(name: &str) -> bool {
    if std::env::var_os("NWUI_G13_BIN_DIR").is_some() { return false; }
    let root = tempfile::tempdir().unwrap();
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--include-ignored", "--nocapture", "--test-threads=1"])
        .env("NWUI_G13_BIN_DIR", root.path())
        .env("PATH", format!("{}:/usr/sbin:/usr/bin:/sbin:/bin", root.path().display()))
        .kill_on_drop(true).output().await.unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{name}: {stdout}\n{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.lines().any(|line| line.starts_with("test result: ok. 1 passed; 0 failed;")), "{name}: expected one child test: {stdout}");
    true
}

pub fn metadata(bin: &Path, main: &Path) {
    let original = std::fs::read_to_string(bin).unwrap();
    if original.contains("# G13 fixture metadata") { return; }
    let prefix = format!(r#"#!/bin/sh
# G13 fixture metadata: preserve the original validation/fault body below.
if [ "$1" = -V ]; then printf '%s\n' 'nginx version: nginx/1.30.6' 'configure arguments: --prefix={prefix} --conf-path={main}' >&2; exit 0; fi
for arg do
  if [ "$arg" = -T ]; then printf '# configuration file %s:\n' '{main}'; fi
done
"#, prefix=main.parent().unwrap().display(), main=main.display());
    std::fs::write(bin, prefix + original.strip_prefix("#!/bin/sh\n").unwrap_or(&original)).unwrap();
    std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o700)).unwrap();
}

pub fn witness(state: &mut AppState) {
    super::auth_support::nginx::witness(state);
    metadata(&state.config.nginx_bin, &state.config.nginx_main_conf);
}

pub fn render(config: &RuntimeConfig, data: &ConfigSnapshotData, certs: &[CertificateRef]) -> RenderOutcome {
    model::render_preview_with_context(config, data, certs, &nwui_api::dns_guard::status(config)).unwrap()
}

pub async fn baseline(state: &AppState) -> anyhow::Result<model::ApplyResult> {
    use nwui_api::publisher::{boot, test_support};
    let directory = std::env::var_os("NWUI_G13_BIN_DIR").or_else(|| std::env::var_os("NWUI_PUBLISHER_BOOT_CHILD"))
        .expect("no-actor boot fixture must use an isolated child PATH");
    let master = std::path::PathBuf::from(directory).join("nginx");
    if !master.exists() { std::fs::copy(&state.config.nginx_bin, &master)?; }
    metadata(&master, &state.config.nginx_main_conf);
    metadata(&state.config.nginx_bin, &state.config.nginx_main_conf);
    let store = test_support::open_store_fixture(state.config.data_key_file.parent().unwrap())?;
    let staged = boot::prepare_payload(&state.config, &state.secrets, boot::PgSource::Pool(&state.db), &store,nwui_api::publisher::boot::BootMode::Conversion).await?;
    let report = publish_stopped(state, &state.config, &store, &staged.id).await?;
    test_support::adopt_current(state, &store).await?;
    Ok(model::ApplyResult {ok:true, output:report.validation_output, rolled_back_to:None})
}

pub async fn publish_stopped(state:&AppState, config:&RuntimeConfig, store:&nwui_api::publisher::PublisherStore, id:&str)
    -> anyhow::Result<nwui_api::publisher::boot::PublishReport>
{
    let root = state.config.nginx_conf_dir.parent().unwrap();
    let pid = root.join(".fake-nginx.pid");
    let running = match std::fs::read(&pid) {
        Ok(bytes) => { std::fs::remove_file(&pid)?; Some(bytes) },
        Err(error) if error.kind()==std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let group = pid.with_extension("group");
    let group_bytes = if running.is_some() {
        let bytes = std::fs::read(&group)?;
        std::fs::remove_file(&group)?;
        Some(bytes)
    } else {
        None
    };
    let active = root.join(".fake-nginx-active-generation");
    let before = std::fs::read(&active).ok();
    let publication = nwui_api::publisher::boot::publish_live(config,store,id).await;
    if publication.is_ok() {
        assert!(state.config.nginx_conf_dir.join(".nwui-apply-uncertain").exists());
        assert_eq!(std::fs::read(&active).ok(),before,"boot publication must not HUP/acknowledge");
    }
    if let Some(bytes)=group_bytes { std::fs::write(&group,bytes)?; }
    if let Some(bytes)=running { std::fs::write(&pid,bytes)?; }
    Ok(publication?)
}
