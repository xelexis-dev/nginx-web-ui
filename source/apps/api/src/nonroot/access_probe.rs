//! Execute the boundary checks with the API's real kernel credentials.
use super::{handles, privilege::{Identity, API_GID, API_UID}};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::{self, Read, Write}, os::{fd::AsRawFd, unix::process::CommandExt}, path::PathBuf, process::Stdio};

#[derive(Serialize, Deserialize)]
pub(super) struct Plan {
    pub key: PathBuf,
    pub writable: PathBuf,
    pub protected: Vec<PathBuf>,
    pub log_entries: Vec<PathBuf>,
    pub conf_probe: PathBuf,
    pub root_pid: i32,
    pub publisher_socket: PathBuf,
    pub node_probe: bool,
    #[serde(default)]
    pub log_probe: bool,
}

pub async fn run() -> Result<(), crate::upgrade::StepError> {
    let plan = (|| {
        if ![API_UID, 1000].contains(&unsafe { libc::geteuid() }) || unsafe { libc::getuid() } != unsafe { libc::geteuid() } {
            return Err(handles::invalid("access probe requires dropped API credentials"));
        }
        let mut bytes = Vec::new();
        io::stdin().take(16385).read_to_end(&mut bytes)?;
        if bytes.len() > 16384 { return Err(handles::invalid("oversized access probe")); }
        serde_json::from_slice::<Plan>(&bytes).map_err(|_| handles::invalid("invalid access probe"))
    })().map_err(|_|super::error("isolation_unavailable","API 저장소 접근 권한을 안전하게 제한하지 못했습니다."))?;
    check_step(&plan)
}
fn check_step(plan:&Plan)->Result<(),crate::upgrade::StepError> {
    check(plan).map_err(|_|if plan.log_probe && !plan.node_probe {
        super::error("log_dir_unsafe","호스트 로그 폴더의 쓰기 권한을 root로 제한하세요.")
    }else{super::error("isolation_unavailable","API 저장소 접근 권한을 안전하게 제한하지 못했습니다.")})
}

