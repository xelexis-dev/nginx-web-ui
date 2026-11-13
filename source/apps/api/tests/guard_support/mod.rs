//! Real guard subprocesses cannot override the trust-checked descriptor path.
use std::path::Path;

pub fn assert_no_host_descriptor() {
    assert_no_descriptor_at(Path::new(nwui_api::dns_guard::DESCRIPTOR_PATH));
}

fn assert_no_descriptor_at(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        result => panic!(
            "{} must not exist on the test host: a real descriptor would change what the guard subprocess observes ({result:?})",
            path.display()
        ),
    }
}

#[test]
fn precondition_rejects_a_descriptor_in_a_scratch_root() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("run/nginx-web-ui/target-dns.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    assert_no_descriptor_at(&path);
    std::fs::write(&path, b"{}").unwrap();
    let failure = std::panic::catch_unwind(|| assert_no_descriptor_at(&path)).unwrap_err();
    let message = failure.downcast_ref::<String>().unwrap();
    assert!(message.contains(path.to_str().unwrap()));
    assert!(message.contains("a real descriptor would change what the guard subprocess observes"));
}
