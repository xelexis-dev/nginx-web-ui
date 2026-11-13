use super::{Branch, Outcome, StepError, StorageLocks};
use serde::{Deserialize, Serialize};
use std::{path::{Path, PathBuf}, fs::{self, File, OpenOptions}, io::{self, Read, Write}};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use crate::db_upgrade::files;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum Step { P0, P0b, F1, Y1, Y2, Y3, P1Backup, P2Preflight, P3Convert, P4Target,
    S4Prepare, P5Publish, P5Guard, P5Commit, S4Activate, Handoff, Phase2Monitor, P6Cleanup,
    RRestore, CmdRestorePg15, CmdRemoveBackup, CmdRemoveDiscarded }
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum StepState { Intent, Done, Failed { code: String } }
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum MovePhase { SetAside, Restore, Target, Guard, Cleanup }
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MoveRecord { pub phase: MovePhase, pub entry: String, pub from: String, pub to: String, pub state: StepState }
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct JournalV1 {
    pub version: u8, pub attempt_id: String, pub branch: Branch, pub candidate: Candidate,
    pub records: Vec<Record>, pub moves: Vec<JournalMove>, pub handoff: Handoff,
    pub outcome: Outcome, pub retry: Retry,
    #[serde(default,skip_serializing_if="Vec::is_empty")]
    pub previous:Vec<PreviousAttempt>,
}
#[derive(Serialize, Deserialize, Clone, Debug,PartialEq,Eq)]
#[serde(rename_all = "camelCase")]
pub struct Candidate { pub revision: String, pub pins_sha256: String, pub migrations_sha256: String }
const BUILD_INPUTS:&str="/app/build-inputs.json";
const BUILD_INPUTS_MAX:u64=1024*1024;
fn build_identity_error()->StepError {StepError{code:"verification_failed",message:"이미지의 빌드 정보를 확인하지 못했습니다. 데이터 변경을 시작하지 않았습니다.",detail:Some("build_identity".into())}}
fn build_revision()->Result<String,StepError> {
    crate::nonroot::startup_value("APP_GIT_SHA").filter(|s|s.len()==40 && s.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b))).ok_or_else(build_identity_error)
}
/// Fixed image artifact, hashed as written; never substitute env/socket paths or reserialize JSON.
pub(crate) fn pins_sha256()->Result<String,StepError> {pins_from(Path::new(BUILD_INPUTS),&build_revision()?)}
fn pins_from(path:&Path,revision:&str)->Result<String,StepError> {
    use std::os::unix::fs::OpenOptionsExt;use sha2::{Digest,Sha256};
    if revision.len()!=40 || !revision.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b)){return Err(build_identity_error());}
    let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path).map_err(|_|build_identity_error())?;let metadata=file.metadata().map_err(|_|build_identity_error())?;
    if !metadata.is_file() || metadata.uid()!=0 || metadata.mode()&0o022!=0 || metadata.len()>BUILD_INPUTS_MAX{return Err(build_identity_error());}
    let mut bytes=Vec::new();file.take(BUILD_INPUTS_MAX+1).read_to_end(&mut bytes).map_err(|_|build_identity_error())?;
    if bytes.len()as u64>BUILD_INPUTS_MAX{return Err(build_identity_error());}
    let value:serde_json::Value=serde_json::from_slice(&bytes).map_err(|_|build_identity_error())?;
    if value.get("product").and_then(|v|v.get("revision")).and_then(|v|v.as_str())!=Some(revision){return Err(build_identity_error());}
    Ok(hex::encode(Sha256::digest(&bytes)))
}
impl Candidate {
    /// Called before mutation; the same startup path/default is used by RuntimeConfig later.
    pub(crate) async fn current()->Result<Self,StepError> {
        let (revision,pins_sha256)=tokio::task::spawn_blocking(||{let revision=build_revision()?;let pins=pins_sha256()?;if build_revision()?!=revision{return Err(build_identity_error());}Ok((revision,pins))}).await.map_err(|_|build_identity_error())??;
        let directory=PathBuf::from(crate::nonroot::startup_value("MIGRATIONS_DIR").unwrap_or_else(||"packages/db/drizzle".into()));
        let migrations=crate::migrations::read_migrations(&directory).await.map_err(|_|build_identity_error())?;
        Ok(Self{revision,pins_sha256,migrations_sha256:migrations_sha256(&migrations)})
    }
}
/// Input is read_migrations from the runner's exact applied MIGRATIONS_DIR, in journal order.
pub(crate) fn migrations_sha256(migrations:&[crate::migrations::Migration])->String {
    use sha2::{Digest,Sha256};
    let mut digest=Sha256::new();
    for migration in migrations {digest.update(format!("{}:{}\n",migration.timestamp,migration.hash));}
    hex::encode(digest.finalize())
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Record { pub seq: u64, pub step: Step, pub state: StepState, pub at: String, pub data: Option<serde_json::Value> }
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JournalMove { pub seq: u64, #[serde(flatten)] pub record: MoveRecord }

#[cfg(test)]
mod migrations_hash_tests {
    use super::migrations_sha256;
    #[test]
    fn w1_migrations_hash_matches_the_28_registered_legacy_signatures() {
        let profile=crate::db_upgrade::profiles::released().unwrap();
        let migrations=profile.signatures.into_iter().map(|row|crate::migrations::Migration{tag:row.tag,timestamp:row.created_at,hash:row.hash,statements:Vec::new()}).collect::<Vec<_>>();
        assert_eq!(migrations.len(),28);assert_eq!(migrations_sha256(&migrations),profile.sha256);
    }
    #[tokio::test]
    async fn w1_migrations_hash_tracks_raw_sql_timestamps_order_and_membership_but_not_tags() {
        use std::fs;
        for change in ["sql-byte","timestamp","order","added","removed","tag"] {
            let t=tempfile::tempdir().unwrap();fs::create_dir(t.path().join("meta")).unwrap();
            let path=t.path().join("meta/_journal.json");
            let mut journal=serde_json::json!({"version":"7","dialect":"postgresql","entries":[{"idx":0,"when":1000,"tag":"0000_first"},{"idx":1,"when":2000,"tag":"0001_second"}]});
            fs::write(&path,serde_json::to_vec(&journal).unwrap()).unwrap();
            fs::write(t.path().join("0000_first.sql"),b"SELECT 1;").unwrap();fs::write(t.path().join("0001_second.sql"),b"SELECT 3;").unwrap();
            let initial=crate::migrations::read_migrations(t.path()).await.unwrap();let before=migrations_sha256(&initial);
            match change {
                "sql-byte"=>fs::write(t.path().join("0000_first.sql"),b"SELECT 2;").unwrap(),
                "timestamp"=>journal["entries"][0]["when"]=serde_json::json!(1001),
                "added"=>{journal["entries"].as_array_mut().unwrap().push(serde_json::json!({"idx":2,"when":3000,"tag":"0002_added"}));fs::write(t.path().join("0002_added.sql"),b"SELECT 4;").unwrap();},
                "removed"=>{journal["entries"].as_array_mut().unwrap().pop();},
                "tag"=>{journal["entries"][0]["tag"]=serde_json::json!("0000_renamed");fs::rename(t.path().join("0000_first.sql"),t.path().join("0000_renamed.sql")).unwrap();},
                "order"=>{},_=>unreachable!(),
            }
            fs::write(&path,serde_json::to_vec(&journal).unwrap()).unwrap();let mut after=crate::migrations::read_migrations(t.path()).await.unwrap();
            if change=="order" {after.swap(0,1);}
            assert_eq!(migrations_sha256(&after)==before,change=="tag","{change}");
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Handoff { None, Intent, Done }
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Retry { pub consumed_labels: Vec<String>, pub interruptions: u64, pub backoff_until: Option<String> }

#[derive(Serialize,Deserialize,Clone,Debug,PartialEq,Eq)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct PreviousAttempt {
    pub attempt_id:String,pub outcome:Outcome,pub candidate:Candidate,
    #[serde(deserialize_with="Option::deserialize")]pub last_failure:Option<PreviousFailure>,
    #[serde(deserialize_with="Option::deserialize")]pub generation:Option<String>,
    #[serde(deserialize_with="Option::deserialize")]pub discard_id:Option<String>,
    pub ended_at:String,
}
#[derive(Serialize,Deserialize,Clone,Debug,PartialEq,Eq)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct PreviousFailure {pub step:Step,pub code:String}

fn command_step(step:Step)->bool {matches!(step,Step::CmdRestorePg15|Step::CmdRemoveBackup|Step::CmdRemoveDiscarded)}
pub(crate) fn commands_settled(records:&[Record])->bool {
    [Step::CmdRestorePg15,Step::CmdRemoveBackup,Step::CmdRemoveDiscarded].into_iter().all(|step|
        records.iter().rev().find(|r|r.step==step && r.state==StepState::Intent).is_none_or(|intent|
            records.iter().any(|r|r.step==step && r.seq>intent.seq && r.state==StepState::Done && r.data==intent.data)))
}
fn pre_pool_frame(j:&JournalV1)->bool{
    j.branch==Branch::Upgrade&&j.moves.is_empty()&&j.handoff==Handoff::None&&j.records.iter().all(|r|{
        if matches!(r.step,Step::P0|Step::P0b){return true;}
        if r.step!=Step::P5Publish||r.state!=StepState::Done{return false;}
        let Some(data)=r.data.as_ref()else{return false;};
        let Some(identity)=data.get("bootIdentity").and_then(|v|v.as_str())else{return false;};
        let Some(value)=data.get("outcome")else{return false;};let Ok(outcome)=serde_json::from_value::<Outcome>(value.clone())else{return false;};
        super::refusal::receipt(outcome,identity).is_ok_and(|expected|expected==*data)
    })
}
fn failure_outcome(outcome:Outcome)->bool {
    matches!(outcome,Outcome::RefusedPreflight|Outcome::RefusedFresh|Outcome::RefusedPrivilege|Outcome::RestoreIncomplete|Outcome::ActivationFailed)
}
pub(crate) fn last_attempt_failure(records:&[Record])->Option<PreviousFailure> {
    records.iter().rev().filter(|r|!command_step(r.step)).find_map(|r|if let StepState::Failed{code}=&r.state{Some(PreviousFailure{step:r.step,code:code.clone()})}else{None})
}
/// Moves are append-only: an old Intent is settled by the latest Done of that exact tuple.
pub(crate) fn moves_done(moves:&[JournalMove])->bool {
    let mut seen=std::collections::HashSet::new();
    moves.iter().rev().all(|m|!seen.insert((m.record.phase,m.record.entry.as_str(),m.record.from.as_str(),m.record.to.as_str())) || m.record.state==StepState::Done)
}

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(crate) enum WorkDirState {None,Intent,Done}
/// One workDir removal per restore-command cycle; command Done/Failed never resets it.
fn work_dir_cycle(records:&[Record],attempt_id:&str)->Result<WorkDirState,StepError> {
    let mut state=WorkDirState::None;
    for record in records {
        if record.step==Step::CmdRestorePg15 && record.state==StepState::Intent{state=WorkDirState::None;}
        if record.step!=Step::P6Cleanup{continue;}
        let Some(value)=record.data.as_ref().filter(|v|v.get("workDir").is_some())else{continue};
        if !value.as_object().is_some_and(|o|o.len()==1 && o.get("workDir").and_then(|v|v.as_str())==Some(attempt_id)){return Err(invalid());}
        state=match (&record.state,state){
            (StepState::Intent,WorkDirState::None|WorkDirState::Intent)=>WorkDirState::Intent,
            (StepState::Done,WorkDirState::Intent)=>WorkDirState::Done,
            _=>return Err(invalid()),
        };
    }Ok(state)
}
/// Unlike workDir, a staged key is never recreated by an operator restore cycle.
fn staged_key_records(records:&[Record])->Result<(),StepError>{
    let mut state=None;
    for record in records.iter().filter(|r|r.step==Step::P6Cleanup){
        let Some(data)=record.data.as_ref().filter(|v|v.get("stagedKey").is_some())else{continue};
        let generation=data.as_object().filter(|o|o.len()==1).and_then(|o|o.get("stagedKey")).and_then(|v|v.as_str()).filter(|g|super::types::canonical_generation(g)).ok_or_else(invalid)?;
        let commit=records.iter().rev().find(|r|r.seq<record.seq && r.step==Step::P5Commit && r.state==StepState::Intent).ok_or_else(invalid)?;
        if commit.data.as_ref().and_then(|d|d.get("generation")).and_then(|v|v.as_str())!=Some(generation)
            || !records.iter().any(|r|r.step==Step::P5Commit && r.state==StepState::Done && r.seq>commit.seq && r.seq<record.seq){return Err(invalid());}
        state=Some(match (state,&record.state){(None,StepState::Intent)=>(generation,false),(Some((previous,false)),StepState::Done)if previous==generation=>(generation,true),_=>return Err(invalid())});
    }Ok(())
}
pub struct Journal { root: PathBuf, state: JournalV1 }
/// In-memory only: old N2 records were discarded in this run and have not changed since.
pub(crate) struct RolloverDiscarded {attempt_id:String,last_seq:u64}
/// Frozen command fields; the private binding is in-memory only, never journal data.
#[derive(Clone,Debug,Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct RestorePg15Data {
    backup_id:String,generation:String,discard_id:String,
    #[serde(skip)] selector_sha256:String,
}
impl RestorePg15Data {
    /// Bind the intended canonical selector BEFORE publishing it; never bind a reread file.
    pub(crate) fn bind(backup_id:&str,generation:&str,discard_id:&str,selector:&super::selector::Selector)->Result<Self,StepError> {
        selector.validate()?;
        if !super::types::canonical_generation(backup_id) || !super::types::canonical_generation(generation) || generation!=discard_id
            || selector.active_major!=15 || selector.origin!=super::selector::Origin::Auto || selector.backup_id.as_deref()!=Some(backup_id)
            || selector.generation.is_some() || selector.operator_reason.as_deref()!=Some("restore-pg15"){return Err(invalid());}
        let bytes=serde_json::to_vec(selector).map_err(|_|invalid())?;
        Ok(Self{backup_id:backup_id.into(),generation:generation.into(),discard_id:discard_id.into(),selector_sha256:super::selector::digest(&bytes)})
    }
}
fn unsupported() -> StepError { StepError { code: "unsupported_layout", message: "지원하지 않는 업그레이드 관리 경로입니다. 기존 내용을 보존했습니다.", detail: None } }
fn invalid() -> StepError { StepError { code: "pair_mismatch", message: "업그레이드 복구 기록을 확인하지 못했습니다.", detail: None } }
fn write_failed() -> StepError { StepError { code: "journal_write_failed", message: "업그레이드 복구 기록을 저장하지 못했습니다. 현재 데이터를 보존했습니다.", detail: None } }
fn disk_full(error: &io::Error) -> bool { matches!(error.raw_os_error(), Some(libc::ENOSPC | libc::EDQUOT)) }
fn sync_dir(path: &Path) -> io::Result<()> { File::open(path)?.sync_all() }
fn checked_file(path: &Path, write: bool) -> io::Result<File> {
    files::real_path(path)?;
    let file = OpenOptions::new().read(true).write(write).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
    let m = file.metadata()?;
    if !m.is_file() || m.uid()!=0 || m.mode()&0o7777!=0o600 || m.nlink()!=1 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "unsafe journal slot"));
    }
    Ok(file)
}
impl JournalV1 {
    fn validate(&self) -> Result<(), StepError> {
        use std::collections::BTreeSet;
        let hash = |v: &str, len| v.len()==len && v.bytes().all(|b|b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
        if self.version!=1 || !super::types::canonical_generation(&self.attempt_id)
            || !hash(&self.candidate.revision,40) || !hash(&self.candidate.pins_sha256,64) || !hash(&self.candidate.migrations_sha256,64) { return Err(invalid()); }
        let mut attempts=BTreeSet::from([self.attempt_id.as_str()]);
        if self.previous.iter().filter(|p|p.discard_id.is_none()).count()>8 {return Err(invalid());}
        for p in &self.previous {
            if !super::types::canonical_generation(&p.attempt_id) || !attempts.insert(p.attempt_id.as_str())
                || !hash(&p.candidate.revision,40) || !hash(&p.candidate.pins_sha256,64) || !hash(&p.candidate.migrations_sha256,64)
                || chrono::DateTime::parse_from_rfc3339(&p.ended_at).is_err()
                || [&p.generation,&p.discard_id].iter().any(|id|id.as_deref().is_some_and(|id|!super::types::canonical_generation(id)))
                || p.last_failure.as_ref().is_some_and(|f|command_step(f.step) || f.code.is_empty() || f.code.len()>128 || !f.code.bytes().all(|b|b.is_ascii_lowercase() || b.is_ascii_digit() || b==b'_')) {return Err(invalid());}
        }
        let mut seqs=BTreeSet::new();
        for r in &self.records {
            if r.seq==0 || !seqs.insert(r.seq) || chrono::DateTime::parse_from_rfc3339(&r.at).is_err()
                || r.data.as_ref().is_some_and(|v|serde_json::to_vec(v).map_or(true,|b|b.len()>65536)) { return Err(invalid()); }
        }
        for m in &self.moves {
            if m.seq==0 || !seqs.insert(m.seq) || !safe_move(&m.record) { return Err(invalid()); }
        }
        if self.records.windows(2).any(|v|v[0].seq>=v[1].seq) || self.moves.windows(2).any(|v|v[0].seq>=v[1].seq)
            || self.retry.consumed_labels.iter().any(|s|s.is_empty() || s.len()>128 || s.contains('\0'))
            || self.retry.backoff_until.as_ref().is_some_and(|s|chrono::DateTime::parse_from_rfc3339(s).is_err()) { return Err(invalid()); }
        work_dir_cycle(&self.records,&self.attempt_id)?;
        staged_key_records(&self.records)?;
        Ok(())
    }
}
fn safe_move(m: &MoveRecord) -> bool {
    !m.entry.is_empty() && m.entry.len()<=255 && m.entry!="." && m.entry!=".." && !m.entry.contains('/') && !m.entry.contains('\0')
        && ["L","BK","W","D17"].contains(&m.from.as_str()) && ["L","BK","W","D17"].contains(&m.to.as_str())
}
/// Read-only diagnostic snapshot: no storage lock, because the runner owns it for life.
pub(crate) fn read_only(a_root:&Path)->Result<JournalV1,StepError> {
    let root=a_root.join(".nwui-auto");files::real_path(&root).map_err(|_|invalid())?;
    let m=fs::symlink_metadata(&root).map_err(|_|invalid())?;
    if !m.is_dir() || m.uid()!=0 || m.mode()&0o7777!=0o700 {return Err(invalid());}
    let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(root.join("journal-v1.json")).map_err(|_|invalid())?;
    read_snapshot(file)
}
fn read_snapshot(file:File)->Result<JournalV1,StepError> {
    let m=file.metadata().map_err(|_|invalid())?;
    // An atomic replacement can unlink this already-open, otherwise valid read snapshot.
    if !m.is_file() || m.uid()!=0 || m.mode()&0o7777!=0o600 || m.nlink()>1 || m.len()>1_048_576 {return Err(invalid());}
    let mut bytes=Vec::new();file.take(1_048_577).read_to_end(&mut bytes).map_err(|_|invalid())?;
    if bytes.len()>1_048_576 {return Err(invalid());}
    let value:JournalV1=serde_json::from_slice(&bytes).map_err(|_|invalid())?;value.validate()?;Ok(value)
}
/// Before source classification: remove only the approved fixed bootstrap temp.
/// True means the existing, now-empty authority directory may be ignored by that classifier.
pub(crate) fn prepare_bootstrap(a_root: &Path, locks: &StorageLocks) -> Result<bool, StepError> {
        if !locks.holds_d() || !locks.matches_app(a_root) { return Err(invalid()); }
        for name in [c".nwui-auto-work", c".nwui-pg15-backup", c".nwui-discard"] {
            if locks.db_entry_exists(name).map_err(|_|unsupported())? { return Err(unsupported()); }
        }
        match fs::symlink_metadata(a_root.join(".nwui-publisher")) {
            Ok(_)=>return Err(unsupported()),
            Err(e) if e.kind()==io::ErrorKind::NotFound=>{},
            Err(_)=>return Err(unsupported()),
        }
        let root=a_root.join(".nwui-auto");
        match fs::symlink_metadata(&root) {
            Ok(m)=>{
                if !m.is_dir() || m.file_type().is_symlink() || m.uid()!=0 || m.mode()&0o7777!=0o700 { return Err(unsupported()); }
                let entries=fs::read_dir(&root).map_err(|_|unsupported())?.map(|r|r.map(|e|e.file_name())).collect::<io::Result<Vec<_>>>().map_err(|_|unsupported())?;
                if entries.len()>1 || entries.first().is_some_and(|name|name!=".journal-v1.json.init") { return Err(unsupported()); }
                if !entries.is_empty() {
                    let tmp=root.join(".journal-v1.json.init"); let m=fs::symlink_metadata(&tmp).map_err(|_|unsupported())?;
                    if !m.is_file() || m.file_type().is_symlink() || m.uid()!=0 { return Err(unsupported()); }
                    fs::remove_file(&tmp).map_err(|_|write_failed())?; sync_dir(&root).map_err(|_|write_failed())?;
                }
            },
            Err(e) if e.kind()==io::ErrorKind::NotFound=>return Ok(false),
            Err(_)=>return Err(unsupported()),
        }
        Ok(true)
}
impl Journal {
    pub fn open(a_root: &Path, locks: &StorageLocks) -> Result<Self, StepError> {
        if !locks.matches_app(a_root) { return Err(invalid()); }
        let root=a_root.join(".nwui-auto");
        let state: JournalV1=files::read_json(&root.join("journal-v1.json")).map_err(|_|invalid())?;
        state.validate()?;
        Ok(Self { root, state })
    }
    pub(crate) fn begin(a_root: &Path, locks: &StorageLocks, branch: Branch, candidate: Candidate) -> Result<Self, StepError> {
        Self::begin_with_attempt(a_root,locks,branch,candidate,uuid::Uuid::new_v4().to_string())
    }
    pub(crate) fn begin_with_attempt(a_root:&Path,locks:&StorageLocks,branch:Branch,candidate:Candidate,attempt_id:String)->Result<Self,StepError> {
        Self::begin_labeled(a_root,locks,branch,candidate,attempt_id,None)
    }
    /// Consume the initial upgrade label in the first durable frame, never in a second write.
    pub(crate) fn begin_upgrade(a_root:&Path,locks:&StorageLocks,candidate:Candidate,label:Option<&str>)->Result<Self,StepError>{
        Self::begin_labeled(a_root,locks,Branch::Upgrade,candidate,uuid::Uuid::new_v4().to_string(),label)
    }
    fn begin_labeled(a_root:&Path,locks:&StorageLocks,branch:Branch,candidate:Candidate,attempt_id:String,label:Option<&str>)->Result<Self,StepError>{
        if !super::types::canonical_generation(&attempt_id) {return Err(invalid());}
        let state=JournalV1 { version:1, attempt_id, branch, candidate,
            records:if branch==Branch::Upgrade { vec![
                Record { seq:1, step:Step::P0, state:StepState::Done, at:chrono::Utc::now().to_rfc3339(), data:None },
                Record { seq:2, step:Step::P0b, state:StepState::Intent, at:chrono::Utc::now().to_rfc3339(), data:None },
            ] } else { Vec::new() }, moves:Vec::new(), handoff:Handoff::None, outcome:Outcome::Upgrading,
            retry:Retry { consumed_labels:label.into_iter().map(str::to_owned).collect(), interruptions:0, backoff_until:None },previous:Vec::new() };
        state.validate()?;
        let root=a_root.join(".nwui-auto");
        if !prepare_bootstrap(a_root, locks)? { files::private_dir(&root).map_err(|_|write_failed())?; }
        let journal=Self { root, state };
        journal.publish_init(&journal.state)?;
        Ok(journal)
    }
    pub fn load(&self) -> &JournalV1 { &self.state }
    pub(crate) fn work_dir_state(&self,attempt_id:&str)->WorkDirState {
        if attempt_id!=self.state.attempt_id{return WorkDirState::None;}
        work_dir_cycle(&self.state.records,attempt_id).expect("validated work-dir cycle")
    }
    pub(crate) fn belongs_to(&self,a_root:&Path)->bool {self.root==a_root.join(".nwui-auto")}
    pub fn key_source(&self) -> Result<super::KeySource, StepError> { super::key_write::source(self) }
    fn next_seq(&self) -> Result<u64, StepError> {
        self.state.records.iter().map(|r|r.seq).chain(self.state.moves.iter().map(|m|m.seq)).max().unwrap_or(0).checked_add(1).ok_or_else(invalid)
    }
    pub fn record(&mut self, step: Step, state: StepState, data: Option<serde_json::Value>) -> Result<(), StepError> {
        let mut next=self.state.clone();
        next.records.push(Record { seq:self.next_seq()?, step, state, at:chrono::Utc::now().to_rfc3339(), data });
        self.commit(next)
    }
    /// Atomic handoff state + record: a crash never exposes one without the other.
    pub(crate) fn handoff(&mut self,state:Handoff,boot_identity:&str)->Result<(),StepError> {
        if self.state.outcome!=Outcome::Pg17 || boot_identity.len()!=64 || !boot_identity.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) || state==Handoff::None {return Err(invalid());}
        let previous=self.state.records.iter().rev().find(|r|r.step==Step::Handoff);
        if state==Handoff::Done && !previous.is_some_and(|r|r.state==StepState::Intent && r.data.as_ref().and_then(|v|v.get("bootIdentity")).and_then(|v|v.as_str())==Some(boot_identity)) {return Err(invalid());}
        let mut next=self.state.clone();next.handoff=state;
        next.records.push(Record {seq:self.next_seq()?,step:Step::Handoff,state:if state==Handoff::Intent{StepState::Intent}else{StepState::Done},at:chrono::Utc::now().to_rfc3339(),data:Some(serde_json::json!({"bootIdentity":boot_identity}))});
        self.commit(next)
    }
    /// The rollover owner completes admission, settled-path checks and N2 discard first.
    /// This is one slot-pool commit; no archived journal or second authority file exists.
    pub(crate) async fn discard_for_rollover(&mut self,roots:&crate::db_upgrade::layout::Roots,locks:&StorageLocks)->Result<RolloverDiscarded,StepError> {
        if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !self.belongs_to(&roots.app)
            || matches!(self.state.outcome,Outcome::Upgrading|Outcome::Pg17|Outcome::ActivationFailed|Outcome::RestoreIncomplete)
            || !moves_done(&self.state.moves) || !commands_settled(&self.state.records) {return Err(invalid());}
        let prepared=crate::nonroot::prepared_from_journal(self).unwrap_or(crate::nonroot::Prepared{payload:None,key_receipt:None});
        crate::nonroot::discard(prepared,self).await?;
        Ok(RolloverDiscarded{attempt_id:self.state.attempt_id.clone(),last_seq:self.next_seq()?.saturating_sub(1)})
    }
    pub(crate) fn replace_attempt_locked(&mut self,roots:&crate::db_upgrade::layout::Roots,locks:&StorageLocks,next:JournalV1,discarded:RolloverDiscarded)->Result<(),StepError> {
        if discarded.attempt_id!=self.state.attempt_id || discarded.last_seq!=self.next_seq()?.saturating_sub(1)
            || !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !self.belongs_to(&roots.app) || self.state.branch!=Branch::Upgrade
            || matches!(self.state.outcome,Outcome::Upgrading|Outcome::Pg17|Outcome::ActivationFailed|Outcome::RestoreIncomplete)
            || !moves_done(&self.state.moves) || !commands_settled(&self.state.records)
            || next.version!=1 || next.attempt_id==self.state.attempt_id || !super::types::canonical_generation(&next.attempt_id)
            || uuid::Uuid::parse_str(&next.attempt_id).is_err()
            || uuid::Uuid::parse_str(&next.attempt_id).is_ok_and(|v|v.get_version()!=Some(uuid::Version::Random))
            || next.branch!=Branch::Upgrade || next.outcome!=Outcome::Upgrading || next.handoff!=Handoff::None || !next.moves.is_empty()
            || next.records.len()!=2 || next.records[0].seq!=1 || next.records[0].step!=Step::P0 || next.records[0].state!=StepState::Done || next.records[0].data.is_some()
            || next.records[1].seq!=2 || next.records[1].step!=Step::P0b || next.records[1].state!=StepState::Intent || next.records[1].data.is_some()
            || next.previous.first().is_none_or(|p|p.attempt_id!=self.state.attempt_id || p.outcome!=self.state.outcome || p.candidate!=self.state.candidate)
            || next.previous[0].last_failure!=last_attempt_failure(&self.state.records)
            || next.retry.interruptions!=self.state.retry.interruptions || next.retry.backoff_until!=self.state.retry.backoff_until
            || !next.retry.consumed_labels.starts_with(&self.state.retry.consumed_labels) || next.retry.consumed_labels.len()>self.state.retry.consumed_labels.len()+1
            || next.retry.consumed_labels.len()>self.state.retry.consumed_labels.len() && next.retry.consumed_labels.last().is_some_and(|label|self.state.retry.consumed_labels.contains(label))
            || self.state.previous.iter().filter(|p|p.discard_id.is_some()).any(|p|!next.previous.contains(p)) {return Err(invalid());}
        self.commit(next)
    }
    /// A.25(g): finish only the fully restored, hash-bound offline command in one frame.
    pub(crate) fn finish_restore_pg15(&mut self,roots:&crate::db_upgrade::layout::Roots,locks:&StorageLocks,data:&RestorePg15Data)->Result<(),StepError> {
        use std::os::fd::AsRawFd;
        if !locks.matches_roots(&roots.db,&roots.app) || !self.belongs_to(&roots.app) || self.state.branch!=Branch::Upgrade
            || !matches!(self.state.outcome,Outcome::Pg17|Outcome::ActivationFailed) || data.backup_id!=self.state.attempt_id {return Err(invalid());}
        let current=Self::open(&roots.app,locks)?;
        if serde_json::to_value(current.load()).map_err(|_|invalid())?!=serde_json::to_value(self.load()).map_err(|_|invalid())? {return Err(invalid());}
        let value=serde_json::to_value(data).map_err(|_|invalid())?;
        let latest=self.state.records.iter().rev().find(|r|r.step==Step::CmdRestorePg15).ok_or_else(invalid)?;
        if latest.state!=StepState::Intent || latest.data.as_ref()!=Some(&value){return Err(invalid());}
        let (selected,bytes):(super::selector::Selector,_)=super::selector::read_record(&roots.app.join(".nwui-auto/active-v1.json"))?;selected.validate()?;
        if selected.active_major!=15 || selected.origin!=super::selector::Origin::Auto || selected.backup_id.as_deref()!=Some(data.backup_id.as_str())
            || super::selector::digest(&bytes)!=data.selector_sha256 {return Err(invalid());}
        fn absent(parent:&File,name:&std::ffi::CStr)->Result<(),StepError> {
            let mut metadata=std::mem::MaybeUninit::<libc::stat>::uninit();
            if unsafe{libc::fstatat(parent.as_raw_fd(),name.as_ptr(),metadata.as_mut_ptr(),libc::AT_SYMLINK_NOFOLLOW)}==0{return Err(invalid());}
            if io::Error::last_os_error().kind()!=io::ErrorKind::NotFound{return Err(invalid());}Ok(())
        }
        let app=super::target::directory(&roots.app)?;absent(&app,c".nwui")?;
        for name in [".nwui-pg15-backup",".nwui-auto-work"] {
            let path=roots.db.join(name);
            match fs::symlink_metadata(&path) {
                Err(e)if e.kind()==io::ErrorKind::NotFound=>{},Err(_)=>return Err(invalid()),Ok(_)=>{
                    let parent=super::target::directory(&path).map_err(|_|invalid())?;
                    if parent.metadata().map_err(|_|invalid())?.dev()!=fs::symlink_metadata(&roots.db).map_err(|_|invalid())?.dev(){return Err(invalid());}
                    let id=std::ffi::CString::new(data.backup_id.as_str()).map_err(|_|invalid())?;absent(&parent,&id)?;
                },
            }
        }
        let mut next=self.state.clone();next.records.push(Record{seq:self.next_seq()?,step:Step::CmdRestorePg15,state:StepState::Done,at:chrono::Utc::now().to_rfc3339(),data:Some(value)});next.outcome=Outcome::Restored;next.handoff=Handoff::None;
        if super::rollover::retention(&next).map_err(|_|invalid())?!=(Some(data.generation.clone()),Some(data.discard_id.clone())){return Err(invalid());}
        self.commit(next)
    }
    /// Caller holds A+D and has removed/fsynced both named trees. One frame retires authority.
    pub(crate) fn finish_remove_discarded(&mut self,discard_id:&str)->Result<(),StepError> {
        if !super::types::canonical_generation(discard_id){return Err(invalid());}
        let data=serde_json::json!({"discardId":discard_id});let latest=self.state.records.iter().rev().find(|r|r.step==Step::CmdRemoveDiscarded && matches!(r.state,StepState::Intent|StepState::Done)).ok_or_else(invalid)?;
        if latest.data.as_ref()!=Some(&data){return Err(invalid());}
        let retained=self.state.previous.iter().filter(|p|p.discard_id.as_deref()==Some(discard_id)).count();
        if latest.state==StepState::Done {return if self.state.previous.iter().any(|p|p.discard_id.as_deref()==Some(discard_id) || p.generation.as_deref()==Some(discard_id)){Err(invalid())}else{Ok(())};}
        let current=self.state.records.iter().rev().find(|r|r.step==Step::CmdRestorePg15 && r.state==StepState::Done).and_then(|r|r.data.as_ref()).is_some_and(|d|d.get("discardId").and_then(|v|v.as_str())==Some(discard_id) && d.get("generation").and_then(|v|v.as_str())==Some(discard_id));
        if retained>1 || retained==0 && !current{return Err(invalid());}
        let mut next=self.state.clone();for previous in &mut next.previous {if previous.discard_id.as_deref()==Some(discard_id){previous.discard_id=None;previous.generation=None;}}
        next.records.push(Record{seq:self.next_seq()?,step:Step::CmdRemoveDiscarded,state:StepState::Done,at:chrono::Utc::now().to_rfc3339(),data:Some(data)});self.commit(next)
    }
    fn retry_failure_frame(&self,step:Step,code:&str,now:chrono::DateTime<chrono::Utc>)->Result<JournalV1,StepError>{
        let mut next=self.state.clone();
        next.retry.interruptions=if code=="interrupted" {next.retry.interruptions.checked_add(1).ok_or_else(invalid)?}else{0};
        next.retry.backoff_until=if next.retry.interruptions>=2 {Some((now+chrono::Duration::hours(24)).to_rfc3339())}else{None};
        next.records.push(Record{seq:self.next_seq()?,step,state:StepState::Failed{code:code.into()},at:now.to_rfc3339(),data:None});Ok(next)
    }
    pub(crate) fn record_retry_failure(&mut self,step:Step,code:&str,now:chrono::DateTime<chrono::Utc>)->Result<(),StepError> {
        let next=self.retry_failure_frame(step,code,now)?;self.commit(next)
    }
    fn restore_receipt(next:&mut JournalV1,identity:&str)->Result<(),StepError>{
        let data=super::refusal::receipt(next.outcome,identity)?;
        let seq=next.records.iter().map(|r|r.seq).chain(next.moves.iter().map(|m|m.seq)).max().unwrap_or(0).checked_add(1).ok_or_else(invalid)?;
        next.records.push(Record{seq,step:Step::P5Publish,state:StepState::Done,at:chrono::Utc::now().to_rfc3339(),data:Some(data)});Ok(())
    }
    pub fn record_move(&mut self, m: MoveRecord) -> Result<(), StepError> {
        if !safe_move(&m) { return Err(invalid()); }
        let mut next=self.state.clone(); next.moves.push(JournalMove { seq:self.next_seq()?, record:m }); self.commit(next)
    }
    pub fn set_outcome(&mut self, o: Outcome) -> Result<(), StepError> {
        if failure_outcome(o) && last_attempt_failure(&self.state.records).is_none() {return Err(invalid());}
        let mut next=self.state.clone(); next.outcome=o; self.commit(next)
    }
    /// Persist the failing step first; a crash can retain Upgrading, never an unexplained refusal.
    pub(crate) fn restore_cleanup_ready(&self)->Result<(),StepError> {
        let event=|name:&str,field:&str|self.state.records.iter().filter(|r|r.step==Step::RRestore && r.state==StepState::Done && r.data.as_ref().is_some_and(|d|d.get("recoveryEvent").and_then(|v|v.as_str())==Some(name) && d.get(field).and_then(|v|v.as_str())==Some(self.state.attempt_id.as_str()))).map(|r|r.seq).max();
        if self.state.branch!=Branch::Upgrade || self.state.handoff!=Handoff::None || !matches!(self.state.outcome,Outcome::Upgrading|Outcome::RestoreIncomplete|Outcome::Restored)
            || last_attempt_failure(&self.state.records).is_none() || !moves_done(&self.state.moves) || !commands_settled(&self.state.records)
            || self.state.records.iter().any(|r|r.step==Step::P5Commit && r.state==StepState::Done && r.data.is_none())
            || event("entries-verified","backupId").is_none() || event("key-discard-done","attemptId").is_none()
            || self.state.records.iter().filter(|r|r.step==Step::S4Prepare).any(|r|r.seq>=event("entries-verified","backupId").unwrap_or(0) || r.seq>=event("key-discard-done","attemptId").unwrap_or(0))
            || !self.state.records.iter().any(|r|r.step==Step::RRestore && r.state==StepState::Intent && r.data.as_ref().is_some_and(|d|d.get("recoveryEvent").and_then(|v|v.as_str())==Some("restore-marker-intent") && d.get("backupId").and_then(|v|v.as_str())==Some(self.state.attempt_id.as_str()))) {return Err(invalid());}Ok(())
    }
    pub(crate) fn restore_completion_ready(&self)->Result<(),StepError> {
        self.restore_cleanup_ready()?;
        if !self.state.records.iter().any(|r|r.step==Step::RRestore && r.state==StepState::Done && r.data.as_ref().is_some_and(|d|d.get("recoveryEvent").and_then(|v|v.as_str())==Some("work-discarded") && d.get("attemptId").and_then(|v|v.as_str())==Some(self.state.attempt_id.as_str()))) {return Err(invalid());}Ok(())
    }
    /// Caller has verified restored bytes and fsynced the last marker removal under A+D.
    /// One frame prevents a crash from asking the now-consumed backup to restore again.
    pub(crate) fn complete_precommit_restore(&mut self,identity:Option<&str>)->Result<(),StepError> {
        let identity=identity.ok_or_else(||StepError{code:"verification_failed",message:"이 실행의 식별자를 확인하지 못해 시작 알림을 보류했습니다.",detail:Some("boot_identity".into())})?;
        self.restore_completion_ready()?;
        if !self.state.records.iter().any(|r|r.step==Step::RRestore && r.state==StepState::Intent && r.data.as_ref().is_some_and(|d|d.get("recoveryEvent").and_then(|v|v.as_str())==Some("restore-finish-intent") && d.get("backupId").and_then(|v|v.as_str())==Some(self.state.attempt_id.as_str()))) {return Err(invalid());}
        if self.state.outcome==Outcome::Restored {return if self.state.records.iter().rev().find(|r|r.step==Step::RRestore).is_some_and(|r|r.state==StepState::Done && r.data.is_none()) && super::refusal::outcome(&self.state,identity)==Some(Outcome::Restored){Ok(())}else{Err(invalid())};}
        let mut next=self.state.clone();next.records.push(Record{seq:self.next_seq()?,step:Step::RRestore,state:StepState::Done,at:chrono::Utc::now().to_rfc3339(),data:None});next.outcome=Outcome::Restored;Self::restore_receipt(&mut next,identity)?;self.commit(next)
    }
    pub(crate) fn fail_attempt(&mut self,step:Step,error:&StepError,outcome:Outcome,now:chrono::DateTime<chrono::Utc>)->Result<(),StepError> {
        self.fail_attempt_with_identity(step,error,outcome,now,None)
    }
    pub(crate) fn fail_attempt_with_identity(&mut self,step:Step,error:&StepError,outcome:Outcome,now:chrono::DateTime<chrono::Utc>,identity:Option<&str>)->Result<(),StepError>{
        if command_step(step) || !failure_outcome(outcome) {return Err(invalid());}
        if outcome==Outcome::RestoreIncomplete && self.state.outcome==outcome && identity.is_some_and(|id|super::refusal::outcome(&self.state,id)==Some(outcome)){
            return if last_attempt_failure(&self.state.records).is_some_and(|f|f.step==step && f.code==error.code){Ok(())}else{Err(invalid())};
        }
        let mut next=self.retry_failure_frame(step,error.code,now)?;
        if outcome==Outcome::RestoreIncomplete && identity.is_none(){
            self.commit(next)?;
            return Err(StepError{code:"verification_failed",message:"이 실행의 식별자를 확인하지 못해 시작 알림을 보류했습니다.",detail:Some("boot_identity".into())});
        }
        next.outcome=outcome;
        if outcome==Outcome::RestoreIncomplete||identity.is_some()&&super::refusal::final_outcome(outcome){Self::restore_receipt(&mut next,identity.ok_or_else(invalid)?)?;}
        self.commit(next)
    }
    fn commit(&mut self, next: JournalV1) -> Result<(), StepError> {
        next.validate()?;let pre_pool=self.pre_pool()?;
        if pre_pool&&!pre_pool_frame(&next){return Err(invalid());}
        self.persist(&next)?; self.state=next;
        if self.state.branch==Branch::Upgrade&&!pre_pool { self.refill_slots()?; }
        Ok(())
    }
    pub(crate) fn app_reserves(&self)->Result<(u64,u8),StepError> {
        let slots=self.root.join("slots");
        if fs::symlink_metadata(&slots).is_ok() {
            files::real_path(&slots).map_err(|_|invalid())?;let m=fs::symlink_metadata(&slots).map_err(|_|invalid())?;
            if !m.is_dir() || m.uid()!=0 || m.mode()&0o7777!=0o700{return Err(invalid());}
            for e in fs::read_dir(&slots).map_err(|_|invalid())? {
                let e=e.map_err(|_|invalid())?;let name=e.file_name();let name=name.to_str().ok_or_else(invalid)?;
                if !(0..16).any(|n|name==n.to_string()){return Err(invalid());}let f=checked_file(&e.path(),false).map_err(|_|invalid())?;
                if f.metadata().map_err(|_|invalid())?.len()>1_048_576{return Err(invalid());}
            }
        }
        let pool=self.root.join("reserve");let mut bytes=0;let mut count=0;
        match fs::symlink_metadata(&pool) {Err(e)if e.kind()==io::ErrorKind::NotFound=>return Ok((0,0)),Err(_)=>return Err(invalid()),Ok(_)=>{}}
        files::real_path(&pool).map_err(|_|invalid())?;
        let m=fs::symlink_metadata(&pool).map_err(|_|invalid())?;
        if !m.is_dir() || m.uid()!=0 || m.mode()&0o7777!=0o700{return Err(invalid());}
        for e in fs::read_dir(&pool).map_err(|_|invalid())? {
            let e=e.map_err(|_|invalid())?;let name=e.file_name();
            if !name.to_str().is_some_and(|s|s.len()==1 && matches!(s.as_bytes()[0],b'0'..=b'7')){return Err(invalid());}
            let f=checked_file(&e.path(),false).map_err(|_|invalid())?;let m=f.metadata().map_err(|_|invalid())?;
            if m.len()>2*1024*1024{return Err(invalid());}bytes+=m.len();count+=1;
        }Ok((bytes,count))
    }
    pub(crate) fn release_app_reserve(&self)->Result<bool,StepError> {self.release_reserve().map_err(|_|invalid())}
    fn release_reserve(&self) -> io::Result<bool> {
        use std::os::fd::AsRawFd;
        if self.app_reserves().map_err(|_|io::Error::new(io::ErrorKind::InvalidData,"unsafe app reserve"))?.1==0{return Ok(false);}
        let pool=self.root.join("reserve");let dir=OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC).open(&pool)?;
        for n in 0..8 {
            match checked_file(&pool.join(n.to_string()),false) {
                Ok(_) => {let name=std::ffi::CString::new(n.to_string()).unwrap();if unsafe{libc::unlinkat(dir.as_raw_fd(),name.as_ptr(),0)}!=0{return Err(io::Error::last_os_error());}dir.sync_all()?;return Ok(true);},
                Err(e) if e.kind()==io::ErrorKind::NotFound => {},
                Err(e) => return Err(e),
            }
        }Ok(false)
    }
    fn retry<T>(&self, mut op: impl FnMut()->io::Result<T>) -> Result<T,StepError> {
        let map=|e:io::Error|if disk_full(&e){StepError{code:"reserve_exhausted",message:"복구 기록용 예약 공간이 부족합니다. 현재 데이터를 보존했습니다.",detail:None}}else if e.kind()==io::ErrorKind::InvalidData{invalid()}else{write_failed()};
        match op(){
            Ok(v)=>Ok(v),
            Err(e)if disk_full(&e)=>{
                if !self.release_reserve().map_err(|_|write_failed())?{return Err(map(e));}
                op().map_err(map)
            },Err(e)=>Err(map(e)),
        }
    }
    /// Explicit P0b allocation; no commit or release refills emergency space.
    pub(crate) fn ensure_app_pool(&self) -> Result<(),StepError> {
        if self.state.branch!=Branch::Upgrade {return Err(invalid());}
        self.app_reserves()?;
        let slots=self.root.join("slots"); let reserve=self.root.join("reserve");
        files::private_dir(&slots).map_err(|_|write_failed())?; files::private_dir(&reserve).map_err(|_|write_failed())?;
        for n in 0..16 {
            let path=slots.join(n.to_string());
            if fs::symlink_metadata(&path).is_ok() { checked_file(&path,false).map_err(|_|invalid())?; continue; }
            let f=OpenOptions::new().write(true).create_new(true).custom_flags(libc::O_NOFOLLOW).mode(0o600).open(&path).map_err(|_|write_failed())?;f.sync_all().map_err(|_|write_failed())?;
        }
        for n in 0..8 {
            let path=reserve.join(n.to_string());
            let mut f=if fs::symlink_metadata(&path).is_ok() {
                let f=checked_file(&path,true).map_err(|_|invalid())?;
                let metadata=f.metadata().map_err(|_|invalid())?; let size=metadata.len();
                if size>2*1024*1024 { return Err(invalid()); }
                if size==2*1024*1024 && metadata.blocks().saturating_mul(512)>=size { continue; }
                f.set_len(0).map_err(|_|write_failed())?; f
            } else { OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).map_err(|_|write_failed())? };
            let zeroes=[0u8;65536];
            for _ in 0..32 { f.write_all(&zeroes).map_err(|_|write_failed())?; } f.sync_all().map_err(|_|write_failed())?;
        }
        sync_dir(&slots).map_err(|_|write_failed())?; sync_dir(&reserve).map_err(|_|write_failed())?; sync_dir(&self.root).map_err(|_|write_failed())
    }
    fn refill_slots(&self) -> Result<(),StepError> {
        let slots=self.root.join("slots");
        let count=(0..16).filter(|n|fs::symlink_metadata(slots.join(n.to_string())).is_ok()).count();
        if count>=8 { return Ok(()); }
        for n in 0..16 {
            let path=slots.join(n.to_string());
            if fs::symlink_metadata(&path).is_ok() { continue; }
            self.retry(|| { let f=OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?; f.sync_all() })?;
        }
        self.retry(||sync_dir(&slots))
    }
    /// The verified streaming payload uses a preallocated, journal-owned temporary inode.
    pub(crate) fn publish_backup_stream(&self,locks:&StorageLocks,header:&super::backup_format::Header,mut fill:impl FnMut(&mut File)->Result<super::backup_format::Summary,StepError>)->Result<super::backup_format::Summary,StepError> {
        if self.state.branch!=Branch::Upgrade || !locks.holds_d() || !locks.matches_app(self.root.parent().ok_or_else(invalid)?)
            || header.backup_id!=self.state.attempt_id || !header.valid() {return Err(invalid());}
        let name=format!("backup-{}.jsonl",header.backup_id);let destination=self.root.join(name);
        match fs::symlink_metadata(&destination) {Err(e)if e.kind()==io::ErrorKind::NotFound=>{},_=>return Err(invalid())}
        let slots=self.root.join("slots");let path=(0..16).map(|n|slots.join(n.to_string())).find(|p|fs::symlink_metadata(p).is_ok()).ok_or_else(write_failed)?;
        let mut released=false;
        let summary=loop {
            let mut file=checked_file(&path,true).map_err(|_|invalid())?;
            file.set_len(0).map_err(|_|write_failed())?;
            match fill(&mut file) {
                Ok(summary)=>{file.sync_all().map_err(|_|write_failed())?;break summary;},
                Err(e)if e.code=="insufficient_space"=>{
                    if released || !self.release_reserve().map_err(|_|write_failed())?{return Err(StepError {code:"reserve_exhausted",message:"보관 기록용 예약 공간이 부족합니다. 기존 데이터를 보존했습니다.",detail:None});}released=true;
                },
                Err(e)=>return Err(e),
            }
        };
        self.retry(||fs::rename(&path,&destination))?;self.retry(||sync_dir(&slots))?;self.finish_backup_publication()?;
        Ok(summary)
    }
    pub(crate) fn finish_backup_publication(&self)->Result<(),StepError> {self.retry(||sync_dir(&self.root))?;self.refill_slots()}
    pub(crate) fn pre_pool(&self)->Result<bool,StepError>{
        files::real_path(&self.root).map_err(|_|write_failed())?;
        let absent=|name:&str|match fs::symlink_metadata(self.root.join(name)){Err(e)if e.kind()==std::io::ErrorKind::NotFound=>Ok(true),Ok(_)=>Ok(false),Err(_)=>Err(invalid())};
        let pre_pool=pre_pool_frame(&self.state)&&absent("slots")?&&absent("reserve")?;
        if pre_pool{let saved:JournalV1=files::read_json(&self.root.join("journal-v1.json")).map_err(|_|write_failed())?;if serde_json::to_value(saved).map_err(|_|invalid())?!=serde_json::to_value(&self.state).map_err(|_|invalid())?{return Err(invalid());}}
        Ok(pre_pool)
    }
    /// A.31: only the journal-owned pre-pool namespace may reuse the fixed init leaf.
    fn publish_init(&self,state:&JournalV1)->Result<(),StepError>{
        if !pre_pool_frame(state)&&state.branch==Branch::Upgrade{return Err(invalid());}
        let temporary=self.root.join(".journal-v1.json.init");
        match fs::symlink_metadata(&temporary){
            Ok(_)=>{let file=checked_file(&temporary,false).map_err(|_|invalid())?;let m=file.metadata().map_err(|_|invalid())?;if m.len()>1_048_576{return Err(invalid());}fs::remove_file(&temporary).map_err(|_|write_failed())?;sync_dir(&self.root).map_err(|_|write_failed())?;},
            Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},Err(_)=>return Err(write_failed()),
        }
        let bytes=serde_json::to_vec(state).map_err(|_|invalid())?;if bytes.len()>1_048_576{return Err(write_failed());}
        let mut file=OpenOptions::new().write(true).create_new(true).custom_flags(libc::O_NOFOLLOW).mode(0o600).open(&temporary).map_err(|_|write_failed())?;
        file.write_all(&bytes).map_err(|_|write_failed())?;publish_checkpoint("init-written");file.sync_all().map_err(|_|write_failed())?;publish_checkpoint("init-fsynced");
        fs::rename(&temporary,self.root.join("journal-v1.json")).map_err(|_|write_failed())?;publish_checkpoint("init-renamed");sync_dir(&self.root).map_err(|_|write_failed())?;publish_checkpoint("init-durable");Ok(())
    }
    /// Final pre-pool outcome and readiness receipt are one publication.
    pub(crate) fn finish_pre_pool(&mut self,identity:&str,error:Option<&StepError>,space:Option<super::types::StatusSpace>)->Result<(),StepError>{
        if !self.pre_pool()?{return Err(invalid());}let mut next=self.state.clone();
        next.outcome=if error.is_some(){Outcome::RefusedPreflight}else{Outcome::Postponed};
        let data=super::refusal::receipt(next.outcome,identity)?;
        next.records.push(Record{seq:self.next_seq()?,step:Step::P0b,state:match error{Some(e)=>StepState::Failed{code:e.code.into()},None=>StepState::Done},at:chrono::Utc::now().to_rfc3339(),data:space.map(|space|serde_json::json!({"space":space}))});
        let seq=next.records.last().ok_or_else(invalid)?.seq.checked_add(1).ok_or_else(invalid)?;
        next.records.push(Record{seq,step:Step::P5Publish,state:StepState::Done,at:chrono::Utc::now().to_rfc3339(),data:Some(data)});self.commit(next)
    }
    fn persist(&self, state: &JournalV1) -> Result<(),StepError> {
        if self.pre_pool()?{return self.publish_init(state);}
        let bytes=serde_json::to_vec(state).map_err(|_|invalid())?;
        if bytes.len()>1_048_576 { return Err(write_failed()); }
        if state.branch!=Branch::Upgrade { return files::atomic_json(&self.root.join("journal-v1.json"),state).map_err(|_|write_failed()); }
        self.publish_slot("journal-v1.json",&bytes)
    }
    fn publish_slot(&self,name:&str,bytes:&[u8])->Result<(),StepError> {
        if bytes.len()>1_048_576 {return Err(write_failed());}
        let slots=self.root.join("slots");
        let path=(0..16).map(|n|slots.join(n.to_string())).find(|p|fs::symlink_metadata(p).is_ok()).ok_or_else(write_failed)?;
        self.retry(|| {
            let mut f=checked_file(&path,true)?; f.set_len(0)?; f.write_all(bytes)?; publish_checkpoint("slot-written");f.sync_all()?;publish_checkpoint("slot-fsynced");Ok(())
        })?;
        self.retry(||fs::rename(&path,self.root.join(name)))?;publish_checkpoint("journal-renamed");
        self.retry(||sync_dir(&slots))?; self.retry(||sync_dir(&self.root))?;publish_checkpoint("journal-durable");
        Ok(())
    }

}

