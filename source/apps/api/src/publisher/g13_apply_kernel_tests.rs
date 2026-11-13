#![cfg(unix)]
use crate::{config::RuntimeConfig,state::AppState,publisher::{apply,files::{CertificateMaterial,RenderedConfig}}};
use std::{collections::BTreeMap,os::unix::fs::PermissionsExt,path::Path};
use tempfile::TempDir;


fn state(dir: &Path, body: &str) -> AppState {
    let bin = dir.join("nginx-fake");
    std::fs::write(&bin, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut config = RuntimeConfig::read(|key| {
        Some(match key {
            "DATABASE_URL" => "postgres://test:test@127.0.0.1:1/test".into(),
            "AUTH_SECRET" => "test-secret".into(),
            "DATA_KEY_FILE" => dir.join("key").display().to_string(),
            "NGINX_BIN" => bin.display().to_string(),
            "NGINX_CONF_D" => dir.join("conf").display().to_string(),
            "NGINX_STREAM_D" => dir.join("stream").display().to_string(),
            "CERTS_DIR" => dir.join("certs").display().to_string(),
            _ => return None,
        })
    })
    .unwrap();
    config.target_dns.descriptor_path = dir.join("target-dns.json");
    std::fs::write(&config.data_key_file,"A5".repeat(32)).unwrap();let mut state=AppState::new(config).unwrap();
    crate::publisher::test_helpers::nginx::witness(&mut state);
    state
}

fn rendered() -> RenderedConfig {
    RenderedConfig {
        conf: BTreeMap::from([
            ("upstreams.conf".into(), b"upstream new {}\n".to_vec()),
            (
                "server-new.conf".into(),
                b"server { listen 8081; }\n".to_vec(),
            ),
        ]),
        stream: BTreeMap::from([(
            "stream-new.conf".into(),
            b"server { listen 9000; }\n".to_vec(),
        )]),
    }
}

async fn w11_changed_target(phase:&str) {
    let temp = TempDir::new().unwrap();
    let bad = temp.path().join("conf/server-bad.conf");
    let once = temp.path().join("changed");
    let s = state(temp.path(), &format!(
        "if [ \"$*\" = '{phase}' ] && [ ! -e '{once}' ]; then touch '{once}'; echo 'server {{ listen 18090; location / {{ proxy_pass http://127.0.0.1:3001; }} }}' > '{bad}'; fi\nexit 0",
        once=once.display(),bad=bad.display()
    ));
    let mut s=s;
    std::sync::Arc::get_mut(&mut s.config).unwrap().in_container=true;
    let mut input=rendered();
    input.conf.insert("server-bad.conf".into(),b"server { listen 18090; location / { proxy_pass http://192.0.2.1:3001; } }".to_vec());
    let result=apply::apply_rendered(&s,&input,&[]).await.unwrap();
    assert!(result.ok,"{phase}: {}",result.output);
    assert!(!bad.exists(),"{phase}: changed target must be isolated");
    assert!(s.config.nginx_conf_dir.join("server-new.conf").exists());
    assert!(result.output.contains("server-bad.conf"),"isolation must be reported: {}",result.output);
}

#[tokio::test]
async fn deleting_certificate_removes_crash_temporary_keys_but_preserves_user_files() {
    let temp = TempDir::new().unwrap();
    let s = state(temp.path(), "exit 0");
    let directory = s.config.certs_dir.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&directory).unwrap();
    let stale = directory.join(format!(".nwui-{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&stale, b"private key left by killed writer").unwrap();
    std::fs::write(directory.join("user-note"), b"keep").unwrap();
    apply::apply_rendered(&s, &rendered(), &[]).await.unwrap();
    assert!(
        !stale.exists(),
        "stale private key must not survive certificate removal"
    );
    assert!(directory.join("user-note").exists());
}

#[tokio::test]
async fn w11_checks_targets_after_validation() {w11_changed_target("-t").await;}

#[tokio::test]
async fn w11_checks_targets_after_reload() {w11_changed_target("-s reload").await;}

#[tokio::test]
async fn successful_apply_writes_managed_files_and_preserves_user_files() {
    let temp = TempDir::new().unwrap();
    let s = state(temp.path(), "echo ok; exit 0");
    std::fs::create_dir_all(&s.config.nginx_conf_dir).unwrap();
    std::fs::write(s.config.nginx_conf_dir.join("user.conf"), "user").unwrap();
    std::fs::write(s.config.nginx_conf_dir.join("server-stale.conf"), "old").unwrap();
    let result = apply::apply_rendered(&s, &rendered(), &[]).await.unwrap();
    assert!(result.ok);
    assert_eq!(
        std::fs::read(s.config.nginx_conf_dir.join("server-new.conf")).unwrap(),
        rendered().conf["server-new.conf"]
    );
    assert!(!s.config.nginx_conf_dir.join("server-stale.conf").exists());
    assert_eq!(
        std::fs::read_to_string(s.config.nginx_conf_dir.join("user.conf")).unwrap(),
        "user"
    );
}

#[tokio::test]
async fn validation_failure_restores_files_without_claiming_an_unconfirmed_worker() {
    let temp = TempDir::new().unwrap();
    let log = temp.path().join("calls");
    let s = state(
        temp.path(),
        &format!(
            "echo \"$*\" >> '{}'; echo invalid >&2; exit 1",
            log.display()
        ),
    );
    std::fs::create_dir_all(&s.config.nginx_conf_dir).unwrap();
    std::fs::create_dir_all(&s.config.nginx_stream_dir).unwrap();
    std::fs::write(s.config.nginx_conf_dir.join("server-old.conf"), "before").unwrap();
    std::fs::write(
        s.config.nginx_stream_dir.join("stream-old.conf"),
        "before-stream",
    )
    .unwrap();
    let result = apply::apply_rendered(&s, &rendered(), &[]).await.unwrap();
    assert!(!result.ok);
    assert!(result.rolled_back_to.is_none());
    assert_eq!(
        std::fs::read_to_string(s.config.nginx_conf_dir.join("server-old.conf")).unwrap(),
        "before"
    );
    assert_eq!(
        std::fs::read_to_string(s.config.nginx_stream_dir.join("stream-old.conf")).unwrap(),
        "before-stream"
    );
    assert!(!s.config.nginx_conf_dir.join("server-new.conf").exists());
    assert_eq!(std::fs::read_to_string(log).unwrap(), "-t\n-t\n");
}

#[tokio::test]
async fn certificates_are_decrypted_private_and_restored_on_failed_validation() {
    let temp = TempDir::new().unwrap();
    let s = state(temp.path(), "echo invalid; exit 1");
    let id = "51f19cb6-336c-4fb0-a3cb-5cfe55ec98f0";
    let cert_dir = s.config.certs_dir.join(id);
    std::fs::create_dir_all(&cert_dir).unwrap();
    std::fs::write(cert_dir.join("privkey.pem"), "old key").unwrap();
    std::fs::write(cert_dir.join("fullchain.pem"), "old cert").unwrap();
    let cert = CertificateMaterial {
        id: id.into(),
        cert_pem: "new cert".into(),
        key_pem: s.secrets.seal("new key").unwrap(),
    };
    let result = apply::apply_rendered(&s, &rendered(), &[cert])
        .await
        .unwrap();
    assert!(!result.ok);
    assert_eq!(
        std::fs::read_to_string(cert_dir.join("privkey.pem")).unwrap(),
        "old key"
    );
    assert_eq!(
        std::fs::metadata(cert_dir.join("privkey.pem"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[tokio::test]
async fn partial_write_failure_restores_previously_written_files() {
    let temp = TempDir::new().unwrap();
    let s = state(temp.path(), "exit 0");
    std::fs::create_dir_all(&s.config.nginx_conf_dir).unwrap();
    std::fs::write(s.config.nginx_conf_dir.join("upstreams.conf"), "previous").unwrap();
    let mut bad = rendered();
    bad.stream.insert(
        format!("stream-{}.conf", "x".repeat(300)),
        b"too long".to_vec(),
    );
    let result = apply::apply_rendered(&s, &bad, &[]).await.unwrap();
    assert!(!result.ok);
    assert_eq!(
        std::fs::read_to_string(s.config.nginx_conf_dir.join("upstreams.conf")).unwrap(),
        "previous"
    );
    assert!(!s.config.nginx_conf_dir.join("server-new.conf").exists());
}

#[tokio::test]
async fn reload_failure_restores_and_reloads_previous_configuration() {
    let temp = TempDir::new().unwrap();
    let marker = temp.path().join("first-reload");
    let s = state(
        temp.path(),
        &format!(
            "if [ \"$1\" = '-s' ] && [ ! -f '{}' ]; then touch '{}'; echo reload-failure; exit 7; fi; exit 0",
            marker.display(),
            marker.display()
        ),
    );
    std::fs::create_dir_all(&s.config.nginx_conf_dir).unwrap();
    std::fs::write(s.config.nginx_conf_dir.join("server-old.conf"), "previous").unwrap();
    let result = apply::apply_rendered(&s, &rendered(), &[]).await.unwrap();
    assert!(!result.ok);
    assert!(result.rolled_back_to.is_some());
    assert!(!s.config.nginx_conf_dir.join(".nwui-apply-uncertain").exists());
    assert!(result.output.contains("exit code 7"));
    assert_eq!(
        std::fs::read_to_string(s.config.nginx_conf_dir.join("server-old.conf")).unwrap(),
        "previous"
    );
    assert!(!s.config.nginx_conf_dir.join("server-new.conf").exists());
}

#[tokio::test]
async fn successful_certificate_materialization_opens_sealed_key_with_private_mode() {
    let temp = TempDir::new().unwrap();
    let s = state(temp.path(), "exit 0");
    let id = "c348c518-da7d-4d7f-8bc9-0da2940b65c2";
    let cert = CertificateMaterial {
        id: id.into(),
        cert_pem: "cert".into(),
        key_pem: s.secrets.seal("plain-key").unwrap(),
    };
    assert!(
        apply::apply_rendered(&s, &rendered(), &[cert])
            .await
            .unwrap()
            .ok
    );
    let key = s.config.certs_dir.join(id).join("privkey.pem");
    assert_eq!(std::fs::read_to_string(&key).unwrap(), "plain-key");
    assert_eq!(
        std::fs::metadata(key).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[tokio::test]
async fn apply_mutex_prevents_interleaved_nginx_commands() {
    let temp = TempDir::new().unwrap();
    let log = temp.path().join("calls");
    let s = state(
        temp.path(),
        &format!("echo \"$*\" >> '{}'; sleep 0.05; exit 0", log.display()),
    );
    let a = rendered();
    let b = rendered();
    let (a, b) = tokio::join!(
        apply::apply_rendered(&s, &a, &[]),
        apply::apply_rendered(&s, &b, &[])
    );
    assert!(a.unwrap().ok && b.unwrap().ok);
    assert_eq!(
        std::fs::read_to_string(log).unwrap(),
        "-t\n-t\n-s reload\n-s reload\n-t\n-t\n-s reload\n-s reload\n"
    );
}

#[tokio::test]
async fn dropping_the_request_does_not_abandon_half_applied_files() {
    let temp = TempDir::new().unwrap();
    let log = temp.path().join("calls");
    let s = state(
        temp.path(),
        &format!("echo \"$*\" >> '{}'; sleep 0.15; exit 0", log.display()),
    );
    let running = {
        let s = s.clone();
        tokio::spawn(async move { apply::apply_rendered(&s, &rendered(), &[]).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !log.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    running.abort();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if std::fs::read_to_string(&log).unwrap().contains("-s reload") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    // Wait until the owned task releases its lock before the temp directory drops.
    let _guard = s.runtime.apply_lock.lock().await;
    assert!(s.config.nginx_conf_dir.join("server-new.conf").exists());
}

#[tokio::test]
async fn a_restore_write_failure_keeps_uncertainty_without_claiming_restoration() {
    let temp=TempDir::new().unwrap();let collision=temp.path().join("conf/server-old.conf");
    let s=state(temp.path(),&format!("if [ \"$1\" = -s ]; then mkdir '{}'; exit 7; fi; exit 0",collision.display()));
    std::fs::write(&collision,"OLD").unwrap();
    let result=apply::apply_rendered(&s,&rendered(),&[]).await.unwrap();
    assert!(!result.ok);assert!(result.rolled_back_to.is_none());
    assert!(s.config.nginx_conf_dir.join(".nwui-apply-uncertain").exists());
    assert!(result.output.contains("확인하지 못했습니다"),"{}",result.output);
}

#[tokio::test]
async fn a_restore_signal_failure_keeps_uncertainty_without_accepting_the_old_worker() {
    let temp=TempDir::new().unwrap();let s=state(temp.path(),"if [ \"$1\" = -s ]; then exit 7; fi; exit 0");
    std::fs::write(s.config.nginx_conf_dir.join("server-old.conf"),"OLD").unwrap();
    let result=apply::apply_rendered(&s,&rendered(),&[]).await.unwrap();
    assert!(!result.ok);assert!(result.rolled_back_to.is_none());
    assert!(s.config.nginx_conf_dir.join(".nwui-apply-uncertain").exists());
    assert_eq!(std::fs::read_to_string(s.config.nginx_conf_dir.join("server-old.conf")).unwrap(),"OLD");
}

#[tokio::test]
async fn multihop_reports_cross_server_cycles_without_isolating_them() {
    let temp=TempDir::new().unwrap();
    let mut s=state(temp.path(), "exit 0");
    std::sync::Arc::get_mut(&mut s.config).unwrap().in_container=true;
    let input=RenderedConfig {conf:BTreeMap::from([
        ("server-a.conf".into(),b"server { listen 18090; location /api { proxy_pass http://127.0.0.1:18091; } }".to_vec()),
        ("server-b.conf".into(),b"server { listen 18091; location /auth { proxy_pass http://127.0.0.1:18090; } }".to_vec()),
    ]),stream:BTreeMap::new()};
    let result=apply::apply_rendered(&s,&input,&[]).await.unwrap();
    assert!(result.ok,"{}",result.output);
    assert!(result.output.contains("같은 요청이 순환하면 누구나 nginx 연결을 소진시킬 수 있습니다"));
    for name in input.conf.keys(){assert!(s.config.nginx_conf_dir.join(name).exists());}
}
