//! A physical selector is committed even when the journal Done/key publication was interrupted.
use super::{Journal,Step,StepState,StepError,StorageLocks,KeySource,generation,selector};
use crate::db_upgrade::{control::Control,key_file::{self,KeyFileError},layout::Roots,paths,postgres};
use std::{path::Path,sync::Arc};
fn mismatch()->StepError {StepError{code:"pair_mismatch",message:"확정된 데이터와 복구 기록을 확인하지 못했습니다. 원본과 대상 모두 보존했습니다.",detail:None}}
fn key_error(e:KeyFileError)->StepError {StepError{code:if matches!(e,KeyFileError::Missing){"key_missing"}else{"key_handle_failed"},message:"확정된 데이터의 기록된 키를 확인하지 못했습니다. 새 키는 만들지 않았습니다.",detail:None}}
pub(crate) async fn resume(roots:Roots,locks:Arc<StorageLocks>,j:Journal)->Result<Journal,StepError> {
    let r=roots.clone();let l=locks.clone();let (j,control)=tokio::task::spawn_blocking(move|| {
        if !l.holds_d() || !l.matches_roots(&r.db,&r.app){return Err(mismatch());}
        super::ready::repair_for_start(&r,&l,&j)?;
        let control=postgres::read_control(17,&r.db.join("17/data")).map_err(|e|StepError{code:e.code,message:e.message,detail:None})?;Ok((j,control))
    }).await.map_err(|_|mismatch())??;
    resume_with_control(roots,locks,j,control).await
}
/// control is from fixed D/17/data under these locks; the public entry always reads pinned PG17.
pub(super) async fn resume_with_control(roots:Roots,locks:Arc<StorageLocks>,j:Journal,control:Control)->Result<Journal,StepError> {
    let r=roots.clone();let l=locks.clone();let (j,id)=tokio::task::spawn_blocking(move|| {
        if !l.holds_d() || !l.matches_roots(&r.db,&r.app) || !j.belongs_to(&r.app){return Err(mismatch());}
        let current=Journal::open(&r.app,&l)?;if serde_json::to_value(current.load()).map_err(|_|mismatch())?!=serde_json::to_value(j.load()).map_err(|_|mismatch())?{return Err(mismatch());}
        control.validate().map_err(|_|mismatch())?;
        let done=j.load().records.iter().any(|record|record.step==Step::P5Commit && record.state==StepState::Done && record.data.is_none());
        // Before Done no service could start; an already-ready PG17 may need crash recovery later.
        if !done {control.require_cold().map_err(|_|mismatch())?;match std::fs::symlink_metadata(r.db.join("17/data/postmaster.pid")){Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},_=>return Err(mismatch())}}
        crate::db_upgrade::layout::control(&r,&|_:&Path|Ok(control.system_identifier.clone())).map_err(|_|mismatch())?;
        let selected=selector::read_selector(&r.app)?;if selected.system_identifier!=control.system_identifier{return Err(mismatch());}
        let canonical=paths::startup_data_key_file(&r.app).map_err(|e|StepError{code:e.code,message:e.message,detail:None})?;let source=j.key_source()?;
        if matches!(&source,KeySource::Staged{generation} if selected.generation.as_deref()!=Some(generation.as_str())){return Err(mismatch());}
        let path=source.path(&r.app,Some(canonical.as_os_str()));let key=key_file::read_key_file(&path,key_file::policy_for(&source,&r.app,&canonical)).map_err(key_error)?;
        let mut j=j;generation::commit(&r,&l,&mut j,&selected,&control.system_identifier,&key)?;Ok((j,control.system_identifier))
    }).await.map_err(|_|mismatch())??;
    // required=true forbids generating a replacement on every committed branch.
    let ensured=super::key_write::ensure_key_locked(roots.clone(),locks.clone(),j,true).await?;
    if ensured.source!=KeySource::Canonical{return Err(mismatch());}
    tokio::task::spawn_blocking(move||{generation::mark_key_required(&roots,&locks,&id,&ensured.key)?;Ok(ensured.journal)}).await.map_err(|_|mismatch())?
}
