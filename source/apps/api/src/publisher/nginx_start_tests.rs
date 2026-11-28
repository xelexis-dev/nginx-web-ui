use super::confirmation::{nginx_state_with,NginxState};
use std::{path::Path,process::{Child,Command},os::unix::process::CommandExt};

pub(super) struct Process(pub(super) Child);
impl Drop for Process {fn drop(&mut self){let _=unsafe{libc::kill(-(self.0.id()as i32),libc::SIGKILL)};let _=self.0.kill();let _=self.0.wait();}}
fn shell(script:&Path)->Process {
    let ack=script.with_extension("ready");
    std::fs::write(script,format!(": > '{}'\nwhile :; do sleep 1; done\n",ack.display())).unwrap();
    let process=Process(Command::new("/bin/sh").arg(script).process_group(0).spawn().unwrap());
    let deadline=std::time::Instant::now()+std::time::Duration::from_secs(2);
    while !ack.exists(){assert!(std::time::Instant::now()<deadline,"run shell did not initialize");std::thread::sleep(std::time::Duration::from_millis(1));}
    process
}

const SERVICE: &str = "/run/service/nginx";
const GROUP: &str = "/run/nginx-web-ui/nginx-process-group";

struct Supervision(Child);

impl Supervision {
    fn start(run: &str, down: bool, configure: impl FnOnce(&mut Command)) -> Self {
        use std::os::unix::fs::PermissionsExt;
        assert!(!Path::new(SERVICE).exists(), "use an isolated container without s6 init");
        std::fs::create_dir_all(SERVICE).unwrap();
        let script = Path::new(SERVICE).join("run");
        std::fs::write(&script, run).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        if down {
            std::fs::write(Path::new(SERVICE).join("down"), b"").unwrap();
        }
        let mut supervisor = Command::new("/command/s6-supervise");
        supervisor.arg(SERVICE);
        configure(&mut supervisor);
        Self(supervisor.spawn().unwrap())
    }

    fn command(&self, option: &str) {
        assert!(Command::new("/command/s6-svc").args([option, SERVICE]).status().unwrap().success());
    }

    fn status(&self) -> String {
        let output = Command::new("/command/s6-svstat")
            .args(["-o", "up,pid", SERVICE]).output().unwrap();
        String::from_utf8(output.stdout).unwrap()
    }
}