#[cfg(test)]fn publish_checkpoint(point:&str){if std::env::var("NWUI_JOURNAL_KILL_POINT").as_deref()==Ok(point){unsafe{libc::kill(libc::getpid(),libc::SIGKILL);}}}
#[cfg(not(test))]fn publish_checkpoint(_point:&str){}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use serde_json::json;

    fn pre_pool_fixture()->(tempfile::TempDir,crate::db_upgrade::layout::Roots,StorageLocks,Journal){
        let t=tempfile::tempdir().unwrap();let roots=crate::db_upgrade::layout::Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for path in [&roots.db,&roots.app]{fs::create_dir(path).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o700)).unwrap();}let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let j=Journal::begin_upgrade(&roots.app,&locks,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)},None).unwrap();(t,roots,locks,j)
    }
    #[tokio::test]
    async fn pre_pool_off_restarts_are_bounded_and_space_failure_can_admit_before_pools(){
        let(_t,r,l,mut j)=pre_pool_fixture();let candidate=j.load().candidate.clone();
        for n in 0..20{
            let identity=format!("{n:064x}");j.finish_pre_pool(&identity,None,None).unwrap();assert_eq!(super::super::runner::boot_plan(j.load(),Some(&identity)),super::super::runner::BootPlan::ParkedRefusal);assert!(j.pre_pool().unwrap());assert!(!r.db.join(".nwui-auto-work").exists());assert!(!r.db.join(".nwui-pg15-backup").exists());assert!(j.load().previous.len()<=8);
            let choice=super::super::rollover::rollover(&r,&l,&mut j,candidate.clone(),None,true,chrono::Utc::now()).await.unwrap();assert_eq!(choice,super::super::rollover::RolloverOutcome::RolledOver);
        }
        let error=StepError{code:"insufficient_space",message:"fixture",detail:None};let id="f".repeat(64);j.finish_pre_pool(&id,Some(&error),None).unwrap();assert_eq!(j.load().outcome,Outcome::RefusedPreflight);assert!(j.pre_pool().unwrap());super::super::rollover::rollover(&r,&l,&mut j,candidate,None,true,chrono::Utc::now()).await.unwrap();j.record(Step::P0b,StepState::Done,None).unwrap();assert!(j.record(Step::P1Backup,StepState::Intent,None).is_err());super::super::reserves::ensure(&r,&l,&j).unwrap();assert!(!j.pre_pool().unwrap());j.record(Step::P1Backup,StepState::Intent,None).unwrap();assert!(!r.app.join(".nwui-auto/.journal-v1.json.init").exists());
    }
    #[test]
    fn pre_pool_refuses_stale_frames_and_never_adopts_a_replacement_coordinator(){
        let(_t,r,l,mut j)=pre_pool_fixture();let mut stale=Journal::open(&r.app,&l).unwrap();j.record(Step::P0b,StepState::Done,None).unwrap();let before=fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap();assert!(stale.record(Step::P0b,StepState::Done,None).is_err());assert_eq!(fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap(),before);
        let root=r.app.join(".nwui-auto");let held=r.app.join("held");fs::rename(&root,&held).unwrap();fs::create_dir(&root).unwrap();fs::set_permissions(&root,fs::Permissions::from_mode(0o700)).unwrap();assert_eq!(j.record(Step::P0b,StepState::Done,None).unwrap_err().code,"journal_write_failed");assert!(fs::read_dir(&root).unwrap().next().is_none());fs::remove_dir(&root).unwrap();fs::rename(held,root).unwrap();j.record(Step::P0b,StepState::Done,None).unwrap();
    }
    #[test]
    fn pre_pool_kill_before_or_after_rename_leaves_a_valid_replayable_frame(){
        use std::os::unix::process::ExitStatusExt;
        if let Ok(root)=std::env::var("NWUI_PRE_POOL_CHILD"){
            let app=Path::new(&root).join("app");let db=Path::new(&root).join("db");let locks=StorageLocks::acquire(&db,&app).unwrap();let mut j=Journal::open(&app,&locks).unwrap();j.finish_pre_pool(&"a".repeat(64),None,None).unwrap();panic!("kill not reached");
        }
        for point in ["init-written","init-fsynced","init-renamed","init-durable"]{
            let(t,r,l,j)=pre_pool_fixture();drop(j);drop(l);let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::journal::tests::pre_pool_kill_before_or_after_rename_leaves_a_valid_replayable_frame","--nocapture"]).env("NWUI_PRE_POOL_CHILD",t.path()).env("NWUI_JOURNAL_KILL_POINT",point).status().unwrap();assert_eq!(status.signal(),Some(9));let locks=StorageLocks::acquire(&r.db,&r.app).unwrap();let mut j=Journal::open(&r.app,&locks).unwrap();assert!(j.pre_pool().unwrap());j.finish_pre_pool(&"a".repeat(64),None,None).unwrap();assert_eq!(super::super::refusal::outcome(j.load(),&"a".repeat(64)),Some(Outcome::Postponed));assert!(!r.app.join(".nwui-auto/slots").exists());
        }
    }
    #[test]
    #[ignore="owned 4MiB tmpfs /nwui-prepool-native; NWUI_TEST_PREPOOL_FULL=1"]
    fn native_pre_pool_full_a_never_publishes_a_final_receipt_and_recovers_after_space_is_freed(){
        if std::env::var("NWUI_TEST_PREPOOL_FULL").as_deref()!=Ok("1"){return;}
        let t=tempfile::tempdir_in("/nwui-prepool-native").unwrap();let db=t.path().join("db");let app=t.path().join("app");for path in [&db,&app]{fs::create_dir(path).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o700)).unwrap();}let locks=StorageLocks::acquire(&db,&app).unwrap();let mut j=Journal::begin_upgrade(&app,&locks,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)},None).unwrap();let original=fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap();
        let ballast=app.join("owned-ballast");let mut file=File::create(&ballast).unwrap();let chunk=[0u8;4096];loop{match file.write_all(&chunk){Ok(())=>{},Err(e)if e.raw_os_error()==Some(libc::ENOSPC)=>break,Err(e)=>panic!("{e}")}}file.sync_all().unwrap();drop(file);
        let identity="a".repeat(64);assert!(j.finish_pre_pool(&identity,None,None).is_err());assert_eq!(fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap(),original);assert_eq!(j.load().outcome,Outcome::Upgrading);assert!(super::super::refusal::outcome(j.load(),&identity).is_none());assert!(!app.join(".nwui-auto/slots").exists());assert!(!app.join(".nwui-auto/reserve").exists());
        fs::remove_file(ballast).unwrap();let mut j=Journal::open(&app,&locks).unwrap();j.finish_pre_pool(&identity,None,None).unwrap();assert_eq!(super::super::refusal::outcome(j.load(),&identity),Some(Outcome::Postponed));
    }
    #[test]
    fn initial_upgrade_label_is_in_the_first_frame_and_invalid_labels_create_nothing(){
        for label in ["first","",&"x".repeat(129)]{
            let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");for path in [&db,&app]{fs::create_dir(path).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o700)).unwrap();}let locks=StorageLocks::acquire(&db,&app).unwrap();
            let result=Journal::begin_upgrade(&app,&locks,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)},Some(label));
            if label=="first"{result.unwrap();let opened=Journal::open(&app,&locks).unwrap();assert_eq!(opened.load().retry.consumed_labels,[label]);assert_eq!(opened.load().records.len(),2);}else{assert!(result.is_err());assert!(!app.join(".nwui-auto").exists());}
        }
    }
    #[test]
    fn n0_journal_preserves_the_frozen_recovery_record() {
        let value = json!({"version":1,"attemptId":"fixture","branch":"upgrade",
            "candidate":{"revision":"0".repeat(40),"pinsSha256":"pins","migrationsSha256":"migrations"},
            "records":[{"seq":1,"step":"p1-backup","state":{"kind":"intent"},"at":"2026-10-27T00:00:00Z","data":null}],
            "moves":[{"seq":7,"phase":"restore","entry":"base","from":"BK","to":"L","state":{"kind":"done"}}],
            "handoff":"intent","outcome":"restore-incomplete",
            "retry":{"consumedLabels":[],"interruptions":0,"backoffUntil":null}});
        let journal: JournalV1 = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(journal).unwrap(), value);
    }
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    fn fixture(branch: Branch) -> (tempfile::TempDir, PathBuf, StorageLocks, Journal) {
        let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");
        for p in [&db,&app] { fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap(); }
        let locks=StorageLocks::acquire(&db,&app).unwrap();
        let candidate=Candidate { revision:"1".repeat(40), pins_sha256:"2".repeat(64), migrations_sha256:"3".repeat(64) };
        let journal={let fixture_journal=Journal::begin(&app,&locks,branch,candidate).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};
        (t,app,locks,journal)
    }
    #[test]
    fn w1_journal_restore_incomplete_requires_identity_and_replay_is_one_frame(){
        let(_t,app,locks,mut j)=fixture(Branch::Upgrade);let e=StepError{code:"io_error",message:"fixture",detail:None};let id="a".repeat(64);let error=j.fail_attempt_with_identity(Step::RRestore,&e,Outcome::RestoreIncomplete,chrono::Utc::now(),None).unwrap_err();assert_eq!(error.detail.as_deref(),Some("boot_identity"));assert_eq!(j.load().outcome,Outcome::Upgrading);assert!(super::super::refusal::outcome(j.load(),&id).is_none());assert_eq!(last_attempt_failure(&j.load().records).unwrap().code,e.code);
        j.fail_attempt_with_identity(Step::RRestore,&e,Outcome::RestoreIncomplete,chrono::Utc::now(),Some(&id)).unwrap();assert_eq!(j.load().outcome,Outcome::RestoreIncomplete);assert_eq!(super::super::refusal::outcome(j.load(),&id),Some(Outcome::RestoreIncomplete));let before=fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap();j.fail_attempt_with_identity(Step::RRestore,&e,Outcome::RestoreIncomplete,chrono::Utc::now(),Some(&id)).unwrap();assert_eq!(fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap(),before);assert_eq!(Journal::open(&app,&locks).unwrap().load().outcome,Outcome::RestoreIncomplete);
    }
    #[test]
    fn w1_journal_incomplete_kill_child(){
        let Ok(root)=std::env::var("NWUI_RESTORE_FAILURE_ROOT")else{return};let root=Path::new(&root);let locks=StorageLocks::acquire(&root.join("db"),&root.join("app")).unwrap();let mut j=Journal::open(&root.join("app"),&locks).unwrap();j.fail_attempt_with_identity(Step::RRestore,&StepError{code:"io_error",message:"fixture",detail:None},Outcome::RestoreIncomplete,chrono::Utc::now(),Some(&"a".repeat(64))).unwrap();panic!("kill not reached");
    }
    #[test]
    fn w1_journal_incomplete_real_kills_never_split_failure_outcome_and_receipt(){
        use std::os::unix::process::ExitStatusExt;
        for point in ["slot-written","slot-fsynced","journal-renamed","journal-durable"]{
            let(t,app,locks,j)=fixture(Branch::Upgrade);let roots=fixture_roots(&app);let old=serde_json::to_value(j.load()).unwrap();fs::write(roots.db.join("backup-proof"),b"preserved").unwrap();drop(j);drop(locks);let result=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::journal::storage_tests::w1_journal_incomplete_kill_child","--nocapture"]).env("NWUI_RESTORE_FAILURE_ROOT",t.path()).env("NWUI_JOURNAL_KILL_POINT",point).status().unwrap();assert_eq!(result.signal(),Some(9));let locks=StorageLocks::acquire(&roots.db,&app).unwrap();let mut j=Journal::open(&app,&locks).unwrap();let id="a".repeat(64);if j.load().outcome==Outcome::Upgrading{assert_eq!(serde_json::to_value(j.load()).unwrap(),old);}else{assert_eq!(j.load().outcome,Outcome::RestoreIncomplete);assert_eq!(super::super::refusal::outcome(j.load(),&id),Some(Outcome::RestoreIncomplete));assert_eq!(last_attempt_failure(&j.load().records).unwrap().code,"io_error");}j.fail_attempt_with_identity(Step::RRestore,&StepError{code:"io_error",message:"fixture",detail:None},Outcome::RestoreIncomplete,chrono::Utc::now(),Some(&id)).unwrap();assert_eq!(super::super::refusal::outcome(j.load(),&id),Some(Outcome::RestoreIncomplete));assert_eq!(fs::read(roots.db.join("backup-proof")).unwrap(),b"preserved");
        }
    }
    #[test]
    fn w1_journal_reader_refuses_attempt_path_aliases_without_mutation() {
        let (_t,app,locks,j)=fixture(Branch::Ready);let mut state=j.load().clone();state.attempt_id="abcdef01-2345-4678-9abc-def012345678".into();let id=uuid::Uuid::parse_str(&state.attempt_id).unwrap();let path=app.join(".nwui-auto/journal-v1.json");
        for alias in [id.simple().to_string(),id.braced().to_string(),id.urn().to_string(),state.attempt_id.to_uppercase()] {
            assert!(uuid::Uuid::parse_str(&alias).is_ok());assert_ne!(alias,state.attempt_id);let mut changed=state.clone();changed.attempt_id=alias;let bytes=serde_json::to_vec(&changed).unwrap();fs::write(&path,&bytes).unwrap();assert!(read_only(&app).is_err());assert!(Journal::open(&app,&locks).is_err());assert_eq!(fs::read(&path).unwrap(),bytes);
        }
        fs::write(&path,serde_json::to_vec(&state).unwrap()).unwrap();assert_eq!(read_only(&app).unwrap().attempt_id,state.attempt_id);
    }
    fn rollover_frame(j:&Journal)->JournalV1 {
        let mut next=j.load().clone();let old=PreviousAttempt{attempt_id:next.attempt_id.clone(),outcome:next.outcome,candidate:next.candidate.clone(),last_failure:last_attempt_failure(&next.records),generation:None,discard_id:None,ended_at:"2026-10-28T00:00:00Z".into()};
        next.previous.insert(0,old);next.attempt_id=uuid::Uuid::new_v4().to_string();next.outcome=Outcome::Upgrading;next.handoff=Handoff::None;next.moves.clear();next.records=vec![Record{seq:1,step:Step::P0,state:StepState::Done,at:"2026-10-28T00:00:00Z".into(),data:None},Record{seq:2,step:Step::P0b,state:StepState::Intent,at:"2026-10-28T00:00:00Z".into(),data:None}];next
    }
    fn fixture_roots(app:&Path)->crate::db_upgrade::layout::Roots {crate::db_upgrade::layout::Roots{db:app.parent().unwrap().join("db"),app:app.to_owned(),architecture:crate::db_upgrade::engine::architecture().unwrap().into()}}
    #[test]
    fn w1_journal_failure_is_durable_before_a_refused_outcome_and_commands_cannot_supply_it() {
        for outcome in [Outcome::RefusedPreflight,Outcome::RefusedFresh,Outcome::RefusedPrivilege,Outcome::RestoreIncomplete,Outcome::ActivationFailed] {
            let (_t,app,locks,mut j)=fixture(Branch::Upgrade);
            j.record(Step::CmdRemoveDiscarded,StepState::Failed{code:"io_error".into()},None).unwrap();
            assert!(j.set_outcome(outcome).is_err());assert_eq!(j.load().outcome,Outcome::Upgrading);
            let now=chrono::Utc::now();let error=StepError{code:"ledger_unknown",message:"fixture",detail:None};
            j.fail_attempt_with_identity(Step::P2Preflight,&error,outcome,now,Some(&"a".repeat(64))).unwrap();
            let reopened=Journal::open(&app,&locks).unwrap();assert_eq!(reopened.load().outcome,outcome);
            let failure=last_attempt_failure(&reopened.load().records).unwrap();assert_eq!(failure.step,Step::P2Preflight);assert_eq!(failure.code,"ledger_unknown");
            let before=fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap();
            assert!(j.fail_attempt(Step::CmdRestorePg15,&error,outcome,now).is_err());
            assert!(j.fail_attempt(Step::P2Preflight,&error,Outcome::Pg17,now).is_err());
            assert_eq!(fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap(),before);
        }
    }
    #[tokio::test]
    async fn w1_journal_rollover_never_folds_an_unfinished_operator_command() {
        for step in [Step::CmdRestorePg15,Step::CmdRemoveBackup,Step::CmdRemoveDiscarded] {
            for completion in [StepState::Done,StepState::Failed{code:"io_error".into()}] {
                let (_t,app,locks,mut j)=fixture(Branch::Upgrade);let roots=fixture_roots(&app);j.set_outcome(Outcome::Restored).unwrap();
                let next=rollover_frame(&j);let proof=j.discard_for_rollover(&roots,&locks).await.unwrap();
                j.record(step,StepState::Intent,None).unwrap();
                assert!(j.discard_for_rollover(&roots,&locks).await.is_err());
                let current_proof=RolloverDiscarded{attempt_id:proof.attempt_id,last_seq:j.next_seq().unwrap()-1};
                assert!(j.replace_attempt_locked(&roots,&locks,next,current_proof).is_err());
                let failed=matches!(completion,StepState::Failed{..});j.record(step,completion,None).unwrap();
                if failed{assert!(j.discard_for_rollover(&roots,&locks).await.is_err());j.record(step,StepState::Done,None).unwrap();}
                let next=rollover_frame(&j);let proof=j.discard_for_rollover(&roots,&locks).await.unwrap();
                j.replace_attempt_locked(&roots,&locks,next,proof).unwrap();assert!(j.load().moves.is_empty());
            }
        }
    }
    #[tokio::test]
    async fn w1_journal_rollover_latest_completed_move_pair_passes_both_api_guards() {
        let (_t,app,locks,mut j)=fixture(Branch::Upgrade);let roots=fixture_roots(&app);
        for state in [StepState::Intent,StepState::Done] {j.record_move(MoveRecord{phase:MovePhase::Restore,entry:"base".into(),from:"BK".into(),to:"L".into(),state}).unwrap();}
        j.set_outcome(Outcome::Restored).unwrap();assert!(moves_done(&j.load().moves));let next=rollover_frame(&j);let proof=j.discard_for_rollover(&roots,&locks).await.unwrap();j.replace_attempt_locked(&roots,&locks,next,proof).unwrap();assert!(j.load().moves.is_empty());
    }
    #[tokio::test]
    async fn w1_journal_rollover_unmatched_move_tuple_refuses_both_api_guards() {
        let (_t,app,locks,mut j)=fixture(Branch::Upgrade);let roots=fixture_roots(&app);j.set_outcome(Outcome::Restored).unwrap();let next=rollover_frame(&j);let proof=j.discard_for_rollover(&roots,&locks).await.unwrap();
        for state in [StepState::Intent,StepState::Done] {j.record_move(MoveRecord{phase:MovePhase::Restore,entry:"base".into(),from:"BK".into(),to:"L".into(),state}).unwrap();}
        j.record_move(MoveRecord{phase:MovePhase::SetAside,entry:"base".into(),from:"L".into(),to:"W".into(),state:StepState::Intent}).unwrap();assert!(!moves_done(&j.load().moves));
        assert!(j.discard_for_rollover(&roots,&locks).await.is_err());let proof=RolloverDiscarded{attempt_id:proof.attempt_id,last_seq:j.next_seq().unwrap()-1};let before=fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap();assert!(j.replace_attempt_locked(&roots,&locks,next,proof).is_err());assert_eq!(fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap(),before);
    }
    #[test]
    fn w1_journal_previous_last_failure_excludes_every_operator_command_step() {
        let (_t,_app,_locks,mut j)=fixture(Branch::Upgrade);j.record_retry_failure(Step::P2Preflight,"ledger_unknown",chrono::Utc::now()).unwrap();
        for step in [Step::CmdRestorePg15,Step::CmdRemoveBackup,Step::CmdRemoveDiscarded] {j.record(step,StepState::Failed{code:"invalid_arguments".into()},None).unwrap();}
        let mut next=rollover_frame(&j);assert_eq!(next.previous[0].last_failure.as_ref().unwrap().step,Step::P2Preflight);next.validate().unwrap();
        for step in [Step::CmdRestorePg15,Step::CmdRemoveBackup,Step::CmdRemoveDiscarded] {next.previous[0].last_failure=Some(PreviousFailure{step,code:"invalid_arguments".into()});assert!(next.validate().is_err());}
    }
    #[tokio::test]
    async fn w1_journal_rollover_installer_replaces_old_records_and_preserves_retry_and_previous() {
        let (_t,app,locks,mut j)=fixture(Branch::Upgrade);let roots=fixture_roots(&app);j.record(Step::P1Backup,StepState::Done,None).unwrap();j.record_retry_failure(Step::P2Preflight,"ledger_unknown",chrono::Utc::now()).unwrap();j.set_outcome(Outcome::Restored).unwrap();
        let old_id=j.load().attempt_id.clone();let mut next=rollover_frame(&j);next.retry.consumed_labels.push("once".into());let new_id=next.attempt_id.clone();
        let proof=j.discard_for_rollover(&roots,&locks).await.unwrap();j.replace_attempt_locked(&roots,&locks,next,proof).unwrap();let opened=Journal::open(&app,&locks).unwrap();assert_eq!(opened.load().attempt_id,new_id);assert_eq!(opened.load().previous[0].attempt_id,old_id);assert_eq!(opened.load().records.len(),2);assert!(!opened.load().records.iter().any(|r|r.step==Step::P1Backup));assert_eq!(opened.load().retry.consumed_labels,["once"]);
    }
    #[tokio::test]
    async fn w1_journal_rollover_installer_rejects_stale_discard_proof_and_nonfresh_frames() {
        for change in ["stale","records","id","retry","previous"] {let (_t,app,locks,mut j)=fixture(Branch::Upgrade);let roots=fixture_roots(&app);j.set_outcome(Outcome::Restored).unwrap();let mut next=rollover_frame(&j);let proof=j.discard_for_rollover(&roots,&locks).await.unwrap();
            match change {"stale"=>j.record(Step::P0,StepState::Done,None).unwrap(),"records"=>next.records.push(Record{seq:3,step:Step::P1Backup,state:StepState::Done,at:"2026-10-28T00:00:00Z".into(),data:None}),"id"=>next.attempt_id=j.load().attempt_id.clone(),"retry"=>next.retry.interruptions+=1,_=>next.previous.clear()};
            let before=fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap();assert!(j.replace_attempt_locked(&roots,&locks,next,proof).is_err());assert_eq!(fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap(),before);
        }
    }
    #[test]
    fn w1_journal_previous_default_and_bounds_preserve_v1_compatibility() {
        let (_t,_app,_locks,j)=fixture(Branch::Upgrade);let before=serde_json::to_value(j.load()).unwrap();assert!(before.get("previous").is_none());let parsed:JournalV1=serde_json::from_value(before.clone()).unwrap();assert!(parsed.previous.is_empty());assert_eq!(serde_json::to_value(parsed).unwrap(),before);
        let mut next=rollover_frame(&j);next.validate().unwrap();let literal=serde_json::to_value(&next).unwrap();assert_eq!(serde_json::to_value(serde_json::from_value::<JournalV1>(literal.clone()).unwrap()).unwrap(),literal);
        next.previous.push(next.previous[0].clone());assert!(next.validate().is_err());next.previous.pop();next.attempt_id=next.previous[0].attempt_id.clone();assert!(next.validate().is_err());
    }
    #[test]
    fn w1_journal_diagnostic_snapshot_survives_atomic_replacement_without_relocking() {
        let (_t,app,_locks,mut journal)=fixture(Branch::Ready);
        let file=File::open(app.join(".nwui-auto/journal-v1.json")).unwrap();let count=journal.load().records.len();
        journal.record(Step::Y1,StepState::Done,None).unwrap();assert_eq!(file.metadata().unwrap().nlink(),0);
        assert_eq!(read_snapshot(file).unwrap().records.len(),count);assert_eq!(read_only(&app).unwrap().records.len(),count+1);
    }
    #[test]
    fn w1_journal_records_survive_reopen_and_refill_the_slot_pool() {
        let (_t,app,locks,mut j)=fixture(Branch::Upgrade);
        for _ in 0..20 { j.record(Step::P1Backup,StepState::Intent,None).unwrap(); }
        j.record_move(MoveRecord { phase:MovePhase::Restore, entry:"base".into(), from:"BK".into(), to:"L".into(), state:StepState::Intent }).unwrap();
        let id=j.load().attempt_id.clone();drop(j);
        let j=Journal::open(&app,&locks).unwrap();assert_eq!(j.load().attempt_id,id);
        assert_eq!(j.load().records.len(),22);assert_eq!(j.load().moves[0].seq,23);
        assert_eq!(j.load().moves[0].record.state,StepState::Intent);
        let slots=fs::read_dir(j.root.join("slots")).unwrap().count();assert!((8..=16).contains(&slots));
        assert_eq!(fs::read_dir(j.root.join("reserve")).unwrap().count(),8);
    }
    #[test]
    fn w1_journal_releases_a_real_reserve_only_for_capacity_errors() {
        let (_t,_app,_locks,j)=fixture(Branch::Upgrade);let mut attempts=0;
        let result=j.retry(|| { attempts+=1; if attempts==1 { Err(io::Error::from_raw_os_error(libc::ENOSPC)) } else { Ok(7) } });
        assert_eq!(result.unwrap(),7);assert_eq!(fs::read_dir(j.root.join("reserve")).unwrap().count(),7);
        assert_eq!(j.retry::<()>(||Err(io::Error::from_raw_os_error(libc::EIO))).unwrap_err().code,"journal_write_failed");
        assert_eq!(fs::read_dir(j.root.join("reserve")).unwrap().count(),7);
        let mut persistent=0;assert_eq!(j.retry::<()>(||{persistent+=1;Err(io::Error::from_raw_os_error(libc::EDQUOT))}).unwrap_err().code,"reserve_exhausted");assert_eq!(persistent,2);assert_eq!(j.app_reserves().unwrap().1,6);
        for _ in 0..6 { assert!(j.release_reserve().unwrap()); }
        assert_eq!(j.retry::<()>(||Err(io::Error::from_raw_os_error(libc::EDQUOT))).unwrap_err().code,"reserve_exhausted");
    }
    #[test]
    fn w1_journal_stream_capacity_retry_releases_only_one_reserve_and_preserves_authority() {
        let (_t,app,locks,j)=fixture(Branch::Upgrade);let before=fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap();
        let header=super::super::backup_format::Header{version:1,backup_id:j.load().attempt_id.clone(),source:"L".into(),created_at:"2026-10-29T00:00:00Z".into(),source_location:super::super::backup_format::SourceLocation::Root,control_state:"shut down".into(),system_identifier:"123".into(),checkpoint_sha256:"0".repeat(64),root:super::super::backup_format::Root{mode:0o700,uid:0,gid:0}};
        let mut tries=0;let e=j.publish_backup_stream(&locks,&header,|_|{tries+=1;Err(StepError{code:"insufficient_space",message:"fixture",detail:None})}).unwrap_err();
        assert_eq!(tries,2);assert_eq!(e.code,"reserve_exhausted");assert_eq!(j.app_reserves().unwrap().1,7);assert_eq!(fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap(),before);assert!(!app.join(".nwui-auto").join(format!("backup-{}.jsonl",j.load().attempt_id)).exists());
    }
    #[test]
    fn w1_journal_refuses_bad_moves_and_oversized_data_without_changing_the_record() {
        let (_t,app,locks,mut j)=fixture(Branch::Ready);
        assert!(!j.root.join("slots").exists());assert!(!j.root.join("reserve").exists());
        let path=j.root.join("journal-v1.json");let before=fs::read(&path).unwrap();
        assert!(j.record_move(MoveRecord { phase:MovePhase::Restore,entry:"../outside".into(),from:"BK".into(),to:"L".into(),state:StepState::Done }).is_err());
        assert!(j.record(Step::P0,StepState::Done,Some(serde_json::json!("x".repeat(65537)))).is_err());
        assert_eq!(fs::read(&path).unwrap(),before);assert!(j.load().records.is_empty());
        let original=j.root.join("original");fs::rename(&path,&original).unwrap();symlink(original,&path).unwrap();
        assert!(Journal::open(&app,&locks).is_err());
    }
}

