//! File-only admission before journal repair, permissions or provisioning.
use super::{Branch,Outcome,Step,StepState,StepError};
use crate::db_upgrade::{files,layout::Roots};
use std::{fs::{self,OpenOptions},io::Read,os::unix::fs::{MetadataExt,OpenOptionsExt},path::Path};
pub(crate) const GUIDANCE:&str="1.x(PostgreSQL 15) 데이터입니다. 먼저 2.0.3 이미지에서 이전을 완료한 뒤 최신 2.x로 올리세요. 기존 데이터는 변경하지 않았습니다.";
pub(crate) fn required()->StepError {StepError{code:"requires_2_0_first",message:GUIDANCE,detail:None}}
fn invalid()->StepError {StepError{code:"unsupported_layout",message:"데이터 경로 또는 파일 형식을 확인하지 못했습니다. 기존 데이터는 변경하지 않았습니다.",detail:None}}
fn present(path:&Path)->Result<bool,StepError>{match fs::symlink_metadata(path){Ok(_)=>Ok(true),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>Ok(false),Err(_)=>Err(invalid())}}
fn version(path:&Path)->Result<Option<String>,StepError>{
    if !present(path)?{return Ok(None);}
    files::real_path(path).map_err(|_|invalid())?;
    let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path).map_err(|_|invalid())?;
    let m=file.metadata().map_err(|_|invalid())?;
    if !m.is_file() || m.nlink()!=1 || m.len()==0 || m.len()>16 || m.mode()&0o022!=0 || !(m.uid()==0 || crate::db_upgrade::postgres::postgres_ids().is_ok_and(|(uid,_)|m.uid()==uid)){return Err(invalid());}
    let mut text=String::new();file.take(17).read_to_string(&mut text).map_err(|_|invalid())?;
    if !matches!(text.as_str(),"15"|"15\n"|"17"|"17\n"){return Err(invalid());}Ok(Some(text.trim_end().into()))
}
pub(crate) fn admit(roots:&Roots)->Result<(),StepError>{
    files::real_path(&roots.db).and_then(|_|files::real_path(&roots.app)).map_err(|_|invalid())?;
    let nested_dir=roots.db.join("pgdata");
    if present(&nested_dir)?{files::real_path(&nested_dir).map_err(|_|invalid())?;if !fs::symlink_metadata(&nested_dir).map_err(|_|invalid())?.is_dir(){return Err(invalid());}}
    let root=version(&roots.db.join("PG_VERSION"))?;
    let nested=version(&roots.db.join("pgdata/PG_VERSION"))?;
    if root.is_some() && nested.is_some(){return Err(invalid());}
    let active=if present(&roots.app.join(".nwui-auto/active-v1.json"))?{Some(super::selector::read_selector(&roots.app)?)}else{None};
    let pg15=root.as_deref()==Some("15") || nested.as_deref()==Some("15");
    if active.as_ref().is_some_and(|s|s.active_major==17 && pg15 || s.active_major==15 && present(&roots.db.join("17/data/PG_VERSION")).unwrap_or(true)){
        return Err(StepError{code:"pair_mismatch",message:"선택 기록과 활성 데이터가 다릅니다. 기존 데이터는 변경하지 않았습니다.",detail:None});
    }
    if root.is_some() || nested.is_some(){return if pg15{Err(required())}else{Err(invalid())};}
    if active.is_some_and(|s|s.active_major==15){return Err(required());}
    if present(&roots.app.join(".nwui-auto/journal-v1.json"))?{
        let j=super::journal::read_only(&roots.app)?;
        let restores=j.records.iter().filter(|r|r.step==Step::CmdRestorePg15).cloned().collect::<Vec<_>>();
        if !super::journal::commands_settled(&restores) || matches!(j.outcome,Outcome::Restored|Outcome::RestoreIncomplete|Outcome::Postponed)
            || j.branch==Branch::Upgrade && !j.records.iter().any(|r|r.step==Step::P5Commit && r.state==StepState::Done && r.data.is_none()) {return Err(required());}
    }
    Ok(())
}
