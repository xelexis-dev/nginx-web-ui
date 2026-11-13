//! Boot publication is a durable file transaction; running-nginx adoption confirms it later.
use crate::{config::RuntimeConfig,upgrade::StepError};
use super::{boot::PublishReport,journal::{Admission,Operation,Phase},store::{PublisherStore,Origin}};
use anyhow::Result;

fn failed(error:impl std::fmt::Display)->StepError {super::store::step_error("publish_failed",error)}
fn incomplete(error:impl std::fmt::Display)->StepError {super::store::step_error("restore_incomplete",error)}

pub(super) async fn publish(mut config:RuntimeConfig,store:PublisherStore,id:String,admitted:crate::nonroot::Admitted)->Result<PublishReport,StepError> {
    config.nginx_bin=admitted.control_bin;config.nginx_main_conf=admitted.conf_path;
    let _store_lock=store.lock(std::time::Duration::from_secs(30)).await.map_err(failed)?;
    let _files=crate::publisher::nginx_lock::acquire(&config).await.map_err(failed)?;
    // Both locks remain held through the down check, done closure and current pointer update.
    crate::publisher::confirmation::ensure_nginx_stopped(&config).await.map_err(failed)?;
    let payload=store.payload(&id).map_err(failed)?;
    if payload.manifest.origin!=Origin::Publisher {return Err(failed("Boot publication requires a publisher payload"));}
    super::recovery::supersede_for_boot(&config,&store,&id).await.map_err(incomplete)?;
    let digest=super::journal::boot_digest(&id,&payload.manifest_sha256).map_err(failed)?;
    let operation_id=uuid::Uuid::new_v4().to_string();let planned=Operation::planned(&operation_id,&digest);
    let previous=store.ensure_previous(&config,&planned).await.map_err(super::capacity::step)?;
    let Admission::New(mut operation)=Operation::admit(&store,&operation_id,&digest).map_err(failed)? else {return Err(failed("Boot operation identity collision"));};
    operation.previous=Some(previous);operation.next=Some(id.clone());
    operation.advance(&store,Phase::PayloadStaged).map_err(failed)?;
    let attempt=async {
        crate::publisher::confirmation::begin_uncertain(&config).await?;
        operation.advance(&store,Phase::LiveWriting)?;
        store.write_live(&config,&operation).await?;
        let output=super::command::validate_includes(store.clone(),config.nginx_bin.clone(),None,std::time::Duration::from_secs(30)).await
            .map_err(|error|anyhow::anyhow!("{}: {}",error.code,error.detail.as_deref().unwrap_or(error.message)))?;
        operation.advance(&store,Phase::Validated)?;
        operation.result=Some(serde_json::to_value(super::protocol::OpResult::Ok{apply:None,skipped:Vec::new(),uncertain:true,phase:"boot-published".into()})?);
        // No fabricated P/C: done closes file publication only; A8 stays until adoption.
        operation.complete_boot(&store)?;
        store.select(&id)?;
        // Re-scan after publication: DNS/material drift is a new immutable derived generation.
        let guarded=super::bootstrap::guard_durable(&config,&config.nginx_main_conf,&config.nginx_quarantine_dir,&store,None).await?;
        let selected=store.manifest()?.current.ok_or_else(||anyhow::anyhow!("Published generation missing"))?;
        let output=if guarded.validation_output.is_empty(){output}else{format!("{output}\n{}",guarded.validation_output)};
        Ok::<_,anyhow::Error>(PublishReport {files_written:store.payload(&selected)?.files.len(),validation_output:output})
    }.await;
    match attempt {
        Ok(report)=>Ok(report),
        Err(error)=>{
            // A completed boot transaction only needs its current pointer finished, never rollback.
            super::recovery::before_start(&config,&store).await.map_err(incomplete)?;
            Err(failed(error))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::recovery::tests::{durable_fixture,unfinished};
    fn admitted(config:&RuntimeConfig)->crate::nonroot::Admitted {
        crate::nonroot::Admitted{master_bin:config.nginx_bin.clone(),control_bin:config.nginx_bin.clone(),conf_path:config.nginx_main_conf.clone(),conf_mismatch:true}
    }
    #[tokio::test]
    async fn activation_completes_then_supersedes_pending_restores_without_clearing_a8() {
        for phase in [Phase::RestoreValidated,Phase::RestoreWriting] {
            let (_temp,config,store,old,new)=durable_fixture();
            let mut operation=unfinished(&store,&old,&new,Phase::P);
            operation.begin_restore(&store,&old,"interrupted restore").unwrap();operation.phase=phase;operation.save(&store).unwrap();
            crate::publisher::files::restore_payload_files(&config,&store.payload(&new).unwrap().files).await.unwrap();
            let report=publish(config.clone(),store.clone(),new.clone(),admitted(&config)).await.unwrap();
            assert_eq!(report.files_written,4);
            assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),store.payload(&new).unwrap().files);
            let closed=Operation::load(&store,&operation.id).unwrap();
            assert_eq!(closed.phase,Phase::Superseded{by:uuid::Uuid::parse_str(&new).unwrap()});
            let boot=super::super::journal::operations(&store).unwrap().into_iter().find(|op|op.id!=operation.id).unwrap();
            assert_eq!(boot.phase,Phase::Done);assert!(boot.resolved()&&boot.uncertain&&!boot.gc_eligible(false));
            assert_eq!(boot.previous.as_deref(),Some(old.as_str()));assert_eq!(boot.next.as_deref(),Some(new.as_str()));
            assert!(crate::publisher::confirmation::uncertain(&config).await);
            super::super::recovery::before_start(&config,&store).await.unwrap();
            assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),store.payload(&new).unwrap().files);
            assert!(crate::publisher::confirmation::uncertain(&config).await);
        }
    }
    #[tokio::test]
    async fn activation_refuses_a_missing_restore_target_before_creating_a_new_intent() {
        let (_temp,config,store,old,new)=durable_fixture();
        let mut operation=unfinished(&store,&old,&new,Phase::P);operation.begin_restore(&store,&old,"interrupted restore").unwrap();
        crate::publisher::files::restore_payload_files(&config,&store.payload(&new).unwrap().files).await.unwrap();
        let before=crate::publisher::files::capture_payload_files(&config).await.unwrap();
        std::fs::remove_file(store.root.join("payloads").join(&old).join("conf/server-fixture.conf")).unwrap();
        let error=publish(config.clone(),store.clone(),new,admitted(&config)).await.unwrap_err();
        assert_eq!(error.code,"restore_incomplete");assert!(error.detail.unwrap().contains(&old));
        assert_eq!(super::super::journal::operations(&store).unwrap().len(),1);
        assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),before);
    }
    #[tokio::test]
    async fn activation_resumes_after_supersede_or_a_partial_new_publication() {
        for partial_write in [false,true] {
            let (temp,config,store,old,new)=durable_fixture();
            let operation=unfinished(&store,&old,&new,Phase::P);
            crate::publisher::files::restore_payload_files(&config,&store.payload(&new).unwrap().files).await.unwrap();
            {
                let _store_lock=store.lock(std::time::Duration::from_secs(1)).await.unwrap();
                let _files=crate::publisher::nginx_lock::acquire(&config).await.unwrap();
                super::super::recovery::supersede_for_boot(&config,&store,&new).await.unwrap();
                assert_eq!(Operation::load(&store,&operation.id).unwrap().phase,Phase::Superseded{by:uuid::Uuid::parse_str(&new).unwrap()});
                if partial_write {
                    let digest=super::super::journal::boot_digest(&new,&store.payload(&new).unwrap().manifest_sha256).unwrap();
                    let Admission::New(mut new_op)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&digest).unwrap() else {panic!()};
                    new_op.previous=Some(old.clone());new_op.next=Some(new.clone());new_op.advance(&store,Phase::PayloadStaged).unwrap();new_op.advance(&store,Phase::LiveWriting).unwrap();
                    std::fs::write(config.nginx_conf_dir.join("server-fixture.conf"),b"interrupted next HTTP").unwrap();
                }
            }
            drop(store);let reopened=PublisherStore::open_test_root(temp.path()).unwrap();
            publish(config.clone(),reopened.clone(),new.clone(),admitted(&config)).await.unwrap();
            assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),reopened.payload(&new).unwrap().files);
            assert!(crate::publisher::confirmation::uncertain(&config).await);
        }
    }
    #[tokio::test]
    async fn done_before_current_is_adopted_without_restoring_previous_or_clearing_a8() {
        let (_temp,config,store,old,new)=durable_fixture();
        let digest=super::super::journal::boot_digest(&new,&store.payload(&new).unwrap().manifest_sha256).unwrap();
        let Admission::New(mut operation)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&digest).unwrap() else {panic!()};
        operation.previous=Some(old.clone());operation.next=Some(new.clone());operation.phase=Phase::Validated;
        operation.result=Some(serde_json::json!({"status":"ok","apply":null,"skipped":[],"uncertain":true,"phase":"boot-published"}));
        crate::publisher::confirmation::begin_uncertain(&config).await.unwrap();
        crate::publisher::files::restore_payload_files(&config,&store.payload(&new).unwrap().files).await.unwrap();
        operation.save(&store).unwrap();
        operation.complete_boot(&store).unwrap();
        assert_eq!(store.manifest().unwrap().current.as_deref(),Some(old.as_str()));
        super::super::recovery::before_start(&config,&store).await.unwrap();
        assert_eq!(store.manifest().unwrap().current.as_deref(),Some(new.as_str()));
        assert_eq!(crate::publisher::files::capture_payload_files(&config).await.unwrap(),store.payload(&new).unwrap().files);
        assert!(crate::publisher::confirmation::uncertain(&config).await);
        let mut ordinary=operation.clone();ordinary.command_digest="f".repeat(64);ordinary.phase=Phase::Validated;
        assert!(ordinary.complete_boot(&store).is_err(),"ordinary mutators cannot skip P/C");
    }
}
