pub use nwui_api::nonroot;
#[path="../src/db_upgrade/error.rs"] mod error;
#[path="../src/db_upgrade/paths.rs"] mod paths;
use std::ffi::OsStr;
#[test]
fn runtime_storage_overrides_are_rejected_before_any_directory_is_created() {
    for (root,data) in [(None,None),(Some(""),Some("")),(Some("/var/lib/postgresql/data"),Some("/var/lib/postgresql/data/17/data"))]{paths::validate(root.map(OsStr::new),data.map(OsStr::new)).unwrap();}
    for (root,data) in [(Some("/custom"),None),(None,Some("/var/lib/postgresql/data")),(None,Some("/var/lib/postgresql/data/17/data/"))]{assert_eq!(paths::validate(root.map(OsStr::new),data.map(OsStr::new)).unwrap_err().code,"storage_override");}
}
