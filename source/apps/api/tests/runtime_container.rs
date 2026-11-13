#![cfg(unix)]
mod guard_support;
use std::{os::unix::fs::PermissionsExt, process::Command};
// These fixtures exercise the consumer after successful selection; real gate outcomes have their own tests.
fn stub_longrun_gate(source: &str, directory: &std::path::Path) -> String {
    let gate=directory.join("nwui-gate.sh");
    std::fs::write(&gate,"nwui_gate_longrun() { :; }\n").unwrap();
    assert!(source.find(". /etc/s6-overlay/scripts/nwui-gate.sh").unwrap()<source.find("nwui_gate_longrun").unwrap());
    source.replace("/etc/s6-overlay/scripts/nwui-gate.sh",gate.to_str().unwrap())
}
#[test]
fn web_runtime_inherits_only_frontend_environment_and_custom_api_port() {
    let temp = tempfile::TempDir::new().unwrap();
    let node = temp.path().join("node");
    std::fs::write(&node, "#!/bin/sh\n/usr/bin/env\n").unwrap();
    std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o700)).unwrap();
    let setuid = temp.path().join("s6-setuidgid");
    std::fs::write(&setuid, "#!/bin/sh\nprintf 'effective-user=%s\\n' \"$1\"\nshift\nexec \"$@\"\n").unwrap();
    std::fs::set_permissions(&setuid, std::fs::Permissions::from_mode(0o700)).unwrap();
    let source = include_str!("../../../docker/s6-rc.d/web/run");
    let script = temp.path().join("run");
    std::fs::write(
        &script,
        stub_longrun_gate(source,temp.path()).replace("cd /app", &format!("cd '{}'", temp.path().display()))
            .replace("/usr/local/bin/node", &node.display().to_string())
            .replace("/command/s6-setuidgid", &setuid.display().to_string()),
    )
    .unwrap();
    let output = Command::new("sh")
        .arg(&script)
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", temp.path().display()))
        .env("PORT", "3000")
        .env("API_PORT", "3101")
        .env("DATABASE_URL", "private-database-test")
        .env("AUTH_SECRET", "private-auth-test")
        .env("DATA_KEY_FILE", "private-key-path-test")
        .env("ADMIN_PASSWORD", "private-admin-test")
        .output()
        .unwrap();
    assert!(output.status.success());
    let env = String::from_utf8(output.stdout).unwrap();
    assert!(env.contains("effective-user=node"), "{env}");
    assert!(env.contains("TZ=UTC"), "{env}");
    assert!(env.contains("API_URL=http://127.0.0.1:3101"));
    assert!(env.contains("HOSTNAME=127.0.0.1"));
    for secret in [
        "DATABASE_URL=",
        "AUTH_SECRET=",
        "DATA_KEY_FILE=",
        "ADMIN_PASSWORD=",
        "private-",
    ] {
        assert!(!env.contains(secret));
    }
}

/// s6 restarts nginx whenever it dies, not only at container start. Every start
/// re-runs the managed-file guard first (1.17.2 did this in the same script), and
/// nginx still starts when the guard itself fails.
#[test]
fn nginx_service_guards_managed_files_before_every_start() {
    let source = include_str!("../../../docker/s6-rc.d/nginx/run");
    for guard_exit in [0, 1, 75] {
        let temp = tempfile::TempDir::new().unwrap();
        let log = temp.path().join("calls");
        let api = temp.path().join("nwui-api");
        let deferral=if guard_exit==75 {"echo '[nginx-guard] nginx start deferred: io_error' >&2\n"}else{""};
        std::fs::write(&api, format!("#!/bin/sh\necho \"api $*\" >> '{}'\n{deferral}exit {guard_exit}\n", log.display())).unwrap();
        let nginx = temp.path().join("nginx");
        std::fs::write(&nginx, format!("#!/bin/sh\necho \"nginx $*\" >> '{}'\n", log.display())).unwrap();
        for bin in [&api, &nginx] {
            std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert!(source.contains("/usr/local/bin/nwui-api guard"), "{source}");
        let script = temp.path().join("run");
        assert!(source.find("nwui_gate_longrun").unwrap()<source.find("/usr/local/bin/nwui-api guard").unwrap());
        std::fs::write(&script, stub_longrun_gate(source,temp.path()).replace("/usr/local/bin/nwui-api", &api.display().to_string()).replace("/run/nginx-web-ui",&temp.path().join("runtime").display().to_string())).unwrap();
        let output = Command::new("sh")
            .arg(&script)
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", temp.path().display()))
            .output()
            .unwrap();
        if guard_exit==75 {
            assert_eq!(output.status.code(),Some(75),"ownership timeout must defer nginx startup");
            assert_eq!(std::fs::read_to_string(&log).unwrap(),"api guard\n");
            assert_eq!(String::from_utf8_lossy(&output.stderr).lines().count(),1,"{output:?}");
            continue;
        }
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "api guard\nnginx -g daemon off;\n",
            "guard exit {guard_exit}"
        );
    }
}

