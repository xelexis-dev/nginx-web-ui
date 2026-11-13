//! Descriptor-relative root writers and read-only API handles; never follow links.
use std::{ffi::{CString, OsStr}, fs::File, io::{self, Read}, os::{fd::{AsRawFd, FromRawFd}, unix::fs::MetadataExt}, path::{Component, Path}};

pub(super) fn invalid(message: &'static str) -> io::Error { io::Error::new(io::ErrorKind::PermissionDenied, message) }
fn name(value: &OsStr) -> io::Result<CString> {
    CString::new(value.as_encoded_bytes()).map_err(|_| invalid("invalid handle path"))
}
fn descriptor(fd: i32) -> io::Result<File> {
    if fd == -1 { Err(io::Error::last_os_error()) } else { Ok(unsafe { File::from_raw_fd(fd) }) }
}
pub(super) fn directory(path: &Path) -> io::Result<File> {
    directory_checked(path, |_,_| Ok(()))
}
fn directory_checked(path: &Path, check: impl Fn(&File, bool) -> io::Result<()>) -> io::Result<File> {
    if !path.is_absolute() { return Err(invalid("handle path must be absolute")); }
    let mut dir = descriptor(unsafe { libc::open(c"/".as_ptr(), libc::O_RDONLY|libc::O_DIRECTORY|libc::O_CLOEXEC) })?;
    for component in path.components() {
        match component {
            Component::RootDir => {},
            Component::Normal(part) => {
                check(&dir, false)?;
                dir = descriptor(unsafe { libc::openat(dir.as_raw_fd(), name(part)?.as_ptr(), libc::O_RDONLY|libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC) })?;
            },
            _ => return Err(invalid("unsafe handle path")),
        }
    }
    check(&dir, true)?;
    Ok(dir)
}
pub(super) fn parent(path: &Path) -> io::Result<(File, CString)> {
    let dir = directory_checked(path.parent().ok_or_else(|| invalid("missing handle parent"))?, |dir, final_parent| {
        let meta = dir.metadata()?;
        if meta.uid() != 0 || (meta.mode() & 0o022 != 0 && (final_parent || meta.mode() & libc::S_ISVTX == 0)) {
            return Err(invalid("unsafe private handle parent"));
        }
        Ok(())
    })?;
    Ok((dir, name(path.file_name().ok_or_else(|| invalid("missing handle name"))?)?))
}

fn api_readonly(meta: &std::fs::Metadata, groups: &[u32], sticky_ancestor: bool) -> io::Result<()> {
    let owned = meta.uid() == super::privilege::API_UID;
    let writable = meta.mode() & 0o002 != 0 || ((meta.gid() == super::privilege::API_GID || groups.contains(&meta.gid())) && meta.mode() & 0o020 != 0);
    if owned || (writable && !(sticky_ancestor && meta.is_dir() && meta.mode() & libc::S_ISVTX != 0)) {
        return Err(invalid("API can replace or modify the trusted handle"));
    }
    Ok(())
}
fn readonly_parent(path: &Path, groups: &[u32]) -> io::Result<(File,CString)> {
    let dir = directory_checked(path.parent().ok_or_else(|| invalid("missing handle parent"))?, |dir, final_parent| {
        let meta = dir.metadata()?;
        api_readonly(&meta, groups, !final_parent)
    })?;
    Ok((dir, name(path.file_name().ok_or_else(|| invalid("missing handle name"))?)?))
}

pub(super) fn open_auth_log(path: &Path, groups: &[u32]) -> io::Result<File> {
    let (dir, file) = readonly_parent(path, groups)?;
    // NONBLOCK prevents an existing FIFO from blocking before fstat rejects it.
    let handle = descriptor(unsafe { libc::openat(dir.as_raw_fd(), file.as_ptr(), libc::O_WRONLY|libc::O_APPEND|libc::O_CREAT|libc::O_NOFOLLOW|libc::O_CLOEXEC|libc::O_NONBLOCK, 0o644) })?;
    let metadata = handle.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.uid() != 0 {
        return Err(invalid("auth log must be a single root-owned regular file"));
    }
    if unsafe { libc::fcntl(handle.as_raw_fd(), libc::F_SETFL, libc::O_WRONLY|libc::O_APPEND) } == -1 {
        return Err(io::Error::last_os_error());
    }
    dir.sync_all()?;
    Ok(handle)
}

pub(super) fn read_runtime_key(path: &Path) -> io::Result<Vec<u8>> {
    let (dir, file) = parent(path)?;
    let meta=dir.metadata()?;
    if meta.uid()!=0 || meta.gid()!=super::privilege::API_GID || meta.mode() & 0o7777 != 0o750 {
        return Err(invalid("unsafe runtime handle parent"));
    }
    let handle = descriptor(unsafe { libc::openat(dir.as_raw_fd(), file.as_ptr(), libc::O_RDONLY|libc::O_NOFOLLOW|libc::O_CLOEXEC|libc::O_NONBLOCK) })?;
    let metadata = handle.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.uid() != 0 || metadata.gid() != super::privilege::API_GID || metadata.mode() & 0o7777 != 0o440 || metadata.len() > 4096 {
        return Err(invalid("unsafe runtime key copy"));
    }
    // Canonical content was parsed once by the shared validator; activation compares these bytes.
    let mut bytes = Vec::new(); handle.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 { return Err(invalid("oversized runtime key copy")); }
    Ok(bytes)
}

#[cfg(test)]
pub(super) fn read_key(path: &Path) -> io::Result<Vec<u8>> {
    crate::db_upgrade::key_file::read_key_file(path,crate::db_upgrade::key_file::KeyPolicy::Strict)
        .map(|key|key.bytes).map_err(io::Error::other)
}
#[cfg(test)]
fn read_override_key(path: &Path, _groups: &[u32]) -> io::Result<Vec<u8>> {
    crate::db_upgrade::key_file::read_key_file(path,crate::db_upgrade::key_file::KeyPolicy::Override)
        .map(|key|key.bytes).map_err(io::Error::other)
}

pub(super) fn create_log_probe(path: &Path, groups: &[u32]) -> io::Result<()> {
    let (dir, entry) = readonly_parent(path, groups)?;
    descriptor(unsafe { libc::openat(dir.as_raw_fd(), entry.as_ptr(), libc::O_WRONLY|libc::O_CREAT|libc::O_EXCL|libc::O_CLOEXEC|libc::O_NOFOLLOW, 0o600) })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::{Read, Write}, os::{fd::AsRawFd, unix::fs::{symlink, PermissionsExt}}};

    #[test]
    fn auth_handle_is_append_only_and_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.log");
        fs::write(&path, b"before\n").unwrap();
        let mut handle = open_auth_log(&path, &[4,101]).unwrap();
        handle.write_all(b"after\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"before\nafter\n");
        assert_eq!(handle.read(&mut [0]).unwrap_err().raw_os_error(), Some(libc::EBADF));
        let flags = unsafe { libc::fcntl(handle.as_raw_fd(), libc::F_GETFL) };
        assert_eq!(flags & libc::O_ACCMODE, libc::O_WRONLY);
        assert_ne!(flags & libc::O_APPEND, 0);
        assert_ne!(unsafe { libc::fcntl(handle.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC, 0);
        fs::write(&path, b"").unwrap(); // copytruncate keeps the inherited inode usable.
        handle.write_all(b"after-rotation\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"after-rotation\n");
    }

    #[test]
    fn auth_log_refuses_symlinks_hardlinks_fifo_and_nonroot_owner() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("original");
        fs::write(&original, b"unchanged").unwrap();
        let linked = dir.path().join("linked");
        symlink(&original, &linked).unwrap();
        assert!(open_auth_log(&linked, &[4,101]).is_err());
        fs::remove_file(&linked).unwrap();
        fs::hard_link(&original, &linked).unwrap();
        assert!(open_auth_log(&linked, &[4,101]).is_err());
        let fifo = dir.path().join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(open_auth_log(&fifo, &[4,101]).is_err());
        let foreign = dir.path().join("foreign");
        fs::write(&foreign, b"foreign").unwrap();
        let name = std::ffi::CString::new(foreign.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::chown(name.as_ptr(), 10001, 10001) }, 0);
        assert!(open_auth_log(&foreign, &[4,101]).is_err());
        assert_eq!(fs::read(original).unwrap(), b"unchanged");
    }

    #[test]
    fn parent_symlink_and_writable_log_parent_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let linked = dir.path().join("linked");
        symlink(dir.path(), &linked).unwrap();
        assert!(open_auth_log(&linked.join("auth.log"), &[4,101]).is_err());
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o777)).unwrap();
        assert!(open_auth_log(&dir.path().join("auth.log"), &[4,101]).is_err());
        assert!(!dir.path().join("auth.log").exists());
    }

    #[test]
    fn key_is_existing_bounded_private_and_never_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data_key");
        assert!(read_key(&path).is_err());
        assert!(!path.exists());
        fs::write(&path, "a".repeat(64)).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_key(&path).unwrap(), vec![b'a';64]);
        let original = format!("\n{}\n", "AB".repeat(32));
        fs::write(&path, &original).unwrap();
        assert_eq!(read_key(&path).unwrap(), original.as_bytes());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_key(&path).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, "a".repeat(4097)).unwrap();
        assert!(read_key(&path).is_err());
    }
}

