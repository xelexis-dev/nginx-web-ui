//! N2 owns the S4-prepare journal shape and its fail-closed recovery selection.
use super::{Prepared,KeyReceipt};
use crate::upgrade::{Journal,Step,StepState,StorageSelection};
use serde::{Deserialize,Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Receipt {
    pub(super) version: u8, pub(super) key: KeyReceipt, pub(super) pair_id: String, pub(super) pgdata: PathBuf, pub(super) boot_identity: String, pub(super) active: bool, pub(super) attempt_id: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct LogMode { pub(super) path: PathBuf, pub(super) uid: u32, pub(super) gid: u32, pub(super) mode: u32 }
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PrepareRecord {
    pub(super) prepared: Option<Prepared>, pub(super) log_modes: Vec<LogMode>, pub(super) previous_receipt: Option<Receipt>,
    #[serde(default)] pub(super) publisher_created: bool,
    #[serde(default)] pub(super) candidate: Option<StorageSelection>,
}

/// Match only the full N2 writer shape, including nested fields and explicit nulls.
pub(crate) fn is_prepare_record(data: &serde_json::Value, state: &StepState) -> bool {
    let Ok(record) = serde_json::from_value::<PrepareRecord>(data.clone()) else {
        return false;
    };
    record.candidate.as_ref().is_some_and(|candidate| matches!(candidate.key_binding, crate::upgrade::KeyBinding::Verified { .. }))
        && match state {
            StepState::Intent => record.prepared.is_none(),
            StepState::Done => record.prepared.as_ref().is_some_and(|p| p.payload.is_some() && p.key_receipt.is_some()),
            StepState::Failed { .. } => false,
        }
        && serde_json::to_value(record).is_ok_and(|written| written == *data)
}

pub(super) fn latest_record(records: &[crate::upgrade::journal::Record]) -> Option<PrepareRecord> {
    records.iter().rev().find(|record| record.step == Step::S4Prepare && matches!(record.state, StepState::Intent | StepState::Done))
        .and_then(|record| record.data.clone().and_then(|data| serde_json::from_value(data).ok()))
}
pub fn prepared_from_journal(j: &Journal) -> Option<Prepared> {
    if !j.load().records.iter().any(|record| record.step == Step::S4Prepare) { return None; }
    Some(latest_record(&j.load().records).and_then(|record| record.prepared).unwrap_or(Prepared { payload: None, key_receipt: None }))
}


/// Test-only owner-generated records; callers may append them without inventing N2 JSON.
#[cfg(test)]
pub(crate) fn fixture_prepare_records(candidate:&StorageSelection, attempt_id:&str, first_seq:u64, completed:bool) -> Vec<crate::upgrade::journal::Record> {
    use crate::{publisher::boot::StagedPayload,upgrade::{KeyBinding,journal::Record}};
    let KeyBinding::Verified{pair_id}=&candidate.key_binding else{panic!("fixture requires a verified selection")};
    let key=KeyReceipt{selection_epoch:candidate.epoch,key_sha256:"a".repeat(64)};
    let previous=Receipt{version:1,key:key.clone(),pair_id:pair_id.clone(),pgdata:candidate.pgdata.clone(),boot_identity:"b".repeat(64),active:false,attempt_id:attempt_id.into()};
    let data=|prepared|serde_json::to_value(PrepareRecord{
        prepared,log_modes:vec![LogMode{path:"/var/log/nginx".into(),uid:0,gid:101,mode:0o750}],
        previous_receipt:Some(previous.clone()),publisher_created:true,candidate:Some(candidate.clone()),
    }).unwrap();
    let mut records=vec![Record{seq:first_seq,step:Step::S4Prepare,state:StepState::Intent,at:"2026-10-28T00:00:00Z".into(),data:Some(data(None))}];
    if completed {
        let prepared=Prepared{payload:Some(StagedPayload{id:"00000000-0000-4000-8000-000000000002".into(),manifest_sha256:"c".repeat(64),skipped:vec![]}),key_receipt:Some(key)};
        records.push(Record{seq:first_seq+1,step:Step::S4Prepare,state:StepState::Done,at:"2026-10-28T00:00:01Z".into(),data:Some(data(Some(prepared)))});
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upgrade::{Branch,KeyBinding,Outcome};
    fn selection()->StorageSelection {
        StorageSelection{version:1,epoch:7,outcome:Outcome::Pg17,branch:Branch::Ready,pgdata:"/fixture/pgdata".into(),backup_id:None,schema_profile:"2.0.0".into(),key_binding:KeyBinding::Verified{pair_id:"00000000-0000-4000-8000-000000000001".into()}}
    }
    #[test]
    fn real_intent_and_done_round_trip_the_complete_owner_shape() {
        let records=fixture_prepare_records(&selection(),"attempt",9,true);
        assert_eq!(records.len(),2);assert_eq!(records[0].state,StepState::Intent);assert_eq!(records[1].state,StepState::Done);
        for (index,record) in records.iter().enumerate() {
            let data=record.data.clone().unwrap();let parsed:PrepareRecord=serde_json::from_value(data.clone()).unwrap();
            assert_eq!(serde_json::to_value(&parsed).unwrap(),data);
            assert_eq!(record.seq,9+index as u64);assert!(parsed.publisher_created);assert_eq!(parsed.log_modes[0].mode,0o750);
            assert_eq!(parsed.candidate.unwrap().epoch,7);assert_eq!(data["previousReceipt"]["bootIdentity"],"b".repeat(64));
            if index==0{assert!(parsed.prepared.is_none());}else{let prepared=parsed.prepared.unwrap();assert!(prepared.payload.is_some());assert!(prepared.key_receipt.is_some());}
        }
    }
    #[test]
    fn latest_prepare_record_wins_without_reusing_an_older_completed_payload() {
        let mut records=fixture_prepare_records(&selection(),"attempt",1,true);
        assert!(latest_record(&records).unwrap().prepared.unwrap().payload.is_some());
        records.extend(fixture_prepare_records(&selection(),"attempt",3,false));
        assert!(latest_record(&records).unwrap().prepared.is_none());
        records.push(crate::upgrade::journal::Record{seq:4,step:Step::Y1,state:StepState::Done,at:"fixture".into(),data:None});
        assert!(latest_record(&records).unwrap().prepared.is_none());
    }
    #[test]
    fn malformed_latest_prepare_record_never_falls_back_to_older_valid_data() {
        let mut records=fixture_prepare_records(&selection(),"attempt",1,true);
        records[1].data=Some(serde_json::json!({"wrong":"shape"}));
        assert!(latest_record(&records).is_none());
        records[1].data=None;assert!(latest_record(&records).is_none());
    }
}
