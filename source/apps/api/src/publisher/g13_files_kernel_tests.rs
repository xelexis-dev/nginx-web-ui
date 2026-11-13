#![cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;


#[tokio::test]
async fn rewriting_identical_content_with_the_same_mode_leaves_the_file_alone() {
    // Each rewrite is a temporary file, an fsync and a rename; an apply rewrote every managed
    // file although usually one changed. Unchanged files are now left alone, but only when the
    // mode matches too, so a private key's 0600 is restored if it ever drifted.
    use crate::publisher::files::atomic_write;
    use std::os::unix::fs::MetadataExt;
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("server-x.conf");
    atomic_write(&path, b"server {}\n", 0o644).await.unwrap();
    let first = std::fs::metadata(&path).unwrap().ino();
    atomic_write(&path, b"server {}\n", 0o644).await.unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().ino(),
        first,
        "identical content was rewritten"
    );
    atomic_write(&path, b"server { listen 81; }\n", 0o644)
        .await
        .unwrap();
    assert_ne!(std::fs::metadata(&path).unwrap().ino(), first);
    assert_eq!(std::fs::read(&path).unwrap(), b"server { listen 81; }\n");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
    atomic_write(&path, b"server { listen 81; }\n", 0o644)
        .await
        .unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().mode() & 0o777,
        0o644,
        "a drifted mode is corrected"
    );
}
