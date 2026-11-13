//! Shared fixtures use the current crate's types in publisher unit tests.
use crate::{state::AppState,config::RuntimeConfig,control::model::RenderedConfig};
#[path="../../tests/auth_support/nginx.rs"]
pub mod nginx;
#[path="../../tests/auth_support/restart_guard.rs"]
pub mod restart_guard;
