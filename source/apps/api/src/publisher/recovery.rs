//! Recovery chooses a complete durable generation, never the observed mixed live set.
use super::journal::{Operation, Phase};
use anyhow::{Context, Result};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Action {
    DiscardStaged,
    StartRestore(String),
    ResumeRestore(String),
    KeepUncertain(String),
    Finish { payload:String, uncertain:bool },
    Superseded,
}

pub(super) fn plan(operation:&Operation) -> Result<Action> {
    use Phase::*;
    if matches!(operation.phase,Superseded{..}) {return Ok(Action::Superseded);}
    // Restore intent has precedence over the forward C/commit-unknown track (R5).
    if let Some(target)=&operation.restore_target {
        return Ok(if operation.phase==Done {
            Action::Finish { payload:target.clone(), uncertain:operation.commit_unknown || operation.uncertain }
        } else { Action::ResumeRestore(target.clone()) });
    }
    match operation.phase {
        Intent|PayloadStaged=>Ok(Action::DiscardStaged),
        Done if operation.next.is_none() || operation.result.as_ref().is_some_and(|result|result["code"]=="interrupted_before_publish")=>Ok(Action::DiscardStaged),
        LiveWriting|Validated|P=>Ok(Action::StartRestore(operation.previous.clone().context("restore-incomplete: previous payload missing")?)),
        C|CommitUnknown=>Ok(Action::KeepUncertain(operation.next.clone().context("restore-incomplete: next payload missing")?)),
        DbCommitted|Done=>Ok(Action::Finish { payload:operation.next.clone().context("restore-incomplete: committed payload missing")?, uncertain:operation.commit_unknown || operation.uncertain }),
        RestoreIntent|RestoreWriting|RestoreValidated|R1|R2|Restored=>anyhow::bail!("restore-incomplete: restore target missing"),
        Superseded{..}=>Ok(Action::Superseded),
    }
}

