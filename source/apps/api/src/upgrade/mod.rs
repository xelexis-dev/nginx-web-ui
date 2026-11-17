//! Automatic storage-upgrade runner. Bodies belong to W1.
pub mod journal;
pub(crate) mod reserves;
mod preflight;
mod legacy;
mod source;
mod profile;
mod fdholder;
mod custody;
mod backup;
mod backup_format;
mod selector;
mod replacement;
mod generation;
mod committed;
mod target;
pub(crate) mod runner;
mod refusal;
mod boot;
mod fresh;
mod ready;
mod service;
mod monitor;
mod cleanup;
mod guard;
mod space;
mod diagnostics;
mod signals;
mod retry;
mod rollover;
mod storage_modes;
mod status;
mod commands;
mod operator;
mod key_write;
pub mod boot_identity;
pub mod types;
pub use journal::{Journal, JournalV1, MovePhase, MoveRecord, Step, StepState};
pub use types::{Branch, ErrorClass, KeyBinding, KeySource, Outcome, PrivatePg, StepError, StorageLocks, StorageSelection, class_of, read_selection};

pub async fn run() -> Result<(), StepError> {
    runner::run().await
}

/// Read-only diagnostics; does not load application secrets or acquire runner locks.
pub async fn auto_status(args:&[String])->i32 {commands::auto_status(args).await}

/// Internal root-only fdholder subprocess entry; no runtime configuration or secrets are read.
pub fn fdholder_passback(args:&[String])->std::process::ExitCode {fdholder::passback(args)}

/// Shared read-only selector policy for the path-included offline transport and runner.
#[doc(hidden)]
pub fn precommit_selector(app_root:&std::path::Path,journal:&Journal)->Result<(),StepError>{replacement::precommit(app_root,journal)}

/// Fixed journal-owned diagnostic mapping for the path-included offline transport tests.
#[doc(hidden)]
pub fn diagnostic_log_path(db:&std::path::Path,j:&Journal,name:&str,check:bool)->Result<std::path::PathBuf,StepError>{diagnostics::owned_path(db,j,name,check)}

/// Offline dispatch shared with the path-included db-upgrade transport tests.
#[doc(hidden)]
pub async fn operator_command(command:&str,args:&[String])->Result<String,StepError>{operator::run(command,args).await}


#[doc(hidden)]
pub fn legacy_admission(roots:&crate::db_upgrade::layout::Roots)->Result<(),StepError>{legacy::admit(roots)}

#[cfg(test)] #[path="g18_combined_tests.rs"] mod g18_combined_tests;
