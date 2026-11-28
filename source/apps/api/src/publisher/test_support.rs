//! Test-only thin root entry points; production never exports writer modules.
#[cfg(all(feature = "test-support", not(debug_assertions)))]
compile_error!("test-support must never be enabled in a release build");

pub use super::identity_fixture::{FakeMaster, RegisteredMaster};

/// Build the same journal-owned S4-prepare bootstrap shape used by the runner.
pub fn prepare_store_fixture(root:&std::path::Path) -> anyhow::Result<()> {
    anyhow::ensure!(root.is_absolute(),"Test publisher root must be absolute");
    let publisher=root.join(".nwui-publisher");
    let manifest=publisher.join("manifest.json");
    if std::fs::symlink_metadata(&manifest).is_err_and(|error|error.kind()==std::io::ErrorKind::NotFound) {
        crate::db_upgrade::files::private_dir(&root.join(".nwui-auto"))?;
        crate::db_upgrade::files::private_dir(&publisher)?;
        let journal=serde_json::json!({"version":1,"attemptId":uuid::Uuid::new_v4().to_string(),"branch":"fresh",
            "candidate":{"revision":"0".repeat(40),"pinsSha256":"1".repeat(64),"migrationsSha256":"2".repeat(64)},
            "records":[{"seq":1,"step":"s4-prepare","state":{"kind":"intent"},"at":"2026-10-28T00:00:00Z","data":{"publisherCreated":true}}],
            "moves":[],"handoff":"none","outcome":"upgrading","retry":{"consumedLabels":[],"interruptions":0,"backoffUntil":null}});
        crate::db_upgrade::files::atomic_json(&root.join(".nwui-auto/journal-v1.json"),&journal)?;
    }
    super::store::PublisherStore::open_test_root(root)?;
    Ok(())
}

/// Real bootstrap token for isolated boot integration fixtures.
pub fn open_store_fixture(root:&std::path::Path)->anyhow::Result<super::PublisherStore> {
    prepare_store_fixture(root)?;
    super::PublisherStore::open_test_root(root).map_err(anyhow::Error::from)
}
/// Boot adoption through the real root path with the shared apply admission held.
pub async fn adopt_current(state:&crate::state::AppState,store:&super::PublisherStore)->Result<(),crate::upgrade::StepError> {
    let _store_lock=store.lock(std::time::Duration::from_secs(30)).await.map_err(|error|super::store::step_error("storage_busy",error))?;
    let _apply=state.runtime.apply_lock.lock().await;
    super::adoption::adopt_locked(&state.config,store).await
}

/// Preserve root render observations in tests that assert apply-time validation.
/// Returns the render before A.27 unix isolation; unix-isolation tests must use dispatch.
pub fn render_for_apply(config:&crate::config::RuntimeConfig,data:&crate::control::model::ConfigSnapshotData,certs:&[crate::control::model::CertificateRef])->crate::control::model::RenderOutcome {
    super::apply::render_with_refs(config,data,certs)
}

/// Exercise the same durable DB-blind guard entry as nginx/run.
pub async fn run_guard(config:&crate::config::RuntimeConfig)->Result<crate::control::model::GuardReport,crate::upgrade::StepError> {
    let root=config.data_key_file.parent().ok_or_else(||super::store::step_error("publish_failed","Guard fixture data root is missing"))?;
    let store=open_store_fixture(root).map_err(|error|super::store::step_error("publish_failed",error))?;
    super::boot::guard(config,&store).await
}

/// Isolated fixture store, real dispatcher and real root mutators; never compiled into release.
pub async fn dispatch(state:&crate::state::AppState,request:super::protocol::Request)->super::protocol::OpResult {
    use super::protocol::OpResult;
    let Some(root)=state.config.data_key_file.parent() else{return OpResult::Failed{code:"publish_failed".into(),output:"Test data key has no parent".into()};};
    let store=match open_store_fixture(root) {Ok(store)=>store,Err(error)=>return OpResult::Failed{code:"publish_failed".into(),output:error.to_string()}};
    let dispatcher=super::dispatcher::Dispatcher::new(state.config.as_ref().clone(),store.clone());
    dispatcher.ready.store(true,std::sync::atomic::Ordering::Release);
    let state=state.clone();
    dispatcher.dispatch(request,move |request,operation,budget|async move {
        super::coordinator::execute(&state,&store,request,operation,budget).await
    }).await
}

