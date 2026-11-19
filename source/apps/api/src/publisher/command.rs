//! Bounded root-only nginx execution; caller retains the mutation locks until completion.
use anyhow::{Result, bail};
use std::{path::PathBuf, process::Stdio, time::Duration};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CommandResult { Accepted(String), Rejected(String), TimedOut }

pub(super) fn strict(result:CommandResult)->std::result::Result<String,crate::upgrade::StepError> {
    match result {
        CommandResult::Accepted(stderr)=>Ok(stderr),
        CommandResult::Rejected(stderr)=>Err(test_failed(&stderr)),
        CommandResult::TimedOut=>Err(root_error("nginx command timed out")),
    }
}
pub(super) fn test_failed(output:&str)->crate::upgrade::StepError {
    test_failed_in(output,&[])
}
pub(super) fn test_failed_in(output:&str,prefixes:&[(&std::path::Path,&std::path::Path)])->crate::upgrade::StepError {
    crate::upgrade::StepError {code:"nginx_test_failed",message:"nginx가 설정을 거부했습니다.",detail:Some(super::containment::reason(output,prefixes))}
}
pub(super) async fn validate_includes(
    store:super::store::PublisherStore,bin:PathBuf,main:Option<PathBuf>,timeout:Duration,
) -> std::result::Result<String,crate::upgrade::StepError> {
    validate_includes_in(store,bin,main,timeout,&[]).await
}
pub(super) async fn validate_includes_in(
    store:super::store::PublisherStore,bin:PathBuf,main:Option<PathBuf>,timeout:Duration,prefixes:&[(&std::path::Path,&std::path::Path)],
) -> std::result::Result<String,crate::upgrade::StepError> {
    match validate_candidate(store,bin,main,timeout).await? {
        CommandResult::Rejected(stderr)=>Err(test_failed_in(&stderr,prefixes)),
        result=>strict(result),
    }
}
pub(super) async fn validate_candidate(
    store:super::store::PublisherStore,bin:PathBuf,main:Option<PathBuf>,timeout:Duration,
) -> std::result::Result<CommandResult,crate::upgrade::StepError> {
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut args=vec![std::ffi::OsString::from("-T")];
        if let Some(main)=main {
            args.insert(0,std::ffi::OsString::from("-t"));
            args.extend([std::ffi::OsString::from("-c"),main.into_os_string()]);
        }
        root_capture(store,bin,args,timeout,|stdout| {
            super::trusted_includes::validate_dump(std::io::BufReader::new(stdout.take((super::trusted_includes::MAX_NGINX_T_OUTPUT as u64)+1)))
        })
    }).await.map_err(|_|root_error("nginx validation worker failed"))?
}

pub(super) async fn execute(store:super::store::PublisherStore,bin:PathBuf,args:Vec<std::ffi::OsString>,timeout:Duration)->std::result::Result<String,crate::upgrade::StepError> {
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let stdout=std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));let captured=stdout.clone();
        let result=root_capture(store,bin,args,timeout,move |pipe| {
            let mut bytes=Vec::new();pipe.take(65537).read_to_end(&mut bytes).map_err(|error|root_error(&error.to_string()))?;
            if bytes.len()>65536 {return Err(root_error("nginx stdout exceeds 64 KiB"));}
            *captured.lock().map_err(|_|root_error("nginx output worker failed"))?=bytes;Ok(true)
        });
        let stdout=stdout.lock().map_err(|_|root_error("nginx output worker failed"))?;
        let text=String::from_utf8_lossy(&stdout);
        let result=result.and_then(|result|match result {CommandResult::Accepted(stderr)=>Ok(stderr),CommandResult::Rejected(stderr)=>{let mut error=root_error("nginx command failed");error.detail=Some(stderr);Err(error)},CommandResult::TimedOut=>Err(root_error("nginx command timed out"))});
        match result {
            Ok(stderr)=>Ok(format!("{text}{stderr}")),
            Err(mut error)=>{if !text.is_empty() {error.detail=Some(format!("{text}{}",error.detail.as_deref().unwrap_or("")));}Err(error)},
        }
    }).await.map_err(|_|root_error("nginx command worker failed"))?
}

