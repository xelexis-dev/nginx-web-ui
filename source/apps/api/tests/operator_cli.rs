use std::{fs, process::{Command,Output}};

fn invoke(args:&[&str],root:&std::path::Path)->Output {
    Command::new(env!("CARGO_BIN_EXE_nwui-api")).arg("db-upgrade").args(args)
        .env_clear().env("PGROOT",root).env("PGDATA",root.join("17/data"))
        .env("DATABASE_URL","invalid:must-not-load-config").output().unwrap()
}

#[test]
fn operator_cli_rejects_missing_or_wrong_confirmation_without_changing_storage() {
    let temp=tempfile::tempdir().unwrap();fs::write(temp.path().join("canary"),b"unchanged").unwrap();
    for (command,wrong) in [("restore-pg15","--confirm"),("remove-pg15-backup","--confirm-loss"),("remove-discarded","--confirm-loss")] {
        for args in [vec![command],vec![command,wrong,"11111111-1111-4111-8111-111111111111"]] {
            let output=invoke(&args,temp.path());assert!(!output.status.success());
            let stderr=String::from_utf8(output.stderr).unwrap();assert!(stderr.contains("invalid_arguments"),"{command}: {stderr}");
            assert!(stderr.contains("restore-pg15 --confirm-loss"),"{command}: {stderr}");
            assert_eq!(fs::read(temp.path().join("canary")).unwrap(),b"unchanged");assert_eq!(fs::read_dir(temp.path()).unwrap().count(),1);
        }
    }
}

#[test]
fn operator_cli_unknown_command_prints_complete_usage() {
    let temp=tempfile::tempdir().unwrap();let output=invoke(&["unknown-command"],temp.path());assert!(!output.status.success());
    let stderr=String::from_utf8(output.stderr).unwrap();
    for expected in ["Usage:","check|run","auto-status","restore-pg15 --confirm-loss","remove-pg15-backup --confirm","remove-discarded --confirm"] {assert!(stderr.contains(expected),"missing {expected}: {stderr}");}
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(),0);
}
