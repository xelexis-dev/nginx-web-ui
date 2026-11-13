//! Only the bound DNS transport drops identity; root still owns startup downgrades.
#[cfg(target_os = "linux")]
pub(super) fn drop_to_node() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    const UID: libc::uid_t = 1000;
    const GID: libc::gid_t = 1000;
    // libc's credential wrappers update all process threads; zero-length group lists use null.
    unsafe {
        ensure!(
            libc::getuid() == 0 && libc::geteuid() == 0,
            "DNS guard setup must start as root"
        );
        if libc::setgroups(0, std::ptr::null()) != 0 {
            return Err(std::io::Error::last_os_error()).context("DNS guard setgroups");
        }
        if libc::setgid(GID) != 0 {
            return Err(std::io::Error::last_os_error()).context("DNS guard setgid");
        }
        if libc::setuid(UID) != 0 {
            return Err(std::io::Error::last_os_error()).context("DNS guard setuid");
        }
        ensure!(
            libc::getuid() == UID && libc::geteuid() == UID,
            "DNS guard uid verification failed"
        );
        ensure!(
            libc::getgid() == GID && libc::getegid() == GID,
            "DNS guard gid verification failed"
        );
        ensure!(
            libc::getgroups(0, std::ptr::null_mut()) == 0,
            "DNS guard supplementary groups were not cleared"
        );
        let regained = libc::setuid(0);
        let error = std::io::Error::last_os_error();
        ensure!(
            regained == -1 && error.raw_os_error() == Some(libc::EPERM),
            "DNS guard root reacquisition was not denied with EPERM"
        );
    }
    Ok(())
}
#[cfg(not(target_os = "linux"))]
pub(super) fn drop_to_node() -> anyhow::Result<()> {
    anyhow::bail!("DNS guard credential transition requires Linux");
}

pub(super) fn drop_and_notify_ready() -> anyhow::Result<()> {
    use anyhow::Context;
    use std::io::Write;
    // A root-owned s6 pipe cannot be reopened after setuid; capture it without signalling yet.
    let ready = if std::env::var("NWUI_READY_FD").ok().as_deref() == Some("3") {
        Some(
            std::fs::OpenOptions::new()
                .write(true)
                .open("/proc/self/fd/3")
                .context("Capture DNS readiness descriptor before privilege drop")?,
        )
    } else {
        None
    };
    drop_to_node()?;
    if let Some(mut pipe) = ready {
        pipe.write_all(b"\n")
            .context("Notify DNS readiness after verified privilege drop")?;
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    const CHILD: &str = "NWUI_DNS_PRIVILEGE_TEST_CHILD";

    fn child(test: &str) -> bool {
        if std::env::var(CHILD).as_deref() == Ok(test) {
            return true;
        }
        // Only an exec'd disposable child may alter process-wide credentials.
        assert_eq!(
            unsafe { libc::geteuid() },
            0,
            "requires a disposable root container"
        );
        let name = format!("dns_guard::privileges::tests::{test}");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &name,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, test)
            .env("NWUI_READY_FD", "3")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "child failed: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            stdout.contains("F10_CHILD_OK"),
            "exact child test must execute: {stdout}"
        );
        for line in stdout.lines().filter(|line| line.contains("F10_CHILD_OK")) {
            eprintln!("{line}");
        }
        assert_eq!(
            unsafe { libc::getuid() },
            0,
            "parent identity must be unchanged"
        );
        false
    }

    #[test]
    #[ignore = "requires disposable Linux root child with SETUID/SETGID"]
    fn bound_transport_drops_all_ids_groups_and_cannot_regain_root() {
        if !child("bound_transport_drops_all_ids_groups_and_cannot_regain_root") {
            return;
        }
        drop_to_node().unwrap();
        // These calls only inspect this disposable child or attempt the forbidden transition.
        unsafe {
            assert_eq!((libc::getuid(), libc::geteuid()), (1000, 1000));
            assert_eq!((libc::getgid(), libc::getegid()), (1000, 1000));
            assert_eq!(libc::getgroups(0, std::ptr::null_mut()), 0);
            assert_eq!(libc::setuid(0), -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EPERM)
            );
        }
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let ids: Vec<_> = status
            .lines()
            .filter(|s| s.starts_with("Uid:") || s.starts_with("Gid:") || s.starts_with("Groups:"))
            .collect();
        assert!(
            ids.iter()
                .filter(|s| s.starts_with("Uid:") || s.starts_with("Gid:"))
                .all(|s| s.split_whitespace().skip(1).collect::<Vec<_>>() == ["1000"; 4])
        );
        println!("F10_CHILD_OK {ids:?}");
    }

    #[test]
    #[ignore = "requires disposable Linux root child with SETUID/SETGID"]
    fn an_unprivileged_start_is_rejected() {
        if !child("an_unprivileged_start_is_rejected") {
            return;
        }
        // This permanently changes only the exec'd child, before exercising the error path.
        unsafe {
            assert_eq!(libc::setgroups(0, std::ptr::null()), 0);
            assert_eq!(libc::setgid(1000), 0);
            assert_eq!(libc::setuid(1000), 0);
        }
        assert!(
            drop_to_node().is_err(),
            "a failed transition must not enter the serving path"
        );
        println!("F10_CHILD_OK rejected non-root start");
    }
    #[test]
    #[ignore = "requires disposable Linux root child with SETUID/SETGID"]
    fn readiness_uses_the_inherited_pipe_after_verified_drop() {
        use std::io::Read;
        use std::os::fd::{AsRawFd, FromRawFd};
        if !child("readiness_uses_the_inherited_pipe_after_verified_drop") {
            return;
        }
        // Keep an actual root-owned pipe reader while assigning its writer to s6's FD3.
        let mut fds = [-1; 2];
        let (mut reader, _writer) = unsafe {
            assert_eq!(libc::pipe(fds.as_mut_ptr()), 0);
            let reader_fd = libc::fcntl(fds[0], libc::F_DUPFD_CLOEXEC, 10);
            assert!(reader_fd >= 10);
            assert_eq!(libc::close(fds[0]), 0);
            let writer = std::fs::File::from_raw_fd(fds[1]);
            assert_eq!(libc::dup2(writer.as_raw_fd(), 3), 3);
            let flags = libc::fcntl(reader_fd, libc::F_GETFL);
            assert_eq!(
                libc::fcntl(reader_fd, libc::F_SETFL, flags | libc::O_NONBLOCK),
                0
            );
            (std::fs::File::from_raw_fd(reader_fd), writer)
        };
        drop_and_notify_ready().unwrap();
        let mut byte = [0];
        reader.read_exact(&mut byte).unwrap();
        assert_eq!(byte, [b'\n']);
        assert_eq!(unsafe { libc::geteuid() }, 1000);
        println!("F10_CHILD_OK readiness emitted after uid drop");
    }
}
