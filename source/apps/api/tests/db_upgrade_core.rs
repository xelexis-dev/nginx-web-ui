// Exercise the database upgrade state and validation modules together.
#[path="../src/db_upgrade/state.rs"] mod state;
use state::{State, Phase, Operation};

#[test]
fn journals_intent_before_completion_and_refuses_skips() {
 let raw=std::fs::read(format!("{}/src/db_upgrade/examples/checked.json",env!("CARGO_MANIFEST_DIR"))).unwrap();
 let mut s:State=serde_json::from_slice(&raw).unwrap();s.phase=Phase::NotStarted;
 assert!(s.begin(Phase::Converted).is_err());
 assert_eq!(s.phase,Phase::NotStarted);
 s.begin(Phase::Validated).unwrap();
 assert_eq!(s.operation,Operation::Running);
 assert_eq!(s.next_phase,Some(Phase::Validated));
 assert!(s.begin(Phase::Copied).is_err());
 assert!(!s.ready);
 s.complete(Phase::Validated).unwrap();
 assert_eq!(s.operation,Operation::Idle);
 assert_eq!(s.next_phase,None);
 assert_eq!(s.phase,Phase::Validated);
 assert!(s.complete(Phase::Copied).is_err());
}

#[test]
fn saved_running_state_never_autoresumes_an_unproven_operation() {
 let mut s=State::not_started("arm64");
 s.begin(Phase::Validated).unwrap();
 let raw=serde_json::to_vec(&s).unwrap();
 let restored:State=serde_json::from_slice(&raw).unwrap();
 assert!(restored.validate().is_ok());
 assert!(!restored.can_continue());
 assert!(!restored.ready);
}

#[path="../src/db_upgrade/files.rs"] mod files;
use std::os::unix::fs::{symlink,PermissionsExt};

#[test]
fn state_parser_and_journal_enforce_the_committed_contract() {
 for name in ["not-started","checked","ready","interrupted"] {
  let raw=std::fs::read(format!("{}/src/db_upgrade/examples/{name}.json",env!("CARGO_MANIFEST_DIR"))).unwrap();
  let state:State=serde_json::from_slice(&raw).unwrap();
  state.validate().unwrap();
 }
 let mut v=serde_json::to_value(State::not_started("arm64")).unwrap();
 v["unexpected"]=serde_json::json!("private");
 assert!(serde_json::from_value::<State>(v).is_err());
}

#[test]
fn atomic_state_updates_are_private_and_never_follow_symlinks() {
 let t=tempfile::tempdir().unwrap();let dir=t.path().join("control");
 files::private_dir(&dir).unwrap();let path=dir.join("state.json");
 files::atomic_json(&path,&State::not_started("arm64")).unwrap();
 assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode()&0o777,0o600);
 assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode()&0o777,0o700);
 let value:State=files::read_json(&path).unwrap();value.validate().unwrap();
 let victim=t.path().join("victim");std::fs::write(&victim,"preserve").unwrap();
 std::fs::remove_file(&path).unwrap();symlink(&victim,&path).unwrap();
 assert!(files::atomic_json(&path,&value).is_err());
 assert!(files::read_json::<State>(&path).is_err());
 assert_eq!(std::fs::read_to_string(&victim).unwrap(),"preserve");
 std::fs::remove_file(&path).unwrap();std::fs::remove_dir(&dir).unwrap();symlink(t.path(),&dir).unwrap();
 assert!(files::private_dir(&dir).is_err());
 assert!(files::atomic_json(&dir.join("state.json"),&value).is_err());
}

