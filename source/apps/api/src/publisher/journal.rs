//! Every forward and restore write has a durable, replayable operation phase.
use super::store::{PublisherStore, uuid};
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all="kebab-case")]
pub(super) enum Phase {
    Intent, PayloadStaged, LiveWriting, Validated,
    #[serde(rename="P")] P,
    #[serde(rename="C")] C,
    DbCommitted, CommitUnknown,
    RestoreIntent, RestoreWriting, RestoreValidated,
    #[serde(rename="R1")] R1,
    #[serde(rename="R2")] R2,
    Restored, Done, Superseded { by:uuid::Uuid },
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all="camelCase")]
pub(super) struct Operation {
    pub version: u8,
    pub id: String,
    pub command_digest: String,
    pub phase: Phase,
    pub previous: Option<String>,
    pub next: Option<String>,
    pub restore_target: Option<String>,
    pub restore_reason: Option<String>,
    pub commit_unknown: bool,
    #[serde(default = "previous_uncertain_default")]
    pub previous_uncertain: bool,
    pub uncertain: bool,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub boot_identity:Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub result: Option<serde_json::Value>,
}

fn previous_uncertain_default() -> bool { true }

pub(super) enum Admission {
    New(Operation),
    Replay(Operation),
    Conflict,
}

impl Operation {
    pub fn planned(id:&str,digest:&str)->Self {
        Self {version:1,id:id.into(),command_digest:digest.into(),phase:Phase::Intent,
            previous:None,next:None,restore_target:None,restore_reason:None,commit_unknown:false,previous_uncertain:true,
            uncertain:false,boot_identity:None,created_at:chrono::Utc::now(),result:None}
    }
    pub fn admit(store: &PublisherStore, id: &str, digest: &str) -> Result<Admission> {
        uuid(id)?;
        ensure!(digest.len()==64 && digest.bytes().all(|b| b.is_ascii_hexdigit()), "Invalid command digest");
        let path=path(store,id)?;
        match std::fs::symlink_metadata(&path) {
            Ok(_)=>{
                let previous=Self::load(store,id)?;
                return Ok(if previous.command_digest==digest { Admission::Replay(previous) } else { Admission::Conflict });
            }
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
            Err(error)=>return Err(error.into()),
        }
        let operation=Self::planned(id,digest);
        if let Err(error)=operation.save(store) {
            let error=super::capacity::stage_error(error);
            if super::capacity::is_full(&error) && path.exists(){operation.remove_unwritten(store)?;}
            return Err(error);
        }
        Ok(Admission::New(operation))
    }

