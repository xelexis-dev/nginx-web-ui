//! API-side publisher seam; the in-process mode preserves today's writer behavior.
use crate::{contracts::Actor, state::AppState};
use crate::control::model::{ApplyResult,RollbackKind,RollbackResult};
use std::{path::{Path, PathBuf}, time::Duration};
use crate::publisher::protocol::{self, Op, OpResult, Request, Response};
use tokio::{io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt}, net::UnixStream, time::{Instant, timeout_at}};

pub const SOCKET: &str = "/run/nwui-publisher/control.sock";

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("설정 적용 서비스에 연결할 수 없습니다.")]
    Unavailable { detail: String },
    #[error("설정 적용 결과를 확인하지 못했습니다.")]
    Unconfirmed { operation_id: String },
    #[error("설정 적용 서비스 응답을 해석하지 못했습니다.")]
    Protocol { detail: String },
}

#[derive(Clone, Debug)]
pub struct Bounds {
    pub admission: Duration,
    pub operation: Duration,
    pub connect: Duration,
    pub write: Duration,
    pub slack: Duration,
    pub status: Duration,
    pub validate: Duration,
    pub hello: Duration,
}
impl Default for Bounds {
    fn default() -> Self {
        Self {
            admission: Duration::from_secs(30),
            operation: Duration::from_secs(280),
            connect: Duration::from_secs(2),
            write: Duration::from_secs(2),
            slack: Duration::from_secs(2),
            status: Duration::from_secs(15),
            validate: Duration::from_secs(35),
            hello: Duration::from_secs(2),
        }
    }
}

#[derive(Clone)]
enum Mode {
    Socket { path: PathBuf, bounds: Bounds, checks: bool },
    #[cfg(any(test, feature = "test-support"))]
    InProcess,
}

