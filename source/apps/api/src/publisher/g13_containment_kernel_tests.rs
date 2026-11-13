#![cfg(unix)]
use crate::{config::RuntimeConfig,state::AppState,publisher::apply,control::model::RenderedConfig};
use std::{collections::BTreeMap,os::unix::fs::PermissionsExt,path::Path};
use tempfile::TempDir;


fn state(dir: &Path, extra: &str) -> AppState {
    let bin = dir.join("nginx");
    let conf = dir.join("conf");
    let stream = dir.join("stream");
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$*" >> '{log}'
[ "$1" = -t ] || exit 0
{extra}
for f in '{conf}'/*.conf '{stream}'/*.conf; do
  [ -f "$f" ] || continue
  n=$(grep -n BAD "$f" | head -1 | cut -d: -f1)
  if [ -n "$n" ]; then
    echo fail >> '{failures}'
    echo "nginx: [emerg] unknown directive BAD in $f:$n"
    exit 1
  fi
done
if grep -q 'proxy_pass http://broken_group' '{conf}'/server-*.conf 2>/dev/null && ! grep -q 'upstream broken_group' '{conf}/upstreams.conf'; then
  echo 'nginx: [emerg] host not found in upstream "broken_group" in {conf}/server-dependent.conf:1'
  exit 1
fi
exit 0
"#,
        log = dir.join("calls").display(),
        failures = dir.join("failures").display(),
        conf = conf.display(),
        stream = stream.display()
    );
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut config = RuntimeConfig::read(|key| {
        Some(match key {
            "DATABASE_URL" => "postgres://test:test@127.0.0.1:1/test".into(),
            "AUTH_SECRET" => "test-secret".into(),
            "DATA_KEY_FILE" => dir.join("key").display().to_string(),
            "NGINX_BIN" => bin.display().to_string(),
            "NGINX_CONF_D" => conf.display().to_string(),
            "NGINX_STREAM_D" => stream.display().to_string(),
            "CERTS_DIR" => dir.join("certs").display().to_string(),
            "NGINX_CONF" => dir.join("nginx.conf").display().to_string(),
            "NGINX_GUARD_QUARANTINE" => dir.join("quarantine").display().to_string(),
            _ => return None,
        })
    })
    .unwrap();
    config.target_dns.descriptor_path = dir.join("target-dns.json");
    std::fs::write(&config.data_key_file,"A5".repeat(32)).unwrap();let mut state=AppState::new(config).unwrap();
    crate::publisher::test_helpers::nginx::witness(&mut state);
    state
}

fn desired(conf: &[(&str, &str)], stream: &[(&str, &str)]) -> RenderedConfig {
    RenderedConfig {
        conf: conf
            .iter()
            .map(|(n, b)| (n.to_string(), b.as_bytes().to_vec()))
            .collect(),
        stream: stream
            .iter()
            .map(|(n, b)| (n.to_string(), b.as_bytes().to_vec()))
            .collect(),
    }
}

fn write_old(s: &AppState, name: &str, text: &str) {
    std::fs::create_dir_all(&s.config.nginx_conf_dir).unwrap();
    std::fs::write(s.config.nginx_conf_dir.join(name), text).unwrap();
}

#[tokio::test]
async fn rejected_change_keeps_previous_file_while_the_healthy_change_applies() {
    let t = TempDir::new().unwrap();
    let s = state(t.path(), "");
    write_old(&s, "server-bad.conf", "server { listen 8082; } # PREVIOUS");
    let r = apply::apply_rendered(
        &s,
        &desired(
            &[
                ("server-good.conf", "server { listen 8081; } # NEW"),
                ("server-bad.conf", "server { BAD; }"),
            ],
            &[],
        ),
        &[],
    )
    .await
    .unwrap();
    assert!(r.ok, "{}", r.output);
    assert!(
        std::fs::read_to_string(s.config.nginx_conf_dir.join("server-bad.conf"))
            .unwrap()
            .contains("PREVIOUS")
    );
    assert!(
        std::fs::read_to_string(s.config.nginx_conf_dir.join("server-good.conf"))
            .unwrap()
            .contains("NEW")
    );
}

#[tokio::test]
async fn rejected_new_stream_leaves_healthy_stream_and_http_files_live() {
    let t = TempDir::new().unwrap();
    let s = state(t.path(), "");
    let r = apply::apply_rendered(
        &s,
        &desired(
            &[("server-good.conf", "server { listen 8081; }")],
            &[
                ("stream-good.conf", "server { listen 9001; }"),
                ("stream-bad.conf", "server { BAD; }"),
            ],
        ),
        &[],
    )
    .await
    .unwrap();
    assert!(r.ok, "{}", r.output);
    assert!(!s.config.nginx_stream_dir.join("stream-bad.conf").exists());
    assert!(s.config.nginx_stream_dir.join("stream-good.conf").exists());
    assert!(s.config.nginx_conf_dir.join("server-good.conf").exists());
}

#[tokio::test]
async fn rejected_upstream_then_its_dependent_server_drop_without_losing_healthy_units() {
    let t = TempDir::new().unwrap();
    let s = state(t.path(), "");
    let r=apply::apply_rendered(&s,&desired(&[("upstreams.conf","upstream healthy { server 192.0.2.1:80; }\nupstream broken_group {\n BAD;\n}\n"),("server-dependent.conf","server { proxy_pass http://broken_group; }"),("server-good.conf","server { listen 8081; }")],&[]),&[]).await.unwrap();
    assert!(r.ok, "{}", r.output);
    assert!(
        !s.config
            .nginx_conf_dir
            .join("server-dependent.conf")
            .exists()
    );
    let upstream = std::fs::read_to_string(s.config.nginx_conf_dir.join("upstreams.conf")).unwrap();
    assert!(upstream.contains("upstream healthy"));
    assert!(!upstream.contains("broken_group"));
    assert!(s.config.nginx_conf_dir.join("server-good.conf").exists());
}

#[tokio::test]
async fn keep_previous_with_a_new_tcp_stream_port_conflict_drops_instead() {
    let t = TempDir::new().unwrap();
    let s = state(t.path(), "");
    write_old(&s, "server-bad.conf", "server { listen 9000; } # PREVIOUS");
    let r = apply::apply_rendered(
        &s,
        &desired(
            &[
                ("server-good.conf", "server { listen 8081; }"),
                ("server-bad.conf", "server { BAD; }"),
            ],
            &[("stream-new.conf", "server { listen 9000; }")],
        ),
        &[],
    )
    .await
    .unwrap();
    assert!(r.ok, "{}", r.output);
    assert!(!s.config.nginx_conf_dir.join("server-bad.conf").exists());
    assert!(s.config.nginx_stream_dir.join("stream-new.conf").exists());
}

#[tokio::test]
async fn ten_extra_rounds_bound_a_burst_and_restore_the_original_backup() {
    let t = TempDir::new().unwrap();
    let s = state(t.path(), "");
    write_old(&s, "server-old.conf", "LAST GOOD");
    let conf: BTreeMap<_, _> = (0..12)
        .map(|i| {
            (
                format!("server-bad{i:02}.conf"),
                b"server { BAD; }".to_vec(),
            )
        })
        .collect();
    let r = apply::apply_rendered(
        &s,
        &RenderedConfig {
            conf,
            stream: BTreeMap::new(),
        },
        &[],
    )
    .await
    .unwrap();
    assert!(!r.ok);
    assert_eq!(
        std::fs::read_to_string(t.path().join("failures"))
            .unwrap()
            .lines()
            .count(),
        11
    );
    assert_eq!(
        std::fs::read_to_string(s.config.nginx_conf_dir.join("server-old.conf")).unwrap(),
        "LAST GOOD"
    );
}

#[tokio::test]
async fn naming_a_file_already_dropped_stops_without_looping_or_destroying_backup() {
    let t = TempDir::new().unwrap();
    let path = t.path().join("conf/server-bad.conf");
    let s = state(
        t.path(),
        &format!(
            "echo 'nginx: [emerg] invalid in {}:1'; exit 1",
            path.display()
        ),
    );
    write_old(&s, "server-old.conf", "LAST GOOD");
    let r = apply::apply_rendered(
        &s,
        &desired(&[("server-bad.conf", "server { BAD; }")], &[]),
        &[],
    )
    .await
    .unwrap();
    assert!(!r.ok);
    assert_eq!(
        std::fs::read_to_string(t.path().join("calls"))
            .unwrap()
            .lines()
            .filter(|l| *l == "-t")
            .count(),
        3
    );
    assert_eq!(
        std::fs::read_to_string(s.config.nginx_conf_dir.join("server-old.conf")).unwrap(),
        "LAST GOOD"
    );
}

#[tokio::test]
async fn a_bad_certificate_touches_all_and_only_servers_with_its_exact_path() {
    let t = TempDir::new().unwrap();
    let key = t
        .path()
        .join("certs/51f19cb6-336c-4fb0-a3cb-5cfe55ec98f0/privkey.pem");
    let extra = format!(
        r#"for f in '{conf}'/server-*.conf; do
 [ -f "$f" ] || continue
 if grep -Fq 'ssl_certificate_key {key};' "$f"; then echo 'nginx: [emerg] SSL_CTX_use_PrivateKey("{key}") failed (SSL: key values mismatch)'; exit 1; fi
done"#,
        conf = t.path().join("conf").display(),
        key = key.display()
    );
    let s = state(t.path(), &extra);
    let bad = format!(
        "server {{ listen 8082; ssl_certificate_key {}; }}",
        key.display()
    );
    let wanted = desired(
        &[
            ("server-first.conf", &bad),
            ("server-second.conf", &bad),
            ("server-good.conf", "server { listen 8081; }"),
        ],
        &[],
    );
    let r = apply::apply_rendered(&s, &wanted, &[]).await.unwrap();
    assert!(r.ok, "{}", r.output);
    assert!(!s.config.nginx_conf_dir.join("server-first.conf").exists());
    assert!(!s.config.nginx_conf_dir.join("server-second.conf").exists());
    assert!(s.config.nginx_conf_dir.join("server-good.conf").exists());
}

#[tokio::test]
async fn a_nonmanaged_global_error_restores_the_backup_without_a_containment_retry() {
    let t = TempDir::new().unwrap();
    let s = state(
        t.path(),
        &format!(
            "echo 'nginx: [emerg] invalid in {}:1'; exit 1",
            t.path().join("nginx.conf").display()
        ),
    );
    write_old(&s, "server-old.conf", "LAST GOOD");
    let r = apply::apply_rendered(
        &s,
        &desired(&[("server-good.conf", "server { listen 8081; }")], &[]),
        &[],
    )
    .await
    .unwrap();
    assert!(!r.ok);
    assert!(r.rolled_back_to.is_none());
    assert_eq!(
        std::fs::read_to_string(s.config.nginx_conf_dir.join("server-old.conf")).unwrap(),
        "LAST GOOD"
    );
    assert!(!s.config.nginx_conf_dir.join("server-good.conf").exists());
    assert_eq!(
        std::fs::read_to_string(t.path().join("calls")).unwrap(),
        "-t\n-t\n"
    );
}
