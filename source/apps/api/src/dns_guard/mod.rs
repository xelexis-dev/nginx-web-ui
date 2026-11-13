//! Filtered target DNS. The descriptor is the sole render/restart availability boundary.
use crate::{config::RuntimeConfig, control::reserved_ports::ReservedPorts};
use serde::{Deserialize, Serialize};
use std::{
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
};

pub const DEFAULT_PORT: u16 = 5354;
pub const DESCRIPTOR_PATH: &str = "/run/nginx-web-ui/target-dns.json";
pub const VALID_SECONDS: u32 = 10;
pub const TIMEOUT_SECONDS: u64 = 2;

#[derive(Debug, Clone)]
pub struct Settings {
    pub port: u16,
    pub ipv6: bool,
    pub servers_override: Option<String>,
    pub descriptor_path: PathBuf,
    /// An invalid optional DNS setting must disable dynamic DNS, not the management UI.
    pub configuration_error: Option<String>,
}
impl Settings {
    pub fn read(get: impl Fn(&str) -> Option<String>) -> Self {
        let mut settings = Self {
            port: DEFAULT_PORT,
            ipv6: true,
            servers_override: get("NWUI_TARGET_DNS_SERVERS").filter(|s| !s.trim().is_empty()),
            descriptor_path: DESCRIPTOR_PATH.into(),
            configuration_error: None,
        };
        if let Some(value) = get("NWUI_TARGET_DNS_PORT") {
            match value.parse::<u16>() {
                Ok(port) if port > 0 => settings.port = port,
                _ => {
                    settings.configuration_error =
                        Some("NWUI_TARGET_DNS_PORT 는 1–65535 포트여야 합니다.".into())
                }
            }
        }
        if let Some(value) = get("NWUI_TARGET_DNS_IPV6") {
            match value.as_str() {
                "on" => {}
                "off" => settings.ipv6 = false,
                _ => {
                    settings.configuration_error =
                        Some("NWUI_TARGET_DNS_IPV6 는 on 또는 off 여야 합니다.".into())
                }
            }
        }
        settings
    }
    pub fn endpoint(&self) -> SocketAddr {
        (Ipv4Addr::LOCALHOST, self.port).into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub schema: u8,
    pub endpoint: SocketAddr,
    pub available: bool,
    pub reason: Option<String>,
}
impl Descriptor {
    pub fn unavailable(settings: &Settings, reason: impl Into<String>) -> Self {
        Self {
            schema: 1,
            endpoint: settings.endpoint(),
            available: false,
            reason: Some(reason.into()),
        }
    }
    pub fn available(settings: &Settings) -> Self {
        Self {
            schema: 1,
            endpoint: settings.endpoint(),
            available: true,
            reason: None,
        }
    }
}

/// Missing, untrusted, unavailable or stale descriptors never enable dynamic rendering.
pub fn status(config: &RuntimeConfig) -> Descriptor {
    status_for(&config.target_dns)
}

const MAX_DESCRIPTOR_BYTES: u64 = 16_384;
fn read_open_descriptor(
    file: std::fs::File,
    expected: &std::fs::Metadata,
) -> anyhow::Result<Descriptor> {
    use std::io::Read;
    let actual = file.metadata()?;
    anyhow::ensure!(
        actual.is_file() && actual.len() <= MAX_DESCRIPTOR_BYTES,
        "invalid opened DNS descriptor"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        anyhow::ensure!(
            actual.dev() == expected.dev() && actual.ino() == expected.ino(),
            "DNS descriptor changed while opening"
        );
        anyhow::ensure!(
            actual.uid() == 0 && actual.mode() & 0o022 == 0,
            "untrusted opened DNS descriptor"
        );
    }
    let mut bytes = Vec::new();
    file.take(MAX_DESCRIPTOR_BYTES + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= MAX_DESCRIPTOR_BYTES as usize,
        "DNS descriptor grew beyond size bound"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn status_for(settings: &Settings) -> Descriptor {
    let unavailable = |reason: &str| Descriptor::unavailable(settings, reason);
    if let Some(reason) = &settings.configuration_error {
        return unavailable(reason);
    }
    let read = || -> anyhow::Result<Descriptor> {
        let parent = settings
            .descriptor_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("missing descriptor parent"))?;
        let parent = std::fs::symlink_metadata(parent)?;
        anyhow::ensure!(parent.is_dir(), "descriptor parent is not a real directory");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            anyhow::ensure!(
                parent.uid() == 0 && parent.mode() & 0o022 == 0,
                "untrusted descriptor parent"
            );
        }
        let expected = std::fs::symlink_metadata(&settings.descriptor_path)?;
        anyhow::ensure!(
            expected.is_file() && expected.len() <= MAX_DESCRIPTOR_BYTES,
            "invalid DNS descriptor file"
        );
        let descriptor =
            read_open_descriptor(std::fs::File::open(&settings.descriptor_path)?, &expected)?;
        anyhow::ensure!(
            descriptor.schema == 1 && descriptor.endpoint == settings.endpoint(),
            "stale DNS descriptor endpoint"
        );
        anyhow::ensure!(
            !descriptor.available || descriptor.reason.is_none(),
            "inconsistent DNS descriptor"
        );
        Ok(descriptor)
    };
    match read() {
        Ok(value) => value,
        Err(_) => {
            unavailable("실행 중 이름 확인 서비스의 준비 정보가 없거나 현재 설정과 다릅니다.")
        }
    }
}

/// api-boot writes this before render; root-owned /run is not a persistent operator volume.
pub async fn write_descriptor(settings: &Settings, descriptor: &Descriptor) -> anyhow::Result<()> {
    anyhow::ensure!(
        descriptor.schema == 1 && descriptor.endpoint == settings.endpoint(),
        "invalid DNS descriptor endpoint"
    );
    use tokio::{fs,io::AsyncWriteExt};
    use std::os::unix::fs::PermissionsExt;
    let path=&settings.descriptor_path;let parent=path.parent().ok_or_else(||anyhow::anyhow!("missing descriptor parent"))?;
    match fs::symlink_metadata(parent).await {
        Ok(meta)=>anyhow::ensure!(meta.is_dir()&&!meta.file_type().is_symlink(),"DNS descriptor parent is not a real directory"),
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>fs::create_dir_all(parent).await?,
        Err(error)=>return Err(error.into()),
    }
    match fs::symlink_metadata(path).await {
        Ok(meta)=>anyhow::ensure!(meta.is_file()&&!meta.file_type().is_symlink(),"DNS descriptor is not a regular file"),
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
        Err(error)=>return Err(error.into()),
    }
    let bytes=serde_json::to_vec(descriptor)?;let temporary=parent.join(format!(".nwui-dns-{}.tmp",uuid::Uuid::new_v4()));
    let mut file=fs::OpenOptions::new().write(true).create_new(true).custom_flags(libc::O_NOFOLLOW).mode(0o644).open(&temporary).await?;
    // File::set_permissions uses fchmod on this opened file, independent of umask.
    let result=async {
        file.set_permissions(std::fs::Permissions::from_mode(0o644)).await?;
        file.write_all(&bytes).await?;file.sync_all().await?;drop(file);
        fs::rename(&temporary,path).await?;fs::File::open(parent).await?.sync_all().await?;
        Ok::<(),anyhow::Error>(())
    }.await;
    // Open failure returns above: only a temporary file created by this call is removed.
    if result.is_err(){let _=fs::remove_file(&temporary).await;}
    result
}

fn startup_note(args: std::fmt::Arguments<'_>) {
    use std::io::Write;
    // A closed logging pipe is not a reason to block supervision readiness.
    let _ = std::io::stderr().lock().write_fmt(args);
}
async fn announce_unavailable<W, R>(settings: &Settings, write: W, ready: R) -> anyhow::Result<()>
where
    W: std::future::Future<Output = anyhow::Result<()>>,
    R: FnOnce() -> anyhow::Result<()>,
{
    if let Err(error) = write.await {
        startup_note(format_args!(
            "[dns-guard] descriptor downgrade failed: {error}\n"
        ));
        if let Err(error) = tokio::fs::remove_file(&settings.descriptor_path).await {
            if error.kind() != std::io::ErrorKind::NotFound {
                startup_note(format_args!(
                    "[dns-guard] descriptor invalidation failed: {error}\n"
                ));
            }
        }
    }
    // Optional DNS filesystem failures must never skip this attempt.
    ready()
}

/// CLI branch is evaluated before RuntimeConfig, so DNS needs no DB/auth secrets.
/// The unavailable state signals supervision readiness without binding a conflicting port.
pub async fn run() -> anyhow::Result<()> {
    let settings = Settings::read(|key| std::env::var(key).ok());
    let mut descriptor = status_for(&settings);
    let mut announced = false;
    if descriptor.available {
        let resolv = tokio::fs::read_to_string("/etc/resolv.conf")
            .await
            .unwrap_or_default();
        let setup = async {
            let servers = upstream::servers(&settings, &resolv)?;
            let (udp, tcp) = tokio::try_join!(
                tokio::net::UdpSocket::bind(settings.endpoint()),
                tokio::net::TcpListener::bind(settings.endpoint())
            )?;
            Ok::<_, anyhow::Error>((servers, udp, tcp))
        }
        .await;
        match setup {
            Ok((servers, udp, tcp)) => {
                // A partial drop cannot safely rewrite the root-owned descriptor: exit before readiness.
                privileges::drop_and_notify_ready()?;
                startup_note(format_args!(
                    "[dns-guard] ready TCP/UDP {} (uid=1000 gid=1000; root reacquisition denied; IPv6 targets={})\n",
                    settings.endpoint(),
                    settings.ipv6
                ));
                let guard = std::sync::Arc::new(service::Guard::new(servers, settings.ipv6));
                return service::serve_loop(udp, tcp, guard, crate::server::shutdown_signal())
                    .await;
            }
            Err(error) => {
                descriptor = Descriptor::unavailable(
                    &settings,
                    format!("대상 DNS 수신 서비스를 사용할 수 없습니다: {error}"),
                );
                let result = announce_unavailable(
                    &settings,
                    write_descriptor(&settings, &descriptor),
                    crate::runtime_boot::notify_ready,
                )
                .await;
                announced = true;
                // Only a failure of the readiness channel itself remains fatal,
                // after readiness was attempted; descriptor I/O never propagates.
                if let Err(error) = result {
                    return Err(error);
                }
            }
        }
    }
    startup_note(format_args!(
        "[dns-guard] unavailable: {}\n",
        descriptor.reason.as_deref().unwrap_or("unknown")
    ));
    if !announced {
        crate::runtime_boot::notify_ready()?;
    }
    crate::server::shutdown_signal().await;
    Ok(())
}
pub fn require_runtime_dns(config: &RuntimeConfig) -> Result<SocketAddr, String> {
    let value = status(config);
    if value.available {
        Ok(value.endpoint)
    } else {
        Err(value
            .reason
            .unwrap_or_else(|| "실행 중 이름 확인 서비스를 사용할 수 없습니다.".into()))
    }
}
/// Save/apply must use the descriptor, never reserve a port that an existing rule owns.
pub fn reserve_port(reserved: &mut ReservedPorts, descriptor: &Descriptor) {
    if descriptor.available {
        let port = descriptor.endpoint.port();
        if !reserved.process.contains(&port) {
            reserved.process.push(port);
        }
        if !reserved.process_udp.contains(&port) {
            reserved.process_udp.push(port);
        }
        reserved
            .owners
            .insert(port, "실행 중 이름 확인 (DNS guard, TCP/UDP)".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::reserved_ports::{ListenContext, find_reserved_port_conflict};

    fn config() -> RuntimeConfig {
        RuntimeConfig::read(|key| match key {
            "DATABASE_URL" => Some("postgres://test:test@127.0.0.1/test".into()),
            "AUTH_SECRET" => Some("test-only".into()),
            _ => None,
        })
        .unwrap()
    }
    #[tokio::test]
    async fn descriptor_umask_child() {
        use std::os::unix::fs::{MetadataExt,PermissionsExt};
        let Some(root)=std::env::var_os("NWUI_DESCRIPTOR_UMASK_DIR")else{return;};let root=std::path::PathBuf::from(root);
        let mut c=config();c.target_dns.descriptor_path=root.join("target-dns.json");
        unsafe{libc::umask(0o077);}
        write_descriptor(&c.target_dns,&Descriptor::available(&c.target_dns)).await.unwrap();
        let file=std::fs::symlink_metadata(&c.target_dns.descriptor_path).unwrap();assert_eq!(file.permissions().mode()&0o7777,0o644);assert_eq!((file.uid(),file.gid()),(0,0));
        let parent=std::fs::symlink_metadata(&root).unwrap();assert_eq!((parent.uid(),parent.gid(),parent.permissions().mode()&0o7777),(0,0,0o755));
        assert!(status(&c).available);assert_eq!(std::fs::read_dir(root).unwrap().count(),1,"no temporary file remains after publication");
    }
    #[test]
    fn descriptor_written_under_umask_077_has_explicit_public_read_mode() {
        use std::os::unix::fs::PermissionsExt;
        let root=tempfile::tempdir().unwrap();std::fs::set_permissions(root.path(),std::fs::Permissions::from_mode(0o755)).unwrap();
        let child=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","dns_guard::tests::descriptor_umask_child","--nocapture"])
            .env_clear().env("NWUI_DESCRIPTOR_UMASK_DIR",root.path()).output().unwrap();
        assert!(child.status.success(),"{} {}",String::from_utf8_lossy(&child.stdout),String::from_utf8_lossy(&child.stderr));
    }

    #[test]
    fn trusted_descriptor_enables_only_its_configured_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config();
        config.target_dns.descriptor_path = dir.path().join("dns.json");
        let descriptor = Descriptor::available(&config.target_dns);
        std::fs::write(
            &config.target_dns.descriptor_path,
            serde_json::to_vec(&descriptor).unwrap(),
        )
        .unwrap();
        assert!(status(&config).available);
        assert_eq!(
            require_runtime_dns(&config).unwrap(),
            config.target_dns.endpoint()
        );
        config.target_dns.port += 1;
        assert!(
            !status(&config).available,
            "stale endpoint must never be queried"
        );
    }
    #[tokio::test]
    async fn malformed_writable_symlink_and_old_descriptors_never_enable_dns() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let mut c = config();
        c.target_dns.descriptor_path = root.path().join("descriptor.json");
        assert!(!status(&c).available);
        for raw in [
            b"{".as_slice(),
            b"{}",
            br#"{"schema":0,"endpoint":"127.0.0.1:5354","available":true,"reason":null}"#,
        ] {
            std::fs::write(&c.target_dns.descriptor_path, raw).unwrap();
            assert!(!status(&c).available);
        }
        let descriptor = Descriptor::available(&c.target_dns);
        write_descriptor(&c.target_dns, &descriptor).await.unwrap();
        assert!(status(&c).available);
        std::fs::set_permissions(
            &c.target_dns.descriptor_path,
            std::fs::Permissions::from_mode(0o666),
        )
        .unwrap();
        assert!(!status(&c).available);
        std::fs::remove_file(&c.target_dns.descriptor_path).unwrap();
        let target = root.path().join("elsewhere.json");
        std::fs::write(&target, serde_json::to_vec(&descriptor).unwrap()).unwrap();
        symlink(target, &c.target_dns.descriptor_path).unwrap();
        assert!(!status(&c).available);
    }
    #[test]
    fn descriptor_parent_must_be_root_owned_and_not_writable_by_group_or_others() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let mut c = config();
        c.target_dns.descriptor_path = root.path().join("dns.json");
        std::fs::write(
            &c.target_dns.descriptor_path,
            serde_json::to_vec(&Descriptor::available(&c.target_dns)).unwrap(),
        )
        .unwrap();
        assert!(status(&c).available);
        for mode in [0o770, 0o707, 0o777] {
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(mode)).unwrap();
            assert!(!status(&c).available, "untrusted parent mode {mode:o}");
        }
    }
    #[tokio::test]
    async fn startup_write_failure_invalidates_stale_availability_before_readiness() {
        let root = tempfile::tempdir().unwrap();
        let mut c = config();
        c.target_dns.descriptor_path = root.path().join("dns.json");
        std::fs::write(
            &c.target_dns.descriptor_path,
            serde_json::to_vec(&Descriptor::available(&c.target_dns)).unwrap(),
        )
        .unwrap();
        let ready = root.path().join("ready");
        announce_unavailable(
            &c.target_dns,
            async { Err(anyhow::anyhow!("injected atomic write failure")) },
            || {
                std::fs::write(&ready, b"\n")?;
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(ready).unwrap(), b"\n");
        assert!(!status(&c).available);
    }
    #[tokio::test]
    async fn startup_read_only_directory_still_announces_readiness() {
        let root = tempfile::tempdir().unwrap();
        let mut settings = Settings::read(|_| None);
        settings.descriptor_path = "/sys/nwui-dns-guard-read-only-test.json".into();
        let descriptor = Descriptor::unavailable(&settings, "test unavailable");
        let ready = root.path().join("ready");
        announce_unavailable(&settings, write_descriptor(&settings, &descriptor), || {
            std::fs::write(&ready, b"\n")?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(ready).unwrap(), b"\n");
        assert!(!status_for(&settings).available);
    }
    #[tokio::test]
    async fn startup_failed_removal_does_not_skip_readiness() {
        let root = tempfile::tempdir().unwrap();
        let mut settings = Settings::read(|_| None);
        settings.descriptor_path = root.path().to_owned();
        let ready = root.path().join("ready");
        announce_unavailable(
            &settings,
            async { Err(anyhow::anyhow!("injected write failure")) },
            || {
                std::fs::write(&ready, b"\n")?;
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(ready).unwrap(), b"\n");
        assert!(!status_for(&settings).available);
    }
    #[test]
    fn opened_descriptor_must_match_the_lstat_identity() {
        let root = tempfile::tempdir().unwrap();
        let settings = Settings::read(|_| None);
        let first = root.path().join("first");
        let second = root.path().join("second");
        let bytes = serde_json::to_vec(&Descriptor::available(&settings)).unwrap();
        std::fs::write(&first, &bytes).unwrap();
        std::fs::write(&second, &bytes).unwrap();
        let expected = std::fs::symlink_metadata(&first).unwrap();
        assert!(read_open_descriptor(std::fs::File::open(&second).unwrap(), &expected).is_err());
        assert!(read_open_descriptor(std::fs::File::open(&first).unwrap(), &expected).is_ok());
    }
    #[test]
    fn descriptor_size_boundary_is_enforced_on_the_opened_file() {
        let root = tempfile::tempdir().unwrap();
        let settings = Settings::read(|_| None);
        let path = root.path().join("dns");
        let mut bytes = serde_json::to_vec(&Descriptor::available(&settings)).unwrap();
        bytes.resize(MAX_DESCRIPTOR_BYTES as usize, b' ');
        std::fs::write(&path, &bytes).unwrap();
        let expected = std::fs::symlink_metadata(&path).unwrap();
        assert!(read_open_descriptor(std::fs::File::open(&path).unwrap(), &expected).is_ok());
        bytes.push(b' ');
        std::fs::write(&path, bytes).unwrap();
        assert!(read_open_descriptor(std::fs::File::open(&path).unwrap(), &expected).is_err());
    }
    #[test]
    fn operator_dns_settings_are_read_without_auth_or_database() {
        let s = Settings::read(|k| match k {
            "NWUI_TARGET_DNS_PORT" => Some("5454".into()),
            "NWUI_TARGET_DNS_IPV6" => Some("off".into()),
            "NWUI_TARGET_DNS_SERVERS" => Some("127.0.0.11".into()),
            _ => None,
        });
        assert_eq!(s.endpoint().to_string(), "127.0.0.1:5454");
        assert!(!s.ipv6);
        assert_eq!(s.servers_override.as_deref(), Some("127.0.0.11"));
    }
    #[test]
    fn bad_optional_dns_settings_disable_only_dns() {
        for (key, value) in [
            ("NWUI_TARGET_DNS_PORT", "0"),
            ("NWUI_TARGET_DNS_PORT", "bad"),
            ("NWUI_TARGET_DNS_IPV6", "maybe"),
        ] {
            let s = Settings::read(|k| (k == key).then(|| value.into()));
            assert!(s.configuration_error.is_some(), "{key} must fail closed");
        }
    }
    #[test]
    fn guard_reserves_both_transports_only_when_available() {
        let settings = Settings::read(|_| None);
        let mut ports = ReservedPorts::default();
        reserve_port(
            &mut ports,
            &Descriptor::unavailable(&settings, "saved UDP listener"),
        );
        assert!(find_reserved_port_conflict(5354, ListenContext::StreamUdp, &ports).is_none());
        reserve_port(&mut ports, &Descriptor::available(&settings));
        assert!(find_reserved_port_conflict(5354, ListenContext::StreamUdp, &ports).is_some());
        assert!(find_reserved_port_conflict(5354, ListenContext::StreamTcp, &ports).is_some());
        assert!(find_reserved_port_conflict(5354, ListenContext::Http, &ports).is_some());
    }
}

#[cfg(test)]
mod bindings;
pub(crate) mod policy;
mod privileges;
pub mod preflight;
mod service;
mod upstream;
