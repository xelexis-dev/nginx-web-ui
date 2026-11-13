//! Runtime storage locations are fixed; conflicting legacy overrides fail before writes.
use super::error::{Error,Result};
use std::ffi::OsStr;
pub const PGROOT:&str="/var/lib/postgresql/data";
pub const PGDATA:&str="/var/lib/postgresql/data/17/data";
pub const APPROOT:&str="/var/lib/nginx-web-ui";
pub fn validate(root:Option<&OsStr>,data:Option<&OsStr>)->Result<()> {
    for (value,expected) in [(root,PGROOT),(data,PGDATA)] {
        if let Some(value)=value {if !value.is_empty()&&value!=OsStr::new(expected){return Err(Error::refusal("storage_override"));}}
    }Ok(())
}
pub fn validate_env()->Result<()> {validate(std::env::var_os("PGROOT").as_deref(),std::env::var_os("PGDATA").as_deref())}

/// Root startup callers pass the original trusted environment value, never the API's /run override.
/// app_root is the fixed APPROOT in the image; explicit roots keep tests/offline mounts isolated.
pub fn resolve_data_key_file(app_root: &std::path::Path, trusted_startup_override: Option<&OsStr>) -> std::path::PathBuf {
    trusted_startup_override.filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| app_root.join("secrets/data_key"))
}

/// Reject invalid raw startup settings before callers perform any storage mutation.
pub fn startup_data_key_file(app_root: &std::path::Path) -> Result<std::path::PathBuf> {
    let raw = crate::nonroot::startup_value("DATA_KEY_FILE");
    if raw.as_deref().is_some_and(|value| value.is_empty() || !std::path::Path::new(value).is_absolute()) {
        return Err(Error::refusal("key_path_invalid"));
    }
    Ok(resolve_data_key_file(app_root, raw.as_deref().map(OsStr::new)))
}

#[cfg(test)]
mod key_path_tests {
    use super::*;
    use std::path::Path;
    #[test]
    fn n0_canonical_key_resolver_preserves_the_trusted_startup_override() {
        assert_eq!(resolve_data_key_file(Path::new(APPROOT), None), Path::new("/var/lib/nginx-web-ui/secrets/data_key"));
        assert_eq!(resolve_data_key_file(Path::new(APPROOT), Some(OsStr::new(""))), Path::new("/var/lib/nginx-web-ui/secrets/data_key"));
        assert_eq!(resolve_data_key_file(Path::new(APPROOT), Some(OsStr::new("/mounted/private/key"))), Path::new("/mounted/private/key"));
        assert_eq!(resolve_data_key_file(Path::new("/isolated/app"), None), Path::new("/isolated/app/secrets/data_key"));
    }
}