/// Caller holds the cross-process writer lock; nginx must not be running.
/// Restores stop before R1: fresh acknowledgements run only after nginx starts.
pub(super) async fn before_start(config:&crate::config::RuntimeConfig,store:&super::store::PublisherStore)->Result<()> {
    before_start_with_parent(config,store,None).await
}
/// Only the guard can exempt its verified, waiting nginx/run parent.
pub(super) async fn before_start_for_guard(config:&crate::config::RuntimeConfig,store:&super::store::PublisherStore)->Result<()> {
    before_start_with_parent(config,store,Some(unsafe{libc::getppid()}as u32)).await
}
pub(super) async fn before_start_with_parent(config:&crate::config::RuntimeConfig,store:&super::store::PublisherStore,parent:Option<u32>)->Result<()> {
    super::confirmation::ensure_no_pending_start(config,parent).await?;
    reconcile(config,store,false,parent).await
}
/// Caller holds store+file locks. Running recovery uses the same plans and fixed-target writer.
pub(super) async fn after_start(config:&crate::config::RuntimeConfig,store:&super::store::PublisherStore)->Result<()> {
    reconcile(config,store,true,None).await
}
async fn reconcile(config:&crate::config::RuntimeConfig,store:&super::store::PublisherStore,running:bool,guard_parent:Option<u32>)->Result<()> {
    use super::journal::{self,Phase};
    super::poison::check(store)?;
    let records=journal::operations(store)?;
    // A done-before-current crash follows causal previous pointers, never wall-clock order.
    let mut current=store.manifest()?.current;
    let mut chain=Vec::new();let mut seen=std::collections::BTreeSet::new();
    loop {
        let candidates:Vec<_>=records.iter().filter_map(|operation| {
            if operation.phase!=Phase::Done || operation.previous!=current {return None;}
            match plan(operation) {Ok(Action::Finish{payload,..}) if Some(&payload)!=current.as_ref()=>Some(payload),_=>None}
        }).collect();
        anyhow::ensure!(candidates.len()<=1,"restore_incomplete: ambiguous completed publication");
        let Some(next)=candidates.into_iter().next() else {break;};
        anyhow::ensure!(seen.insert(next.clone()),"restore_incomplete: cyclic completed publications");
        current=Some(next.clone());chain.push(next);
    }
    for next in chain {store.select(&next).with_context(||format!("restore_incomplete: {next}"))?;}
    let mut operations:Vec<_>=records.into_iter().filter(|operation|!operation.terminal()).collect();
    let writes=operations.iter().filter(|operation|!matches!(operation.phase,Phase::Intent|Phase::PayloadStaged)).count();
    anyhow::ensure!(writes<=1,"restore-incomplete: multiple unfinished live operations");
    if writes>0 {
        if running {anyhow::ensure!(crate::publisher::confirmation::nginx_running(config).await?==super::confirmation::NginxState::Running,"nginx master is not running");}
        else {super::confirmation::require_stopped(super::confirmation::nginx_state(config,guard_parent).await?)?;}
    }
    for operation in &mut operations {
        match plan(operation)? {
            Action::Superseded=>{},
            Action::DiscardStaged=>{
                operation.result=Some(serde_json::json!({"status":"failed","code":"interrupted_before_publish","output":"설정을 공개하기 전에 발행기가 중지됐습니다."}));
                operation.advance(store,Phase::Done)?;
            },
            Action::StartRestore(target)|Action::ResumeRestore(target)=>{
                let attempt=async {
                    if running {
                        if operation.restore_target.is_none() {operation.begin_restore(store,&target,"publisher restart before confirmed completion")?;}
                        finish_restore(config,store,operation,std::time::Instant::now()+crate::publisher::confirmation::RESTORE_RESERVE).await?;
                    } else {restore_validated(config,store,operation,&target,std::time::Instant::now()+std::time::Duration::from_secs(30)).await?;}
                    Ok::<_,anyhow::Error>(())
                }.await;
                attempt.with_context(||format!("restore-incomplete: {target}"))?;
            },
            Action::KeepUncertain(target)=>{
                let attempt=async {
                    operation.uncertain=true;
                    if operation.phase==Phase::C {operation.advance(store,Phase::CommitUnknown)?;} else {operation.save(store)?;}
                    crate::publisher::confirmation::begin_uncertain(config).await?;
                    store.write_recovery(config,operation).await?;
                    super::command::validate_includes(store.clone(),config.nginx_bin.clone(),None,std::time::Duration::from_secs(30)).await.map_err(|error|anyhow::anyhow!("{}: {}",error.code,error.detail.as_deref().unwrap_or(error.message)))?;
                    store.select(&target)?;
                    operation.result=Some(serde_json::to_value(super::protocol::OpResult::Ok {
                        apply:Some(crate::control::model::ApplyResult{ok:false,output:"설정 DB 커밋 결과를 확인할 수 없어 새 설정을 유지했습니다. 설정을 확인하세요.".into(),rolled_back_to:None}),
                        skipped:Vec::new(),uncertain:true,phase:"commit-unknown".into(),
                    })?);
                    if running {operation.advance(store,Phase::Done)?;}else{operation.save(store)?;}
                    Ok::<_,anyhow::Error>(())
                }.await;
                attempt.with_context(||format!("restore-incomplete: {target}"))?;
            },
            Action::Finish{payload,..}=>{
                let attempt=async {
                    store.write_recovery(config,operation).await?;
                    super::command::validate_includes(store.clone(),config.nginx_bin.clone(),None,std::time::Duration::from_secs(30)).await.map_err(|error|anyhow::anyhow!("{}: {}",error.code,error.detail.as_deref().unwrap_or(error.message)))?;
                    store.select(&payload)?;
                    // Completion never borrows a boot adoption to clear a non-boot marker.
                    operation.uncertain=crate::publisher::confirmation::uncertain(config).await||operation.commit_unknown;
                    let mut result=operation.result.clone().and_then(|value|serde_json::from_value::<super::protocol::OpResult>(value).ok()).unwrap_or(super::protocol::OpResult::Failed{code:"publish_failed".into(),output:"중단된 설정 작업의 파일 발행을 마무리했습니다. 적용 상태를 확인하세요.".into()});
                    if let super::protocol::OpResult::Ok{uncertain,..}=&mut result {*uncertain=operation.uncertain;}
                    operation.result=Some(serde_json::to_value(result)?);
                    operation.advance(store,Phase::Done)?;
                    Ok::<_,anyhow::Error>(())
                }.await;
                attempt.with_context(||format!("restore-incomplete: {payload}"))?;
            },
        }
    }
    Ok(())
}

