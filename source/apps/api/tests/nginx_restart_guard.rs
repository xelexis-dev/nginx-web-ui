#![cfg(unix)]
//! `nwui-api guard`, run by nginx/run before every nginx start, as the separate process it is:
//! the internal-target recheck, and its exclusion against the API's own file changes.
use nwui_api::{
    config::RuntimeConfig,
};
use std::{
    os::unix::fs::PermissionsExt,
    process::Command,
    time::{Duration, Instant},
};
use tempfile::TempDir;
mod auth_support;
mod g13_support;
mod guard_support;

use auth_support::restart_guard::*;

fn guard(nginx:&Nginx,p:&Paths,wait_secs:&str)->Command {
    guard_support::assert_no_host_descriptor();
    let mut command=Command::new(env!("CARGO_BIN_EXE_nwui-api"));command.arg("guard");
    guard_command(command,nginx,p,wait_secs)
}

/// A restart must not bring back a target that reaches the web app, API or database port,
/// whether it was written as IPv4, IPv6, a hosts-file name or an upstream name, for HTTP and
/// stream alike. The admin UI (8080) and outside hosts that happen to use such a port stay.
#[test]
fn guard_process_quarantines_internal_targets_without_a_database() {
    let temp = TempDir::new().unwrap();
    let p = paths(temp.path());
    let nginx = Nginx::new(temp.path());
    let conf = [
        ("upstreams.conf", "upstream loop6 {\n    server [::1]:3001;\n}\n\nupstream outside {\n    server 10.9.8.7:3001 weight=2;\n}\n"),
        ("server-loop6.conf", "server {\n    listen 80;\n    location / {\n        proxy_pass http://loop6;\n    }\n}\n"),
        ("server-outside-pool.conf", "server {\n    listen 80;\n    location / {\n        proxy_pass http://outside/app;\n    }\n}\n"),
        ("server-v4.conf", "server {\n    listen 80;\n    location / {\n        proxy_pass http://127.0.0.1:3000/;\n        add_header X-Note \"# proxy_pass http://10.0.0.1:80;\";\n    }\n}\n"),
        ("server-hosts-name.conf", "server {\n    listen 80;\n    location / {\n        proxy_pass http://localhost:5432;\n    }\n}\n"),
        ("server-admin.conf", "server {\n    listen 80;\n    location / {\n        proxy_pass http://127.0.0.1:8080;\n    }\n}\n"),
        ("server-outside.conf", "server {\n    listen 80;\n    # proxy_pass http://127.0.0.1:3001;\n    location / {\n        proxy_pass http://10.9.8.7:3001;\n    }\n}\n"),
        ("custom.conf", "server { location / { proxy_pass http://127.0.0.1:3001; } }\n"),
        // The image's own files: not managed, never touched, though they use the runtime names.
        ("00-runtime-upstreams.conf", "upstream nwui_api { server 127.0.0.1:3001; keepalive 16; }\nupstream nwui_web { server 127.0.0.1:3000; keepalive 16; }\n"),
        ("server-runtime-name.conf", "server {\n    listen 80;\n    include /etc/nginx/snippets/acme-challenge.conf;\n    location / {\n        proxy_pass http://NWUI_API;\n    }\n}\n"),
        ("server-acme-include.conf", "server {\n    listen 80;\n    include /etc/nginx/snippets/acme-challenge.conf;\n    location / {\n        proxy_pass http://10.9.8.7:3000;\n    }\n}\n"),
    ];
    for (name, content) in conf {
        std::fs::write(p.conf.join(name), content).unwrap();
    }
    for (name, content) in [
        ("stream-tcp.conf", "server {\n    listen 9000;\n    proxy_pass 127.0.0.1:3001;\n}\n"),
        ("stream-udp.conf", "server {\n    listen 9001 udp;\n    proxy_pass localhost:3000;\n}\n"),
        ("stream-outside.conf", "server {\n    listen 9002;\n    proxy_pass 10.9.8.7:5432;\n}\n"),
    ] {
        std::fs::write(p.stream.join(name), content).unwrap();
    }
    let (ok, stderr) = finish(guard(&nginx, &p, "5").spawn().unwrap());
    assert!(ok, "{stderr}");
    for gone in ["server-loop6.conf", "server-v4.conf", "server-hosts-name.conf", "server-runtime-name.conf"] {
        assert!(!p.conf.join(gone).exists(), "{gone} kept: {stderr}");
    }
    for gone in ["stream-tcp.conf", "stream-udp.conf"] {
        assert!(!p.stream.join(gone).exists(), "{gone} kept: {stderr}");
    }
    for kept in ["server-outside-pool.conf", "server-admin.conf", "server-outside.conf", "custom.conf", "server-acme-include.conf"] {
        assert!(p.conf.join(kept).exists(), "{kept} removed: {stderr}");
    }
    assert_eq!(std::fs::read_to_string(p.conf.join("00-runtime-upstreams.conf")).unwrap(), conf[8].1);
    assert!(p.stream.join("stream-outside.conf").exists());
    assert_eq!(
        std::fs::read_to_string(p.conf.join("upstreams.conf")).unwrap(),
        "\nupstream outside {\n    server 10.9.8.7:3001 weight=2;\n}\n",
        "only the unsafe block leaves upstreams.conf (the renderer's blank separator stays)"
    );
    let recovery: Vec<_> = std::fs::read_dir(&p.quarantine).unwrap().flatten().map(|e| e.path()).collect();
    assert_eq!(recovery.len(), 1);
    assert!(recovery[0].display().to_string().ends_with("-internal-target"));
    assert_eq!(
        std::fs::read_to_string(recovery[0].join("conf.d/upstreams.conf")).unwrap(),
        conf[0].1,
        "the original upstreams.conf is kept for recovery"
    );
    assert!(recovery[0].join("conf.d/server-v4.conf").exists());
    assert!(recovery[0].join("stream.d/stream-udp.conf").exists());
    assert!(stderr.contains("6 internal-target") && stderr.contains("loop6"), "{stderr}");
    assert_eq!(nginx.calls(), ["guard start -t", "guard end -t"], "validated once, after the recheck");
}