fn denied(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(error) if error.raw_os_error() == Some(libc::EACCES) => Ok(()),
        _ => Err(handles::invalid("protected operation was not denied with EACCES")),
    }
}
pub(super) fn log_destination(entry:&std::path::Path)->io::Result<PathBuf> {
    let mut name=entry.file_name().ok_or_else(||handles::invalid("missing log probe name"))?.to_os_string();
    name.push("-renamed");Ok(entry.with_file_name(name))
}
fn check_logs(plan:&Plan)->io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    for entry in &plan.log_entries {
        denied(std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(entry).map(|_|()))?;
        let destination=log_destination(entry)?;
        let create=std::fs::OpenOptions::new().write(true).create_new(true).open(&destination);
        let create=match create {Ok(file)=>{drop(file);std::fs::remove_file(&destination)?;Ok(())},Err(error)=>Err(error)};
        denied(create)?;
        let rename=std::fs::rename(entry,&destination);
        if rename.is_ok(){std::fs::rename(&destination,entry)?;}
        denied(rename)?;
        denied(std::fs::remove_file(entry))?;
    }
    Ok(())
}
fn check(plan: &Plan) -> io::Result<()> {
    if plan.node_probe {
        if unsafe { libc::geteuid() } != 1000 { return Err(handles::invalid("incorrect node probe identity")); }
        denied(File::open(&plan.key).map(|_| ()))?;
        denied(std::os::unix::net::UnixStream::connect(&plan.publisher_socket).map(|_| ()))?;
        return Ok(());
    }
    if unsafe { libc::geteuid() } != API_UID { return Err(handles::invalid("incorrect API probe identity")); }
    if plan.log_probe {return check_logs(plan);}
    crate::crypto::SecretBox::load_existing(&plan.key).map_err(|_| handles::invalid("runtime key is unreadable or invalid"))?;
    File::open(plan.publisher_socket.parent().ok_or_else(||handles::invalid("missing publisher parent"))?)?;
    let nonce = format!(".nwui-access-probe-{}", uuid::Uuid::new_v4());
    let own = plan.writable.join(&nonce);
    let renamed = plan.writable.join(format!("{nonce}-renamed"));
    let mut writable = std::fs::OpenOptions::new().write(true).create_new(true).open(&own)?;
    writable.write_all(b"probe")?;
    std::fs::rename(&own, &renamed)?;
    std::fs::remove_file(&renamed)?;
    // Probe siblings, never destructive operations on the real uncertainty marker.
    std::fs::symlink_metadata(&plan.conf_probe)?;
    let conf_name = plan.conf_probe.file_name().ok_or_else(|| handles::invalid("missing conf probe name"))?.to_string_lossy();
    let no_write = |path: &std::path::Path| {
        use std::os::unix::fs::OpenOptionsExt;
        denied(std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).map(|_| ()))
    };
    no_write(&plan.conf_probe)?;
    let marker = plan.conf_probe.with_file_name(".nwui-apply-uncertain");
    match std::fs::symlink_metadata(&marker) {
        Ok(_) => no_write(&marker)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {},
        Err(error) => return Err(error),
    }
    let destination = plan.conf_probe.with_file_name(format!("{conf_name}-renamed"));
    let rename = std::fs::rename(&plan.conf_probe, &destination);
    if rename.is_ok() { std::fs::rename(&destination, &plan.conf_probe)?; }
    denied(rename)?;
    let replacement = plan.conf_probe.with_file_name(format!("{conf_name}-replacement"));
    denied(std::fs::rename(&replacement, &plan.conf_probe))?;
    denied(std::fs::remove_file(&plan.conf_probe))?;
    let create = std::fs::OpenOptions::new().write(true).create_new(true).open(&destination);
    let create = match create {
        Ok(file) => { drop(file); std::fs::remove_file(&destination)?; Ok(()) },
        Err(error) => Err(error),
    };
    denied(create)?;
    for dir in &plan.protected {
        let mut path = dir.join(&nonce);
        loop {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Err(error) if error.raw_os_error() == Some(libc::EACCES) => break,
                // For a missing directory, test creating its name in the nearest existing parent.
                Err(error) if error.raw_os_error() == Some(libc::ENOENT) => {
                    path = path.parent().ok_or_else(|| handles::invalid("missing protected ancestor"))?.to_owned();
                },
                Ok(file) => { drop(file); let _ = std::fs::remove_file(&path); return Err(handles::invalid("API can create persistent files")); },
                Err(_) => return Err(handles::invalid("persistent denial was not observed")),
            }
        }
    }
    let mut read_buffer = [0u8];
    if unsafe { libc::read(4, read_buffer.as_mut_ptr().cast(), 1) } != -1 || io::Error::last_os_error().raw_os_error() != Some(libc::EBADF) {
        return Err(handles::invalid("auth-log descriptor is readable"));
    }
    let line = b"nginx-web-ui access-probe\n";
    if unsafe { libc::write(4, line.as_ptr().cast(), line.len()) } != line.len() as isize {
        return Err(handles::invalid("auth-log descriptor is not writable"));
    }
    if unsafe { libc::kill(plan.root_pid, 0) } != -1 || io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
        return Err(handles::invalid("API can signal the root controller"));
    }
    Ok(())
}

/// A rejected check is false; launch, I/O and timeout failures remain errors.
pub(super) async fn execute(plan: &Plan, auth_log: &File, identity: Identity) -> io::Result<bool> {
    let auth_fd = auth_log.as_raw_fd();
    let node_probe = plan.node_probe;
    let mut identity = identity;
    if node_probe { identity.groups.clear(); }
    #[cfg(not(test))]
    let mut command = tokio::process::Command::new(std::env::current_exe()?);
    #[cfg(test)]
    let mut command = tokio::process::Command::new(match std::env::var_os("NWUI_TEST_API_BIN") {
        Some(path) => PathBuf::from(path),
        None => std::env::current_exe()?,
    });
    // The UID 1000 probe must not expose root's credentials through /proc/environ.
    command.env_clear().arg("access-probe").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
    unsafe { command.as_std_mut().pre_exec(move || {
        if node_probe { libc::close(4); }
        else if libc::dup2(auth_fd, 4) == -1 || libc::fcntl(4, libc::F_SETFD, 0) == -1 { return Err(io::Error::last_os_error()); }
        libc::close(3); // Only serve inherits readiness FD 3; the probe must not inherit runner locks.
        if libc::syscall(libc::SYS_close_range, 5u32, u32::MAX, libc::CLOSE_RANGE_CLOEXEC) == -1 { return Err(io::Error::last_os_error()); }
        identity.drop(if node_probe { 1000 } else { API_UID }, if node_probe { 1000 } else { API_GID })
    }); }
    let mut child = command.spawn()?;
    let bytes = serde_json::to_vec(plan).map_err(|_| handles::invalid("invalid access probe"))?;
    wait_probe(&mut child, &bytes, std::time::Duration::from_secs(30)).await
}