#[test]
fn tree_hashes_detect_content_and_permission_changes_and_reject_escape() {
 let t=tempfile::tempdir().unwrap();let root=t.path().join("source");std::fs::create_dir(&root).unwrap();
 let path=root.join("value");std::fs::write(&path,"old").unwrap();
 let first=files::snapshot(&root).unwrap();assert_eq!(first,files::snapshot(&root).unwrap());
 std::fs::write(&path,"new").unwrap();assert_ne!(first,files::snapshot(&root).unwrap());
 let changed=files::snapshot(&root).unwrap();std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o600)).unwrap();
 assert_ne!(changed,files::snapshot(&root).unwrap());
 symlink("/etc/passwd",root.join("escape")).unwrap();assert!(files::snapshot(&root).is_err());
}

#[test]
fn copied_tree_has_independent_files_preserving_secret_modes() {
 let t=tempfile::tempdir().unwrap();let source=t.path().join("source");let dest=t.path().join("destination");std::fs::create_dir(&source).unwrap();
 std::fs::write(source.join("key"),"synthetic-key").unwrap();std::fs::set_permissions(source.join("key"),std::fs::Permissions::from_mode(0o600)).unwrap();
 files::copy_tree(&source,&dest).unwrap();assert_eq!(files::snapshot(&source).unwrap(),files::snapshot(&dest).unwrap());
 std::fs::write(dest.join("key"),"changed").unwrap();assert_eq!(std::fs::read_to_string(source.join("key")).unwrap(),"synthetic-key");
 assert!(files::copy_tree(&source,&dest).is_err());
}

#[test]
fn runtime_resolver_refuses_legacy_or_conflicting_overrides() {
    use std::process::Command;
    let temp=tempfile::tempdir().unwrap();let script=temp.path().join("pg-env.sh");
    std::fs::write(&script,include_str!("../../../docker/scripts/pg-env.sh")).unwrap();
    for (root,data,code) in [(None,None,0),(Some("/var/lib/postgresql/data"),Some("/var/lib/postgresql/data/17/data"),0),(Some("/old"),None,2),(None,Some("/var/lib/postgresql/data"),2),(None,Some("/tmp/cluster"),2)] {
        let mut cmd=Command::new("sh");cmd.args(["-c",". \"$1\"; printf '%s\\n%s\\n%s\\n' \"$PGROOT\" \"$PGDATA\" \"$PGBIN\"","test"]).arg(&script).env_clear().env("PATH","/usr/bin:/bin");
        if let Some(value)=root{cmd.env("PGROOT",value);}if let Some(value)=data{cmd.env("PGDATA",value);}
        let out=cmd.output().unwrap();assert_eq!(out.status.code(),Some(code));
        if code==0{assert_eq!(String::from_utf8(out.stdout).unwrap(),"/var/lib/postgresql/data\n/var/lib/postgresql/data/17/data\n/usr/lib/postgresql/17/bin\n");}
        else{assert!(String::from_utf8_lossy(&out.stderr).contains("storage_override"));}
    }
}

#[test]
fn boot_classification_is_an_explicit_pre_secret_dependency() {
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docker");
    assert_eq!(std::fs::read_to_string(root.join("s6-rc.d/storage-classify/type")).unwrap().trim(),"oneshot");
    let classify=std::fs::read_to_string(root.join("s6-rc.d/storage-classify/up")).unwrap();
    assert!(classify.contains(". /etc/s6-overlay/scripts/nwui-gate.sh; nwui_gate_oneshot;"));
    assert!(!classify.contains("nwui-api storage classify"));
    // Runner readiness publishes the classified selection before secret consumers start.
    assert_eq!(std::fs::read_to_string(root.join("s6-rc.d/upgrade-runner/type")).unwrap().trim(),"longrun");
    assert_eq!(std::fs::read_to_string(root.join("s6-rc.d/upgrade-runner/notification-fd")).unwrap().trim(),"3");
    let runner=std::fs::read_to_string(root.join("s6-rc.d/upgrade-runner/run")).unwrap();
    assert_eq!(runner.lines().last(),Some("exec /usr/local/bin/nwui-api upgrade-runner"));
    for path in ["s6-rc.d/upgrade-runner/dependencies.d/storage-locks","s6-rc.d/storage-classify/dependencies.d/upgrade-runner","s6-rc.d/secrets-init/dependencies.d/upgrade-runner","s6-rc.d/postgres-init/dependencies.d/secrets-init","user-bundles.d/user/contents.d/upgrade-runner","user-bundles.d/user/contents.d/storage-classify"] {
        assert!(root.join(path).is_file(),"missing startup ordering: {path}");
    }
    assert!(!root.join("s6-rc.d/secrets-init/dependencies.d/storage-classify").exists());
}