/// Contention, guard first: an apply that starts while the guard process works waits for it.

/// Timeout: the guard gives up without touching anything, even with files it would move out,
/// and nginx/run defers startup until exclusive file ownership becomes available.
#[test]
fn guard_that_cannot_get_the_files_changes_nothing() {
    let temp = TempDir::new().unwrap();
    let p = paths(temp.path());
    let nginx = Nginx::new(temp.path());
    std::fs::write(p.conf.join("server-internal.conf"), "proxy_pass http://127.0.0.1:3001;\n").unwrap();
    std::fs::write(p.conf.join("server-reserved.conf"), "listen 3001;\n").unwrap();
    std::fs::write(p.stream.join("stream-internal.conf"), "proxy_pass 127.0.0.1:5432;\n").unwrap();
    let config = RuntimeConfig::read(|key| match key {
        "DATABASE_URL" => Some("postgres://u@127.0.0.1:1/x".into()),
        "AUTH_SECRET" => Some("s".into()),
        "NGINX_CONF_D" => Some(p.conf.display().to_string()),
        _ => None,
    })
    .unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(config.nginx_conf_dir.join(".nwui-nginx-files.lock"))
        .unwrap();
    held.lock().unwrap();
    let before = snapshot(temp.path());
    let started = Instant::now();
    let (ok, stderr) = finish(guard(&nginx, &p, "1").spawn().unwrap());
    assert!(!ok, "the guard must report that it did not run: {stderr}");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(stderr.contains("[nginx-guard] nginx start deferred: storage_busy\n"), "{stderr}");
    assert_eq!(snapshot(temp.path()), before, "a guard that timed out touched files");
    assert!(!p.quarantine.exists());
    assert!(nginx.calls().is_empty(), "nginx -t ran without the lock");
    drop(held);
}

/// Process exit: a guard killed while it holds the files releases them at once. Its nginx child
/// does not inherit the lock.

/// Timeout on the API side: an apply that cannot get the files fails without touching them,
/// and waiting does not hold up this runtime's other tasks.

async fn real_db(nginx: &Nginx, p: &Paths, wait_secs: &str) -> auth_support::TestDb {
    let extra = api_config(nginx, p, wait_secs);
    let pairs: Vec<(&str, &str)> = extra.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let mut db=auth_support::test_db_with(&pairs).await;
    g13_support::witness(&mut db.state);
    db
}

