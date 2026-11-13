#![cfg(unix)]
use crate::{config::RuntimeConfig,state::AppState,publisher::bootstrap};
use std::{os::unix::fs::PermissionsExt,path::Path};
use tempfile::TempDir;
use serde_json::json;


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



fn layout(dir: &Path) -> Vec<(String, String)> {
    let conf = dir.join("conf");
    let stream = dir.join("stream");
    let bin = dir.join("nginx");
    std::fs::create_dir_all(&conf).unwrap();
    std::fs::create_dir_all(&stream).unwrap();
    std::fs::write(
        dir.join("nginx.conf"),
        "listen 80;\nlisten 443;\nlisten 8080;\n",
    )
    .unwrap();
    std::fs::write(
        &bin,
        format!(
            r#"#!/bin/sh
[ "$1" = -t ] || exit 0
for f in '{conf}'/*.conf '{stream}'/*.conf; do
 [ -f "$f" ] || continue
 n=$(grep -n -E 'rejected\.example|BAD' "$f" | head -1 | cut -d: -f1)
 if [ -n "$n" ]; then echo "nginx: [emerg] unknown directive BAD in $f:$n"; exit 1; fi
done
exit 0
"#,
            conf = conf.display(),
            stream = stream.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    vec![
        (
            "MIGRATIONS_DIR",
            Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../packages/db/drizzle"
            ))
            .to_path_buf(),
        ),
        ("NGINX_BIN", bin),
        ("NGINX_CONF_D", conf),
        ("NGINX_STREAM_D", stream),
        ("CERTS_DIR", dir.join("certs")),
        ("NGINX_CONF", dir.join("nginx.conf")),
        ("NGINX_GUARD_QUARANTINE", dir.join("quarantine")),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.display().to_string()))
    .chain([("API_DB_POOL_SIZE".into(), "1".into())])
    .collect()
}

#[tokio::test]
async fn guard_reserves_api_web_database_and_preserves_udp_and_custom_files() {
    use crate::publisher::bootstrap;
    let temp = TempDir::new().unwrap();
    let s = state(temp.path(), "exit 0");
    std::fs::create_dir_all(&s.config.nginx_conf_dir).unwrap();
    std::fs::create_dir_all(&s.config.nginx_stream_dir).unwrap();
    for (name, content) in [
        ("server-api.conf", "server {\n listen 3001;\n}"),
        ("server-ok.conf", "server {\n listen 8443 ssl;\n}"),
        ("custom.conf", "listen 3001;"),
    ] {
        std::fs::write(s.config.nginx_conf_dir.join(name), content).unwrap();
    }
    for (name, content) in [
        ("stream-api.conf", "listen 3001;"),
        ("stream-http.conf", "listen 8443;"),
        ("stream-udp.conf", "listen 3001 udp;"),
    ] {
        std::fs::write(s.config.nginx_stream_dir.join(name), content).unwrap();
    }
    let base = temp.path().join("nginx.conf");
    std::fs::write(&base, "listen 80;\nlisten 443;\nlisten 8080;\n").unwrap();
    let report = bootstrap::guard_before_start(&s.config, &base, &temp.path().join("quarantine"))
        .await
        .unwrap();
    assert_eq!(report.reserved_files, 3);
    assert!(s.config.nginx_conf_dir.join("custom.conf").exists());
    assert!(s.config.nginx_conf_dir.join("server-ok.conf").exists());
    assert!(s.config.nginx_stream_dir.join("stream-udp.conf").exists());
    assert!(!s.config.nginx_stream_dir.join("stream-http.conf").exists());
}

#[tokio::test]
async fn invalid_guard_restores_unrelated_failures_without_reviving_reserved_files() {
    use crate::publisher::bootstrap;
    let temp = TempDir::new().unwrap();
    let s = state(temp.path(), "echo base-error; exit 1");
    std::fs::create_dir_all(&s.config.nginx_conf_dir).unwrap();
    std::fs::write(
        s.config.nginx_conf_dir.join("server-reserved.conf"),
        "listen 3001;",
    )
    .unwrap();
    std::fs::write(
        s.config.nginx_conf_dir.join("server-safe.conf"),
        "listen 8081;",
    )
    .unwrap();
    std::fs::write(s.config.nginx_conf_dir.join("custom.conf"), "broken custom").unwrap();
    let base = temp.path().join("nginx.conf");
    std::fs::write(&base, "listen 8080;").unwrap();
    let report = bootstrap::guard_before_start(&s.config, &base, &temp.path().join("quarantine"))
        .await
        .unwrap();
    assert_eq!(report.reserved_files, 1);
    assert_eq!(report.invalid_files, 0);
    assert!(
        !s.config
            .nginx_conf_dir
            .join("server-reserved.conf")
            .exists()
    );
    assert!(s.config.nginx_conf_dir.join("server-safe.conf").exists());
    assert_eq!(
        std::fs::read_to_string(s.config.nginx_conf_dir.join("custom.conf")).unwrap(),
        "broken custom"
    );
}

#[tokio::test]
async fn guard_quarantines_only_the_rejected_file_and_cuts_only_the_bad_upstream_without_db() {
    let temp = TempDir::new().unwrap();
    let env = layout(temp.path());
    let mut config = RuntimeConfig::read(|key| {
        env.iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .or_else(|| match key {
                "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/test".into()),
                "AUTH_SECRET" => Some("test-secret".into()),
                _ => None,
            })
    })
    .unwrap();
    config.target_dns.descriptor_path = temp.path().join("target-dns.json");
    std::fs::write(
        config.nginx_conf_dir.join("server-good.conf"),
        "server { listen 8081; }",
    )
    .unwrap();
    std::fs::write(
        config.nginx_conf_dir.join("server-bad.conf"),
        "server { listen 8082; BAD; }",
    )
    .unwrap();
    std::fs::write(
        config.nginx_conf_dir.join("upstreams.conf"),
        "upstream healthy { server 192.0.2.1:80; }\nupstream broken {\n BAD;\n}\n",
    )
    .unwrap();
    let r = bootstrap::guard_managed_files(
        &config,
        &config.nginx_main_conf,
        &config.nginx_quarantine_dir,
    )
    .await
    .unwrap();
    let value = serde_json::to_value(r).unwrap();
    assert!(config.nginx_conf_dir.join("server-good.conf").exists());
    assert!(!config.nginx_conf_dir.join("server-bad.conf").exists());
    let text = std::fs::read_to_string(config.nginx_conf_dir.join("upstreams.conf")).unwrap();
    assert!(text.contains("upstream healthy"));
    assert!(!text.contains("upstream broken"));
    assert_eq!(value["rejectedFiles"], json!(["server-bad.conf"]));
    assert_eq!(value["rejectedUpstreams"], json!(["broken"]));
}