#[derive(Clone)]
pub struct Client { mode: Mode }
impl Client {
    pub fn system() -> Self {
        Self { mode: Mode::Socket { path: SOCKET.into(), bounds: Bounds::default(), checks: true } }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn socket_for_tests(path: PathBuf, bounds: Bounds) -> Self {
        Self { mode: Mode::Socket { path, bounds, checks: false } }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn in_process() -> Self { Self { mode: Mode::InProcess } }
}
impl Default for Client {
    fn default() -> Self {
        #[cfg(any(test, feature = "test-support"))]
        { Self::in_process() }
        #[cfg(not(any(test, feature = "test-support")))]
        { Self::system() }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum LiveValidation { Checked { ok: bool, output: String }, Busy }
#[derive(Debug, PartialEq, Eq)]
pub struct Hello { pub ready: bool, pub uncertain: bool, pub epoch: u64 }

const ADMISSION_ERROR: &str = "설정 적용 대기가 30초를 넘었습니다. 잠시 후 다시 시도하세요.";

fn failure_line(name:&str,id:&str,error:&(dyn std::error::Error+'static))->String {
    match error.downcast_ref::<ClientError>() {
        Some(client)=>format!("[publisher-client] {name} {id}: {client:?}"),
        None=>format!("[publisher-client] {name} {id}: {:?}",error.to_string()),
    }
}
fn unavailable(detail: impl ToString) -> ClientError { ClientError::Unavailable { detail: detail.to_string() } }
fn protocol_error(detail: impl ToString) -> ClientError { ClientError::Protocol { detail: detail.to_string() } }
fn deadline_ms(deadline: Instant, now: Instant) -> u32 {
    deadline.saturating_duration_since(now).as_millis().clamp(1,280_000) as u32
}
fn request(op:Op, actor:protocol::Actor, deadline_ms:u32) -> Request {
    Request { v:1, id:uuid::Uuid::new_v4().to_string(), op, actor, deadline_ms }
}
fn wire_frame(request:&Request)->Result<Vec<u8>,ClientError> {
    let body=serde_json::to_vec(request).map_err(protocol_error)?;
    if body.len()>65_536 { return Err(protocol_error("request exceeds 64 KiB")); }
    let mut frame=Vec::with_capacity(body.len()+4);frame.extend_from_slice(&(body.len()as u32).to_be_bytes());frame.extend(body);Ok(frame)
}
#[derive(Debug)]
struct ExchangeError { error:ClientError, sent:bool }
impl ExchangeError {
    fn before(error:ClientError)->Self {Self{error,sent:false}}
    fn after(error:ClientError)->Self {Self{error,sent:true}}
}
async fn connect(path:&Path,checks:bool)->Result<UnixStream,ClientError> {
    use std::os::unix::fs::{FileTypeExt,MetadataExt};
    if checks {
        let parent=path.parent().ok_or_else(||unavailable("socket parent missing"))?;
        let directory=tokio::fs::symlink_metadata(parent).await.map_err(unavailable)?;
        if !directory.is_dir()||directory.uid()!=0||directory.mode()&0o022!=0 {return Err(unavailable("unsafe socket parent"));}
        let socket=tokio::fs::symlink_metadata(path).await.map_err(unavailable)?;
        if !socket.file_type().is_socket()||socket.uid()!=0 {return Err(unavailable("unsafe socket owner or type"));}
    }
    let stream=UnixStream::connect(path).await.map_err(unavailable)?;
    if checks&&stream.peer_cred().map_err(unavailable)?.uid()!=0 {return Err(unavailable("publisher peer is not root"));}
    Ok(stream)
}
async fn read_response<S:AsyncRead+Unpin>(stream:&mut S,id:&str)->Result<Response,ClientError> {
    let length=stream.read_u32().await.map_err(unavailable)?;
    if !(1..=1_048_576).contains(&length){return Err(protocol_error("response length outside 1..=1 MiB"));}
    let mut body=vec![0;length as usize];stream.read_exact(&mut body).await.map_err(unavailable)?;
    let response:Response=serde_json::from_slice(&body).map_err(protocol_error)?;
    let accepted_id=if response.id.is_empty(){matches!(&response.result,OpResult::BadRequest{..})}else{response.id==id};
    if response.v!=1||!accepted_id {return Err(protocol_error("response version or operation id mismatch"));}
    Ok(response)
}
async fn framed_exchange<S:AsyncRead+AsyncWrite+Unpin>(stream:&mut S,frame:&[u8],id:&str,write_deadline:Instant,read_deadline:Instant)->Result<Response,ExchangeError> {
    timeout_at(write_deadline,stream.write_all(frame)).await
        .map_err(|_|ExchangeError::before(unavailable("write timed out")))?
        .map_err(|e|ExchangeError::before(unavailable(e)))?;
    // Every frame byte is written: even a failed half-close can now admit the mutation.
    timeout_at(write_deadline,stream.shutdown()).await
        .map_err(|_|ExchangeError::after(unavailable("half-close timed out")))?
        .map_err(|e|ExchangeError::after(unavailable(e)))?;
    timeout_at(read_deadline,read_response(stream,id)).await
        .map_err(|_|ExchangeError::after(unavailable("response timed out")))?
        .map_err(ExchangeError::after)
}
async fn exchange(path:&Path,bounds:&Bounds,checks:bool,request:&Request,until:Instant)->Result<Response,ExchangeError> {
    let frame=wire_frame(request).map_err(ExchangeError::before)?;
    let mut stream=timeout_at(until.min(Instant::now()+bounds.connect),connect(path,checks)).await
        .map_err(|_|ExchangeError::before(unavailable("connect timed out")))?
        .map_err(ExchangeError::before)?;
    framed_exchange(&mut stream,&frame,&request.id,until.min(Instant::now()+bounds.write),until).await
}
fn mutation_result(result:OpResult,id:&str,rollback:bool)->anyhow::Result<RollbackResult> {
    let result=match result {
        OpResult::Ok{apply:Some(result),..}=>return Ok(RollbackResult{result,kind:None}),
        OpResult::Busy=>anyhow::anyhow!(ADMISSION_ERROR),
        OpResult::StaleMaterial if !rollback=>anyhow::anyhow!("stale_material"),
        OpResult::Failed{code,output} if code=="snapshot_not_found"||code=="db_restore_failed"=>{
            if !rollback {return Err(protocol_error("rollback failure for a different operation").into());}
            return Ok(RollbackResult{result:ApplyResult{ok:false,output,rolled_back_to:None},kind:Some(if code=="snapshot_not_found"{RollbackKind::NotFound}else{RollbackKind::DbRestoreFailed})});
        }
        OpResult::Failed{code,output} if code=="publisher_storage_full"=>return Ok(RollbackResult{result:ApplyResult{ok:false,output,rolled_back_to:None},kind:None}),
        OpResult::Failed{code,..} if code=="response_too_large"||code=="unconfirmed"=>ClientError::Unconfirmed{operation_id:id.into()}.into(),
        OpResult::Failed{output,..}=>anyhow::anyhow!(output),
        OpResult::Unavailable=>unavailable("publisher unavailable").into(),
        _=>protocol_error("unexpected mutation result").into(),
    };Err(result)
}
async fn recover_response(path:&Path,bounds:&Bounds,checks:bool,req:&Request,first:Result<Response,ExchangeError>)->anyhow::Result<Response> {
    match first {
        Ok(response)=>Ok(response),
        Err(failure) if !failure.sent=>Err(failure.error.into()),
        Err(_)=>{
            let status=request(Op::OperationStatus{operation_id:req.id.clone()},protocol::Actor{user_id:req.actor.user_id.clone(),label:req.actor.label.clone(),ip:req.actor.ip.clone()},bounds.status.as_millis().clamp(1,280_000)as u32);
            match exchange(path,bounds,checks,&status,Instant::now()+bounds.status).await {
                Ok(response) if !matches!(&response.result,OpResult::Unknown|OpResult::Busy|OpResult::Unavailable|OpResult::Conflict|OpResult::BadRequest{..}) && !matches!(&response.result,OpResult::Ok{phase,..}if phase=="running")=>Ok(response),
                _=>Err(ClientError::Unconfirmed{operation_id:req.id.clone()}.into()),
            }
        }
    }
}
async fn mutate(state:&AppState,op:Op,actor:&Actor)->anyhow::Result<RollbackResult> {
    let (path,bounds,checks)=match &state.runtime.publisher.mode {
        Mode::Socket{path,bounds,checks}=>(path,bounds,checks),
        #[cfg(any(test, feature = "test-support"))]
        Mode::InProcess=>unreachable!(),
    };
    let start=Instant::now();let deadline=start+bounds.operation;
    let rollback=matches!(op,Op::RollbackSnapshot{..});
    let name=match &op{Op::ApplySaved{..}=>"ApplySaved",Op::RollbackSnapshot{..}=>"RollbackSnapshot",_=>"DeployCertificate"};
    let req=request(op,protocol::Actor{user_id:actor.id.clone(),label:actor.label.clone(),ip:actor.ip.clone()},deadline_ms(deadline,Instant::now()));
    let _guard=match timeout_at(start+bounds.admission,state.runtime.apply_lock.lock()).await {
        Ok(guard)=>guard,
        Err(_)=>{
            eprintln!("{}",failure_line(name,&req.id,anyhow::anyhow!(ADMISSION_ERROR).as_ref()));
            return Err(anyhow::anyhow!(ADMISSION_ERROR));
        }
    };
    let result=async {
        let req=Request{v:req.v,id:req.id.clone(),op:req.op,deadline_ms:deadline_ms(deadline,Instant::now()),actor:req.actor};
        let response=recover_response(path,bounds,*checks,&req,exchange(path,bounds,*checks,&req,deadline+bounds.slack).await).await?;
        mutation_result(response.result,&req.id,rollback)
    }.await;
    if let Err(error)=&result {eprintln!("{}",failure_line(name,&req.id,error.as_ref()));}
    result
}
async fn readonly<T>(state:&AppState,op:Op,map:impl FnOnce(Response)->Result<T,ClientError>)->Result<T,ClientError> {
    let (path,bounds,checks)=match &state.runtime.publisher.mode {
        Mode::Socket{path,bounds,checks}=>(path,bounds,checks),
        #[cfg(any(test, feature = "test-support"))]
        Mode::InProcess=>unreachable!(),
    };
    let is_hello=matches!(op,Op::Hello);let name=if is_hello{"Hello"}else{"ValidateLive"};
    let duration=if is_hello{bounds.hello}else{bounds.validate+bounds.slack};
    let req=request(op,protocol::Actor{user_id:None,label:None,ip:None},if is_hello{2000}else{bounds.validate.as_millis().clamp(1,280_000)as u32});
    let result=exchange(path,bounds,*checks,&req,Instant::now()+duration).await.map_err(|failure|failure.error).and_then(map);
    if let Err(error)=&result {eprintln!("{}",failure_line(name,&req.id,error));}
    result
}

#[cfg(any(test, feature = "test-support"))]
async fn in_process_mutate(state:&AppState,op:Op,actor:&Actor)->anyhow::Result<RollbackResult> {
    let rollback=matches!(op,Op::RollbackSnapshot{..});
    let req=request(op,protocol::Actor{user_id:actor.id.clone(),label:actor.label.clone(),ip:actor.ip.clone()},280000);
    let id=req.id.clone();
    mutation_result(crate::publisher::test_support::dispatch(state,req).await,&id,rollback)
}

pub async fn apply_saved(state: &AppState, actor: &Actor, label: Option<&str>) -> anyhow::Result<ApplyResult> {
    match &state.runtime.publisher.mode {
        #[cfg(any(test, feature = "test-support"))]
        Mode::InProcess => in_process_mutate(state,Op::ApplySaved{label:label.map(str::to_owned),expected_saved_digest:None},actor).await.map(|r|r.result),
        Mode::Socket { .. } => mutate(state,Op::ApplySaved{label:label.map(str::to_owned),expected_saved_digest:None},actor).await.map(|r|r.result),
    }
}

pub async fn rollback_snapshot(state: &AppState, snapshot_id: &str, actor: &Actor) -> anyhow::Result<RollbackResult> {
    if !uuid::Uuid::parse_str(snapshot_id).is_ok_and(|id|id.to_string()==snapshot_id) {
        return Ok(RollbackResult {
            result: ApplyResult { ok: false, output: "스냅샷을 찾을 수 없습니다.".into(), rolled_back_to: None },
            kind: Some(RollbackKind::NotFound),
        });
    }
    match &state.runtime.publisher.mode {
        #[cfg(any(test, feature = "test-support"))]
        Mode::InProcess => in_process_mutate(state,Op::RollbackSnapshot{snapshot_id:snapshot_id.into()},actor).await,
        Mode::Socket { .. } => mutate(state,Op::RollbackSnapshot{snapshot_id:snapshot_id.into()},actor).await,
    }
}

pub async fn deploy_certificate(state: &AppState, certificate_id: &str, content_digest: &str, actor: &Actor) -> anyhow::Result<ApplyResult> {
    match &state.runtime.publisher.mode {
        #[cfg(any(test, feature = "test-support"))]
        Mode::InProcess => in_process_mutate(state,Op::DeployCertificate{certificate_id:certificate_id.into(),content_digest:content_digest.into()},actor).await.map(|r|r.result),
        Mode::Socket { .. } => mutate(state,Op::DeployCertificate{certificate_id:certificate_id.into(),content_digest:content_digest.into()},actor).await.map(|r|r.result),
    }
}

pub async fn validate_live(state: &AppState) -> Result<LiveValidation, ClientError> {
    match &state.runtime.publisher.mode {
        #[cfg(any(test, feature = "test-support"))]
        Mode::InProcess => {
            let req=request(Op::ValidateLive,protocol::Actor{user_id:None,label:None,ip:None},35000);
            match crate::publisher::test_support::dispatch(state,req).await {
                OpResult::Ok{apply:Some(r),..}=>Ok(LiveValidation::Checked{ok:r.ok,output:r.output}),
                OpResult::Busy=>Ok(LiveValidation::Busy),
                OpResult::Failed{output,..}=>Ok(LiveValidation::Checked{ok:false,output}),
                OpResult::Unavailable=>Err(unavailable("publisher unavailable")),
                _=>Err(protocol_error("unexpected validation result")),
            }
        }
        Mode::Socket { .. } => readonly(state,Op::ValidateLive,|response|match response.result {
            OpResult::Ok{apply:Some(r),..}=>Ok(LiveValidation::Checked{ok:r.ok,output:r.output}),
            OpResult::Busy=>Ok(LiveValidation::Busy),
            OpResult::Failed{output,..}=>Ok(LiveValidation::Checked{ok:false,output}),
            OpResult::Unavailable=>Err(unavailable("publisher unavailable")),
            _=>Err(protocol_error("unexpected validation result")),
        }).await,
    }
}

pub async fn hello(state: &AppState) -> Result<Hello, ClientError> {
    match &state.runtime.publisher.mode {
        #[cfg(any(test, feature = "test-support"))]
        Mode::InProcess => {
            let req=request(Op::Hello,protocol::Actor{user_id:None,label:None,ip:None},2000);
            match crate::publisher::test_support::dispatch(state,req).await {
                OpResult::Ok{phase,uncertain,..}=>Ok(Hello{ready:phase=="ready",uncertain,epoch:0}),
                OpResult::Unavailable=>Err(unavailable("publisher unavailable")),
                _=>Err(protocol_error("unexpected hello result")),
            }
        }
        Mode::Socket { .. } => readonly(state,Op::Hello,|response|match response.result {
                OpResult::Ok{phase,uncertain,..}=>Ok(Hello{ready:phase=="ready",uncertain,epoch:response.epoch}),
                OpResult::Unavailable=>Err(unavailable("publisher unavailable")),
                _=>Err(protocol_error("unexpected hello result")),
        }).await,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod fake {
    use crate::publisher::protocol::{OpResult, Request, Response};
    use std::{collections::VecDeque, io, path::{Path, PathBuf}, sync::Arc, time::Duration};
    use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::{UnixListener, UnixStream}, sync::Mutex, task::{JoinHandle, JoinSet}};

    pub enum Script {
        Reply(OpResult),
        Delay(Duration, OpResult),
        CloseAfterRead,
        /// Raw response bytes, including any framing (also allows trailing-byte probes).
        Garbage(Vec<u8>),
        Length(u32),
        WrongId,
    }

    pub struct Server {
        path: PathBuf,
        task: JoinHandle<()>,
        pub requests: Arc<Mutex<Vec<Request>>>,
        pub errors: Arc<Mutex<Vec<String>>>,
        pub connections: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl Server {
        pub async fn start(script: impl IntoIterator<Item = Script>) -> io::Result<Self> {
            use std::os::unix::fs::DirBuilderExt;
            let dir=std::env::temp_dir().join(format!("nwui-pub-{}",uuid::Uuid::new_v4().simple()));
            std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
            let path=dir.join("control.sock");
            let listener=match UnixListener::bind(&path) {
                Ok(listener)=>listener,
                Err(error)=>{let _=std::fs::remove_dir(&dir);return Err(error);}
            };
            let requests=Arc::new(Mutex::new(Vec::new()));
            let errors=Arc::new(Mutex::new(Vec::new()));
            let recorded=requests.clone();let failures=errors.clone();
            let connections_count=Arc::new(std::sync::atomic::AtomicUsize::new(0));let accepted_count=connections_count.clone();
            let script=Arc::new(Mutex::new(script.into_iter().collect::<VecDeque<_>>()));
            let task=tokio::spawn(async move {
                let mut connections=JoinSet::new();
                loop {
                    tokio::select! {
                        accepted=listener.accept()=>{
                            let Ok((stream,_))=accepted else {break;};
                            accepted_count.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
                            let (recorded,failures,script)=(recorded.clone(),failures.clone(),script.clone());
                            connections.spawn(async move {
                                if let Err(error)=serve(stream,recorded,script).await {failures.lock().await.push(error.to_string());}
                            });
                        }
                        _=connections.join_next(),if !connections.is_empty()=>{}
                    }
                }
            });
            Ok(Self { path, task, requests, errors, connections:connections_count })
        }
        pub fn path(&self) -> &Path { &self.path }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
            let _=std::fs::remove_file(&self.path);
            if let Some(parent)=self.path.parent(){let _=std::fs::remove_dir(parent);}
        }
    }

    async fn read_request(stream:&mut UnixStream)->io::Result<Request> {
        let read=async {
            let length=stream.read_u32().await? as usize;
            if !(1..=65_536).contains(&length){return Err(io::Error::new(io::ErrorKind::InvalidData,"request length"));}
            let mut body=vec![0;length];stream.read_exact(&mut body).await?;
            let mut extra=[0];
            if stream.read(&mut extra).await?!=0{return Err(io::Error::new(io::ErrorKind::InvalidData,"no half-close"));}
            serde_json::from_slice(&body).map_err(|error|io::Error::new(io::ErrorKind::InvalidData,error))
        };
        tokio::time::timeout(Duration::from_secs(2),read).await
            .map_err(|_|io::Error::new(io::ErrorKind::TimedOut,"no half-close"))?
    }
    async fn serve(mut stream:UnixStream,requests:Arc<Mutex<Vec<Request>>>,script:Arc<Mutex<VecDeque<Script>>>)->io::Result<()> {
        let request=read_request(&mut stream).await?;
        let id=request.id.clone();requests.lock().await.push(request);
        let action=script.lock().await.pop_front().unwrap_or(Script::Reply(OpResult::Unknown));
        let result=match action {
            Script::Reply(result)=>result,
            Script::Delay(delay,result)=>{tokio::time::sleep(delay).await;result}
            Script::CloseAfterRead=>return Ok(()),
            Script::Garbage(bytes)=>return stream.write_all(&bytes).await,
            Script::Length(length)=>return stream.write_u32(length).await,
            Script::WrongId=>{
                return reply(&mut stream,uuid::Uuid::new_v4().to_string(),OpResult::Unknown).await;
            }
        };
        reply(&mut stream,id,result).await
    }
    async fn reply(stream:&mut UnixStream,id:String,result:OpResult)->io::Result<()> {
        let body=serde_json::to_vec(&Response {v:1,id,epoch:1,result})
            .map_err(|error|io::Error::new(io::ErrorKind::InvalidData,error))?;
        stream.write_u32(body.len() as u32).await?;
        stream.write_all(&body).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::RuntimeConfig,publisher::protocol::{self,Op,OpResult}};
    use tokio::io::{AsyncReadExt,AsyncWriteExt};

    fn state(dir:&std::path::Path)->AppState {
        let mut config=RuntimeConfig::read(|key|match key {
            "DATABASE_URL"=>Some("postgres://test@127.0.0.1/test".into()),
            "AUTH_SECRET"=>Some("publisher-client-test-only".into()),
            _=>None,
        }).unwrap();
        config.data_key_file=dir.join("key");
        config.nginx_conf_dir=dir.join("conf");
        config.nginx_bin="/bin/true".into();
        std::fs::write(&config.data_key_file,"A5".repeat(32)).unwrap();
        AppState::new(config).unwrap()
    }
    fn request()->protocol::Request {
        protocol::Request {v:1,id:uuid::Uuid::new_v4().to_string(),op:Op::Hello,deadline_ms:2000,actor:protocol::Actor{user_id:None,label:None,ip:None}}
    }
    #[test]
    fn tests_default_to_in_process_and_system_remains_socket() {
        assert!(matches!(Client::default().mode,Mode::InProcess));
        assert!(matches!(Client::system().mode,Mode::Socket{checks:true,..}));
    }
    #[tokio::test]
    async fn in_process_hello_goes_through_the_real_store_dispatcher() {
        let dir=tempfile::tempdir().unwrap();let state=state(dir.path());
        assert!(hello(&state).await.unwrap().ready);
        assert!(dir.path().join(".nwui-publisher/manifest.json").exists());
    }
    #[tokio::test]
    async fn in_process_validation_does_not_take_the_apply_lock() {
        let dir=tempfile::tempdir().unwrap();let state=state(dir.path());
        let _apply=state.runtime.apply_lock.lock().await;
        let result=tokio::time::timeout(Duration::from_secs(5),validate_live(&state)).await.unwrap().unwrap();
        assert!(matches!(result,LiveValidation::Checked{ok:true,..}));
    }
    #[tokio::test]
    async fn fake_observes_eof_and_records_the_typed_request() {
        let server=fake::Server::start([fake::Script::Reply(OpResult::Ok{apply:None,skipped:vec![],uncertain:false,phase:"ready".into()})]).await.unwrap();
        let req=request();let body=serde_json::to_vec(&req).unwrap();
        let mut stream=tokio::net::UnixStream::connect(server.path()).await.unwrap();
        stream.write_u32(body.len()as u32).await.unwrap();stream.write_all(&body).await.unwrap();stream.shutdown().await.unwrap();
        let length=stream.read_u32().await.unwrap();let mut body=vec![0;length as usize];stream.read_exact(&mut body).await.unwrap();
        let response:protocol::Response=serde_json::from_slice(&body).unwrap();assert_eq!(response.id,req.id);
        assert_eq!(server.requests.lock().await.len(),1);assert!(server.errors.lock().await.is_empty());
    }
    #[tokio::test]
    async fn fake_requires_the_client_half_close() {
        let server=fake::Server::start([fake::Script::Reply(OpResult::Unknown)]).await.unwrap();
        let body=serde_json::to_vec(&request()).unwrap();
        let mut stream=tokio::net::UnixStream::connect(server.path()).await.unwrap();
        stream.write_u32(body.len()as u32).await.unwrap();stream.write_all(&body).await.unwrap();
        tokio::time::timeout(Duration::from_secs(4),async {
            loop {
                if !server.errors.lock().await.is_empty(){break;}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        assert!(server.requests.lock().await.is_empty());
        assert_eq!(server.errors.lock().await.as_slice(),["no half-close"]);
    }
    #[tokio::test]
    async fn fake_supports_every_script_without_interpreting_raw_bytes() {
        let server=fake::Server::start([
            fake::Script::Delay(Duration::from_millis(5),OpResult::Unknown),
            fake::Script::CloseAfterRead,
            fake::Script::Garbage(vec![1,2,3]),
            fake::Script::Length(1_048_577),
            fake::Script::WrongId,
        ]).await.unwrap();
        for i in 0..5 {
            let req=request();let body=serde_json::to_vec(&req).unwrap();
            let mut stream=tokio::net::UnixStream::connect(server.path()).await.unwrap();
            stream.write_u32(body.len()as u32).await.unwrap();stream.write_all(&body).await.unwrap();stream.shutdown().await.unwrap();
            let mut bytes=Vec::new();stream.read_to_end(&mut bytes).await.unwrap();
            match i {
                0=>{let response:protocol::Response=serde_json::from_slice(&bytes[4..]).unwrap();assert!(matches!(response.result,OpResult::Unknown));}
                1=>assert!(bytes.is_empty()),
                2=>assert_eq!(bytes,vec![1,2,3]),
                3=>assert_eq!(bytes,1_048_577u32.to_be_bytes()),
                _=>{let response:protocol::Response=serde_json::from_slice(&bytes[4..]).unwrap();assert_ne!(response.id,req.id);}
            }
        }
        assert_eq!(server.requests.lock().await.len(),5);assert!(server.errors.lock().await.is_empty());
    }
    #[tokio::test]
    async fn noncanonical_rollback_returns_not_found_without_io() {
        let dir=tempfile::tempdir().unwrap();let mut state=state(dir.path());
        std::sync::Arc::get_mut(&mut state.runtime).unwrap().publisher=Client::system();
        let actor=Actor{id:None,label:None,ip:None};
        for id in ["missing","AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA"] {
            let result=rollback_snapshot(&state,id,&actor).await.unwrap();
            assert!(!result.result.ok);assert!(matches!(result.kind,Some(RollbackKind::NotFound)));
        }
    }
    fn fast_bounds() -> Bounds {
        Bounds { admission:Duration::from_millis(100),operation:Duration::from_millis(150),connect:Duration::from_millis(100),write:Duration::from_millis(100),slack:Duration::from_millis(50),status:Duration::from_millis(150),validate:Duration::from_millis(150),hello:Duration::from_millis(150) }
    }
    fn socket_state(dir:&std::path::Path,server:&fake::Server)->AppState {
        let mut state=state(dir);
        std::sync::Arc::get_mut(&mut state.runtime).unwrap().publisher=Client::socket_for_tests(server.path().into(),fast_bounds());
        state
    }
    fn actor()->Actor {Actor{id:Some("user-only".into()),label:Some("operator-only".into()),ip:Some("192.0.2.1".into())}}
    fn success()->OpResult {OpResult::Ok{apply:Some(ApplyResult{ok:true,output:"applied".into(),rolled_back_to:Some(crate::control::model::RestoredCounts{conf:1,stream:2})}),skipped:vec![],uncertain:false,phase:"done".into()}}
    fn failed(code:&str)->OpResult {OpResult::Failed{code:code.into(),output:"original output".into()}}
    fn client_error(error:&anyhow::Error)->&ClientError {error.downcast_ref::<ClientError>().expect("typed client error")}

    #[tokio::test]
    async fn socket_mutations_map_success_and_preserve_actor_and_fields() {
        let server=fake::Server::start([fake::Script::Reply(success()),fake::Script::Reply(success()),fake::Script::Reply(success())]).await.unwrap();
        let dir=tempfile::tempdir().unwrap();let state=socket_state(dir.path(),&server);let snapshot=uuid::Uuid::new_v4().to_string();
        let applied=apply_saved(&state,&actor(),Some("label")).await.unwrap();assert!(applied.ok);let counts=applied.rolled_back_to.unwrap();assert_eq!((counts.conf,counts.stream),(1,2));
        assert!(rollback_snapshot(&state,&snapshot,&actor()).await.unwrap().kind.is_none());
        assert!(deploy_certificate(&state,"certificate","digest",&actor()).await.unwrap().ok);
        let requests=server.requests.lock().await;assert_eq!(requests.len(),3);
        assert!(matches!(&requests[0].op,Op::ApplySaved{label,expected_saved_digest:None}if label.as_deref()==Some("label")));
        assert!(matches!(&requests[1].op,Op::RollbackSnapshot{snapshot_id}if snapshot_id==&snapshot));
        assert!(matches!(&requests[2].op,Op::DeployCertificate{certificate_id,content_digest}if certificate_id=="certificate"&&content_digest=="digest"));
        for req in requests.iter(){assert_eq!(req.v,1);assert!(uuid::Uuid::parse_str(&req.id).is_ok());assert_eq!(req.actor.user_id,actor().id);assert_eq!(req.actor.label,actor().label);assert_eq!(req.actor.ip,actor().ip);assert!((1..=150).contains(&req.deadline_ms));}
        assert!(server.errors.lock().await.is_empty());
    }
    #[tokio::test]
    async fn socket_storage_full_preserves_operator_output_as_apply_failure() {
        let output="설정 보관 공간이 부족해 적용하지 않았습니다. 현재 설정은 그대로입니다. 디스크 공간을 확보한 뒤 다시 시도하세요.";
        for operation in 0..3 {
            let server=fake::Server::start([fake::Script::Reply(OpResult::Failed{code:"publisher_storage_full".into(),output:output.into()})]).await.unwrap();
            let dir=tempfile::tempdir().unwrap();let state=socket_state(dir.path(),&server);
            let result=match operation {
                0=>apply_saved(&state,&actor(),None).await.map(|result|RollbackResult{result,kind:None}),
                1=>deploy_certificate(&state,"id","digest",&actor()).await.map(|result|RollbackResult{result,kind:None}),
                _=>rollback_snapshot(&state,&uuid::Uuid::new_v4().to_string(),&actor()).await,
            }.unwrap();
            assert!(!result.result.ok);assert_eq!(result.result.output,output);assert!(result.result.rolled_back_to.is_none());assert!(result.kind.is_none());
            assert_eq!(server.requests.lock().await.len(),1);
        }
    }
    #[tokio::test]
    async fn socket_mutation_result_table_every_row() {
        for operation in 0..3 {
            let replies=vec![OpResult::Ok{apply:None,skipped:vec![],uncertain:false,phase:"done".into()},OpResult::Busy,OpResult::StaleMaterial,failed("snapshot_not_found"),failed("db_restore_failed"),failed("response_too_large"),failed("other"),OpResult::Unavailable,OpResult::Unknown,OpResult::Conflict,OpResult::BadRequest{code:"invalid".into()},failed("unconfirmed")];
            for (index,reply) in replies.into_iter().enumerate(){
                let server=fake::Server::start([fake::Script::Reply(reply)]).await.unwrap();let dir=tempfile::tempdir().unwrap();let state=socket_state(dir.path(),&server);
                let result=match operation {
                    0=>apply_saved(&state,&actor(),None).await.map(|r|RollbackResult{result:r,kind:None}),
                    1=>deploy_certificate(&state,"id","digest",&actor()).await.map(|r|RollbackResult{result:r,kind:None}),
                    _=>rollback_snapshot(&state,&uuid::Uuid::new_v4().to_string(),&actor()).await,
                };
                if operation==2&&(index==3||index==4){let r=result.unwrap();assert!(!r.result.ok);assert_eq!(r.result.output,"original output");assert!(r.result.rolled_back_to.is_none());assert!(match r.kind{Some(RollbackKind::NotFound)=>index==3,Some(RollbackKind::DbRestoreFailed)=>index==4,None=>false});continue;}
                let error=result.err().expect("must fail");
                match index {
                    1=>assert_eq!(error.to_string(),"설정 적용 대기가 30초를 넘었습니다. 잠시 후 다시 시도하세요."),
                    2 if operation!=2=>assert_eq!(error.to_string(),"stale_material"),
                    5|11=>assert!(matches!(client_error(&error),ClientError::Unconfirmed{..})),
                    6=>assert_eq!(error.to_string(),"original output"),
                    7=>assert!(matches!(client_error(&error),ClientError::Unavailable{..})),
                    _=>assert!(matches!(client_error(&error),ClientError::Protocol{..}),"op {operation}, row {index}"),
                }
                assert_eq!(server.requests.lock().await.len(),1,"mapping never polls");
            }
        }
    }
    #[tokio::test]
    async fn socket_drop_recovers_once_with_original_id_or_is_unconfirmed() {
        for reply in [success(),OpResult::Ok{apply:None,skipped:vec![],uncertain:false,phase:"running".into()},OpResult::Unknown,OpResult::Busy,OpResult::Unavailable,OpResult::Conflict,OpResult::BadRequest{code:"status_error".into()}] {
            let stored=matches!(reply,OpResult::Ok{apply:Some(_),..});
            let server=fake::Server::start([fake::Script::CloseAfterRead,fake::Script::Reply(reply)]).await.unwrap();let dir=tempfile::tempdir().unwrap();let state=socket_state(dir.path(),&server);
            let result=apply_saved(&state,&actor(),None).await;
            let requests=server.requests.lock().await;assert_eq!(requests.len(),2);
            assert!(matches!(&requests[1].op,Op::OperationStatus{operation_id}if operation_id==&requests[0].id));assert_ne!(requests[0].id,requests[1].id);assert_eq!(requests[1].deadline_ms,150);
            if stored {assert!(result.unwrap().ok);} else {assert!(matches!(client_error(&result.err().unwrap()),ClientError::Unconfirmed{operation_id}if operation_id==&requests[0].id));}
        }
    }
    #[tokio::test]
    async fn socket_read_failures_recover_but_never_retry_the_mutation() {
        for script in [fake::Script::WrongId,fake::Script::Garbage(vec![0,0,0,1,b'{']),fake::Script::Length(0),fake::Script::Length(1_048_577),fake::Script::Garbage(vec![0,0]),fake::Script::Delay(Duration::from_secs(1),success())] {
            let server=fake::Server::start([script,fake::Script::Reply(success())]).await.unwrap();let dir=tempfile::tempdir().unwrap();let state=socket_state(dir.path(),&server);
            assert!(apply_saved(&state,&actor(),None).await.unwrap().ok);assert_eq!(server.requests.lock().await.len(),2);
        }
    }
    #[tokio::test]
    async fn socket_status_error_is_unconfirmed_without_polling() {
        let server=fake::Server::start([fake::Script::CloseAfterRead,fake::Script::WrongId]).await.unwrap();let dir=tempfile::tempdir().unwrap();let state=socket_state(dir.path(),&server);
        assert!(matches!(client_error(&apply_saved(&state,&actor(),None).await.err().unwrap()),ClientError::Unconfirmed{..}));assert_eq!(server.requests.lock().await.len(),2);
    }
    #[tokio::test]
    async fn socket_oversize_request_is_protocol_before_connect() {
        let server=fake::Server::start([]).await.unwrap();let dir=tempfile::tempdir().unwrap();let state=socket_state(dir.path(),&server);
        let error=apply_saved(&state,&actor(),Some(&"x".repeat(65_536))).await.err().unwrap();assert!(matches!(client_error(&error),ClientError::Protocol{..}));assert!(server.requests.lock().await.is_empty());assert!(server.errors.lock().await.is_empty());assert_eq!(server.connections.load(std::sync::atomic::Ordering::SeqCst),0);
    }
    #[tokio::test]
    async fn socket_readonly_lengths_are_protocol_without_status() {
        for length in [0,1_048_577] {
            let server=fake::Server::start([fake::Script::Length(length)]).await.unwrap();let dir=tempfile::tempdir().unwrap();let state=socket_state(dir.path(),&server);
            assert!(matches!(validate_live(&state).await,Err(ClientError::Protocol{..})));assert_eq!(server.requests.lock().await.len(),1);
        }
    }
    #[tokio::test]
    async fn socket_readonly_mapping_and_no_apply_lock() {
        for (index,reply) in [success(),OpResult::Busy,failed("spawn"),OpResult::Unavailable,OpResult::Unknown].into_iter().enumerate(){
            let server=fake::Server::start([fake::Script::Reply(reply)]).await.unwrap();let dir=tempfile::tempdir().unwrap();let state=socket_state(dir.path(),&server);let _lock=state.runtime.apply_lock.lock().await;
            let result=validate_live(&state).await;
            match index{0=>assert_eq!(result.unwrap(),LiveValidation::Checked{ok:true,output:"applied".into()}),1=>assert_eq!(result.unwrap(),LiveValidation::Busy),2=>assert_eq!(result.unwrap(),LiveValidation::Checked{ok:false,output:"original output".into()}),3=>assert!(matches!(result,Err(ClientError::Unavailable{..}))),_=>assert!(matches!(result,Err(ClientError::Protocol{..})))}
            let requests=server.requests.lock().await;assert_eq!(requests.len(),1);assert!(matches!(requests[0].op,Op::ValidateLive));assert_eq!(requests[0].deadline_ms,150);
        }
        for phase in ["ready","starting"] {
            let server=fake::Server::start([fake::Script::Reply(OpResult::Ok{apply:None,skipped:vec![],uncertain:true,phase:phase.into()})]).await.unwrap();let dir=tempfile::tempdir().unwrap();let state=socket_state(dir.path(),&server);let _lock=state.runtime.apply_lock.lock().await;
            assert_eq!(hello(&state).await.unwrap(),Hello{ready:phase=="ready",uncertain:true,epoch:1});assert_eq!(server.requests.lock().await[0].deadline_ms,2000);
        }
    }
    #[tokio::test]
    async fn socket_apply_lock_is_held_and_admission_is_bounded() {
        let server=fake::Server::start([fake::Script::Delay(Duration::from_millis(80),success())]).await.unwrap();let dir=tempfile::tempdir().unwrap();let state=socket_state(dir.path(),&server);
        let check=async{tokio::time::timeout(Duration::from_secs(2),async{while server.requests.lock().await.is_empty(){tokio::task::yield_now().await;}}).await.unwrap();assert!(state.runtime.apply_lock.try_lock().is_err());};
        let operator=actor();let (result,())=tokio::join!(apply_saved(&state,&operator,None),check);assert!(result.unwrap().ok);
        let _lock=state.runtime.apply_lock.lock().await;
        let error=apply_saved(&state,&actor(),None).await.err().unwrap();assert_eq!(error.to_string(),"설정 적용 대기가 30초를 넘었습니다. 잠시 후 다시 시도하세요.");assert_eq!(server.requests.lock().await.len(),1);
    }
    #[tokio::test]
    async fn system_checks_refuse_writable_parent_and_nonroot_socket_owner() {
        use std::os::unix::fs::PermissionsExt;
        let server=fake::Server::start([]).await.unwrap();let dir=tempfile::tempdir().unwrap();let mut state=socket_state(dir.path(),&server);
        std::sync::Arc::get_mut(&mut state.runtime).unwrap().publisher=Client{mode:Mode::Socket{path:server.path().into(),bounds:fast_bounds(),checks:true}};
        let parent=server.path().parent().unwrap();std::fs::set_permissions(parent,std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(matches!(client_error(&apply_saved(&state,&actor(),None).await.err().unwrap()),ClientError::Unavailable{..}));
        std::fs::set_permissions(parent,std::fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::chown(server.path(),Some(10001),None).unwrap();
        assert!(matches!(client_error(&apply_saved(&state,&actor(),None).await.err().unwrap()),ClientError::Unavailable{..}));assert!(server.requests.lock().await.is_empty());
    }

    #[test]
    fn socket_deadline_arithmetic_and_clamp() {
        let now=Instant::now();assert_eq!(deadline_ms(now,now),1);assert_eq!(deadline_ms(now-Duration::from_secs(1),now),1);
        assert_eq!(deadline_ms(now+Duration::from_millis(125),now),125);assert_eq!(deadline_ms(now+Duration::from_secs(300),now),280_000);
        assert_eq!(deadline_ms(now+Duration::from_secs(280),now+Duration::from_secs(30)),250_000);
    }
    #[tokio::test]
    async fn socket_partial_write_is_unavailable_and_not_eligible_for_status() {
        let (mut client,mut peer)=tokio::io::duplex(8);
        let closed=tokio::spawn(async move {let mut prefix=[0;4];peer.read_exact(&mut prefix).await.unwrap();});
        let result=framed_exchange(&mut client,&vec![0;65_540],"operation",Instant::now()+Duration::from_secs(1),Instant::now()+Duration::from_secs(1)).await;
        closed.await.unwrap();let error=result.err().expect("short write");assert!(!error.sent);assert!(matches!(error.error,ClientError::Unavailable{..}));
        let server=fake::Server::start([]).await.unwrap();
        let error=recover_response(server.path(),&fast_bounds(),false,&request(),Err(error)).await.err().unwrap();
        assert!(matches!(client_error(&error),ClientError::Unavailable{..}));assert_eq!(server.connections.load(std::sync::atomic::Ordering::SeqCst),0);assert!(server.requests.lock().await.is_empty());
    }
    #[tokio::test]
    async fn socket_reads_exactly_one_frame_without_waiting_for_eof_or_trailer() {
        let (mut client,mut peer)=tokio::io::duplex(1024);
        let body=serde_json::to_vec(&Response{v:1,id:"operation".into(),epoch:999,result:success()}).unwrap();
        peer.write_u32(body.len()as u32).await.unwrap();peer.write_all(&body).await.unwrap();peer.write_all(b"trailing").await.unwrap();
        let result=tokio::time::timeout(Duration::from_millis(100),read_response(&mut client,"operation")).await.unwrap().unwrap();assert_eq!(result.epoch,999);
        let mut remainder=[0;8];client.read_exact(&mut remainder).await.unwrap();assert_eq!(&remainder,b"trailing");
    }
    #[tokio::test]
    async fn socket_missing_peer_and_readonly_timeout_do_not_poll() {
        let dir=tempfile::tempdir().unwrap();let mut state=state(dir.path());std::sync::Arc::get_mut(&mut state.runtime).unwrap().publisher=Client::socket_for_tests(dir.path().join("absent.sock"),fast_bounds());
        assert!(matches!(client_error(&apply_saved(&state,&actor(),None).await.err().unwrap()),ClientError::Unavailable{..}));
        let server=fake::Server::start([fake::Script::Delay(Duration::from_secs(1),success())]).await.unwrap();let state=socket_state(dir.path(),&server);
        assert!(matches!(hello(&state).await,Err(ClientError::Unavailable{..})));assert_eq!(server.requests.lock().await.len(),1);
    }
    #[tokio::test]
    async fn socket_status_timeout_is_bounded_and_never_retried() {
        let server=fake::Server::start([fake::Script::CloseAfterRead,fake::Script::Delay(Duration::from_secs(1),success())]).await.unwrap();let dir=tempfile::tempdir().unwrap();let state=socket_state(dir.path(),&server);
        let start=Instant::now();assert!(matches!(client_error(&apply_saved(&state,&actor(),None).await.err().unwrap()),ClientError::Unconfirmed{..}));assert!(start.elapsed()<Duration::from_secs(1));assert_eq!(server.requests.lock().await.len(),2);
    }

    #[tokio::test]
    async fn system_checks_accept_root_peer_and_refuse_nonroot_parent_or_symlink() {
        let server=fake::Server::start([fake::Script::Reply(success())]).await.unwrap();let dir=tempfile::tempdir().unwrap();let mut state=socket_state(dir.path(),&server);
        std::sync::Arc::get_mut(&mut state.runtime).unwrap().publisher=Client{mode:Mode::Socket{path:server.path().into(),bounds:fast_bounds(),checks:true}};
        assert!(apply_saved(&state,&actor(),None).await.unwrap().ok);
        let parent=server.path().parent().unwrap();std::os::unix::fs::chown(parent,Some(10001),None).unwrap();
        assert!(matches!(client_error(&apply_saved(&state,&actor(),None).await.err().unwrap()),ClientError::Unavailable{..}));
        std::os::unix::fs::chown(parent,Some(0),None).unwrap();
        let link=dir.path().join("control.sock");std::os::unix::fs::symlink(server.path(),&link).unwrap();
        std::sync::Arc::get_mut(&mut state.runtime).unwrap().publisher=Client{mode:Mode::Socket{path:link,bounds:fast_bounds(),checks:true}};
        assert!(matches!(client_error(&apply_saved(&state,&actor(),None).await.err().unwrap()),ClientError::Unavailable{..}));assert_eq!(server.requests.lock().await.len(),1);
    }

    #[tokio::test]
    async fn socket_refuses_wrong_response_version() {
        let (mut client,mut peer)=tokio::io::duplex(1024);
        let body=serde_json::to_vec(&Response{v:2,id:"operation".into(),epoch:1,result:success()}).unwrap();peer.write_u32(body.len()as u32).await.unwrap();peer.write_all(&body).await.unwrap();
        assert!(matches!(read_response(&mut client,"operation").await,Err(ClientError::Protocol{..})));
    }

    #[tokio::test]
    async fn socket_noncanonical_rollback_never_connects() {
        let server=fake::Server::start([]).await.unwrap();let dir=tempfile::tempdir().unwrap();let state=socket_state(dir.path(),&server);
        let result=rollback_snapshot(&state,"missing",&actor()).await.unwrap();assert!(matches!(result.kind,Some(RollbackKind::NotFound)));assert_eq!(server.connections.load(std::sync::atomic::Ordering::SeqCst),0);
    }
    #[tokio::test]
    async fn socket_status_maps_a_stored_failure_through_the_same_table() {
        let server=fake::Server::start([fake::Script::CloseAfterRead,fake::Script::Reply(failed("db_restore_failed"))]).await.unwrap();let dir=tempfile::tempdir().unwrap();let state=socket_state(dir.path(),&server);
        let result=rollback_snapshot(&state,&uuid::Uuid::new_v4().to_string(),&actor()).await.unwrap();assert!(matches!(result.kind,Some(RollbackKind::DbRestoreFailed)));assert!(!result.result.ok);assert_eq!(result.result.output,"original output");assert!(result.result.rolled_back_to.is_none());assert_eq!(server.requests.lock().await.len(),2);
    }

    #[test]
    fn socket_failure_log_remains_one_line_for_multiline_outputs() {
        let error=anyhow::anyhow!("line one\nline two\r\nlast\tline");let error_ref:&(dyn std::error::Error+'static)=error.as_ref();let line=failure_line("ApplySaved","operation",error_ref);
        assert_eq!(line.lines().count(),1);assert!(line.starts_with("[publisher-client] ApplySaved operation: "));
        assert!(line.contains("line one\\nline two\\r\\nlast\\tline"));
    }

    fn raw_reply(id:&str,result:OpResult,version:u8)->Vec<u8> {
        let body=serde_json::to_vec(&Response{v:version,id:id.into(),epoch:1,result}).unwrap();
        let mut bytes=(body.len()as u32).to_be_bytes().to_vec();bytes.extend(body);bytes
    }
    #[tokio::test]
    async fn empty_id_bad_request_is_protocol_without_a_status_lookup() {
        for op in 0..3 {
            let server=fake::Server::start([fake::Script::Garbage(raw_reply("",OpResult::BadRequest{code:"invalid_request".into()},1))]).await.unwrap();
            let dir=tempfile::tempdir().unwrap();let state=socket_state(dir.path(),&server);
            let error=match op {
                0=>apply_saved(&state,&actor(),None).await.err().unwrap(),
                1=>deploy_certificate(&state,"certificate","digest",&actor()).await.err().unwrap(),
                _=>rollback_snapshot(&state,&uuid::Uuid::new_v4().to_string(),&actor()).await.err().unwrap(),
            };
            assert!(matches!(client_error(&error),ClientError::Protocol{..}),"empty-id BadRequest must not become Unconfirmed");
            assert_eq!(server.requests.lock().await.len(),1);assert_eq!(server.connections.load(std::sync::atomic::Ordering::SeqCst),1);
        }
    }
    #[tokio::test]
    async fn empty_id_exception_is_only_for_v1_bad_request() {
        for (id,result,version)in [("",success(),1),("wrong",OpResult::BadRequest{code:"invalid_request".into()},1),("",OpResult::BadRequest{code:"invalid_request".into()},2)] {
            let server=fake::Server::start([fake::Script::Garbage(raw_reply(id,result,version)),fake::Script::Reply(success())]).await.unwrap();
            let dir=tempfile::tempdir().unwrap();let state=socket_state(dir.path(),&server);
            assert!(apply_saved(&state,&actor(),None).await.unwrap().ok);assert_eq!(server.requests.lock().await.len(),2);
        }
    }
    #[test]
    fn failure_log_preserves_transport_detail_without_actor_fields() {
        for error in [ClientError::Unavailable{detail:"connect timed out".into()},ClientError::Unavailable{detail:"unsafe socket owner".into()},ClientError::Unavailable{detail:"peer is not root".into()},ClientError::Protocol{detail:"response length outside 1..=1 MiB".into()}] {
            let wanted=match &error{ClientError::Unavailable{detail}|ClientError::Protocol{detail}=>detail.clone(),_=>unreachable!()};
            let error=anyhow::Error::from(error);let error_ref:&(dyn std::error::Error+'static)=error.as_ref();let line=failure_line("ApplySaved","operation",error_ref);
            assert!(line.contains(&wanted),"lost diagnostic detail: {line}");assert_eq!(line.lines().count(),1);
            for value in ["user-only","operator-only","192.0.2.1"]{assert!(!line.contains(value));}
        }
    }

}
