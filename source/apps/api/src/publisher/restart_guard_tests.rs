//! Writer kernel tests stay inside the publisher; CLI integration stays in its binary target.
use super::{apply,nginx_lock};
use super::test_helpers::restart_guard::*;
use std::{process::Command,time::{Duration,Instant}};
use tempfile::TempDir;
#[path="../../tests/guard_support/mod.rs"]
mod guard_support;

fn guard(nginx:&Nginx,p:&Paths,wait_secs:&str)->Command {
    guard_support::assert_no_host_descriptor();
    let mut command=Command::new(std::env::current_exe().unwrap());
    command.args(["--exact","publisher::restart_guard_tests::guard_worker","--nocapture","--test-threads=1"]);
    let mut command=guard_command(command,nginx,p,wait_secs);
    // guard_command intentionally clears the environment before applying its own inputs.
    command.env("NWUI_GUARD_KERNEL_CHILD","1");command
}

#[tokio::test]
async fn guard_worker() {
    if std::env::var("NWUI_GUARD_KERNEL_CHILD").as_deref()!=Ok("1") {return;}
    let config=crate::config::RuntimeConfig::from_env().unwrap();
    let store=super::boot::PublisherStore::open(&config).unwrap();
    super::boot::guard(&config,&store).await.unwrap();
}

#[tokio::test]
async fn apply_waits_for_the_guard_process() {
    let temp = TempDir::new().unwrap();
    let p = paths(temp.path());
    let nginx = Nginx::new(temp.path());
    nginx.sleep("guard", 1.0);
    let state = api_state(temp.path(), &nginx, &p, "10");
    let child = guard(&nginx, &p, "10").spawn().unwrap();
    nginx.wait_for("guard start -t").await;
    let result = apply::apply_rendered(&state, &rendered(), &[]).await.unwrap();
    assert!(result.ok, "{}", result.output);
    let (ok, stderr) = finish(child);
    assert!(ok, "{stderr}");
    nginx.assert_before("guard", "api");
}

#[tokio::test]
async fn a_killed_guard_releases_the_files() {
    let temp = TempDir::new().unwrap();
    let p = paths(temp.path());
    let nginx = Nginx::new(temp.path());
    nginx.sleep("guard", 30.0);
    let state = api_state(temp.path(), &nginx, &p, "20");
    let mut child = guard(&nginx, &p, "20").spawn().unwrap();
    nginx.wait_for("guard start -t").await;
    let probe = std::fs::File::open(nginx_lock::path(&state.config)).unwrap();
    assert!(probe.try_lock().is_err(), "the guard holds the files while it validates");
    child.kill().unwrap();
    child.wait().unwrap();
    let started = Instant::now();
    let result = apply::apply_rendered(&state, &rendered(), &[]).await.unwrap();
    assert!(result.ok, "{}", result.output);
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    nginx.kill_leftover("guard");
}

#[tokio::test]
async fn apply_gives_up_without_touching_files_and_without_blocking_the_runtime() {
    let temp = TempDir::new().unwrap();
    let p = paths(temp.path());
    let nginx = Nginx::new(temp.path());
    nginx.sleep("guard", 4.0);
    std::fs::write(p.conf.join("server-old.conf"), "server { listen 8082; }\n").unwrap();
    let state = api_state(temp.path(), &nginx, &p, "1");
    let child = guard(&nginx, &p, "10").spawn().unwrap();
    nginx.wait_for("guard start -t").await;
    let before = snapshot(&p.conf);
    let ticks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let ticker = {
        let ticks = ticks.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                ticks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        })
    };
    let error = apply::apply_rendered(&state, &rendered(), &[]).await.unwrap_err();
    ticker.abort();
    assert!(error.to_string().contains("건드리지 않았습니다"), "{error:#}");
    assert!(ticks.load(std::sync::atomic::Ordering::SeqCst) >= 10, "the wait blocked the runtime");
    assert_eq!(snapshot(&p.conf), before);
    assert!(!nginx.calls().iter().any(|l| l.starts_with("api")));
    let (ok, stderr) = finish(child);
    assert!(ok, "{stderr}");
}