/// Contention, apply first, through its failure path: the apply's commit fails after nginx
/// reloaded, so it restores and reloads the previous files. The guard waits for all of it and
/// sees only the restored files.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn guard_process_waits_for_an_apply_restoring_after_a_failed_commit() {
    let temp = TempDir::new().unwrap();
    let p = paths(temp.path());
    let nginx = Nginx::new(temp.path());
    nginx.sleep("api", 0.4);
    std::fs::write(p.conf.join("server-old.conf"), "server { listen 8082; }\n").unwrap();
    let t = real_db(&nginx, &p, "20").await;
    t.client.batch_execute("CREATE FUNCTION fail_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected commit failure'; END $$; CREATE CONSTRAINT TRIGGER fail_commit AFTER INSERT OR UPDATE ON applied_config DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION fail_commit();").await.unwrap();
    let applying = {
        let state = t.state.clone();
        tokio::spawn(async move { nwui_api::publisher_client::apply_saved(&state,&nwui_api::contracts::Actor{id:None,label:Some("guard-test".into()),ip:None},None).await })
    };
    nginx.wait_for("api start -t").await;
    let child = guard(&nginx, &p, "20").spawn().unwrap();
    let error = applying.await.unwrap().unwrap_err();
    assert_eq!(error.to_string(), "설정 DB 커밋 결과를 확인하지 못했습니다.");
    let (ok, stderr) = finish(child);
    assert!(ok, "{stderr}");
    let calls = nginx.calls();
    assert_eq!(
        calls.iter().filter(|l| l.starts_with("api start")).cloned().collect::<Vec<_>>(),
        ["api start -t", "api start -t", "api start -s reload", "api start -s reload", "api start -T", "api start -t", "api start -s reload", "api start -s reload"],
        "A.15/N1-R trusted include check precedes restore validation: {calls:?}"
    );
    // Retain the original seven-command assertion as well as the complete durable trace.
    let calls: Vec<_> = calls.into_iter().filter(|line| line != "api start -T" && line != "api end -T").collect();
    assert_eq!(
        calls.iter().filter(|l| l.starts_with("api start")).cloned().collect::<Vec<_>>(),
        ["api start -t", "api start -t", "api start -s reload", "api start -s reload", "api start -t", "api start -s reload", "api start -s reload"],
        "apply, then restore after the failed commit: {calls:?}"
    );
    nginx.assert_before("api", "guard");
    assert!(p.conf.join("server-old.conf").exists(), "the previous files were restored");
}


/// Rolling back to a snapshot writes the same files, so it also waits for the guard.
#[tokio::test]
#[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
async fn snapshot_rollback_waits_for_the_guard_process() {
    let temp = TempDir::new().unwrap();
    let p = paths(temp.path());
    let nginx = Nginx::new(temp.path());
    nginx.sleep("guard", 1.0);
    let t = real_db(&nginx, &p, "20").await;
    t.client
        .execute("INSERT INTO config_snapshots(id,label,data) VALUES('77777777-7777-4777-8777-777777777777','empty','{}'::jsonb)", &[])
        .await
        .unwrap();
    let child = guard(&nginx, &p, "20").spawn().unwrap();
    nginx.wait_for("guard start -t").await;
    let actor = nwui_api::contracts::Actor { id: None, label: Some("test".into()), ip: None };
    let restored = nwui_api::publisher_client::rollback_snapshot(&t.state, "77777777-7777-4777-8777-777777777777", &actor).await.unwrap();
    assert!(restored.result.ok, "{}", restored.result.output);
    let (ok, stderr) = finish(child);
    assert!(ok, "{stderr}");
    nginx.assert_before("guard", "api");
}

#[test]
fn fxb2_restart_quarantines_missing_or_empty_certificate_files_without_database() {
    let temp=TempDir::new().unwrap();let nginx=Nginx::new(temp.path());let p=paths(temp.path());
    std::fs::create_dir_all(&p.certs).unwrap();
    let mut refused=Vec::new();
    for (index,label) in ["missing-cert","empty-cert","missing-key","empty-key","valid"].iter().enumerate() {
        let dir=p.certs.join(label);std::fs::create_dir_all(&dir).unwrap();let cert=dir.join("fullchain.pem");let key=dir.join("privkey.pem");
        if *label!="missing-cert" {std::fs::write(&cert,if *label=="empty-cert"{""}else{"certificate fixture"}).unwrap();}
        if *label!="missing-key" {std::fs::write(&key,if *label=="empty-key"{""}else{"private-key fixture"}).unwrap();}
        let file=format!("server-{label}.conf");
        std::fs::write(p.conf.join(&file),format!("server {{ listen {} ssl; server_name {label}.test; ssl_certificate {}; ssl_certificate_key {}; location / {{ proxy_pass http://10.0.0.5:9000; }} }}\n",18443+index,cert.display(),key.display())).unwrap();
        if *label!="valid" {refused.push((file,*label));}
    }
    std::fs::write(p.conf.join("server-good.conf"),"server { listen 18087; server_name good.test; location / { proxy_pass http://10.0.0.5:9000; } }\n").unwrap();
    let output=guard(&nginx,&p,"2").output().unwrap();let stderr=String::from_utf8_lossy(&output.stderr);assert!(output.status.success(),"{stderr}");
    assert!(p.conf.join("server-valid.conf").is_file());assert!(p.conf.join("server-good.conf").is_file());
    let remaining:Vec<_>=refused.iter().filter(|(file,_)|p.conf.join(file).exists()).map(|(file,_)|file).collect();
    assert!(remaining.is_empty(),"unavailable TLS files remain live: {remaining:?}; {stderr}");
    for (file,label) in refused {
        assert!(std::fs::read_dir(&p.quarantine).unwrap().any(|dir|dir.unwrap().path().join("conf.d").join(&file).is_file()),"{file} lost its recovery copy");
        assert!(stderr.contains(&file)&&stderr.contains(&format!("{}/",p.certs.join(label).display())),"report server and certificate: {stderr}");
    }
}
