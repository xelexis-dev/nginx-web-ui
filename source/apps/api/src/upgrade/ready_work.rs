//! Success cleanup is scoped to the directory and leaves opened by this attempt.
use super::{StepError, cleanup::{self, Boundary}};
use crate::db_upgrade::postgres;
use std::{
    ffi::{CStr, CString}, fs::{File, OpenOptions},
    os::{fd::AsRawFd, unix::fs::{MetadataExt, OpenOptionsExt}},
    path::{Path, PathBuf},
};

fn invalid() -> StepError {
    StepError {
        code: "io_error",
        message: "전용 PostgreSQL 작업 폴더를 안전하게 확인하지 못했습니다. 기존 작업을 보존했습니다.",
        detail: None,
    }
}

pub(super) struct ReadyWork {
    pub path: PathBuf,
    run: File,
    directory: File,
    name: CString,
}

impl ReadyWork {
    pub(super) fn create(run_path: &Path) -> Result<Self, StepError> {
        let run = OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(run_path).map_err(|_| invalid())?;
        let metadata = run.metadata().map_err(|_| invalid())?;
        if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 { return Err(invalid()); }
        let leaf = format!("nwui-ready-{}", uuid::Uuid::new_v4());
        let name = CString::new(leaf.as_str()).expect("UUID leaf is ASCII");
        if unsafe { libc::mkdirat(run.as_raw_fd(), name.as_ptr(), 0o700) } != 0 { return Err(invalid()); }
        let directory = cleanup::opened(&run, &name, libc::O_RDONLY | libc::O_DIRECTORY)?.ok_or_else(invalid)?;
        let (_, gid) = postgres::postgres_ids().map_err(|_| invalid())?;
        // PostgreSQL can traverse the directory, while only root controls its entry names.
        if unsafe { libc::fchown(directory.as_raw_fd(), 0, gid) } != 0
            || unsafe { libc::fchmod(directory.as_raw_fd(), 0o710) } != 0 {
            return Err(invalid());
        }
        Ok(Self { path: run_path.join(leaf), run, directory, name })
    }

    pub(super) fn clean_success(self, files: Option<postgres::ReadyFiles>) {
        if files.is_none_or(|files| self.remove(files).is_err()) {
            eprintln!("[upgrade] 작업 폴더를 정리하지 못해 그대로 두었습니다.");
        }
    }

    fn remove(&self, files: postgres::ReadyFiles) -> Result<(), StepError> {
        let boundary = cleanup::boundary(&self.run)?;
        for (name, file) in &files {
            self.check_leaf(&self.directory, name, file, &boundary)?;
        }
        for (name, file) in files {
            self.check_leaf(&self.run, &self.name, &self.directory, &boundary)?;
            self.check_leaf(&self.directory, name, &file, &boundary)?;
            unlink(&self.directory, name, 0)?;
        }
        self.check_leaf(&self.run, &self.name, &self.directory, &boundary)?;
        unlink(&self.run, &self.name, libc::AT_REMOVEDIR)
    }

    fn check_leaf(&self, parent: &File, name: &CStr, original: &File, boundary: &Boundary) -> Result<(), StepError> {
        let current = cleanup::opened(parent, name, libc::O_PATH)?.ok_or_else(invalid)?;
        cleanup::check_boundary(&current, boundary)?;
        let (current, original) = (current.metadata().map_err(|_| invalid())?, original.metadata().map_err(|_| invalid())?);
        if (current.dev(), current.ino()) != (original.dev(), original.ino()) { return Err(invalid()); }
        if current.is_dir() {
            if current.uid() != 0 || current.mode() & 0o7777 != 0o710 { return Err(invalid()); }
        } else if !current.is_file() || current.nlink() != 1 {
            return Err(invalid());
        }
        Ok(())
    }
}

fn unlink(parent: &File, name: &CStr, flags: libc::c_int) -> Result<(), StepError> {
    if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) } != 0 { return Err(invalid()); }
    Ok(())
}

#[cfg(test)]
mod tests;