#[cfg(test)]
mod bootstrap_tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::{PermissionsExt, symlink};
    fn roots() -> (tempfile::TempDir,PathBuf,PathBuf,StorageLocks) {
        let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");
        for p in [&db,&app] { fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap(); }
        let locks=StorageLocks::acquire(&db,&app).unwrap();(t,db,app,locks)
    }
    fn tree(root:&Path) -> Vec<String> {
        fn walk(root:&Path,p:&Path,out:&mut Vec<String>) {
            let m=fs::symlink_metadata(p).unwrap();
            let value=if m.file_type().is_symlink(){format!("link:{:?}",fs::read_link(p).unwrap())}
                else if m.is_file(){hex::encode(Sha256::digest(fs::read(p).unwrap()))}else{"directory".into()};
            out.push(format!("{:?}:{}:{}:{}:{}:{value}",p.strip_prefix(root).unwrap(),m.mode(),m.uid(),m.gid(),m.nlink()));
            if m.is_dir() { let mut children=fs::read_dir(p).unwrap().map(|r|r.unwrap().path()).collect::<Vec<_>>();children.sort();for c in children { walk(root,&c,out); } }
        }
        let mut out=Vec::new();walk(root,root,&mut out);out
    }
    #[test]
    fn w1_bootstrap_accepts_only_empty_or_the_fixed_initial_temp() {
        for payload in [None,Some(""),Some("interrupted bytes, not yet valid JSON")] {
            let (_t,_db,app,locks)=roots();let root=app.join(".nwui-auto");files::private_dir(&root).unwrap();
            if let Some(bytes)=payload { fs::write(root.join(".journal-v1.json.init"),bytes).unwrap(); }
            assert!(prepare_bootstrap(&app,&locks).unwrap());assert_eq!(fs::read_dir(&root).unwrap().count(),0);
            let candidate=Candidate { revision:"1".repeat(40),pins_sha256:"2".repeat(64),migrations_sha256:"3".repeat(64) };
            let j=Journal::begin(&app,&locks,Branch::Ready,candidate).unwrap();
            assert_eq!(Journal::open(&app,&locks).unwrap().load().attempt_id,j.load().attempt_id);
            assert!(!root.join(".journal-v1.json.init").exists());
        }
    }
    #[test]
    fn w1_bootstrap_lookalikes_refuse_without_deleting_anything() {
        for bad in ["second","other-temp","temp-link","temp-owner","root-owner","mode","root-link","temp-dir","db-work","db-backup","db-discard","publisher","journal"] {
            let (t,db,app,locks)=roots();let root=app.join(".nwui-auto");files::private_dir(&root).unwrap();let tmp=root.join(".journal-v1.json.init");fs::write(&tmp,"preserve").unwrap();
            match bad {
                "second"=>fs::write(root.join("other"),"preserve").unwrap(),
                "other-temp"=>fs::rename(&tmp,root.join(".nwui-random.tmp")).unwrap(),
                "temp-link"=>{fs::remove_file(&tmp).unwrap();let p=t.path().join("outside");fs::write(&p,"preserve").unwrap();symlink(p,&tmp).unwrap();},
                "temp-owner"=>std::os::unix::fs::chown(&tmp,Some(1000),None).unwrap(),
                "root-owner"=>std::os::unix::fs::chown(&root,Some(1000),None).unwrap(),
                "mode"=>fs::set_permissions(&root,fs::Permissions::from_mode(0o755)).unwrap(),
                "root-link"=>{fs::remove_file(&tmp).unwrap();fs::remove_dir(&root).unwrap();let p=t.path().join("outside");fs::create_dir(&p).unwrap();symlink(p,&root).unwrap();},
                "temp-dir"=>{fs::remove_file(&tmp).unwrap();fs::create_dir(&tmp).unwrap();},
                "db-work"=>fs::create_dir(db.join(".nwui-auto-work")).unwrap(),
                "db-backup"=>fs::create_dir(db.join(".nwui-pg15-backup")).unwrap(),
                "db-discard"=>fs::create_dir(db.join(".nwui-discard")).unwrap(),
                "publisher"=>fs::create_dir(app.join(".nwui-publisher")).unwrap(),
                _=>fs::write(root.join("journal-v1.json"),"{}").unwrap(),
            }
            let before=tree(t.path());
            assert_eq!(prepare_bootstrap(&app,&locks).unwrap_err().code,"unsupported_layout","{bad}");
            assert_eq!(tree(t.path()),before,"{bad}");
        }
    }
}

