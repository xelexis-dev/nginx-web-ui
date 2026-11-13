#[path="../src/db_upgrade/state.rs"] mod state;
#[path="../src/db_upgrade/error.rs"] mod error;
#[path="../src/db_upgrade/arguments.rs"] mod arguments;
use arguments::{Arguments,Operation};
fn parse(args:&[&str])->Result<Arguments,error::Error>{Arguments::parse(args.iter().map(|s|s.to_string()))}
#[test]
fn status_and_verify_cannot_accept_path_or_backup_options() {
    for name in ["status","verify"] {
        let a=parse(&[name,"--json"]).unwrap();assert!(a.json);assert!(a.destinations.is_none());assert!(a.backup_id.is_none());
        assert!(parse(&[name,"--backup-id","backup"]).is_err());
    }
    for bad in [vec![],vec!["--help"],vec!["status","--json","--json"],vec!["status","--force"],vec!["status","--json=true"]]{assert!(parse(&bad).is_err());}
}
#[test]
fn check_and_run_require_explicit_distinct_display_targets_and_backup() {
    for op in ["check","run"] {
        let a=parse(&[op,"--backup-id","cold-2026","--destination-db-volume","new-db","--destination-app-bind","/safe/new-app","--json"]).unwrap();
        assert!(matches!(a.operation,Operation::Check|Operation::Run));assert!(a.json);assert_eq!(a.backup_id.as_deref(),Some("cold-2026"));
        for bad in [
            vec![op],
            vec![op,"--backup-id","x","--destination-db-volume","same","--destination-app-volume","same"],
            vec![op,"--backup-id","x","--destination-db-volume","db","--destination-app-bind","relative"],
            vec![op,"--backup-id","x","--destination-db-volume","db","--destination-app-bind","/safe/../escape"],
            vec![op,"--backup-id","x","--destination-db-volume","db","--destination-db-bind","/other","--destination-app-volume","app"],
            vec![op,"--backup-id","secret\nvalue","--destination-db-volume","db","--destination-app-volume","app"],
        ]{assert!(parse(&bad).is_err(),"{bad:?}");}
    }
}
