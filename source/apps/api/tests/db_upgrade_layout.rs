#![cfg(unix)]
#[path="../src/db_upgrade/mounts.rs"] mod mounts;
#[path="../src/db_upgrade/files.rs"] mod files;
#[path="../src/db_upgrade/state.rs"] mod state;
#[path="../src/db_upgrade/pair.rs"] mod pair;
#[path="../src/db_upgrade/error.rs"] mod error;
#[path="../src/db_upgrade/layout.rs"] mod layout;
pub use nwui_api::db_upgrade::{paths,key_file};
pub use nwui_api::{upgrade,certificates,crypto};
use pair::Pair;
use layout::{Roots,Decision};
use std::{path::Path,fs};
use std::os::unix::fs::{PermissionsExt,MetadataExt,symlink};
fn fixture_key(path:&Path)->crypto::SecretBox {
    fs::write(path,"A5".repeat(32)).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o600)).unwrap();
    crypto::SecretBox::load_existing(path).unwrap()
}


fn roots(temp:&Path)->Roots {
 let db=temp.join("db");let app=temp.join("app");fs::create_dir(&db).unwrap();fs::create_dir(&app).unwrap();
 Roots{db,app,architecture:"arm64".into()}
}
fn write_pair(roots:&Roots,pair:&Pair) {
 for root in [&roots.db,&roots.app] { files::private_dir(&root.join(".nwui")).unwrap();files::atomic_json(&root.join(".nwui/pair-v1.json"),pair).unwrap(); }
}
fn classify(roots:&Roots)->Result<Decision,error::Error> { layout::classify(roots,&|_|Ok("7382591915349309999".into())) }
fn material(roots:&Roots) {
 fs::create_dir(roots.app.join("secrets")).unwrap();fs::set_permissions(roots.app.join("secrets"),fs::Permissions::from_mode(0o700)).unwrap();
 for (name,value) in [("auth_secret","synthetic-auth"),("db_password","synthetic-pw"),("data_key",&"0a".repeat(32))] {
  fs::write(roots.app.join("secrets").join(name),value).unwrap();fs::set_permissions(roots.app.join("secrets").join(name),fs::Permissions::from_mode(0o600)).unwrap();
 }
 fs::create_dir(roots.app.join("tls")).unwrap();fs::set_permissions(roots.app.join("tls"),fs::Permissions::from_mode(0o700)).unwrap();
 let key=rcgen::KeyPair::generate().unwrap();let params=rcgen::CertificateParams::new(vec!["unconfigured.invalid".into()]).unwrap();let cert=params.self_signed(&key).unwrap();
 for (name,value,mode)in[("default.crt",cert.pem(),0o644),("default.key",key.serialize_pem(),0o600)] {
  let p=roots.app.join("tls").join(name);fs::write(&p,value).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(mode)).unwrap();
 }
}
fn cluster(roots:&Roots,major:&str) { let data=roots.db.join("17/data");fs::create_dir_all(data.join("global")).unwrap();fs::write(data.join("PG_VERSION"),major).unwrap();fs::write(data.join("global/pg_control"),"synthetic control: callback supplies identity").unwrap(); }
fn ready(roots:&Roots)->Pair {
 let file=Path::new(env!("CARGO_MANIFEST_DIR")).join("src/db_upgrade/examples/pair-ready.json");let pair:Pair=serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
 let file=Path::new(env!("CARGO_MANIFEST_DIR")).join("src/db_upgrade/examples/ready.json");let state:state::State=serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
 write_pair(roots,&pair);files::atomic_json(&roots.db.join(".nwui/migration-v1.json"),&state).unwrap();cluster(roots,"17");material(roots);pair
}
#[test]
fn fresh_intent_is_written_to_both_empty_volumes_before_provisioning() {
 let t=tempfile::tempdir().unwrap();let r=roots(t.path());assert!(matches!(classify(&r).unwrap(),Decision::Empty));
 let p=layout::prepare(&r,&|_|Ok("7382591915349309999".into())).unwrap();
 assert!(!p.ready);assert_eq!(p.mode,"fresh");assert_eq!(p.fresh_step.as_deref(),Some("secrets"));assert!(p.system_identifier.is_none());
 let db:Pair=files::read_json(&r.db.join(".nwui/pair-v1.json")).unwrap();let app:Pair=files::read_json(&r.app.join(".nwui/pair-v1.json")).unwrap();assert_eq!(db,app);
 assert!(!r.app.join("secrets").exists());assert!(!r.db.join("17").exists());assert!(matches!(classify(&r).unwrap(),Decision::Fresh(_)));
}
#[test]
fn interrupted_pair_write_is_not_automatically_adopted_or_repaired() {
 let t=tempfile::tempdir().unwrap();let r=roots(t.path());let p=Pair::fresh("arm64");files::private_dir(&r.db.join(".nwui")).unwrap();files::atomic_json(&r.db.join(".nwui/pair-v1.json"),&p).unwrap();
 let before=(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap());assert_eq!(classify(&r).unwrap_err().code,"pair_mismatch");assert_eq!(before,(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap()));
}
#[test]
fn pg15_and_unknown_layouts_are_refused_without_mutation() {
 for (path,major,code)in[("PG_VERSION","15","pg15_requires_upgrade"),("PG_VERSION","17","unsupported_layout"),("PG_VERSION","16","unsupported_layout"),("postgres/pgdata/PG_VERSION","15","unsupported_layout")] {
  let t=tempfile::tempdir().unwrap();let r=roots(t.path());let path=r.db.join(path);fs::create_dir_all(path.parent().unwrap()).unwrap();fs::write(path,major).unwrap();
  let before=(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap());assert_eq!(classify(&r).unwrap_err().code,code);assert_eq!(before,(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap()));
 }
}
#[test]
fn empty_db_with_existing_app_does_not_generate_a_new_key() {
 let t=tempfile::tempdir().unwrap();let r=roots(t.path());fs::write(r.app.join("existing"),"preserve").unwrap();assert_eq!(classify(&r).unwrap_err().code,"pair_mismatch");assert!(!r.app.join("secrets").exists());
}
#[test]
fn lost_found_exception_is_narrow_and_checked_before_pair_mismatch() {
 for bad in ["empty","file","nonempty","symlink","owner"] {
  let t=tempfile::tempdir().unwrap();let r=roots(t.path());let path=r.app.join("lost+found");
  match bad {"file"=>fs::write(&path,"").unwrap(),"symlink"=>symlink(t.path(),&path).unwrap(),_=>{fs::create_dir(&path).unwrap();if bad=="nonempty"{fs::write(path.join("foreign"),"keep").unwrap();}if bad=="owner"{std::os::unix::fs::chown(&path,Some(1000),None).unwrap();}}}
  if bad=="empty"{assert!(matches!(classify(&r).unwrap(),Decision::Empty));assert!(path.is_dir());}
  else{assert_eq!(classify(&r).unwrap_err().code,"unsupported_layout", "{bad}");assert!(fs::symlink_metadata(&path).is_ok());}
 }
}
#[test]
fn ready_pair_requires_state_identity_matching_partner_and_existing_keys() {
 for bad in ["none","partner","control","key","state"] {
  let t=tempfile::tempdir().unwrap();let r=roots(t.path());let mut p=ready(&r);
  match bad {
   "partner"=>{p.pair_id=uuid::Uuid::new_v4().to_string();files::atomic_json(&r.app.join(".nwui/pair-v1.json"),&p).unwrap();},
   "key"=>fs::remove_file(r.app.join("secrets/data_key")).unwrap(),
   "state"=>fs::remove_file(r.db.join(".nwui/migration-v1.json")).unwrap(),_=>{}
  }
  if bad=="control" {assert_eq!(layout::classify(&r,&|_|Ok("7382591915349309000".into())).unwrap_err().code,"pair_mismatch");}
  else if bad=="none" {assert!(matches!(classify(&r).unwrap(),Decision::Ready(_)));}
  else {assert_eq!(classify(&r).unwrap_err().code,match bad{"state"=>"migration_incomplete","key"=>"key_missing",_=>"pair_mismatch"});}
 }
}
#[test]
fn incomplete_migration_never_enters_the_provisioning_or_normal_service_path() {
 let t=tempfile::tempdir().unwrap();let r=roots(t.path());let mut pair=ready(&r);pair.ready=false;write_pair(&r,&pair);
 let file=Path::new(env!("CARGO_MANIFEST_DIR")).join("src/db_upgrade/examples/interrupted.json");let state:state::State=serde_json::from_slice(&fs::read(file).unwrap()).unwrap();files::atomic_json(&r.db.join(".nwui/migration-v1.json"),&state).unwrap();
 assert_eq!(classify(&r).unwrap_err().code,"migration_incomplete");
}
#[test]
fn fresh_resume_refuses_extra_files_broken_material_and_partial_initdb() {
 for bad in ["extra","key","partial","step"] {
  let t=tempfile::tempdir().unwrap();let r=roots(t.path());let mut pair=Pair::fresh("arm64");
  match bad {"extra"=>{fs::write(r.app.join("foreign"),"keep").unwrap();},"key"=>{material(&r);},"partial"=>{fs::create_dir_all(r.db.join("17/data")).unwrap();fs::write(r.db.join("17/data/partial"),"keep").unwrap();pair.fresh_step=Some("initdb".into());},"step"=>pair.fresh_step=Some("unknown".into()),_=>{}}
  write_pair(&r,&pair);assert!(classify(&r).is_err(),"{bad}");assert!(!pair.ready);
 }
}

