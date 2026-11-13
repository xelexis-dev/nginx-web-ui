//! Typed protocol dispatch; the writer callback owns durable phase transitions and audits.
use super::confirmation::Budget;
use super::{journal::{Admission,Operation,Phase},protocol::{Op,OpResult,Request},store::PublisherStore};
use crate::{config::RuntimeConfig,control::model::ApplyResult};
use std::{sync::{Arc,atomic::{AtomicBool,Ordering}},time::Duration};

pub(super) struct Dispatcher {
    pub config:Arc<RuntimeConfig>,pub store:PublisherStore,pub ready:AtomicBool,
    mutator:tokio::sync::Mutex<()>,
}
fn failed(error:impl std::fmt::Display)->OpResult {OpResult::Failed{code:"publish_failed".into(),output:error.to_string()}}
fn admission_failed(error:anyhow::Error)->OpResult {
    let error=super::capacity::stage_error(error);
    if super::capacity::is_full(&error){super::capacity::result()}else{failed(error)}
}
fn persist_failed(store:&PublisherStore,operation:&Operation,error:anyhow::Error)->OpResult {
    let error=super::capacity::stage_error(error);
    if super::capacity::is_full(&error)&&matches!(operation.phase,Phase::Intent|Phase::Done)&&operation.next.is_none()&&operation.restore_target.is_none()&&!operation.commit_unknown {
        if let Err(error)=operation.remove_unwritten(store){return failed(error);}
        return super::capacity::result();
    }
    failed(error)
}
fn missing(error:&anyhow::Error)->bool {error.downcast_ref::<std::io::Error>().is_some_and(|e|e.kind()==std::io::ErrorKind::NotFound)}
pub(super) fn status(store:&PublisherStore,id:&str)->OpResult {
    match Operation::load(store,id) {
        Ok(operation) if !operation.terminal()=>OpResult::Ok{apply:None,skipped:Vec::new(),uncertain:operation.uncertain||operation.commit_unknown,phase:"running".into()},
        Ok(operation)=>operation.result.map(|value|serde_json::from_value(value).map_err(failed)).transpose().unwrap_or_else(|error|Some(error)).unwrap_or(OpResult::Unknown),
        Err(error) if missing(&error)=>OpResult::Unknown,
        Err(error)=>failed(error),
    }
}
fn digest(request:&Request)->anyhow::Result<String> {
    Ok(super::store::digest(&serde_json::to_vec(&(&request.op,&request.actor))?))
}
impl Dispatcher {
    pub fn new(config:RuntimeConfig,store:PublisherStore)->Arc<Self> {Arc::new(Self{config:Arc::new(config),store,ready:AtomicBool::new(false),mutator:tokio::sync::Mutex::new(())})}