/// Recreate the root runtime and run production nginx-running restart reconciliation.
pub async fn restart_after_start(state:&crate::state::AppState)->Result<(),crate::upgrade::StepError> {
    let directory=state.config.data_key_file.parent().ok_or_else(||super::store::step_error("publish_failed","Test data key has no parent"))?;
    let store=open_store_fixture(directory).map_err(|error|super::store::step_error("publish_failed",error))?;
    let mut root=state.clone();root.runtime=std::sync::Arc::new(crate::state::RuntimeState::default());
    super::coordinator::reconcile_and_adopt(&root,&store).await
}

/// The fixture bypass is absent from builds without test-support.
pub struct TestServer { server:tokio::task::JoinHandle<anyhow::Result<()>> }
impl Drop for TestServer {fn drop(&mut self){self.server.abort();}}
pub fn serve(state:&crate::state::AppState,directory:&std::path::Path,epoch:u64)->anyhow::Result<TestServer> {
    let mut state=state.clone();
    // The real publisher runs in a separate process and owns its own apply admission.
    state.runtime=std::sync::Arc::new(crate::state::RuntimeState::default());
    let server=super::server::serve_for_tests(directory,epoch,move |request| {
        let state=state.clone();async move {dispatch(&state,request).await}
    })?;
    Ok(TestServer{server})
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt,AsyncWriteExt};
    #[tokio::test]
    async fn fixture_server_uses_real_framing_and_dispatch_but_accepts_the_root_test_peer() {
        let (root,state,_store,_id)=super::super::adoption::tests::fixture();
        let directory=root.path().join("fixture-socket");let _server=serve(&state,&directory,17).unwrap();
        let mut socket=tokio::net::UnixStream::connect(directory.join("control.sock")).await.unwrap();
        let id=uuid::Uuid::new_v4().to_string();
        let request=super::super::protocol::Request{v:1,id:id.clone(),op:super::super::protocol::Op::Hello,deadline_ms:5000,actor:super::super::protocol::Actor{via: None,user_id:None,label:None,ip:None}};
        let bytes=serde_json::to_vec(&request).unwrap();socket.write_all(&(bytes.len() as u32).to_be_bytes()).await.unwrap();socket.write_all(&bytes).await.unwrap();socket.shutdown().await.unwrap();
        let mut size=[0;4];socket.read_exact(&mut size).await.unwrap();let mut bytes=vec![0;u32::from_be_bytes(size) as usize];socket.read_exact(&mut bytes).await.unwrap();
        let response:super::super::protocol::Response=serde_json::from_slice(&bytes).unwrap();
        assert_eq!(response.id,id);assert_eq!(response.epoch,17);
        assert!(matches!(response.result,super::super::protocol::OpResult::Ok{phase,..} if phase=="ready"));
    }
    #[tokio::test]
    async fn real_server_oversized_result_maps_to_client_unconfirmed_without_status_poll() {
        use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
        let (root,mut state,_store,_id)=super::super::adoption::tests::fixture();
        let directory=root.path().join("oversize-socket");let calls=Arc::new(AtomicUsize::new(0));let observed=calls.clone();
        let server=super::super::server::serve_for_tests(&directory,23,move |_| {
            observed.fetch_add(1,Ordering::SeqCst);
            async {super::super::protocol::OpResult::Ok{apply:None,skipped:Vec::new(),uncertain:false,phase:"x".repeat(1_048_576)}}
        }).unwrap();
        let mut runtime=crate::state::RuntimeState::default();
        runtime.publisher=crate::publisher_client::Client::socket_for_tests(directory.join("control.sock"),Default::default());
        state.runtime=Arc::new(runtime);
        let error=crate::publisher_client::apply_saved(&state,&crate::contracts::Actor{via: None,id:None,label:None,ip:None},None).await.unwrap_err();
        assert!(matches!(error.downcast_ref::<crate::publisher_client::ClientError>(),Some(crate::publisher_client::ClientError::Unconfirmed{operation_id}) if uuid::Uuid::parse_str(operation_id).is_ok()));
        assert_eq!(calls.load(Ordering::SeqCst),1);server.abort();let _=server.await;
    }

}

/// Hold the real files lock for HTTP exclusion fixtures without exposing writer capabilities.
pub async fn hold_files_lock_for_tests(config:&crate::config::RuntimeConfig)->anyhow::Result<impl Send> {
    super::nginx_lock::acquire(config).await
}
