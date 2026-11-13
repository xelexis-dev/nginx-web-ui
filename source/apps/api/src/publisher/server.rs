//! The bounded v1 framing used by the root-only publisher socket.
use super::{protocol::{Op,OpResult,Request,Response}, store::uuid};
use anyhow::{Result, ensure};
use tokio::{io::{AsyncReadExt,AsyncWriteExt},net::UnixStream};
use std::time::Duration;

const MAX_REQUEST:usize=64*1024;
pub(super) const MAX_RESPONSE:usize=1024*1024;
const IO_TIMEOUT:Duration=Duration::from_secs(2);

fn sha256(value:&str) -> Result<()> {
    ensure!(value.len()==64 && value.bytes().all(|byte|byte.is_ascii_hexdigit()),"Invalid content digest");
    Ok(())
}

fn decode(bytes:&[u8]) -> Result<Request> {
    ensure!(!bytes.is_empty() && bytes.len()<=MAX_REQUEST,"Invalid publisher frame size");
    let request:Request=serde_json::from_slice(bytes).map_err(|_|anyhow::anyhow!("Invalid publisher request"))?;
    ensure!(request.v==1 && (1..=280000).contains(&request.deadline_ms),"Invalid publisher version or deadline");
    uuid(&request.id)?;
    match &request.op {
        Op::ApplySaved {expected_saved_digest:Some(digest),..}=>sha256(digest)?,
        Op::RollbackSnapshot {snapshot_id}=>uuid(snapshot_id)?,
        Op::DeployCertificate {certificate_id,content_digest}=>{uuid(certificate_id)?;sha256(content_digest)?;},
        Op::OperationStatus {operation_id}=>uuid(operation_id)?,
        _=>{},
    }
    Ok(request)
}

/// The client half-closes its write side; EOF proves there is no trailing command.
pub(super) async fn read_request(stream:&mut UnixStream) -> Result<Request> {
    tokio::time::timeout(IO_TIMEOUT,async {
        let mut size=[0;4];stream.read_exact(&mut size).await?;
        let size=u32::from_be_bytes(size) as usize;
        ensure!(size>0 && size<=MAX_REQUEST,"Invalid publisher frame size");
        let mut bytes=vec![0;size];stream.read_exact(&mut bytes).await?;
        let mut trailing=[0;1];
        ensure!(stream.read(&mut trailing).await?==0,"Trailing publisher request data");
        decode(&bytes)
    }).await.map_err(|_|anyhow::anyhow!("Publisher request timed out"))?
}

fn truncate(value:&mut String) {
    const MARKER:&str="\n[출력이 잘렸습니다. 전체 기록은 발행기 저장소에 보관됩니다.]";
    let limit=MAX_RESPONSE/8;
    if value.len()>limit {
        let mut end=limit;
        while !value.is_char_boundary(end) {end-=1;}
        value.truncate(end);value.push_str(MARKER);
    }
}

fn encode(response:&mut Response) -> Result<Vec<u8>> {
    ensure!(response.id.len()<=64,"Invalid response identity");
    loop {
        let bytes=serde_json::to_vec(&response)?;
        if bytes.len()<=MAX_RESPONSE {return Ok(bytes);}
        match &mut response.result {
            OpResult::Failed {output,..} if output.len()>MAX_RESPONSE/8=>truncate(output),
            OpResult::Ok {apply:Some(apply),..} if apply.output.len()>MAX_RESPONSE/8=>truncate(&mut apply.output),
            OpResult::Ok {skipped,..} if !skipped.is_empty()=>skipped.truncate(skipped.len()/2),
            _=>{
                response.result=OpResult::Failed {code:"response_too_large".into(),output:"응답 크기 제한을 초과했습니다. 발행기 기록을 확인하세요.".into()};
            }
        }
    }
}

pub(super) async fn write_response(stream:&mut UnixStream,response:&mut Response) -> Result<()> {
    let bytes=encode(response)?;
    tokio::time::timeout(IO_TIMEOUT,async {
        stream.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
        stream.write_all(&bytes).await?;
        stream.shutdown().await?;
        Ok::<_,anyhow::Error>(())
    }).await.map_err(|_|anyhow::anyhow!("Publisher response timed out"))?
}


