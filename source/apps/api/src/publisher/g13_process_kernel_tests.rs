#![cfg(unix)]
use crate::{config::RuntimeConfig,publisher::process::NginxProcess};






#[tokio::test]
async fn process_missing_binary_is_a_validation_result() {
    let result = NginxProcess::new("/does-not-exist/nginx").validate().await;
    assert!(!result.ok);
    assert!(!result.output.is_empty());
}
