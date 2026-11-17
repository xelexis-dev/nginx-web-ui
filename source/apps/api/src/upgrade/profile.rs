//! Read immutable historical provenance without selecting a current source catalogue.
use super::{Branch,Journal,Step,StepState,StepError};
use serde::{Serialize,Deserialize};
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(crate) struct SourceProof {pub profile_id:String,pub profile_sha256:String}
fn failed()->StepError {StepError{code:"verification_failed",message:"과거 변환의 검증 기록을 확인하지 못했습니다. 기존 데이터를 보존했습니다.",detail:None}}
pub(crate) fn proof(j:&Journal)->Result<SourceProof,StepError>{
    if j.load().branch!=Branch::Upgrade{return Err(failed());}
    let data=j.load().records.iter().rev().find(|r|r.step==Step::P2Preflight&&r.state==StepState::Done).and_then(|r|r.data.as_ref()).ok_or_else(failed)?;
    if data.as_object().is_none_or(|o|o.len()!=3) || !matches!(data.get("mode").and_then(|v|v.as_str()),Some("copy"|"link")){return Err(failed());}
    let id=data.get("profileId").and_then(|v|v.as_str()).filter(|v|!v.is_empty()&&v.len()<=128&&v.bytes().all(|c|c.is_ascii_alphanumeric()||matches!(c,b'.'|b'-'|b'_'))).ok_or_else(failed)?;
    let hash=data.get("profileSha256").and_then(|v|v.as_str()).filter(|v|v.len()==64&&v.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))).ok_or_else(failed)?;
    Ok(SourceProof{profile_id:id.into(),profile_sha256:hash.into()})
}
#[cfg(test)] pub(crate) fn historical_identity()->Result<SourceProof,StepError>{Ok(SourceProof{profile_id:"legacy-28".into(),profile_sha256:"a".repeat(64)})}
#[cfg(test)] pub(crate) fn fixture_done(_mode:())->serde_json::Value{let p=historical_identity().unwrap();serde_json::json!({"profileId":p.profile_id,"profileSha256":p.profile_sha256,"mode":"copy"})}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs,os::unix::fs::PermissionsExt};
    #[test]
    fn historical_proof_validates_shape_without_a_current_catalogue() {
        for case in ["missing","id","hash","extra","latest","mode","historical"] {
            let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");
            for path in [&db,&app]{fs::create_dir(path).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o700)).unwrap();}
            let locks=super::super::StorageLocks::acquire(&db,&app).unwrap();
            let mut j=Journal::begin_upgrade(&app,&locks,super::super::journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)},None).unwrap();
            j.ensure_app_pool().unwrap();
            let mut data=fixture_done(());
            match case {
                "id"=>data["profileId"]="../untrusted".into(),
                "hash"=>data["profileSha256"]="not-a-hash".into(),
                "extra"=>data["unexpected"]=true.into(),
                "mode"=>data["mode"]="clone".into(),
                "historical"=>{data["profileId"]="old-release-unknown-to-this-image".into();data["mode"]="link".into();},
                _=>{},
            }
            if case!="missing"{j.record(Step::P2Preflight,StepState::Done,Some(data)).unwrap();}
            if case=="latest"{j.record(Step::P2Preflight,StepState::Done,None).unwrap();}
            let before=fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap();
            if case=="historical"{assert_eq!(proof(&j).unwrap().profile_id,"old-release-unknown-to-this-image");}else{assert_eq!(proof(&j).unwrap_err().code,"verification_failed");}
            assert_eq!(fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap(),before);
        }
    }
}