#[test]
fn an_existing_2x_pair_from_another_architecture_is_refused() {
    let t=tempfile::tempdir().unwrap();let mut r=roots(t.path());ready(&r);r.architecture="amd64".into();
    let before=(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap());
    assert_eq!(classify(&r).unwrap_err().code,"unsupported_layout");
    assert_eq!(before,(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap()));
}

#[test]
fn a_competing_provisioner_cannot_write_a_second_pair() {
    let t=tempfile::tempdir().unwrap();let r=roots(t.path());let lock=fs::File::open(&r.db).unwrap();lock.try_lock().unwrap();
    assert_eq!(layout::prepare(&r,&|_|Ok("7382591915349309999".into())).unwrap_err().code,"storage_busy");
    assert!(!r.db.join(".nwui").exists());assert!(!r.app.join(".nwui").exists());drop(lock);
    assert!(layout::prepare(&r,&|_|Ok("7382591915349309999".into())).is_ok());
}

#[path="../src/db_upgrade/key_binding.rs"] mod key_binding;
fn fresh_ready_without_key(r:&Roots)->Pair {
    let mut p=Pair::fresh("arm64");p.ready=true;p.fresh_step=None;p.system_identifier=Some("7382591915349309999".into());
    write_pair(r,&p);cluster(r,"17");material(r);fs::remove_file(r.app.join("secrets/data_key")).unwrap();p
}
#[test]
fn published_key_is_bound_to_both_ready_records_before_business_writes() {
    let t=tempfile::tempdir().unwrap();let r=roots(t.path());fresh_ready_without_key(&r);let path=r.app.join("secrets/data_key");
    assert!(matches!(classify(&r).unwrap(),Decision::Ready(_)));
    let key=fixture_key(&path);
    key_binding::mark_key_required(&r,&|_|Ok("7382591915349309999".into()),&key).unwrap();
    for root in [&r.db,&r.app] {let pair:Pair=files::read_json(&root.join(".nwui/pair-v1.json")).unwrap();assert!(pair.encryption_key_required);}
    let before=(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap(),fs::metadata(r.db.join(".nwui/pair-v1.json")).unwrap().modified().unwrap());
    key_binding::mark_key_required(&r,&|_|Ok("7382591915349309999".into()),&key).unwrap();
    assert_eq!(before,(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap(),fs::metadata(r.db.join(".nwui/pair-v1.json")).unwrap().modified().unwrap()));
    fs::remove_file(&path).unwrap();let before=(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap());
    assert_eq!(classify(&r).unwrap_err().code,"key_missing");assert!(!path.exists());assert_eq!(before,(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap()));
}
#[test]
fn a_different_loaded_key_refuses_without_writes() {
    let t=tempfile::tempdir().unwrap();let r=roots(t.path());fresh_ready_without_key(&r);
    fixture_key(&r.app.join("secrets/data_key"));
    let before=(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap());
    assert_eq!(key_binding::mark_key_required(&r,&|_|Ok("7382591915349309999".into()),&crypto::SecretBox::from_key([19;32])).unwrap_err().code,"pair_mismatch");
    assert_eq!(before,(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap()));
}
#[test]
fn flag_only_torn_key_binding_recovers_in_both_write_orders() {
    for first in ["db","app"] {
        let t=tempfile::tempdir().unwrap();let r=roots(t.path());let mut pair=fresh_ready_without_key(&r);
        let key=fixture_key(&r.app.join("secrets/data_key"));pair.encryption_key_required=true;
        files::atomic_json(&if first=="db"{&r.db}else{&r.app}.join(".nwui/pair-v1.json"),&pair).unwrap();
        let before=(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap());
        assert!(matches!(classify(&r).unwrap(),Decision::Ready(_)));assert_eq!(before,(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap()));
        key_binding::mark_key_required(&r,&|_|Ok("7382591915349309999".into()),&key).unwrap();
        for root in [&r.db,&r.app]{assert_eq!(files::read_json::<Pair>(&root.join(".nwui/pair-v1.json")).unwrap(),pair);}
    }
}
#[test]
fn a_torn_flag_with_no_key_has_a_distinct_refusal_and_never_generates_one() {
    let t=tempfile::tempdir().unwrap();let r=roots(t.path());let mut p=fresh_ready_without_key(&r);p.encryption_key_required=true;files::atomic_json(&r.db.join(".nwui/pair-v1.json"),&p).unwrap();
    let before=(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap());
    assert_eq!(classify(&r).unwrap_err().code,"key_missing");
    assert_eq!(key_binding::mark_key_required(&r,&|_|Ok("7382591915349309999".into()),&crypto::SecretBox::from_key([19;32])).unwrap_err().code,"key_missing");
    assert_eq!(before,(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap()));
}
#[test]
fn two_required_flags_with_a_deleted_key_have_the_same_distinct_refusal() {
    let t=tempfile::tempdir().unwrap();let r=roots(t.path());ready(&r);fs::remove_file(r.app.join("secrets/data_key")).unwrap();
    let before=(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap());
    assert_eq!(classify(&r).unwrap_err().code,"key_missing");
    assert_eq!(key_binding::mark_key_required(&r,&|_|Ok("7382591915349309999".into()),&crypto::SecretBox::from_key([19;32])).unwrap_err().code,"key_missing");
    assert_eq!(before,(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap()));
}
#[test]
fn other_torn_fields_or_wrong_loaded_keys_never_repair_the_records() {
    for bad in ["pair","loaded","malformed"] {
        let t=tempfile::tempdir().unwrap();let r=roots(t.path());let mut p=fresh_ready_without_key(&r);let path=r.app.join("secrets/data_key");let key=fixture_key(&path);
        p.encryption_key_required=true;if bad=="pair"{p.pair_id=uuid::Uuid::new_v4().to_string();}if bad=="malformed"{fs::write(&path,"not a key").unwrap();}
        files::atomic_json(&r.db.join(".nwui/pair-v1.json"),&p).unwrap();let wrong=crypto::SecretBox::from_key([19;32]);
        let before=(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap());
        assert_eq!(key_binding::mark_key_required(&r,&|_|Ok("7382591915349309999".into()),if bad=="loaded"{&wrong}else{&key}).unwrap_err().code,"pair_mismatch");
        assert_eq!(before,(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap()));
    }
}
#[test]
fn key_binding_coexists_with_the_postmaster_shared_lock_but_offline_work_refuses() {
    let t=tempfile::tempdir().unwrap();let r=roots(t.path());fresh_ready_without_key(&r);let key=fixture_key(&r.app.join("secrets/data_key"));
    let running=fs::File::open(&r.db).unwrap();running.try_lock_shared().unwrap();
    assert_eq!(layout::prepare(&r,&|_|Ok("7382591915349309999".into())).unwrap_err().code,"storage_busy");
    key_binding::mark_key_required(&r,&|_|Ok("7382591915349309999".into()),&key).unwrap();
    let app_lock=fs::File::open(&r.app).unwrap();app_lock.try_lock().unwrap();
    assert_eq!(key_binding::mark_key_required(&r,&|_|Ok("7382591915349309999".into()),&key).unwrap_err().code,"storage_busy");drop(app_lock);drop(running);
    assert!(layout::prepare(&r,&|_|Ok("7382591915349309999".into())).is_ok());
}

