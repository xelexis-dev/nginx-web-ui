#[path="../src/db_upgrade/files.rs"] mod files;
#[test]
fn cancellation_stops_hashing_and_copying_before_destination_mutation() {
    let root=tempfile::tempdir().unwrap();let source=root.path().join("source");let target=root.path().join("target");std::fs::create_dir(&source).unwrap();std::fs::write(source.join("keep"),b"data").unwrap();
    files::CANCELLED.store(true,std::sync::atomic::Ordering::Relaxed);
    assert_eq!(files::snapshot(&source).unwrap_err().kind(),std::io::ErrorKind::Interrupted);
    assert_eq!(files::copy_tree(&source,&target).unwrap_err().kind(),std::io::ErrorKind::Interrupted);assert!(!target.exists());
    files::CANCELLED.store(false,std::sync::atomic::Ordering::Relaxed);
    files::copy_tree(&source,&target).unwrap();assert_eq!(files::snapshot(&source).unwrap(),files::snapshot(&target).unwrap());
}