fn timeout_result(termination:anyhow::Result<()>)->std::result::Result<CommandResult,crate::upgrade::StepError> {
    termination.map(|_|CommandResult::TimedOut).map_err(|error|root_error(&error.to_string()))
}
fn root_error(detail:&str) -> crate::upgrade::StepError {
    crate::upgrade::StepError {code:"publish_failed",message:"nginx 설정 검증을 완료하지 못했습니다.",detail:Some(detail.chars().take(4096).collect())}
}

struct RootChild {
    child:Option<std::process::Child>,
    store:super::store::PublisherStore,
    pid:u32,
    ticks:u64,
}
impl RootChild {
    fn terminate(&mut self) -> Result<()> {
        let Some(child)=self.child.as_mut() else {return Ok(())};
        // The leader is not reaped while its pipes are still open, so its PGID cannot be reused.
        unsafe {libc::kill(-(self.pid as i32),libc::SIGKILL);}
        let deadline=std::time::Instant::now()+Duration::from_secs(2);
        loop {
            match child.try_wait() {
                Ok(Some(_))=>{self.child.take();return Ok(());}
                Ok(None) if std::time::Instant::now()<deadline=>std::thread::sleep(Duration::from_millis(10)),
                _=>break,
            }
        }
        // Persist the refusal before any caller can release its mutation lock.
        let poison=super::poison::mark(&self.store,self.pid,self.ticks);
        let mut child=self.child.take().unwrap();
        if let Err(error)=poison {
            // With no durable poison, keep the operation and its locks until the child is reaped.
            let _=child.wait();
            return Err(error);
        }
        std::thread::spawn(move || {let _=child.wait();});
        bail!("nginx child could not be reaped; publisher is poisoned")
    }
}
impl Drop for RootChild {
    fn drop(&mut self) {
        if let Err(error)=self.terminate() {eprintln!("[publisher] root child cleanup: {error:#}");}
    }
}