/// `nwui-api guard` is the boot guard on its own: a managed file that fails
/// `nginx -t` moves to the recovery directory so nginx and the admin UI start.
#[test]
fn guard_subcommand_quarantines_managed_files_that_fail_validation() {
    guard_support::assert_no_host_descriptor();
    let temp = tempfile::TempDir::new().unwrap();
    let conf = temp.path().join("conf");
    let stream = temp.path().join("stream");
    let quarantine = temp.path().join("quarantine");
    std::fs::create_dir_all(&conf).unwrap();
    std::fs::create_dir_all(&stream).unwrap();
    let main = temp.path().join("nginx.conf");
    std::fs::write(&main, "listen 80;\nlisten 443 ssl;\nlisten 8080;\n").unwrap();
    std::fs::write(conf.join("server-ghost.conf"), "proxy_pass http://unresolved;\n").unwrap();
    std::fs::write(conf.join("server-ok.conf"), "proxy_pass http://127.0.0.1:8080;\n").unwrap();
    // The API is running while nginx restarts: its in-flight atomic write is not ours to remove.
    let in_flight = conf.join(".nwui-0f0e2d3c-1b2a-4c5d-8e9f-a0b1c2d3e4f5.tmp");
    std::fs::write(&in_flight, "being written").unwrap();
    let nginx = temp.path().join("nginx");
    std::fs::write(
        &nginx,
        format!("#!/bin/sh\nif [ \"$1\" = -V ]; then printf '%s\\n' 'nginx version: nginx/1.30.6' 'configure arguments: --prefix={} --conf-path={}' >&2; exit 0; fi\nif grep -rq unresolved '{}' '{}'; then echo 'host not found in upstream'; exit 1; fi\nexit 0\n", temp.path().display(), main.display(), conf.display(), stream.display()),
    )
    .unwrap();
    std::fs::set_permissions(&nginx, std::fs::Permissions::from_mode(0o700)).unwrap();
    let store_root=temp.path().join("publisher-state");
    std::fs::create_dir_all(&store_root).unwrap();
    nwui_api::publisher::test_support::prepare_store_fixture(&store_root).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_nwui-api"))
        .arg("guard")
        .env_clear()
        .env("NWUI_TEST_PUBLISHER_ROOT", &store_root)
        .env("PATH",format!("{}:/usr/bin:/bin",temp.path().display()))

        .env("DATABASE_URL", "postgres://unused@127.0.0.1:1/unused")
        .env("AUTH_SECRET", "unused-by-the-guard")
        .env("NGINX_BIN", &nginx)
        .env("NGINX_CONF", &main)
        .env("NGINX_CONF_D", &conf)
        .env("NGINX_STREAM_D", &stream)
        .env("NGINX_GUARD_QUARANTINE", &quarantine)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(!conf.join("server-ghost.conf").exists());
    assert!(in_flight.exists(), "removed a write the running API owns");
    assert!(stderr.contains("[nginx-guard]"), "{stderr}");
    let moved: Vec<_> = std::fs::read_dir(&quarantine).unwrap().collect();
    assert_eq!(moved.len(), 1, "{stderr}");
}

