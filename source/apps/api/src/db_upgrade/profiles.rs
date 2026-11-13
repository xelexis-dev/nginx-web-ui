//! Released ledger identities only; object/schema and V1/V2 verification remain separate.
use serde::Deserialize;
use sha2::{Digest,Sha256};
use std::collections::BTreeSet;
use crate::upgrade::StepError;
#[derive(Debug,Clone,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(crate) struct Signature {pub tag:String,pub hash:String,pub created_at:i64}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Legacy {release:String,commit:String,signatures:Vec<Signature>}
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Registry {version:u8,profile_id:String,legacy_fixture_sha256:String,ledger_sha256:String,releases:Vec<Release>}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Release {version:String,commit:String}
#[derive(Debug)]
pub(crate) struct LedgerProfile {pub id:String,pub sha256:String,pub releases:Vec<String>,pub signatures:Vec<Signature>}
fn failed()->StepError {StepError {code:"ledger_unknown",message:"데이터 변경 이력이 지원되는 이력과 일치하지 않습니다. 원본을 변경하지 않았습니다. 기존 버전과 변경 이력을 확인하세요.",detail:None}}
fn hex(value:&str,len:usize)->bool {value.len()==len && value.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b))}
fn load_from(registry:&str,legacy:&[u8])->Result<LedgerProfile,StepError> {
    let registry:Registry=serde_json::from_str(registry).map_err(|_|failed())?;
    if registry.version!=1 || registry.profile_id!="legacy-28" || registry.legacy_fixture_sha256!=hex::encode(Sha256::digest(legacy))
        || registry.releases.len()!=12 {return Err(failed());}
    let legacy:Legacy=serde_json::from_slice(legacy).map_err(|_|failed())?;
    if legacy.release!="1.19.11" || legacy.signatures.len()!=28 || registry.releases.last().is_none_or(|r|r.commit!=legacy.commit){return Err(failed());}
    for (index,release) in registry.releases.iter().enumerate() {
        if release.version!=format!("1.19.{index}") || !hex(&release.commit,40){return Err(failed());}
    }
    let mut previous=0;let mut tags=BTreeSet::new();let mut digest=Sha256::new();
    for row in &legacy.signatures {
        if row.created_at<=previous || !hex(&row.hash,64) || row.tag.is_empty() || row.tag.len()>128
            || !row.tag.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'_') || !tags.insert(&row.tag){return Err(failed());}
        digest.update(format!("{}:{}\n",row.created_at,row.hash));previous=row.created_at;
    }
    if registry.ledger_sha256!=hex::encode(digest.finalize()){return Err(failed());}
    Ok(LedgerProfile {id:registry.profile_id,sha256:registry.ledger_sha256,releases:registry.releases.into_iter().map(|r|r.version).collect(),signatures:legacy.signatures})
}
pub(crate) fn released()->Result<LedgerProfile,StepError> {
    load_from(include_str!("fixtures/released-profiles-v1.json"),include_bytes!("legacy-1.19.11.json"))
}
/// Exact registered set, preserving the legacy allowance for duplicate identical ledger rows.
/// This identifies the schema prefix, never the installed product version or object safety.
pub(crate) fn select_source(rows:&[(String,i64)])->Result<LedgerProfile,StepError> {
    let profile=released()?;
    let expected=profile.signatures.iter().map(|s|(s.hash.clone(),s.created_at)).collect::<BTreeSet<_>>();
    let actual=rows.iter().cloned().collect::<BTreeSet<_>>();if actual!=expected{return Err(failed());}Ok(profile)
}
impl LedgerProfile {
    pub fn check_packaged_prefix(&self,packaged:&[(String,i64)])->Result<(),StepError> {
        if packaged.len()<self.signatures.len() || self.signatures.iter().zip(packaged).any(|(expected,actual)|expected.hash!=actual.0 || expected.created_at!=actual.1){return Err(failed());}Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn rows()->Vec<(String,i64)> {released().unwrap().signatures.into_iter().map(|s|(s.hash,s.created_at)).collect()}
    #[test]
    fn w1_profiles_all_released_119_tags_share_one_exact_registered_prefix() {
        let profile=released().unwrap();assert_eq!(profile.releases,(0..12).map(|n|format!("1.19.{n}")).collect::<Vec<_>>());assert_eq!(profile.signatures.len(),28);
        assert_eq!(select_source(&rows()).unwrap().sha256,profile.sha256);assert_eq!(profile.id,"legacy-28");
    }
    #[test]
    fn w1_profiles_accept_only_exact_ledger_values_without_inventing_an_installed_version() {
        let mut actual=rows();actual.reverse();actual.push(actual[0].clone());select_source(&actual).unwrap();
        for change in ["gap","hash","time","extra"] {let mut actual=rows();match change {"gap"=>{actual.remove(3);},"hash"=>actual[3].0="0".repeat(64),"time"=>actual[3].1+=1,_=>actual.push(("f".repeat(64),actual.last().unwrap().1+1))};assert_eq!(select_source(&actual).unwrap_err().code,"ledger_unknown");}
    }
    #[test]
    fn w1_profiles_packaged_migrations_cannot_rewrite_a_released_prefix() {
        let profile=released().unwrap();let mut packaged=rows();packaged.push(("f".repeat(64),packaged.last().unwrap().1+1));profile.check_packaged_prefix(&packaged).unwrap();
        packaged[2].0="0".repeat(64);assert!(profile.check_packaged_prefix(&packaged).is_err());assert!(profile.check_packaged_prefix(&rows()[..27]).is_err());
    }
    #[test]
    fn w1_profiles_registry_binds_the_release_list_and_exact_legacy_fixture() {
        let original=include_str!("fixtures/released-profiles-v1.json");let legacy=include_bytes!("legacy-1.19.11.json");
        for change in ["version","release","fixture","ledger"] {let mut value:serde_json::Value=serde_json::from_str(original).unwrap();match change {"version"=>value["version"]=2.into(),"release"=>value["releases"][0]["version"]="1.19.99".into(),"fixture"=>value["legacyFixtureSha256"]="0".repeat(64).into(),_=>value["ledgerSha256"]="0".repeat(64).into()};assert!(load_from(&value.to_string(),legacy).is_err());}
    }
}
