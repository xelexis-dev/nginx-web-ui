//! I1 consumes the runner's trusted selection without reopening its storage locks.
use crate::{config::RuntimeConfig, db_upgrade::{key_file::{self, KeyFileError}, paths::APPROOT}, state::AppState, upgrade::{KeyBinding, KeySource, Outcome, StepError, StorageSelection}};
use std::{ffi::OsStr, fs, path::Path};

const SELECTION: &str = "/run/nwui-storage.json";
fn invalid(code: &'static str) -> StepError {
    StepError {code, message: match code {
        "key_missing"=>"선택된 저장소의 암호화 키가 없습니다. 기존 키를 복원한 뒤 다시 시작하세요.",
        "key_handle_failed"=>"선택된 저장소의 암호화 키를 검증하지 못했습니다.",
        "key_path_invalid"=>"DATA_KEY_FILE은 runner가 선택한 비어 있지 않은 절대 경로와 같아야 합니다.",
        "key_unverified"=>"선택된 저장소의 암호화 키 일치를 확인하지 못했습니다.",
        _=>"runner가 기록한 저장소 선택을 검증하지 못했습니다.",
    }, detail:None}
}
fn validate_selection(selection: &StorageSelection) -> Result<(),StepError> {
    if selection.version!=1 || selection.outcome!=Outcome::Pg17 {return Err(invalid("verification_failed"));}
    if !matches!(selection.key_binding,KeyBinding::Verified{..}) {return Err(invalid("key_unverified"));}
    Ok(())
}
fn load_with(
    path: &Path, configured_key: &Path, raw_override: Option<&OsStr>,
    read: impl FnOnce() -> Result<StorageSelection,StepError>,
) -> Result<Option<StorageSelection>,StepError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(None),
        Err(_)=>return Err(invalid("verification_failed")),
        Ok(_)=>{},
    }
    // A present file never downgrades to the legacy classifier, including reader errors.
    let selection=read()?;
    validate_selection(&selection)?;
    if raw_override.is_some_and(OsStr::is_empty) || !configured_key.is_absolute() {return Err(invalid("key_path_invalid"));}
    let canonical=KeySource::Canonical.path(Path::new(APPROOT),raw_override);
    if configured_key!=canonical {return Err(invalid("key_path_invalid"));}
    key_file::read_key_file(&canonical,key_file::canonical_policy(Path::new(APPROOT),&canonical))
        .map_err(|error|invalid(if error==KeyFileError::Missing {"key_missing"}else{"key_handle_failed"}))?;
    Ok(Some(selection))
}

fn verify_with(
    path:&Path, configured_key:&Path, raw_override:Option<&OsStr>,
    read:impl FnOnce()->Result<StorageSelection,StepError>,
) -> Result<StorageSelection,StepError> {
    load_with(path,configured_key,raw_override,read)?.ok_or_else(||invalid("verification_failed"))
}

/// I2 boot is a read-only check: no pool, storage classification, key binding or publication.
pub async fn verify_boot(config:&RuntimeConfig) -> Result<(),StepError> {
    let key=config.data_key_file.clone();let raw=std::env::var_os("DATA_KEY_FILE");
    crate::control::internal_targets::blocking(move||verify_with(Path::new(SELECTION),&key,raw.as_deref(),crate::upgrade::read_selection))
        .await.map_err(|_|invalid("verification_failed"))??;
    Ok(())
}

fn load_runtime_handles(key:&Path, receipt:&Path, read:impl FnOnce()->Result<StorageSelection,StepError>) -> Result<(StorageSelection,crate::crypto::SecretBox),StepError> {
    let selection=read().map_err(|_|invalid("key_handle_failed"))?;
    validate_selection(&selection).map_err(|_|invalid("key_handle_failed"))?;
    let receipt:super::Receipt=super::handles::read_private_json(receipt).map_err(|_|invalid("key_handle_failed"))?;
    receipt.validate(&selection,true).map_err(|_|invalid("key_handle_failed"))?;
    let bytes=super::handles::read_runtime_key(key).map_err(|_|invalid("key_handle_failed"))?;
    receipt.validate_key(&bytes).map_err(|_|invalid("key_handle_failed"))?;
    let content=std::str::from_utf8(&bytes).map_err(|_|invalid("key_handle_failed"))?;
    let secrets=crate::crypto::parse_key_file(key,content).map_err(|_|invalid("key_handle_failed"))?;
    Ok((selection,secrets))
}

