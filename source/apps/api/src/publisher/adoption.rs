//! Only the publisher confirms the selected live generation and clears A8.
use crate::{config::RuntimeConfig,upgrade::StepError};
use super::{journal::{Operation,Phase},store::PublisherStore};
fn failed(error:impl std::fmt::Display)->StepError {super::store::step_error("restore_incomplete",error)}

#[cfg(test)]
pub(super) async fn adopt_current(config:&RuntimeConfig,store:&PublisherStore)->Result<(),StepError> {
    let _store_lock=store.lock(std::time::Duration::from_secs(30)).await.map_err(failed)?;
    adopt_locked(config,store).await
}
pub(super) async fn adopt_locked(config:&RuntimeConfig,store:&PublisherStore)->Result<(),StepError> {
    let _files=crate::publisher::nginx_lock::acquire(config).await.map_err(failed)?;
    let records=super::journal::operations(store).map_err(failed)?;
    if records.iter().any(|op|!op.terminal()) {return Err(failed("Running restore reconciliation is still required"));}
    let current=store.manifest().map_err(failed)?.current.ok_or_else(||failed("Publisher has no selected payload"))?;
    let payload=store.payload(&current).map_err(failed)?;
    if crate::publisher::files::capture_payload_files(config).await.map_err(failed)?!=payload.files {return Err(failed(format!("Live payload differs from selected generation {current}")));}
    let this_boot=crate::upgrade::boot_identity::current()?;
    let eligible=|op:&Operation|payload.manifest.derived_from.is_none() && op.phase==Phase::Done && op.restore_target.is_none() && op.next.as_deref()==Some(current.as_str())
        && op.boot_identity.as_deref()==Some(this_boot.as_str()) && op.result.as_ref().and_then(|value|value["phase"].as_str())==Some("boot-published");
    // A later unknown restore of an earlier boot generation cannot borrow its old confirmation.
    let foreign_uncertainty=records.iter().any(|op|!eligible(op) && (op.uncertain||op.commit_unknown)
        && !matches!(op.phase,Phase::Superseded{by} if by.to_string()==current)
        && [&op.previous,&op.next,&op.restore_target].into_iter().flatten().any(|id|id==&current));
    if foreign_uncertainty {return Ok(());}
    let mut matching:Vec<Operation>=records.into_iter().filter(eligible).collect();
    if matching.is_empty() {return Ok(());}
    if !crate::publisher::confirmation::uncertain(config).await && matching.iter().all(|op|!op.uncertain) {return Ok(());}
    crate::publisher::confirmation::begin_uncertain(config).await.map_err(failed)?;
    let first=uuid::Uuid::new_v4().to_string();
    crate::publisher::confirmation::publish(config,&first).await.map_err(failed)?;
    super::command::execute(store.clone(),config.nginx_bin.clone(),vec!["-t".into()],std::time::Duration::from_secs(30)).await.map_err(|error|StepError{code:error.code,message:error.message,detail:Some(format!("nginx -t failed: {}",config.nginx_main_conf.display()))})?;
    let witness=crate::publisher::confirmation::Witness::from_config(config).map_err(failed)?;
    let child_store=store.clone();let bin=config.nginx_bin.clone();
    witness.confirm_with(config,&first,std::time::Instant::now()+std::time::Duration::from_secs(15),move |timeout| {
        let store=child_store.clone();let bin=bin.clone();
        async move {super::command::execute(store,bin,vec!["-s".into(),"reload".into()],timeout).await.map(|_|()).map_err(anyhow::Error::from)}
    },|_|Ok(())).await.map_err(failed)?;
    // A.5: clearing follows fresh observation; confirming the record follows clearing.
    crate::publisher::confirmation::clear_uncertain(config).await.map_err(failed)?;
    for operation in &mut matching {
        operation.uncertain=false;
        if let Some(result)=operation.result.as_mut().and_then(serde_json::Value::as_object_mut) {
            if let Some(uncertain)=result.get_mut("uncertain") {*uncertain=serde_json::Value::Bool(false);}
        }
        operation.save(store).map_err(failed)?;
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::state::AppState;
    use std::collections::BTreeMap;
    pub(in crate::publisher) fn fixture()->(tempfile::TempDir,AppState,PublisherStore,String) {
        use super::super::test_helpers::restart_guard::*;
        let temp=tempfile::tempdir().unwrap();let paths=paths(temp.path());let nginx=Nginx::new(temp.path());
        let state=api_state(temp.path(),&nginx,&paths,"1");
        let store=PublisherStore::open_test_root(&temp.path().join("publisher-state")).unwrap();
        let old=uuid::Uuid::new_v4().to_string();let next=uuid::Uuid::new_v4().to_string();
        for id in [&old,&next] {store.stage(id,super::super::store::Origin::Publisher,None,BTreeMap::new(),BTreeMap::new()).unwrap();}
        store.select(&old).unwrap();
        let digest=super::super::journal::boot_digest(&next,&store.payload(&next).unwrap().manifest_sha256).unwrap();
        let super::super::journal::Admission::New(mut operation)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&digest).unwrap() else{panic!()};
        operation.previous=Some(old);operation.next=Some(next.clone());operation.phase=Phase::Validated;
        operation.result=Some(serde_json::to_value(super::super::protocol::OpResult::Ok{apply:None,skipped:Vec::new(),uncertain:true,phase:"boot-published".into()}).unwrap());operation.save(&store).unwrap();operation.complete_boot(&store).unwrap();store.select(&next).unwrap();
        (temp,state,store,operation.id)
    }
    #[tokio::test]
    async fn g36_adoption_validation_error_never_exposes_child_output() {
        let(_temp,state,store,_id)=fixture();
        crate::publisher::confirmation::begin_uncertain(&state.config).await.unwrap();
        std::fs::write(&state.config.nginx_bin,"#!/bin/sh\nprintf 'G36_SECRET_SENTINEL server { secret value; }\\n' >&2\nexit 1\n").unwrap();
        let records=serde_json::to_value(super::super::journal::operations(&store).unwrap()).unwrap();
        let error=adopt_current(&state.config,&store).await.unwrap_err();
        assert_eq!(error.code,"publish_failed");
        assert_eq!(serde_json::to_value(super::super::journal::operations(&store).unwrap()).unwrap(),records,"a refused adoption must not write a result or receipt");
        fn check_records(path:&std::path::Path) {
            for item in std::fs::read_dir(path).unwrap(){let entry=item.unwrap();let kind=entry.file_type().unwrap();if kind.is_dir(){check_records(&entry.path());}else if kind.is_file(){assert!(!std::fs::read(entry.path()).unwrap().windows(b"G36_SECRET_SENTINEL".len()).any(|bytes|bytes==b"G36_SECRET_SENTINEL"),"raw child content reached a journal/receipt: {:?}",entry.path());}}
        }
        check_records(&store.root);
        let detail=error.detail.unwrap();assert!(!detail.contains("G36_SECRET_SENTINEL"));assert!(!detail.contains("server {"));
        assert!(detail.contains(&state.config.nginx_main_conf.display().to_string()));
    }
    #[tokio::test]
    async fn only_fresh_two_phase_observation_clears_a8_and_then_confirms_the_record() {
        let (_temp,state,store,id)=fixture();
        crate::publisher::confirmation::begin_uncertain(&state.config).await.unwrap();
        adopt_current(&state.config,&store).await.unwrap();
        assert!(!crate::publisher::confirmation::uncertain(&state.config).await);
        let operation=Operation::load(&store,&id).unwrap();assert!(!operation.uncertain);assert!(operation.gc_eligible(false));
        assert_eq!(operation.result.unwrap()["uncertain"],false);
        let calls=std::fs::read_to_string(state.config.nginx_conf_dir.parent().unwrap().join("calls")).unwrap();
        assert_eq!(calls.lines().filter(|line|line.ends_with("start -s reload")).count(),2);
    }
    #[tokio::test]
    async fn adoption_failure_keeps_a8_and_the_unconfirmed_record() {
        let (temp,state,store,id)=fixture();
        crate::publisher::confirmation::begin_uncertain(&state.config).await.unwrap();
        std::fs::write(temp.path().join("nginx"),"#!/bin/sh\nif [ \"$1\" = -s ]; then echo reload-refused; exit 1; fi\nexit 0\n").unwrap();
        assert!(adopt_current(&state.config,&store).await.is_err());
        assert!(crate::publisher::confirmation::uncertain(&state.config).await);assert!(Operation::load(&store,&id).unwrap().uncertain);
    }
    #[tokio::test]
    async fn adoption_never_claims_a_drifted_live_payload_or_clears_its_marker() {
        let (_temp,state,store,id)=fixture();
        crate::publisher::confirmation::begin_uncertain(&state.config).await.unwrap();
        std::fs::write(state.config.nginx_conf_dir.join("server-drift.conf"),b"different generation").unwrap();
        assert!(adopt_current(&state.config,&store).await.is_err());
        assert!(crate::publisher::confirmation::uncertain(&state.config).await);assert!(Operation::load(&store,&id).unwrap().uncertain);
        assert!(!state.config.nginx_conf_dir.parent().unwrap().join("calls").exists());
    }
    #[tokio::test]
    async fn old_boot_or_unknown_restore_cannot_clear_a8_without_a_replacement() {
        for unknown_restore in [false,true] {
            let (_temp,state,store,id)=fixture();crate::publisher::confirmation::begin_uncertain(&state.config).await.unwrap();
            if unknown_restore {
                let boot=Operation::load(&store,&id).unwrap();let current=boot.next.unwrap();let next=uuid::Uuid::new_v4().to_string();
                store.stage(&next,super::super::store::Origin::Publisher,None,BTreeMap::new(),BTreeMap::new()).unwrap();
                let super::super::journal::Admission::New(mut unknown)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&"a".repeat(64)).unwrap() else{panic!()};
                unknown.previous=Some(current.clone());unknown.next=Some(next);unknown.restore_target=Some(current);unknown.phase=Phase::Done;unknown.commit_unknown=true;unknown.uncertain=true;unknown.save(&store).unwrap();
            } else {
                let mut old=Operation::load(&store,&id).unwrap();old.boot_identity=Some("e".repeat(64));old.save(&store).unwrap();
            }
            adopt_current(&state.config,&store).await.unwrap();
            assert!(crate::publisher::confirmation::uncertain(&state.config).await);assert!(Operation::load(&store,&id).unwrap().uncertain);
            assert!(!state.config.nginx_conf_dir.parent().unwrap().join("calls").exists());
        }
    }
    #[tokio::test]
    async fn a_successful_replacement_boot_can_confirm_after_superseding_unknown_sql() {
        let (_temp,state,store,id)=fixture();crate::publisher::confirmation::begin_uncertain(&state.config).await.unwrap();
        let boot=Operation::load(&store,&id).unwrap();let current=boot.next.unwrap();
        let super::super::journal::Admission::New(mut unknown)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&"a".repeat(64)).unwrap() else{panic!()};
        unknown.previous=boot.previous.clone();unknown.next=boot.previous;unknown.phase=Phase::Superseded{by:uuid::Uuid::parse_str(&current).unwrap()};unknown.commit_unknown=true;unknown.uncertain=true;unknown.save(&store).unwrap();
        adopt_current(&state.config,&store).await.unwrap();
        assert!(!crate::publisher::confirmation::uncertain(&state.config).await);
        assert!(!Operation::load(&store,&id).unwrap().uncertain);
        let history=Operation::load(&store,&unknown.id).unwrap();assert!(history.commit_unknown&&history.uncertain&&!history.gc_eligible(false));
    }

    fn restore_nginx_fixture(temp:&std::path::Path,state:&AppState) {
        let bin=temp.join("nginx");let script=std::fs::read_to_string(&bin).unwrap();
        let header=format!("#!/bin/sh\nif [ \"$1\" = -T ]; then printf '# configuration file %s:\\n' '{}'; exit 0; fi\n",state.config.nginx_main_conf.display());
        std::fs::write(bin,script.replacen("#!/bin/sh\n",&header,1)).unwrap();
    }
    #[tokio::test]
    async fn running_restore_rewrites_the_full_target_and_keeps_unknown_sql_uncertain() {
        for unknown in [false,true] {
            let (temp,state,store,id)=fixture();restore_nginx_fixture(temp.path(),&state);
            let mut operation=Operation::load(&store,&id).unwrap();
            operation.phase=Phase::P;operation.commit_unknown=unknown;operation.uncertain=true;
            operation.previous_uncertain=false; // This fixture starts from an explicitly certain baseline.
            let target=operation.previous.clone().unwrap();
            operation.begin_restore(&store,&target,"test interrupted restore").unwrap();
            crate::publisher::confirmation::begin_uncertain(&state.config).await.unwrap();
            std::fs::write(state.config.nginx_conf_dir.join("server-partial.conf"),b"mixed generation").unwrap();
            super::super::recovery::finish_restore(&state.config,&store,&mut operation,std::time::Instant::now()+crate::publisher::confirmation::RESTORE_RESERVE).await.unwrap();
            assert_eq!(crate::publisher::files::capture_payload_files(&state.config).await.unwrap(),store.payload(&target).unwrap().files);
            let finished=Operation::load(&store,&id).unwrap();assert_eq!(finished.phase,Phase::Done);
            assert_eq!(finished.commit_unknown,unknown);assert_eq!(finished.uncertain,unknown);
            assert!(matches!(super::super::dispatcher::status(&store,&id),super::super::protocol::OpResult::Failed{..}),"restore cannot replay the original boot success or Unknown");
            assert_eq!(crate::publisher::confirmation::uncertain(&state.config).await,unknown);
            let calls=std::fs::read_to_string(state.config.nginx_conf_dir.parent().unwrap().join("calls")).unwrap();
            assert_eq!(calls.lines().filter(|line|line.ends_with("start -s reload")).count(),2);
        }
    }
    #[tokio::test]
    async fn failed_running_restore_ack_keeps_the_fixed_target_and_a8_for_retry() {
        let (temp,state,store,id)=fixture();restore_nginx_fixture(temp.path(),&state);let mut operation=Operation::load(&store,&id).unwrap();
        operation.phase=Phase::P;let target=operation.previous.clone().unwrap();operation.begin_restore(&store,&target,"test failed reload").unwrap();
        let bin=temp.path().join("nginx");let script=std::fs::read_to_string(&bin).unwrap();
        std::fs::write(&bin,script.replacen("#!/bin/sh\n","#!/bin/sh\nif [ \"$1\" = -s ]; then exit 1; fi\n",1)).unwrap();
        assert!(super::super::recovery::finish_restore(&state.config,&store,&mut operation,std::time::Instant::now()+crate::publisher::confirmation::RESTORE_RESERVE).await.is_err());
        let pending=Operation::load(&store,&id).unwrap();assert_eq!(pending.phase,Phase::RestoreValidated);assert_eq!(pending.restore_target.as_deref(),Some(target.as_str()));
        assert!(crate::publisher::confirmation::uncertain(&state.config).await);
    }

    #[tokio::test]
    async fn running_reconciliation_completes_the_fixed_restore_with_fresh_r1_r2() {
        let (temp,state,store,_)=fixture();restore_nginx_fixture(temp.path(),&state);
        let previous=store.manifest().unwrap().current.unwrap();let next=uuid::Uuid::new_v4().to_string();
        store.stage(&next,super::super::store::Origin::Publisher,None,BTreeMap::new(),BTreeMap::new()).unwrap();
        let super::super::journal::Admission::New(mut operation)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&"f".repeat(64)).unwrap() else{panic!()};
        operation.previous=Some(previous.clone());operation.next=Some(next);operation.phase=Phase::P;operation.uncertain=true;operation.save(&store).unwrap();
        let _store=store.lock(std::time::Duration::from_secs(1)).await.unwrap();let _files=crate::publisher::nginx_lock::acquire(&state.config).await.unwrap();
        crate::publisher::confirmation::begin_uncertain(&state.config).await.unwrap();
        std::fs::write(state.config.nginx_conf_dir.join("server-partial.conf"),b"partial candidate").unwrap();
        super::super::recovery::after_start(&state.config,&store).await.unwrap();
        assert_eq!(crate::publisher::files::capture_payload_files(&state.config).await.unwrap(),store.payload(&previous).unwrap().files);
        let restored=Operation::load(&store,&operation.id).unwrap();assert_eq!(restored.phase,Phase::Done);assert_eq!(restored.restore_target.as_deref(),Some(previous.as_str()));
        assert!(matches!(super::super::dispatcher::status(&store,&operation.id),super::super::protocol::OpResult::Failed{..}));
        let calls=std::fs::read_to_string(temp.path().join("calls")).unwrap();assert_eq!(calls.lines().filter(|line|line.ends_with("start -s reload")).count(),2);
    }
    #[tokio::test]
    async fn running_reconciliation_keeps_commit_unknown_current_without_clearing_a8_or_reloading() {
        let (temp,state,store,_)=fixture();restore_nginx_fixture(temp.path(),&state);
        let previous=store.manifest().unwrap().current.unwrap();let next=uuid::Uuid::new_v4().to_string();
        let files=BTreeMap::from([("conf/upstreams.conf".into(),b"upstream retained { server 192.0.2.5:80; }\n".to_vec())]);
        store.stage(&next,super::super::store::Origin::Publisher,None,BTreeMap::new(),files.clone()).unwrap();
        let super::super::journal::Admission::New(mut operation)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&"f".repeat(64)).unwrap() else{panic!()};
        operation.previous=Some(previous);operation.next=Some(next.clone());operation.phase=Phase::C;operation.save(&store).unwrap();
        let _store=store.lock(std::time::Duration::from_secs(1)).await.unwrap();let _files=crate::publisher::nginx_lock::acquire(&state.config).await.unwrap();
        super::super::recovery::after_start(&state.config,&store).await.unwrap();
        let kept=Operation::load(&store,&operation.id).unwrap();assert_eq!(kept.phase,Phase::Done);assert!(kept.commit_unknown&&kept.uncertain);
        assert_eq!(store.manifest().unwrap().current.as_deref(),Some(next.as_str()));assert_eq!(crate::publisher::files::capture_payload_files(&state.config).await.unwrap(),files);
        assert!(crate::publisher::confirmation::uncertain(&state.config).await);
        assert!(matches!(super::super::dispatcher::status(&store,&operation.id),super::super::protocol::OpResult::Ok{uncertain:true,phase,..} if phase=="commit-unknown"));
        assert!(!std::fs::read_to_string(temp.path().join("calls")).unwrap_or_default().contains("-s reload"));
    }

}
