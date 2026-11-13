//! Root process startup resources; no key creation and no implicit readiness.
use crate::{config::RuntimeConfig,state::{AppState,RuntimeState},upgrade::StepError};
use std::{sync::Arc,time::Duration,os::fd::{AsRawFd,FromRawFd,OwnedFd}};

use std::sync::atomic::Ordering;

fn failed(error:impl std::fmt::Display)->StepError {super::store::step_error("publish_failed",error)}

pub(super) fn state(config:RuntimeConfig)->Result<AppState,StepError> {
    use crate::db_upgrade::key_file::{self,KeyFileError};
    let path=&config.data_key_file;
    let policy=key_file::canonical_policy(std::path::Path::new(crate::db_upgrade::paths::APPROOT),path);
    let key=key_file::read_key_file(path,policy).map_err(|error|match error {
        KeyFileError::Unsafe=>error.trusted_input_failure(path),
        _=>StepError{code:"key_unavailable",message:"암호화 키를 읽을 수 없습니다.",detail:Some(format!("{error}: {}",path.display()))},
    })?;
    let pg=config.database_url.parse::<tokio_postgres::Config>().map_err(failed)?;
    let manager=deadpool_postgres::Manager::from_config(pg,tokio_postgres::NoTls,deadpool_postgres::ManagerConfig{recycling_method:deadpool_postgres::RecyclingMethod::Fast});
    // One serialized writer plus optional post-transaction metadata; no API-sized pool in root.
    let db=deadpool_postgres::Pool::builder(manager).max_size(2).runtime(deadpool_postgres::Runtime::Tokio1)
        .wait_timeout(Some(Duration::from_secs(5))).create_timeout(Some(Duration::from_secs(5))).recycle_timeout(Some(Duration::from_secs(2)))
        .build().map_err(failed)?;
    let node_http=crate::nodes::client::NodeHttp::new(&config.version,&config).map_err(failed)?;
    let webhook_http=crate::notify::send::client(&config);
    let passwords=crate::auth::password::PasswordHasher::new(config.password_concurrency);
    Ok(AppState{db,secrets:Arc::new(key.secrets),node_http,webhook_http,passwords,config:Arc::new(config),runtime:Arc::new(RuntimeState::default())})
}

pub(super) struct Readiness(OwnedFd);
impl Readiness {
    /// Must run first, before another startup resource can reuse a missing fd 3.
    pub fn capture()->Result<Self,StepError> {
        if unsafe{libc::fcntl(3,libc::F_GETFD)}<0 {return Err(failed("Publisher readiness pipe fd 3 is missing"));}
        Self::from_fd(unsafe{OwnedFd::from_raw_fd(3)})
    }
    fn from_fd(fd:OwnedFd)->Result<Self,StepError> {
        let mut stat=std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe{libc::fstat(fd.as_raw_fd(),stat.as_mut_ptr())}<0 {return Err(failed(std::io::Error::last_os_error()));}
        let stat=unsafe{stat.assume_init()};
        if stat.st_mode&libc::S_IFMT!=libc::S_IFIFO {return Err(failed("Publisher readiness descriptor is not a pipe"));}
        let access=unsafe{libc::fcntl(fd.as_raw_fd(),libc::F_GETFL)};
        if access<0 || access&libc::O_ACCMODE==libc::O_RDONLY {return Err(failed("Publisher readiness pipe is not writable"));}
        let flags=unsafe{libc::fcntl(fd.as_raw_fd(),libc::F_GETFD)};
        if flags<0 || unsafe{libc::fcntl(fd.as_raw_fd(),libc::F_SETFD,flags|libc::FD_CLOEXEC)}<0 {return Err(failed(std::io::Error::last_os_error()));}
        Ok(Self(fd))
    }
    /// Consume the only writer after socket binding and epoch initialization, never earlier.
    pub fn notify(self)->Result<(),StepError> {
        use std::io::Write;
        std::fs::File::from(self.0).write_all(b"\n").map_err(failed)
    }
}

