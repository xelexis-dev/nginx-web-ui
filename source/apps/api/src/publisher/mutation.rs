//! Durable candidate writes shared by normal apply and snapshot rollback.
use super::{certificates::Validated,confirmation::Budget,files::RenderedConfig,journal::{Operation,Phase},store::{Origin,PublisherStore}};
use crate::{config::RuntimeConfig,control::model::ApplyResult};
use anyhow::{Context,Result,ensure};
use std::{collections::BTreeMap,time::{Duration,Instant}};

pub(super) struct Mutation {
    pub store:PublisherStore,
    pub operation:Operation,
    pub previous_confirmed:bool,
    materials:BTreeMap<String,super::files::CertificateMaterial>,
    digests:BTreeMap<String,String>,
    previous_files:BTreeMap<String,Vec<u8>>,
}
impl Mutation {
    /// Caller owns store, apply and files locks. Existing durable current is authoritative.
    pub async fn begin(config:&RuntimeConfig,store:PublisherStore,mut operation:Operation,certificates:Vec<Validated>)->Result<Self> {
        ensure!(operation.phase==Phase::Intent && operation.next.is_none(),"Mutation already started");
        operation.previous_uncertain=super::confirmation::uncertain(config).await;
        operation.save(&store)?;
        let previous=store.ensure_previous(config,&operation).await?;
        let previous_files=store.payload(&previous)?.files;
        ensure!(super::files::capture_payload_files(config).await?==previous_files,"Live files differ from the selected durable payload");
        let previous_confirmed=!operation.previous_uncertain;
        operation.previous=Some(previous);operation.save(&store)?;
        let mut materials=BTreeMap::new();let mut digests=BTreeMap::new();
        for certificate in certificates {
            let id=certificate.id().to_owned();digests.insert(id.clone(),certificate.content_digest().to_owned());
            materials.insert(id,certificate.into_material());
        }
        Ok(Self{store,operation,previous_confirmed,materials,digests,previous_files})
    }
    pub fn previous_rendered(&self)->RenderedConfig {
        let mut rendered=RenderedConfig::default();
        for (name,bytes) in &self.previous_files {
            if let Some(name)=name.strip_prefix("conf/") {if super::files::ManagedKind::Conf.matches(name){rendered.conf.insert(name.into(),bytes.clone());}}
            if let Some(name)=name.strip_prefix("stream/") {if super::files::ManagedKind::Stream.matches(name){rendered.stream.insert(name.into(),bytes.clone());}}
        }
        rendered
    }
    pub async fn write_candidate(&mut self,config:&RuntimeConfig,rendered:&RenderedConfig)->Result<()> {
        ensure!(!self.operation.commit_unknown && matches!(self.operation.phase,Phase::Intent|Phase::LiveWriting|Phase::Validated|Phase::P|Phase::C),"Candidate cannot replace a committed or restoring operation");
        super::vocab::validate(config,&rendered.conf,&rendered.stream)?;
        let mut files=BTreeMap::new();
        for (prefix,set) in [("conf",&rendered.conf),("stream",&rendered.stream)] {
            files.extend(set.iter().map(|(name,bytes)|(format!("{prefix}/{name}"),bytes.clone())));
        }
        // Normal apply preserves the boot-owned loopback upstream file.
        if let Some(bytes)=self.previous_files.get("conf/00-runtime-upstreams.conf") {
            files.entry("conf/00-runtime-upstreams.conf".into()).or_insert_with(||bytes.clone());
        }
        let mut digests=BTreeMap::new();
        for id in super::files::certificate_consumers(config,rendered)?.keys() {
            let material=self.materials.get(id).context("Rendered certificate material is unavailable")?;
            files.insert(format!("certs/{id}/fullchain.pem"),material.cert_pem.as_bytes().to_vec());
            files.insert(format!("certs/{id}/privkey.pem"),material.key_pem.as_bytes().to_vec());
            digests.insert(id.clone(),self.digests.get(id).context("Certificate digest is unavailable")?.clone());
        }
        let id=uuid::Uuid::new_v4().to_string();
        let capacity=super::capacity::StageBudget::new(config,&self.operation,&self.previous_files,!super::confirmation::uncertain(config).await);
        self.store.stage_with_budget(&id,Origin::Publisher,None,digests,files,&capacity)?;
        let before=self.operation.clone();
        self.operation.next=Some(id);self.operation.uncertain=true;
        let intent=if self.operation.phase==Phase::Intent {
            self.operation.advance(&self.store,Phase::PayloadStaged)
        } else {
            // W-11 can replace a candidate after ACKs, before any metadata commit attempt.
            self.operation.phase=Phase::LiveWriting;self.operation.save(&self.store)
        };
        if let Err(error)=intent {self.operation=before;return Err(error);}
        if self.operation.phase==Phase::PayloadStaged {self.operation.advance(&self.store,Phase::LiveWriting)?;}
        self.store.write_live(config,&self.operation).await
    }
    pub async fn validate(&self,config:&RuntimeConfig,timeout:Duration)->super::process::ValidationResult {
        match super::command::execute(self.store.clone(),config.nginx_bin.clone(),vec!["-t".into()],timeout).await {
            Ok(output)=>super::process::ValidationResult{ok:true,output},
            Err(error)=>super::process::ValidationResult{ok:false,output:error.detail.clone().unwrap_or_else(||error.to_string())},
        }
    }
    pub async fn freeze(&mut self,config:&RuntimeConfig,budget:&Budget)->Result<(String,String)> {
        budget.forward()?;
        let first=uuid::Uuid::new_v4().to_string();super::confirmation::publish(config,&first).await?;
        let frozen=self.validate(config,budget.remaining().min(Duration::from_secs(30))).await;
        ensure!(frozen.ok,"{}",frozen.output);
        self.operation.advance(&self.store,Phase::Validated)?;
        Ok((first,frozen.output))
    }
    pub async fn observe(&mut self,config:&RuntimeConfig,first:&str,budget:&Budget)->Result<()> {
        let witness=super::confirmation::Witness::from_config(config)?;
        witness.confirm_with(config,first,budget.ack_deadline()?,&self.store,|phase|self.operation.advance(&self.store,if phase==0 {Phase::P}else{Phase::C})).await
    }
    #[cfg(test)]
    pub async fn acknowledge(&mut self,config:&RuntimeConfig,budget:&Budget)->Result<String> {
        let (first,output)=self.freeze(config,budget).await?;
        self.observe(config,&first,budget).await?;Ok(output)
    }
    pub fn unknown_commit(&mut self)->Result<()> {self.operation.advance(&self.store,Phase::CommitUnknown)}
    pub async fn complete(&mut self,config:&RuntimeConfig,result:&super::protocol::OpResult)->Result<()> {
        self.operation.advance(&self.store,Phase::DbCommitted)?;
        self.store.select(self.operation.next.as_deref().context("Missing committed payload")?)?;
        super::confirmation::clear_uncertain(config).await?;
        self.operation.uncertain=false;self.operation.result=Some(serde_json::to_value(result)?);
        self.operation.advance(&self.store,Phase::Done)
    }
    pub async fn restore(&mut self,config:&RuntimeConfig,budget:&Budget,reason:&str,mut result:super::protocol::OpResult)->Result<ApplyResult> {
        let previous=self.operation.previous.clone().context("Missing previous payload")?;
        let counts=self.previous_rendered();
        let restored=(!self.operation.commit_unknown&&self.previous_confirmed).then_some(crate::control::model::RestoredCounts{conf:counts.conf.len(),stream:counts.stream.len()});
        if let super::protocol::OpResult::Ok{apply:Some(apply),uncertain,..}=&mut result {apply.rolled_back_to=restored.clone();*uncertain=true;}
        self.operation.result=Some(serde_json::to_value(result)?);
        if self.operation.restore_target.is_none() {
            // Reasons are fixed codes; diagnostics stay in the final result, not the restore selector.
            self.operation.begin_restore(&self.store,&previous,reason)?;
        } else {self.operation.save(&self.store)?;}
        super::recovery::finish_restore(config,&self.store,&mut self.operation,Instant::now()+budget.remaining()).await?;
        let result=self.operation.result.clone().context("Restoration result missing")?;
        Ok(match serde_json::from_value::<super::protocol::OpResult>(result)? {
            super::protocol::OpResult::Ok{apply:Some(apply),..}=>apply,
            super::protocol::OpResult::Failed{output,..}=>ApplyResult{ok:false,output,rolled_back_to:restored},
            _=>anyhow::bail!("Invalid restoration result"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publisher::{journal::Admission,protocol::OpResult};
    fn fixture()->(tempfile::TempDir,crate::state::AppState,PublisherStore,Operation) {
        let (temp,state,store,_)=super::super::adoption::tests::fixture();
        let Admission::New(operation)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&"a".repeat(64)).unwrap() else{panic!()};
        (temp,state,store,operation)
    }
    fn candidate(state:&crate::state::AppState)->RenderedConfig {
        let data=crate::control::model::ConfigSnapshotData {servers:vec![serde_json::json!({"id":"11111111-1111-4111-8111-111111111111","serverName":"mutation.example.test","listen":18081,"enabled":true,"config":{"proxyPass":"http://127.0.0.1:19000"}})],..Default::default()};
        super::super::apply::render_config(&state.config,&data,&[]).rendered
    }
    #[tokio::test]
    async fn boot_owned_upstream_file_is_not_counted_as_a_legacy_restored_resource() {
        let (_temp,state,store,operation)=fixture();let id=uuid::Uuid::new_v4().to_string();
        let files=BTreeMap::from([("conf/00-runtime-upstreams.conf".into(),b"boot upstream bytes".to_vec()),("conf/server-11111111-1111-4111-8111-111111111111.conf".into(),b"managed server bytes".to_vec())]);
        store.stage(&id,Origin::Publisher,None,BTreeMap::new(),files.clone()).unwrap();store.select(&id).unwrap();
        super::super::files::restore_payload_files(&state.config,&files).await.unwrap();
        let mutation=Mutation::begin(&state.config,store,operation,Vec::new()).await.unwrap();
        let previous=mutation.previous_rendered();assert_eq!(previous.conf.len(),1);assert!(!previous.conf.contains_key("00-runtime-upstreams.conf"));
        assert!(mutation.previous_files.contains_key("conf/00-runtime-upstreams.conf"));
    }
    #[tokio::test]
    async fn every_candidate_has_a_durable_intent_and_preserves_the_fixed_previous() {
        let (_temp,state,store,operation)=fixture();let previous=store.manifest().unwrap().current.unwrap();
        let mut mutation=Mutation::begin(&state.config,store.clone(),operation,Vec::new()).await.unwrap();
        let intent=Operation::load(&store,&mutation.operation.id).unwrap();
        assert_eq!(intent.phase,Phase::Intent);assert!(!intent.previous_uncertain);
        mutation.write_candidate(&state.config,&candidate(&state)).await.unwrap();
        let first=mutation.operation.next.clone().unwrap();
        assert!(super::super::confirmation::uncertain(&state.config).await);
        assert_eq!(Operation::load(&store,&mutation.operation.id).unwrap().phase,Phase::LiveWriting);
        assert_eq!(super::super::files::capture_payload_files(&state.config).await.unwrap(),store.payload(&first).unwrap().files);
        mutation.write_candidate(&state.config,&RenderedConfig::default()).await.unwrap();
        assert_ne!(mutation.operation.next.as_deref(),Some(first.as_str()));assert_eq!(mutation.operation.previous.as_deref(),Some(previous.as_str()));
        assert!(super::super::files::capture_payload_files(&state.config).await.unwrap().is_empty());
        assert_eq!(store.manifest().unwrap().current.as_deref(),Some(previous.as_str()),"uncommitted candidates never replace current");
    }
    #[tokio::test]
    async fn pc_and_committed_completion_persist_the_exact_result_and_clear_a8() {
        let (_temp,state,store,operation)=fixture();let id=operation.id.clone();
        let mut mutation=Mutation::begin(&state.config,store.clone(),operation,Vec::new()).await.unwrap();
        mutation.write_candidate(&state.config,&candidate(&state)).await.unwrap();
        mutation.acknowledge(&state.config,&Budget::new()).await.unwrap();assert_eq!(mutation.operation.phase,Phase::C);
        let result=OpResult::Ok{apply:Some(ApplyResult{ok:true,output:"full result".into(),rolled_back_to:None}),skipped:Vec::new(),uncertain:false,phase:"done".into()};
        mutation.complete(&state.config,&result).await.unwrap();
        assert!(!super::super::confirmation::uncertain(&state.config).await);
        let done=Operation::load(&store,&id).unwrap();assert_eq!(done.phase,Phase::Done);assert!(!done.uncertain);
        assert_eq!(done.result,Some(serde_json::to_value(result).unwrap()));assert_eq!(store.manifest().unwrap().current,done.next);
    }
    #[tokio::test]
    async fn unknown_commit_restores_the_full_previous_and_retains_a8() {
        let (_temp,state,store,operation)=fixture();let id=operation.id.clone();let budget=Budget::new();
        let mut mutation=Mutation::begin(&state.config,store.clone(),operation,Vec::new()).await.unwrap();
        mutation.write_candidate(&state.config,&candidate(&state)).await.unwrap();mutation.acknowledge(&state.config,&budget).await.unwrap();
        mutation.unknown_commit().unwrap();
        // The existing nginx double also supplies the root include-closure header for restoration.
        let original=std::fs::read_to_string(&state.config.nginx_bin).unwrap();
        std::fs::write(&state.config.nginx_bin,original.replacen("#!/bin/sh\n",&format!("#!/bin/sh\nif [ \"$1\" = -T ]; then printf '# configuration file %s:\\n' '{}'; exit 0; fi\n",state.config.nginx_main_conf.display()),1)).unwrap();
        let result=mutation.restore(&state.config,&budget,"test_commit_unknown",OpResult::Failed{code:"publish_failed".into(),output:"test unknown commit".into()}).await.unwrap();assert!(result.rolled_back_to.is_none());
        let restored=Operation::load(&store,&id).unwrap();assert!(restored.commit_unknown&&restored.uncertain);assert_eq!(restored.phase,Phase::Done);
        assert_eq!(super::super::files::capture_payload_files(&state.config).await.unwrap(),mutation.previous_files);
        assert!(super::super::confirmation::uncertain(&state.config).await);
    }
}
