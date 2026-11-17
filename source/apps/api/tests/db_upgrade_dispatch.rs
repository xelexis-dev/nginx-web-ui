use std::process::Command;
fn command(args:&[&str],root:&std::path::Path)->std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_nwui-api")).args(args).env_clear().env("PGROOT",root).output().unwrap()
}
#[test]
fn storage_subcommands_refuse_conflicting_roots_before_config_or_secret_creation() {
    for args in [&["storage","classify"][..],&["storage-secrets"][..],&["storage-provision"][..]] {
        let root=tempfile::tempdir().unwrap();let output=command(args,root.path());
        assert_eq!(output.status.code(),Some(2));assert!(output.stdout.is_empty());
        let error=String::from_utf8_lossy(&output.stderr);assert!(error.contains("storage_override"));assert!(!error.contains("DATABASE_URL"));
        assert!(std::fs::read_dir(root.path()).unwrap().next().is_none());
    }
}

#[test]
fn normal_boot_refuses_storage_before_appstate_can_create_a_key() {
    let root=tempfile::tempdir().unwrap();let key=root.path().join("data_key");
    let mut child=Command::new(env!("CARGO_BIN_EXE_nwui-api")).arg("boot").env_clear()
        .env("PGROOT",root.path()).env("DATABASE_URL","postgres://nginx:fixture@127.0.0.1:9/nginx_web_ui")
        .env("AUTH_SECRET","synthetic-precheck").env("DATA_KEY_FILE",&key)
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    let deadline=std::time::Instant::now()+std::time::Duration::from_secs(5);
    while child.try_wait().unwrap().is_none()&&!key.exists()&&std::time::Instant::now()<deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if child.try_wait().unwrap().is_none(){child.kill().unwrap();}
    let output=child.wait_with_output().unwrap();
    assert!(!key.exists(),"AppState generated a key before storage classification");
    assert!(String::from_utf8_lossy(&output.stderr).contains("storage_override"));
}

#[test]
fn f1_runner_parks_once_and_auto_status_precedes_invalid_startup_inputs(){
    use std::{fs,io::Read,os::{fd::AsRawFd,unix::{fs::PermissionsExt,net::UnixStream,process::CommandExt}},path::Path,time::{Duration,Instant}};
    let app=Path::new("/var/lib/nginx-web-ui");let status=Path::new("/run/nwui-db-upgrade.json");let descriptor=Path::new("/run/nwui-storage.json");
    assert!(!status.exists()&&!descriptor.exists()&&!app.join(".nwui-auto/journal-v1.json").exists(),"requires disposable verifier container");
    let (mut ready,notify)=UnixStream::pair().unwrap();ready.set_read_timeout(Some(Duration::from_secs(5))).unwrap();let fd=notify.as_raw_fd();
    let mut command=Command::new(env!("CARGO_BIN_EXE_nwui-api"));command.arg("upgrade-runner").env_clear().env("APP_GIT_SHA","invalid-candidate").env("PGDATA","/invalid").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    unsafe{command.pre_exec(move||{if libc::dup2(fd,3)<0||libc::fcntl(3,libc::F_SETFD,0)<0{return Err(std::io::Error::last_os_error());}Ok(())});}
    let mut runner=command.spawn().unwrap();drop(notify);
    let mut byte=[0];let published=ready.read_exact(&mut byte);
    if published.is_err(){let _=runner.kill();let output=runner.wait_with_output().unwrap();panic!("readiness failed: {}",String::from_utf8_lossy(&output.stderr));}
    assert_eq!(&byte,b"\n");std::thread::sleep(Duration::from_millis(200));assert!(runner.try_wait().unwrap().is_none());
    let original=fs::read(status).unwrap();let mut value:serde_json::Value=serde_json::from_slice(&original).unwrap();assert_eq!(value["failure"]["code"],"verification_failed");
    for code in ["verification_failed","storage_override"]{
        value["failure"]["code"]=serde_json::json!(code);fs::write(status,serde_json::to_vec(&value).unwrap()).unwrap();fs::set_permissions(status,fs::Permissions::from_mode(0o644)).unwrap();
        let before=fs::read(status).unwrap();
        for (name,input) in [("PGDATA","/invalid"),("APP_GIT_SHA","invalid-candidate"),("MIGRATIONS_DIR","/nonexistent")]{
            let output=Command::new(env!("CARGO_BIN_EXE_nwui-api")).arg("auto-status").env_clear().env(name,input).output().unwrap();
            assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));let text=String::from_utf8(output.stdout).unwrap();assert!(text.contains(code));assert!(text.contains("다시 확인합니다"));assert!(!text.contains("준비를 마쳤습니다"));assert_eq!(fs::read(status).unwrap(),before);
        }
    }
    assert!(!app.join(".nwui-auto/journal-v1.json").exists());
    let start=Instant::now();assert_eq!(unsafe{libc::kill(runner.id()as i32,libc::SIGTERM)},0);
    while runner.try_wait().unwrap().is_none()&&start.elapsed()<Duration::from_secs(3){std::thread::sleep(Duration::from_millis(10));}
    if runner.try_wait().unwrap().is_none(){let _=runner.kill();panic!("parked runner did not stop");}
    let output=runner.wait_with_output().unwrap();assert!(output.status.success());let log=String::from_utf8_lossy(&output.stderr);assert_eq!(log.lines().count(),1,"{log}");assert!(log.contains("preparation failed before the recovery record: verification_failed"));
    fs::remove_file(status).unwrap();fs::remove_file(descriptor).unwrap();
}