/// Explicit image gate; the ordinary PG suite has no Docker socket/image fixture.
#[test]
#[ignore = "native image: set NWUI_NATIVE_IMAGE, NWUI_CI_REV, NWUI_NATIVE_HOST_ROOT and NWUI_NATIVE_TEST_SCRIPT"]
fn native_image_containment_reload_and_recovery() {
    let image=std::env::var("NWUI_NATIVE_IMAGE").expect("set NWUI_NATIVE_IMAGE");
    let revision=std::env::var("NWUI_CI_REV").expect("exact image revision");
    let script=std::path::PathBuf::from(std::env::var("NWUI_NATIVE_TEST_SCRIPT").expect("set NWUI_NATIVE_TEST_SCRIPT"));
    let fixture=Command::new("node").arg("--test").arg(script.with_file_name("fixture.test.mjs")).status().unwrap();
    assert!(fixture.success(),"native HTTP fixture regression failed: {fixture}");
    let status=Command::new("node").arg(script).arg(image).arg(revision).status().unwrap();
    assert!(status.success(),"native M1/A8 image gate failed: {status}");
}

#[test]
fn web_timezone_accepts_installed_zones_and_rejects_paths_or_missing_zones() {
    let temp = tempfile::TempDir::new().unwrap();
    let node = temp.path().join("node");
    std::fs::write(&node, "#!/bin/sh\n/usr/bin/env\n").unwrap();
    std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o700)).unwrap();
    let setuid = temp.path().join("setuid");
    std::fs::write(&setuid, "#!/bin/sh\nshift\nexec \"$@\"\n").unwrap();
    std::fs::set_permissions(&setuid, std::fs::Permissions::from_mode(0o700)).unwrap();
    let zones = temp.path().join("zones");
    std::fs::create_dir_all(zones.join("Asia")).unwrap();
    std::fs::write(zones.join("UTC"), "fixture").unwrap();
    std::fs::write(zones.join("Asia/Seoul"), "fixture").unwrap();
    let source = stub_longrun_gate(include_str!("../../../docker/s6-rc.d/web/run"),temp.path())
        .replace("/usr/local/bin/node", &node.display().to_string())
        .replace("cd /app", &format!("cd '{}'", temp.path().display()))
        .replace("/command/s6-setuidgid", &setuid.display().to_string())
        .replace("/usr/share/zoneinfo", &zones.display().to_string());
    let script = temp.path().join("run");
    std::fs::write(&script, source).unwrap();
    for (zone, expected) in [("Asia/Seoul", "Asia/Seoul"), ("", "UTC"), ("Unknown/Zone", "UTC"), ("../UTC", "UTC"), ("/etc/passwd", "UTC"), (":/tmp/zone", "UTC")] {
        let output = Command::new("sh").arg(&script).env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", temp.path().display()))
            .env("TZ", zone).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        let env = String::from_utf8(output.stdout).unwrap();
        assert!(env.lines().any(|line| line == format!("TZ={expected}")), "input={zone}: {env}");
    }
}

