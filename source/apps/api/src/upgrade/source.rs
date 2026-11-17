//! Shared PG17 control-state validation.
use super::StepError;
use crate::db_upgrade::control::Control;
fn error(code:&'static str)->StepError {StepError{code,message:"이전 PostgreSQL의 실행 여부를 안전하게 확인하지 못했습니다. 원본을 변경하지 않았습니다.",detail:None}}
pub(super) fn allowed_state(state:&str)->bool {matches!(state,"shut down"|"shut down in recovery"|"in production"|"in crash recovery")}
pub(crate) fn control(control:&Control)->Result<(),StepError> {control.validate().map_err(|_|error("unsupported_layout"))?;if !allowed_state(&control.cluster_state){return Err(error("unsupported_layout"));}Ok(())}

#[cfg(test)]
mod restored_tests {
use super::*;
    #[test]
    fn w1_source_state_accepts_only_frozen_inactive_or_unclean_states() {for s in ["shut down","shut down in recovery","in production","in crash recovery"]{assert!(allowed_state(s));}for s in ["starting up","in archive recovery","unknown",""]{assert!(!allowed_state(s));}}
}