pub(super) struct BoundSocket {
    listener:tokio::net::UnixListener,
    path:std::path::PathBuf,
    identity:(u64,u64),
    _owner:std::fs::File,
}
impl BoundSocket {
    pub(super) fn bind()->Result<Self> {Self::bind_at(std::path::Path::new("/run/nwui-publisher"))}
    fn bind_at(parent:&std::path::Path)->Result<Self> {
        use std::{os::{unix::fs::{DirBuilderExt,MetadataExt,OpenOptionsExt,PermissionsExt,FileTypeExt}},fs};
        ensure!(unsafe{libc::geteuid()}==0,"Publisher socket requires root");
        match fs::DirBuilder::new().mode(0o750).create(parent) {
            Ok(())=>{
                let name=std::ffi::CString::new(parent.as_os_str().as_encoded_bytes())?;
                if unsafe{libc::chown(name.as_ptr(),0,10001)}!=0 {return Err(std::io::Error::last_os_error().into());}
                fs::set_permissions(parent,fs::Permissions::from_mode(0o750))?;
            },
            Err(error) if error.kind()==std::io::ErrorKind::AlreadyExists=>{},
            Err(error)=>return Err(error.into()),
        }
        crate::db_upgrade::files::real_path(parent)?;
        let meta=fs::symlink_metadata(parent)?;
        ensure!(meta.is_dir() && meta.uid()==0 && meta.gid()==10001 && meta.mode()&0o7777==0o750,"Unsafe publisher socket directory");
        let owner=fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC).open(parent.join(".publisher.lock"))?;
        let meta=owner.metadata()?;
        ensure!(meta.is_file() && meta.uid()==0 && meta.nlink()==1 && meta.mode()&0o7777==0o600,"Unsafe publisher ownership lock");
        owner.try_lock()?;
        let path=parent.join("control.sock");
        match fs::symlink_metadata(&path) {
            Ok(meta)=>{
                ensure!(meta.file_type().is_socket() && meta.uid()==0 && meta.nlink()==1,"Refusing an unowned publisher socket path");
                fs::remove_file(&path)?;
            },
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
            Err(error)=>return Err(error.into()),
        }
        let listener=tokio::net::UnixListener::bind(&path)?;
        let meta=fs::symlink_metadata(&path)?;
        let identity=(meta.dev(),meta.ino());
        let bound=Self{listener,path,identity,_owner:owner};
        // Only the root-held ownership lock can create or remove this leaf.
        let name=std::ffi::CString::new(bound.path.as_os_str().as_encoded_bytes())?;
        if unsafe{libc::chown(name.as_ptr(),0,10001)}!=0 {return Err(std::io::Error::last_os_error().into());}
        fs::set_permissions(&bound.path,fs::Permissions::from_mode(0o660))?;
        fs::File::open(parent)?.sync_all()?;
        Ok(bound)
    }
}
impl Drop for BoundSocket {
    fn drop(&mut self) {
        use std::os::unix::fs::{MetadataExt,FileTypeExt};
        if std::fs::symlink_metadata(&self.path).is_ok_and(|meta|meta.file_type().is_socket() && meta.uid()==0 && (meta.dev(),meta.ino())==self.identity) {
            let _=std::fs::remove_file(&self.path);
        }
    }
}

async fn exchange_authenticated<F,Fut>(mut stream:UnixStream,epoch:u64,dispatch:std::sync::Arc<F>)->Result<()>
where F:Fn(Request)->Fut+Send+Sync+'static,Fut:std::future::Future<Output=OpResult>+Send {
    let mut response=match read_request(&mut stream).await {
        Ok(request)=>{let id=request.id.clone();Response{v:1,id,epoch,result:dispatch(request).await}},
        Err(_)=>Response{v:1,id:String::new(),epoch,result:OpResult::BadRequest{code:"bad_request".into()}},
    };
    write_response(&mut stream,&mut response).await
}
async fn exchange<F,Fut>(stream:UnixStream,epoch:u64,dispatch:std::sync::Arc<F>)->Result<()>
where F:Fn(Request)->Fut+Send+Sync+'static,Fut:std::future::Future<Output=OpResult>+Send {
    ensure!(stream.peer_cred()?.uid()==10001,"Publisher peer refused");
    exchange_authenticated(stream,epoch,dispatch).await
}