/// Only called once nginx is running; locks stay in store→apply→files order.
pub(super) async fn reconcile_and_adopt(state:&AppState,store:&super::store::PublisherStore)->Result<(),StepError> {
    let _store=store.lock(Duration::from_secs(30)).await.map_err(failed)?;
    let _apply=state.runtime.apply_lock.lock().await;
    match super::confirmation::nginx_running(&state.config).await.map_err(|error|super::store::step_error("restore_incomplete",error))? {
        super::confirmation::NginxState::Running=>{},
        super::confirmation::NginxState::Starting=>return Err(StepError{code:"nginx_starting",message:"nginx 시작이 끝날 때까지 복구를 기다립니다.",detail:None}),
        super::confirmation::NginxState::Stopped=>return Err(StepError{code:"nginx_not_ready",message:"nginx 시작을 기다리고 있습니다.",detail:None}),
    }
    {
        let _files=super::nginx_lock::acquire(&state.config).await.map_err(failed)?;
        super::recovery::after_start(&state.config,store).await.map_err(|error|super::store::step_error("restore_incomplete",error))?;
    }
    super::adoption::adopt_locked(&state.config,store).await?;
    let a8_clear=!super::confirmation::uncertain(&state.config).await;
    super::journal::collect_completed(store,a8_clear);
    if let Err(error)=super::journal::sweep_unreferenced(store){eprintln!("[publisher] payload sweep deferred: {error:#}");}
    Ok(())
}

/// Shared by the production socket and test-support; dispatcher already owns the store lock.
pub(super) async fn execute(state:&AppState,store:&super::store::PublisherStore,request:super::protocol::Request,mut operation:super::journal::Operation,budget:super::confirmation::Budget)->super::protocol::OpResult {
    use super::protocol::{Op,OpResult};
    let _apply=match budget.admit(state.runtime.apply_lock.lock()).await {Ok(owner)=>owner,Err(_)=>return OpResult::Busy};
    let actor=crate::contracts::Actor{id:request.actor.user_id,label:request.actor.label,ip:request.actor.ip};
    match request.op {
        command@Op::ApplySaved{..}|command@Op::RollbackSnapshot{..}=>super::apply::mutate(state,store,operation,command,&actor,&budget).await,
        Op::DeployCertificate{certificate_id,content_digest}=>{
            let result=super::cert_deploy::deploy(state,store,&mut operation,&certificate_id,&content_digest,&actor,&budget).await;
            deployment_result(result,&operation,super::confirmation::uncertain(&state.config).await)
        },
        _=>OpResult::BadRequest{code:"unsupported_operation".into()},
    }
}

fn deployment_result(result:anyhow::Result<crate::control::model::ApplyResult>,operation:&super::journal::Operation,uncertain:bool)->super::protocol::OpResult {
    use super::protocol::OpResult;
    match result {
        Ok(apply)=>OpResult::Ok{apply:Some(apply),skipped:Vec::new(),uncertain,phase:"done".into()},
        Err(error)=>operation.result.clone().and_then(|value|serde_json::from_value(value).ok())
            .filter(|result|matches!(result,OpResult::StaleMaterial|OpResult::Failed{..}))
            .unwrap_or_else(||OpResult::Failed{code:"publish_failed".into(),output:error.to_string()}),
    }
}

pub(super) async fn recover_before_readiness(state:&AppState,store:&super::store::PublisherStore)->Result<(),StepError> {
    let _store=store.lock(Duration::from_secs(30)).await.map_err(failed)?;
    let _apply=state.runtime.apply_lock.lock().await;
    let _files=super::nginx_lock::acquire(&state.config).await.map_err(failed)?;
    let running=super::confirmation::nginx_running(&state.config).await.map_err(|error|super::store::step_error("restore_incomplete",error))?;
    let recovery=match running {
        super::confirmation::NginxState::Running=>super::recovery::after_start(&state.config,store).await,
        super::confirmation::NginxState::Stopped=>super::recovery::before_start(&state.config,store).await,
        super::confirmation::NginxState::Starting=>return Err(StepError{code:"nginx_starting",message:"nginx 시작이 끝날 때까지 복구를 기다립니다.",detail:None}),
    };
    recovery.map_err(|error|super::store::step_error("restore_incomplete",error))
}

