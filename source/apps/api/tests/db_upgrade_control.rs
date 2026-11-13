#[path="../src/db_upgrade/state.rs"] mod state;
#[path="../src/db_upgrade/error.rs"] mod error;
#[path="../src/db_upgrade/control.rs"] mod control;
use control::Control;
const SOURCE:&str=include_str!("../src/db_upgrade/examples/control-15.txt");
#[test]
fn compatible_control_data_does_not_pretend_to_record_source_cpu_architecture() {
    let old=Control::parse(SOURCE,"").unwrap();let target_text=SOURCE.replace("7382591915349309145","7382591915349309999");let new=Control::parse(&target_text,"").unwrap();
    old.compatible_with(&new).unwrap();assert_ne!(old.system_identifier,new.system_identifier);assert_eq!(old.cluster_state,"shut down");old.require_cold().unwrap();
}
#[test]
fn every_required_control_property_is_compared_and_missing_or_duplicate_values_refuse() {
    let old=Control::parse(SOURCE,"").unwrap();
    for label in control::COMPATIBILITY_FIELDS {
        let line=SOURCE.lines().find(|line|line.starts_with(label)).unwrap();
        let changed=SOURCE.replace(line,&format!("{label}: incompatible"));
        let result=Control::parse(&changed,"").and_then(|new|old.compatible_with(&new));
        assert_eq!(result.unwrap_err().code,"unsupported_layout","{label}");
        let missing=SOURCE.replace(line,"");assert!(Control::parse(&missing,"").is_err(),"{label}");
        let duplicated=format!("{SOURCE}{line}\n");assert!(Control::parse(&duplicated,"").is_err(),"{label}");
    }
}
#[test]
fn synthetic_tampered_control_output_copy_is_refused_and_original_is_unchanged() {
    let t=tempfile::tempdir().unwrap();let original=t.path().join("source-control.txt");let copy=t.path().join("tampered-copy.txt");std::fs::write(&original,SOURCE).unwrap();std::fs::copy(&original,&copy).unwrap();
    let tampered=std::fs::read_to_string(&copy).unwrap().replace("Database block size:                  8192","Database block size:                  4096");std::fs::write(&copy,tampered).unwrap();
    let source=Control::parse(&std::fs::read_to_string(&original).unwrap(),"").unwrap();let target=Control::parse(&std::fs::read_to_string(&copy).unwrap(),"").unwrap();
    assert_eq!(source.compatible_with(&target).unwrap_err().code,"unsupported_layout");assert_eq!(std::fs::read_to_string(&original).unwrap(),SOURCE);
}
#[test]
fn crc_warnings_and_a_dirty_source_cannot_authorize_conversion() {
    assert!(Control::parse(SOURCE,"pg_controldata: warning: CRC mismatch").is_err());
    let dirty=SOURCE.replace("shut down","in production");assert_eq!(Control::parse(&dirty,"").unwrap().require_cold().unwrap_err().code,"unsupported_layout");
}