pub(super) fn ensure_directory(path: &Path, uid: u32, gid: u32, mode: u32) -> io::Result<File> {
    let (parent, entry) = parent(path)?;
    if unsafe { libc::mkdirat(parent.as_raw_fd(), entry.as_ptr(), mode) } == -1 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EEXIST) { return Err(error); }
    }
    let dir = descriptor(unsafe { libc::openat(parent.as_raw_fd(), entry.as_ptr(), libc::O_RDONLY|libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC) })?;
    if dir.metadata()?.uid() != 0 && dir.metadata()?.uid() != uid { return Err(invalid("foreign runtime directory")); }
    if unsafe { libc::fchown(dir.as_raw_fd(), uid, gid) } == -1 || unsafe { libc::fchmod(dir.as_raw_fd(), mode) } == -1 {
        return Err(io::Error::last_os_error());
    }
    dir.sync_all()?;
    parent.sync_all()?;
    Ok(dir)
}

pub(super) fn atomic_write(path: &Path, bytes: &[u8], gid: u32, mode: u32) -> io::Result<()> {
    use std::io::Write;
    let (dir, destination) = parent(path)?;
    let existing = unsafe { libc::openat(dir.as_raw_fd(), destination.as_ptr(), libc::O_RDONLY|libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC) };
    if existing != -1 {
        let old = descriptor(existing)?.metadata()?;
        if !old.is_file() || old.nlink() != 1 || old.uid() != 0 { return Err(invalid("unsafe runtime handle")); }
    } else if io::Error::last_os_error().raw_os_error() != Some(libc::ENOENT) { return Err(io::Error::last_os_error()); }
    let temporary = CString::new(format!(".nwui-{}.tmp", uuid::Uuid::new_v4())).unwrap();
    let result = (|| {
        let mut file = descriptor(unsafe { libc::openat(dir.as_raw_fd(), temporary.as_ptr(), libc::O_WRONLY|libc::O_CREAT|libc::O_EXCL|libc::O_NOFOLLOW|libc::O_CLOEXEC, 0o600) })?;
        file.write_all(bytes)?;
        if unsafe { libc::fchown(file.as_raw_fd(), 0, gid) } == -1 || unsafe { libc::fchmod(file.as_raw_fd(), mode) } == -1 {
            return Err(io::Error::last_os_error());
        }
        file.sync_all()?;
        if unsafe { libc::renameat(dir.as_raw_fd(), temporary.as_ptr(), dir.as_raw_fd(), destination.as_ptr()) } == -1 { return Err(io::Error::last_os_error()); }
        dir.sync_all()
    })();
    // Only this invocation owns this unpredictable temporary name.
    unsafe { libc::unlinkat(dir.as_raw_fd(), temporary.as_ptr(), 0); }
    result
}

