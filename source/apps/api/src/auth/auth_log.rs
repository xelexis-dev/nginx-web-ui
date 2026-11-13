//! Append-only failed sign-in log for fail2ban, a port of `apps/web/lib/auth-log.ts`.
//!
//! A failed login is invisible in nginx's access log (success and failure answer alike), so the
//! app emits the signal. Not `audit_logs`: unauthenticated requests must not be able to write a
//! database row each. The email is never written — the file has a wider audience than the audit
//! view, and fail2ban needs only the address.
//!
//! Line: `2026-08-09T14:05:00.123Z nginx-web-ui auth-failure ip=203.0.113.5 reason=bad_credentials`

use std::io::Write as _;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureReason {
    /// Wrong password, wrong/missing 2FA code, unknown or locked account.
    BadCredentials,
    AccountDisabled,
    /// Waiting for approval; fail2ban should not act on it.
    AccountPending,
    /// Refused before any password work: this address is over its cap.
    IpThrottled,
}

impl FailureReason {
    pub fn as_str(self) -> &'static str {
        match self {
            FailureReason::BadCredentials => "bad_credentials",
            FailureReason::AccountDisabled => "account_disabled",
            FailureReason::AccountPending => "account_pending",
            FailureReason::IpThrottled => "ip_throttled",
        }
    }
}

/// Only plain IPv4/IPv6 characters, at most 45: an attacker-shaped value must never become a
/// line fail2ban acts on (it could get the operator's address banned).
fn is_plausible_ip(value: &str) -> bool {
    value.len() <= 45 && !value.is_empty() && value.bytes().all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.')
}

