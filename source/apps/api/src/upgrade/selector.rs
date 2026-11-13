//! A.18 immutable provenance and exact-byte authority readers; no target schema inference.
use super::{KeyBinding,StepError,types::canonical_generation};
#[cfg(test)]
use super::{Branch,Outcome,StorageSelection};
use crate::db_upgrade::{files,pair::Pair,state};
#[cfg(test)]
use crate::db_upgrade::paths;
use serde::{Deserialize,Serialize,de::DeserializeOwned};
use sha2::{Digest,Sha256};
use std::{fs::OpenOptions,io::Read,os::unix::fs::{MetadataExt,OpenOptionsExt},path::Path};
#[cfg(test)]
use std::path::PathBuf;
#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="lowercase")]
pub(crate) enum Origin {Auto,Fresh,Manual}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(crate) struct Selector {
    pub version:u8,pub active_major:u8,
    #[serde(deserialize_with="Option::deserialize")]pub generation:Option<String>,
    pub system_identifier:String,pub origin:Origin,
    #[serde(deserialize_with="Option::deserialize")]pub backup_id:Option<String>,
    #[serde(deserialize_with="Option::deserialize")]pub profile_id:Option<String>,
    #[serde(deserialize_with="Option::deserialize")]pub profile_sha256:Option<String>,
    #[serde(deserialize_with="Option::deserialize")]pub manifest_sha256:Option<String>,
    #[serde(deserialize_with="Option::deserialize")]pub operator_reason:Option<String>,
}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(crate) struct GenerationManifest {
    pub version:u8,pub generation:String,pub system_identifier:String,pub origin:Origin,
    #[serde(deserialize_with="Option::deserialize")]pub profile_id:Option<String>,
    #[serde(deserialize_with="Option::deserialize")]pub profile_sha256:Option<String>,
    pub db_pair_sha256:String,pub app_pair_sha256:String,pub key_binding_sha256:String,pub created_at:String,
}
#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="lowercase")]
pub(crate) enum StoredKeySource {Canonical,Staged}
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(crate) struct KeyRecord {pub version:u8,pub key_binding:KeyBinding,pub key_source:StoredKeySource,pub key_sha256:String}
fn mismatch()->StepError {StepError {code:"pair_mismatch",message:"선택된 데이터와 보관된 검증 기록이 일치하지 않습니다. 기존 내용을 보존했습니다.",detail:None}}
pub(crate) fn digest(bytes:&[u8])->String {hex::encode(Sha256::digest(bytes))}
fn hash(value:&str)->bool {value.len()==64 && value.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b))}
fn generation(value:&str)->bool {canonical_generation(value) && uuid::Uuid::parse_str(value).is_ok_and(|u|u.get_version()==Some(uuid::Version::Random))}
fn provenance(origin:Origin,id:Option<&str>,sha:Option<&str>)->bool {
    match origin {
        // Immutable source provenance is not a decision about the current target schema.
        Origin::Auto=>id.is_some_and(|id|!id.is_empty() && id.len()<=128 && id.bytes().all(|b|b.is_ascii_alphanumeric() || matches!(b,b'.'|b'-'|b'_'))) && sha.is_some_and(hash),
        Origin::Fresh|Origin::Manual=>id.is_none() && sha.is_none(),
    }
}
impl Selector {
    pub fn validate(&self)->Result<(),StepError> {
        if self.version!=1 || !state::identifier(&self.system_identifier)
            || !provenance(self.origin,self.profile_id.as_deref(),self.profile_sha256.as_deref()) {return Err(mismatch());}
        match self.origin {
            Origin::Auto if self.backup_id.as_deref().is_some_and(generation)=>{},
            Origin::Fresh|Origin::Manual if self.backup_id.is_none()=>{},
            _=>return Err(mismatch()),
        }
        match self.active_major {
            17 if self.generation.as_deref().is_some_and(generation) && self.manifest_sha256.as_deref().is_some_and(hash) && self.operator_reason.is_none()=>Ok(()),
            15 if self.origin==Origin::Auto && self.generation.is_none() && self.manifest_sha256.is_none() && self.operator_reason.as_deref()==Some("restore-pg15")=>Ok(()),
            _=>Err(mismatch()),
        }
    }
}
impl GenerationManifest {
    pub fn validate(&self)->Result<(),StepError> {
        if self.version!=1 || !generation(&self.generation) || !state::identifier(&self.system_identifier)
            || !provenance(self.origin,self.profile_id.as_deref(),self.profile_sha256.as_deref())
            || ![&self.db_pair_sha256,&self.app_pair_sha256,&self.key_binding_sha256].iter().all(|s|hash(s))
            || chrono::DateTime::parse_from_rfc3339(&self.created_at).is_err() {return Err(mismatch());}Ok(())
    }
}
fn binding_id(binding:&KeyBinding)->Result<&str,StepError> {
    match binding {KeyBinding::Verified {pair_id} if canonical_generation(pair_id)=>Ok(pair_id),_=>Err(mismatch())}
}
/// Hash the exact opened bytes. Never re-serialize a parsed record to validate its digest.
pub(super) fn read_record<T:DeserializeOwned>(path:&Path)->Result<(T,Vec<u8>),StepError> {
    files::real_path(path).map_err(|_|mismatch())?;
    let parent=std::fs::symlink_metadata(path.parent().ok_or_else(mismatch)?).map_err(|_|mismatch())?;
    if !parent.is_dir() || parent.uid()!=0 || parent.mode()&0o7777!=0o700 {return Err(mismatch());}
    let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path).map_err(|_|mismatch())?;let m=file.metadata().map_err(|_|mismatch())?;
    if !m.is_file() || m.uid()!=0 || m.mode()&0o7777!=0o600 || m.nlink()!=1 || m.len()==0 || m.len()>65536 {return Err(mismatch());}
    let mut bytes=Vec::new();file.take(65537).read_to_end(&mut bytes).map_err(|_|mismatch())?;
    if bytes.len()>65536 {return Err(mismatch());}let value=serde_json::from_slice(&bytes).map_err(|_|mismatch())?;Ok((value,bytes))
}
pub(crate) fn read_selector(app_root:&Path)->Result<Selector,StepError> {
    let (selector,_): (Selector,_) = read_record(&app_root.join(".nwui-auto/active-v1.json"))?;selector.validate()?;Ok(selector)
}
pub(super) fn same_pair_except_flag(snapshot:&Pair,live:&Pair)->Result<(),StepError> {
    snapshot.validate().and_then(|_|live.validate()).map_err(|_|mismatch())?;
    let mut live=live.clone();live.encryption_key_required=snapshot.encryption_key_required;
    if live!=*snapshot {return Err(mismatch());}Ok(())
}
pub(crate) struct SelectedGeneration {pub selector:Selector,pub manifest:GenerationManifest,pub db_pair:Pair,pub app_pair:Pair,pub key_record:KeyRecord}
impl SelectedGeneration {
    /// This boot's agreement is required; the stored keyBinding is never reused as proof.
    #[cfg(test)]
    pub fn descriptor(&self,epoch:u64,branch:Branch,agreement:KeyBinding)->Result<StorageSelection,StepError> {
        if epoch==0 || binding_id(&agreement)?!=self.db_pair.pair_id {return Err(mismatch());}
        Ok(StorageSelection {version:1,epoch,outcome:Outcome::Pg17,branch,pgdata:PathBuf::from(paths::PGDATA),backup_id:self.selector.backup_id.clone(),schema_profile:self.manifest.profile_id.clone().unwrap_or_else(||"none".into()),key_binding:agreement})
    }
}
/// actual_id is read from fixed D/17/data by the caller, never from socket/JSON metadata.
/// key_bytes are the one-open validated canonical file bytes from this boot's K step.
pub(super) fn read_prepared(app_root:&Path,architecture:&str,selector:Selector,actual_id:&str,key_bytes:&[u8])->Result<SelectedGeneration,StepError> {
    if selector.system_identifier!=actual_id{return Err(mismatch());}
    read_authority(app_root,architecture,selector,key_bytes)
}
/// Validate trusted record authority before repair, without asserting any physical control ID.
fn read_authority(app_root:&Path,architecture:&str,selector:Selector,key_bytes:&[u8])->Result<SelectedGeneration,StepError>{
    selector.validate()?;if selector.active_major!=17{return Err(mismatch());}
    let expected_id=selector.system_identifier.as_str();
    let root=app_root.join(".nwui-auto/gen").join(selector.generation.as_deref().ok_or_else(mismatch)?);
    let (manifest,manifest_bytes):(GenerationManifest,_)=read_record(&root.join("manifest.json"))?;
    manifest.validate()?;
    if selector.manifest_sha256.as_deref()!=Some(digest(&manifest_bytes).as_str()) || selector.generation.as_deref()!=Some(manifest.generation.as_str())
        || manifest.system_identifier!=selector.system_identifier || manifest.origin!=selector.origin || manifest.profile_id!=selector.profile_id || manifest.profile_sha256!=selector.profile_sha256 {return Err(mismatch());}
    let (db_pair,db_bytes):(Pair,_)=read_record(&root.join("db-pair.json"))?;
    let (app_pair,app_bytes):(Pair,_)=read_record(&root.join("app-pair.json"))?;
    let (key_record,key_bytes_record):(KeyRecord,_)=read_record(&root.join("key-binding.json"))?;
    if digest(&db_bytes)!=manifest.db_pair_sha256 || digest(&app_bytes)!=manifest.app_pair_sha256 || digest(&key_bytes_record)!=manifest.key_binding_sha256 {return Err(mismatch());}
    if !db_pair.ready || !app_pair.ready || db_pair.system_identifier.as_deref()!=Some(expected_id) || app_pair.system_identifier.as_deref()!=Some(expected_id)
        || db_pair.architecture!=architecture || app_pair.architecture!=architecture || key_record.version!=1 || !hash(&key_record.key_sha256)
        || key_record.key_sha256!=digest(key_bytes) || binding_id(&key_record.key_binding)?!=db_pair.pair_id {return Err(mismatch());}
    same_pair_except_flag(&db_pair,&app_pair)?;
    Ok(SelectedGeneration {selector,manifest,db_pair,app_pair,key_record})
}
pub(crate) fn load_generation(db_root:&Path,app_root:&Path,architecture:&str,selector:Selector,actual_id:&str,key_bytes:&[u8])->Result<SelectedGeneration,StepError> {
    if selector.system_identifier!=actual_id{return Err(mismatch());}
    load_authority(db_root,app_root,architecture,selector,key_bytes)
}
/// Read-only authority admission; callers still cross-check the real control ID after repair.
pub(super) fn load_authority(db_root:&Path,app_root:&Path,architecture:&str,selector:Selector,key_bytes:&[u8])->Result<SelectedGeneration,StepError>{
    let selected=read_authority(app_root,architecture,selector,key_bytes)?;
    if selected.selector.origin==Origin::Auto {
        match std::fs::symlink_metadata(db_root.join(".nwui")) {
            Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},
            _=>return Err(StepError {code:"unsupported_layout",message:"자동 이전 데이터에 외래 D/.nwui 항목이 있습니다. 기존 내용을 보존했습니다.",detail:None}),
        }
    } else {
        let (live_db,_):(Pair,_)=read_record(&db_root.join(".nwui/pair-v1.json"))?;
        same_pair_except_flag(&selected.db_pair,&live_db)?;
    }
    let (live_app,_):(Pair,_)=read_record(&app_root.join(".nwui/pair-v1.json"))?;
    same_pair_except_flag(&selected.app_pair,&live_app)?;
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs,os::unix::fs::PermissionsExt};
    const SYSTEM:&str="18446744073709551614";
    fn write<T:Serialize>(path:&Path,value:&T)->Vec<u8> {let bytes=serde_json::to_vec(value).unwrap();fs::write(path,&bytes).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o600)).unwrap();bytes}
    fn fixture(origin:Origin)->(tempfile::TempDir,PathBuf,PathBuf,Selector,Vec<u8>) {
        let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");let generation_id=uuid::Uuid::new_v4().to_string();
        for p in [&db,&app,&app.join(".nwui"),&app.join(".nwui-auto"),&app.join(".nwui-auto/gen"),&app.join(".nwui-auto/gen").join(&generation_id)] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let mut pair=Pair::fresh("arm64");pair.ready=true;pair.fresh_step=None;pair.system_identifier=Some(SYSTEM.into());
        if origin!=Origin::Auto {fs::create_dir(db.join(".nwui")).unwrap();fs::set_permissions(db.join(".nwui"),fs::Permissions::from_mode(0o700)).unwrap();write(&db.join(".nwui/pair-v1.json"),&pair);}
        write(&app.join(".nwui/pair-v1.json"),&pair);
        let root=app.join(".nwui-auto/gen").join(&generation_id);let db_bytes=write(&root.join("db-pair.json"),&pair);let app_bytes=write(&root.join("app-pair.json"),&pair);
        let key_bytes=b"01".repeat(32);let key=KeyRecord {version:1,key_binding:KeyBinding::Verified {pair_id:pair.pair_id},key_source:StoredKeySource::Canonical,key_sha256:digest(&key_bytes)};
        let key_bytes_record=write(&root.join("key-binding.json"),&key);
        let (profile_id,profile_sha256,backup_id):(Option<String>,Option<String>,Option<String>)=if origin==Origin::Auto {(Some("fixture-source-profile".into()),Some("1".repeat(64)),Some(uuid::Uuid::new_v4().to_string()))}else{(None,None,None)};
        let manifest=GenerationManifest {version:1,generation:generation_id.clone(),system_identifier:SYSTEM.into(),origin,profile_id:profile_id.clone(),profile_sha256:profile_sha256.clone(),db_pair_sha256:digest(&db_bytes),app_pair_sha256:digest(&app_bytes),key_binding_sha256:digest(&key_bytes_record),created_at:"2026-10-28T00:00:00Z".into()};
        let bytes=write(&root.join("manifest.json"),&manifest);let selector=Selector {version:1,active_major:17,generation:Some(generation_id),system_identifier:SYSTEM.into(),origin,backup_id,profile_id,profile_sha256,manifest_sha256:Some(digest(&bytes)),operator_reason:None};
        write(&app.join(".nwui-auto/active-v1.json"),&selector);(t,db,app,selector,key_bytes)
    }
    #[test]
    fn w1_selector_all_origins_roundtrip_large_decimal_ids_and_derive_a_fresh_descriptor() {
        for origin in [Origin::Auto,Origin::Fresh,Origin::Manual] {let (_t,db,app,selector,key)=fixture(origin);assert_eq!(read_selector(&app).unwrap(),selector);
            let selected=load_generation(&db,&app,"arm64",selector,SYSTEM,&key).unwrap();let binding=KeyBinding::Verified {pair_id:selected.db_pair.pair_id.clone()};
            let descriptor=selected.descriptor(7,Branch::Ready,binding).unwrap();assert_eq!(descriptor.pgdata,Path::new(paths::PGDATA));assert_eq!(descriptor.schema_profile,if origin==Origin::Auto{"fixture-source-profile"}else{"none"});
            assert!(selected.descriptor(8,Branch::Ready,KeyBinding::Unverified).is_err());
        }
    }
    #[test]
    fn w1_selector_hashes_exact_file_bytes_instead_of_a_reserialized_value() {
        for file in ["manifest.json","db-pair.json","app-pair.json","key-binding.json"] {let (_t,db,app,selector,key)=fixture(Origin::Auto);let path=app.join(".nwui-auto/gen").join(selector.generation.as_ref().unwrap()).join(file);
            let mut bytes=fs::read(&path).unwrap();bytes.push(b' ');fs::write(path,bytes).unwrap();assert_eq!(load_generation(&db,&app,"arm64",selector,SYSTEM,&key).err().unwrap().code,"pair_mismatch");
        }
    }
    #[test]
    fn w1_selector_live_pairs_can_change_only_the_monotonic_encryption_flag() {
        for change in ["flag","pair","identity","architecture"] {let (_t,db,app,selector,key)=fixture(Origin::Fresh);let (mut pair,_):(Pair,_)=read_record(&db.join(".nwui/pair-v1.json")).unwrap();
            match change {"flag"=>pair.encryption_key_required=true,"pair"=>pair.pair_id=uuid::Uuid::new_v4().to_string(),"identity"=>pair.system_identifier=Some("123456".into()),_=>pair.architecture="amd64".into()};write(&db.join(".nwui/pair-v1.json"),&pair);
            let result=load_generation(&db,&app,"arm64",selector,SYSTEM,&key);assert_eq!(result.is_ok(),change=="flag");
        }
    }
    #[test]
    fn w1_selector_refuses_key_byte_or_actual_control_identity_changes() {
        let (_t,db,app,selector,mut key)=fixture(Origin::Auto);assert!(load_generation(&db,&app,"arm64",selector.clone(),"123456",&key).is_err());key.push(b'\n');assert!(load_generation(&db,&app,"arm64",selector,SYSTEM,&key).is_err());
    }
    #[test]
    fn w1_selector_auto_uses_a_only_and_refuses_any_d_mirror() {
        let (_t,db,app,selector,key)=fixture(Origin::Auto);assert!(!db.join(".nwui").exists());
        load_generation(&db,&app,"arm64",selector.clone(),SYSTEM,&key).unwrap();assert!(!db.join(".nwui").exists());
        fs::create_dir(db.join(".nwui")).unwrap();assert_eq!(load_generation(&db,&app,"arm64",selector,SYSTEM,&key).err().unwrap().code,"unsupported_layout");assert!(db.join(".nwui").exists());
    }
    #[test]
    fn w1_selector_auto_authority_survives_d_only_ownership_changes() {
        let (_t,db,app,selector,key)=fixture(Origin::Auto);let data=db.join("17/data");fs::create_dir_all(&data).unwrap();fs::write(data.join("control-id"),SYSTEM).unwrap();
        for path in [&db,&db.join("17"),&data,&data.join("control-id")] {std::os::unix::fs::chown(path,Some(100),Some(102)).unwrap();}
        let actual=fs::read_to_string(data.join("control-id")).unwrap();load_generation(&db,&app,"arm64",selector,SYSTEM,&key).unwrap();assert_eq!(actual,SYSTEM);
        assert_eq!(fs::metadata(app.join(".nwui/pair-v1.json")).unwrap().uid(),0);assert!(!db.join(".nwui").exists());
    }
    #[test]
    fn w1_selector_manual_and_fresh_pairs_keep_strict_root_ownership_on_d() {
        for origin in [Origin::Manual,Origin::Fresh] {let (_t,db,app,selector,key)=fixture(origin);std::os::unix::fs::chown(db.join(".nwui/pair-v1.json"),Some(100),None).unwrap();assert!(load_generation(&db,&app,"arm64",selector,SYSTEM,&key).is_err());}
    }
    #[test]
    fn w1_selector_pg15_is_only_an_explicit_operator_restore_without_a_generation() {
        let (_t,_db,_app,mut selector,_key)=fixture(Origin::Auto);selector.active_major=15;selector.generation=None;selector.manifest_sha256=None;
        assert!(selector.validate().is_err());selector.operator_reason=Some("restore-pg15".into());selector.validate().unwrap();
        let value=serde_json::to_value(&selector).unwrap();assert_eq!(value["systemIdentifier"],SYSTEM);assert_eq!(serde_json::from_value::<Selector>(value).unwrap(),selector);
        selector.generation=Some(uuid::Uuid::new_v4().to_string());assert!(selector.validate().is_err());
    }
}