impl Drop for Supervision {
    fn drop(&mut self) {
        let _ = Command::new("/command/s6-svc").args(["-dx", SERVICE]).status();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if self.0.try_wait().is_ok_and(|status| status.is_some()) {
                let _ = std::fs::remove_dir_all(SERVICE);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn wait_native(mut predicate: impl FnMut() -> bool) {
    use std::time::Duration;
    tokio::time::timeout(Duration::from_secs(10), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("native nginx did not reach the expected state");
}

#[tokio::test]
#[ignore = "uses fixed nginx and s6 paths in a disposable runtime image; NWUI_C3_NATIVE_NGINX=1"]
async fn c3_real_s6_cold_start_fifo_init_and_foreign_nginx() {
    use super::{confirmation, recovery};
    use std::time::{Duration, Instant};
    if std::env::var("NWUI_C3_NATIVE_NGINX").as_deref() != Ok("1") { return; }
    let (root, mut config, store, _, _) = recovery::tests::durable_fixture();
    config.nginx_bin = "/usr/sbin/nginx".into();
    config.nginx_main_conf = "/etc/nginx/nginx.conf".into();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    config.nginx_status_url = format!("http://127.0.0.1:{port}/nginx_status");
    let pid_file = root.path().join("custom.pid");
    let fifo = root.path().join("init.fifo");
    use std::os::unix::ffi::OsStrExt;
    let fifo_path = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o600) }, 0);
    let managed = format!(
        "pid {}; error_log {} notice; events {{}} http {{ access_log off; server {{ listen 127.0.0.1:{port}; location = /nginx_status {{ stub_status; allow 127.0.0.3; deny all; include {}/.nwui-generation; }} }} }}",
        pid_file.display(), root.path().join("nginx.log").display(), config.nginx_conf_dir.display(),
    );
    std::fs::write(&config.nginx_main_conf, &managed).unwrap();
    let first = uuid::Uuid::new_v4().to_string();
    confirmation::publish(&config, &first).await.unwrap();
    use std::os::unix::fs::{PermissionsExt, chown};
    let (uid, gid) = crate::db_upgrade::postgres::postgres_ids().unwrap();
    let nonroot = |bin: &Path| {
        let process = Process(Command::new(bin).arg("60").uid(uid).gid(gid).process_group(0).spawn().unwrap());
        let exe = format!("/proc/{}/exe", process.0.id());
        let error = std::fs::metadata(exe).unwrap_err();
        assert!(matches!(error.raw_os_error(), Some(libc::EACCES | libc::EPERM)), "{error}");
        process
    };
    let foreign = nonroot(Path::new("/bin/sleep"));
    let named = tempfile::tempdir().unwrap();
    std::fs::set_permissions(named.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let named_nginx = named.path().join("nginx");
    std::fs::copy("/bin/sleep", &named_nginx).unwrap();
    let nginx_named_foreign = nonroot(&named_nginx);
    std::fs::write(&pid_file, nginx_named_foreign.0.id().to_string()).unwrap();
    std::fs::create_dir_all(Path::new(GROUP).parent().unwrap()).unwrap();
    let supervisor = Supervision::start(&format!(
        r#"#!/bin/sh
umask 077
printf '%s\n' "$$" > '{GROUP}'
printf '%s\n' "$$" > '{}'
exec /usr/sbin/nginx -g 'daemon off;'
"#, pid_file.display()
    ), true, |_| {});
    wait_native(|| supervisor.status().trim_end() == "false -1").await;
    assert!(confirmation::nginx_running(&config).await.unwrap_err().is::<confirmation::IdentityUncertain>());
    std::fs::write(&pid_file, foreign.0.id().to_string()).unwrap();
    chown(&pid_file, Some(uid), Some(gid)).unwrap();
    std::fs::set_permissions(&pid_file, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert_eq!(confirmation::nginx_running(&config).await.unwrap(), NginxState::Stopped);
    // Admission checks regular includes; the FIFO is added only to hold the real nginx in init.
    std::fs::write(&config.nginx_main_conf, format!("include {};\n{managed}", fifo.display())).unwrap();
    supervisor.command("-u");
    wait_native(|| {
        let Ok(pid) = std::fs::read_to_string(GROUP) else { return false };
        std::fs::metadata(format!("/proc/{}/exe", pid.trim())).ok().zip(std::fs::metadata("/usr/sbin/nginx").ok())
            .is_some_and(|(observed, expected)| {
                use std::os::unix::fs::MetadataExt;
                observed.dev() == expected.dev() && observed.ino() == expected.ino()
            })
    }).await;
    let live_pid = std::fs::read_to_string(GROUP).unwrap();
    let files = super::files::capture_payload_files(&config).await.unwrap();
    let manifest = std::fs::read(store.root.join("manifest.json")).unwrap();
    assert_eq!(confirmation::nginx_running(&config).await.unwrap(), NginxState::Starting);
    assert!(confirmation::ensure_nginx_stopped(&config).await.is_err());
    let witness = confirmation::Witness::from_config(&config).unwrap();
    assert!(witness.confirm_with(&config, &first, Instant::now() + Duration::from_secs(3), &store,
        |_| panic!("blocked master must not acknowledge a reload")
    ).await.is_err());
    assert_eq!(std::fs::read(store.root.join("manifest.json")).unwrap(), manifest);
    assert_eq!(super::files::capture_payload_files(&config).await.unwrap(), files);
    assert_eq!(unsafe { libc::kill(live_pid.trim().parse().unwrap(), 0) }, 0);

    tokio::time::timeout(Duration::from_secs(5), tokio::task::spawn_blocking(move || {
        std::fs::OpenOptions::new().write(true).open(fifo).unwrap()
    })).await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while reqwest::get(&config.nginx_status_url).await.is_err() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    std::fs::write(&config.nginx_main_conf, &managed).unwrap();
    assert_eq!(confirmation::nginx_running(&config).await.unwrap(), NginxState::Running);
    let mut acknowledged = 0;
    witness.confirm_with(&config, &first, Instant::now() + Duration::from_secs(10), &store,
        |phase| { assert_eq!(phase, acknowledged); acknowledged += 1; Ok(()) }
    ).await.unwrap();
    assert_eq!(acknowledged, 2);
    assert_eq!(std::fs::read_to_string(GROUP).unwrap(), live_pid);

    std::fs::remove_file(&pid_file).unwrap();
    std::fs::remove_file(GROUP).unwrap();
    assert_eq!(confirmation::nginx_running(&config).await.unwrap(), NginxState::Starting);
    let restore_records = async {
        tokio::time::sleep(Duration::from_millis(150)).await;
        std::fs::write(&pid_file, &live_pid).unwrap();
        std::fs::write(GROUP, &live_pid).unwrap();
    };
    let (observed, ()) = tokio::join!(
        confirmation::observe_current(&config, Instant::now() + Duration::from_secs(5)),
        restore_records,
    );
    observed.unwrap();

    let foreign_conf = root.path().join("foreign.conf");
    let foreign_pid = root.path().join("foreign.pid");
    std::fs::write(&foreign_conf, format!(
        "pid {}; error_log {} notice; events {{}}",
        foreign_pid.display(), root.path().join("foreign.log").display(),
    )).unwrap();
    let other = Process(Command::new(&config.nginx_bin).arg("-c").arg(&foreign_conf)
        .args(["-g", "daemon off;"]).process_group(0).spawn().unwrap());
    wait_native(|| foreign_pid.exists()).await;
    for record in [&pid_file, Path::new(GROUP)] {
        std::fs::write(record, other.0.id().to_string()).unwrap();
    }
    assert!(confirmation::nginx_running(&config).await.unwrap_err().is::<confirmation::IdentityUncertain>());
    assert_eq!(unsafe { libc::kill(other.0.id() as i32, 0) }, 0);
    assert_eq!(unsafe { libc::kill(foreign.0.id() as i32, 0) }, 0);
    // Give the managed nginx its original PID file before s6 shuts it down.
    std::fs::write(&pid_file, &live_pid).unwrap();
    std::fs::write(GROUP, live_pid).unwrap();
    drop(supervisor);
    std::fs::remove_file(GROUP).unwrap();
}


#[test]
fn g35_marker_first_blocks_a_publisher_but_exempts_only_the_waiting_guard_parent() {
    let root=tempfile::tempdir().unwrap();let script=root.path().join("nginx-run");let parent=shell(&script);
    let marker=root.path().join("nginx-process-group");std::fs::write(&marker,parent.0.id().to_string()).unwrap();let pid=root.path().join("nginx.pid");
    let mut child=Command::new("/bin/true").spawn().unwrap();let dead=child.id();child.wait().unwrap();
    for bytes in [None,Some(Vec::new()),Some(format!("{dead}\n").into_bytes())] {
        if let Some(bytes)=bytes{std::fs::write(&pid,bytes).unwrap();}
        assert_eq!(nginx_state_with(&pid,&marker,Path::new("/bin/true"),&script,None).unwrap(),NginxState::Starting);
        assert_eq!(nginx_state_with(&pid,&marker,Path::new("/bin/true"),&script,Some(parent.0.id())).unwrap(),NginxState::Stopped);
        assert_eq!(nginx_state_with(&pid,&marker,Path::new("/bin/true"),&script,Some(std::process::id())).unwrap(),NginxState::Starting);
    }
}
#[test]
fn g35_relative_run_script_is_resolved_in_the_run_shells_cwd() {
    let root=tempfile::tempdir().unwrap();let script=root.path().join("run");let ack=root.path().join("ready");
    std::fs::write(&script,": > ready\nwhile :; do sleep 1; done\n").unwrap();
    let process=Process(Command::new("/bin/sh").arg("./run").current_dir(root.path()).process_group(0).spawn().unwrap());
    let deadline=std::time::Instant::now()+std::time::Duration::from_secs(2);
    while !ack.exists(){assert!(std::time::Instant::now()<deadline);std::thread::sleep(std::time::Duration::from_millis(1));}
    let marker=root.path().join("group");std::fs::write(&marker,process.0.id().to_string()).unwrap();
    assert_eq!(nginx_state_with(&root.path().join("pid"),&marker,Path::new("/bin/true"),&script,None).unwrap(),NginxState::Starting);
    assert_eq!(nginx_state_with(&root.path().join("pid"),&marker,Path::new("/bin/true"),&script,Some(process.0.id())).unwrap(),NginxState::Stopped);
}

#[test]
fn g35_admitted_executable_is_starting_and_dead_or_recycled_marker_is_stopped() {
    let root=tempfile::tempdir().unwrap();let pid=root.path().join("nginx.pid");let marker=root.path().join("nginx-process-group");
    let mut old=Command::new("/bin/true").spawn().unwrap();let old_pid=old.id();old.wait().unwrap();std::fs::write(&pid,old_pid.to_string()).unwrap();
    let master = super::identity_fixture::FakeMaster::spawn(&root.path().join("standalone.pid"), false);
    std::fs::write(&marker,master.pid().to_string()).unwrap();
    assert_eq!(nginx_state_with(&pid,&marker,Path::new("/bin/bash"),Path::new("/unused/run"),None).unwrap(),NginxState::Starting);
    assert_eq!(nginx_state_with(&pid,&marker,Path::new("/bin/true"),Path::new("/unused/run"),None).unwrap(),NginxState::Stopped);
    let mut child=Command::new("/bin/true").spawn().unwrap();let dead=child.id();child.wait().unwrap();std::fs::write(&marker,dead.to_string()).unwrap();
    assert_eq!(nginx_state_with(&pid,&marker,Path::new("/bin/true"),Path::new("/unused/run"),None).unwrap(),NginxState::Stopped);
}

#[test]
fn g35_run_script_records_its_identity_before_guard_without_changing_guard_umask() {
    use std::os::unix::fs::PermissionsExt;
    let root=tempfile::tempdir().unwrap();let script=root.path().join("run");let runtime=root.path().join("runtime");let log=root.path().join("calls");
    let gate=root.path().join("gate.sh");std::fs::write(&gate,"nwui_gate_longrun() { :; }\n").unwrap();
    let guard=root.path().join("guard");
    std::fs::write(&guard,format!("#!/bin/sh\ntest \"$(cat '{}')\" = \"$PPID\" || exit 75\nprintf 'guard %s ' \"$PPID\" >> '{}'\numask >> '{}'\nexec \"$G35_TEST_EXE\" --exact publisher::nginx_start_tests::g35_waiting_guard_child_is_exempt --ignored --nocapture > '{}' 2>&1\n",runtime.join("nginx-process-group").display(),log.display(),log.display(),root.path().join("guard-child.log").display())).unwrap();
    std::fs::set_permissions(&guard,std::fs::Permissions::from_mode(0o700)).unwrap();
    let nginx=root.path().join("nginx");std::fs::write(&nginx,format!("#!/bin/sh\nprintf 'nginx %s ' \"$$\" >> '{}'\numask >> '{}'\n",log.display(),log.display())).unwrap();std::fs::set_permissions(&nginx,std::fs::Permissions::from_mode(0o700)).unwrap();
    let text=include_str!("../../../../docker/s6-rc.d/nginx/run").replace("/etc/s6-overlay/scripts/nwui-gate.sh",&gate.display().to_string()).replace("/usr/local/bin/nwui-api",&guard.display().to_string()).replace("/run/nginx-web-ui",&runtime.display().to_string());
    std::fs::write(&script,text).unwrap();
    let output=Command::new("/bin/sh").arg("-c").arg("umask 022; exec /bin/sh \"$1\"").arg("fixture").arg(&script).env_clear().env("PATH",format!("{}:/usr/bin:/bin",root.path().display())).env("G35_TEST_EXE",std::env::current_exe().unwrap()).env("G35_RUN_SCRIPT",&script).env("G35_GROUP",runtime.join("nginx-process-group")).env("G35_PID",root.path().join("nginx.pid")).output().unwrap();
    assert!(output.status.success(),"{output:?}; child: {}",std::fs::read_to_string(root.path().join("guard-child.log")).unwrap_or_default());
    let pid=std::fs::read_to_string(runtime.join("nginx-process-group")).unwrap();let lines=std::fs::read_to_string(log).unwrap();
    assert_eq!(lines,format!("guard {} 0022\nnginx {} 0077\n",pid.trim(),pid.trim()));
    assert_eq!(std::fs::metadata(runtime.join("nginx-process-group")).unwrap().permissions().mode()&0o777,0o600);
}

#[tokio::test]
#[ignore = "disposable real-nginx image; requires NWUI_G35_STOP_LIBRARY"]
async fn g35_real_nginx_start_overlap_refuses_writes_then_uses_running_recovery() {
    use super::{confirmation,recovery,files,journal::{Operation,Phase},store::Origin};
    use std::{collections::BTreeMap,sync::Arc,time::Duration};
    let Ok(preload)=std::env::var("NWUI_G35_STOP_LIBRARY") else {eprintln!("G35 real-nginx fixture not selected; no native qualification");return;};
    async fn wait_for(mut predicate:impl FnMut()->bool) {
        tokio::time::timeout(Duration::from_secs(10),async{while !predicate(){tokio::time::sleep(Duration::from_millis(10)).await;}}).await.expect("real nginx did not reach the acknowledged PID-write boundary");
    }
    fn port()->u16{std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()}
    let (_state_root,mut state,_,_)=super::adoption::tests::fixture();
    let(root,mut config,store,_,_)=recovery::tests::durable_fixture();
    config.nginx_bin="/usr/sbin/nginx".into();config.nginx_main_conf="/etc/nginx/nginx.conf".into();
    let status=port();config.nginx_status_url=format!("http://127.0.0.3:{status}/nginx_status");
    let pid=root.path().join("nginx.pid");let marker=Path::new("/run/nginx-web-ui/nginx-process-group");std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    let main=format!("pid {}; error_log {} notice; events {{}} http {{ access_log off; server {{ listen 127.0.0.3:{status}; location = /nginx_status {{ stub_status; allow 127.0.0.3; deny all; include {}; }} }} include {}/*.conf; }} stream {{ include {}/*.conf; }}\n",pid.display(),root.path().join("error.log").display(),config.nginx_conf_dir.join(".nwui-generation").display(),config.nginx_conf_dir.display(),config.nginx_stream_dir.display());
    std::fs::write(&config.nginx_main_conf,main).unwrap();
    let old=uuid::Uuid::new_v4().to_string();let next=uuid::Uuid::new_v4().to_string();let http=port();let mut stream=port();while stream==http{stream=port();}
    let previous=BTreeMap::from([("conf/server-fixture.conf".into(),format!("server {{ listen 127.0.0.1:{http}; return 200 old; }}\n").into_bytes()),("stream/stream-fixture.conf".into(),format!("server {{ listen 127.0.0.1:{stream}; proxy_pass 127.0.0.1:9; }}\n").into_bytes())]);
    store.stage(&old,Origin::Publisher,None,BTreeMap::new(),previous.clone()).unwrap();store.stage(&next,Origin::Publisher,None,BTreeMap::new(),BTreeMap::new()).unwrap();store.select(&old).unwrap();
    files::restore_payload_files(&config,&previous).await.unwrap();confirmation::publish(&config,&uuid::Uuid::new_v4().to_string()).await.unwrap();
    let old_master=Process(Command::new(&config.nginx_bin).args(["-g","daemon off;"]).process_group(0).spawn().unwrap());
    wait_for(||std::fs::read_to_string(&pid).is_ok_and(|value|value.trim()==old_master.0.id().to_string())).await;
    let dead_pid=old_master.0.id();drop(old_master);
    assert_eq!(std::fs::read_to_string(&pid).unwrap().trim(),dead_pid.to_string(),"SIGKILL must leave the stale, nonempty PID fixture");
    let operation=recovery::tests::unfinished(&store,&old,&next,Phase::LiveWriting);
    let ack=root.path().join("pid-write-ack");
    let run=Path::new("/etc/s6-overlay/s6-rc.d/nginx/run");
    let gate=root.path().join("gate.sh");std::fs::write(&gate,"nwui_gate_longrun() { :; }\n").unwrap();
    let guard=root.path().join("guard");
    std::fs::write(&guard,format!("#!/bin/sh\nexec '{}' --exact publisher::nginx_start_tests::g35_native_guard_restores_for_its_waiting_parent --ignored --nocapture > '{}' 2>&1\n",std::env::current_exe().unwrap().display(),root.path().join("guard.log").display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&guard,std::fs::Permissions::from_mode(0o700)).unwrap();
    let text=include_str!("../../../../docker/s6-rc.d/nginx/run").replace("/etc/s6-overlay/scripts/nwui-gate.sh",&gate.display().to_string()).replace("/usr/local/bin/nwui-api",&guard.display().to_string());
    std::fs::write(run,text).unwrap();
    let master = Supervision::start("#!/bin/sh\nexec /bin/sh /etc/s6-overlay/s6-rc.d/nginx/run\n", false, |command| {
        command.env("LD_PRELOAD",preload).env("NWUI_G35_PIDFILE",&pid).env("NWUI_G35_ACK",&ack)
        .env("G35_NATIVE_STORE",store.root.parent().unwrap()).env("G35_GUARD_EXIT_ACK",root.path().join("guard-exit-ack")).env("G35_NATIVE_OP",&operation.id)
        .env("DATABASE_URL","postgres://fixture@127.0.0.1/fixture").env("AUTH_SECRET","g35-synthetic-fixture")
        .env("NGINX_BIN",&config.nginx_bin).env("NGINX_CONF",&config.nginx_main_conf).env("NGINX_CONF_D",&config.nginx_conf_dir).env("NGINX_STREAM_D",&config.nginx_stream_dir)
        .env("CERTS_DIR",&config.certs_dir).env("NGINX_GUARD_QUARANTINE",&config.nginx_quarantine_dir).env("NGINX_STATUS_URL",&config.nginx_status_url);
    });
    let guard_ack=root.path().join("guard-exit-ack");
    wait_for(||{let log=std::fs::read_to_string(root.path().join("guard.log")).unwrap_or_default();guard_ack.exists()&&log.contains("1 passed; 0 failed")||log.contains("FAILED")}).await;
    let master_pid: u32 = std::fs::read_to_string(marker).unwrap().trim().parse().unwrap();
    assert!(guard_ack.exists(),"guard failed: {}",std::fs::read_to_string(root.path().join("guard.log")).unwrap_or_default());
    assert_eq!(Operation::load(&store,&operation.id).unwrap().phase,Phase::RestoreValidated,"the real guard child must restore before its waiting parent execs nginx");
    assert!(std::fs::read_to_string(root.path().join("guard.log")).unwrap().contains("1 passed; 0 failed"));
    assert_eq!(std::fs::read_to_string(&pid).unwrap().trim(),dead_pid.to_string());assert_eq!(confirmation::nginx_running(&config).await.unwrap(),NginxState::Starting);
    let before=files::capture_payload_files(&config).await.unwrap();let manifest=serde_json::to_value(store.manifest().unwrap()).unwrap();let op=serde_json::to_value(Operation::load(&store,&operation.id).unwrap()).unwrap();
    state.config=Arc::new(config.clone());
    assert_eq!(super::coordinator::recover_before_readiness(&state,&store).await.unwrap_err().code,"nginx_starting");
    assert!(recovery::before_start(&config,&store).await.is_err());assert!(confirmation::ensure_nginx_stopped(&config).await.is_err());
    assert!(super::boot::publish_live(&config,&store,&next).await.is_err());
    assert_eq!(files::capture_payload_files(&config).await.unwrap(),before);assert_eq!(serde_json::to_value(store.manifest().unwrap()).unwrap(),manifest);assert_eq!(serde_json::to_value(Operation::load(&store,&operation.id).unwrap()).unwrap(),op);
    let mut pipe=[0;2];assert_eq!(unsafe{libc::pipe2(pipe.as_mut_ptr(),libc::O_CLOEXEC|libc::O_NONBLOCK)},0);
    use std::os::fd::{FromRawFd,AsRawFd};
    let mut ready=unsafe{std::fs::File::from_raw_fd(pipe[0])};let writer=unsafe{std::fs::File::from_raw_fd(pipe[1])};
    let key=root.path().join("publisher-key");std::fs::write(&key,"A5".repeat(32)).unwrap();std::fs::set_permissions(&key,std::fs::Permissions::from_mode(0o600)).unwrap();
    let root_log=root.path().join("publisher.log");let log=std::fs::File::create(&root_log).unwrap();
    let mut command=Command::new(std::env::current_exe().unwrap());
    command.args(["--exact","publisher::nginx_start_tests::g35_native_publisher_initial_recovery","--ignored","--nocapture"])
        .env("NWUI_TEST_PUBLISHER_ROOT",store.root.parent().unwrap()).env("G35_NATIVE_PUBLISHER","1").env("DATA_KEY_FILE",&key)
        .env("DATABASE_URL","postgres://fixture@127.0.0.1/fixture").env("AUTH_SECRET","g35-synthetic-fixture")
        .env("NGINX_BIN",&config.nginx_bin).env("NGINX_CONF",&config.nginx_main_conf).env("NGINX_CONF_D",&config.nginx_conf_dir).env("NGINX_STREAM_D",&config.nginx_stream_dir)
        .env("CERTS_DIR",&config.certs_dir).env("NGINX_GUARD_QUARANTINE",&config.nginx_quarantine_dir).env("NGINX_STATUS_URL",&config.nginx_status_url)
        .stdout(log.try_clone().unwrap()).stderr(log).process_group(0);
    let fd=writer.as_raw_fd();unsafe{command.pre_exec(move||{if libc::dup2(fd,3)<0{return Err(std::io::Error::last_os_error());}Ok(())});}
    let publisher=Process(command.spawn().unwrap());drop(writer);
    wait_for(||std::fs::read_to_string(&root_log).unwrap_or_default().contains("nginx 시작을 기다리고 있습니다. 자동으로 다시 확인합니다.")).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let diagnostics=std::fs::read_to_string(&root_log).unwrap();
    assert_eq!(diagnostics.matches("nginx 시작을 기다리고 있습니다. 자동으로 다시 확인합니다.").count(),1);assert!(!diagnostics.contains("[publisher] recovery failed:"));assert!(!diagnostics.contains("postgres://"));
    use std::io::Read;let mut byte=[0];assert_eq!(ready.read(&mut byte).unwrap_err().kind(),std::io::ErrorKind::WouldBlock,"publisher must not notify while nginx is Starting");
    assert_eq!(unsafe{libc::kill(publisher.0.id()as i32,0)},0);assert_eq!(files::capture_payload_files(&config).await.unwrap(),before);assert_eq!(serde_json::to_value(Operation::load(&store,&operation.id).unwrap()).unwrap(),op);
    assert_eq!(unsafe{libc::kill(master_pid as i32,libc::SIGCONT)},0);
    wait_for(||ack.exists()).await;
    assert!(std::fs::read(&pid).unwrap().is_empty());assert_eq!(confirmation::nginx_running(&config).await.unwrap(),NginxState::Starting);
    assert_eq!(ready.read(&mut byte).unwrap_err().kind(),std::io::ErrorKind::WouldBlock);
    assert_eq!(files::capture_payload_files(&config).await.unwrap(),before);assert_eq!(serde_json::to_value(Operation::load(&store,&operation.id).unwrap()).unwrap(),op);
    assert_eq!(unsafe{libc::kill(master_pid as i32,libc::SIGCONT)},0);
    wait_for(||std::fs::read_to_string(&pid).is_ok_and(|s|s.trim()==master_pid.to_string())).await;
    assert_eq!(confirmation::nginx_running(&config).await.unwrap(),NginxState::Running);
    wait_for(||match ready.read(&mut byte){Ok(1)=>true,Err(error)if error.kind()==std::io::ErrorKind::WouldBlock=>false,other=>panic!("publisher readiness failed: {other:?}; {}",std::fs::read_to_string(&root_log).unwrap_or_default())}).await;
    assert_eq!(byte,[b'\n']);
    assert!(Operation::load(&store,&operation.id).unwrap().terminal());assert_eq!(files::capture_payload_files(&config).await.unwrap(),previous);
    drop(publisher);drop(master);std::fs::remove_file(marker).unwrap();
}

#[test]
#[ignore = "subprocess of G35 run-script fixture"]
fn g35_waiting_guard_child_is_exempt() {
    let Ok(script)=std::env::var("G35_RUN_SCRIPT") else {eprintln!("G35 guard subprocess fixture not selected");return;};
    let group=std::env::var("G35_GROUP").unwrap();let pid=std::env::var("G35_PID").unwrap();
    assert_eq!(nginx_state_with(Path::new(&pid),Path::new(&group),Path::new("/bin/true"),Path::new(&script),Some(unsafe{libc::getppid()}as u32)).unwrap(),NginxState::Stopped);
    assert_eq!(nginx_state_with(Path::new(&pid),Path::new(&group),Path::new("/bin/true"),Path::new(&script),None).unwrap(),NginxState::Starting);
}

#[tokio::test]
#[ignore = "subprocess of G35 native nginx/run fixture"]
async fn g35_native_guard_restores_for_its_waiting_parent() {
    let Ok(root)=std::env::var("G35_NATIVE_STORE") else {eprintln!("G35 native guard fixture not selected");return;};
    let config=crate::config::RuntimeConfig::read(|key|std::env::var(key).ok()).unwrap();
    let store=super::store::PublisherStore::open_test_root(Path::new(&root)).unwrap();
    assert_eq!(super::confirmation::nginx_running(&config).await.unwrap(),NginxState::Starting);
    assert_eq!(super::confirmation::nginx_state(&config,Some(unsafe{libc::getppid()}as u32)).await.unwrap(),NginxState::Stopped);
    super::boot::guard(&config,&store).await.unwrap();
    let id=std::env::var("G35_NATIVE_OP").unwrap();
    assert_eq!(super::journal::Operation::load(&store,&id).unwrap().phase,super::journal::Phase::RestoreValidated);
    std::fs::write(std::env::var("G35_GUARD_EXIT_ACK").unwrap(),b"guard released locks").unwrap();
    assert_eq!(unsafe{libc::kill(libc::getppid(),libc::SIGSTOP)},0);
}

#[tokio::test]
#[ignore = "fresh publisher process in G35 native startup overlap fixture"]
async fn g35_native_publisher_initial_recovery() {
    if std::env::var_os("G35_NATIVE_PUBLISHER").is_none(){eprintln!("G35 publisher subprocess fixture not selected");return;}
    let readiness=super::coordinator::Readiness::capture().unwrap();
    let config=crate::config::RuntimeConfig::read(|key|std::env::var(key).ok()).unwrap();
    super::coordinator::run(config,readiness).await.unwrap();
}

#[test]
fn g35_empty_or_partial_group_marker_is_stale_but_io_errors_still_refuse() {
    let root=tempfile::tempdir().unwrap();let pid=root.path().join("pid");let group=root.path().join("group");
    let mut child=Command::new("/bin/true").spawn().unwrap();let dead=child.id();child.wait().unwrap();
    for pid_bytes in [None,Some(Vec::new()),Some(dead.to_string().into_bytes())] {
        if let Some(bytes)=pid_bytes{std::fs::write(&pid,bytes).unwrap();}
        for content in ["","\n","12x","1"] {
            std::fs::write(&group,content).unwrap();
            assert_eq!(nginx_state_with(&pid,&group,Path::new("/bin/true"),Path::new("/unused/run"),None).unwrap(),NginxState::Stopped);
        }
    }
    std::fs::remove_file(&group).unwrap();std::fs::create_dir(&group).unwrap();
    assert!(nginx_state_with(&pid,&group,Path::new("/bin/true"),Path::new("/unused/run"),None).is_err());
}

fn copied_run_script()->(tempfile::TempDir,std::path::PathBuf,std::path::PathBuf,std::path::PathBuf,Process) {
    use std::os::unix::fs::PermissionsExt;
    let root=tempfile::tempdir().unwrap();let source=root.path().join("source-run");let copy=root.path().join("runtime-run");let ack=root.path().join("copy-ready");
    std::fs::write(&source,": > \"$G35_SCRIPT_ACK\"\nwhile :; do sleep 1; done\n").unwrap();std::fs::copy(&source,&copy).unwrap();std::fs::set_permissions(&source,std::fs::Permissions::from_mode(0o644)).unwrap();std::fs::set_permissions(&copy,std::fs::Permissions::from_mode(0o644)).unwrap();
    let process=Process(Command::new("/bin/sh").arg(&copy).env("G35_SCRIPT_ACK",&ack).process_group(0).spawn().unwrap());
    let deadline=std::time::Instant::now()+std::time::Duration::from_secs(2);while !ack.exists(){assert!(std::time::Instant::now()<deadline);std::thread::sleep(std::time::Duration::from_millis(1));}
    let group=root.path().join("group");std::fs::write(&group,process.0.id().to_string()).unwrap();
    (root,source,copy,group,process)
}
#[test]
fn g35_root_owned_copied_unit_has_positive_identity_but_modified_or_writable_units_do_not() {
    use std::os::unix::fs::{MetadataExt,PermissionsExt,chown};
    let(root,source,copy,group,process)=copied_run_script();let pid=root.path().join("pid");let master=Path::new("/bin/true");
    assert_ne!(std::fs::metadata(&source).unwrap().ino(),std::fs::metadata(&copy).unwrap().ino());
    assert_eq!(nginx_state_with(&pid,&group,master,&source,None).unwrap(),NginxState::Starting);
    assert_eq!(nginx_state_with(&pid,&group,master,&source,Some(process.0.id())).unwrap(),NginxState::Stopped);
    let original=std::fs::read(&source).unwrap();let mut changed=original.clone();changed.extend_from_slice(b"# different\n");std::fs::write(&copy,&changed).unwrap();
    assert_eq!(nginx_state_with(&pid,&group,master,&source,None).unwrap(),NginxState::Stopped);
    std::fs::write(&copy,&original).unwrap();std::fs::set_permissions(&copy,std::fs::Permissions::from_mode(0o664)).unwrap();
    assert_eq!(nginx_state_with(&pid,&group,master,&source,None).unwrap(),NginxState::Stopped);
    std::fs::set_permissions(&copy,std::fs::Permissions::from_mode(0o644)).unwrap();
    for uid in [1000,10001] {chown(&copy,Some(uid),Some(0)).unwrap();assert_eq!(nginx_state_with(&pid,&group,master,&source,None).unwrap(),NginxState::Stopped);}
    chown(&copy,Some(0),Some(0)).unwrap();assert_eq!(nginx_state_with(&pid,&group,master,&source,None).unwrap(),NginxState::Starting);
}
#[test]
fn g35_symlinked_script_argument_is_not_an_identified_run_shell() {
    use std::os::unix::fs::symlink;
    let(root,source,copy,_group,_process)=copied_run_script();std::fs::remove_file(&copy).unwrap();symlink(&source,&copy).unwrap();
    let ack=root.path().join("symlink-ready");let child=Process(Command::new("/bin/sh").arg(&copy).env("G35_SCRIPT_ACK",&ack).process_group(0).spawn().unwrap());
    let deadline=std::time::Instant::now()+std::time::Duration::from_secs(2);while !ack.exists(){assert!(std::time::Instant::now()<deadline);std::thread::sleep(std::time::Duration::from_millis(1));}
    let group=root.path().join("symlink-group");std::fs::write(&group,child.0.id().to_string()).unwrap();
    assert_eq!(nginx_state_with(&root.path().join("pid"),&group,Path::new("/bin/true"),&source,None).unwrap(),NginxState::Stopped);
}

#[tokio::test]
#[ignore = "G35 non-child evaluator in the merged s6-supervise image; requires NWUI_G35_EXPECT_STATE"]
async fn g35_merged_image_nonchild_evaluator() {
    let Ok(expected)=std::env::var("NWUI_G35_EXPECT_STATE") else {eprintln!("G35 merged s6-supervise image fixture not selected; no real-image qualification");return;};
    let mut config=crate::config::RuntimeConfig::from_env().unwrap();let admitted=crate::nonroot::admit_nginx(&config).await.unwrap();config.nginx_bin=admitted.control_bin;config.nginx_main_conf=admitted.conf_path;
    let before=super::files::capture_payload_files(&config).await.unwrap();
    let state=super::confirmation::nginx_running(&config).await.unwrap();
    let expected=match expected.as_str(){"Starting"=>NginxState::Starting,"Running"=>NginxState::Running,"Stopped"=>NginxState::Stopped,_=>panic!("invalid test state")};
    assert_eq!(state,expected);if state==NginxState::Starting {assert!(super::confirmation::ensure_nginx_stopped(&config).await.is_err());}
    assert!(before==super::files::capture_payload_files(&config).await.unwrap(),"non-child state evaluator changed live bytes");
    eprintln!("G35_STATE={state:?}; live bytes unchanged");
}

#[test]
fn g35_oversize_identical_prefix_and_oversize_expected_script_are_stopped() {
    let(root,source,copy,group,_process)=copied_run_script();
    let mut bytes=std::fs::read(&source).unwrap();bytes.resize(64*1024,b' ');
    std::fs::write(&source,&bytes).unwrap();bytes.push(b' ');std::fs::write(&copy,&bytes).unwrap();
    assert_eq!(nginx_state_with(&root.path().join("pid"),&group,Path::new("/bin/true"),&source,None).unwrap(),NginxState::Stopped,"an identical 64 KiB prefix must not hide the extra byte");
    std::fs::write(&source,&bytes).unwrap();
    assert_eq!(nginx_state_with(&root.path().join("pid"),&group,Path::new("/bin/true"),&source,None).unwrap(),NginxState::Stopped,"the expected script has the same independent cap");
}
