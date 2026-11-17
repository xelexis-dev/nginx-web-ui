#![cfg(unix)]
use nwui_api::{db_upgrade::layout::Roots,upgrade::{self}};
use std::{collections::BTreeMap,fs,os::unix::fs::{MetadataExt,PermissionsExt,symlink},path::Path};
use sha2::{Digest,Sha256};
fn snapshot(root:&Path)->BTreeMap<String,(u64,u64,u32,u32,u32,i64,i64,Vec<u8>)>{
    fn visit(root:&Path,p:&Path,out:&mut BTreeMap<String,(u64,u64,u32,u32,u32,i64,i64,Vec<u8>)>){
        let m=fs::symlink_metadata(p).unwrap();let bytes=if m.is_file(){Sha256::digest(fs::read(p).unwrap()).to_vec()}else if m.file_type().is_symlink(){fs::read_link(p).unwrap().as_os_str().as_encoded_bytes().to_vec()}else{vec![]};
        out.insert(p.strip_prefix(root).unwrap().display().to_string(),(m.ino(),m.nlink(),m.uid(),m.gid(),m.mode(),m.mtime(),m.mtime_nsec(),bytes));
        if m.is_dir(){for e in fs::read_dir(p).unwrap(){visit(root,&e.unwrap().path(),out);}}
    }
    let mut out=BTreeMap::new();visit(root,root,&mut out);out
}
fn fixture()->(tempfile::TempDir,Roots){let t=tempfile::tempdir().unwrap();let r=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for p in [&r.db,&r.app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}(t,r)}
fn version(path:&Path,text:&str){fs::create_dir_all(path.parent().unwrap()).unwrap();fs::write(path,text).unwrap();}
#[test]
fn every_observation_refuses_without_changing_either_root(){
 for case in ["root","nested","both","bad","symlink","fifo","hardlink","selector15","uncommitted","restored","restore-incomplete","postponed","owner","nested-link","selector-mismatch","malformed-journal","restore-intent"]{
    let(_t,r)=fixture();
    match case {
        "root"=>version(&r.db.join("PG_VERSION"),"15\n"),
        "nested"=>version(&r.db.join("pgdata/PG_VERSION"),"15\n"),
        "both"=>{version(&r.db.join("PG_VERSION"),"15\n");version(&r.db.join("pgdata/PG_VERSION"),"15\n");},
        "owner"=>{version(&r.db.join("PG_VERSION"),"15\n");std::os::unix::fs::chown(r.db.join("PG_VERSION"),Some(12345),None).unwrap();},
        "nested-link"=>{version(&r.app.join("cluster/PG_VERSION"),"15\n");symlink(r.app.join("cluster"),r.db.join("pgdata")).unwrap();},
        "malformed-journal"=>{version(&r.app.join(".nwui-auto/journal-v1.json"),"invalid");fs::set_permissions(r.app.join(".nwui-auto"),fs::Permissions::from_mode(0o700)).unwrap();fs::set_permissions(r.app.join(".nwui-auto/journal-v1.json"),fs::Permissions::from_mode(0o600)).unwrap();},
        "restore-intent"=>{journal(&r,"pg17",true);let path=r.app.join(".nwui-auto/journal-v1.json");let mut value:serde_json::Value=serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();let generation=uuid::Uuid::new_v4().to_string();let record=serde_json::json!({"seq":2,"at":"2026-11-17T00:00:00Z","step":"cmd-restore-pg15","state":{"kind":"intent"},"data":{"backupId":value["attemptId"],"generation":generation,"discardId":generation}});value["records"].as_array_mut().unwrap().push(record);fs::write(path,serde_json::to_vec(&value).unwrap()).unwrap();},
        "bad"=>version(&r.db.join("PG_VERSION"),"150\n"),
        "symlink"=>{version(&r.app.join("saved"),"15\n");symlink(r.app.join("saved"),r.db.join("PG_VERSION")).unwrap();},
        "fifo"=>{let path=std::ffi::CString::new(r.db.join("PG_VERSION").as_os_str().as_encoded_bytes()).unwrap();assert_eq!(unsafe{libc::mkfifo(path.as_ptr(),0o600)},0);},
        "hardlink"=>{version(&r.app.join("saved"),"15\n");fs::hard_link(r.app.join("saved"),r.db.join("PG_VERSION")).unwrap();},
        "selector15"|"selector-mismatch"=>{if case=="selector-mismatch"{version(&r.db.join("17/data/PG_VERSION"),"17\n");}let dir=r.app.join(".nwui-auto");fs::create_dir(&dir).unwrap();fs::set_permissions(&dir,fs::Permissions::from_mode(0o700)).unwrap();let id=uuid::Uuid::new_v4().to_string();let value=serde_json::json!({"version":1,"activeMajor":15,"generation":null,"systemIdentifier":"123","origin":"auto","backupId":id,"profileId":"historical","profileSha256":"a".repeat(64),"manifestSha256":null,"operatorReason":"restore-pg15"});let p=dir.join("active-v1.json");fs::write(&p,serde_json::to_vec(&value).unwrap()).unwrap();fs::set_permissions(&p,fs::Permissions::from_mode(0o600)).unwrap();},
        _=>journal(&r,match case{"restored"=>"restored","restore-incomplete"=>"restore-incomplete","postponed"=>"postponed",_=>"upgrading"},false),
    }
    let had17=r.db.join("17").exists();
    let before=(snapshot(&r.db),snapshot(&r.app));
    for _ in 0..2 {let error=upgrade::legacy_admission(&r).unwrap_err();assert_eq!(error.code,if matches!(case,"both"|"bad"|"symlink"|"fifo"|"hardlink"|"owner"|"nested-link"){"unsupported_layout"}else if matches!(case,"selector-mismatch"|"malformed-journal"){"pair_mismatch"}else{"requires_2_0_first"},"{case}");}
    assert_eq!(before,(snapshot(&r.db),snapshot(&r.app)),"{case}");
    assert!(!r.db.join(".nwui").exists());assert!(!r.app.join(".nwui").exists());assert_eq!(r.db.join("17").exists(),had17);
 }
}
#[test]
fn fresh_pg17_and_committed_history_do_not_admit_retained_backups_as_active15(){
 for committed in [false,true]{let(_t,r)=fixture();upgrade::legacy_admission(&r).unwrap();version(&r.db.join("17/data/PG_VERSION"),"17\n");version(&r.db.join(".nwui-pg15-backup/saved/PG_VERSION"),"15\n");if committed{journal(&r,"pg17",true);}let before=(snapshot(&r.db),snapshot(&r.app));upgrade::legacy_admission(&r).unwrap();assert_eq!(before,(snapshot(&r.db),snapshot(&r.app)));}
}

