pub use nwui_api::{upgrade,db_upgrade::layout};
#[path="../src/db_upgrade/settings.rs"] mod settings;
#[path="../src/db_upgrade/files.rs"] mod files;
#[path="../src/db_upgrade/state.rs"] mod state;
#[path="../src/db_upgrade/error.rs"] mod error;
#[path="../src/db_upgrade/control.rs"] mod control;
#[path="../src/db_upgrade/shared_folder.rs"] mod shared_folder;
#[path="../src/db_upgrade/mounts.rs"] mod mounts;
#[path="../src/db_upgrade/postgres.rs"] mod postgres;
use std::path::Path;
#[test]
fn postgres_commands_drop_root_and_discard_inherited_connection_and_secret_settings() {
    let command=postgres::command(17,"pg_controldata").unwrap();
    assert_eq!(command.get_program(),"/usr/sbin/runuser");
    assert_eq!(command.get_args().collect::<Vec<_>>(),["-u","postgres","--","/usr/lib/postgresql/17/bin/pg_controldata"]);
    let env=command.get_envs().filter_map(|(k,v)|v.map(|v|(k.to_string_lossy().into_owned(),v.to_string_lossy().into_owned()))).collect::<Vec<_>>();
    assert_eq!(env,[("HOME".into(),"/var/lib/postgresql".into()),("LC_ALL".into(),"C".into()),("PATH".into(),"/usr/bin:/bin".into()),("TZ".into(),"UTC".into())]);
    assert!(postgres::command(16,"postgres").is_err());assert!(postgres::command(17,"/bin/sh").is_err());
}
#[test]
fn private_postmaster_is_socket_only_and_cannot_consume_source_auto_configuration() {
    let command=postgres::server_command(15,Path::new("/destination/db/.nwui-work/attempt/pg15"),Path::new("/destination/db/.nwui-work/attempt/socket15"),Path::new("/destination/db/.nwui-work/attempt/private.conf")).unwrap();
    let args=command.get_args().map(|a|a.to_string_lossy().into_owned()).collect::<Vec<_>>();
    for expected in ["listen_addresses=","unix_socket_permissions=0700","port=50432","config_file=/destination/db/.nwui-work/attempt/private.conf","shared_preload_libraries=","session_preload_libraries=","local_preload_libraries=","archive_mode=off","ssl=off"] {assert!(args.contains(&expected.into()),"{expected}");}
    assert!(args.windows(2).any(|a|a==["-D","/destination/db/.nwui-work/attempt/pg15"]));
}
#[test]
fn warning_control_output_and_failed_processes_are_sanitized() {
    let warning=std::process::Command::new("/bin/sh").args(["-c","printf 'private secret' >&2; exit 1"]).stderr(std::process::Stdio::null()).spawn().unwrap();
    let error=postgres::wait(warning,std::time::Duration::from_secs(1)).unwrap_err();
    assert_eq!(error.exit,1);assert!(!error.to_string().contains("private secret"));
}

#[test]
fn readonly_control_output_is_bounded_and_does_not_create_files() {
    let dir=tempfile::tempdir().unwrap();
    let mut command=std::process::Command::new("/bin/sh");command.current_dir(dir.path()).args(["-c","printf control; printf warning >&2"]);
    assert_eq!(postgres::output(command,std::time::Duration::from_secs(1)).unwrap(),("control".into(),"warning".into()));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(),0);
    let mut command=std::process::Command::new("/bin/sh");command.args(["-c","head -c 70000 /dev/zero"]);
    assert!(postgres::output(command,std::time::Duration::from_secs(1)).is_err());
}

#[test]
fn private_socket_length_is_refused_before_starting_a_postmaster() {
    let socket=std::path::PathBuf::from(format!("/{}","x".repeat(110)));
    assert!(postgres::server_command(15,Path::new("/data"),&socket,Path::new("/config")).is_err());
}

#[test]
fn every_offline_socket_uses_container_run_even_with_a_long_bind_log_path() {
    let work=std::path::PathBuf::from(format!("/destination/db/.nwui-work/{}/inspect15-log","x".repeat(150)));
    for label in ["s15","s17","upgrade"] {
        let socket=postgres::socket_path(&work,label);
        assert!(socket.starts_with("/run"),"socket escaped container-local /run: {}",socket.display());
        assert!(socket.as_os_str().len()+"/.s.PGSQL.50432".len()<108);
        assert_ne!(socket,postgres::socket_path(Path::new("/other/log"),label));
    }
}