    pub fn load(store: &PublisherStore, id: &str) -> Result<Self> {
        let operation: Self=crate::db_upgrade::files::read_json(&path(store,id)?)?;
        ensure!(operation.version==1 && operation.id==id, "Invalid operation record");
        for id in [&operation.previous,&operation.next,&operation.restore_target].into_iter().flatten() { uuid(id)?; }
        if let Some(identity)=&operation.boot_identity {ensure!(identity.len()==64 && identity.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)),"Invalid boot identity");}
        Ok(operation)
    }

    /// Only a no-live-write refusal may release its own record when the filesystem is full.
    pub(super) fn remove_unwritten(&self,store:&PublisherStore)->Result<()> {
        let stored=Self::load(store,&self.id)?;
        ensure!(matches!(stored.phase,Phase::Intent|Phase::Done)&&stored.next.is_none()&&stored.restore_target.is_none()&&!stored.commit_unknown,"Refusing to remove a publisher record with live-write history");
        let path=path(store,&self.id)?;std::fs::remove_file(&path)?;
        std::fs::File::open(path.parent().unwrap())?.sync_all()?;Ok(())
    }
    pub fn save(&self, store: &PublisherStore) -> Result<()> {
        crate::db_upgrade::files::atomic_json(&path(store,&self.id)?, self)?;
        Ok(())
    }

    pub fn advance(&mut self, store: &PublisherStore, phase: Phase) -> Result<()> {
        use Phase::*;
        let valid=matches!((self.phase,phase),
            (Intent,Done)|(PayloadStaged,Done)|(Intent,PayloadStaged)|(PayloadStaged,LiveWriting)|(LiveWriting,Validated)|(Validated,P)|(P,C)
            |(C,DbCommitted)|(C,CommitUnknown)|(DbCommitted,Done)|(CommitUnknown,Done)
            |(RestoreIntent,RestoreWriting)|(RestoreWriting,RestoreValidated)|(RestoreValidated,R1)|(R1,R2)|(R2,Restored)|(Restored,Done)
            |(RestoreValidated,Superseded{..})|(R1,Superseded{..})|(R2,Superseded{..})|(CommitUnknown,Superseded{..}));
        ensure!(valid, "Invalid publisher phase transition");
        if phase==PayloadStaged { uuid(self.next.as_deref().ok_or_else(||anyhow::anyhow!("Missing next payload"))?)?; }
        if phase==LiveWriting { uuid(self.previous.as_deref().ok_or_else(||anyhow::anyhow!("Missing previous payload"))?)?; }
        let old=self.phase;
        let old_unknown=self.commit_unknown;
        let old_next=self.next.clone();
        // Pre-write completion discards a candidate; Done must never make it publishable on replay.
        if phase==Done && matches!(old,Intent|PayloadStaged) {self.next=None;}
        self.phase=phase;
        self.commit_unknown |= phase==CommitUnknown;
        if let Err(error)=self.save(store) {
            self.phase=old;
            self.commit_unknown=old_unknown;
            self.next=old_next;
            return Err(error);
        }
        Ok(())
    }

    pub fn begin_restore(&mut self, store: &PublisherStore, target: &str, reason: &str) -> Result<()> {
        uuid(target)?;
        ensure!(self.phase!=Phase::Done && self.restore_target.is_none(), "Restore already decided");
        ensure!(!reason.is_empty() && reason.len()<=512, "Invalid restore reason");
        let before=self.clone();
        self.restore_target=Some(target.into());
        self.restore_reason=Some(reason.into());
        self.phase=Phase::RestoreIntent;
        // No caller may touch live files until this fsync has succeeded.
        if let Err(error)=self.save(store) { *self=before; return Err(error); }
        Ok(())
    }

    /// A restart repeats the whole restore and requires fresh R1/R2; it never resumes midway.
    pub fn restart_restore_write(&mut self, store:&PublisherStore) -> Result<()> {
        ensure!(self.restore_target.is_some() && matches!(self.phase,
            Phase::RestoreIntent|Phase::RestoreWriting|Phase::RestoreValidated|Phase::R1|Phase::R2|Phase::Restored),"Missing recovery restore track");
        let old=self.phase;self.phase=Phase::RestoreWriting;
        if let Err(error)=self.save(store) {self.phase=old;return Err(error);}
        Ok(())
    }

    pub fn terminal(&self)->bool {matches!(self.phase,Phase::Done|Phase::Superseded{..})}

    pub fn resolved(&self) -> bool {self.terminal()}

    pub fn gc_eligible(&self,a8_clear:bool)->bool {self.resolved() && (a8_clear || (!self.uncertain && !self.commit_unknown))}

    pub fn complete_boot(&mut self,store:&PublisherStore)->Result<()> {
        ensure!(self.phase==Phase::Validated,"Boot publication was not validated");
        let persisted=Self::load(store,&self.id)?;
        ensure!(persisted.phase==Phase::Validated && persisted.command_digest==self.command_digest && persisted.previous==self.previous && persisted.next==self.next,"Boot validation was not durably recorded");
        let id=self.next.as_deref().ok_or_else(||anyhow::anyhow!("Missing boot payload"))?;
        let manifest=store.payload(id)?.manifest_sha256;
        ensure!(self.command_digest==boot_digest(id,&manifest)?,"Only boot publication can finish without a running nginx");
        let identity=crate::upgrade::boot_identity::current()?;
        let before=self.clone();self.phase=Phase::Done;self.uncertain=true;self.boot_identity=Some(identity);
        if let Err(error)=self.save(store) {*self=before;return Err(error);}
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_uncertainty_survives_reopen_and_old_records_default_to_uncertain() {
        let temp=tempfile::tempdir().unwrap();let store=PublisherStore::at(temp.path().into());
        let Admission::New(mut operation)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&"a".repeat(64)).unwrap() else {panic!()};
        assert!(operation.previous_uncertain);
        for uncertain in [false,true] {
            operation.previous_uncertain=uncertain;operation.save(&store).unwrap();
            assert_eq!(Operation::load(&store,&operation.id).unwrap().previous_uncertain,uncertain);
        }
        let mut old=serde_json::to_value(&operation).unwrap();
        assert!(old.as_object_mut().unwrap().remove("previousUncertain").is_some());
        crate::db_upgrade::files::atomic_json(&path(&store,&operation.id).unwrap(),&old).unwrap();
        assert!(Operation::load(&store,&operation.id).unwrap().previous_uncertain);
    }
    #[test]
    fn failed_discard_save_keeps_the_in_memory_staged_next() {
        let temp=tempfile::tempdir().unwrap();let store=PublisherStore::at(temp.path().into());
        let Admission::New(mut operation)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&"a".repeat(64)).unwrap() else{panic!()};
        let next=uuid::Uuid::new_v4().to_string();operation.next=Some(next.clone());operation.advance(&store,Phase::PayloadStaged).unwrap();
        let record=store.directory("ops").unwrap().join(format!("{}.json",operation.id));
        std::fs::remove_file(&record).unwrap();std::fs::create_dir(&record).unwrap();
        assert!(operation.advance(&store,Phase::Done).is_err());
        assert_eq!(operation.phase,Phase::PayloadStaged);assert_eq!(operation.next.as_deref(),Some(next.as_str()));
        assert!(record.is_dir());
    }
    #[test]
    fn n1_replay_preserves_command_identity_and_completed_result() {
        let temp=tempfile::tempdir().unwrap();
        let store=PublisherStore::at(temp.path().into());
        let id=uuid::Uuid::new_v4().to_string();
        let digest="a".repeat(64);
        let Admission::New(mut operation)=Operation::admit(&store,&id,&digest).unwrap() else { panic!() };
        operation.result=Some(serde_json::json!({"status":"ok"}));
        operation.save(&store).unwrap();
        let Admission::Replay(replay)=Operation::admit(&store,&id,&digest).unwrap() else { panic!() };
        assert_eq!(replay.result,operation.result);
        assert!(matches!(Operation::admit(&store,&id,&"b".repeat(64)).unwrap(),Admission::Conflict));
        assert!(Operation::admit(&store,"../outside",&digest).is_err());
    }

    #[test]
    fn n1_restore_intent_overrides_unknown_commit_and_survives_reopen() {
        let temp=tempfile::tempdir().unwrap();
        let store=PublisherStore::at(temp.path().into());
        let id=uuid::Uuid::new_v4().to_string();
        let Admission::New(mut operation)=Operation::admit(&store,&id,&"a".repeat(64)).unwrap() else { panic!() };
        assert!(operation.advance(&store,Phase::C).is_err());
        operation.previous=Some(uuid::Uuid::new_v4().to_string());
        operation.next=Some(uuid::Uuid::new_v4().to_string());
        for phase in [Phase::PayloadStaged,Phase::LiveWriting,Phase::Validated,Phase::P,Phase::C,Phase::CommitUnknown] {
            operation.advance(&store,phase).unwrap();
        }
        let target=uuid::Uuid::new_v4().to_string();
        operation.begin_restore(&store,&target,"commit-unknown").unwrap();
        let mut recovered=Operation::load(&store,&id).unwrap();
        assert_eq!(recovered.phase,Phase::RestoreIntent);
        assert_eq!(recovered.restore_target.as_deref(),Some(target.as_str()));
        assert!(recovered.commit_unknown);
        for phase in [Phase::RestoreWriting,Phase::RestoreValidated,Phase::R1,Phase::R2,Phase::Restored,Phase::Done] {
            recovered.advance(&store,phase).unwrap();
        }
        assert!(recovered.resolved());
        assert!(!recovered.gc_eligible(false),"unknown SQL outcome must remain exempt from GC");
    }

    #[test]
    fn n1_gc_retains_current_previous_and_unresolved_payloads_after_age_and_count_limits() {
        use crate::publisher::store::{Origin,StoreManifest};
        use std::collections::BTreeMap;
        let temp=tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(temp.path(),std::fs::Permissions::from_mode(0o700)).unwrap();
        let store=PublisherStore::at(temp.path().into());
        let now=chrono::Utc::now();
        let current=uuid::Uuid::new_v4().to_string();
        let previous=uuid::Uuid::new_v4().to_string();
        let active=uuid::Uuid::new_v4().to_string();
        let garbage=uuid::Uuid::new_v4().to_string();
        let source=uuid::Uuid::new_v4().to_string();
        store.stage(&source,Origin::Publisher,None,BTreeMap::new(),BTreeMap::new()).unwrap();
        store.stage_derived(&current,&source,"retained current provenance",BTreeMap::new()).unwrap();
        for id in [&previous,&active,&garbage] {
            store.stage(id,Origin::Publisher,None,BTreeMap::new(),BTreeMap::new()).unwrap();
        }
        crate::db_upgrade::files::atomic_json(&store.root.join("manifest.json"),&StoreManifest {version:1,current:Some(current.clone()),previous:Some(previous.clone())}).unwrap();
        let active_op=uuid::Uuid::new_v4().to_string();
        let Admission::New(mut operation)=Operation::admit(&store,&active_op,&"a".repeat(64)).unwrap() else {panic!()};
        operation.next=Some(active.clone());operation.commit_unknown=true;operation.phase=Phase::CommitUnknown;
        operation.created_at=now-chrono::Duration::hours(1000);operation.save(&store).unwrap();
        for index in 0..102 {
            let Admission::New(mut old)=Operation::admit(&store,&uuid::Uuid::new_v4().to_string(),&"a".repeat(64)).unwrap() else {panic!()};
            old.phase=Phase::Done;old.created_at=now-chrono::Duration::hours(48);
            old.next=Some(match index {0=>current.clone(),1=>previous.clone(),_=>garbage.clone()});
            old.save(&store).unwrap();
        }
        collect(&store,now,true).unwrap();
        for id in [&current,&previous,&active,&source] {assert!(store.payload(id).is_ok());}
        assert!(!store.root.join("payloads").join(garbage).exists());
        assert!(Operation::load(&store,&active_op).is_ok());
        assert_eq!(operations(&store).unwrap().len(),1);
    }
}

