//! A.20 custody transport through the pinned s6 tools; runner policy stays outside.
use super::StepError;
use std::{collections::BTreeSet, ffi::OsString, fs::File, io::{self, Read}, os::{fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd}, unix::{net::UnixStream, process::CommandExt}}, path::PathBuf, process::{Child, Command, ExitCode, ExitStatus, Stdio}, time::{Duration, Instant}};

pub(crate) const SOCKET: &str = "/run/nwui-locks/fdholder.sock";
pub(crate) const SERVICE_DIR: &str = "/run/service/storage-locks";
const IMAGE_PATH: &str = "/command:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
const API: &str = "/usr/local/bin/nwui-api";
const TOOL_WALL: Duration = Duration::from_secs(5);
const READY_MARGIN: Duration = Duration::from_millis(250);
const OUTPUT_LIMIT: usize = 4096;
// Distinct from pinned s6 tool exits: the exec'ed helper's failures are protocol violations.
const PASSBACK_PROTOCOL_EXIT: u8 = 112;
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum CustodyId { A, D }
impl CustodyId { fn name(self) -> &'static str { match self { Self::A => "lock:A", Self::D => "lock:D" } } }
fn failure(code: &'static str, reason: &'static str) -> StepError {
    StepError { code, message: "잠금 보관 작업을 완료하지 못했습니다.", detail: Some(reason.into()) }
}
fn tool_path() -> OsString {
    #[cfg(test)]
    if let Some(value) = TEST_SETTINGS.with(|s| s.borrow().as_ref().map(|s| s.path.clone())) { return value; }
    IMAGE_PATH.into()
}
fn passback_program() -> PathBuf {
    #[cfg(test)]
    if let Some(value) = TEST_SETTINGS.with(|s| s.borrow().as_ref().map(|s| s.passback.clone())) { return value; }
    API.into()
}
fn command(tool: &str) -> Command {
    let mut command = Command::new(tool);
    command.env_clear().env("PATH", tool_path()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
    command
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(Some(_))) { return; }
        unsafe { libc::kill(-(self.0.id() as i32), libc::SIGKILL); }
        let _ = self.0.kill(); let _ = self.0.wait();
    }
}
struct Output { status: ExitStatus, stdout: Vec<u8> }
fn nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 { return Err(io::Error::last_os_error()); }
    Ok(())
}
fn drain(pipe: &mut impl Read, captured: &mut Vec<u8>, overflow: &mut bool) -> io::Result<()> {
    let mut bytes = [0u8; 1024];
    // A noisy stderr must not keep this loop from checking the child's wall deadline.
    for _ in 0..16 {
        match pipe.read(&mut bytes) {
            Ok(0) => break,
            Ok(n) => { let keep = n.min(OUTPUT_LIMIT - captured.len()); captured.extend_from_slice(&bytes[..keep]); *overflow |= keep < n; }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
fn execute(mut command: Command, wall: Duration, strict_stdout: bool, timeout_code: &'static str) -> Result<Output, StepError> {
    let start = Instant::now();
    let mut child = Running(command.spawn().map_err(|_| failure("io_error", "s6 tool spawn failed"))?);
    nonblocking(child.0.stdout.as_ref().unwrap().as_raw_fd()).and_then(|_| nonblocking(child.0.stderr.as_ref().unwrap().as_raw_fd()))
        .map_err(|_| failure("io_error", "s6 tool pipe setup failed"))?;
    let (mut out, mut err, mut out_over, mut err_over) = (Vec::new(), Vec::new(), false, false);
    loop {
        if start.elapsed() >= wall { return Err(failure(timeout_code, "s6 tool wall deadline exceeded")); }
        drain(child.0.stdout.as_mut().unwrap(), &mut out, &mut out_over).map_err(|_| failure("io_error", "s6 tool output read failed"))?;
        let _ = drain(child.0.stderr.as_mut().unwrap(), &mut err, &mut err_over);
        if strict_stdout && out_over { return Err(failure("custody_protocol", "list output limit exceeded")); }
        if let Some(status) = child.0.try_wait().map_err(|_| failure("io_error", "s6 tool wait failed"))? {
            drain(child.0.stdout.as_mut().unwrap(), &mut out, &mut out_over).map_err(|_| failure("io_error", "s6 tool output read failed"))?;
            let _ = drain(child.0.stderr.as_mut().unwrap(), &mut err, &mut err_over);
            if strict_stdout && out_over { return Err(failure("custody_protocol", "list output limit exceeded")); }
            return Ok(Output { status, stdout: out });
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn check_status(status: ExitStatus, readiness: bool) -> Result<(), StepError> {
    match status.code() {
        Some(0) => Ok(()),
        Some(111) => Err(failure("custody_unavailable", "storage-locks unavailable")),
        Some(99) if readiness => Err(failure("custody_unavailable", "storage-locks readiness timed out")),
        Some(1) if !readiness => Err(failure("custody_denied", "storage-locks access denied")),
        _ => Err(failure("io_error", "unexpected s6 tool exit")),
    }
}
pub(crate) fn list() -> Result<BTreeSet<CustodyId>, StepError> {
    let mut cmd = command("s6-fdholder-list"); cmd.args(["-t", "2000", SOCKET]);
    let output = execute(cmd, TOOL_WALL, true, "io_error")?; check_status(output.status, false)?;
    let mut ids = BTreeSet::new();
    let mut lines = output.stdout.split(|b| *b == b'\n').peekable();
    while let Some(line) = lines.next() {
        if line.is_empty() && lines.peek().is_none() { break; }
        let id = match line { b"lock:A" => CustodyId::A, b"lock:D" => CustodyId::D, _ => return Err(failure("custody_protocol", "invalid custody list")) };
        if !ids.insert(id) { return Err(failure("custody_protocol", "duplicate custody id")); }
    }
    Ok(ids)
}
pub(crate) fn store(id: CustodyId, fd: BorrowedFd<'_>) -> Result<(), StepError> {
    if list()?.contains(&id) { return Err(failure("custody_present", id.name())); }
    let mut cmd = command("s6-fdholder-store"); cmd.args(["-t", "2000", SOCKET, id.name()]);
    cmd.stdin(Stdio::from(File::from(fd.try_clone_to_owned().map_err(|_| failure("io_error", "custody fd duplication failed"))?)));
    let result = execute(cmd, TOOL_WALL, false, "io_error")?;
    if result.status.code() == Some(1) {
        return Err(failure(if list()?.contains(&id) { "custody_present" } else { "custody_denied" }, id.name()));
    }
    check_status(result.status, false)?;
    let after=list()?;
    if !after.contains(&id) { return Err(failure(if after.is_empty(){"custody_unavailable"}else{"custody_protocol"}, "store contradicted by list")); }
    Ok(())
}
pub(crate) fn delete(id: CustodyId) -> Result<(), StepError> {
    if !list()?.contains(&id) { return Ok(()); }
    let mut cmd = command("s6-fdholder-delete"); cmd.args(["-t", "2000", SOCKET, id.name()]);
    let result = execute(cmd, TOOL_WALL, false, "io_error")?;
    if result.status.code() == Some(1) {
        return if !list()?.contains(&id) { Ok(()) } else { Err(failure("custody_denied", id.name())) };
    }
    check_status(result.status, false)?;
    if list()?.contains(&id) { return Err(failure("custody_protocol", "delete contradicted by list")); }
    Ok(())
}
fn receive(socket: &UnixStream) -> Result<OwnedFd, StepError> {
    let mut bytes = [0u8; 2]; let mut control: [libc::cmsghdr; 8] = unsafe { std::mem::zeroed() };
    let mut iov = libc::iovec { iov_base: bytes.as_mut_ptr().cast(), iov_len: bytes.len() };
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut iov; message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast(); message.msg_controllen = std::mem::size_of_val(&control);
    let n = unsafe { libc::recvmsg(socket.as_raw_fd(), &mut message, libc::MSG_CMSG_CLOEXEC | libc::MSG_DONTWAIT) };
    if n < 0 { return Err(failure("custody_protocol", "missing passback frame")); }
    let mut fds: Vec<OwnedFd> = Vec::new(); let mut invalid = false;
    let mut cmsg = unsafe { libc::CMSG_FIRSTHDR(&message) };
    while !cmsg.is_null() {
        let header = unsafe { &*cmsg }; let base = unsafe { libc::CMSG_LEN(0) as usize };
        let offset = cmsg as usize - message.msg_control as usize;
        if header.cmsg_len < base || offset > message.msg_controllen || header.cmsg_len > message.msg_controllen - offset { invalid = true; break; }
        if header.cmsg_level == libc::SOL_SOCKET && header.cmsg_type == libc::SCM_RIGHTS {
            let length = header.cmsg_len - base;
            invalid |= length % std::mem::size_of::<RawFd>() != 0;
            for index in 0..length / std::mem::size_of::<RawFd>() {
                let fd = unsafe { libc::CMSG_DATA(cmsg).cast::<RawFd>().add(index).read_unaligned() };
                if fd < 0 || fds.iter().any(|v| v.as_raw_fd() == fd) { invalid = true; }
                else { fds.push(unsafe { OwnedFd::from_raw_fd(fd) }); }
            }
        } else { invalid = true; }
        cmsg = unsafe { libc::CMSG_NXTHDR(&message, cmsg) };
    }
    // Adopt every received fd before rejecting truncation, so malformed passbacks cannot leak them.
    if invalid || message.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) != 0 || n != 1 || bytes[0] != b'F' || fds.len() != 1 {
        return Err(failure("custody_protocol", "invalid passback frame"));
    }
    let fd = fds.pop().unwrap(); let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd.as_raw_fd(), metadata.as_mut_ptr()) } != 0
        || unsafe { metadata.assume_init() }.st_mode & libc::S_IFMT != libc::S_IFDIR { return Err(failure("custody_protocol", "passback fd is not a directory")); }
    if fd.as_raw_fd() == 3 { return fd.try_clone().map_err(|_| failure("io_error", "custody fd duplication failed")); }
    Ok(fd)
}
pub(crate) fn retrieve(id: CustodyId) -> Result<OwnedFd, StepError> {
    if !list()?.contains(&id) { return Err(failure("custody_missing", id.name())); }
    let (parent, child) = UnixStream::pair().map_err(|_| failure("io_error", "passback socketpair failed"))?;
    let child: OwnedFd = if child.as_raw_fd() <= 3 {
        let moved = unsafe { libc::fcntl(child.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 4) };
        if moved < 0 { return Err(failure("io_error", "passback fd relocation failed")); }
        drop(child);
        unsafe { OwnedFd::from_raw_fd(moved) }
    } else { child.into() };
    let source = child.as_raw_fd();
    let mut cmd = command("s6-fdholder-retrieve");
    cmd.args(["-t", "2000", SOCKET, id.name()]).arg(passback_program()).arg("fdholder-passback").arg(source.to_string());
    // Inherit this already-owned socket in place; do not overwrite std's exec-error pipe or fd 3.
    unsafe { cmd.pre_exec(move || {
        let flags = libc::fcntl(source, libc::F_GETFD);
        if flags < 0 || libc::fcntl(source, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 { return Err(io::Error::last_os_error()); }
        Ok(())
    }); }
    let result = execute(cmd, TOOL_WALL, false, "io_error")?; drop(child);
    if result.status.code() == Some(i32::from(PASSBACK_PROTOCOL_EXIT)) {
        return Err(failure("custody_protocol", "passback helper failed"));
    }
    if result.status.code() == Some(1) {
        return Err(failure(if !list()?.contains(&id) { "custody_missing" } else { "custody_denied" }, id.name()));
    }
    check_status(result.status, false)?; receive(&parent)
}
pub(crate) fn wait_ready(timeout: Duration) -> Result<(), StepError> {
    let wall = timeout.checked_add(READY_MARGIN).ok_or_else(|| failure("io_error", "invalid readiness timeout"))?;
    let mut cmd = command("s6-svwait"); cmd.args(["-U", "-t"]).arg(timeout.as_millis().to_string()).arg(SERVICE_DIR);
    let result = execute(cmd, wall, false, "custody_unavailable")?; check_status(result.status, true)
}
pub(crate) fn passback(args: &[String]) -> ExitCode {
    let Some(fd) = args.first().filter(|_| args.len() == 1).and_then(|v| v.parse::<RawFd>().ok()).filter(|fd| *fd > 3) else { return ExitCode::from(PASSBACK_PROTOCOL_EXIT); };
    let mut payload = [b'F']; let mut control: [libc::cmsghdr; 2] = unsafe { std::mem::zeroed() };
    let mut iov = libc::iovec { iov_base: payload.as_mut_ptr().cast(), iov_len: 1 };
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut iov; message.msg_iovlen = 1; message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as _) as usize };
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&message); (*cmsg).cmsg_level = libc::SOL_SOCKET; (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as _) as usize;
        libc::CMSG_DATA(cmsg).cast::<RawFd>().write(0);
        if libc::sendmsg(fd, &message, libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT) == 1 { ExitCode::SUCCESS } else { ExitCode::from(PASSBACK_PROTOCOL_EXIT) }
    }
}
async fn capture_watch(mut pipe: impl tokio::io::AsyncRead + Unpin) {
    use tokio::io::AsyncReadExt;
    let (mut captured, mut bytes) = (Vec::new(), [0u8; 1024]);
    while let Ok(n) = pipe.read(&mut bytes).await {
        if n == 0 { break; }
        let keep = n.min(OUTPUT_LIMIT - captured.len()); captured.extend_from_slice(&bytes[..keep]);
    }
}
pub(crate) struct DaemonWatch { child: tokio::process::Child, drains: Vec<tokio::task::JoinHandle<()>> }
impl DaemonWatch {
    pub(crate) fn start() -> Result<Self, StepError> {
        if tokio::runtime::Handle::try_current().is_err() { return Err(failure("io_error", "daemon watch requires the runner runtime")); }
        let mut cmd = tokio::process::Command::new("s6-svwait");
        cmd.args(["-d", SERVICE_DIR]).env_clear().env("PATH", tool_path()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0).kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|_| failure("io_error", "daemon watch spawn failed"))?;
        let out = tokio::spawn(capture_watch(child.stdout.take().unwrap())); let err = tokio::spawn(capture_watch(child.stderr.take().unwrap()));
        Ok(Self { child, drains: vec![out, err] })
    }
    pub(crate) async fn down(&mut self) -> Result<(), StepError> {
        let result = self.child.wait().await;
        for task in &self.drains { task.abort(); }
        match result {
            Ok(status) if matches!(status.code(), Some(0 | 102)) => Ok(()),
            _ => { self.stop(); Err(failure("io_error", "daemon watch failed")) },
        }
    }
    fn stop(&mut self) {
        if matches!(self.child.try_wait(), Ok(Some(_))) { return; }
        if let Some(pid) = self.child.id() { unsafe { libc::kill(-(pid as i32), libc::SIGKILL); } }
        let _ = self.child.start_kill();
        loop {
            match self.child.try_wait() { Ok(None) => std::thread::sleep(Duration::from_millis(5)), _ => break }
        }
    }
}
impl Drop for DaemonWatch {
    fn drop(&mut self) { self.stop(); for task in &self.drains { task.abort(); } }
}

