//! One writer of nginx's managed files at a time, across processes.
//!
//! The API applies configuration while s6 may restart nginx, and `nginx/run` runs
//! `nwui-api guard` — a separate process — before every start. Both rewrite, quarantine and
//! restore the same managed files and run `nginx -t`, and the in-process apply mutex cannot
//! see the guard. An exclusive `flock` on a file in conf.d serializes them. The kernel releases
//! it when its holder exits, killed or crashed included, so a dead holder never leaves it held.
//!
//! Not covered: nginx reading its files as it starts after the guard has exited (`exec nginx`
//! in nginx/run). An apply in that gap finds no running nginx, fails its reload and restores
//! its previous files.
use crate::config::RuntimeConfig;
use anyhow::{Context, Result};
use std::{
    fs::{File, TryLockError},
    path::PathBuf,
    time::{Duration, Instant},
};

/// Hidden and without `.conf`: nginx's `include conf.d/*.conf`, the managed-file scans and the
/// interrupted-write cleanup all pass over it.
pub const LOCK_FILE: &str = ".nwui-nginx-files.lock";
const POLL: Duration = Duration::from_millis(100);

pub fn path(config: &RuntimeConfig) -> PathBuf {
    config.nginx_conf_dir.join(LOCK_FILE)
}

/// Held until dropped: closing the file releases the lock.
#[must_use = "the lock is released when this is dropped"]
pub struct NginxFilesLock {
    _file: File,
}

/// Polls instead of blocking, so a waiting apply never occupies a runtime thread. Gives up after
/// `config.nginx_lock_wait` without having touched any file.
pub async fn acquire(config: &RuntimeConfig) -> Result<NginxFilesLock> {
    acquire_with_wait(config, config.nginx_lock_wait).await
}

#[derive(Debug, thiserror::Error)]
#[error("다른 nginx 설정 작업(nginx 재시작 점검 또는 설정 적용)이 {seconds}초 안에 끝나지 않아 설정 파일을 건드리지 않았습니다. 잠시 후 다시 시도하세요.")]
pub(crate) struct LockWaitTimeout { seconds: u64 }

pub(crate) async fn acquire_with_wait(config: &RuntimeConfig, wait: Duration) -> Result<NginxFilesLock> {
    super::files::real_directory(&config.nginx_conf_dir).await?;
    let path = path(config);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("Could not open {}", path.display()))?;
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(NginxFilesLock { _file: file }),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                tokio::time::sleep(POLL).await
            }
            Err(TryLockError::WouldBlock) => return Err(LockWaitTimeout { seconds: wait.as_secs() }.into()),
            Err(TryLockError::Error(e)) => {
                return Err(e).with_context(|| format!("Could not lock {}", path.display()));
            }
        }
    }
}