pub fn line(ip: Option<&str>, reason: FailureReason, now_ms: i64) -> String {
    let at = chrono::DateTime::from_timestamp_millis(now_ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let ip = ip.filter(|ip| is_plausible_ip(ip)).unwrap_or("-");
    format!("{at} nginx-web-ui auth-failure ip={ip} reason={}\n", reason.as_str())
}

/// Best effort: never fails the sign-in (read-only or missing directory is only a warning).
pub fn append(path: &Path, ip: Option<&str>, reason: FailureReason, now_ms: i64) {
    let bytes=line(ip,reason,now_ms);
    let marker=std::env::var_os("NWUI_AUTH_LOG_FD");
    let result = match marker.as_deref() {
        Some(value) => (|| {
            if value != "4" {return Err(std::io::Error::other("invalid inherited auth log marker"));}
            inherited_log()?.write_all(bytes.as_bytes())
        })(),
        None => (|| {
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::OpenOptions::new().create(true).append(true).open(path)?.write_all(bytes.as_bytes())
        })(),
    };
    if let Err(error) = result {
        eprintln!("[auth-log] failed to record a sign-in failure: {error}");
        if marker.is_some() {eprint!("{bytes}");}
    }
}

fn inherited_log() -> std::io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd,FromRawFd};
    // Duplicate first: validation and writing use the same open description; FD 4 stays inherited.
    let fd=unsafe {libc::fcntl(4,libc::F_DUPFD_CLOEXEC,5)};
    if fd == -1 {return Err(std::io::Error::last_os_error());}
    let file=unsafe {std::fs::File::from_raw_fd(fd)};
    let flags=unsafe {libc::fcntl(file.as_raw_fd(),libc::F_GETFL)};
    if flags == -1 || flags & libc::O_ACCMODE != libc::O_WRONLY || flags & libc::O_APPEND == 0 || !file.metadata()?.is_file() {
        return Err(std::io::Error::other("invalid inherited auth log descriptor"));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_matches_the_ts_format_and_hides_implausible_addresses() {
        let at = 1_786_284_300_123; // 2026-08-09T14:05:00.123Z
        assert_eq!(line(Some("203.0.113.5"), FailureReason::BadCredentials, at), "2026-08-09T14:05:00.123Z nginx-web-ui auth-failure ip=203.0.113.5 reason=bad_credentials\n");
        assert_eq!(line(Some("2001:db8::1"), FailureReason::IpThrottled, at), "2026-08-09T14:05:00.123Z nginx-web-ui auth-failure ip=2001:db8::1 reason=ip_throttled\n");
        for forged in ["1.2.3.4 reason=x", "evil\nline", "a".repeat(46).as_str(), ""] {
            assert!(line(Some(forged), FailureReason::AccountPending, at).contains(" ip=- reason=account_pending"), "{forged:?}");
        }
        assert!(line(None, FailureReason::AccountDisabled, at).contains(" ip=- reason=account_disabled"));
    }

    #[test]
    fn append_creates_the_file_and_never_panics_when_it_cannot() {
        let dir = std::env::temp_dir().join(format!("nwui-auth-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("nested").join("auth.log");
        append(&file, Some("198.51.100.1"), FailureReason::BadCredentials, 0);
        append(&file, None, FailureReason::IpThrottled, 0);
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.lines().count(), 2);
        // A directory in place of the file: reported, not raised.
        append(&dir, None, FailureReason::BadCredentials, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inherited_fd_child() {
        let Some(path)=std::env::var_os("NWUI_AUTH_LOG_TEST_PATH") else{return;};
        append(Path::new(&path),Some("198.51.100.9"),FailureReason::BadCredentials,0);
    }

    #[test]
    fn marked_auth_log_never_opens_a_path_and_only_accepts_append_only_regular_fd4() {
        use std::{fs,os::{fd::AsRawFd,unix::{ffi::OsStringExt,fs::PermissionsExt,process::CommandExt}},process::Command};
        let temp=tempfile::tempdir().unwrap();
        fs::set_permissions(temp.path(),fs::Permissions::from_mode(0o755)).unwrap();
        let inherited=temp.path().join("inherited.log");
        let fallback=temp.path().join("must-not-open.log");
        let append_file=fs::OpenOptions::new().create_new(true).append(true).open(&inherited).unwrap();
        fs::set_permissions(&inherited,fs::Permissions::from_mode(0o600)).unwrap();
        let read_only=fs::File::open(&inherited).unwrap();
        let read_write=fs::OpenOptions::new().read(true).append(true).open(&inherited).unwrap();
        let no_append=fs::OpenOptions::new().write(true).open(&inherited).unwrap();
        let special=fs::OpenOptions::new().append(true).open("/dev/null").unwrap();
        for (marker,fd,drop_uid,valid) in [
            (std::ffi::OsString::from("4"),Some(append_file.as_raw_fd()),false,true),
            ("4".into(),Some(append_file.as_raw_fd()),true,true),
            ("4".into(),None,false,false),
            ("4".into(),Some(read_only.as_raw_fd()),false,false),
            ("4".into(),Some(read_write.as_raw_fd()),false,false),
            ("4".into(),Some(no_append.as_raw_fd()),false,false),
            ("4".into(),Some(special.as_raw_fd()),false,false),
            ("3".into(),Some(append_file.as_raw_fd()),false,false),
            ("".into(),Some(append_file.as_raw_fd()),false,false),
            (std::ffi::OsString::from_vec(vec![0xff]),Some(append_file.as_raw_fd()),false,false),
        ] {
            let before=fs::read(&inherited).unwrap().len();
            let mut command=Command::new(std::env::current_exe().unwrap());
            command.args(["--exact","auth::auth_log::tests::inherited_fd_child","--nocapture"])
                .env("NWUI_AUTH_LOG_FD",marker).env("NWUI_AUTH_LOG_TEST_PATH",&fallback);
            unsafe {command.pre_exec(move||{
                if let Some(fd)=fd {
                    if libc::dup2(fd,4)==-1 || libc::fcntl(4,libc::F_SETFD,0)==-1 {return Err(std::io::Error::last_os_error());}
                }else{libc::close(4);}
                if drop_uid && (libc::setgroups(0,std::ptr::null())==-1 || libc::setgid(10001)==-1 || libc::setuid(10001)==-1){return Err(std::io::Error::last_os_error());}
                Ok(())
            });}
            let output=command.output().unwrap();assert!(output.status.success(),"{output:?}");
            assert!(!fallback.exists(),"a set marker must NEVER open the path");
            assert_eq!(fs::read(&inherited).unwrap().len()>before,valid);
            if !valid {
                let stderr=String::from_utf8_lossy(&output.stderr);
                assert!(stderr.contains("[auth-log]"));assert!(stderr.contains("auth-failure ip=198.51.100.9"));
            }
        }
    }

}