#[cfg(test)]
#[derive(Clone)]
struct TestSettings { path: std::ffi::OsString, passback: std::path::PathBuf }
#[cfg(test)]
thread_local! { static TEST_SETTINGS: std::cell::RefCell<Option<TestSettings>> = const { std::cell::RefCell::new(None) }; }

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::{self, File}, os::{fd::{AsFd, AsRawFd}, unix::fs::{MetadataExt, PermissionsExt}}, path::{Path, PathBuf}, process::Command, time::Instant};
    const CHILD: &str = "upgrade::fdholder::tests::passback_child";
    fn quote(path: &Path) -> String { format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''")) }
    struct SettingsGuard(Option<TestSettings>);
    impl SettingsGuard {
        fn set(root: &Path) -> Self {
            Self(TEST_SETTINGS.with(|v| v.replace(Some(TestSettings {
                path: format!("{}:/usr/bin:/bin", root.join("tools").display()).into(), passback: root.join("passback"),
            }))))
        }
    }
    impl Drop for SettingsGuard { fn drop(&mut self) { TEST_SETTINGS.with(|v| v.replace(self.0.take())); } }
    struct Fixture { _settings: SettingsGuard, dir: tempfile::TempDir }
    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap(); let root = dir.path();
            fs::create_dir(root.join("tools")).unwrap(); fs::create_dir(root.join("A")).unwrap(); fs::create_dir(root.join("D")).unwrap();
            fs::write(root.join("state"), "").unwrap();
            let settings = SettingsGuard::set(root); let f = Self { _settings: settings, dir };
            f.script("s6-fdholder-list", &format!("/bin/cat {}", quote(&f.path("state"))));
            f.script("s6-fdholder-store", &format!(r#"/usr/bin/stat -Lc '%d:%i' /proc/self/fd/0 > {}
printf '%s\n' "$4" >> {}"#, quote(&f.path("stdin")), quote(&f.path("state"))));
            f.script("s6-fdholder-delete", &format!(r#"/bin/grep -vxF "$4" {} > {} || true
/bin/mv {} {}"#, quote(&f.path("state")), quote(&f.path("next")), quote(&f.path("next")), quote(&f.path("state"))));
            f.script("s6-fdholder-retrieve", &format!(r#"case "$4" in lock:A) input={};; lock:D) input={};; *) exit 100;; esac
exec "$5" "$6" "$7" < "$input""#, quote(&f.path("A")), quote(&f.path("D"))));
            f.script("s6-svwait", "exit 0");
            let wrapper = format!(r#"#!/bin/sh
NWUI_PASSBACK_FD="$2" exec {} --exact {} --nocapture
"#, quote(&std::env::current_exe().unwrap()), CHILD);
            fs::write(f.path("passback"), wrapper).unwrap(); fs::set_permissions(f.path("passback"), fs::Permissions::from_mode(0o755)).unwrap(); f
        }
        fn path(&self, name: &str) -> PathBuf { self.dir.path().join(name) }
        fn script(&self, name: &str, body: &str) {
            let script = format!(r#"#!/bin/sh
printf '%s\n' '{name}' >> {calls}
printf '%s\n' "$@" > {args}
{body}
"#, calls=quote(&self.path("calls")), args=quote(&self.path(&format!("args-{name}"))));
            let path = self.path("tools").join(name); fs::write(&path, script).unwrap(); fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fn state(&self, value: &[u8]) { fs::write(self.path("state"), value).unwrap(); }
        fn calls(&self) -> Vec<String> { fs::read_to_string(self.path("calls")).unwrap_or_default().lines().map(str::to_owned).collect() }
        fn args(&self, tool: &str) -> Vec<String> { fs::read_to_string(self.path(&format!("args-{tool}"))).unwrap().lines().map(str::to_owned).collect() }
        fn reset_calls(&self) { fs::write(self.path("calls"), "").unwrap(); }
        fn fd(&self, id: CustodyId) -> File { File::open(self.path(if id == CustodyId::A { "A" } else { "D" })).unwrap() }
    }
    fn error<T>(result: Result<T, StepError>, code: &str) { assert_eq!(result.err().unwrap().code, code); }

    #[test]
    fn list_empty_and_both_ids_and_every_error_mapping() {
        let f = Fixture::new(); assert!(list().unwrap().is_empty());
        f.state(b"lock:D\nlock:A\n"); assert_eq!(list().unwrap(), BTreeSet::from([CustodyId::A, CustodyId::D]));
        assert_eq!(f.args("s6-fdholder-list"), ["-t", "2000", SOCKET]);
        for output in [b"lock:X\n".as_slice(), b"lock:A \n", b"\n", b"lock:A\nlock:A\n", b"\xff"] {
            f.state(output); error(list(), "custody_protocol");
        }
        for (status, code) in [(111, "custody_unavailable"), (1, "custody_denied"), (100, "io_error"), (42, "io_error")] {
            f.script("s6-fdholder-list", &format!("exit {status}")); f.reset_calls(); error(list(), code);
            assert_eq!(f.calls(), ["s6-fdholder-list"]);
        }
        fs::remove_file(f.path("tools/s6-fdholder-list")).unwrap(); error(list(), "io_error");
    }
    #[test]
    fn store_passes_stdin_preserves_original_and_verifies_before_and_after() {
        let f = Fixture::new(); let fd = f.fd(CustodyId::A); let metadata = fd.metadata().unwrap();
        store(CustodyId::A, fd.as_fd()).unwrap();
        assert_eq!(fs::read_to_string(f.path("stdin")).unwrap().trim(), format!("{}:{}", metadata.dev(), metadata.ino()));
        assert_eq!(f.calls(), ["s6-fdholder-list", "s6-fdholder-store", "s6-fdholder-list"]);
        assert_eq!(f.args("s6-fdholder-store"), ["-t", "2000", SOCKET, "lock:A"]);
        assert_eq!(fd.metadata().unwrap().ino(), metadata.ino());
        f.reset_calls(); error(store(CustodyId::A, fd.as_fd()), "custody_present"); assert_eq!(f.calls(), ["s6-fdholder-list"]);
        f.state(b""); f.script("s6-fdholder-store", "exit 0"); error(store(CustodyId::A, fd.as_fd()), "custody_unavailable");
        f.state(b"lock:D\n"); error(store(CustodyId::A, fd.as_fd()), "custody_protocol");
    }
    #[test]
    fn empty_daemon_after_store_is_transient_and_repopulation_recovers() {
        let f=Fixture::new();let locks=super::super::StorageLocks::acquire(&f.path("D"),&f.path("A")).unwrap();
        f.script("s6-fdholder-store",&format!("if [ ! -e {flag} ]; then /usr/bin/touch {flag}; : > {state}; exit 0; fi\nprintf '%s\\n' \"$4\" >> {state}",flag=quote(&f.path("restarted")),state=quote(&f.path("state"))));
        super::super::custody::wait_and_publish(&locks,true).unwrap();assert_eq!(list().unwrap(),BTreeSet::from([CustodyId::A,CustodyId::D]));
        assert_eq!(f.calls().iter().filter(|name|name.as_str()=="s6-fdholder-store").count(),3);assert!(f.fd(CustodyId::A).try_lock().is_err());assert!(f.fd(CustodyId::D).try_lock().is_err());
    }
    #[test]
    fn delete_is_idempotent_but_success_must_agree_with_the_post_list() {
        let f = Fixture::new(); delete(CustodyId::D).unwrap(); assert_eq!(f.calls(), ["s6-fdholder-list"]);
        f.reset_calls(); f.state(b"lock:A\nlock:D\n"); delete(CustodyId::D).unwrap();
        assert_eq!(f.args("s6-fdholder-delete"), ["-t", "2000", SOCKET, "lock:D"]);
        assert_eq!(list().unwrap(), BTreeSet::from([CustodyId::A]));
        f.state(b"lock:D\n"); f.script("s6-fdholder-delete", "exit 0"); error(delete(CustodyId::D), "custody_protocol");
    }
    #[test]
    fn exit_one_is_reconciled_once_and_a_failed_followup_uses_its_own_code() {
        let f = Fixture::new(); let fd = f.fd(CustodyId::A);
        f.script("s6-fdholder-store", &format!("printf 'lock:A\\n' > {}\nexit 1", quote(&f.path("state"))));
        error(store(CustodyId::A, fd.as_fd()), "custody_present");
        f.state(b""); f.script("s6-fdholder-store", "exit 1"); error(store(CustodyId::A, fd.as_fd()), "custody_denied");
        f.state(b"lock:A\n"); f.script("s6-fdholder-retrieve", &format!(": > {}\nexit 1", quote(&f.path("state"))));
        error(retrieve(CustodyId::A), "custody_missing");
        f.state(b"lock:A\n"); f.script("s6-fdholder-retrieve", "exit 1"); error(retrieve(CustodyId::A), "custody_denied");
        f.script("s6-fdholder-delete", &format!(": > {}\nexit 1", quote(&f.path("state")))); delete(CustodyId::A).unwrap();
        f.state(b"lock:A\n"); f.script("s6-fdholder-delete", "exit 1"); error(delete(CustodyId::A), "custody_denied");
        let next = f.path("tools/s6-fdholder-list");
        f.script("s6-fdholder-delete", &format!("printf '#!/bin/sh\\nexit 111\\n' > {}\nexit 1", quote(&next)));
        error(delete(CustodyId::A), "custody_unavailable");
    }
    #[test]
    fn retrieve_gets_a_cloexec_directory_copy_without_deleting_the_id() {
        let f = Fixture::new(); error(retrieve(CustodyId::A), "custody_missing");
        for id in [CustodyId::A, CustodyId::D] {
            f.state(b"lock:A\nlock:D\n"); let fd = retrieve(id).unwrap(); let m = fs::metadata(format!("/proc/self/fd/{}", fd.as_raw_fd())).unwrap();
            let args = f.args("s6-fdholder-retrieve"); assert_eq!(args.len(), 7); assert!(!args.iter().any(|v| v == "-D"));
            assert_eq!(&args[..4], ["-t", "2000", SOCKET, id.name()]); assert_eq!(args[4], f.path("passback").to_str().unwrap());
            assert_eq!(args[5], "fdholder-passback"); assert!(args[6].parse::<RawFd>().unwrap() > 3);
            let wanted = f.fd(id).metadata().unwrap(); assert!(m.is_dir()); assert_eq!((m.dev(), m.ino()), (wanted.dev(), wanted.ino()));
            assert_ne!(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC, 0);
            assert_eq!(list().unwrap(), BTreeSet::from([CustodyId::A, CustodyId::D]));
        }
    }
    #[test]
    fn retrieve_refuses_no_fd_and_wrong_type_and_never_logs_fd_contents() {
        let f = Fixture::new(); f.state(b"lock:A\n"); f.script("s6-fdholder-retrieve", "exit 0"); error(retrieve(CustodyId::A), "custody_protocol");
        let file = f.path("secret-file"); fs::write(&file, "secret-marker-fd-data").unwrap();
        f.script("s6-fdholder-retrieve", &format!(r#"exec "$5" "$6" "$7" < {}"#, quote(&file)));
        let err = retrieve(CustodyId::A).err().unwrap(); assert_eq!(err.code, "custody_protocol");
        assert!(!format!("{err:?}").contains("secret-marker"));
    }
    #[test]
    fn list_stdout_is_bounded_but_stderr_is_truncated_without_failure() {
        let f = Fixture::new(); f.script("s6-fdholder-list", "/usr/bin/head -c 4097 /dev/zero"); error(list(), "custody_protocol");
        f.state(b"lock:A\n"); f.script("s6-fdholder-list", &format!("/bin/cat {}\n/usr/bin/head -c 16384 /dev/zero >&2", quote(&f.path("state"))));
        assert_eq!(list().unwrap(), BTreeSet::from([CustodyId::A]));
    }
    #[test]
    fn readiness_exit_mapping_and_its_own_wall_bound() {
        let f = Fixture::new(); wait_ready(Duration::from_millis(50)).unwrap();
        assert_eq!(f.args("s6-svwait"), ["-U", "-t", "50", SERVICE_DIR]);
        for (status, code) in [(99, "custody_unavailable"), (111, "custody_unavailable"), (100, "io_error")] {
            f.script("s6-svwait", &format!("exit {status}")); error(wait_ready(Duration::from_millis(50)), code);
        }
        f.script("s6-svwait", "exec /bin/sleep 30"); let start = Instant::now();
        error(wait_ready(Duration::from_millis(20)), "custody_unavailable"); assert!(start.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn five_second_tool_wall_kills_and_reaps_the_child() {
        let f = Fixture::new(); f.script("s6-fdholder-list", &format!(r#"printf '%s' "$$" > {}
exec /bin/sleep 30"#, quote(&f.path("pid"))));
        let start = Instant::now(); error(list(), "io_error");
        assert!(start.elapsed() >= Duration::from_secs(5)); assert!(start.elapsed() < Duration::from_secs(10));
        let pid = fs::read_to_string(f.path("pid")).unwrap(); assert!(!Path::new("/proc").join(pid).exists());
    }
    #[tokio::test(flavor = "current_thread")]
    async fn daemon_watch_maps_down_and_supervisor_death_and_other_exits() {
        let f = Fixture::new();
        for status in [0, 102, 42, 111] {
            f.script("s6-svwait", &format!("exit {status}")); let mut watch = DaemonWatch::start().unwrap();
            let result = watch.down().await; if matches!(status, 42 | 111) { error(result, "io_error"); } else { result.unwrap(); }
            assert_eq!(f.args("s6-svwait"), ["-d", SERVICE_DIR]);
        }
    }
    fn fd_count(fd: &File) -> usize {
        let expected = fd.metadata().unwrap();
        fs::read_dir("/proc/self/fd").unwrap().filter_map(|entry| {
            entry.ok().and_then(|entry| fs::metadata(entry.path()).ok())
        }).filter(|metadata| (metadata.dev(), metadata.ino()) == (expected.dev(), expected.ino())).count()
    }
    fn send_fds(socket: &UnixStream, fds: &[RawFd], payload: &[u8]) {
        let mut control: [libc::cmsghdr; 32] = unsafe { std::mem::zeroed() };
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        let mut iov = libc::iovec { iov_base: payload.as_ptr().cast_mut().cast(), iov_len: payload.len() };
        message.msg_iov = &mut iov; message.msg_iovlen = 1;
        if !fds.is_empty() {
            message.msg_control = control.as_mut_ptr().cast();
            message.msg_controllen = unsafe { libc::CMSG_SPACE(std::mem::size_of_val(fds) as _) as usize };
            unsafe {
                let cmsg = libc::CMSG_FIRSTHDR(&message); (*cmsg).cmsg_level = libc::SOL_SOCKET; (*cmsg).cmsg_type = libc::SCM_RIGHTS;
                (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of_val(fds) as _) as usize;
                std::ptr::copy_nonoverlapping(fds.as_ptr(), libc::CMSG_DATA(cmsg).cast::<RawFd>(), fds.len());
            }
        }
        assert_eq!(unsafe { libc::sendmsg(socket.as_raw_fd(), &message, libc::MSG_NOSIGNAL) }, payload.len() as isize);
    }
    #[test]
    fn ancillary_violations_close_all_received_fds_even_when_control_is_truncated() {
        let f = Fixture::new(); let fd = f.fd(CustodyId::A);
        for count in [0, 2, 64] {
            let (parent, peer) = UnixStream::pair().unwrap(); let before = fd_count(&fd);
            send_fds(&peer, &vec![fd.as_raw_fd(); count], b"F"); error(receive(&parent), "custody_protocol");
            assert_eq!(fd_count(&fd), before, "leaked ancillary fds, count={count}");
        }
        let (parent, peer) = UnixStream::pair().unwrap(); let before = fd_count(&fd);
        send_fds(&peer, &[fd.as_raw_fd()], b"XX"); error(receive(&parent), "custody_protocol"); assert_eq!(fd_count(&fd), before);
    }
    #[test]
    fn store_never_unlocks_or_closes_the_callers_open_file_description() {
        let f = Fixture::new(); let fd = f.fd(CustodyId::A);
        assert_eq!(unsafe { libc::flock(fd.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }, 0);
        store(CustodyId::A, fd.as_fd()).unwrap();
        let other = f.fd(CustodyId::A);
        assert_eq!(unsafe { libc::flock(other.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }, -1);
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::EWOULDBLOCK));
    }
    #[test]
    fn cleared_child_environment_does_not_forward_runtime_secrets() {
        let f = Fixture::new();
        f.script("s6-fdholder-list", &format!("/usr/bin/env > {}\n/bin/cat {}", quote(&f.path("environment")), quote(&f.path("state"))));
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "upgrade::fdholder::tests::cleared_environment_child", "--nocapture"])
            .env("NWUI_FDHOLDER_ENV_FIXTURE", f.dir.path()).env("AUTH_SECRET", "secret-marker").env("DATABASE_URL", "secret-marker").env("HOME", "secret-marker").output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let environment = fs::read_to_string(f.path("environment")).unwrap();
        assert!(environment.contains("PATH="));
        for variable in ["secret-marker", "AUTH_SECRET=", "DATABASE_URL=", "HOME=", "NWUI_FDHOLDER_ENV_FIXTURE="] { assert!(!environment.contains(variable)); }
    }
    #[test]
    fn cleared_environment_child() {
        let Some(root) = std::env::var_os("NWUI_FDHOLDER_ENV_FIXTURE") else { return; };
        let _settings = SettingsGuard::set(Path::new(&root)); assert!(list().unwrap().is_empty());
    }
    #[test]
    fn notification_fd_three_is_preserved_across_all_operations() {
        let f = Fixture::new();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "upgrade::fdholder::tests::fd_three_child", "--nocapture"])
            .env("NWUI_FDHOLDER_FD3_FIXTURE", f.dir.path()).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }
    #[test]
    fn fd_three_child() {
        let Some(root) = std::env::var_os("NWUI_FDHOLDER_FD3_FIXTURE") else { return; }; let root = Path::new(&root);
        let _settings = SettingsGuard::set(root); let marker = root.join("notification"); fs::write(&marker, "fd3 sentinel").unwrap();
        let source = File::open(&marker).unwrap(); assert_eq!(unsafe { libc::dup2(source.as_raw_fd(), 3) }, 3);
        assert_eq!(unsafe { libc::fcntl(3, libc::F_SETFD, libc::FD_CLOEXEC) }, 0);
        let expected = source.metadata().unwrap();
        let fd = File::open(root.join("A")).unwrap(); store(CustodyId::A, fd.as_fd()).unwrap();
        let copy = retrieve(CustodyId::A).unwrap(); drop(copy); delete(CustodyId::A).unwrap(); wait_ready(Duration::from_millis(50)).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async { DaemonWatch::start().unwrap().down().await.unwrap(); }); drop(runtime);
        let mut actual = std::mem::MaybeUninit::<libc::stat>::uninit(); assert_eq!(unsafe { libc::fstat(3, actual.as_mut_ptr()) }, 0);
        let actual = unsafe { actual.assume_init() }; assert_eq!((actual.st_dev, actual.st_ino), (expected.dev(), expected.ino()));
        assert_eq!(unsafe { libc::fcntl(3, libc::F_GETFD) }, libc::FD_CLOEXEC);
        assert_eq!(fs::read(marker).unwrap(), b"fd3 sentinel");
    }
    #[tokio::test(flavor = "current_thread")]
    async fn daemon_watch_is_held_beyond_five_seconds_and_drop_kills_and_reaps() {
        let f = Fixture::new();
        f.script("s6-svwait", "/usr/bin/head -c 16384 /dev/zero >&2\nexec /bin/sleep 30");
        let mut watch = DaemonWatch::start().unwrap(); let pid = watch.child.id().unwrap();
        tokio::time::sleep(Duration::from_millis(5100)).await; assert!(watch.child.try_wait().unwrap().is_none());
        drop(watch); assert!(!Path::new("/proc").join(pid.to_string()).exists());
    }
    #[test]
    fn passback_rejects_missing_extra_or_reserved_fd_arguments() {
        for args in [vec![], vec!["3".into()], vec!["-1".into()], vec!["not-fd".into()], vec!["20".into(), "extra".into()]] {
            assert_ne!(passback(&args), ExitCode::SUCCESS);
        }
    }
    #[test]
    fn passback_failure_is_protocol_not_daemon_unavailability() {
        assert!(![0, 1, 99, 100, 111].contains(&PASSBACK_PROTOCOL_EXIT));
        let f = Fixture::new(); f.state(b"lock:A\n");
        f.script("s6-fdholder-retrieve", &format!(r#"exec "$5" "$6" 3 < {}"#, quote(&f.path("A"))));
        error(retrieve(CustodyId::A), "custody_protocol");
    }
    #[test]
    fn forced_low_child_socket_is_moved_above_notification_descriptors() {
        let f = Fixture::new(); f.state(b"lock:A\n");
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "upgrade::fdholder::tests::low_fd_child", "--nocapture"])
            .env("NWUI_FDHOLDER_LOW_FIXTURE", f.dir.path()).output().unwrap();
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }
    #[test]
    fn low_fd_child() {
        let Some(root) = std::env::var_os("NWUI_FDHOLDER_LOW_FIXTURE") else { return; }; let root = Path::new(&root);
        let _settings = SettingsGuard::set(root);
        // Isolated subprocess: deliberately create the (parent=0, child=3) allocation window.
        unsafe { libc::close(0); libc::close(3); }
        let (parent, child) = UnixStream::pair().unwrap();
        assert_eq!((parent.as_raw_fd(), child.as_raw_fd()), (0, 3)); drop(child); drop(parent);
        let fd = retrieve(CustodyId::A).unwrap(); assert!(fd.as_raw_fd() >= 4);
        let metadata = File::from(fd).metadata().unwrap(); let expected = fs::metadata(root.join("A")).unwrap();
        assert_eq!((metadata.dev(), metadata.ino()), (expected.dev(), expected.ino()));
        let args = fs::read_to_string(root.join("args-s6-fdholder-retrieve")).unwrap();
        assert!(args.lines().last().unwrap().parse::<RawFd>().unwrap() >= 4);
        assert_eq!(unsafe { libc::fcntl(3, libc::F_GETFD) }, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::EBADF));
    }
    #[test]
    fn passback_child() {
        let Some(fd) = std::env::var_os("NWUI_PASSBACK_FD") else { return; };
        let result = passback(&[fd.into_string().unwrap()]);
        // The test harness cannot return a process ExitCode; preserve every possible u8 code.
        for code in 0..=u8::MAX { if result == ExitCode::from(code) { std::process::exit(i32::from(code)); } }
        unreachable!("passback returned an unrepresentable exit code");
    }
}