fn journal(r:&Roots,outcome:&str,committed:bool){
 let dir=r.app.join(".nwui-auto");fs::create_dir(&dir).unwrap();fs::set_permissions(&dir,fs::Permissions::from_mode(0o700)).unwrap();
 let records=if committed{serde_json::json!([{"seq":1,"at":"2026-11-17T00:00:00Z","step":"p5-commit","state":{"kind":"done"},"data":null}])}else{serde_json::json!([])};
 let value=serde_json::json!({"version":1,"attemptId":uuid::Uuid::new_v4().to_string(),"branch":"upgrade","candidate":{"revision":"a".repeat(40),"pinsSha256":"b".repeat(64),"migrationsSha256":"c".repeat(64)},"records":records,"moves":[],"handoff":"none","outcome":outcome,"retry":{"consumedLabels":[],"interruptions":0,"backoffUntil":null}});
 let p=dir.join("journal-v1.json");fs::write(&p,serde_json::to_vec(&value).unwrap()).unwrap();fs::set_permissions(&p,fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn obsolete_cli_commands_refuse_before_paths_configuration_or_secrets(){
    let t=tempfile::tempdir().unwrap();
    for command in ["check","run","status","verify","restore-pg15"]{
        let result=std::process::Command::new(env!("CARGO_BIN_EXE_nwui-api")).args(["db-upgrade",command]).env_clear().env("PGROOT","/invalid-root").env("DATA_KEY_FILE","").current_dir(t.path()).output().unwrap();
        assert_eq!(result.status.code(),Some(2),"{command}");
        let error=String::from_utf8(result.stderr).unwrap();
        assert!(error.contains("2.0.x 최신(2.0.3 이상)"));assert!(error.contains("offline entrypoint"));assert!(error.contains("변경하지 않았습니다"));assert!(result.stdout.is_empty());
        assert_eq!(fs::read_dir(t.path()).unwrap().count(),0);
    }
}
