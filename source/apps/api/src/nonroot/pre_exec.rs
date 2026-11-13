//! Root validates the selected receipt and inherits write-only auth FD 4 before exec.
use super::{handles, privilege::{Identity, API_GID, API_UID}, Receipt, RUNTIME_KEY, RECEIPT};
use crate::upgrade::{StepError, read_selection};
use std::{io, os::{fd::AsRawFd, unix::process::CommandExt}, path::Path, process::Command};

pub fn run() -> Result<(), StepError> {
    require_single_threaded().map_err(|_| super::error("privilege_prepare_failed", "다른 스레드가 있어 API 권한 전환을 거부했습니다."))?;
    let config = super::config().map_err(|_| super::error("privilege_prepare_failed", "루트 시작 설정을 읽지 못했습니다."))?;
    let canonical = super::canonical_key(&config)?;
    let result = (|| -> io::Result<()> {
        validate_readiness(3)?;
        let identity = Identity::read()?;
        let selection = read_selection().map_err(|_| handles::invalid("missing storage selection"))?;
        let receipt: Receipt = handles::read_private_json(Path::new(RECEIPT))?;
        receipt.validate(&selection, true)?;
        let policy = crate::db_upgrade::key_file::canonical_policy(Path::new(crate::db_upgrade::paths::APPROOT),&canonical);
        let key = crate::db_upgrade::key_file::read_key_file(&canonical,policy)
            .map_err(|_|handles::invalid("invalid canonical key"))?.bytes;
        receipt.validate_key(&key)?;
        handles::ensure_directory(Path::new(super::RUNTIME_PRIVATE), 0, API_GID, 0o750)?;
        handles::ensure_directory(Path::new(super::RUNTIME_TMP), API_UID, API_GID, 0o700)?;
        handles::atomic_write(Path::new(RUNTIME_KEY), &key, API_GID, 0o440)?;
        let log = handles::open_auth_log(&config.auth_log_file, &identity.groups)?;
        let fd = log.as_raw_fd();
        let mut command = Command::new("/usr/local/bin/nwui-api");
        command.arg("serve").env("DATA_KEY_FILE", RUNTIME_KEY).env("TMPDIR", super::RUNTIME_TMP).env("NWUI_READY_FD", "3").env("NWUI_AUTH_LOG_FD", "4").env_remove("ADMIN_EMAIL").env_remove("ADMIN_PASSWORD");
        unsafe { command.pre_exec(move || {
            if libc::dup2(fd, 4) == -1 || libc::fcntl(4, libc::F_SETFD, 0) == -1 { return Err(io::Error::last_os_error()); }
            // FD 3 is s6's already-open readiness pipe; neither handle is reopened by path.
            if libc::fcntl(3, libc::F_GETFD) == -1 { return Err(io::Error::last_os_error()); }
            if libc::fcntl(3, libc::F_SETFD, 0) == -1 { return Err(io::Error::last_os_error()); }
            identity.drop(API_UID, API_GID)?;
            if libc::syscall(libc::SYS_close_range, 5u32, u32::MAX, libc::CLOSE_RANGE_CLOEXEC) == -1 { return Err(io::Error::last_os_error()); }
            Ok(())
        }); }
        Err(command.exec())
    })();
    result.map_err(|_| super::error("privilege_prepare_failed", "API 전용 권한으로 시작하지 못했습니다."))
}

fn require_single_threaded() -> io::Result<()> {
    validate_thread_count(&std::fs::read_to_string("/proc/self/status")?)
}

fn validate_thread_count(status: &str) -> io::Result<()> {
    if status.lines().find_map(|line| line.strip_prefix("Threads:")).map(str::trim) != Some("1") {
        return Err(handles::invalid("single-threaded pre-exec required"));
    }
    Ok(())
}

fn validate_readiness(fd: i32) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || flags & libc::O_ACCMODE != libc::O_WRONLY {
        return Err(handles::invalid("missing write-only readiness pipe"));
    }
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut stat) } == -1 || stat.st_mode & libc::S_IFMT != libc::S_IFIFO {
        return Err(handles::invalid("readiness descriptor is not a pipe"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    #[test]
    fn single_thread_status_is_required() {
        validate_thread_count("Name:\tnwui-api\nThreads:\t1\n").unwrap();
        for status in ["", "Threads:\t0\n", "Threads:\t2\n", "Threads:\tinvalid\n"] {
            assert!(validate_thread_count(status).is_err(), "{status}");
        }
    }

    #[test]
    fn spawned_thread_refuses_pre_exec_before_preparation() {
        let (ready, started) = std::sync::mpsc::channel();
        let (finish, wait) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || { ready.send(()).unwrap(); wait.recv().unwrap(); });
        started.recv().unwrap();
        let guard = require_single_threaded();
        let result = run();
        finish.send(()).unwrap();
        worker.join().unwrap();
        assert!(guard.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.code, "privilege_prepare_failed");
        assert_eq!(error.message, "다른 스레드가 있어 API 권한 전환을 거부했습니다.");
    }

    #[test]
    fn readiness_must_be_an_inherited_write_only_pipe_not_a_reopened_file() {
        let mut pipes = [-1;2];
        assert_eq!(unsafe { libc::pipe2(pipes.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
        let read = unsafe { OwnedFd::from_raw_fd(pipes[0]) };
        let write = unsafe { OwnedFd::from_raw_fd(pipes[1]) };
        validate_readiness(write.as_raw_fd()).unwrap();
        assert!(validate_readiness(read.as_raw_fd()).is_err());
        let file = tempfile::tempfile().unwrap();
        assert!(validate_readiness(file.as_raw_fd()).is_err());
        assert!(validate_readiness(-1).is_err());
    }
}