fn path(store: &PublisherStore, id: &str) -> Result<PathBuf> {
    uuid(id)?;
    Ok(store.directory("ops")?.join(format!("{id}.json")))
}

pub(super) fn operations(store: &PublisherStore) -> Result<Vec<Operation>> {
    let mut operations=Vec::new();
    for entry in std::fs::read_dir(store.directory("ops")?)? {
        let entry=entry?;
        let name=entry.file_name();
        let Some(id)=name.to_str().and_then(|name|name.strip_suffix(".json")) else { continue };
        if uuid(id).is_err() { bail!("Invalid operation filename"); }
        operations.push(Operation::load(store,id)?);
    }
    operations.sort_by_key(|operation|operation.created_at);
    Ok(operations)
}

/// Caller holds the store lock; retention failure cannot replace an already completed result.
pub(super) fn collect_completed(store:&PublisherStore,a8_clear:bool){
    if let Err(error)=collect(store,chrono::Utc::now(),a8_clear){eprintln!("[publisher] journal/payload collection deferred: {error:#}");}
}

/// Caller holds the store lock; active records and unresolved uncertainty never expire.
pub(super) fn collect(store:&PublisherStore,now:chrono::DateTime<chrono::Utc>,a8_clear:bool) -> Result<()> {
    use std::collections::BTreeSet;
    let records=operations(store)?;
    let resolved:Vec<_>=records.iter().filter(|operation|operation.gc_eligible(a8_clear)).collect();
    let old_count=resolved.len().saturating_sub(100);
    let expired:BTreeSet<_>=resolved.iter().enumerate().filter(|(index,operation)|
        *index<old_count || operation.created_at<now-chrono::Duration::hours(24))
        .map(|(_,operation)|operation.id.as_str()).collect();
    let manifest=store.manifest()?;
    let mut protected:BTreeSet<String>=[manifest.current,manifest.previous].into_iter().flatten().collect();
    for operation in records.iter().filter(|operation|!expired.contains(operation.id.as_str())) {
        protected.extend([&operation.previous,&operation.next,&operation.restore_target].into_iter().flatten().cloned());
    }
    protect_sources(store,&mut protected)?;
    for operation in records.iter().filter(|operation|expired.contains(operation.id.as_str())) {
        for id in [&operation.previous,&operation.next,&operation.restore_target].into_iter().flatten() {
            if protected.contains(id) {continue;}
            remove_payload(store,id)?;
        }
        std::fs::remove_file(path(store,&operation.id)?)?;
        std::fs::File::open(store.directory("ops")?)?.sync_all()?;
    }
    Ok(())
}