#[cfg(test)]
mod pins_tests {
    use super::*;use std::os::unix::fs::{PermissionsExt,symlink};use sha2::{Digest,Sha256};
    #[test]
    fn w1_pins_hashes_exact_safe_baked_bytes_and_requires_the_same_revision() {
        let t=tempfile::tempdir().unwrap();let path=t.path().join("inputs");let revision="a".repeat(40);let bytes=format!("{{\"product\":{{\"revision\":\"{revision}\"}},\"pins\":[1]}}\n");fs::write(&path,&bytes).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o644)).unwrap();
        let first=pins_from(&path,&revision).unwrap();assert_eq!(first,hex::encode(Sha256::digest(bytes.as_bytes())));fs::write(&path,bytes.trim_end()).unwrap();assert_ne!(pins_from(&path,&revision).unwrap(),first);assert_eq!(pins_from(&path,&"b".repeat(40)).unwrap_err().detail.as_deref(),Some("build_identity"));assert!(pins_from(&path,&"A".repeat(40)).is_err());
    }
    #[test]
    fn w1_pins_refuses_missing_alias_fifo_directory_wrong_owner_writable_or_oversize_without_writing() {
        for case in ["missing","link","fifo","directory","owner","group","world","oversize","json","revision"] {
            let t=tempfile::tempdir().unwrap();let path=t.path().join("inputs");let revision="a".repeat(40);fs::write(&path,format!("{{\"product\":{{\"revision\":\"{revision}\"}}}}\n")).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o644)).unwrap();
            match case {"missing"=>fs::remove_file(&path).unwrap(),"link"=>{fs::rename(&path,t.path().join("real")).unwrap();symlink(t.path().join("real"),&path).unwrap();},"fifo"=>{fs::remove_file(&path).unwrap();let c=std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();assert_eq!(unsafe{libc::mkfifo(c.as_ptr(),0o600)},0);},"directory"=>{fs::remove_file(&path).unwrap();fs::create_dir(&path).unwrap();},"owner"=>std::os::unix::fs::chown(&path,Some(10001),None).unwrap(),"group"=>fs::set_permissions(&path,fs::Permissions::from_mode(0o664)).unwrap(),"world"=>fs::set_permissions(&path,fs::Permissions::from_mode(0o646)).unwrap(),"oversize"=>OpenOptions::new().write(true).open(&path).unwrap().set_len(BUILD_INPUTS_MAX+1).unwrap(),"json"=>fs::write(&path,b"invalid").unwrap(),_=>fs::write(&path,b"{}").unwrap()}
            let error=pins_from(&path,&revision).unwrap_err();assert_eq!(error.code,"verification_failed","{case}");assert_eq!(error.detail.as_deref(),Some("build_identity"));
        }
    }
}

