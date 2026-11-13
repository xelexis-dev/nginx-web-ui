//! DB-blind corrections retain the prepared generation and never settle applied state.
use super::{files::RenderedConfig,journal::{Admission,Operation,Phase},store::PublisherStore};
use crate::config::RuntimeConfig;
use anyhow::{Context,Result,ensure};
use std::collections::BTreeMap;

pub(super) struct Derivation {
    store:PublisherStore,
    source:String,
    operation:Option<Operation>,
    validated:bool,
}
impl Derivation {
    /// Caller holds store and nginx-files locks for the complete correction/validation flow.
    pub fn new(store:&PublisherStore)->Result<Self> {
        let source=store.manifest()?.current.context("No selected derivation source")?;
        store.payload(&source)?;
        Ok(Self{store:store.clone(),source,operation:None,validated:false})
    }
    pub async fn write(&mut self,config:&RuntimeConfig,rendered:&RenderedConfig,reason:&str)->Result<()> {
        let source=self.store.payload(&self.source)?;
        let previous_files=source.files.clone();
        let mut files:BTreeMap<_,_>=source.files.into_iter().filter(|(name,_)|!name.starts_with("conf/")&&!name.starts_with("stream/")||name=="conf/00-runtime-upstreams.conf").collect();
        for (prefix,set) in [("conf",&rendered.conf),("stream",&rendered.stream)] {
            files.extend(set.iter().map(|(name,bytes)|(format!("{prefix}/{name}"),bytes.clone())));
        }
        let previous=self.operation.as_ref().and_then(|op|op.next.as_deref()).unwrap_or(&self.source).to_owned();
        if self.store.payload(&previous)?.files==files {return Ok(());}
        self.validated=false;
        let id=uuid::Uuid::new_v4().to_string();
        let fresh=self.operation.is_none();
        if fresh {
            let Admission::New(mut operation)=Operation::admit(&self.store,&uuid::Uuid::new_v4().to_string(),&super::store::digest(&[]))? else {anyhow::bail!("Derivation identity collision")};
            // DB-blind corrections cannot prove that the old applied-state marker was consistent.
            operation.previous_uncertain=true;operation.previous=Some(self.source.clone());operation.save(&self.store)?;
            self.operation=Some(operation);
        }
        let operation=self.operation.as_mut().unwrap();
        let capacity=super::capacity::StageBudget::new(config,operation,&previous_files,!super::confirmation::uncertain(config).await);
        let payload=match self.store.stage_derived_with_budget(&id,&self.source,reason,files,&capacity) {
            Ok(payload)=>payload,
            Err(error)=>{
                if fresh&&super::capacity::is_full(&error){operation.result=Some(serde_json::to_value(super::capacity::result())?);operation.advance(&self.store,Phase::Done)?;}
                return Err(error);
            },
        };
        operation.command_digest=super::journal::boot_digest(&id,&payload.manifest_sha256)?;operation.next=Some(id);operation.uncertain=true;
        if fresh {operation.advance(&self.store,Phase::PayloadStaged)?;operation.advance(&self.store,Phase::LiveWriting)?;}
        else {ensure!(operation.phase==Phase::LiveWriting,"Derivation already closed");operation.save(&self.store)?;}
        self.store.write_live(config,self.operation.as_ref().unwrap()).await
    }
    pub async fn validate(&mut self,config:&RuntimeConfig)->super::process::ValidationResult {
        let result=match super::command::execute(self.store.clone(),config.nginx_bin.clone(),vec!["-t".into()],std::time::Duration::from_secs(30)).await {
            Ok(output)=>super::process::ValidationResult{ok:true,output},
            Err(error)=>super::process::ValidationResult{ok:false,output:error.detail.clone().unwrap_or_else(||error.to_string())},
        };
        self.validated=result.ok;result
    }
    pub async fn finish(&mut self,config:&RuntimeConfig)->Result<()> {
        let Some(operation)=self.operation.as_mut() else {return Ok(())};
        let next=operation.next.clone().context("Derivation target missing")?;
        let payload=self.store.payload(&next)?;
        ensure!(super::files::capture_payload_files(config).await?==payload.files,"Derived live generation changed before closure");
        ensure!(self.validated,"Derived generation has no successful live validation since its last write");
        operation.advance(&self.store,Phase::Validated)?;
        operation.result=Some(serde_json::to_value(super::protocol::OpResult::Ok{apply:None,skipped:Vec::new(),uncertain:true,phase:"boot-derived".into()})?);
        operation.complete_boot(&self.store)?;
        self.store.select(&next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn q23_db_blind_guard_isolates_only_socket_consumers_and_keeps_durable_provenance() {
        let (_temp,state,store,_)=super::super::adoption::tests::fixture();let source=uuid::Uuid::new_v4().to_string();
        let files=BTreeMap::from([
            ("conf/server-internal.conf".into(),b"server { listen 18081; location / { proxy_pass http://unix:/run/nwui-publisher/control.sock; } }".to_vec()),
            ("conf/server-variable.conf".into(),b"server { listen 18082; location / { proxy_pass http://unix:/tmp/$socket:/api/; } }".to_vec()),
            ("conf/server-public.conf".into(),b"server { listen 18083; location / { proxy_pass http://unix:/tmp/q23-public.sock:$request_uri; } }".to_vec()),
        ]);
        store.stage(&source,super::super::store::Origin::Publisher,None,BTreeMap::new(),files.clone()).unwrap();store.select(&source).unwrap();super::super::files::restore_payload_files(&state.config,&files).await.unwrap();
        let report=super::super::bootstrap::guard_durable(&state.config,&state.config.nginx_main_conf,&state.config.nginx_quarantine_dir,&store,None).await.unwrap();
        assert_eq!(report.internal_target_files,2);assert!(report.validation_output.contains("unix"));assert!(report.validation_output.contains("입증"));
        let current=store.manifest().unwrap().current.unwrap();assert_ne!(current,source);let derived=store.payload(&current).unwrap();assert_eq!(derived.manifest.derived_from.as_deref(),Some(source.as_str()));
        assert_eq!(derived.files,BTreeMap::from([("conf/server-public.conf".into(),files["conf/server-public.conf"].clone())]));assert_eq!(store.payload(&source).unwrap().files,files);
        let operations=super::super::journal::operations(&store).unwrap();let operation=operations.iter().find(|op|op.next.as_deref()==Some(current.as_str())).unwrap();assert_eq!(operation.phase,Phase::Done);assert_eq!(operation.result.as_ref().unwrap()["phase"],"boot-derived");assert!(operation.uncertain);
        assert!(super::super::confirmation::uncertain(&state.config).await);
    }
    #[tokio::test]
    async fn reserved_guard_correction_keeps_the_prepared_source_and_applied_uncertainty() {
        let (_temp,state,store,_)=super::super::adoption::tests::fixture();
        let id=uuid::Uuid::new_v4().to_string();
        let bytes=b"server { listen 3001; }".to_vec();
        let files=BTreeMap::from([("conf/server-11111111-1111-4111-8111-111111111111.conf".into(),bytes)]);
        store.stage(&id,super::super::store::Origin::Publisher,None,BTreeMap::new(),files.clone()).unwrap();store.select(&id).unwrap();
        super::super::files::restore_payload_files(&state.config,&files).await.unwrap();
        let report=super::super::bootstrap::guard_durable(&state.config,&state.config.nginx_main_conf,&state.config.nginx_quarantine_dir,&store,None).await.unwrap();
        assert_eq!(report.reserved_files,1);
        let current=store.manifest().unwrap().current.unwrap();assert_ne!(current,id);
        assert_eq!(store.payload(&current).unwrap().manifest.derived_from.as_deref(),Some(id.as_str()));
        assert_eq!(store.payload(&id).unwrap().files,files);assert!(store.payload(&current).unwrap().files.is_empty());
        let ops=super::super::journal::operations(&store).unwrap();let done=ops.iter().find(|op|op.next.as_deref()==Some(current.as_str())).unwrap();
        assert_eq!(done.phase,Phase::Done);assert_eq!(done.result.as_ref().unwrap()["phase"],"boot-derived");
        super::super::adoption::adopt_current(&state.config,&store).await.unwrap();assert!(super::super::confirmation::uncertain(&state.config).await);
    }
    #[tokio::test]
    async fn correction_is_immutable_journaled_before_write_and_closed_under_actual_id() {
        let (_temp,state,store,_)=super::super::adoption::tests::fixture();
        let original=store.manifest().unwrap().current.unwrap();let source=store.payload(&original).unwrap();
        let rendered=RenderedConfig{conf:BTreeMap::from([("server-11111111-1111-4111-8111-111111111111.conf".into(),b"server { listen 18081; }".to_vec())]),stream:BTreeMap::new()};
        let mut writer=Derivation::new(&store).unwrap();writer.write(&state.config,&rendered,"guard-reserved").await.unwrap();
        let operation_id=writer.operation.as_ref().unwrap().id.clone();let persisted=Operation::load(&store,&operation_id).unwrap();
        assert_eq!(persisted.phase,Phase::LiveWriting);assert_eq!(persisted.previous.as_deref(),Some(original.as_str()));assert!(persisted.previous_uncertain);
        let next=persisted.next.clone().unwrap();assert_ne!(next,original);
        let payload=store.payload(&next).unwrap();assert_eq!(payload.manifest.derived_from.as_deref(),Some(original.as_str()));assert_eq!(payload.manifest.reason.as_deref(),Some("guard-reserved"));
        assert_eq!(store.payload(&original).unwrap().manifest_sha256,source.manifest_sha256);
        assert_eq!(super::super::files::capture_payload_files(&state.config).await.unwrap(),payload.files);
        assert!(writer.finish(&state.config).await.is_err(),"closure requires a fresh successful live validation");
        assert!(writer.validate(&state.config).await.ok);
        writer.finish(&state.config).await.unwrap();assert_eq!(store.manifest().unwrap().current.as_deref(),Some(next.as_str()));
        let done=Operation::load(&store,&operation_id).unwrap();assert_eq!(done.phase,Phase::Done);assert_eq!(done.result.unwrap()["phase"],"boot-derived");
        super::super::adoption::adopt_current(&state.config,&store).await.unwrap();assert!(super::super::confirmation::uncertain(&state.config).await);
    }
}
