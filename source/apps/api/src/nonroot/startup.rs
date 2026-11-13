//! Trusted startup values, usable before F1 has published AUTH_SECRET.
use std::{ffi::CString, fs::File, io::Read, os::{fd::{AsRawFd, FromRawFd}, unix::fs::MetadataExt}, path::{Component, Path}};

/// UTF-8 process environment wins (including an empty value), then root's s6 envdir.
/// Callers use fixed variable names; no RuntimeConfig or secret is required.
pub fn startup_value(name: &str) -> Option<String> {
    read_value(name, std::env::var(name).ok(), Path::new("/run/s6/container_environment"))
}

/// Runner credentials bypass inherited image defaults, using the same trusted file reader.
pub(crate) fn envdir_value(name:&str)->Option<String>{read_value(name,None,Path::new("/run/s6/container_environment"))}

/// Every runner-process config reader uses published credentials; other startup values keep their precedence.
pub(crate) fn runner_value(name:&str)->Option<String> {
    match name {
        "AUTH_SECRET" | "DATABASE_URL" | "ADMIN_PASSWORD" => envdir_value(name),
        _ => startup_value(name),
    }
}

fn read_value(name: &str, process_value: Option<String>, envdir: &Path) -> Option<String> {
    process_value.or_else(|| {
        let mut components = Path::new(name).components();
        if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() { return None; }
        let directory = open_envdir(envdir)?;
        let name = CString::new(name).ok()?;
        let raw = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), libc::O_RDONLY|libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC) };
        if raw == -1 { return None; }
        let file = unsafe { File::from_raw_fd(raw) };
        let meta = file.metadata().ok()?;
        if !meta.is_file() || meta.uid() != 0 || meta.mode() & 0o022 != 0 || meta.len() > 65536 { return None; }
        let mut value = String::new(); file.take(65537).read_to_string(&mut value).ok()?;
        (value.len() <= 65536).then(|| value.trim_end_matches(['\n','\0']).to_owned())
    })
}

fn open_envdir(path: &Path) -> Option<File> {
    if !path.is_absolute() { return None; }
    let raw = unsafe { libc::open(c"/".as_ptr(), libc::O_RDONLY|libc::O_DIRECTORY|libc::O_CLOEXEC) };
    if raw == -1 { return None; }
    let mut directory = unsafe { File::from_raw_fd(raw) };
    for component in path.components() {
        match component {
            Component::RootDir => {},
            Component::Normal(part) => {
                let parent = directory.metadata().ok()?;
                if parent.uid() != 0 || (parent.mode() & 0o022 != 0 && parent.mode() & libc::S_ISVTX == 0) { return None; }
                let part = CString::new(part.as_encoded_bytes()).ok()?;
                let raw = unsafe { libc::openat(directory.as_raw_fd(), part.as_ptr(), libc::O_RDONLY|libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC) };
                if raw == -1 { return None; }
                directory = unsafe { File::from_raw_fd(raw) };
            },
            _ => return None,
        }
    }
    let meta = directory.metadata().ok()?;
    (meta.uid() == 0 && meta.mode() & 0o022 == 0).then_some(directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::{symlink, PermissionsExt}};

    #[test]
    fn utf8_process_value_including_empty_wins_before_the_root_envdir() {
        let temp = tempfile::tempdir().unwrap(); fs::write(temp.path().join("DATA_KEY_FILE"), "/from-file\n").unwrap();
        assert_eq!(read_value("DATA_KEY_FILE", Some("/from-process".into()), temp.path()), Some("/from-process".into()));
        assert_eq!(read_value("DATA_KEY_FILE", Some(String::new()), temp.path()), Some(String::new()));
    }
    #[test]
    fn key_path_fallback_needs_no_auth_secret_and_keeps_the_existing_trimming() {
        let temp = tempfile::tempdir().unwrap(); fs::write(temp.path().join("DATA_KEY_FILE"), "/from-file\n\0").unwrap();
        assert!(!temp.path().join("AUTH_SECRET").exists());
        assert_eq!(read_value("DATA_KEY_FILE", None, temp.path()), Some("/from-file".into()));
        assert_eq!(read_value("AUTH_SECRET", None, temp.path()), None);
    }
    #[test]
    fn envdir_refuses_links_foreign_owners_nonregular_oversize_and_non_utf8_files() {
        let temp = tempfile::tempdir().unwrap(); let file = temp.path().join("DATA_KEY_FILE");
        fs::write(&file, "/original").unwrap(); let link = temp.path().join("LINK"); symlink(&file,&link).unwrap();
        assert!(read_value("LINK", None,temp.path()).is_none());
        let parent_link = temp.path().join("linked-parent"); symlink(temp.path(), &parent_link).unwrap();
        assert!(read_value("DATA_KEY_FILE",None,&parent_link).is_none());
        let name = CString::new(file.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe {libc::chown(name.as_ptr(),10001,10001)},0);
        assert!(read_value("DATA_KEY_FILE",None,temp.path()).is_none());
        assert_eq!(unsafe {libc::chown(name.as_ptr(),0,0)},0);
        fs::write(&file, vec![b'x';65537]).unwrap(); assert!(read_value("DATA_KEY_FILE",None,temp.path()).is_none());
        fs::write(&file,[0xff]).unwrap(); assert!(read_value("DATA_KEY_FILE",None,temp.path()).is_none());
        fs::remove_file(&file).unwrap(); fs::create_dir(&file).unwrap(); assert!(read_value("DATA_KEY_FILE",None,temp.path()).is_none());
        fs::set_permissions(temp.path(),fs::Permissions::from_mode(0o777)).unwrap();
        assert!(read_value("DATA_KEY_FILE",None,temp.path()).is_none());
        assert_eq!(read_value("DATA_KEY_FILE",Some("process".into()),temp.path()),Some("process".into()));
    }
    #[test]
    fn variable_name_cannot_escape_the_fixed_envdir() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["", "..", "../key", "/etc/passwd", "a/b", "BAD\0KEY"] { assert!(read_value(name,None,temp.path()).is_none()); }
    }
}