async fn wait_probe(child: &mut tokio::process::Child, bytes: &[u8], limit: std::time::Duration) -> io::Result<bool> {
    use tokio::io::AsyncWriteExt;
    let result = tokio::time::timeout(limit, async {
        let mut input = child.stdin.take().ok_or_else(|| handles::invalid("missing probe input"))?;
        input.write_all(bytes).await?;
        drop(input);
        Ok(child.wait().await?.success())
    }).await;
    match result {
        Ok(Ok(passed)) => Ok(passed),
        other => {
            // Reap on timeout or a broken pipe, rather than leaving a credentialed child behind.
            if child.try_wait()?.is_none() {
                tokio::time::timeout(std::time::Duration::from_secs(5), child.kill()).await
                    .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
            }
            match other { Ok(result) => result, Err(_) => Err(io::Error::from(io::ErrorKind::TimedOut)) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::{PermissionsExt, MetadataExt}, process::Command};

    #[tokio::test]
    async fn probe_deadline_kills_and_reaps_the_child() {
        let mut child = tokio::process::Command::new("/bin/sleep").arg("60").stdin(Stdio::piped()).kill_on_drop(true).spawn().unwrap();
        let error = wait_probe(&mut child, b"", std::time::Duration::from_millis(50)).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(child.try_wait().unwrap().is_some());
    }

    #[tokio::test]
    async fn completed_probe_rejection_is_distinct_from_transport_failure() {
        for (status,passed) in [(0,true),(1,false)] {
            let mut child=tokio::process::Command::new("/bin/sh").args(["-c",&format!("exit {status}")]).stdin(Stdio::piped()).kill_on_drop(true).spawn().unwrap();
            assert_eq!(wait_probe(&mut child,b"",std::time::Duration::from_secs(1)).await.unwrap(),passed);
        }
    }
    #[test]
    #[ignore = "child process fixture; the parent runs it after the real UID transition"]
    fn dropped_probe_child() {
        let Ok(value) = std::env::var("NWUI_N2_PROBE") else { return; };
        for secret in ["AUTH_SECRET", "DATABASE_URL", "ADMIN_PASSWORD"] { assert!(std::env::var_os(secret).is_none()); }
        let plan: Plan = serde_json::from_str(&value).unwrap();
        check_step(&plan).unwrap();
    }

    #[test]
    fn actual_uid_can_use_only_runtime_key_directory_and_write_only_log() {
        let temp = tempfile::tempdir_in("/run").unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let key_dir = temp.path().join("private");
        handles::ensure_directory(&key_dir, 0, API_GID, 0o750).unwrap();
        let key = key_dir.join("data_key");
        handles::atomic_write(&key, "a".repeat(64).as_bytes(), API_GID, 0o440).unwrap();
        let writable = temp.path().join("api");
        handles::ensure_directory(&writable, API_UID, API_GID, 0o700).unwrap();
        let protected = temp.path().join("storage");
        handles::ensure_directory(&protected, 0, 0, 0o700).unwrap();
        let logs = temp.path().join("logs");
        handles::ensure_directory(&logs, 0, 0, 0o755).unwrap();
        let entry = logs.join("probe-original");
        fs::write(&entry, b"unchanged").unwrap();
        let log = handles::open_auth_log(&logs.join("auth.log"), &[4,101]).unwrap();
        let conf = temp.path().join("conf");
        handles::ensure_directory(&conf, 0, 0, 0o755).unwrap();
        let conf_probe = conf.join(format!(".nwui-access-probe-{}", uuid::Uuid::new_v4()));
        handles::create_log_probe(&conf_probe, &[]).unwrap();
        let probe_meta = fs::symlink_metadata(&conf_probe).unwrap();
        assert_eq!((probe_meta.uid(), probe_meta.mode() & 0o777), (0, 0o600));
        let replacement = conf_probe.with_file_name(format!("{}-replacement", conf_probe.file_name().unwrap().to_string_lossy()));
        handles::create_log_probe(&replacement, &[]).unwrap();
        let marker = conf.join(".nwui-apply-uncertain");
        fs::write(&marker, b"real uncertainty must survive").unwrap();
        fs::set_permissions(&marker, fs::Permissions::from_mode(0o600)).unwrap();
        let marker_inode = fs::metadata(&marker).unwrap().ino();
        let mut plan = Plan { key, writable, conf_probe: conf_probe.clone(), protected: vec![protected, logs.clone(), temp.path().join("absent/child")], log_entries: vec![entry.clone()],
            root_pid: std::process::id() as i32, publisher_socket: key_dir.join("control.sock"), node_probe: false, log_probe:false };
        for case in ["safe", "node", "writable-leaf", "unsearchable-parent", "writable-parent", "writable-marker", "log-only-safe", "writable-log-parent", "absent-marker"] {
            let node = case == "node";
            if case == "absent-marker" { fs::remove_file(&marker).unwrap(); }
            plan.node_probe = node;
            plan.log_probe=matches!(case,"log-only-safe"|"writable-log-parent");
            fs::set_permissions(&logs,fs::Permissions::from_mode(if case=="writable-log-parent"{0o777}else{0o755})).unwrap();
            fs::set_permissions(&conf, fs::Permissions::from_mode(if case == "unsearchable-parent" {0o700} else if case == "writable-parent" {0o777} else {0o755})).unwrap();
            fs::set_permissions(&conf_probe, fs::Permissions::from_mode(if case == "writable-leaf" {0o666} else {0o600})).unwrap();
            if case != "absent-marker" { fs::set_permissions(&marker, fs::Permissions::from_mode(if case == "writable-marker" {0o666} else {0o600})).unwrap(); }
            let fd = log.as_raw_fd();
            let mut identity = Identity::for_test_groups(if node { vec![] } else { vec![4,101] });
            // Key access is through the primary API GID, never a supplementary root group.
            identity.groups.retain(|gid| *gid != 0);
            let mut command = Command::new(std::env::current_exe().unwrap());
            command.env_clear().args(["--ignored", "--exact", "nonroot::access_probe::tests::dropped_probe_child", "--test-threads=1"])
                .env("NWUI_N2_PROBE", serde_json::to_string(&plan).unwrap());
            unsafe { command.pre_exec(move || {
                if libc::dup2(fd,4) == -1 || libc::fcntl(4,libc::F_SETFD,0) == -1 { return Err(io::Error::last_os_error()); }
                identity.drop(if node {1000} else {API_UID}, if node {1000} else {API_GID})
            }); }
            let result = command.output().unwrap();
            assert_eq!(result.status.success(), matches!(case, "safe" | "node" | "absent-marker" | "log-only-safe"), "case={case}: {} {}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
            let diagnostics=format!("{}{}",String::from_utf8_lossy(&result.stdout),String::from_utf8_lossy(&result.stderr));
            if result.status.success() {assert!(diagnostics.contains("test result: ok. 1 passed; 0 failed;"),"case={case}: {diagnostics}");}
            else {let code=if case=="writable-log-parent"{"log_dir_unsafe"}else{"isolation_unavailable"};assert!(diagnostics.contains(code),"case={case}: {diagnostics}");}
            if case=="absent-marker" { assert!(!marker.exists()); }
            else {
                assert_eq!(fs::read(&marker).unwrap(), b"real uncertainty must survive");
                assert_eq!(fs::metadata(&marker).unwrap().ino(), marker_inode);
            }
        }
        assert_eq!(fs::read(entry).unwrap(), b"unchanged");
    }
}
