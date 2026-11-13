//! P2 admission and durable source-file provenance; generation writers consume this proof only.
use super::{Branch,Journal,Step,StepState,StepError,verification::{self,SourceProfile,TargetProfile,VerifiedSource},journal::Candidate};
use crate::db_upgrade::{cluster::Snapshot,inspect,profiles};
use serde::{Serialize,Deserialize};
use tokio_postgres::Client;
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(crate) struct SourceProof {pub profile_id:String,pub profile_sha256:String}
pub(crate) struct Admitted {pub snapshot:Snapshot,pub source:SourceProfile,pub target:TargetProfile,verified:VerifiedSource,pub(super) candidate:Candidate,pub(super) ledger:Vec<(String,i64)>}
fn failed()->StepError {StepError{code:"verification_failed",message:"검증된 원본 구조와 복구 기록이 다릅니다. 기존 데이터를 보존했습니다.",detail:None}}
fn map(e:crate::db_upgrade::error::Error)->StepError {StepError{code:e.code,message:e.message,detail:e.detail.map(str::to_owned)}}
fn object_error()->StepError {StepError{code:"object_unsupported",message:"저장된 데이터 구조가 출시된 구조와 다릅니다. 원본을 변경하지 않았습니다.",detail:None}}
/// Inspection must use the private postgres connection, never the application-role pool.
pub(super) async fn require_postgres(app:&Client)->Result<(),StepError> {
    let privileged:bool=app.query_one("SELECT current_user='postgres' AND session_user='postgres'",&[]).await.map_err(|_|failed())?.get(0);
    if !privileged{return Err(failed());}Ok(())
}
/// Strict source inspector errors remain unchanged outside this P2 boundary.
/// On an object refusal collect the actual ordered schema exactly once for bounded diagnosis.
pub(crate) async fn admit(snapshot:crate::db_upgrade::error::Result<Snapshot>,app:&Client,rows:&[(String,i64)],candidate:&Candidate,packaged:&[crate::migrations::Migration])->Result<Admitted,StepError> {
    if let Ok(snapshot)=&snapshot{super::collation::validate(snapshot)?;}
    require_postgres(app).await?;
    let ledger=profiles::select_source(rows)?;let source=verification::source_profile(&ledger)?;let target=verification::target_profile(candidate,&source,packaged)?;
    let checked=match snapshot {Ok(snapshot)=>verification::verify_source(&snapshot,&source).map(|verified|(snapshot,verified)),Err(error)if error.code=="unsupported_layout"=>Err(object_error()),Err(error)=>Err(map(error))};
    match checked {
        Ok((snapshot,verified))=>Ok(Admitted{snapshot,source,target,verified,candidate:candidate.clone(),ledger:rows.to_vec()}),
        Err(mut error)=>{if error.code=="object_unsupported"{let lines=inspect::schema_lines(app).await.map_err(map)?;error.detail=Some(source.difference_detail(&lines));}Err(error)},
    }
}
/// Application admission can precede the stopped-source template0 read; no phase gate may use a partial map.
pub(super) fn require_complete(source:&Admitted)->Result<(),StepError>{
    require_complete_snapshot(&source.snapshot)
}
pub(super) fn require_complete_snapshot(snapshot:&Snapshot)->Result<(),StepError>{
    snapshot.application().map_err(|_|failed())?;
    if snapshot.data.len()!=snapshot.databases.len() || snapshot.collations.len()!=snapshot.databases.len() || snapshot.databases.iter().any(|db|!snapshot.data.contains_key(&db.name) || !snapshot.collations.contains_key(&db.name)){return Err(failed());}Ok(())
}
pub(crate) struct Rehearsed {proof:SourceProof,candidate:Candidate,attempt_id:String,log_name:String}
/// Inspect the running schema-only PG17, but do not publish P2 Done while it or its scratch exists.
pub(crate) async fn verify_rehearsal(j:&Journal,source:&Admitted,app:&Client,packaged:&[(String,i64)])->Result<Rehearsed,StepError>{
    require_complete(source)?;if j.load().candidate!=source.candidate{return Err(failed());}
    let log_name=j.load().records.iter().rev().find(|r|r.step==Step::P2Preflight).filter(|r|r.state==StepState::Intent).and_then(|r|r.data.as_ref()?.get("logName")?.as_str()).ok_or_else(failed)?.to_owned();
    require_postgres(app).await?;verification::v2_rehearsal(app,&source.target,packaged).await?;
    Ok(Rehearsed{proof:SourceProof{profile_id:source.verified.profile_id().into(),profile_sha256:source.verified.profile_sha256().into()},candidate:source.candidate.clone(),attempt_id:j.load().attempt_id.clone(),log_name})
}
/// Call only after private PG17 has been reaped, the real check has passed, and W/data was retired.
pub(crate) fn finish_rehearsal(roots:&crate::db_upgrade::layout::Roots,locks:&super::StorageLocks,j:&mut Journal,source:&Admitted,checked:Rehearsed,mode:crate::db_upgrade::migration::UpgradeMode)->Result<SourceProof,StepError>{
    require_complete(source)?;
    if !locks.holds_d()||!locks.matches_roots(&roots.db,&roots.app)||!j.belongs_to(&roots.app)||j.load().attempt_id!=checked.attempt_id||j.load().candidate!=checked.candidate||source.candidate!=checked.candidate{return Err(failed());}
    let current=Journal::open(&roots.app,locks)?;if serde_json::to_value(current.load()).map_err(|_|failed())?!=serde_json::to_value(j.load()).map_err(|_|failed())?{return Err(failed());}
    super::diagnostics::owned_path(&roots.db,j,&checked.log_name,true)?;
    for name in ["data","target"]{match std::fs::symlink_metadata(roots.db.join(".nwui-auto-work").join(&checked.attempt_id).join(name)){Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},_=>return Err(failed())}}
    persist(j,&checked.proof,mode)?;Ok(checked.proof)
}