/// Gate refusal precedes PG environment/version checks, and the shell never provisions storage.
#[test]
fn bootstrap_recovery_wait_is_bounded_and_cleans_up_a_timed_out_postmaster() {
    let temp = tempfile::TempDir::new().unwrap();
    let bin = temp.path().join("bin");std::fs::create_dir(&bin).unwrap();
    let data = temp.path().join("data");let app = temp.path().join("app");
    std::fs::create_dir(&data).unwrap();std::fs::create_dir(&app).unwrap();
    let api = bin.join("api");
    std::fs::write(&api,r#"#!/bin/sh
printf 'binary:%s\n' "$*" >> "$TRACE"
[ "$GATE_OUTCOME" != reader-failed ] || exit 2
printf '%s\n' "$GATE_OUTCOME"
"#).unwrap();std::fs::set_permissions(&api,std::fs::Permissions::from_mode(0o700)).unwrap();
    for name in ["chown","initdb","pg_ctl","openssl"] {
        let command=bin.join(name);std::fs::write(&command,"#!/bin/sh\necho mutation >> \"$TRACE\"\nexit 99\n").unwrap();
        std::fs::set_permissions(command,std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let resolver=temp.path().join("pg-env.sh");std::fs::write(&resolver,include_str!("../../../docker/scripts/pg-env.sh")).unwrap();
    let gate=temp.path().join("nwui-gate.sh");
    std::fs::write(&gate,include_str!("../../../docker/scripts/nwui-gate.sh").replace("/usr/local/bin/nwui-api",&api.display().to_string())).unwrap();
    let source=include_str!("../../../docker/scripts/pg-init.sh")
        .replace("/etc/s6-overlay/scripts/nwui-gate.sh",gate.to_str().unwrap())
        .replace(". /etc/s6-overlay/scripts/pg-env.sh",&format!(". '{}'\nPGDATA='{}'",resolver.display(),data.display()));
    let script=temp.path().join("pg-init");std::fs::write(&script,source).unwrap();
    for (outcome,version,legacy_override,code) in [("pg17","17",false,0),("refused-fresh","15",false,0),("reader-failed","15",false,2),("pg17","17",true,2),("pg17","15",false,2)] {
        std::fs::write(data.join("PG_VERSION"),version).unwrap();
        let trace=temp.path().join(format!("calls-{outcome}-{version}-{legacy_override}"));
        let mut command=Command::new("sh");command.arg(&script).env_clear().env("PATH",format!("{}:/usr/bin:/bin",bin.display())).env("TRACE",&trace).env("GATE_OUTCOME",outcome);
        if legacy_override{command.env("PGDATA","/var/lib/postgresql/data");}
        let output=command.output().unwrap();assert_eq!(output.status.code(),Some(code),"{outcome}/{version}/{legacy_override}: {output:?}");
        assert_eq!(std::fs::read_to_string(&trace).unwrap(),"binary:gate-outcome\n");
        if legacy_override{assert!(String::from_utf8_lossy(&output.stderr).contains("storage_override"));}
        if outcome=="pg17"&&version=="15"{assert!(String::from_utf8_lossy(&output.stderr).contains("storage_selection_invalid"));}
        assert_eq!(std::fs::read_to_string(data.join("PG_VERSION")).unwrap(),version);
        assert!(!app.join("secrets").exists());assert_eq!(std::fs::read_dir(&data).unwrap().count(),1);
    }
}


#[test]
fn unsafe_nginx_admission_defers_before_executing_or_guarding() {
    guard_support::assert_no_host_descriptor();
    let temp=tempfile::TempDir::new().unwrap();let bin=temp.path().join("nginx");let ran=temp.path().join("ran");
    std::fs::write(&bin,format!("#!/bin/sh\necho ran > '{}'\nexit 0\n",ran.display())).unwrap();
    std::fs::set_permissions(&bin,std::fs::Permissions::from_mode(0o777)).unwrap();
    let conf=temp.path().join("conf");std::fs::create_dir(&conf).unwrap();
    let managed=conf.join("server-keep.conf");std::fs::write(&managed,"must stay unchanged").unwrap();
    let main=temp.path().join("nginx.conf");std::fs::write(&main,"events {}\n").unwrap();
    let output=Command::new(env!("CARGO_BIN_EXE_nwui-api")).arg("guard").env_clear()
        .env("PATH",format!("{}:/usr/bin:/bin",temp.path().display()))
        .env("DATABASE_URL","postgres://unused@127.0.0.1:1/unused").env("AUTH_SECRET","fixture-only")
        .env("NGINX_BIN",&bin).env("NGINX_CONF",&main).env("NGINX_CONF_D",&conf)
        .env("NGINX_STREAM_D",temp.path().join("stream")).env("NGINX_GUARD_QUARANTINE",temp.path().join("quarantine")).output().unwrap();
    assert_eq!(output.status.code(),Some(75),"{output:?}");
    assert_eq!(String::from_utf8(output.stderr).unwrap(),"[nginx-guard] nginx start deferred: trusted_input_unsafe\n");
    assert!(!ran.exists());assert_eq!(std::fs::read_to_string(managed).unwrap(),"must stay unchanged");
}