#[test]
#[ignore="native PG15/17 as root: NWUI_TEST_STOP_WORKING=1"]
fn working_postmaster_stop_uses_live_identity_even_when_pid_file_is_root_owned() {
    if std::env::var("NWUI_TEST_STOP_WORKING").as_deref()!=Ok("1"){return;}
    use std::{fs,os::unix::fs::{MetadataExt,PermissionsExt},time::Duration};
    use std::sync::atomic::Ordering;
    struct Reset;impl Drop for Reset{fn drop(&mut self){postgres::SIGNAL.store(0,Ordering::Relaxed);}}let _reset=Reset;
    assert_eq!(unsafe{libc::geteuid()},0);
    for major in [15,17] {
        let root=std::env::temp_dir().join(format!("nwui-stop-{}",uuid::Uuid::new_v4()));postgres::directory(&root).unwrap();
        let data=root.join("data");postgres::directory(&data).unwrap();let init=root.join("init");postgres::directory(&init).unwrap();
        let mut command=postgres::command(major,"initdb").unwrap();command.arg("-D").arg(&data).args(["--locale=C","--encoding=UTF8","--auth-local=trust","--auth-host=scram-sha-256"]);postgres::capture(command,&init,Duration::from_secs(60)).unwrap();
        let log=root.join("log");postgres::directory(&log).unwrap();let pg=postgres::Private::start(major,&data,&log).unwrap();let socket=pg.socket.clone();
        let path=data.join("postmaster.pid");let pid:i32=fs::read_to_string(&path).unwrap().lines().next().unwrap().parse().unwrap();
        let uid=fs::metadata(&path).unwrap().uid();assert_ne!(uid,0);std::os::unix::fs::chown(&path,Some(0),Some(0)).unwrap();assert_eq!(fs::metadata(&path).unwrap().uid(),0);
        // Keep it readable to postgres, as on the uid-remapped Mac bind; the parent remains 0700.
        fs::set_permissions(&path,fs::Permissions::from_mode(0o644)).unwrap();
        postgres::SIGNAL.store(15,Ordering::Relaxed);
        let result=postgres::stop_working(major,&data);postgres::SIGNAL.store(0,Ordering::Relaxed);
        assert!(result.is_ok(),"root-owned pid file must not hide our live postmaster, even during TERM cleanup");
        assert!(!path.exists(),"stop_working must stop the postmaster before its owner drops");drop(pg);
        assert!(!path.exists());assert!(!socket.exists());assert_eq!(unsafe{libc::kill(pid,0)},-1);assert_eq!(std::io::Error::last_os_error().raw_os_error(),Some(libc::ESRCH));
    }
}

#[test]
#[ignore="native Linux postgres account as root: NWUI_TEST_STOP_WORKING=1"]
fn working_postmaster_stop_refuses_another_live_uid_or_cwd_without_signalling() {
    if std::env::var("NWUI_TEST_STOP_WORKING").as_deref()!=Ok("1"){return;}
    use std::{fs,process::Command};
    struct Child(std::process::Child);impl Drop for Child{fn drop(&mut self){let _=self.0.kill();let _=self.0.wait();}}
    let root=std::env::temp_dir().join(format!("nwui-stop-{}",uuid::Uuid::new_v4()));postgres::directory(&root).unwrap();
    let data=root.join("data");postgres::directory(&data).unwrap();let other=root.join("other");postgres::directory(&other).unwrap();
    for other_uid in [true,false] {
        let mut command=Command::new("/bin/sleep");
        // Direct setuid avoids testing runuser's wrapper PID instead of its child.
        if !other_uid{use std::os::unix::{fs::MetadataExt,process::CommandExt};command.uid(fs::metadata(&data).unwrap().uid());}
        command.arg("60").current_dir(if other_uid{&data}else{&other});let mut child=Child(command.spawn().unwrap());
        let path=data.join("postmaster.pid");fs::write(&path,format!("{}\n{}\n",child.0.id(),data.display())).unwrap();postgres::own(&path).unwrap();
        assert!(postgres::stop_working(15,&data).is_err());assert!(child.0.try_wait().unwrap().is_none(),"unrelated process was signalled");fs::remove_file(path).unwrap();
    }
}