#[derive(Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct PreflightDone {profile_id:String,profile_sha256:String,mode:crate::db_upgrade::migration::UpgradeMode}
fn persist(j:&mut Journal,proof:&SourceProof,mode:crate::db_upgrade::migration::UpgradeMode)->Result<(),StepError> {
    if j.load().branch!=Branch::Upgrade || *proof!=packaged_identity()? {return Err(failed());}
    j.record(Step::P2Preflight,StepState::Done,Some(serde_json::to_value(PreflightDone{profile_id:proof.profile_id.clone(),profile_sha256:proof.profile_sha256.clone(),mode}).map_err(|_|failed())?))
}
pub(super) fn packaged_identity()->Result<SourceProof,StepError> {
    let source=verification::source_profile(&profiles::released()?)?;let(id,sha)=source.identity();Ok(SourceProof{profile_id:id.into(),profile_sha256:sha.into()})
}
/// A malformed latest Done is not hidden by an older valid proof; fixture drift refuses on resume.
pub(crate) fn proof(j:&Journal)->Result<SourceProof,StepError>{let done=completed(j)?;Ok(SourceProof{profile_id:done.profile_id,profile_sha256:done.profile_sha256})}
pub(crate) fn mode(j:&Journal)->Result<crate::db_upgrade::migration::UpgradeMode,StepError>{Ok(completed(j)?.mode)}
fn completed(j:&Journal)->Result<PreflightDone,StepError>{
    if j.load().branch!=Branch::Upgrade{return Err(failed());}
    let data=j.load().records.iter().rev().find(|r|r.step==Step::P2Preflight&&r.state==StepState::Done).and_then(|r|r.data.as_ref()).ok_or_else(failed)?;
    let saved:PreflightDone=serde_json::from_value(data.clone()).map_err(|_|failed())?;
    if (SourceProof{profile_id:saved.profile_id.clone(),profile_sha256:saved.profile_sha256.clone()})!=packaged_identity()?{return Err(failed());}Ok(saved)
}
#[cfg(test)]
pub(crate) fn fixture_done(mode:crate::db_upgrade::migration::UpgradeMode)->serde_json::Value{
    let proof=packaged_identity().unwrap();serde_json::to_value(PreflightDone{profile_id:proof.profile_id,profile_sha256:proof.profile_sha256,mode}).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;use std::{fs,os::unix::fs::PermissionsExt};
    #[test]
    fn w1_p2_proof_refuses_missing_forged_drifted_or_malformed_latest_records() {
        for case in ["missing","id","digest","extra","latest","mode","missing-mode","valid"] {
            let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");for p in [&db,&app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}let locks=super::super::StorageLocks::acquire(&db,&app).unwrap();let mut j={let fixture_journal=Journal::begin(&app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};let expected=packaged_identity().unwrap();
            if case!="missing" {let mut data=fixture_done(crate::db_upgrade::migration::UpgradeMode::Copy);match case{"id"=>data["profileId"]="other".into(),"digest"=>data["profileSha256"]="1".repeat(64).into(),"extra"=>data["unexpected"]=true.into(),"mode"=>data["mode"]="clone".into(),"missing-mode"=>{data.as_object_mut().unwrap().remove("mode");},_=>{}}j.record(Step::P2Preflight,StepState::Done,Some(data)).unwrap();if case=="latest"{j.record(Step::P2Preflight,StepState::Done,None).unwrap();}}
            let before=fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap();if case=="valid"{assert_eq!(proof(&j).unwrap(),expected);assert_eq!(mode(&j).unwrap(),crate::db_upgrade::migration::UpgradeMode::Copy);}else{assert_eq!(proof(&j).unwrap_err().code,"verification_failed");}assert_eq!(fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap(),before);
        }
    }
}
