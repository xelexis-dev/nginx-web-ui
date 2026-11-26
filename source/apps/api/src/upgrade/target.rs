//! Open and validate a target directory.
use super::StepError;
use crate::db_upgrade::{files,postgres};
use std::{fs::{File,OpenOptions},os::unix::fs::{MetadataExt,OpenOptionsExt},path::Path};
fn mismatch()->StepError {StepError {code:"pair_mismatch",message:"대상 데이터의 위치와 복구 기록이 다릅니다. 모든 위치를 보존했습니다.",detail:None}}
fn io_error(e:std::io::Error)->StepError {StepError {code:match e.raw_os_error(){Some(libc::ENOSPC|libc::EDQUOT)=>"insufficient_space",Some(libc::EROFS)=>"storage_read_only",Some(libc::EINTR)=>"interrupted",_=>"io_error"},message:"대상 데이터 위치 변경을 완료하지 못했습니다. 복구 기록과 데이터를 보존했습니다.",detail:None}}
pub(super) fn directory(path:&Path)->Result<File,StepError> {
    files::real_path(path).map_err(|_|mismatch())?;let f=OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW).open(path).map_err(io_error)?;let m=f.metadata().map_err(io_error)?;
    if !m.is_dir() || m.mode()&0o022!=0 || !(m.uid()==0 || postgres::postgres_ids().is_ok_and(|(uid,_)|m.uid()==uid)){return Err(mismatch());}Ok(f)
}