#[test]
fn optional_values_must_be_explicit_null_rather_than_missing_state_fields() {
    let raw=std::fs::read(format!("{}/src/db_upgrade/examples/not-started.json",env!("CARGO_MANIFEST_DIR"))).unwrap();
    let v:serde_json::Value=serde_json::from_slice(&raw).unwrap();
    for key in ["attemptId","pairId","nextPhase","source","inputsSha256","destinations","lastError"] {
        let mut missing=v.clone();missing.as_object_mut().unwrap().remove(key);
        assert!(serde_json::from_value::<State>(missing).is_err(),"accepted missing {key}");
    }
}

#[test]
fn supervised_postgres_uses_pg17_resolver_and_keeps_socket_and_tcp_restrictions() {
    let temp=tempfile::tempdir().unwrap();let setuid=temp.path().join("setuid");
    std::fs::write(&setuid,"#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();std::fs::set_permissions(&setuid,std::fs::Permissions::from_mode(0o700)).unwrap();
    let resolver=temp.path().join("pg-env");std::fs::write(&resolver,include_str!("../../../docker/scripts/pg-env.sh")).unwrap();
    let gate=temp.path().join("nwui-gate");std::fs::write(&gate,"nwui_gate_longrun() { :; }\n").unwrap();
    let source=include_str!("../../../docker/s6-rc.d/postgres/run").replace("/command/s6-setuidgid",setuid.to_str().unwrap()).replace("/etc/s6-overlay/scripts/nwui-gate.sh",gate.to_str().unwrap()).replace("/etc/s6-overlay/scripts/pg-env.sh",resolver.to_str().unwrap()).replace("exec 9<\"$PGROOT\"",&format!("exec 9<\"{}\"",temp.path().display()));
    let run=temp.path().join("run");std::fs::write(&run,source).unwrap();
    let out=std::process::Command::new("sh").arg(run).env_clear().env("PATH","/usr/bin:/bin").output().unwrap();
    assert!(out.status.success());let args=String::from_utf8(out.stdout).unwrap();let args:Vec<_>=args.lines().collect();
    assert_eq!(&args[..4],["postgres","/usr/lib/postgresql/17/bin/postgres","-D","/var/lib/postgresql/data/17/data"]);
    assert!(args.contains(&"listen_addresses=127.0.0.1"));assert!(args.contains(&"unix_socket_permissions=0700"));
}

#[test]
fn migration_existing_key_loader_never_creates_or_replaces_a_key() {
    use nwui_api::crypto::SecretBox;
    let temp=tempfile::tempdir().unwrap();let path=temp.path().join("missing/key");
    assert!(SecretBox::load_existing(&path).is_err());assert!(!path.parent().unwrap().exists());
    let path=temp.path().join("key");
    for bad in ["", "partial", &"g".repeat(64)] {
        std::fs::write(&path,bad).unwrap();assert!(SecretBox::load_existing(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(),bad);
    }
    std::fs::write(&path,format!("{}\n","0A".repeat(32))).unwrap();
    let value=SecretBox::from_key([10;32]).seal("synthetic").unwrap();
    assert_eq!(SecretBox::load_existing(&path).unwrap().open(Some(&value)).as_deref(),Some("synthetic"));
    let link=temp.path().join("link");symlink(&path,&link).unwrap();assert!(SecretBox::load_existing(&link).is_err());
    assert!(SecretBox::load_existing(temp.path()).is_err());
}