pub(super) fn read_private_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<T> {
    let bytes=read_runtime_key(path)?;
    serde_json::from_slice(&bytes).map_err(|_| invalid("invalid handle receipt"))
}

#[cfg(test)]
mod durability_tests {
    use super::*;
    use std::{fs, os::unix::fs::{symlink, PermissionsExt}};

    #[test]
    fn trusted_override_keeps_legacy_owner_and_mode_but_refuses_api_write_access() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("key");
        fs::write(&path, "AB".repeat(32)).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let name = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::chown(name.as_ptr(), 501, 501) }, 0);
        assert_eq!(read_override_key(&path, &[4,101]).unwrap(), "AB".repeat(32).as_bytes());
        assert!(read_key(&path).is_err());
        assert_eq!(unsafe { libc::chown(name.as_ptr(), 10001, 501) }, 0);
        assert!(read_override_key(&path, &[4,101]).is_err());
        assert_eq!(unsafe { libc::chown(name.as_ptr(), 0, 10001) }, 0);
        assert!(read_override_key(&path, &[4,101]).is_err());
        assert_eq!(unsafe { libc::chown(name.as_ptr(), 0, 4) }, 0);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o660)).unwrap();
        assert!(read_override_key(&path, &[4,101]).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(read_override_key(&path, &[4,101]).is_err());
    }

    #[test]
    fn override_parent_and_ancestors_cannot_belong_to_or_be_writable_by_the_api() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("legacy"); fs::create_dir(&parent).unwrap();
        let key = parent.join("key"); fs::write(&key, "a".repeat(64)).unwrap();
        let name = CString::new(parent.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::chown(name.as_ptr(),501,501) }, 0);
        assert!(read_override_key(&key, &[4,101]).is_ok());
        assert_eq!(unsafe { libc::chown(name.as_ptr(),10001,501) }, 0);
        assert!(read_override_key(&key, &[4,101]).is_err());
        assert_eq!(unsafe { libc::chown(name.as_ptr(),501,10001) }, 0);
        assert!(read_override_key(&key, &[4,101]).is_err());
        assert_eq!(unsafe { libc::chown(name.as_ptr(),501,4) }, 0);
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o770)).unwrap();
        assert!(read_override_key(&key, &[4,101]).is_err());
        assert_eq!(unsafe { libc::chown(name.as_ptr(),501,501) }, 0);
        for mode in [0o777,0o1777] {
            fs::set_permissions(&parent, fs::Permissions::from_mode(mode)).unwrap();
            assert!(read_override_key(&key, &[4,101]).is_err());
        }
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o777)).unwrap();
        assert!(read_override_key(&key, &[4,101]).is_err());
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o1777)).unwrap();
        assert!(read_override_key(&key, &[4,101]).is_ok());
    }

    #[test]
    fn legacy_override_still_refuses_symlinks_invalid_and_oversized_keys() {
        let temp = tempfile::tempdir().unwrap();
        let key = temp.path().join("key"); fs::write(&key, "a".repeat(64)).unwrap();
        let linked = temp.path().join("linked"); symlink(&key, &linked).unwrap();
        assert!(read_override_key(&linked, &[4,101]).is_err());
        let parent_link = temp.path().join("parent-link"); symlink(temp.path(), &parent_link).unwrap();
        assert!(read_override_key(&parent_link.join("key"), &[4,101]).is_ok());
        fs::write(&key, "z".repeat(64)).unwrap();
        assert!(read_override_key(&key, &[4,101]).is_err());
        fs::write(&key, "a".repeat(4097)).unwrap();
        assert!(read_override_key(&key, &[4,101]).is_err());
        assert_eq!(fs::read(&key).unwrap().len(), 4097);
    }

    #[test]
    fn writable_nonsticky_ancestor_cannot_replace_a_protected_handle_parent() {
        let temp = tempfile::tempdir().unwrap();
        let writable = temp.path().join("writable");
        fs::create_dir(&writable).unwrap();
        let protected = writable.join("protected");
        fs::create_dir(&protected).unwrap();
        fs::set_permissions(&protected, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&writable, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(open_auth_log(&protected.join("auth.log"), &[4,101]).is_err());
        assert!(!protected.join("auth.log").exists());
    }

    #[test]
    fn atomic_write_refuses_foreign_links_and_never_follows_a_replaced_parent() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("original");
        fs::write(&original, b"original").unwrap();
        let destination = temp.path().join("destination");
        symlink(&original, &destination).unwrap();
        assert!(atomic_write(&destination, b"changed", 0, 0o600).is_err());
        fs::remove_file(&destination).unwrap();
        fs::hard_link(&original, &destination).unwrap();
        assert!(atomic_write(&destination, b"changed", 0, 0o600).is_err());
        assert_eq!(fs::read(original).unwrap(), b"original");
        let linked = temp.path().join("linked");
        symlink(temp.path(), &linked).unwrap();
        assert!(atomic_write(&linked.join("other"), b"changed", 0, 0o600).is_err());
        assert!(!temp.path().join("other").exists());
    }

    #[test]
    fn receipt_rejects_world_readability_links_special_files_and_oversize() {
        let temp = tempfile::tempdir_in("/run").unwrap();
        ensure_directory(temp.path(),0,10001,0o750).unwrap();
        let path = temp.path().join("receipt");
        atomic_write(&path, b"{}", 10001, 0o440).unwrap();
        assert!(read_private_json::<serde_json::Value>(&path).is_ok());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_private_json::<serde_json::Value>(&path).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o440)).unwrap();
        let linked = temp.path().join("linked");
        fs::hard_link(&path, &linked).unwrap();
        assert!(read_private_json::<serde_json::Value>(&path).is_err());
        fs::remove_file(&linked).unwrap();
        let symlinked=temp.path().join("symlinked");symlink(&path,&symlinked).unwrap();
        assert!(read_private_json::<serde_json::Value>(&symlinked).is_err());
        let fifo=temp.path().join("fifo");let name=CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe{libc::mkfifo(name.as_ptr(),0o440)},0);
        assert!(read_private_json::<serde_json::Value>(&fifo).is_err());
        let parent_link=temp.path().join("parent-link");symlink(temp.path(),&parent_link).unwrap();
        assert!(read_private_json::<serde_json::Value>(&parent_link.join("receipt")).is_err());
        fs::write(&path, vec![b' ';65537]).unwrap();
        assert!(read_private_json::<serde_json::Value>(&path).is_err());
    }

    #[test]
    fn runtime_receipt_requires_exact_group_parent_mode_and_four_kib_cap() {
        let temp=tempfile::tempdir_in("/run").unwrap();
        ensure_directory(temp.path(),0,10001,0o750).unwrap();
        let path=temp.path().join("receipt.json");
        atomic_write(&path,b"{}",10001,0o440).unwrap();
        assert!(read_private_json::<serde_json::Value>(&path).is_ok());
        for mode in [0o600,0o444] {
            fs::set_permissions(&path,fs::Permissions::from_mode(mode)).unwrap();
            assert!(read_private_json::<serde_json::Value>(&path).is_err());
        }
        atomic_write(&path,b"{}",0,0o440).unwrap();assert!(read_private_json::<serde_json::Value>(&path).is_err());
        atomic_write(&path,b"{}",10001,0o440).unwrap();
        for mode in [0o700,0o755] {
            fs::set_permissions(temp.path(),fs::Permissions::from_mode(mode)).unwrap();
            assert!(read_private_json::<serde_json::Value>(&path).is_err());
        }
        fs::set_permissions(temp.path(),fs::Permissions::from_mode(0o750)).unwrap();
        let boundary=format!("{}{}"," ".repeat(4094),"{}");
        atomic_write(&path,boundary.as_bytes(),10001,0o440).unwrap();
        assert!(read_private_json::<serde_json::Value>(&path).is_ok());
        atomic_write(&path,format!(" {boundary}").as_bytes(),10001,0o440).unwrap();
        assert!(read_private_json::<serde_json::Value>(&path).is_err());
    }

}