pub(super) fn boot_digest(id:&str,manifest:&str)->Result<String> {
    uuid(id)?;Ok(super::store::digest(&serde_json::to_vec(&("BootPublish",id,manifest))?))
}

fn protect_sources(store:&PublisherStore,protected:&mut std::collections::BTreeSet<String>)->Result<()> {
    let mut pending:Vec<_>=protected.iter().cloned().collect();
    while let Some(id)=pending.pop() {
        if let Some(source)=store.payload(&id)?.manifest.derived_from {
            if protected.insert(source.clone()) {pending.push(source);}
        }
    }
    Ok(())
}

fn remove_payload(store:&PublisherStore,id:&str)->Result<()> {
    uuid(id)?;
    let directory=store.directory("payloads")?.join(id);
    match std::fs::symlink_metadata(&directory) {
        Ok(meta)=>{
            ensure!(meta.is_dir() && !meta.file_type().is_symlink(),"Unsafe expired payload");
            crate::db_upgrade::files::real_path(&directory)?;
            super::staging::cleanup_generation(&directory)?;
            std::fs::remove_dir_all(&directory)?;
            std::fs::File::open(store.directory("payloads")?)?.sync_all()?;
        }
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
        Err(error)=>return Err(error.into()),
    }
    Ok(())
}

/// Startup only, under store/apply locks after recovery and adoption close every operation.
pub(super) fn sweep_unreferenced(store:&PublisherStore)->Result<()> {
    use std::collections::BTreeSet;
    let manifest=store.manifest()?;
    let mut protected:BTreeSet<String>=[manifest.current,manifest.previous].into_iter().flatten().collect();
    for operation in operations(store)? {
        protected.extend([operation.previous,operation.next,operation.restore_target].into_iter().flatten());
    }
    protect_sources(store,&mut protected)?;
    for entry in std::fs::read_dir(store.directory("payloads")?)? {
        let entry=entry?;let name=entry.file_name();let Some(id)=name.to_str() else{continue;};
        if uuid(id).is_err() || protected.contains(id) {continue;}
        remove_payload(store,id)?;
    }
    Ok(())
}