    /// The owned task retains every admission guard if its caller disconnects/cancels.
    pub async fn dispatch<F,Fut>(self:&Arc<Self>,request:Request,execute:F)->OpResult
    where F:FnOnce(Request,Operation,Budget)->Fut+Send+'static,Fut:std::future::Future<Output=OpResult>+Send+'static {
        let budget=Budget::with_limit(Duration::from_millis(request.deadline_ms as u64));
        let this=self.clone();
        match tokio::spawn(async move {this.dispatch_owned(request,budget,execute).await}).await {
            Ok(result)=>result,Err(error)=>{self.ready.store(false,Ordering::Release);failed(error)},
        }
    }
    async fn dispatch_owned<F,Fut>(&self,request:Request,budget:Budget,execute:F)->OpResult
    where F:FnOnce(Request,Operation,Budget)->Fut,Fut:std::future::Future<Output=OpResult> {
        match &request.op {
            Op::Hello=>return OpResult::Ok{apply:None,skipped:Vec::new(),uncertain:crate::publisher::confirmation::uncertain(&self.config).await,
                phase:if self.ready.load(Ordering::Acquire)&&super::poison::check(&self.store).is_ok(){"ready"}else{"starting"}.into()},
            Op::OperationStatus{operation_id}=>return status(&self.store,operation_id),
            _=>{},
        }
        if !self.ready.load(Ordering::Acquire) {return OpResult::Unavailable;}
        if super::poison::check(&self.store).is_err() {return OpResult::Unavailable;}
        if matches!(request.op,Op::ValidateLive) {
            let wait=budget.remaining().min(Duration::from_secs(3));
            let _files=match crate::publisher::nginx_lock::acquire_with_wait(&self.config,wait).await {Ok(files)=>files,Err(_)=>return OpResult::Busy};
            let result=super::command::execute(self.store.clone(),self.config.nginx_bin.clone(),vec!["-t".into()],budget.remaining().min(Duration::from_secs(30))).await;
            let apply=match result {Ok(output)=>ApplyResult{ok:true,output,rolled_back_to:None},Err(error)=>ApplyResult{ok:false,output:error.detail.clone().unwrap_or_else(||error.to_string()),rolled_back_to:None}};
            return OpResult::Ok{apply:Some(apply),skipped:Vec::new(),uncertain:crate::publisher::confirmation::uncertain(&self.config).await,phase:"done".into()};
        }
        let _mutator=match budget.admit(self.mutator.lock()).await {Ok(owner)=>owner,Err(_)=>return OpResult::Busy};
        let wait=match budget.admission_remaining(){Ok(wait)=>wait,Err(_)=>return OpResult::Busy};
        let _store_lock=match budget.admit(self.store.lock(wait)).await {Ok(Ok(owner))=>owner,Ok(Err(_))=>return OpResult::Unavailable,Err(_)=>return OpResult::Busy};
        let command=match digest(&request){Ok(digest)=>digest,Err(error)=>return failed(error)};
        match Operation::load(&self.store,&request.id) {
            Ok(operation)=>return if operation.command_digest==command {status(&self.store,&request.id)}else{OpResult::Conflict},
            Err(error) if missing(&error)=>{},Err(error)=>return failed(error),
        }
        match super::journal::operations(&self.store) {
            Ok(operations) if operations.iter().any(|operation|!operation.terminal())=>return OpResult::Unavailable,
            Err(error)=>return failed(error),_=>{},
        }
        let operation=match Operation::admit(&self.store,&request.id,&command) {Ok(Admission::New(operation))=>operation,Ok(Admission::Replay(replay))=>return status(&self.store,&replay.id),Ok(Admission::Conflict)=>return OpResult::Conflict,Err(error)=>return admission_failed(error)};
        let id=request.id.clone();
        let mut result=execute(request,operation,budget).await;
        let mut operation=match Operation::load(&self.store,&id){Ok(operation)=>operation,Err(error)=>return failed(error)};
        let no_write=operation.phase==Phase::Intent && operation.next.is_none();
        if !operation.terminal() && !no_write {
            self.ready.store(false,Ordering::Release);
            let original=match &result {OpResult::Failed{code,..}|OpResult::BadRequest{code}=>code.as_str(),OpResult::Ok{..}=>"ok",OpResult::Busy=>"busy",OpResult::Conflict=>"conflict",OpResult::StaleMaterial=>"stale-material",OpResult::Unknown=>"unknown",OpResult::Unavailable=>"unavailable"};
            eprintln!("[publisher] unfinished operation returned {original}");
            result=OpResult::Failed{code:"unconfirmed".into(),output:"발행기의 작업 완료를 확인할 수 없습니다.".into()};
        }
        operation.result=match serde_json::to_value(&result){Ok(result)=>Some(result),Err(error)=>return failed(error)};
        // No-consumer and rejected requests close atomically with their complete result.
        let persisted=if no_write {operation.advance(&self.store,Phase::Done)} else {operation.save(&self.store)};
        if let Err(error)=persisted {return persist_failed(&self.store,&operation,error);}
        if operation.terminal(){
            let a8_clear=!super::confirmation::uncertain(&self.config).await;
            super::journal::collect_completed(&self.store,a8_clear);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn request(id:&str,op:Op)->Request {Request{v:1,id:id.into(),op,deadline_ms:280000,actor:super::super::protocol::Actor{user_id:None,label:Some("fixture".into()),ip:None}}}
    fn fixture()->(tempfile::TempDir,Arc<Dispatcher>) {
        let (temp,config,store,_,_)=super::super::recovery::tests::durable_fixture();
        (temp,Dispatcher::new(config,store))
    }
    fn no_write_result()->OpResult {OpResult::Ok{apply:Some(ApplyResult{ok:true,output:"no live consumers".into(),rolled_back_to:None}),skipped:Vec::new(),uncertain:false,phase:"done".into()}}
    #[test]
    fn a29_full_intent_and_refusal_record_errors_allow_the_same_id_to_retry() {
        for code in [libc::ENOSPC,libc::EDQUOT] {
            assert!(matches!(admission_failed(std::io::Error::from_raw_os_error(code).into()),OpResult::Failed{code,..}if code=="publisher_storage_full"));
            let (_root,dispatcher)=fixture();let id=uuid::Uuid::new_v4().to_string();
            let Admission::New(operation)=Operation::admit(&dispatcher.store,&id,&"a".repeat(64)).unwrap() else{panic!()};
            let result=persist_failed(&dispatcher.store,&operation,std::io::Error::from_raw_os_error(code).into());
            assert!(matches!(result,OpResult::Failed{code,..}if code=="publisher_storage_full"));assert!(super::super::journal::operations(&dispatcher.store).unwrap().is_empty());
            assert!(matches!(Operation::admit(&dispatcher.store,&id,&"a".repeat(64)).unwrap(),Admission::New(_)));
        }
    }
    #[test]
    fn a29_full_filesystem_cannot_discard_a_record_after_a_live_write() {
        let (_root,dispatcher)=fixture();let id=uuid::Uuid::new_v4().to_string();
        let Admission::New(mut operation)=Operation::admit(&dispatcher.store,&id,&"a".repeat(64)).unwrap() else{panic!()};
        operation.previous=Some(uuid::Uuid::new_v4().to_string());operation.next=Some(uuid::Uuid::new_v4().to_string());operation.phase=Phase::LiveWriting;operation.save(&dispatcher.store).unwrap();
        let before=serde_json::to_value(Operation::load(&dispatcher.store,&id).unwrap()).unwrap();
        assert!(operation.remove_unwritten(&dispatcher.store).is_err());
        assert!(matches!(persist_failed(&dispatcher.store,&operation,std::io::Error::from_raw_os_error(libc::ENOSPC).into()),OpResult::Failed{code,..}if code=="publish_failed"));
        assert_eq!(serde_json::to_value(Operation::load(&dispatcher.store,&id).unwrap()).unwrap(),before);
    }
    #[tokio::test]
    async fn g11_poisoned_slot_is_unavailable_without_admitting_or_changing_files() {
        for op in [Op::ApplySaved{label:None,expected_saved_digest:None},Op::ValidateLive] {
            let (_temp,dispatcher)=fixture();dispatcher.ready.store(true,Ordering::Release);
            dispatcher.store.directory("ops").unwrap();
            let pid=std::process::id();let ticks=super::super::poison::start_ticks(pid).unwrap().unwrap();
            super::super::poison::mark(&dispatcher.store,pid,ticks).unwrap();
            let before=crate::db_upgrade::files::snapshot(&dispatcher.store.root).unwrap();let id=uuid::Uuid::new_v4().to_string();
            let result=dispatcher.dispatch(request(&id,op),|_,_,_|async{panic!("poisoned slot must not dispatch")}).await;
            assert!(matches!(result,OpResult::Unavailable),"{}",serde_json::to_string(&result).unwrap());
            assert_eq!(crate::db_upgrade::files::snapshot(&dispatcher.store.root).unwrap(),before);
            assert!(!crate::publisher::confirmation::uncertain(&dispatcher.config).await);
        }
    }
    #[tokio::test]
    async fn g11_unsafe_store_lock_is_unavailable_without_admitting_or_changing_files() {
        use std::os::unix::fs::PermissionsExt;
        let (_temp,dispatcher)=fixture();dispatcher.ready.store(true,Ordering::Release);
        dispatcher.store.directory("ops").unwrap();
        std::fs::set_permissions(dispatcher.store.root.join(".mutator.lock"),std::fs::Permissions::from_mode(0o666)).unwrap();
        let before=crate::db_upgrade::files::snapshot(&dispatcher.store.root).unwrap();let id=uuid::Uuid::new_v4().to_string();
        let result=dispatcher.dispatch(request(&id,Op::ApplySaved{label:None,expected_saved_digest:None}),|_,_,_|async{panic!("unsafe store must not dispatch")}).await;
        assert!(matches!(result,OpResult::Unavailable),"{}",serde_json::to_string(&result).unwrap());
        assert_eq!(crate::db_upgrade::files::snapshot(&dispatcher.store.root).unwrap(),before);
        assert!(!crate::publisher::confirmation::uncertain(&dispatcher.config).await);
    }
    #[tokio::test]
    async fn every_unresolved_mutation_result_is_unconfirmed_and_fences_following_operations() {
        let mut results=vec![OpResult::Busy,OpResult::Conflict,OpResult::StaleMaterial,OpResult::Unknown,OpResult::Unavailable,OpResult::BadRequest{code:"fixture_bad".into()},OpResult::Failed{code:"publish_failed".into(),output:"private diagnostics".into()}];
        for ok in [false,true] {results.push(OpResult::Ok{apply:Some(ApplyResult{ok,output:"private diagnostics".into(),rolled_back_to:None}),skipped:Vec::new(),uncertain:true,phase:"incomplete".into()});}
        results.push(OpResult::Ok{apply:None,skipped:Vec::new(),uncertain:true,phase:"incomplete".into()});
        for original in results {
            let (_root,dispatcher)=fixture();dispatcher.ready.store(true,Ordering::Release);let store=dispatcher.store.clone();let config=dispatcher.config.clone();let id=uuid::Uuid::new_v4().to_string();
            let result=dispatcher.dispatch(request(&id,Op::ApplySaved{label:None,expected_saved_digest:None}),move |_,mut operation,_|async move {
                let current=store.manifest().unwrap().current.unwrap();operation.previous=Some(current.clone());operation.next=Some(current);
                operation.advance(&store,Phase::PayloadStaged).unwrap();operation.advance(&store,Phase::LiveWriting).unwrap();super::super::confirmation::begin_uncertain(&config).await.unwrap();original
            }).await;
            assert!(matches!(result,OpResult::Failed{code,output}if code=="unconfirmed"&&output=="발행기의 작업 완료를 확인할 수 없습니다."));assert!(!dispatcher.ready.load(Ordering::Acquire));
            let operation=Operation::load(&dispatcher.store,&id).unwrap();assert!(!operation.terminal());assert_eq!(operation.result.unwrap()["code"],"unconfirmed");assert!(super::super::confirmation::uncertain(&dispatcher.config).await);
            assert!(matches!(dispatcher.dispatch(request(&uuid::Uuid::new_v4().to_string(),Op::ApplySaved{label:None,expected_saved_digest:None}),|_,_,_|async{panic!("unresolved publisher cannot dispatch")}).await,OpResult::Unavailable));
        }
    }
    #[tokio::test]
    async fn hello_reports_starting_or_ready_and_the_exact_a8_marker_state() {
        let (_temp,dispatcher)=fixture();let id=uuid::Uuid::new_v4().to_string();
        let result=dispatcher.dispatch(request(&id,Op::Hello),|_,_,_|async{panic!("Hello must not mutate")}).await;
        assert!(matches!(result,OpResult::Ok{phase,uncertain:false,..} if phase=="starting"));
        dispatcher.ready.store(true,Ordering::Release);
        crate::publisher::confirmation::begin_uncertain(&dispatcher.config).await.unwrap();
        let result=dispatcher.dispatch(request(&id,Op::Hello),|_,_,_|async{panic!("Hello must not mutate")}).await;
        assert!(matches!(result,OpResult::Ok{phase,uncertain:true,..} if phase=="ready"));
    }
    #[test]
    fn operation_status_replays_the_final_result_or_reports_running_and_unknown() {
        let (_temp,dispatcher)=fixture();let id=uuid::Uuid::new_v4().to_string();
        assert!(matches!(status(&dispatcher.store,&id),OpResult::Unknown));
        let Admission::New(mut operation)=Operation::admit(&dispatcher.store,&id,&"a".repeat(64)).unwrap() else{panic!()};
        assert!(matches!(status(&dispatcher.store,&id),OpResult::Ok{phase,..} if phase=="running"));
        let final_result=OpResult::Failed{code:"fixture".into(),output:"unchanged\n문자열".into()};
        operation.result=Some(serde_json::to_value(&final_result).unwrap());operation.advance(&dispatcher.store,Phase::Done).unwrap();
        assert_eq!(serde_json::to_value(status(&dispatcher.store,&id)).unwrap(),serde_json::to_value(final_result).unwrap());
    }
    #[tokio::test]
    async fn same_command_replays_once_across_deadline_changes_and_a_different_command_conflicts() {
        use std::sync::atomic::AtomicUsize;
        let (_temp,dispatcher)=fixture();dispatcher.ready.store(true,Ordering::Release);
        let calls=Arc::new(AtomicUsize::new(0));let id=uuid::Uuid::new_v4().to_string();
        for deadline in [280000,279000] {
            let mut req=request(&id,Op::ApplySaved{label:None,expected_saved_digest:None});req.deadline_ms=deadline;
            let calls=calls.clone();
            let result=dispatcher.dispatch(req,move |_,_,_|async move{calls.fetch_add(1,Ordering::SeqCst);no_write_result()}).await;
            assert_eq!(serde_json::to_value(result).unwrap(),serde_json::to_value(no_write_result()).unwrap());
        }
        assert_eq!(calls.load(Ordering::SeqCst),1);
        assert!(!crate::publisher::confirmation::uncertain(&dispatcher.config).await,"no-consumer success changes no live byte");
        let result=dispatcher.dispatch(request(&id,Op::ApplySaved{label:Some("different".into()),expected_saved_digest:None}),|_,_,_|async{panic!("conflicting command must not run")}).await;
        assert!(matches!(result,OpResult::Conflict));
    }
    #[tokio::test]
    async fn admission_uses_the_request_deadline_and_does_not_journal_a_busy_request() {
        let (_temp,dispatcher)=fixture();dispatcher.ready.store(true,Ordering::Release);
        let _held=dispatcher.mutator.lock().await;let id=uuid::Uuid::new_v4().to_string();
        let mut req=request(&id,Op::ApplySaved{label:None,expected_saved_digest:None});req.deadline_ms=1;
        let result=dispatcher.dispatch(req,|_,_,_|async{panic!("busy request must not run")}).await;
        assert!(matches!(result,OpResult::Busy));assert!(Operation::load(&dispatcher.store,&id).is_err());
        assert!(!crate::publisher::confirmation::uncertain(&dispatcher.config).await);
    }
    #[tokio::test]
    async fn validate_live_returns_apply_some_and_never_calls_the_mutating_backend() {
        let (_temp,dispatcher)=fixture();dispatcher.ready.store(true,Ordering::Release);
        std::fs::write(&dispatcher.config.nginx_bin,"#!/bin/sh\n[ \"$*\" = -t ] || exit 9\nprintf 'validated live\\n'\n").unwrap();
        let result=dispatcher.dispatch(request(&uuid::Uuid::new_v4().to_string(),Op::ValidateLive),|_,_,_|async{panic!("validate is read-only")}).await;
        let value=serde_json::to_value(result).unwrap();assert_eq!(value["status"],"ok");assert_eq!(value["apply"]["ok"],true);
        assert_eq!(value["apply"]["output"],"validated live\n");assert!(!crate::publisher::confirmation::uncertain(&dispatcher.config).await);
    }
    #[tokio::test]
    async fn completed_mutations_collect_results_and_payloads_but_keep_all_referenced_generations(){
        use super::super::store::Origin;use std::collections::BTreeMap;
        let (_root,dispatcher)=fixture();dispatcher.ready.store(true,Ordering::Release);
        super::super::confirmation::begin_uncertain(&dispatcher.config).await.unwrap();
        let current=uuid::Uuid::new_v4().to_string();let uncertain=uuid::Uuid::new_v4().to_string();
        for id in [&current,&uncertain]{dispatcher.store.stage(id,Origin::Publisher,None,BTreeMap::new(),BTreeMap::new()).unwrap();}dispatcher.store.select(&current).unwrap();
        let previous=dispatcher.store.manifest().unwrap().previous.unwrap();
        let Admission::New(mut exempt)=Operation::admit(&dispatcher.store,&uuid::Uuid::new_v4().to_string(),&"a".repeat(64)).unwrap()else{panic!()};
        exempt.phase=Phase::Done;exempt.next=Some(uncertain.clone());exempt.uncertain=true;exempt.created_at=chrono::Utc::now()-chrono::Duration::hours(48);exempt.save(&dispatcher.store).unwrap();
        for _ in 0..103{
            let store=dispatcher.store.clone();let id=uuid::Uuid::new_v4().to_string();
            let result=dispatcher.dispatch(request(&id,Op::ApplySaved{label:None,expected_saved_digest:None}),move |_,mut operation,_|async move{
                let next=uuid::Uuid::new_v4().to_string();store.stage(&next,Origin::Publisher,None,BTreeMap::new(),BTreeMap::new()).unwrap();operation.next=Some(next);operation.phase=Phase::DbCommitted;operation.advance(&store,Phase::Done).unwrap();no_write_result()
            }).await;assert!(matches!(result,OpResult::Ok{..}));
        }
        assert_eq!(super::super::journal::operations(&dispatcher.store).unwrap().len(),101);
        // Two original fixture payloads + current + uncertain + 100 retained results.
        assert!(std::fs::read_dir(dispatcher.store.directory("payloads").unwrap()).unwrap().count()<=104);
        for id in [&current,&previous,&uncertain]{assert!(dispatcher.store.payload(id).is_ok());}assert!(Operation::load(&dispatcher.store,&exempt.id).is_ok());
    }
    #[tokio::test]
    async fn gc_failure_cannot_replace_the_completed_mutation_result(){
        let (_root,dispatcher)=fixture();dispatcher.ready.store(true,Ordering::Release);
        std::fs::write(dispatcher.store.root.join("manifest.json"),b"invalid GC manifest").unwrap();
        let id=uuid::Uuid::new_v4().to_string();let result=dispatcher.dispatch(request(&id,Op::ApplySaved{label:None,expected_saved_digest:None}),|_,_,_|async{no_write_result()}).await;
        assert_eq!(serde_json::to_value(result).unwrap(),serde_json::to_value(no_write_result()).unwrap());assert!(Operation::load(&dispatcher.store,&id).unwrap().terminal());
    }

    #[tokio::test]
    async fn a28_terminal_uncertainty_expires_only_when_a8_is_absent(){
        use super::super::store::Origin;use std::collections::BTreeMap;
        for (uncertain,unknown,derived) in [(true,false,false),(false,true,false),(true,false,true)] {
            let (_root,dispatcher)=fixture();dispatcher.ready.store(true,Ordering::Release);
            let payload=uuid::Uuid::new_v4().to_string();
            if derived {let source=dispatcher.store.manifest().unwrap().current.unwrap();dispatcher.store.stage_derived(&payload,&source,"fixture derivation",BTreeMap::new()).unwrap();}
            else {dispatcher.store.stage(&payload,Origin::Publisher,None,BTreeMap::new(),BTreeMap::new()).unwrap();}
            let Admission::New(mut old)=Operation::admit(&dispatcher.store,&uuid::Uuid::new_v4().to_string(),&"a".repeat(64)).unwrap()else{panic!()};
            old.phase=Phase::Done;old.next=Some(payload.clone());old.uncertain=uncertain;old.commit_unknown=unknown;old.created_at=chrono::Utc::now()-chrono::Duration::hours(48);old.save(&dispatcher.store).unwrap();
            super::super::confirmation::begin_uncertain(&dispatcher.config).await.unwrap();
            let marker=dispatcher.config.nginx_conf_dir.join(".nwui-apply-uncertain");
            for directory in [false,true] {
                if directory {std::fs::remove_file(&marker).unwrap();std::fs::create_dir(&marker).unwrap();}
                let result=dispatcher.dispatch(request(&uuid::Uuid::new_v4().to_string(),Op::ApplySaved{label:None,expected_saved_digest:None}),|_,_,_|async{no_write_result()}).await;
                assert!(matches!(result,OpResult::Ok{..}));assert!(Operation::load(&dispatcher.store,&old.id).is_ok());assert!(dispatcher.store.payload(&payload).is_ok());
            }
            std::fs::remove_dir(&marker).unwrap();
            let result=dispatcher.dispatch(request(&uuid::Uuid::new_v4().to_string(),Op::ApplySaved{label:None,expected_saved_digest:None}),|_,_,_|async{no_write_result()}).await;
            assert!(matches!(result,OpResult::Ok{..}));
            assert!(Operation::load(&dispatcher.store,&old.id).is_err(),"A8-clear supersedes old terminal uncertainty");
            assert!(!dispatcher.store.root.join("payloads").join(payload).exists());
        }
    }
    #[tokio::test]
    async fn a28_dispatch_never_sweeps_an_unreferenced_payload(){
        let (_root,dispatcher)=fixture();dispatcher.ready.store(true,Ordering::Release);let store=dispatcher.store.clone();let orphan=uuid::Uuid::new_v4().to_string();let id=orphan.clone();
        let result=dispatcher.dispatch(request(&uuid::Uuid::new_v4().to_string(),Op::ApplySaved{label:None,expected_saved_digest:None}),move |_,_,_|async move{
            store.stage(&id,super::super::store::Origin::Publisher,None,Default::default(),Default::default()).unwrap();no_write_result()
        }).await;
        assert!(matches!(result,OpResult::Ok{..}));assert!(dispatcher.store.payload(&orphan).is_ok());
    }

}