#[cfg(test)]
mod removal_tests {
    use super::*;use std::os::unix::fs::PermissionsExt;
    #[test]
    fn w1_remove_discarded_atomically_clears_only_named_retained_fields_with_done_and_replays() {
        let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");for p in [&db,&app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}let locks=StorageLocks::acquire(&db,&app).unwrap();let candidate=Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)};let mut j=Journal::begin(&app,&locks,Branch::Ready,candidate.clone()).unwrap();let g=uuid::Uuid::new_v4().to_string();let other=uuid::Uuid::new_v4().to_string();let mut next=j.load().clone();for id in [&g,&other]{next.previous.push(PreviousAttempt{attempt_id:uuid::Uuid::new_v4().to_string(),outcome:Outcome::Restored,candidate:candidate.clone(),last_failure:None,generation:Some(id.clone()),discard_id:Some(id.clone()),ended_at:"2026-10-29T00:00:00Z".into()});}j.commit(next).unwrap();
        assert!(j.finish_remove_discarded(&g).is_err());j.record(Step::CmdRemoveDiscarded,StepState::Intent,Some(serde_json::json!({"discardId":g}))).unwrap();assert!(!commands_settled(&j.load().records));assert!(j.finish_remove_discarded(&other).is_err());j.finish_remove_discarded(&g).unwrap();assert!(j.load().previous[0].generation.is_none() && j.load().previous[0].discard_id.is_none());assert_eq!(j.load().previous[1].discard_id.as_deref(),Some(other.as_str()));assert!(commands_settled(&j.load().records));let before=fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap();j.finish_remove_discarded(&g).unwrap();assert_eq!(fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap(),before);let reopened=Journal::open(&app,&locks).unwrap();assert!(reopened.load().previous[0].discard_id.is_none());assert_eq!(reopened.load().records.last().unwrap().state,StepState::Done);
    }
    #[test]
    fn w1_command_failure_or_unrelated_done_never_settles_an_unfinished_intent() {
        let record=|seq,state,data|Record{seq,step:Step::CmdRemoveDiscarded,state,at:"2026-10-29T00:00:00Z".into(),data:Some(data)};let g=serde_json::json!({"discardId":"a"});let mut rows=vec![record(1,StepState::Intent,g.clone()),record(2,StepState::Failed{code:"io_error".into()},g.clone())];assert!(!commands_settled(&rows));rows.push(record(3,StepState::Done,serde_json::json!({"discardId":"b"})));assert!(!commands_settled(&rows));rows.push(record(4,StepState::Done,g));assert!(commands_settled(&rows));rows.push(record(5,StepState::Failed{code:"io_error".into()},serde_json::Value::Null));assert!(commands_settled(&rows));
    }
}