fn root_capture(
    store:super::store::PublisherStore,bin:PathBuf,args:Vec<std::ffi::OsString>,timeout:Duration,
    validate:impl FnOnce(std::process::ChildStdout)->std::result::Result<bool,crate::upgrade::StepError>+Send+'static,
) -> std::result::Result<CommandResult,crate::upgrade::StepError> {
    use std::{io::Read,os::unix::process::CommandExt,sync::mpsc,time::Instant};
    let deadline=Instant::now()+timeout;
    super::poison::check(&store).map_err(|error|root_error(&error.to_string()))?;
    // Refuse unsupported /proc before starting a child whose identity we cannot preserve.
    super::poison::start_ticks(std::process::id()).map_err(|error|root_error(&error.to_string()))?
        .ok_or_else(||root_error("publisher process identity unavailable"))?;
    let mut command=std::process::Command::new(bin);
    command.args(args).env_clear().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
    for key in ["PATH", "TZ"] {
        if let Some(value)=std::env::var_os(key) {command.env(key,value);}
    }
    #[cfg(target_os="linux")]
    unsafe {
        let parent=libc::getpid();
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG,libc::SIGKILL)!=0 {return Err(std::io::Error::last_os_error());}
            if libc::getppid()!=parent {return Err(std::io::Error::from_raw_os_error(libc::ECHILD));}
            Ok(())
        });
    }
    if Instant::now()>=deadline {return Ok(CommandResult::TimedOut);}
    let mut child=command.spawn().map_err(|error|root_error(&error.to_string()))?;
    let pid=child.id();
    let ticks=match super::poison::start_ticks(pid) {
        Ok(Some(ticks))=>ticks,
        _=>{
            unsafe {libc::kill(-(pid as i32),libc::SIGKILL);}
            let _=child.wait();
            return Err(root_error("nginx child identity unavailable"));
        }
    };
    let stdout=child.stdout.take().unwrap();let stderr=child.stderr.take().unwrap();
    let mut owned=RootChild {child:Some(child),store,pid,ticks};
    enum Captured {Out(std::result::Result<bool,crate::upgrade::StepError>),Err(std::io::Result<Vec<u8>>)}
    let (send,receive)=mpsc::channel();let out_send=send.clone();
    std::thread::spawn(move || {let _=out_send.send(Captured::Out(validate(stdout)));});
    std::thread::spawn(move || {
        let mut bytes=Vec::new();
        let result=stderr.take(65537).read_to_end(&mut bytes).and_then(|_| {
            if bytes.len()>65536 {Err(std::io::Error::other("nginx stderr exceeds 64 KiB"))} else {Ok(bytes)}
        });
        let _=send.send(Captured::Err(result));
    });
    let mut out_done=false;let mut admitted=false;let mut stderr=None;
    loop {
        if Instant::now()>=deadline {return timeout_result(owned.terminate());}
        match receive.recv_timeout(deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(10))) {
            Ok(Captured::Out(result))=>{admitted=result?;out_done=true;}
            Ok(Captured::Err(result))=>{stderr=Some(result.map_err(|error|root_error(&error.to_string()))?);}
            Err(mpsc::RecvTimeoutError::Disconnected) if !out_done || stderr.is_none()=>return Err(root_error("nginx output worker failed")),
            Err(mpsc::RecvTimeoutError::Disconnected)=>std::thread::sleep(Duration::from_millis(10)),
            Err(_)=>{},
        }
        if out_done && stderr.is_some() {
            #[cfg(target_os="linux")]
            let finished=unsafe {
                let mut info:libc::siginfo_t=std::mem::zeroed();
                if libc::waitid(libc::P_PID,owned.pid,&mut info,libc::WEXITED|libc::WNOHANG|libc::WNOWAIT)!=0 {
                    return Err(root_error(&std::io::Error::last_os_error().to_string()));
                }
                if info.si_pid()!=0 && info.si_code!=libc::CLD_EXITED {return Err(root_error("nginx command terminated by signal"));}
                (info.si_pid()!=0).then(||info.si_status()==0)
            };
            #[cfg(not(target_os="linux"))]
            let finished=owned.child.as_mut().unwrap().try_wait().map_err(|error|root_error(&error.to_string()))?.map(|status|status.code().map(|code|code==0).ok_or_else(||root_error("nginx command terminated by signal"))).transpose()?;
            if let Some(success)=finished {
                let mut text=String::from_utf8_lossy(&stderr.unwrap()).into_owned();
                if text.len()>65536 {
                    let mut end=65536;while !text.is_char_boundary(end) {end-=1;}text.truncate(end);
                }
                owned.child.as_mut().unwrap().wait().map_err(|error|root_error(&error.to_string()))?;
                owned.child.take();
                if !success {return Ok(CommandResult::Rejected(text));}
                if !admitted {return Err(super::trusted_includes::validate_stream(&b""[..]).unwrap_err());}
                return Ok(CommandResult::Accepted(text));
            }
        }
    }
}

#[cfg(all(test,target_os="linux"))]
mod root_stream_tests {
    use super::*;
    use std::{io,os::unix::fs::PermissionsExt,path::Path,sync::{Arc,atomic::{AtomicU64,Ordering}},time::Instant};

    fn fixture(body:&str) -> (tempfile::TempDir,super::super::store::PublisherStore,PathBuf) {
        let temp=tempfile::tempdir().unwrap();
        std::fs::set_permissions(temp.path(),std::fs::Permissions::from_mode(0o700)).unwrap();
        let bin=temp.path().join("nginx with spaces");
        std::fs::write(&bin,format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&bin,std::fs::Permissions::from_mode(0o700)).unwrap();
        let store=super::super::store::PublisherStore::at(temp.path().into());
        (temp,store,bin)
    }
    fn drain(mut stdout:std::process::ChildStdout) -> std::result::Result<bool,crate::upgrade::StepError> {
        io::copy(&mut stdout,&mut io::sink()).map_err(|error|root_error(&error.to_string()))?;Ok(true)
    }
    fn assert_reaped(pid_file:&Path) {
        let pid=std::fs::read_to_string(pid_file).unwrap().trim().parse().unwrap();
        assert!(super::super::poison::start_ticks(pid).unwrap().is_none(),"owned leader must be reaped");
    }

