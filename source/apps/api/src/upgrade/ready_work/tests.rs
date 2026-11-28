use super::*;
use std::{fs, os::unix::fs::PermissionsExt};

fn fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o755)).unwrap();
    temp
}

fn make_files(work: &ReadyWork) -> postgres::ReadyFiles {
    [c"private.conf",c"hba.conf",c"ident.conf",c"server.stdout",c"server.stderr"].map(|name| {
        let file = File::options().create_new(true).write(true).mode(0o600)
            .open(work.path.join(name.to_str().unwrap())).unwrap();
        (name,file)
    })
}

#[test]
fn cleanup_checks_the_creation_fds_before_touching_any_replaced_name() {
    let temp = fixture();
    let work = ReadyWork::create(temp.path()).unwrap();
    let files = make_files(&work);
    assert_eq!(fs::metadata(&work.path).unwrap().uid(),0);
    assert_eq!(fs::metadata(&work.path).unwrap().mode() & 0o7777,0o710);

    fs::rename(work.path.join("server.stderr"), temp.path().join("original")).unwrap();
    fs::write(work.path.join("server.stderr"), b"operator copy").unwrap();
    assert!(work.remove(files).is_err());
    assert!(work.path.join("private.conf").exists());
    assert_eq!(fs::read(work.path.join("server.stderr")).unwrap(), b"operator copy");
}

#[test]
fn cleanup_never_reaches_another_directory_or_unknown_leaf() {
    let temp = fixture();
    let work = ReadyWork::create(temp.path()).unwrap();
    let files = make_files(&work);
    let foreign = temp.path().join("nwui-ready-operator-copy");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("private.conf"), b"keep").unwrap();
    fs::write(work.path.join("unrecognized"), b"keep").unwrap();
    assert!(work.remove(files).is_err());
    assert_eq!(fs::read(foreign.join("private.conf")).unwrap(), b"keep");
    assert_eq!(fs::read(work.path.join("unrecognized")).unwrap(), b"keep");
}

#[test]
#[ignore = "native PG17, run with NWUI_TEST_C3_READY_WORK=1 in a disposable container"]
fn native_ready_work_keeps_failures_cleans_success_and_does_not_block_postgres() {
    use crate::db_upgrade::layout::Roots;
    use std::{process::Command,time::{Duration,Instant}};
    if std::env::var("NWUI_TEST_C3_READY_WORK").as_deref() != Ok("1") { return; }
    const CHILD: &str = "NWUI_C3_READY_WORK_CHILD";
    let roots_at = |path: &Path| Roots { db: path.join("db"), app: path.join("app"), architecture: "arm64".into() };
    if let Some(path) = std::env::var_os(CHILD).map(PathBuf::from) {
        let roots = roots_at(&path);
        let locks = super::super::StorageLocks::acquire(&roots.db,&roots.app).unwrap();
        let work = ReadyWork::create(&path).unwrap();
        let pg = postgres::Private::start_locked_ready(&locks,&roots,&work.path).unwrap();
        fs::write(path.join("orphan"),pg.socket.as_os_str().as_encoded_bytes()).unwrap();
        std::hint::black_box((&pg,&locks));
        std::thread::park();
        unreachable!("the parent kills this runner");
    }
    let temp = fixture();
    let roots = roots_at(temp.path());
    fs::create_dir_all(roots.db.join("17")).unwrap();
    fs::create_dir(&roots.app).unwrap();
    let data = roots.db.join("17/data");
    postgres::directory(&data).unwrap();
    let init = temp.path().join("init");
    postgres::directory(&init).unwrap();
    let mut command = postgres::command(17,"initdb").unwrap();
    command.arg("-D").arg(&data).args(["--locale=C","--encoding=UTF8","--auth-local=trust","--auth-host=scram-sha-256"]);
    postgres::capture(command,&init,Duration::from_secs(60)).unwrap();
    let locks = super::super::StorageLocks::acquire(&roots.db,&roots.app).unwrap();
    let mut stale_pid = None;
    let mut kept = vec![];
    for success in [true,true,true,true,true,false,false,false] {
        let work = ReadyWork::create(temp.path()).unwrap();
        let path = work.path.clone();
        let pg = postgres::Private::start_locked_ready(&locks,&roots,&path).unwrap();
        let mut query = postgres::command(17,"psql").unwrap();
        query.arg("-h").arg(&pg.socket).args(["-p","50432","-U","postgres","-v","ON_ERROR_STOP=1","-Atqc",
            if success { "SELECT 17" } else { "SELECT missing_column" }]);
        assert_eq!(postgres::output(query,Duration::from_secs(5)).is_ok(),success);
        stale_pid.get_or_insert_with(|| fs::read_to_string(data.join("postmaster.pid")).unwrap());
        let files = pg.stop_ready().unwrap();
        if success {
            work.clean_success(files);
            assert!(!path.exists());
        } else {
            assert!(path.join("server.stderr").exists());
            kept.push(path);
        }
        let count=fs::read_dir(temp.path()).unwrap().filter(|entry|entry.as_ref().unwrap().file_name().to_string_lossy().starts_with("nwui-ready-")).count();
        assert_eq!(count,kept.len());
    }
    assert_eq!(kept.len(),3);

    let stale = stale_pid.unwrap();
    let (_,rest) = stale.split_once('\n').unwrap();
    postgres::private_file(&data.join("postmaster.pid"),format!("{}\n{rest}",std::process::id()).as_bytes()).unwrap();
    let unrelated = (0..4097).map(|n|Path::new("/run").join(format!("nwui-test-unrelated-{}-{n}",std::process::id()))).collect::<Vec<_>>();
    struct Unrelated(Vec<PathBuf>);
    impl Drop for Unrelated {
        fn drop(&mut self) { for path in &self.0 { let _=fs::remove_file(path); } }
    }
    let unrelated = Unrelated(unrelated);
    for path in &unrelated.0 { File::options().create_new(true).write(true).open(path).unwrap(); }
    let work = ReadyWork::create(Path::new("/run")).unwrap();
    let pg = postgres::Private::start_locked_ready(&locks,&roots,&work.path).unwrap();
    work.clean_success(pg.stop_ready().unwrap());
    drop(unrelated);
    drop(locks);

    let mut child=Command::new(std::env::current_exe().unwrap())
        .args(["--ignored","--exact","upgrade::ready_work::tests::native_ready_work_keeps_failures_cleans_success_and_does_not_block_postgres","--nocapture"])
        .env(CHILD,temp.path()).spawn().unwrap();
    let result=std::panic::catch_unwind(||{
        let deadline=Instant::now()+Duration::from_secs(30);
        let socket=loop {
            if let Ok(path)=fs::read_to_string(temp.path().join("orphan")) {
                break PathBuf::from(path);
            }
            assert!(Instant::now()<deadline,"private postmaster did not publish the socket path");
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGKILL) }, 0);
        let mut query=postgres::command(17,"psql").unwrap();
        query.arg("-h").arg(&socket).args(["-p","50432","-U","postgres","-Atqc","SELECT 17"]);
        assert_eq!(postgres::output(query,Duration::from_secs(5)).unwrap().0.trim(),"17");
        // An empty runtime folder isolates the PID proof from socket and lease evidence.
        let empty=temp.path().join("empty-run");
        fs::create_dir(&empty).unwrap();
        assert!(!postgres::private_quiescent(&data,&empty));
    });
    let _=child.kill();
    child.wait().unwrap();
    postgres::stop_working(17,&data).unwrap();
    if let Err(error)=result{std::panic::resume_unwind(error);}
}