/// Caller holds store+file locks; the persisted target is fixed before any live write.
async fn restore_validated(config:&crate::config::RuntimeConfig,store:&super::store::PublisherStore,operation:&mut Operation,target:&str,validate_until:std::time::Instant)->Result<()> {
    if operation.restore_target.is_none() {operation.begin_restore(store,target,"publisher restart before confirmed completion")?;}
    operation.restart_restore_write(store)?;
    let written=store.write_recovery(config,operation).await?;
    anyhow::ensure!(written==target,"Recovery target changed");
    super::command::validate_includes(store.clone(),config.nginx_bin.clone(),None,validate_until.checked_duration_since(std::time::Instant::now()).context("Restore validation deadline exceeded")?).await
        .map_err(|error|anyhow::anyhow!("{}: {}",error.code,error.detail.as_deref().unwrap_or(error.message)))?;
    operation.advance(store,Phase::RestoreValidated)?;
    store.select(target)?;
    Ok(())
}

/// Caller holds store+file locks. Every attempt rewrites the complete target and obtains fresh R1/R2.
pub(super) async fn finish_restore(config:&crate::config::RuntimeConfig,store:&super::store::PublisherStore,operation:&mut Operation,deadline:std::time::Instant)->Result<()> {
    let validate_until=deadline.min(std::time::Instant::now()+std::time::Duration::from_secs(30));
    let target=operation.restore_target.clone().context("restore_incomplete: restore target missing")?;
    restore_validated(config,store,operation,&target,validate_until).await.with_context(||format!("restore_incomplete: {target}"))?;
    let first=uuid::Uuid::new_v4().to_string();
    crate::publisher::confirmation::publish(config,&first).await?;
    super::command::execute(store.clone(),config.nginx_bin.clone(),vec!["-t".into()],validate_until.checked_duration_since(std::time::Instant::now()).context("Restore validation deadline exceeded")?).await?;
    let witness=crate::publisher::confirmation::Witness::from_config(config)?;
    let child_store=store.clone();let bin=config.nginx_bin.clone();
    witness.confirm_with(config,&first,deadline.min(std::time::Instant::now()+std::time::Duration::from_secs(15)),move |timeout| {
        let store=child_store.clone();let bin=bin.clone();
        async move {super::command::execute(store,bin,vec!["-s".into(),"reload".into()],timeout).await.map(|_|()).map_err(anyhow::Error::from)}
    },|phase|operation.advance(store,if phase==0 {Phase::R1}else{Phase::R2})).await?;
    operation.advance(store,Phase::Restored)?;
    if !operation.commit_unknown && !operation.previous_uncertain {
        crate::publisher::confirmation::clear_uncertain(config).await?;
        operation.uncertain=false;
    }
    // A restored request must never replay a prior success or become a result-less terminal op.
    use super::protocol::OpResult;
    let result=match operation.result.clone().and_then(|value|serde_json::from_value::<OpResult>(value).ok()) {
        Some(result@OpResult::Failed{..})=>result,
        Some(OpResult::Ok{apply:Some(apply),skipped,..}) if !apply.ok=>OpResult::Ok{apply:Some(apply),skipped,uncertain:operation.uncertain,phase:"done".into()},
        _=>OpResult::Failed{code:"publish_failed".into(),output:"중단된 설정 작업의 이전 파일을 복원했습니다. 적용 상태를 확인하세요.".into()},
    };
    operation.result=Some(serde_json::to_value(result)?);
    operation.advance(store,Phase::Done)?;
    Ok(())
}