pub(super) async fn run(config:RuntimeConfig,readiness:Readiness)->Result<(),StepError> {
    let store=super::store::PublisherStore::open(&config)?;
    let state=state(config.clone())?;
    let socket=super::server::BoundSocket::bind().map_err(failed)?;
    let epoch=u64::from_be_bytes(uuid::Uuid::new_v4().as_bytes()[..8].try_into().unwrap());
    let dispatcher=super::dispatcher::Dispatcher::new(config,store.clone());
    let serving_dispatcher=dispatcher.clone();let serving_state=state.clone();let serving_store=store.clone();
    let mut serving=tokio::spawn(super::server::serve_bound(socket,epoch,move |request| {
        let dispatcher=serving_dispatcher.clone();let state=serving_state.clone();let store=serving_store.clone();
        async move {dispatcher.dispatch(request,move |request,operation,budget|async move {execute(&state,&store,request,operation,budget).await}).await}
    }));
    if let Err(error)=initial_recovery(&mut serving,||recover_before_readiness(&state,&store)).await {serving.abort();let _=serving.await;return Err(error);}
    // nginx depends on this socket-ready signal; Hello stays starting until running reconciliation.
    if let Err(error)=readiness.notify() {serving.abort();let _=serving.await;return Err(error);}
    serve_loop(dispatcher,serving,||reconcile_and_adopt(&state,&store)).await
}

async fn initial_recovery<F,Fut>(serving:&mut tokio::task::JoinHandle<anyhow::Result<()>>,mut recover:F)->Result<(),StepError>
where F:FnMut()->Fut,Fut:std::future::Future<Output=Result<(),StepError>> {
    let mut delay=Duration::from_secs(1);
    let mut last_failure=None;
    loop {
        tokio::select! {
            result=&mut *serving=>return result.map_err(failed)?.map_err(failed),
            result=recover()=>match result {
                Ok(())=>return Ok(()),
                Err(error)=>eprintln!("{}",recovery_message(&error,&mut last_failure,delay)),
            },
        }
        tokio::select! {
            result=&mut *serving=>return result.map_err(failed)?.map_err(failed),
            _=tokio::time::sleep(delay)=>{},
        }
        delay=(delay*2).min(Duration::from_secs(15*60));
    }
}

async fn serve_loop<F,Fut>(dispatcher:Arc<super::dispatcher::Dispatcher>,mut serving:tokio::task::JoinHandle<anyhow::Result<()>>,mut recover:F)->Result<(),StepError>
where F:FnMut()->Fut,Fut:std::future::Future<Output=Result<(),StepError>> {
    let mut adopted_once=false;
    let mut next_attempt=tokio::time::Instant::now();
    let mut retry_delay=Duration::from_secs(1);
    let mut last_failure=None;
    loop {
        tokio::select! {
            result=&mut serving=>return result.map_err(failed)?.map_err(failed),
            _=tokio::time::sleep(Duration::from_millis(100))=>{
                if !dispatcher.ready.load(Ordering::Acquire) {
                    if adopted_once {
                        eprintln!("[publisher] fenced after readiness; restarting for recovery");
                        serving.abort();let _=serving.await;
                        return Err(failed("Publisher fenced after readiness; restart required"));
                    }
                    if tokio::time::Instant::now()<next_attempt {continue;}
                    match recover().await {
                        Ok(())=>{adopted_once=true;dispatcher.ready.store(true,Ordering::Release);},
                        Err(error) if error.code=="nginx_not_ready"=>{},
                        Err(error)=>{
                            eprintln!("{}",recovery_message(&error,&mut last_failure,retry_delay));
                            next_attempt=tokio::time::Instant::now()+retry_delay;
                            retry_delay=(retry_delay*2).min(Duration::from_secs(15*60));
                        },
                    }
                }
            }
        }
    }
}