#[test]
fn aliased_roots_and_unsafe_parent_ownership_or_permissions_refuse_before_writes() {
    for bad in ["alias","owner","permissions"] {
        let t=tempfile::tempdir().unwrap();let mut r=roots(t.path());
        match bad {"alias"=>r.app=r.db.clone(),"owner"=>std::os::unix::fs::chown(&r.app,Some(1000),None).unwrap(),_=>fs::set_permissions(&r.app,fs::Permissions::from_mode(0o777)).unwrap()}
        let before=(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap());
        assert_eq!(layout::prepare(&r,&|_|Ok("7382591915349309999".into())).unwrap_err().code,if bad=="alias"{"pair_mismatch"}else{"unsupported_layout"});
        assert_eq!(before,(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap()));
    }
}
#[test]
fn migrated_119_secret_modes_are_preserved_only_with_the_private_directory() {
    let t=tempfile::tempdir().unwrap();let r=roots(t.path());ready(&r);
    for name in ["auth_secret","db_password"]{fs::set_permissions(r.app.join("secrets").join(name),fs::Permissions::from_mode(0o644)).unwrap();}
    let before=files::snapshot(&r.app).unwrap();assert!(matches!(classify(&r).unwrap(),Decision::Ready(_)));assert_eq!(before,files::snapshot(&r.app).unwrap());
    for bad in ["directory","key","mode","owner"] {
        let t=tempfile::tempdir().unwrap();let r=roots(t.path());ready(&r);
        for name in ["auth_secret","db_password"]{fs::set_permissions(r.app.join("secrets").join(name),fs::Permissions::from_mode(0o644)).unwrap();}
        match bad {
            "directory"=>fs::set_permissions(r.app.join("secrets"),fs::Permissions::from_mode(0o755)).unwrap(),
            "key"=>fs::set_permissions(r.app.join("secrets/data_key"),fs::Permissions::from_mode(0o644)).unwrap(),
            "mode"=>fs::set_permissions(r.app.join("secrets/auth_secret"),fs::Permissions::from_mode(0o640)).unwrap(),
            _=>std::os::unix::fs::chown(r.app.join("secrets/auth_secret"),Some(1000),None).unwrap(),
        }
        let before=files::snapshot(&r.app).unwrap();assert_eq!(classify(&r).unwrap_err().code,"pair_mismatch","{bad}");assert_eq!(before,files::snapshot(&r.app).unwrap());
    }
}
#[test]
fn fresh_ready_secrets_do_not_adopt_legacy_file_modes() {
    let t=tempfile::tempdir().unwrap();let r=roots(t.path());fresh_ready_without_key(&r);
    fs::set_permissions(r.app.join("secrets/auth_secret"),fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(classify(&r).unwrap_err().code,"pair_mismatch");
}

#[test]
fn a_cluster_internal_symlink_is_refused_before_any_offline_identity_read() {
    let t=tempfile::tempdir().unwrap();let r=roots(t.path());ready(&r);
    let external=t.path().join("unrelated");fs::create_dir(&external).unwrap();fs::write(external.join("keep"),b"unchanged").unwrap();
    std::os::unix::fs::symlink(&external,r.db.join("17/data/base")).unwrap();
    let before=files::snapshot(&external).unwrap();let reads=std::cell::Cell::new(0);
    let result=layout::classify(&r,&|_|{reads.set(reads.get()+1);Ok("7382591915349309999".into())});
    assert!(result.is_err());assert_eq!(reads.get(),0);assert_eq!(before,files::snapshot(&external).unwrap());
}

fn cluster_link_refusal(case:&str,test:&str,diagnostic:&str) {
    if std::env::var("NWUI_TEST_CLUSTER_LINK").as_deref()!=Ok(case) {
        let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact",test,"--nocapture"])
            .env("NWUI_TEST_CLUSTER_LINK",case).output().unwrap();
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("private-target-not-for-logs"));
        assert!(String::from_utf8_lossy(&output.stderr).contains(diagnostic),"missing sanitized diagnostic: {}",String::from_utf8_lossy(&output.stderr));return;
    }
    let t=tempfile::tempdir().unwrap();let r=roots(t.path());ready(&r);
    let external=t.path().join("private-target-not-for-logs");fs::create_dir(&external).unwrap();fs::write(external.join("keep"),b"unchanged").unwrap();
    let path=r.db.join("17/data").join(case);fs::create_dir_all(path.parent().unwrap()).unwrap();
    let before=(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap(),files::snapshot(&external).unwrap());
    symlink(&external,&path).unwrap();let metadata=fs::symlink_metadata(&path).unwrap();let reads=std::cell::Cell::new(0);
    let error=layout::classify(&r,&|_|{reads.set(reads.get()+1);Ok("7382591915349309999".into())}).unwrap_err();
    assert_eq!(error.code,"unsupported_layout");assert_eq!(error.message,error::Error::refusal("unsupported_layout").message);assert_eq!(reads.get(),0);
    assert_eq!(fs::read_link(&path).unwrap(),external);let after=fs::symlink_metadata(&path).unwrap();assert_eq!((metadata.ino(),metadata.mode(),metadata.uid(),metadata.gid()),(after.ino(),after.mode(),after.uid(),after.gid()));
    fs::remove_file(&path).unwrap();assert_eq!(before,(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap(),files::snapshot(&external).unwrap()));
}
#[test]
fn symlinked_wal_is_refused_unchanged_with_a_safe_diagnostic() {
    cluster_link_refusal("pg_wal","symlinked_wal_is_refused_unchanged_with_a_safe_diagnostic","symbolic link in cluster: pg_wal");
}
#[test]
fn tablespace_link_is_refused_unchanged_with_a_safe_diagnostic() {
    cluster_link_refusal("pg_tblspc/16384","tablespace_link_is_refused_unchanged_with_a_safe_diagnostic","tablespace link: pg_tblspc/16384");
}

#[test]
fn runner_key_agreement_is_readonly_until_the_flag_step() {
    let t=tempfile::tempdir().unwrap();let r=roots(t.path());let original=fresh_ready_without_key(&r);
    let key=fixture_key(&r.app.join("secrets/data_key"));
    let locks=upgrade::StorageLocks::acquire(&r.db,&r.app).unwrap();
    let before=(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap());
    let verified=key_binding::verify_key_locked(&locks,&r,&|_|Ok("7382591915349309999".into()),&key).unwrap();
    assert_eq!(verified.pair_id,original.pair_id);assert!(!verified.encryption_key_required);
    assert_eq!(before,(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap()));
    let wrong=crypto::SecretBox::from_key([19;32]);
    assert_eq!(key_binding::verify_key_locked(&locks,&r,&|_|Ok("7382591915349309999".into()),&wrong).unwrap_err().code,"pair_mismatch");
    assert_eq!(before,(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap()));
    key_binding::mark_key_required_locked(&locks,&r,&|_|Ok("7382591915349309999".into()),&key).unwrap();
    for root in [&r.db,&r.app] {let saved:Pair=files::read_json(&root.join(".nwui/pair-v1.json")).unwrap();assert!(saved.encryption_key_required);}
}