/// I2 verifies and parses the same single runtime-key read; it never runs a root D1 probe.
/// The I2 switch requires the forced runtime path; it never downgrades to I1 startup.
pub async fn load_for_serve(config:&RuntimeConfig) -> Result<(StorageSelection,crate::crypto::SecretBox),StepError> {
    let runtime=Path::new(super::RUNTIME_KEY);
    if config.data_key_file != runtime {return Err(invalid("key_handle_failed"));}
    let key=config.data_key_file.clone();
    let (selection,secrets)=crate::control::internal_targets::blocking(move||load_runtime_handles(&key,Path::new(super::RECEIPT),crate::upgrade::read_selection))
        .await.map_err(|_|invalid("key_handle_failed"))??;
    Ok((selection,secrets))
}

pub fn install(state: &AppState, selection: StorageSelection) -> Result<(),StepError> {
    validate_selection(&selection)?;
    state.runtime.storage_selection.set(selection).map_err(|_|invalid("verification_failed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::{ffi::OsStringExt,fs::PermissionsExt};
    fn fixture() -> (tempfile::TempDir,RuntimeConfig,std::path::PathBuf) {
        let temp=tempfile::tempdir().unwrap();fs::set_permissions(temp.path(),fs::Permissions::from_mode(0o700)).unwrap();
        let key=temp.path().join("canonical-key");fs::write(&key,"AB".repeat(32)).unwrap();fs::set_permissions(&key,fs::Permissions::from_mode(0o600)).unwrap();
        let config=RuntimeConfig::read(|name|match name {
            "DATABASE_URL"=>Some("postgres://fixture@127.0.0.1/test".into()),"AUTH_SECRET"=>Some("fixture-only".into()),
            "DATA_KEY_FILE"=>Some(key.display().to_string()),_=>None,
        }).unwrap();
        let descriptor=temp.path().join("selection.json");
        fs::write(&descriptor,serde_json::to_vec(&json!({"version":1,"epoch":93,"outcome":"pg17","branch":"ready","pgdata":"/var/lib/postgresql/data/17/data","backupId":null,"schemaProfile":"2.0.0","keyBinding":{"state":"verified","pairId":"fixture-pair"}})).unwrap()).unwrap();
        (temp,config,descriptor)
    }
    fn read_fixture(path:&Path) -> Result<StorageSelection,StepError> {
        serde_json::from_slice(&fs::read(path).unwrap()).map_err(|_|invalid("verification_failed"))
    }
    #[test]
    fn absent_descriptor_preserves_the_legacy_branch_without_reading_a_key_or_selection() {
        let (temp,config,_)=fixture();
        assert!(load_with(&temp.path().join("absent"),&config.data_key_file,None,||panic!("no descriptor means no trusted reader call")).unwrap().is_none());
    }
    #[test]
    fn present_valid_descriptor_binds_pair_epoch_pgdata_once_without_classifying_storage() {
        let (_temp,config,path)=fixture();
        let selected=load_with(&path,&config.data_key_file,Some(config.data_key_file.as_os_str()),||read_fixture(&path)).unwrap().unwrap();
        // No PG tree exists in this test; a legacy storage classifier would refuse it.
        let before=fs::read(&config.data_key_file).unwrap();let key=config.data_key_file.clone();
        let state=AppState::new(config).unwrap();install(&state,selected.clone()).unwrap();
        assert_eq!(fs::read(key).unwrap(),before);
        let bound=state.runtime.storage_selection.get().unwrap();
        assert_eq!(bound.epoch,93);assert_eq!(bound.pgdata,Path::new("/var/lib/postgresql/data/17/data"));
        assert!(matches!(&bound.key_binding,KeyBinding::Verified{pair_id} if pair_id=="fixture-pair"));
        assert!(install(&state,selected).is_err());
    }
    #[test]
    fn present_bad_or_untrusted_descriptor_never_returns_the_legacy_branch() {
        let (_temp,config,path)=fixture();
        for value in [json!({"version":2}),json!({"outcome":"upgrading"}),json!({"keyBinding":{"state":"unverified"}})] {
            let mut selected=read_fixture(&path).unwrap();
            if value.get("version").is_some(){selected.version=2;}
            if value.get("outcome").is_some(){selected.outcome=Outcome::Upgrading;}
            if value.get("keyBinding").is_some(){selected.key_binding=KeyBinding::Unverified;}
            assert!(load_with(&path,&config.data_key_file,Some(config.data_key_file.as_os_str()),||Ok(selected)).is_err());
        }
        for code in ["verification_failed","io_error"] {
            assert_eq!(load_with(&path,&config.data_key_file,None,||Err(invalid(code))).unwrap_err().code,code);
        }
        fs::write(&path,"invalid JSON").unwrap();
        assert!(load_with(&path,&config.data_key_file,Some(config.data_key_file.as_os_str()),||read_fixture(&path)).is_err());
    }
    #[test]
    fn missing_canonical_key_refuses_before_state_construction_and_never_creates_it() {
        let (temp,_config,path)=fixture();let missing=temp.path().join("missing-key");
        assert_eq!(load_with(&path,&missing,Some(missing.as_os_str()),||read_fixture(&path)).unwrap_err().code,"key_missing");
        assert!(!missing.exists());
    }
    #[test]
    fn raw_empty_and_non_utf8_overrides_cannot_select_a_second_key() {
        let (_temp,config,path)=fixture();
        let read_config = |raw: Option<String>| RuntimeConfig::read(|name|match name {
            "DATABASE_URL"=>Some("postgres://fixture@127.0.0.1/test".into()),
            "AUTH_SECRET"=>Some("fixture-only".into()),"DATA_KEY_FILE"=>raw.clone(),_=>None,
        }).unwrap();
        let empty=read_config(Some(String::new()));assert!(empty.data_key_file.as_os_str().is_empty());
        assert_eq!(load_with(&path,&empty.data_key_file,Some(OsStr::new("")),||read_fixture(&path)).unwrap_err().code,"key_path_invalid");
        let non_utf8=std::ffi::OsString::from_vec(b"/tmp/non-utf8-\xff-key".to_vec());
        let fallback=read_config(non_utf8.clone().into_string().ok());
        assert_eq!(fallback.data_key_file,KeySource::Canonical.path(Path::new(APPROOT),None));
        assert_eq!(load_with(&path,&fallback.data_key_file,Some(non_utf8.as_os_str()),||read_fixture(&path)).unwrap_err().code,"key_path_invalid");
        assert_eq!(load_with(&path,&config.data_key_file,None,||read_fixture(&path)).unwrap_err().code,"key_path_invalid");
    }

    fn runtime_fixture()->(tempfile::TempDir,StorageSelection,std::path::PathBuf,std::path::PathBuf) {
        let (_old,_config,descriptor)=fixture();let selection=read_fixture(&descriptor).unwrap();
        let temp=tempfile::tempdir_in("/run").unwrap();fs::set_permissions(temp.path(),fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(temp.path().join("selection.json"),serde_json::to_vec(&selection).unwrap()).unwrap();
        let dir=temp.path().join("private");super::super::handles::ensure_directory(&dir,0,10001,0o750).unwrap();
        let key=dir.join("data_key");let receipt=dir.join("receipt.json");
        let bytes=format!("\n{}\n","AB".repeat(32)).into_bytes();
        super::super::handles::atomic_write(&key,&bytes,10001,0o440).unwrap();
        let mut attestation=super::super::Receipt::new(&selection,&bytes,"attempt").unwrap();attestation.active=true;
        super::super::handles::atomic_write(&receipt,&serde_json::to_vec(&attestation).unwrap(),10001,0o440).unwrap();
        (temp,selection,key,receipt)
    }

    #[test]
    fn runtime_receipt_rejects_every_binding_mismatch_without_mutation() {
        let (_temp,selection,key,path)=runtime_fixture();
        load_runtime_handles(&key,&path,||Ok(selection.clone())).unwrap();
        let original=fs::read(&path).unwrap();let key_before=fs::read(&key).unwrap();
        for (field,value) in [("version",json!(2)),("active",json!(false)),("bootIdentity",json!("wrong")),("pairId",json!("wrong")),("pgdata",json!("/wrong")),("unknown",json!(true))] {
            let mut receipt:serde_json::Value=serde_json::from_slice(&original).unwrap();receipt[field]=value;
            super::super::handles::atomic_write(&path,&serde_json::to_vec(&receipt).unwrap(),10001,0o440).unwrap();
            let before=fs::read(&path).unwrap();
            assert_eq!(load_runtime_handles(&key,&path,||Ok(selection.clone())).err().unwrap().code,"key_handle_failed","{field}");
            assert_eq!(fs::read(&path).unwrap(),before);assert_eq!(fs::read(&key).unwrap(),key_before);
        }
        for (field,value) in [("selectionEpoch",json!(selection.epoch+1)),("keySha256",json!("wrong")),("unknown",json!(true))] {
            let mut receipt:serde_json::Value=serde_json::from_slice(&original).unwrap();receipt["key"][field]=value;
            super::super::handles::atomic_write(&path,&serde_json::to_vec(&receipt).unwrap(),10001,0o440).unwrap();
            assert_eq!(load_runtime_handles(&key,&path,||Ok(selection.clone())).err().unwrap().code,"key_handle_failed");
        }
        super::super::handles::atomic_write(&path,&original,10001,0o440).unwrap();
        for bad in [json!({}),json!({"version":2}),json!({"outcome":"upgrading"})] {
            assert_eq!(load_runtime_handles(&key,&path,||serde_json::from_value(bad).map_err(|_|invalid("verification_failed"))).err().unwrap().code,"key_handle_failed");
        }
        let mut unverified=selection.clone();unverified.key_binding=KeyBinding::Unverified;
        assert_eq!(load_runtime_handles(&key,&path,||Ok(unverified)).err().unwrap().code,"key_handle_failed");
        super::super::handles::atomic_write(&key,b"not a valid hex key",10001,0o440).unwrap();
        let mut attestation=super::super::Receipt::new(&selection,b"not a valid hex key","attempt").unwrap();attestation.active=true;
        super::super::handles::atomic_write(&path,&serde_json::to_vec(&attestation).unwrap(),10001,0o440).unwrap();
        assert_eq!(load_runtime_handles(&key,&path,||Ok(selection)).err().unwrap().code,"key_handle_failed");
    }

    #[test]
    fn runtime_handles_child() {
        let Some(root)=std::env::var_os("NWUI_N2_RUNTIME_TEST_ROOT") else{return;};let root=Path::new(&root);
        let key=root.join("private/data_key");let receipt=root.join("private/receipt.json");
        if std::env::var_os("NWUI_N2_RUNTIME_TEST_KILL").is_some() {
            super::super::handles::atomic_write(&key,"CD".repeat(32).as_bytes(),10001,0o440).unwrap();
            unsafe{libc::kill(libc::getpid(),libc::SIGKILL);}
            panic!("SIGKILL must terminate the writer before its receipt write");
        }
        assert_eq!(unsafe{libc::geteuid()},10001);
        let selection=read_fixture(&root.join("selection.json")).unwrap();
        load_runtime_handles(&key,&receipt,||Ok(selection)).unwrap();
        for file in [&key,&receipt] {
            assert!(fs::OpenOptions::new().write(true).open(file).is_err());
            assert!(fs::rename(file,file.with_extension("moved")).is_err());
            assert!(fs::remove_file(file).is_err());
        }
        assert!(fs::write(root.join("private/created"),b"x").is_err());
    }

    #[test]
    fn actual_api_uid_reads_both_handles_but_cannot_replace_or_modify_them() {
        use std::os::unix::process::CommandExt;
        let (temp,_selection,key,receipt)=runtime_fixture();let before=(fs::read(&key).unwrap(),fs::read(&receipt).unwrap());
        let mut command=std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact","nonroot::selection::tests::runtime_handles_child","--nocapture"]).env("NWUI_N2_RUNTIME_TEST_ROOT",temp.path()).env_remove("NWUI_N2_RUNTIME_TEST_KILL");
        unsafe{command.pre_exec(||{
            if libc::setgroups(0,std::ptr::null())==-1 || libc::setgid(10001)==-1 || libc::setuid(10001)==-1{return Err(std::io::Error::last_os_error());}Ok(())
        });}
        let output=command.output().unwrap();assert!(output.status.success(),"{output:?}");
        assert_eq!((fs::read(key).unwrap(),fs::read(receipt).unwrap()),before);
    }

    #[test]
    fn kill_between_key_and_receipt_refuses_until_the_next_writer_completes() {
        use std::os::unix::process::ExitStatusExt;
        let (temp,selection,key,receipt)=runtime_fixture();let before=fs::read(&receipt).unwrap();
        let status=std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","nonroot::selection::tests::runtime_handles_child","--nocapture"])
            .env("NWUI_N2_RUNTIME_TEST_ROOT",temp.path()).env("NWUI_N2_RUNTIME_TEST_KILL","1").status().unwrap();
        assert_eq!(status.signal(),Some(libc::SIGKILL));assert_eq!(fs::read(&receipt).unwrap(),before);
        assert_eq!(load_runtime_handles(&key,&receipt,||Ok(selection.clone())).err().unwrap().code,"key_handle_failed");
        let bytes="CD".repeat(32).into_bytes();
        super::super::handles::atomic_write(&key,&bytes,10001,0o440).unwrap();
        let mut attestation=super::super::Receipt::new(&selection,&bytes,"attempt").unwrap();attestation.active=true;
        super::super::handles::atomic_write(&receipt,&serde_json::to_vec(&attestation).unwrap(),10001,0o440).unwrap();
        load_runtime_handles(&key,&receipt,||Ok(selection)).unwrap();
    }


    #[test]
    fn verified_secret_is_passed_to_state_without_a_second_key_read_or_creation() {
        let (_temp,selection,key,receipt)=runtime_fixture();
        let (_selection,secrets)=load_runtime_handles(&key,&receipt,||Ok(selection)).unwrap();
        fs::remove_file(&key).unwrap();
        let config=RuntimeConfig::read(|name|match name {
            "DATABASE_URL"=>Some("postgres://fixture@127.0.0.1/test".into()),"AUTH_SECRET"=>Some("fixture-only".into()),
            "DATA_KEY_FILE"=>Some(key.display().to_string()),_=>None,
        }).unwrap();
        let _state=AppState::new_with_secrets(config,secrets).unwrap();
        assert!(!key.exists(),"state must not reopen or create a receipt-verified key");
    }


    #[test]
    fn boot_verification_requires_a_descriptor_and_never_changes_the_key_or_files() {
        let (temp,config,path)=fixture();
        let key=fs::read(&config.data_key_file).unwrap();let descriptor=fs::read(&path).unwrap();
        let selected=verify_with(&path,&config.data_key_file,Some(config.data_key_file.as_os_str()),||read_fixture(&path)).unwrap();
        assert_eq!(selected.epoch,93);assert_eq!(fs::read(&config.data_key_file).unwrap(),key);
        assert_eq!(fs::read(&path).unwrap(),descriptor);
        let missing=temp.path().join("must-not-create-key");
        assert_eq!(verify_with(&temp.path().join("missing-descriptor"),&missing,Some(missing.as_os_str()),||panic!("absent means refusal, never a legacy classifier")).err().unwrap().code,"verification_failed");
        assert!(!missing.exists());assert_eq!(fs::read_dir(temp.path()).unwrap().count(),2);
        assert!(verify_with(&path,&config.data_key_file,Some(config.data_key_file.as_os_str()),||Err(invalid("verification_failed"))).is_err());
        assert_eq!(fs::read(&config.data_key_file).unwrap(),key);
    }

    #[tokio::test]
    async fn i2_serve_cannot_use_the_canonical_key_or_missing_runtime_path() {
        let (_temp,config,_path)=fixture();
        assert_eq!(load_for_serve(&config).await.err().unwrap().code,"key_handle_failed");
        assert!(config.data_key_file.exists());
    }

}
