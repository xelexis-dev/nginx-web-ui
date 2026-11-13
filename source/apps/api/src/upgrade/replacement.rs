//! The single A.18 exception: a journal-authorized restored-15 selector, bound by exact bytes.
use super::{Journal,Step,StepState,StepError,selector::{self,Selector}};
use crate::db_upgrade::layout::Roots;
use std::path::Path;
fn mismatch()->StepError {StepError{code:"pair_mismatch",message:"이전 선택 기록과 복구 권한이 다릅니다. 모든 데이터를 보존했습니다.",detail:None}}
fn exists(path:&Path)->Result<bool,StepError>{match std::fs::symlink_metadata(path){Ok(_)=>Ok(true),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>Ok(false),Err(_)=>Err(mismatch())}}
pub(crate) fn source_id(j:&Journal)->Result<&str,StepError> {
    let header=j.load().records.iter().rev().find(|r|r.step==Step::P1Backup && r.state==StepState::Done).and_then(|r|r.data.as_ref()).and_then(|v|v.get("header")).ok_or_else(mismatch)?;
    if header.get("backupId").and_then(|v|v.as_str())!=Some(j.load().attempt_id.as_str()){return Err(mismatch());}
    header.get("systemIdentifier").and_then(|v|v.as_str()).filter(|id|crate::db_upgrade::state::identifier(id)).ok_or_else(mismatch)
}
pub(crate) fn restored(_j:&Journal,old:&Selector,source_id:&str)->Result<(),StepError> {
    old.validate()?;
    // A restored selector outlives discard deletion and the bounded previous-attempt summaries.
    if old.active_major!=15 || old.operator_reason.as_deref()!=Some("restore-pg15") || old.generation.is_some() || old.system_identifier!=source_id {return Err(mismatch());}Ok(())
}
/// Pre-selector steps may preserve an authoritative restored-15 selector, never a 17 selector.
pub(crate) fn precommit(app:&Path,j:&Journal)->Result<(),StepError> {
    if !j.belongs_to(app){return Err(mismatch());}let path=app.join(".nwui-auto/active-v1.json");if !exists(&path)?{return Ok(());}let old=selector::read_selector(app)?;restored(j,&old,source_id(j)?)
}
pub(crate) fn is_committed(roots:&Roots)->Result<bool,StepError> {
    let path=roots.app.join(".nwui-auto/active-v1.json");if !exists(&path)?{return Ok(false);}Ok(selector::read_selector(&roots.app)?.active_major==17)
}
