//! Publisher writers are unavailable to API/library callers.
//!
//! ```compile_fail,E0603
//! use nwui_api::publisher::apply;
//! ```
//!
//! ```compile_fail,E0603
//! use nwui_api::publisher::rollback;
//! ```
//!
//! ```compile_fail,E0603
//! use nwui_api::publisher::files;
//! ```
//!
//! ```compile_fail,E0603
//! use nwui_api::publisher::containment;
//! ```
//!
//! ```compile_fail,E0603
//! use nwui_api::publisher::process;
//! ```
//!
//! ```compile_fail,E0603
//! use nwui_api::publisher::nginx_lock;
//! ```
//!
//! ```compile_fail,E0603
//! use nwui_api::publisher::bootstrap;
//! ```
//!
//! ```compile_fail,E0603
//! use nwui_api::publisher::confirmation::begin_uncertain;
//! ```
//!
//! ```compile_fail,E0603
//! use nwui_api::publisher::confirmation::clear_uncertain;
//! ```
//!
//! ```compile_fail,E0603
//! use nwui_api::publisher::confirmation::publish;
//! ```
//!
//! Shared read-only model access remains available.
//! ```
//! use nwui_api::control::model::{ConfigSnapshotData, load_snapshot, apply_uncertain};
//! ```
pub mod boot;
pub mod protocol;
pub(in crate::publisher) mod apply;
pub(in crate::publisher) mod rollback;
pub(in crate::publisher) mod files;
pub(in crate::publisher) mod containment;
pub(in crate::publisher) mod confirmation;
pub(in crate::publisher) mod process;
pub(in crate::publisher) mod nginx_lock;
pub(in crate::publisher) mod bootstrap;
// Shared render/validation code remains in control.
use crate::control::{guard_targets,internal_targets,location_refs,pending,render,reserved_ports,self_proxy,validation};
#[cfg(test)]
use crate::control::resources;
mod store;
mod capacity;
mod journal;
mod poison;
mod recovery;
mod server;
mod dispatcher;
mod coordinator;
mod mutation;
mod derivation;
mod unix_targets;
mod certificates;
mod cert_deploy;
mod trusted_includes;
pub(crate) mod vocab;
mod staging;
mod command;
mod prepare;
mod publication;
mod adoption;
#[cfg(feature = "test-support")]
pub mod test_support;
pub use boot::PublisherStore;
pub(crate) use confirmation::{observe_current,uncertain};
// The old in-place writer shares only this pure predicate until the private move.
pub(crate) use store::payload_path as validate_payload_path;
use crate::upgrade::StepError;

pub async fn run() -> Result<(), StepError> {
    // Capture before opening resources: a missing readiness fd must never become another file.
    let readiness=coordinator::Readiness::capture()?;
    let mut config=crate::config::RuntimeConfig::from_env().map_err(|error|store::step_error("publish_failed",error))?;
    let admitted=crate::nonroot::admit_nginx(&config).await?;
    config.nginx_bin=admitted.control_bin;config.nginx_main_conf=admitted.conf_path;
    coordinator::run(config,readiness).await
}

#[cfg(all(test, unix))]
mod test_helpers;
#[cfg(all(test, unix))]
mod restart_guard_tests;

#[cfg(test)]
mod nginx_start_tests;