fn recovery_message(error:&StepError,last:&mut Option<(&'static str,Option<String>)>,delay:Duration)->String {
    let identity=(error.code,error.detail.clone());
    if last.as_ref()==Some(&identity){return format!("[publisher] retrying in {}s",delay.as_secs());}
    *last=Some(identity);
    let mut detail=String::new();
    for character in error.detail.as_deref().unwrap_or("상세 정보 없음").chars().filter(|character|!character.is_control()) {
        if detail.len()+character.len_utf8()>2048{break;}
        detail.push(character);
    }
    format!("[publisher] recovery failed: {}: {} — {} — 원인을 해결하면 자동으로 다시 시도합니다(최대 15분 간격). 컨테이너를 다시 시작해도 됩니다.",error.code,error.message,detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt,MetadataExt};
    fn config(key:&std::path::Path)->RuntimeConfig {
        RuntimeConfig::read(|name|match name {
            "DATABASE_URL"=>Some("postgres://fixture@remote.invalid:6543/not_connected".into()),
            "AUTH_SECRET"=>Some("root-startup-fixture".into()),"DATA_KEY_FILE"=>Some(key.display().to_string()),_=>None,
        }).unwrap()
    }
    async fn clock_tick(duration:Duration) {
        tokio::time::advance(duration).await;
        for _ in 0..4 {tokio::task::yield_now().await;}
    }
    fn loop_fixture()->(tempfile::TempDir,Arc<super::super::dispatcher::Dispatcher>,tokio::task::JoinHandle<anyhow::Result<()>>) {
        let(root,config,store,_,_)=super::super::recovery::tests::durable_fixture();
        let dispatcher=super::super::dispatcher::Dispatcher::new(config,store);
        (root,dispatcher,tokio::spawn(std::future::pending()))
    }
    #[test]
    fn g36_recovery_diagnostics_are_actionable_bounded_and_deduplicated() {
        let mut previous=None;
        let mut error=StepError{code:"restore_incomplete",message:"nginx의 이전 설정 복원을 완료하지 못했습니다.",detail:Some("payload abc /var/lib/nwui: Permission denied\n\u{1b}".into())};
        let first=recovery_message(&error,&mut previous,Duration::from_secs(1));
        assert_eq!(first,"[publisher] recovery failed: restore_incomplete: nginx의 이전 설정 복원을 완료하지 못했습니다. — payload abc /var/lib/nwui: Permission denied — 원인을 해결하면 자동으로 다시 시도합니다(최대 15분 간격). 컨테이너를 다시 시작해도 됩니다.");
        assert_eq!(recovery_message(&error,&mut previous,Duration::from_secs(2)),"[publisher] retrying in 2s");
        error.detail=Some("changed payload".into());assert!(recovery_message(&error,&mut previous,Duration::from_secs(4)).contains(" — changed payload — "));
        error.code="publish_failed";assert!(recovery_message(&error,&mut previous,Duration::from_secs(8)).starts_with("[publisher] recovery failed: publish_failed:"));
        error.detail=Some("가".repeat(2000));let bounded=recovery_message(&error,&mut previous,Duration::from_secs(16));
        let detail=bounded.split(" — ").nth(1).unwrap();assert!(detail.len()<=2048);assert!(!detail.is_empty());
        assert!(!bounded.chars().any(char::is_control));
    }
    #[tokio::test]
    async fn g30_initial_recovery_serves_starting_without_notifying_then_adopts_in_the_same_process() {
        use super::super::protocol::{Actor,Op,OpResult,Request,Response};
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        async fn exchange(path:&std::path::Path,op:Op)->OpResult {
            let mut socket=tokio::net::UnixStream::connect(path).await.unwrap();
            let request=Request{v:1,id:uuid::Uuid::new_v4().to_string(),op,deadline_ms:5000,actor:Actor{user_id:None,label:None,ip:None}};
            let body=serde_json::to_vec(&request).unwrap();socket.write_u32(body.len()as u32).await.unwrap();socket.write_all(&body).await.unwrap();socket.shutdown().await.unwrap();
            let size=socket.read_u32().await.unwrap();let mut body=vec![0;size as usize];socket.read_exact(&mut body).await.unwrap();serde_json::from_slice::<Response>(&body).unwrap().result
        }
        let(root,dispatcher,dummy)=loop_fixture();dummy.abort();let process=std::process::id();
        let directory=root.path().join("socket");let path=directory.join("control.sock");let owner=dispatcher.clone();
        let mut serving=super::super::server::serve_for_tests(&directory,99,move|request|{let owner=owner.clone();async move{owner.dispatch(request,|_,_,_|async{panic!("not-ready request must never execute")}).await}}).unwrap();
        let fixed=Arc::new(std::sync::atomic::AtomicBool::new(false));let condition=fixed.clone();
        let mut fds=[0;2];assert_eq!(unsafe{libc::pipe2(fds.as_mut_ptr(),libc::O_NONBLOCK|libc::O_CLOEXEC)},0);
        let mut reader=unsafe{std::fs::File::from_raw_fd(fds[0])};let readiness=Readiness::from_fd(unsafe{OwnedFd::from_raw_fd(fds[1])}).unwrap();
        let owner=dispatcher.clone();let task=tokio::spawn(async move {
            initial_recovery(&mut serving,move||std::future::ready(if condition.load(Ordering::Acquire){Ok(())}else{Err(failed("injected before_start failure"))})).await.unwrap();
            readiness.notify().unwrap();serve_loop(owner,serving,||std::future::ready(Ok(()))).await
        });
        assert!(matches!(exchange(&path,Op::Hello).await,OpResult::Ok{phase,..} if phase=="starting"));
        assert!(matches!(exchange(&path,Op::ApplySaved{label:None,expected_saved_digest:None}).await,OpResult::Unavailable));
        use std::io::Read;let mut byte=[0];assert_eq!(reader.read(&mut byte).unwrap_err().kind(),std::io::ErrorKind::WouldBlock);assert!(!task.is_finished());
        fixed.store(true,Ordering::Release);
        tokio::time::timeout(Duration::from_secs(3),async {loop{if dispatcher.ready.load(Ordering::Acquire){break;}tokio::time::sleep(Duration::from_millis(10)).await;}}).await.unwrap();
        assert_eq!(std::process::id(),process);assert_eq!(reader.read(&mut byte).unwrap(),1);assert_eq!(byte,[b'\n']);
        assert!(matches!(exchange(&path,Op::Hello).await,OpResult::Ok{phase,..} if phase=="ready"));task.abort();
    }
    #[tokio::test(start_paused=true)]
    async fn g30_before_readiness_keeps_serving_starting_and_signals_only_after_recovery() {
        use std::sync::atomic::AtomicBool;
        let(_root,dispatcher,mut serving)=loop_fixture();let fixed=Arc::new(AtomicBool::new(false));let condition=fixed.clone();
        let attempts=Arc::new(std::sync::Mutex::new(Vec::new()));let record=attempts.clone();
        let (reader,writer)=std::os::unix::net::UnixStream::pair().unwrap();reader.set_nonblocking(true).unwrap();
        // The production readiness writer is a pipe; this byte channel observes the same notify boundary.
        let task=tokio::spawn(async move {
            initial_recovery(&mut serving,move||{record.lock().unwrap().push(tokio::time::Instant::now());std::future::ready(if condition.load(Ordering::Acquire){Ok(())}else{Err(failed("injected before_start failure"))})}).await.unwrap();
            use std::io::Write;let mut writer=writer;writer.write_all(b"\n").unwrap();serving.abort();
        });
        tokio::task::yield_now().await;
        for seconds in [1,2,4,8,16,29] {clock_tick(Duration::from_secs(seconds)).await;}
        assert!(!task.is_finished());assert!(!dispatcher.ready.load(Ordering::Acquire));
        use std::io::Read;let mut reader=reader;let mut byte=[0];assert_eq!(reader.read(&mut byte).unwrap_err().kind(),std::io::ErrorKind::WouldBlock);
        assert_eq!(attempts.lock().unwrap().len(),6);
        fixed.store(true,Ordering::Release);clock_tick(Duration::from_secs(3)).await;task.await.unwrap();assert_eq!(reader.read(&mut byte).unwrap(),1);assert_eq!(byte,b"\n"[..]);
    }
    #[tokio::test(start_paused=true)]
    async fn a35_a_fence_after_first_adoption_exits_without_reconciling_even_if_the_fault_is_gone() {
        use std::sync::atomic::{AtomicUsize,Ordering};
        let(_root,dispatcher,serving)=loop_fixture();let calls=Arc::new(AtomicUsize::new(0));let observed=calls.clone();let owner=dispatcher.clone();
        let task=tokio::spawn(serve_loop(owner,serving,move||{observed.fetch_add(1,Ordering::SeqCst);std::future::ready(Ok(()))}));
        tokio::task::yield_now().await;clock_tick(Duration::from_millis(100)).await;
        assert!(dispatcher.ready.load(Ordering::Acquire));assert_eq!(calls.load(Ordering::SeqCst),1);
        dispatcher.ready.store(false,Ordering::Release);clock_tick(Duration::from_millis(100)).await;
        let exited=task.is_finished();if !exited{task.abort();}
        assert!(exited,"a post-ready fence must exit rather than re-adopt in this process");
        assert!(task.await.unwrap().is_err());assert_eq!(calls.load(Ordering::SeqCst),1);
    }
    #[tokio::test(start_paused=true)]
    async fn g30_initial_failures_stay_alive_for_sixty_seconds_with_exponential_retry_then_recover() {
        use std::sync::atomic::{AtomicBool,Ordering};
        let(_root,dispatcher,serving)=loop_fixture();let abort=serving.abort_handle();let fixed=Arc::new(AtomicBool::new(false));let condition=fixed.clone();
        let attempts=Arc::new(std::sync::Mutex::new(Vec::new()));let record=attempts.clone();let owner=dispatcher.clone();
        let task=tokio::spawn(serve_loop(owner,serving,move||{
            record.lock().unwrap().push(tokio::time::Instant::now());
            std::future::ready(if condition.load(Ordering::Acquire){Ok(())}else{Err(StepError{code:"restore_incomplete",message:"injected permanent recovery failure",detail:None})})
        }));
        tokio::task::yield_now().await;clock_tick(Duration::from_millis(100)).await;
        assert!(!task.is_finished(),"initial recovery failure must retain the process/socket");
        for (seconds,count) in [(1,2),(2,3),(4,4),(8,5),(16,6),(29,6)] {
            clock_tick(Duration::from_secs(seconds)).await;assert!(!task.is_finished());assert_eq!(attempts.lock().unwrap().len(),count);
            assert!(!dispatcher.ready.load(Ordering::Acquire));
        }
        let times=attempts.lock().unwrap().clone();let gaps:Vec<_>=times.windows(2).map(|p|p[1]-p[0]).collect();assert_eq!(gaps,[1,2,4,8,16].map(Duration::from_secs));
        fixed.store(true,Ordering::Release);clock_tick(Duration::from_secs(3)).await;assert!(dispatcher.ready.load(Ordering::Acquire));assert!(!task.is_finished());
        task.abort();abort.abort();
    }
    #[tokio::test(start_paused=true)]
    async fn g30_initial_recovery_backoff_caps_at_the_frozen_fifteen_minutes() {
        let(_root,dispatcher,serving)=loop_fixture();let abort=serving.abort_handle();let attempts=Arc::new(std::sync::Mutex::new(Vec::new()));let record=attempts.clone();
        let task=tokio::spawn(serve_loop(dispatcher,serving,move||{record.lock().unwrap().push(tokio::time::Instant::now());std::future::ready(Err(StepError{code:"restore_incomplete",message:"injected",detail:None}))}));
        tokio::task::yield_now().await;clock_tick(Duration::from_millis(100)).await;
        for seconds in [1,2,4,8,16,32,64,128,256,512,900,900] {clock_tick(Duration::from_secs(seconds)).await;}
        let times=attempts.lock().unwrap().clone();let gaps:Vec<_>=times.windows(2).map(|p|p[1]-p[0]).collect();
        task.abort();abort.abort();assert_eq!(gaps,[1,2,4,8,16,32,64,128,256,512,900,900].map(Duration::from_secs));
    }
    #[tokio::test(start_paused=true)]
    async fn a35_initial_nginx_not_ready_waits_without_exiting_or_becoming_available() {
        use std::sync::atomic::{AtomicUsize,Ordering};
        let(_root,dispatcher,serving)=loop_fixture();let abort=serving.abort_handle();let owner=dispatcher.clone();let calls=Arc::new(AtomicUsize::new(0));let count=calls.clone();
        let task=tokio::spawn(serve_loop(owner,serving,move||{count.fetch_add(1,Ordering::SeqCst);std::future::ready(Err(StepError{code:"nginx_not_ready",message:"waiting",detail:None}))}));
        tokio::task::yield_now().await;
        for _ in 0..3 {clock_tick(Duration::from_millis(100)).await;}
        assert!(!task.is_finished());assert!(!dispatcher.ready.load(Ordering::Acquire));assert_eq!(calls.load(Ordering::SeqCst),3);
        task.abort();abort.abort();
    }
    #[tokio::test]
    async fn root_state_never_creates_a_missing_key_or_changes_an_unsafe_key() {
        let root=tempfile::tempdir().unwrap();let path=root.path().join("key");
        assert_eq!(state(config(&path)).err().unwrap().code,"key_unavailable");assert!(!path.exists());
        std::fs::write(&path,"03".repeat(32)).unwrap();std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o666)).unwrap();
        let before=std::fs::read(&path).unwrap();let inode=std::fs::metadata(&path).unwrap().ino();
        assert_eq!(state(config(&path)).err().unwrap().code,"trusted_input_unsafe");
        assert_eq!(std::fs::read(&path).unwrap(),before);assert_eq!(std::fs::metadata(&path).unwrap().ino(),inode);assert_eq!(std::fs::metadata(&path).unwrap().mode()&0o777,0o666);
    }
    #[tokio::test]
    async fn root_state_uses_validated_existing_material_and_opens_no_db_connection() {
        let root=tempfile::tempdir().unwrap();let path=root.path().join("key");
        std::fs::write(&path,"03".repeat(32)).unwrap();std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o600)).unwrap();
        let before=std::fs::read(&path).unwrap();let state=state(config(&path)).unwrap();
        assert_eq!(state.db.status().max_size,2);assert_eq!(state.db.status().size,0);
        let token=crate::crypto::SecretBox::from_key([3;32]).seal("startup fixture").unwrap();
        assert_eq!(state.secrets.open_maybe_plain(Some(&token)).as_deref(),Some("startup fixture"));assert_eq!(std::fs::read(path).unwrap(),before);
    }
    #[tokio::test]
    async fn reconciliation_cannot_claim_ready_when_the_master_is_missing() {
        let (root,state,store,_)=super::super::adoption::tests::fixture();
        std::fs::remove_file(root.path().join(".fake-nginx.pid")).unwrap();
        let before=super::super::files::capture_payload_files(&state.config).await.unwrap();
        assert_eq!(reconcile_and_adopt(&state,&store).await.err().unwrap().code,"nginx_not_ready");
        assert_eq!(super::super::files::capture_payload_files(&state.config).await.unwrap(),before);
        assert!(!super::super::confirmation::uncertain(&state.config).await);
    }
    #[test]
    fn certificate_dispatch_preserves_stale_and_failure_results_and_never_replays_uncommitted_success() {
        use super::super::{journal::{Admission,Operation},protocol::OpResult};
        let (_root,_config,store,_old,_new)=super::super::recovery::tests::durable_fixture();
        let Admission::New(mut operation)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&"a".repeat(64)).unwrap() else {panic!()};
        for expected in [OpResult::StaleMaterial,OpResult::Failed{code:"publish_failed".into(),output:"actual restore failure".into()}] {
            operation.result=Some(serde_json::to_value(&expected).unwrap());
            assert_eq!(serde_json::to_value(deployment_result(Err(anyhow::anyhow!("outer error")),&operation,true)).unwrap(),serde_json::to_value(expected).unwrap());
        }
        operation.result=Some(serde_json::to_value(OpResult::Ok{apply:None,skipped:Vec::new(),uncertain:false,phase:"done".into()}).unwrap());
        assert!(matches!(deployment_result(Err(anyhow::anyhow!("commit error")),&operation,true),OpResult::Failed{output,..} if output=="commit error"));
    }
    #[tokio::test]
    async fn startup_recovery_uses_the_nginx_down_track_without_fabricating_confirmation() {
        let (root,state,store,_)=super::super::adoption::tests::fixture();
        std::fs::remove_file(root.path().join(".fake-nginx.pid")).unwrap();
        super::super::confirmation::begin_uncertain(&state.config).await.unwrap();
        recover_before_readiness(&state,&store).await.unwrap();
        assert!(super::super::confirmation::uncertain(&state.config).await);
        assert_eq!(reconcile_and_adopt(&state,&store).await.unwrap_err().code,"nginx_not_ready");
    }
    #[test]
    fn readiness_accepts_only_a_writable_pipe_and_is_consumed_after_one_newline() {
        use std::io::Read;
        let mut pipe=[-1;2];assert_eq!(unsafe{libc::pipe(pipe.as_mut_ptr())},0);
        let mut reader=std::fs::File::from(unsafe{OwnedFd::from_raw_fd(pipe[0])});
        let ready=Readiness::from_fd(unsafe{OwnedFd::from_raw_fd(pipe[1])}).unwrap();
        assert_ne!(unsafe{libc::fcntl(ready.0.as_raw_fd(),libc::F_GETFD)}&libc::FD_CLOEXEC,0);
        ready.notify().unwrap();let mut bytes=Vec::new();reader.read_to_end(&mut bytes).unwrap();assert_eq!(bytes,b"\n");
        assert!(Readiness::from_fd(tempfile::tempfile().unwrap().into()).is_err());
        let mut pipe=[-1;2];assert_eq!(unsafe{libc::pipe(pipe.as_mut_ptr())},0);
        let reader=unsafe{OwnedFd::from_raw_fd(pipe[0])};let writer=unsafe{OwnedFd::from_raw_fd(pipe[1])};
        assert!(Readiness::from_fd(reader).is_err());drop(writer);
    }
    #[tokio::test]
    async fn startup_collects_expired_payload_and_its_staged_crash_leftover_after_adoption(){
        use super::super::{journal::{Admission,Operation,Phase},store::Origin,staging::{self,PreparedMain}};
        let (_root,state,store,_)=super::super::adoption::tests::fixture();let generation=uuid::Uuid::new_v4().to_string();
        store.stage(&generation,Origin::Publisher,None,std::collections::BTreeMap::new(),std::collections::BTreeMap::new()).unwrap();
        let payload=store.root.join("payloads").join(&generation);
        let PreparedMain::Staged(staged)=staging::prepare_main(&state.config.nginx_main_conf,&state.config,&payload,&generation).unwrap()else{panic!("writable staged fixture")};
        let leftover=staged.path.clone();std::mem::forget(staged);assert!(leftover.exists());
        let Admission::New(mut expired)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&"a".repeat(64)).unwrap()else{panic!()};
        expired.phase=Phase::Done;expired.next=Some(generation);expired.created_at=chrono::Utc::now()-chrono::Duration::hours(48);expired.save(&store).unwrap();
        reconcile_and_adopt(&state,&store).await.unwrap();
        assert!(!leftover.exists());assert!(!payload.exists());assert!(Operation::load(&store,&expired.id).is_err());
        let manifest=store.manifest().unwrap();for id in [manifest.current,manifest.previous].into_iter().flatten(){assert!(store.payload(&id).is_ok());}
    }

    #[tokio::test]
    async fn a28_startup_sweeps_complete_and_partial_orphans_but_keeps_all_references(){
        use super::super::{journal::{Admission,Operation,Phase},store::Origin,staging::{self,PreparedMain}};
        let (_root,state,store,_)=super::super::adoption::tests::fixture();
        let complete=uuid::Uuid::new_v4().to_string();store.stage(&complete,Origin::Publisher,None,Default::default(),Default::default()).unwrap();
        let payload=store.root.join("payloads").join(&complete);
        let PreparedMain::Staged(staged)=staging::prepare_main(&state.config.nginx_main_conf,&state.config,&payload,&complete).unwrap()else{panic!()};let leftover=staged.path.clone();std::mem::forget(staged);
        let partial=store.directory("payloads").unwrap().join(uuid::Uuid::new_v4().to_string());crate::db_upgrade::files::private_dir(&partial).unwrap();
        let other=store.directory("payloads").unwrap().join("operator-note");std::fs::create_dir(&other).unwrap();std::fs::write(other.join("keep"),"untouched").unwrap();
        let source=uuid::Uuid::new_v4().to_string();store.stage(&source,Origin::Publisher,None,Default::default(),Default::default()).unwrap();let derived=uuid::Uuid::new_v4().to_string();store.stage_derived(&derived,&source,"retained provenance",Default::default()).unwrap();
        let Admission::New(mut referenced)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&"a".repeat(64)).unwrap()else{panic!()};referenced.phase=Phase::Done;referenced.restore_target=Some(derived.clone());referenced.save(&store).unwrap();
        let before=store.manifest().unwrap();reconcile_and_adopt(&state,&store).await.unwrap();
        assert!(!payload.exists(),"complete orphan must be swept");assert!(!leftover.exists());assert!(!partial.exists());assert_eq!(std::fs::read_to_string(other.join("keep")).unwrap(),"untouched");
        for id in [before.current,before.previous,Some(source),Some(derived)].into_iter().flatten(){assert!(store.payload(&id).is_ok(),"protected {id}");}
    }
    #[tokio::test]
    async fn a28_startup_logs_sweep_error_without_following_uuid_symlink_or_failing(){
        let (_root,state,store,_)=super::super::adoption::tests::fixture();let outside=tempfile::tempdir().unwrap();std::fs::write(outside.path().join("keep"),"untouched").unwrap();
        let link=store.directory("payloads").unwrap().join(uuid::Uuid::new_v4().to_string());std::os::unix::fs::symlink(outside.path(),&link).unwrap();
        reconcile_and_adopt(&state,&store).await.unwrap();assert!(link.symlink_metadata().unwrap().file_type().is_symlink());assert_eq!(std::fs::read_to_string(outside.path().join("keep")).unwrap(),"untouched");
    }

}