/// Caller holds store+file locks. The frozen no-running-nginx reconciliation is shared with guard.
pub(super) async fn supersede_for_boot(config:&crate::config::RuntimeConfig,store:&super::store::PublisherStore,id:&str)->Result<()> {
    super::store::uuid(id)?;
    before_start(config,store).await?;
    for mut operation in super::journal::operations(store)?.into_iter().filter(|operation|!operation.terminal()) {
        anyhow::ensure!(matches!(operation.phase,Phase::RestoreValidated|Phase::R1|Phase::R2|Phase::CommitUnknown),"restore_incomplete: operation is not ready to supersede");
        operation.result=Some(serde_json::to_value(super::protocol::OpResult::Failed{code:"superseded".into(),output:format!("부팅 설정 세대 {id}로 대체됐습니다.")})?);
        operation.advance(store,Phase::Superseded{by:uuid::Uuid::parse_str(id)?})?;
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::publisher::{journal::Admission, store::PublisherStore};

    #[test]
    fn n1_recovery_never_adopts_a_partial_restore_after_an_unknown_commit() {
        let temp=tempfile::tempdir().unwrap();
        let store=PublisherStore::at(temp.path().into());
        let id=uuid::Uuid::new_v4().to_string();
        let Admission::New(mut operation)=Operation::admit(&store,&id,&"a".repeat(64)).unwrap() else { panic!() };
        let previous=uuid::Uuid::new_v4().to_string();
        let next=uuid::Uuid::new_v4().to_string();
        operation.previous=Some(previous.clone());operation.next=Some(next.clone());
        for phase in [Phase::LiveWriting,Phase::Validated,Phase::P] {
            operation.phase=phase;
            assert_eq!(plan(&operation).unwrap(),Action::StartRestore(previous.clone()));
        }
        operation.phase=Phase::CommitUnknown;operation.commit_unknown=true;
        assert_eq!(plan(&operation).unwrap(),Action::KeepUncertain(next));
        operation.begin_restore(&store,&previous,"commit-unknown").unwrap();
        for phase in [Phase::RestoreIntent,Phase::RestoreWriting,Phase::RestoreValidated,Phase::R1,Phase::R2,Phase::Restored] {
            operation.phase=phase;operation.save(&store).unwrap();
            let reopened=Operation::load(&store,&id).unwrap();
            assert_eq!(plan(&reopened).unwrap(),Action::ResumeRestore(previous.clone()));
            assert!(reopened.commit_unknown);
        }
        operation.phase=Phase::Done;
        assert_eq!(plan(&operation).unwrap(),Action::Finish { payload:previous,uncertain:true });
    }

    #[test]
    fn n1_recovery_refuses_missing_payload_identity_instead_of_guessing_live_files() {
        let temp=tempfile::tempdir().unwrap();
        let store=PublisherStore::at(temp.path().into());
        let Admission::New(mut operation)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&"a".repeat(64)).unwrap() else { panic!() };
        for phase in [Phase::P,Phase::C,Phase::RestoreWriting,Phase::DbCommitted] {
            operation.phase=phase;
            assert!(plan(&operation).is_err());
        }
    }
    pub(in crate::publisher) fn durable_fixture()->(tempfile::TempDir,crate::config::RuntimeConfig,PublisherStore,String,String) {
        use std::{collections::BTreeMap,os::unix::fs::PermissionsExt};
        let temp=tempfile::tempdir().unwrap();
        crate::publisher::test_support::prepare_store_fixture(temp.path()).unwrap();
        let store=PublisherStore::open_test_root(temp.path()).unwrap();
        let mut config=crate::config::RuntimeConfig::read(|key|match key {
            "DATABASE_URL"=>Some("postgres://fixture@127.0.0.1/fixture".into()),"AUTH_SECRET"=>Some("recovery-fixture".into()),_=>None,
        }).unwrap();
        config.nginx_conf_dir=temp.path().join("conf");config.nginx_stream_dir=temp.path().join("stream");config.certs_dir=temp.path().join("certs");
        config.nginx_main_conf=temp.path().join("nginx.conf");config.nginx_status_url="http://127.0.0.1:18080/nginx_status".into();
        std::fs::write(&config.nginx_main_conf,format!("pid {}; events {{}} http {{ server {{ listen 127.0.0.1:18080; location = /nginx_status {{ stub_status; allow 127.0.0.3; deny all; include {}; }} }} }}",temp.path().join("nginx.pid").display(),config.nginx_conf_dir.join(".nwui-generation").display())).unwrap();
        std::fs::set_permissions(&config.nginx_main_conf,std::fs::Permissions::from_mode(0o600)).unwrap();
        config.nginx_bin=temp.path().join("nginx");
        std::fs::write(&config.nginx_bin,format!("#!/bin/sh\n[ \"$*\" = -T ] || [ \"$*\" = -t ] || exit 9\nprintf '# configuration file %s:\\n' '{}'\nprintf 'validated without reload\\n' >&2\n",config.nginx_main_conf.display())).unwrap();
        std::fs::set_permissions(&config.nginx_bin,std::fs::Permissions::from_mode(0o700)).unwrap();
        let old=uuid::Uuid::new_v4().to_string();let new=uuid::Uuid::new_v4().to_string();
        let cert="55555555-5555-4555-8555-555555555555";
        for (id,label) in [(&old,"old"),(&new,"new")] {
            let files=BTreeMap::from([
                ("conf/server-fixture.conf".into(),format!("{label} HTTP").into_bytes()),
                ("stream/stream-fixture.conf".into(),format!("{label} stream").into_bytes()),
                (format!("certs/{cert}/fullchain.pem"),format!("{label} cert").into_bytes()),
                (format!("certs/{cert}/privkey.pem"),format!("{label} key").into_bytes()),
            ]);
            store.stage(id,super::super::store::Origin::Publisher,None,BTreeMap::new(),files).unwrap();
        }
        store.select(&old).unwrap();
        (temp,config,store,old,new)
    }

    pub(in crate::publisher) fn unfinished(store:&PublisherStore,old:&str,new:&str,phase:Phase)->Operation {
        let Admission::New(mut operation)=Operation::admit(store,&uuid::Uuid::new_v4().to_string(),&"a".repeat(64)).unwrap() else {panic!()};
        operation.previous=Some(old.into());operation.next=Some(new.into());operation.phase=phase;operation.save(store).unwrap();operation
    }

    #[tokio::test]
    async fn discarded_staged_payload_never_becomes_current_on_a_second_reconciliation() {
        for phase in [Phase::Intent,Phase::PayloadStaged] {
            let (_temp,config,store,old,new)=durable_fixture();
            crate::publisher::files::restore_payload_files(&config,&store.payload(&old).unwrap().files).await.unwrap();
            let operation=unfinished(&store,&old,&new,phase);
            let before=crate::publisher::files::capture_payload_files(&config).await.unwrap();
            for _ in 0..2 {
                before_start(&config,&store).await.unwrap();
                assert_eq!(store.manifest().unwrap().current.as_deref(),Some(old.as_str()),"discarded next must never be selected");
                assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),before);
                assert!(!crate::publisher::confirmation::uncertain(&config).await);
            }
            let discarded=Operation::load(&store,&operation.id).unwrap();assert_eq!(discarded.phase,Phase::Done);assert!(discarded.next.is_none());
        }
    }
    #[tokio::test]
    async fn an_older_discarded_done_record_with_next_is_not_a_publication() {
        let (_temp,config,store,old,new)=durable_fixture();
        crate::publisher::files::restore_payload_files(&config,&store.payload(&old).unwrap().files).await.unwrap();
        let mut operation=unfinished(&store,&old,&new,Phase::PayloadStaged);
        operation.phase=Phase::Done;operation.result=Some(serde_json::json!({"status":"failed","code":"interrupted_before_publish","output":"discarded"}));operation.save(&store).unwrap();
        for _ in 0..2 {before_start(&config,&store).await.unwrap();assert_eq!(store.manifest().unwrap().current.as_deref(),Some(old.as_str()));}
        assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),store.payload(&old).unwrap().files);
    }
    #[tokio::test]
    async fn running_recovery_refuses_without_a_master_and_changes_no_live_bytes() {
        let (_temp,config,store,old,new)=durable_fixture();
        let operation=unfinished(&store,&old,&new,Phase::P);
        let before=crate::publisher::files::capture_payload_files(&config).await.unwrap();
        assert!(after_start(&config,&store).await.is_err());
        assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),before);
        assert_eq!(Operation::load(&store,&operation.id).unwrap().phase,Phase::P);
        assert!(!crate::publisher::confirmation::uncertain(&config).await);
    }
    #[tokio::test]
    async fn pre_c_restart_restores_every_manifest_file_and_stops_before_fresh_r1_r2() {
        let (_temp,config,store,old,new)=durable_fixture();
        let operation=unfinished(&store,&old,&new,Phase::P);
        let mut mixed=store.payload(&old).unwrap().files;
        mixed.insert("conf/server-fixture.conf".into(),b"partial new HTTP".to_vec());
        crate::publisher::files::restore_payload_files(&config,&mixed).await.unwrap();
        assert!(store.write_recovery(&config,&operation).await.is_err(),"forward P alone is not a restore-write intent");
        assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),mixed);
        before_start(&config,&store).await.unwrap();
        assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),store.payload(&old).unwrap().files);
        let recovered=Operation::load(&store,&operation.id).unwrap();
        assert_eq!(recovered.restore_target.as_deref(),Some(old.as_str()));assert_eq!(recovered.phase,Phase::RestoreValidated);
        assert!(!recovered.commit_unknown);assert!(!recovered.resolved());
        assert!(crate::publisher::confirmation::uncertain(&config).await);
    }

    #[tokio::test]
    async fn interrupted_restore_always_rewrites_the_complete_fixed_target_even_after_unknown_commit() {
        let (_temp,config,store,old,new)=durable_fixture();
        let mut operation=unfinished(&store,&old,&new,Phase::CommitUnknown);operation.commit_unknown=true;
        operation.begin_restore(&store,&old,"lost COMMIT response").unwrap();
        for phase in [Phase::RestoreIntent,Phase::RestoreWriting,Phase::RestoreValidated,Phase::R1,Phase::R2,Phase::Restored] {
            operation.phase=phase;operation.save(&store).unwrap();
            crate::publisher::files::restore_payload_files(&config,&store.payload(&new).unwrap().files).await.unwrap();
            before_start(&config,&store).await.unwrap();
            assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),store.payload(&old).unwrap().files);
            let recovered=Operation::load(&store,&operation.id).unwrap();
            assert_eq!(recovered.phase,Phase::RestoreValidated);assert!(recovered.commit_unknown);
            assert_eq!(recovered.restore_target.as_deref(),Some(old.as_str()));
        }
    }

    #[tokio::test]
    async fn c_without_restore_keeps_full_next_uncertain_and_completed_records_never_overwrite_later_live() {
        let (_temp,config,store,old,new)=durable_fixture();
        let operation=unfinished(&store,&old,&new,Phase::C);
        crate::publisher::files::restore_payload_files(&config,&store.payload(&old).unwrap().files).await.unwrap();
        before_start(&config,&store).await.unwrap();
        assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),store.payload(&new).unwrap().files);
        let recovered=Operation::load(&store,&operation.id).unwrap();
        assert_eq!(recovered.phase,Phase::CommitUnknown);assert!(recovered.uncertain&&recovered.commit_unknown);
        assert!(recovered.restore_target.is_none());assert!(recovered.result.is_some());assert!(!recovered.resolved());
        let mut superseded=recovered;superseded.advance(&store,Phase::Superseded{by:uuid::Uuid::parse_str(&old).unwrap()}).unwrap();
        assert!(superseded.resolved());assert!(!superseded.gc_eligible(false));
        store.select(&old).unwrap();crate::publisher::files::restore_payload_files(&config,&store.payload(&old).unwrap().files).await.unwrap();
        before_start(&config,&store).await.unwrap();
        assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),store.payload(&old).unwrap().files);
    }

    #[tokio::test]
    async fn missing_target_or_a_live_master_refuses_instead_of_adopting_the_mixed_live_set() {
        let (temp,config,store,old,new)=durable_fixture();
        let operation=unfinished(&store,&old,&new,Phase::P);
        crate::publisher::files::restore_payload_files(&config,&store.payload(&new).unwrap().files).await.unwrap();
        let before=crate::publisher::files::capture_payload_files(&config).await.unwrap();
        std::fs::write(temp.path().join("nginx.pid"),std::process::id().to_string()).unwrap();
        assert!(before_start(&config,&store).await.unwrap_err().to_string().contains("still running"));
        assert_eq!(Operation::load(&store,&operation.id).unwrap().phase,Phase::P);
        std::fs::remove_file(temp.path().join("nginx.pid")).unwrap();
        std::fs::remove_dir_all(store.root.join("payloads").join(&old)).unwrap();
        let error=before_start(&config,&store).await.unwrap_err();
        assert!(format!("{error:#}").contains(&format!("restore-incomplete: {old}")));
        assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),before);
        assert_eq!(Operation::load(&store,&operation.id).unwrap().restore_target.as_deref(),Some(old.as_str()));
    }

}
