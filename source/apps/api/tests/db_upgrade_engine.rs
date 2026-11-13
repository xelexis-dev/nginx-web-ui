pub use nwui_api::{upgrade,nonroot,certificates};
#[path="../src/crypto.rs"] mod crypto;
#[path="../src/db_upgrade/mod.rs"] mod db_upgrade;
use db_upgrade::{engine::{Engine,required_bytes},layout::Roots,files};
use std::{fs,path::Path};
fn engine(root:&Path)->Engine {
    let paths=["source-db","source-app","destination-db","destination-app"].map(|name|root.join(name));
    for path in &paths{fs::create_dir(path).unwrap();}
    Engine{source_db:paths[0].clone(),source_app:paths[1].clone(),roots:Roots{db:paths[2].clone(),app:paths[3].clone(),architecture:"arm64".into()}}
}
#[test]
fn status_uses_only_destinations_and_never_creates_a_control_record() {
    let t=tempfile::tempdir().unwrap();let mut e=engine(t.path());e.source_db=t.path().join("absent-source");e.source_app=t.path().join("absent-app");
    let before=(files::snapshot(&e.roots.db).unwrap(),files::snapshot(&e.roots.app).unwrap());
    let status=e.status().unwrap();assert!(!status.ready);assert_eq!(status.phase,db_upgrade::state::Phase::NotStarted);
    assert_eq!(before,(files::snapshot(&e.roots.db).unwrap(),files::snapshot(&e.roots.app).unwrap()));
    files::private_dir(&e.roots.db.join(".nwui")).unwrap();
    files::atomic_json(&e.roots.db.join(".nwui/migration-v1.json"),&serde_json::json!({"schemaVersion":999})).unwrap();
    assert!(e.status().is_err());
}
#[test]
fn all_source_and_destination_aliases_and_nesting_refuse_without_writes() {
    for bad in ["same-source","same-destination","nested","symlink"] {
        let t=tempfile::tempdir().unwrap();let mut e=engine(t.path());
        match bad {
            "same-source"=>e.source_app=e.source_db.clone(),
            "same-destination"=>e.roots.app=e.roots.db.clone(),
            "nested"=>{e.roots.db=e.source_db.join("nested");fs::create_dir(&e.roots.db).unwrap();},
            _=>{let path=t.path().join("alias");std::os::unix::fs::symlink(&e.source_db,&path).unwrap();e.roots.db=path;},
        }
        assert_eq!(e.validate_roots().unwrap_err().code,"unsupported_layout");
        assert!(!e.source_db.join(".nwui").exists());assert!(!e.source_app.join(".nwui").exists());
    }
}
#[test]
fn capacity_includes_copies_wal_rewrites_largest_indexes_reserve_and_app() {
    assert_eq!(required_bytes(100,20,30).unwrap(),400+60+1_073_741_824+20);
    assert!(required_bytes(u64::MAX,0,0).is_err());assert!(required_bytes(u64::MAX/4+1,0,0).is_err());assert!(required_bytes(0,u64::MAX,0).is_err());assert!(required_bytes(0,0,u64::MAX).is_err());
}

#[test]
fn a_dangling_control_symlink_is_corruption_instead_of_not_started() {
    let t=tempfile::tempdir().unwrap();let e=engine(t.path());
    std::os::unix::fs::symlink(t.path().join("missing"),e.roots.db.join(".nwui")).unwrap();
    assert_eq!(e.status().unwrap_err().code,"unsupported_layout");
}

#[test]
fn distinct_unmounted_test_roots_are_safe_paths_but_not_migration_mounts() {
    let t=tempfile::tempdir().unwrap();let e=engine(t.path());
    e.validate_roots().unwrap();assert!(e.validate_mounts().is_err());
}

#[tokio::test]
async fn check_rejects_unmounted_inputs_before_creating_any_journal_or_secret() {
    let t=tempfile::tempdir().unwrap();let e=engine(t.path());
    let args=db_upgrade::arguments::Arguments::parse(["check","--backup-id","cold-backup","--destination-db-volume","new-db","--destination-app-volume","new-app"].into_iter().map(str::to_owned)).unwrap();
    let before=(files::snapshot(&e.roots.db).unwrap(),files::snapshot(&e.roots.app).unwrap());
    assert_eq!(e.execute(&args).await.unwrap_err().code,"unsupported_layout");
    assert_eq!(before,(files::snapshot(&e.roots.db).unwrap(),files::snapshot(&e.roots.app).unwrap()));
}
#[tokio::test]
async fn an_interrupted_attempt_is_not_reused_even_without_source_mounts() {
    let t=tempfile::tempdir().unwrap();let mut e=engine(t.path());e.source_db=t.path().join("missing-source");
    let value=include_str!("../src/db_upgrade/examples/interrupted.json");
    let state:db_upgrade::state::State=serde_json::from_str(value).unwrap();
    files::private_dir(&e.roots.db.join(".nwui")).unwrap();files::atomic_json(&e.roots.db.join(".nwui/migration-v1.json"),&state).unwrap();
    let args=db_upgrade::arguments::Arguments{operation:db_upgrade::arguments::Operation::Run,json:true,backup_id:Some(state.source.unwrap().backup_id),destinations:state.destinations};
    let before=(files::snapshot(&e.roots.db).unwrap(),files::snapshot(&e.roots.app).unwrap());
    assert_eq!(e.execute(&args).await.unwrap_err().code,"migration_incomplete");
    assert_eq!(before,(files::snapshot(&e.roots.db).unwrap(),files::snapshot(&e.roots.app).unwrap()));
}

#[test]
fn capacity_covers_final_candidate_measured_peaks_with_twenty_percent_margin() {
    // G6 c3481594: sourceDbBytes/sourceAppBytes/largestIndexBytes and sampled destination peak.
    let cases:[(&str,u64,u64,u64,u64);3]=[
        ("representative",49_008_809,3_023,253_952,100_728_832),
        ("1GiB",2_211_409_336,3_023,12_263_424,5_542_178_816),
        ("5GiB",6_716_787_334,3_023,61_243_392,23_166_222_336),
    ];
    let mut underestimates=Vec::new();
    for (name,db,app,index,peak) in cases {
        let required=required_bytes(db,app,index).unwrap();
        if u128::from(required)*5<u128::from(peak)*6 {underestimates.push((name,required,peak));}
    }
    assert!(underestimates.is_empty(),"estimate lacks 20% over measured peak: {underestimates:?}");
}
