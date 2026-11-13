#![cfg(unix)]
use crate::{config::RuntimeConfig,state::AppState,publisher::apply,control::model::RenderedConfig};
use std::{collections::BTreeMap,os::unix::fs::PermissionsExt,time::Duration};


use axum::{Router,extract::State,routing::get};

#[tokio::test]
async fn a_successful_signal_with_only_the_old_generation_never_reports_apply_or_restore_success() {
    let temp = tempfile::tempdir().unwrap();
    let conf = temp.path().join("conf");
    std::fs::create_dir(&conf).unwrap();
    let old = uuid::Uuid::new_v4().to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = Router::new()
        .route(
            "/nginx_status",
            get(|State(token): State<String>| async move {
                (
                    [
                        ("x-nwui-config-generation", token),
                        ("cache-control", "no-store".into()),
                    ],
                    "Active connections: 1\n",
                )
            }),
        )
        .with_state(old.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let bin = temp.path().join("nginx");
    let calls = temp.path().join("calls");
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
            calls.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    let pid = temp.path().join("nginx.pid");
    std::fs::write(&pid, std::process::id().to_string()).unwrap();
    let main = temp.path().join("nginx.conf");
    std::fs::write(&main,format!("pid {}; events {{}} http {{ server {{ listen 127.0.0.1:{port}; location = /nginx_status {{ stub_status; allow 127.0.0.3; deny all; include {}/.nwui-generation; }} }} }}\n",pid.display(),conf.display())).unwrap();
    std::fs::write(
        conf.join(".nwui-generation"),
        format!("add_header X-NWUI-Config-Generation \"{old}\";\n"),
    )
    .unwrap();
    let mut config = RuntimeConfig::read(|key| {
        Some(match key {
            "DATABASE_URL" => "postgres://unused@127.0.0.1:1/test".into(),
            "AUTH_SECRET" => "test-secret".into(),
            "DATA_KEY_FILE" => temp.path().join("key").display().to_string(),
            "NGINX_BIN" => bin.display().to_string(),
            "NGINX_CONF" => main.display().to_string(),
            "NGINX_CONF_D" => conf.display().to_string(),
            "NGINX_STREAM_D" => temp.path().join("stream").display().to_string(),
            "CERTS_DIR" => temp.path().join("certs").display().to_string(),
            "NGINX_STATUS_URL" => format!("http://127.0.0.1:{port}/nginx_status"),
            _ => return None,
        })
    })
    .unwrap();
    config.target_dns.descriptor_path = temp.path().join("target-dns.json");
    std::fs::write(&config.data_key_file,"A5".repeat(32)).unwrap();let state = AppState::new(config).unwrap();
    let desired = RenderedConfig {
        conf: BTreeMap::from([(
            "server-new.conf".into(),
            b"server { listen 18443; return 200 NEW; }".to_vec(),
        )]),
        stream: BTreeMap::new(),
    };
    let result = tokio::time::timeout(
        Duration::from_secs(35),
        apply::apply_rendered(&state, &desired, &[]),
    )
    .await
    .unwrap()
    .unwrap();
    server.abort();
    assert!(
        !result.ok,
        "signal delivery is not generation acknowledgement: {}",
        result.output
    );
    assert!(
        result.rolled_back_to.is_none(),
        "old token is not restoration acknowledgement"
    );
    assert!(conf.join(".nwui-apply-uncertain").exists());
    assert!(
        std::fs::read_to_string(calls)
            .unwrap()
            .lines()
            .filter(|l| *l == "-s reload")
            .count()
            >= 2,
        "candidate and recovery must both be attempted"
    );
}

#[tokio::test]
async fn a_confirmed_candidate_clears_uncertainty_and_retains_unmanaged_files() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("nginx");
    std::fs::write(&bin, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = RuntimeConfig::read(|key| {
        Some(match key {
            "DATABASE_URL" => "postgres://unused@127.0.0.1:1/test".into(),
            "AUTH_SECRET" => "test-secret".into(),
            "DATA_KEY_FILE" => temp.path().join("key").display().to_string(),
            "NGINX_BIN" => bin.display().to_string(),
            "NGINX_CONF_D" => temp.path().join("conf").display().to_string(),
            "NGINX_STREAM_D" => temp.path().join("stream").display().to_string(),
            "CERTS_DIR" => temp.path().join("certs").display().to_string(),
            _ => return None,
        })
    })
    .unwrap();
    std::fs::write(&config.data_key_file,"A5".repeat(32)).unwrap();let mut state = AppState::new(config).unwrap();
    crate::publisher::test_helpers::nginx::witness(&mut state);
    std::fs::write(state.config.nginx_conf_dir.join("user.conf"), "KEEP").unwrap();
    let desired = RenderedConfig {
        conf: BTreeMap::from([(
            "server-new.conf".into(),
            b"server { listen 18443; }".to_vec(),
        )]),
        stream: BTreeMap::new(),
    };
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        apply::apply_rendered(&state, &desired, &[]),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(result.ok, "{}", result.output);
    assert!(result.rolled_back_to.is_none());
    assert!(
        !state
            .config
            .nginx_conf_dir
            .join(".nwui-apply-uncertain")
            .exists()
    );
    assert_eq!(
        std::fs::read_to_string(state.config.nginx_conf_dir.join("user.conf")).unwrap(),
        "KEEP"
    );
}