#[cfg(test)]
mod restore_finish_tests {
    use super::*;
    use super::super::{selector::{Selector,Origin},runner::{self,BootPlan}};
    use crate::db_upgrade::layout::Roots;
    use std::os::unix::{fs::{PermissionsExt,symlink},process::ExitStatusExt};
    fn fixture(outcome:Outcome)->(tempfile::TempDir,Roots,StorageLocks,Journal,RestorePg15Data,Selector) {
        let t=tempfile::tempdir().unwrap();let r=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
        for p in [&r.db,&r.app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let l=StorageLocks::acquire(&r.db,&r.app).unwrap();let mut j=Journal::begin(&r.app,&l,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();j.ensure_app_pool().unwrap();
        let g=uuid::Uuid::new_v4().to_string();j.record(Step::P5Commit,StepState::Intent,Some(serde_json::json!({"generation":g}))).unwrap();j.record(Step::P5Commit,StepState::Done,None).unwrap();j.set_outcome(Outcome::Pg17).unwrap();j.handoff(Handoff::Intent,&"a".repeat(64)).unwrap();j.handoff(Handoff::Done,&"a".repeat(64)).unwrap();
        if outcome==Outcome::ActivationFailed{j.record_retry_failure(Step::S4Activate,"activation_timeout",chrono::Utc::now()).unwrap();}j.set_outcome(outcome).unwrap();
        let selector=Selector{version:1,active_major:15,generation:None,system_identifier:"123456".into(),origin:Origin::Auto,backup_id:Some(j.load().attempt_id.clone()),profile_id:Some("legacy-28".into()),profile_sha256:Some("3".repeat(64)),manifest_sha256:None,operator_reason:Some("restore-pg15".into())};
        let data=RestorePg15Data::bind(&j.load().attempt_id,&g,&g,&selector).unwrap();let path=r.app.join(".nwui-auto/active-v1.json");fs::write(&path,serde_json::to_vec(&selector).unwrap()).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();j.record(Step::CmdRestorePg15,StepState::Intent,Some(serde_json::to_value(&data).unwrap())).unwrap();(t,r,l,j,data,selector)
    }
    #[test]
    fn w1_restore_pg15_finish_atomically_records_done_restored_and_void_handoff() {
        for outcome in [Outcome::Pg17,Outcome::ActivationFailed]{let (_t,r,l,mut j,data,_)=fixture(outcome);let count=j.load().records.len();let failures=j.load().records.iter().filter(|r|matches!(r.state,StepState::Failed{..})).count();let bytes=fs::read(r.app.join(".nwui-auto/active-v1.json")).unwrap();
            assert_eq!(serde_json::to_value(&data).unwrap().as_object().unwrap().len(),3);
            j.finish_restore_pg15(&r,&l,&data).unwrap();let saved=Journal::open(&r.app,&l).unwrap();assert_eq!(saved.load().outcome,Outcome::Restored);assert_eq!(saved.load().handoff,Handoff::None);assert_eq!(saved.load().records.len(),count+1);let last=saved.load().records.last().unwrap();assert_eq!(last.step,Step::CmdRestorePg15);assert_eq!(last.state,StepState::Done);assert_eq!(last.data,Some(serde_json::to_value(&data).unwrap()));assert_eq!(saved.load().records.iter().filter(|r|matches!(r.state,StepState::Failed{..})).count(),failures);assert!(commands_settled(&saved.load().records));assert_eq!(runner::boot_plan(saved.load(),Some(&"a".repeat(64))),BootPlan::Phase1);assert_eq!(fs::read(r.app.join(".nwui-auto/active-v1.json")).unwrap(),bytes);
            let before=fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap();assert!(j.finish_restore_pg15(&r,&l,&data).is_err());assert_eq!(fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap(),before);
        }
    }
    #[test]
    fn w1_restore_pg15_finish_refuses_every_unfinished_or_unbound_precondition_without_writes() {
        for case in ["pair","pair-link","backup","work","work-link","parent-link","selector-bytes","selector-id","selector-link","intent","generation","done","failed","outcome","stale","locks","roots"]{
            let (t,r,mut l,mut j,mut data,mut selector)=fixture(Outcome::Pg17);let path=r.app.join(".nwui-auto/active-v1.json");
            match case {
                "pair"=>fs::create_dir(r.app.join(".nwui")).unwrap(),"pair-link"=>symlink("missing",r.app.join(".nwui")).unwrap(),
                "backup"=>fs::create_dir_all(r.db.join(".nwui-pg15-backup").join(&data.backup_id)).unwrap(),
                "work"=>fs::create_dir_all(r.db.join(".nwui-auto-work").join(&data.backup_id)).unwrap(),
                "work-link"=>{fs::create_dir(r.db.join(".nwui-auto-work")).unwrap();symlink("missing",r.db.join(".nwui-auto-work").join(&data.backup_id)).unwrap();},
                "parent-link"=>{let outside=t.path().join("outside");fs::create_dir(&outside).unwrap();symlink(outside,r.db.join(".nwui-pg15-backup")).unwrap();},
                "selector-bytes"=>{let mut b=fs::read(&path).unwrap();b.push(b'\n');fs::write(&path,b).unwrap();},
                "selector-id"=>{selector.system_identifier="654321".into();fs::write(&path,serde_json::to_vec(&selector).unwrap()).unwrap();},
                "selector-link"=>{let outside=t.path().join("outside");fs::rename(&path,&outside).unwrap();symlink(outside,&path).unwrap();},
                "intent"=>j.record(Step::CmdRestorePg15,StepState::Intent,Some(serde_json::json!({"backupId":data.backup_id,"generation":data.generation,"discardId":uuid::Uuid::new_v4().to_string()}))).unwrap(),
                "generation"=>{let g=uuid::Uuid::new_v4().to_string();data=RestorePg15Data::bind(&data.backup_id,&g,&g,&selector).unwrap();j.record(Step::CmdRestorePg15,StepState::Intent,Some(serde_json::to_value(&data).unwrap())).unwrap();},
                "done"=>j.record(Step::CmdRestorePg15,StepState::Done,Some(serde_json::to_value(&data).unwrap())).unwrap(),
                "failed"=>j.record(Step::CmdRestorePg15,StepState::Failed{code:"io_error".into()},Some(serde_json::to_value(&data).unwrap())).unwrap(),
                "outcome"=>j.set_outcome(Outcome::Postponed).unwrap(),
                "stale"=>Journal::open(&r.app,&l).unwrap().record(Step::P6Cleanup,StepState::Intent,Some(serde_json::json!({"workDir":data.backup_id}))).unwrap(),
                "locks"=>l.release_d(),"roots"=>{},_=>unreachable!(),
            }
            let before=fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap();let state=serde_json::to_value(j.load()).unwrap();let mut roots=r.clone();if case=="roots"{roots.db=t.path().join("other");fs::create_dir(&roots.db).unwrap();}
            assert_eq!(j.finish_restore_pg15(&roots,&l,&data).unwrap_err().code,"pair_mismatch","{case}");assert_eq!(fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap(),before,"{case}");assert_eq!(serde_json::to_value(j.load()).unwrap(),state,"{case}");
        }
    }
    #[test]
    fn w1_work_dir_reader_rejects_intent_after_done_in_the_same_cycle() {
        let (_t,r,l,mut j,data,_)=fixture(Outcome::Pg17);let work=Some(serde_json::json!({"workDir":data.backup_id}));j.record(Step::P6Cleanup,StepState::Intent,work.clone()).unwrap();j.record(Step::P6Cleanup,StepState::Done,work.clone()).unwrap();
        let mut bad=j.load().clone();bad.records.push(Record{seq:j.next_seq().unwrap(),step:Step::P6Cleanup,state:StepState::Intent,at:chrono::Utc::now().to_rfc3339(),data:work});fs::write(r.app.join(".nwui-auto/journal-v1.json"),serde_json::to_vec(&bad).unwrap()).unwrap();assert!(Journal::open(&r.app,&l).is_err());
    }
    #[test]
    fn w1_work_dir_cycles_reset_only_on_restore_intent_and_finish_in_the_current_cycle() {
        let (_t,r,l,mut j,data,_)=fixture(Outcome::Pg17);let mut start=j.load().clone();assert_eq!(start.records.pop().unwrap().step,Step::CmdRestorePg15);j.commit(start).unwrap();let work=Some(serde_json::json!({"workDir":data.backup_id}));
        assert_eq!(j.work_dir_state(&data.backup_id),WorkDirState::None);j.record(Step::P6Cleanup,StepState::Intent,work.clone()).unwrap();assert_eq!(j.work_dir_state(&data.backup_id),WorkDirState::Intent);j.record(Step::P6Cleanup,StepState::Intent,work.clone()).unwrap();assert_eq!(j.work_dir_state(&data.backup_id),WorkDirState::Intent);j.record(Step::P6Cleanup,StepState::Done,work.clone()).unwrap();assert_eq!(j.work_dir_state(&data.backup_id),WorkDirState::Done);
        j.record(Step::CmdRestorePg15,StepState::Intent,Some(serde_json::to_value(&data).unwrap())).unwrap();assert_eq!(j.work_dir_state(&data.backup_id),WorkDirState::None);assert_eq!(j.work_dir_state(&uuid::Uuid::new_v4().to_string()),WorkDirState::None);
        j.record(Step::P6Cleanup,StepState::Intent,work.clone()).unwrap();assert_eq!(Journal::open(&r.app,&l).unwrap().work_dir_state(&data.backup_id),WorkDirState::Intent);j.record(Step::P6Cleanup,StepState::Done,work).unwrap();assert_eq!(j.work_dir_state(&data.backup_id),WorkDirState::Done);j.finish_restore_pg15(&r,&l,&data).unwrap();assert_eq!(j.work_dir_state(&data.backup_id),WorkDirState::Done);assert_eq!(Journal::open(&r.app,&l).unwrap().load().outcome,Outcome::Restored);
    }
    #[test]
    fn w1_work_dir_reader_refuses_duplicates_or_a_done_borrowed_from_an_older_cycle() {
        for case in ["duplicate-done","done-no-intent","done-old-cycle","done-not-reset","failed-not-reset","other-command-not-reset"]{
            let (_t,r,l,mut j,data,_)=fixture(Outcome::Pg17);let work=Some(serde_json::json!({"workDir":data.backup_id}));
            if case!="done-no-intent"{j.record(Step::P6Cleanup,StepState::Intent,work.clone()).unwrap();}
            if case!="done-no-intent"{j.record(Step::P6Cleanup,StepState::Done,work.clone()).unwrap();}
            if case=="done-old-cycle"{j.record(Step::CmdRestorePg15,StepState::Intent,Some(serde_json::to_value(&data).unwrap())).unwrap();}
            if case=="done-not-reset"{j.record(Step::CmdRestorePg15,StepState::Done,Some(serde_json::to_value(&data).unwrap())).unwrap();}
            if case=="failed-not-reset"{j.record(Step::CmdRestorePg15,StepState::Failed{code:"io_error".into()},None).unwrap();}
            if case=="other-command-not-reset"{j.record(Step::CmdRemoveBackup,StepState::Intent,None).unwrap();}
            let bad_state=if matches!(case,"duplicate-done"|"done-no-intent"|"done-old-cycle"){StepState::Done}else{StepState::Intent};let before=fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap();assert!(j.record(Step::P6Cleanup,bad_state.clone(),work.clone()).is_err(),"{case}");assert_eq!(fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap(),before);
            let mut bad=j.load().clone();bad.records.push(Record{seq:j.next_seq().unwrap(),step:Step::P6Cleanup,state:bad_state,at:chrono::Utc::now().to_rfc3339(),data:work});assert!(bad.validate().is_err(),"{case}");fs::write(r.app.join(".nwui-auto/journal-v1.json"),serde_json::to_vec(&bad).unwrap()).unwrap();assert!(Journal::open(&r.app,&l).is_err(),"{case}");
        }
    }
    #[test]
    fn w1_restore_pg15_data_binds_only_frozen_three_fields_to_intended_selector() {
        let (_t,_r,_l,_j,data,mut selector)=fixture(Outcome::Pg17);
        assert!(RestorePg15Data::bind(&data.backup_id,&data.generation,&uuid::Uuid::new_v4().to_string(),&selector).is_err());selector.backup_id=Some(uuid::Uuid::new_v4().to_string());assert!(RestorePg15Data::bind(&data.backup_id,&data.generation,&data.generation,&selector).is_err());
    }
    #[test]
    fn w1_restore_pg15_finish_kill_child() {
        let Some(root)=std::env::var_os("NWUI_RESTORE_FINISH_ROOT")else{return};let t=PathBuf::from(root);let r=Roots{db:t.join("db"),app:t.join("app"),architecture:"arm64".into()};let l=StorageLocks::acquire(&r.db,&r.app).unwrap();let mut j=Journal::open(&r.app,&l).unwrap();let selector:Selector=serde_json::from_str(&std::env::var("NWUI_RESTORE_FINISH_EXPECTED").unwrap()).unwrap();let intent=j.load().records.iter().rev().find(|r|r.step==Step::CmdRestorePg15).unwrap().data.as_ref().unwrap();let data=RestorePg15Data::bind(intent["backupId"].as_str().unwrap(),intent["generation"].as_str().unwrap(),intent["discardId"].as_str().unwrap(),&selector).unwrap();j.finish_restore_pg15(&r,&l,&data).unwrap();panic!("kill point did not fire");
    }
    #[test]
    fn w1_restore_pg15_finish_real_kills_never_split_done_outcome_and_handoff() {
        for point in ["slot-written","slot-fsynced","journal-renamed","journal-durable"]{let (t,r,l,j,data,selector)=fixture(Outcome::Pg17);drop(j);drop(l);let result=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::journal::restore_finish_tests::w1_restore_pg15_finish_kill_child","--nocapture"]).env("NWUI_RESTORE_FINISH_ROOT",t.path()).env("NWUI_RESTORE_FINISH_EXPECTED",serde_json::to_string(&selector).unwrap()).env("NWUI_JOURNAL_KILL_POINT",point).status().unwrap();assert_eq!(result.signal(),Some(libc::SIGKILL),"{point}");let l=StorageLocks::acquire(&r.db,&r.app).unwrap();let mut j=Journal::open(&r.app,&l).unwrap();let old=matches!(point,"slot-written"|"slot-fsynced");assert_eq!(j.load().outcome,if old{Outcome::Pg17}else{Outcome::Restored},"{point}");assert_eq!(j.load().handoff,if old{Handoff::Done}else{Handoff::None},"{point}");assert_eq!(j.load().records.last().unwrap().state,if old{StepState::Intent}else{StepState::Done},"{point}");if old{j.finish_restore_pg15(&r,&l,&data).unwrap();assert_eq!(j.load().outcome,Outcome::Restored);assert_eq!(j.load().handoff,Handoff::None);}
        }
    }
}

#[cfg(test)]
mod staged_key_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn w1_staged_key_reader_and_commit_require_one_pair_bound_to_the_committed_generation(){
        for case in ["valid","done-only","duplicate-intent","duplicate-done","new-cycle","wrong-generation","extra-field","null","bad-id","failed","no-commit-done","new-generation"]{
            let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");for p in [&db,&app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
            let locks=StorageLocks::acquire(&db,&app).unwrap();let mut j=Journal::begin(&app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();j.ensure_app_pool().unwrap();
            let g=uuid::Uuid::new_v4().to_string();let other=uuid::Uuid::new_v4().to_string();j.record(Step::P5Commit,StepState::Intent,Some(serde_json::json!({"generation":g}))).unwrap();if case!="no-commit-done"{j.record(Step::P5Commit,StepState::Done,None).unwrap();}
            let value=Some(serde_json::json!({"stagedKey":g}));
            if matches!(case,"valid"|"duplicate-intent"|"duplicate-done"|"new-cycle"|"new-generation") {j.record(Step::P6Cleanup,StepState::Intent,value.clone()).unwrap();}
            if matches!(case,"valid"|"duplicate-done"|"new-cycle"){j.record(Step::P6Cleanup,StepState::Done,value.clone()).unwrap();}
            if case=="valid"{Journal::open(&app,&locks).unwrap();continue;}
            if case=="new-cycle"{j.record(Step::CmdRestorePg15,StepState::Intent,Some(serde_json::json!({"backupId":j.load().attempt_id,"generation":g,"discardId":g}))).unwrap();}
            if case=="new-generation"{j.record(Step::P5Commit,StepState::Intent,Some(serde_json::json!({"generation":other}))).unwrap();j.record(Step::P5Commit,StepState::Done,None).unwrap();}
            let state=match case{"done-only"|"duplicate-done"|"new-generation"=>StepState::Done,"failed"=>StepState::Failed{code:"io_error".into()},_=>StepState::Intent};
            let data=Some(match case{"wrong-generation"|"new-generation"=>serde_json::json!({"stagedKey":other}),"extra-field"=>serde_json::json!({"stagedKey":g,"entry":"data_key"}),"null"=>serde_json::json!({"stagedKey":null}),"bad-id"=>serde_json::json!({"stagedKey":"../foreign"}),_=>serde_json::json!({"stagedKey":g})});
            let path=app.join(".nwui-auto/journal-v1.json");let before=fs::read(&path).unwrap();assert!(j.record(Step::P6Cleanup,state.clone(),data.clone()).is_err(),"{case}");assert_eq!(fs::read(&path).unwrap(),before,"{case}");
            let mut bad=j.load().clone();bad.records.push(Record{seq:j.next_seq().unwrap(),step:Step::P6Cleanup,state,at:chrono::Utc::now().to_rfc3339(),data});assert!(bad.validate().is_err(),"{case}");fs::write(&path,serde_json::to_vec(&bad).unwrap()).unwrap();assert!(Journal::open(&app,&locks).is_err(),"{case}");
        }
    }
}