#[derive(Clone,Copy)]
enum PeerPolicy {
    Required,
    #[cfg(feature="test-support")]
    Fixture,
}
async fn exchange_with_policy<F,Fut>(stream:UnixStream,epoch:u64,dispatch:std::sync::Arc<F>,policy:PeerPolicy)->Result<()>
where F:Fn(Request)->Fut+Send+Sync+'static,Fut:std::future::Future<Output=OpResult>+Send {
    match policy {
        PeerPolicy::Required=>exchange(stream,epoch,dispatch).await,
        #[cfg(feature="test-support")]
        PeerPolicy::Fixture=>exchange_authenticated(stream,epoch,dispatch).await,
    }
}
#[cfg(feature="test-support")]
pub(super) fn serve_for_tests<F,Fut>(directory:&std::path::Path,epoch:u64,dispatch:F)->Result<tokio::task::JoinHandle<Result<()>>>
where F:Fn(Request)->Fut+Send+Sync+'static,Fut:std::future::Future<Output=OpResult>+Send+'static {
    let socket=BoundSocket::bind_at(directory)?;
    Ok(tokio::spawn(serve_bound_with_policy(socket,epoch,dispatch,PeerPolicy::Fixture)))
}

/// Dispatch owns its mutation/recovery admission; each accepted operation outlives client EOF.
pub(super) async fn serve_bound<F,Fut>(socket:BoundSocket,epoch:u64,dispatch:F)->Result<()>
where F:Fn(Request)->Fut+Send+Sync+'static,Fut:std::future::Future<Output=OpResult>+Send+'static {
    serve_bound_with_policy(socket,epoch,dispatch,PeerPolicy::Required).await
}
async fn serve_bound_with_policy<F,Fut>(socket:BoundSocket,epoch:u64,dispatch:F,policy:PeerPolicy)->Result<()>
where F:Fn(Request)->Fut+Send+Sync+'static,Fut:std::future::Future<Output=OpResult>+Send+'static {
    let readers=std::sync::Arc::new(tokio::sync::Semaphore::new(4));
    let responders=std::sync::Arc::new(tokio::sync::Semaphore::new(4));
    let dispatch=std::sync::Arc::new(dispatch);
    loop {
        let (stream,_)=socket.listener.accept().await?;
        let Ok(permit)=readers.clone().try_acquire_owned() else {
            if let Ok(permit)=responders.clone().try_acquire_owned() {
                tokio::spawn(async move {
                    let _permit=permit;
                    // A.21: a bounded rejected request receives Busy with its own id.
                    let _=exchange_with_policy(stream,epoch,std::sync::Arc::new(|_|async {OpResult::Busy}),policy).await;
                });
            } else {drop(stream);}
            continue;
        };
        let dispatch=dispatch.clone();
        tokio::spawn(async move {
            let _permit=permit;
            // A disconnect only affects the eventual write, never the admitted dispatch.
            let _=exchange_with_policy(stream,epoch,dispatch,policy).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> serde_json::Value {
        json!({"v":1,"id":"435a950a-6472-4d45-8bb4-eaa70360c58e","op":{"op":"Hello"},"deadlineMs":280000,"actor":{"userId":null,"label":null,"ip":null}})
    }

    #[test]
    fn n1_codec_refuses_duplicate_unknown_fields_invalid_ids_and_deadlines() {
        let valid=request();assert!(decode(&serde_json::to_vec(&valid).unwrap()).is_ok());
        let mut invalid=valid.clone();invalid["shell"]=json!("nginx -s reload");assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
        for value in [json!(0),json!(280001),json!(-1),json!(1.5)] {
            let mut invalid=valid.clone();invalid["deadlineMs"]=value;assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        let mut invalid=valid.clone();invalid["id"]=json!("../outside");assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
        let duplicate=valid.to_string().replacen("{","{\"v\":1,",1);assert!(decode(duplicate.as_bytes()).is_err());
        let mut invalid=valid;invalid["op"]=json!({"op":"DeployCertificate","args":{"certificate_id":"435a950a-6472-4d45-8bb4-eaa70360c58e","content_digest":"not-a-digest"}});assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }

    #[tokio::test]
    async fn n1_framing_accepts_one_half_closed_frame_and_refuses_trailing_or_oversized_data() {
        let bytes=serde_json::to_vec(&request()).unwrap();
        for trailing in [false,true] {
            let (mut client,mut server)=UnixStream::pair().unwrap();
            client.write_all(&(bytes.len() as u32).to_be_bytes()).await.unwrap();client.write_all(&bytes).await.unwrap();
            if trailing {client.write_all(b"x").await.unwrap();}
            client.shutdown().await.unwrap();
            assert_eq!(read_request(&mut server).await.is_err(),trailing);
        }
        let (mut client,mut server)=UnixStream::pair().unwrap();
        client.write_all(&((MAX_REQUEST+1) as u32).to_be_bytes()).await.unwrap();
        assert!(read_request(&mut server).await.is_err());
    }

    #[test]
    fn n1_response_bound_preserves_valid_unicode_and_has_a_visible_truncation_marker() {
        let mut response=Response {v:1,id:uuid::Uuid::new_v4().to_string(),epoch:1,result:OpResult::Failed {code:"fixture".into(),output:"한".repeat(MAX_RESPONSE)}};
        let bytes=encode(&mut response).unwrap();assert!(bytes.len()<=MAX_RESPONSE);
        let value:serde_json::Value=serde_json::from_slice(&bytes).unwrap();
        assert!(value["result"]["output"].as_str().unwrap().contains("잘렸습니다"));
    }

    async fn send_frame(stream:&mut UnixStream,trailing:bool) {
        let bytes=serde_json::to_vec(&request()).unwrap();
        stream.write_all(&(bytes.len() as u32).to_be_bytes()).await.unwrap();stream.write_all(&bytes).await.unwrap();
        if trailing {stream.write_all(b"x").await.unwrap();}
    }

    #[tokio::test]
    async fn serving_does_not_dispatch_until_half_close_and_refuses_trailing_bytes() {
        use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
        for trailing in [false,true] {
            let calls=Arc::new(AtomicUsize::new(0));let observed=calls.clone();
            let dispatch=Arc::new(move |_|{observed.fetch_add(1,Ordering::SeqCst);async {OpResult::Unknown}});
            let (mut client,server)=UnixStream::pair().unwrap();
            let serving=tokio::spawn(exchange_authenticated(server,7,dispatch));
            send_frame(&mut client,trailing).await;
            tokio::time::sleep(Duration::from_millis(25)).await;
            assert_eq!(calls.load(Ordering::SeqCst),0,"an open request write side is not admitted");
            client.shutdown().await.unwrap();
            let size=client.read_u32().await.unwrap();let mut bytes=vec![0;size as usize];client.read_exact(&mut bytes).await.unwrap();
            let response:Response=serde_json::from_slice(&bytes).unwrap();assert_eq!(response.epoch,7);
            assert_eq!(calls.load(Ordering::SeqCst),usize::from(!trailing));
            assert!(if trailing {matches!(response.result,OpResult::BadRequest{..})} else {matches!(response.result,OpResult::Unknown)});
            serving.await.unwrap().unwrap();
        }
    }

    #[tokio::test]
    async fn client_disconnect_does_not_cancel_an_admitted_dispatch() {
        use std::sync::{Arc,atomic::{AtomicBool,Ordering}};
        let admitted=Arc::new(tokio::sync::Notify::new());let release=Arc::new(tokio::sync::Notify::new());let completed=Arc::new(AtomicBool::new(false));
        let (mut client,server)=UnixStream::pair().unwrap();
        let dispatch=Arc::new({let admitted=admitted.clone();let release=release.clone();let completed=completed.clone();move |_| {
            let admitted=admitted.clone();let release=release.clone();let completed=completed.clone();async move {
                admitted.notify_one();release.notified().await;completed.store(true,Ordering::SeqCst);OpResult::Unknown
            }
        }});
        let serving=tokio::spawn(exchange_authenticated(server,7,dispatch));
        send_frame(&mut client,false).await;client.shutdown().await.unwrap();admitted.notified().await;
        drop(client);release.notify_one();let _=serving.await.unwrap();
        assert!(completed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn root_peer_is_refused_before_framing_or_dispatch() {
        use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
        assert_eq!(unsafe{libc::geteuid()},0,"root test container");
        let calls=Arc::new(AtomicUsize::new(0));let observed=calls.clone();
        let dispatch=Arc::new(move |_|{observed.fetch_add(1,Ordering::SeqCst);async {OpResult::Unknown}});
        let (_client,server)=UnixStream::pair().unwrap();
        assert!(exchange(server,7,dispatch).await.is_err());assert_eq!(calls.load(Ordering::SeqCst),0);
    }

    #[tokio::test]
    async fn socket_has_exact_ownership_and_a_second_owner_or_foreign_leaf_is_refused() {
        use std::os::unix::fs::MetadataExt;
        let temp=tempfile::tempdir().unwrap();let parent=temp.path().join("socket");
        let socket=BoundSocket::bind_at(&parent).unwrap();
        let meta=std::fs::metadata(&socket.path).unwrap();assert_eq!((meta.uid(),meta.gid(),meta.mode()&0o7777),(0,10001,0o660));
        let meta=std::fs::metadata(&parent).unwrap();assert_eq!((meta.uid(),meta.gid(),meta.mode()&0o7777),(0,10001,0o750));
        assert!(BoundSocket::bind_at(&parent).is_err());
        let path=socket.path.clone();drop(socket);assert!(!path.exists());
        std::fs::write(&path,b"foreign file").unwrap();assert!(BoundSocket::bind_at(&parent).is_err());assert_eq!(std::fs::read(&path).unwrap(),b"foreign file");
    }


    #[tokio::test]
    async fn missing_half_close_times_out_without_dispatch() {
        use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
        let calls=Arc::new(AtomicUsize::new(0));let observed=calls.clone();
        let dispatch=Arc::new(move |_|{observed.fetch_add(1,Ordering::SeqCst);async {OpResult::Unknown}});
        let (mut client,server)=UnixStream::pair().unwrap();
        let serving=tokio::spawn(exchange_authenticated(server,7,dispatch));
        send_frame(&mut client,false).await;
        let size=client.read_u32().await.unwrap();let mut bytes=vec![0;size as usize];client.read_exact(&mut bytes).await.unwrap();
        let response:Response=serde_json::from_slice(&bytes).unwrap();
        assert!(matches!(response.result,OpResult::BadRequest{..}));assert_eq!(calls.load(Ordering::SeqCst),0);
        serving.await.unwrap().unwrap();
    }

    #[test]
    fn api_socket_client() {
        use std::io::{Read,Write};
        let Some(path)=std::env::var_os("NWUI_SOCKET_API_CLIENT") else {return;};
        assert_eq!(unsafe{libc::setgroups(0,std::ptr::null())},0);
        assert_eq!(unsafe{libc::setgid(10001)},0);assert_eq!(unsafe{libc::setuid(10001)},0);
        if std::env::var("NWUI_SOCKET_CLIENT_MODE").as_deref()==Ok("over-cap") {
            let mut held=Vec::new();
            for _ in 0..4 {
                let mut stream=std::os::unix::net::UnixStream::connect(&path).unwrap();
                let bytes=serde_json::to_vec(&request()).unwrap();stream.write_all(&(bytes.len() as u32).to_be_bytes()).unwrap();stream.write_all(&bytes).unwrap();
                stream.shutdown(std::net::Shutdown::Write).unwrap();held.push(stream);
            }
            let mut stream=std::os::unix::net::UnixStream::connect(&path).unwrap();stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut value=request();value["id"]=json!("55555555-5555-4555-8555-555555555555");
            let bytes=serde_json::to_vec(&value).unwrap();stream.write_all(&(bytes.len() as u32).to_be_bytes()).unwrap();stream.write_all(&bytes).unwrap();stream.shutdown(std::net::Shutdown::Write).unwrap();
            let mut size=[0;4];stream.read_exact(&mut size).unwrap();let mut bytes=vec![0;u32::from_be_bytes(size) as usize];stream.read_exact(&mut bytes).unwrap();
            let response:Response=serde_json::from_slice(&bytes).unwrap();
            assert_eq!(response.id,"55555555-5555-4555-8555-555555555555");assert!(matches!(response.result,OpResult::Busy));return;
        }
        let mut stream=std::os::unix::net::UnixStream::connect(path).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let bytes=serde_json::to_vec(&request()).unwrap();
        stream.write_all(&(bytes.len() as u32).to_be_bytes()).unwrap();stream.write_all(&bytes).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let mut size=[0;4];stream.read_exact(&mut size).unwrap();let size=u32::from_be_bytes(size) as usize;assert!(size<=MAX_RESPONSE);
        let mut bytes=vec![0;size];stream.read_exact(&mut bytes).unwrap();let response:Response=serde_json::from_slice(&bytes).unwrap();
        assert_eq!(response.epoch,7);assert!(matches!(response.result,OpResult::Unknown));
    }

    #[tokio::test]
    async fn real_uid_10001_peer_passes_the_same_authenticated_exchange() {
        use std::{os::unix::fs::PermissionsExt,sync::Arc};
        let temp=tempfile::tempdir().unwrap();
        let name=std::ffi::CString::new(temp.path().as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe{libc::chown(name.as_ptr(),0,10001)},0);
        std::fs::set_permissions(temp.path(),std::fs::Permissions::from_mode(0o750)).unwrap();
        let socket=BoundSocket::bind_at(&temp.path().join("socket")).unwrap();
        let child=std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","publisher::server::tests::api_socket_client","--nocapture","--test-threads=1"])
            .env("NWUI_SOCKET_API_CLIENT",&socket.path).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
        let (stream,_)=tokio::time::timeout(Duration::from_secs(2),socket.listener.accept()).await.unwrap().unwrap();
        exchange(stream,7,Arc::new(|_|async{OpResult::Unknown})).await.unwrap();
        let output=child.wait_with_output().unwrap();assert!(output.status.success(),"{output:?}");
    }


    #[tokio::test]
    async fn over_cap_real_api_peer_gets_busy_with_its_id_without_dispatch() {
        use std::{os::unix::fs::PermissionsExt,sync::{Arc,atomic::{AtomicUsize,Ordering}}};
        let temp=tempfile::tempdir().unwrap();let name=std::ffi::CString::new(temp.path().as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe{libc::chown(name.as_ptr(),0,10001)},0);std::fs::set_permissions(temp.path(),std::fs::Permissions::from_mode(0o750)).unwrap();
        let socket=BoundSocket::bind_at(&temp.path().join("socket")).unwrap();let path=socket.path.clone();
        let calls=Arc::new(AtomicUsize::new(0));let release=Arc::new(tokio::sync::Semaphore::new(0));
        let serving=tokio::spawn(serve_bound(socket,7,{let calls=calls.clone();let release=release.clone();move |_|{
            let calls=calls.clone();let release=release.clone();async move {calls.fetch_add(1,Ordering::SeqCst);let _permit=release.acquire().await.unwrap();OpResult::Unknown}
        }}));
        let output=tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","publisher::server::tests::api_socket_client","--nocapture","--test-threads=1"])
            .env("NWUI_SOCKET_API_CLIENT",path).env("NWUI_SOCKET_CLIENT_MODE","over-cap").kill_on_drop(true).output().await.unwrap();
        assert!(output.status.success(),"{output:?}");
        tokio::time::timeout(Duration::from_secs(2),async {while calls.load(Ordering::SeqCst)<4 {tokio::task::yield_now().await;}}).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst),4,"the Busy request must not dispatch");release.add_permits(4);serving.abort();let _=serving.await;
    }

    #[tokio::test]
    async fn root_socket_killed_before_chown_is_reclaimed_but_non_root_socket_is_preserved() {
        use std::os::unix::fs::MetadataExt;
        let temp=tempfile::tempdir().unwrap();let parent=temp.path().join("socket");
        let first=BoundSocket::bind_at(&parent).unwrap();let path=first.path.clone();drop(first);
        let stale=std::os::unix::net::UnixListener::bind(&path).unwrap();drop(stale);
        let meta=std::fs::symlink_metadata(&path).unwrap();assert_eq!(meta.uid(),0);assert_ne!(meta.gid(),10001);
        let reclaimed=BoundSocket::bind_at(&parent).unwrap();drop(reclaimed);
        let stale=std::os::unix::net::UnixListener::bind(&path).unwrap();drop(stale);
        let name=std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();assert_eq!(unsafe{libc::chown(name.as_ptr(),10001,10001)},0);
        let before=std::fs::symlink_metadata(&path).unwrap();assert!(BoundSocket::bind_at(&parent).is_err());
        let after=std::fs::symlink_metadata(&path).unwrap();assert_eq!((before.ino(),before.uid(),before.gid()),(after.ino(),after.uid(),after.gid()));
    }

}