    #[test]
    fn child_environment_tz_probe() {
        let Ok(expected)=std::env::var("NWUI_EXPECT_TZ") else {return;};
        let (_temp,store,bin)=fixture("/usr/bin/env >&2");
        let result=root_capture(store,bin,vec![],Duration::from_secs(2),drain).unwrap();
        let CommandResult::Accepted(env)=result else {panic!("{result:?}")};
        let mut lines:Vec<_>=env.lines().collect(); lines.sort();
        // /bin/sh may add PWD; the captured executable must see no injected secret.
        assert!(!env.contains("NWUI_TEST_SECRET")); assert!(!env.contains("NWUI_EXPECT_TZ"));
        assert_eq!(env.lines().find_map(|line|line.strip_prefix("TZ=")), if expected=="<unset>" {None} else {Some(expected.as_str())});
        assert!(env.lines().any(|line|line.starts_with("PATH=")));
        assert!(lines.iter().all(|line|line.starts_with("PATH=")||line.starts_with("TZ=")||line.starts_with("PWD=")));
    }

    #[test]
    fn root_children_keep_only_path_and_optional_unmodified_tz() {
        for zone in [None,Some("Asia/Seoul"),Some(":KST-9"),Some("")] {
            let mut command=std::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact","publisher::command::root_stream_tests::child_environment_tz_probe","--nocapture"])
                .env_clear().env("PATH","/usr/bin:/bin").env("NWUI_TEST_SECRET","not-forwarded")
                .env("NWUI_EXPECT_TZ",zone.unwrap_or("<unset>"));
            if let Some(zone)=zone {command.env("TZ",zone);}
            let result=command.output().unwrap();
            assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stdout));
        }
    }

    #[tokio::test]
    async fn staged_and_live_include_checks_use_exact_nginx_arguments_without_prefix_override() {
        let (temp,store,bin)=fixture("exit 99");
        let main=temp.path().join("effective main");
        std::fs::write(&main,b"events {}\n").unwrap();
        std::fs::set_permissions(&main,std::fs::Permissions::from_mode(0o600)).unwrap();
        for staged in [true,false] {
            let check=if staged {r#"[ "$#" -eq 4 ] && [ "$1" = '-t' ] && [ "$2" = '-T' ] && [ "$3" = '-c' ]"#} else {r#"[ "$#" -eq 1 ] && [ "$1" = '-T' ]"#};
            std::fs::write(&bin,format!("#!/bin/sh\n{check} || exit 8\nprintf '# configuration file %s:\\n' '{}'\n",main.display())).unwrap();
            let result=validate_includes(store.clone(),bin.clone(),staged.then(||main.clone()),Duration::from_secs(2)).await;
            assert!(result.is_ok(),"staged={staged}: {result:?}");
        }
    }

    #[test]
    fn root_reader_streams_raw_stdout_and_bounds_diagnostics_without_shell_arguments() {
        let (temp,store,bin)=fixture(r#"[ -z "$HOME" ] || exit 9
[ "$#" -eq 3 ] && [ "$1" = '-T' ] && [ "$2" = '-c' ] || exit 8
if read value; then exit 10; fi
printf '\377body\n'
printf 'diagnostic\n' >&2"#);
        let count=Arc::new(AtomicU64::new(0));let observed=count.clone();
        let output=root_capture(store,bin,vec!["-T".into(),"-c".into(),temp.path().join("wrapper with spaces").into_os_string()],Duration::from_secs(2),move |mut stdout| {
            let bytes=io::copy(&mut stdout,&mut io::sink()).map_err(|error|root_error(&error.to_string()))?;
            observed.store(bytes,Ordering::Relaxed);Ok(true)
        }).unwrap();
        assert_eq!(count.load(Ordering::Relaxed),6);
        assert_eq!(output,CommandResult::Accepted("diagnostic\n".into()));
    }

    #[test]
    fn root_timeout_kills_the_group_and_reaps_its_owned_leader() {
        let (temp,store,bin)=fixture(r#"echo $$ > "$1"
sleep 60 &
wait"#);
        let pid=temp.path().join("pid");let started=Instant::now();
        let result=root_capture(store,bin,vec![pid.clone().into_os_string()],Duration::from_millis(500),drain).unwrap();
        assert_eq!(result,CommandResult::TimedOut);
        assert!(started.elapsed()<Duration::from_secs(5));
        assert_reaped(&pid);
    }

    #[test]
    fn root_stderr_overflow_refuses_and_reaps_the_child() {
        let (temp,store,bin)=fixture(r#"echo $$ > "$1"
head -c 65537 /dev/zero >&2
sleep 60"#);
        let pid=temp.path().join("pid");let started=Instant::now();
        let error=root_capture(store,bin,vec![pid.clone().into_os_string()],Duration::from_secs(2),drain).unwrap_err();
        assert!(error.detail.unwrap().contains("64 KiB"));
        assert!(started.elapsed()<Duration::from_secs(5));
        assert_reaped(&pid);
    }

    #[test]
    fn root_parser_refusal_keeps_its_error_and_reaps_before_returning() {
        let (temp,store,bin)=fixture(r#"echo $$ > "$1"
printf '# refused\n'
sleep 60"#);
        let pid=temp.path().join("pid");
        let error=root_capture(store,bin,vec![pid.clone().into_os_string()],Duration::from_secs(2),|mut stdout| {
            use std::io::Read;
            let mut byte=[0];stdout.read_exact(&mut byte).map_err(|error|root_error(&error.to_string()))?;
            Err(crate::upgrade::StepError {code:"trusted_input_unsafe",message:"fixture",detail:Some("/fixture/include".into())})
        }).unwrap_err();
        assert_eq!(error.code,"trusted_input_unsafe");assert_eq!(error.detail.as_deref(),Some("/fixture/include"));
        assert_reaped(&pid);
    }
}

#[cfg(all(test,target_os="linux"))]
mod classification_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    async fn check(stdout:&str,status:&str)->std::result::Result<CommandResult,crate::upgrade::StepError> {
        let temp=tempfile::tempdir().unwrap();let main=temp.path().join("main");let bin=temp.path().join("nginx");
        std::fs::write(&main,"events {}\n").unwrap();std::fs::set_permissions(&main,std::fs::Permissions::from_mode(0o600)).unwrap();
        let stdout=stdout.replace("{MAIN}",main.to_str().unwrap());
        std::fs::write(&bin,format!("#!/bin/sh\n{stdout}\nprintf 'nginx: [emerg] host not found in upstream \\\"absent-svc:80\\\"\\n' >&2\n{status}\n")).unwrap();
        std::fs::set_permissions(&bin,std::fs::Permissions::from_mode(0o700)).unwrap();
        validate_candidate(super::super::store::PublisherStore::at(temp.path().into()),bin,Some(main),Duration::from_secs(2)).await
    }
    #[tokio::test]
    async fn rejection_requires_normal_exit_and_empty_or_fully_admitted_stdout() {
        assert!(matches!(check("","exit 1").await.unwrap(),CommandResult::Rejected(_)));
        assert_eq!(check("","exit 0").await.unwrap_err().code,"trusted_input_unsafe");
        assert_eq!(check("printf 'malformed dump\\n'","exit 1").await.unwrap_err().code,"trusted_input_unsafe");
        assert_eq!(check("","kill -TERM $$").await.unwrap_err().code,"publish_failed");
        assert_eq!(strict(check("","exit 1").await.unwrap()).unwrap_err().code,"nginx_test_failed");
        assert_eq!(strict(CommandResult::TimedOut).unwrap_err().code,"publish_failed");
        assert!(matches!(check("printf '# configuration file %s:\\n' '{MAIN}'","exit 1").await.unwrap(),CommandResult::Rejected(_)));
        assert!(matches!(check("printf '# configuration file %s:\\n' '{MAIN}'","exit 0").await.unwrap(),CommandResult::Accepted(_)));
        assert_eq!(timeout_result(Err(anyhow::anyhow!("publisher is poisoned"))).unwrap_err().code,"publish_failed");
    }
}
