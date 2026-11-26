#[path="../src/db_upgrade/settings.rs"] mod settings;
pub use nwui_api::crypto;
#[path="../src/db_upgrade/state.rs"] mod state;
#[path="../src/db_upgrade/error.rs"] mod error;
#[path="../src/db_upgrade/catalog.rs"] mod catalog;
#[path="../src/db_upgrade/inspect.rs"] mod inspect;
#[path="../src/db_upgrade/cluster.rs"] mod cluster;
use std::collections::BTreeMap;
fn sample(application:&str)->cluster::Snapshot {
    let databases=["postgres","template0","template1",application].into_iter().map(|name|catalog::Database{name:name.into(),owner:if name==application{"nginx".into()}else{"postgres".into()},encoding:"UTF8".into(),collate:"C".into(),ctype:"C".into(),provider:"c".into(),icu:None}).collect::<Vec<_>>();
    let roles=vec![catalog::Role{name:"postgres".into(),oid:"10".into(),login:true,elevated:true,password_sha256:None},catalog::Role{name:"nginx".into(),oid:"16384".into(),login:true,elevated:false,password_sha256:Some("b".repeat(64))}];
    let mut data=BTreeMap::new();
    for db in &databases {data.insert(db.name.clone(),inspect::DatabaseSnapshot{tables:BTreeMap::new(),sequences:BTreeMap::new(),schema_sha256:"a".repeat(64),has_encrypted:false});}
    data.get_mut(application).unwrap().tables.insert("public.users".into(),inspect::Table{rows:1,sha256:"c".repeat(64)});
    cluster::Snapshot{databases,roles,data,collations:BTreeMap::new(),largest_index_bytes:10}
}
#[test]
fn only_the_documented_legacy_database_name_is_normalized_for_logical_comparison() {
    let source=sample("nginx_control");let mut target=sample("nginx_web_ui");target.largest_index_bytes=20;
    source.require_equivalent(&target).unwrap();
    target.databases.last_mut().unwrap().name="foreign".into();assert!(source.require_equivalent(&target).is_err());
}
#[test]
fn missing_database_or_changed_role_locale_rows_and_secrets_never_verify() {
    let source=sample("nginx_web_ui");
    for changed in ["missing","role","locale","rows","key"] {
        let mut target=sample("nginx_web_ui");
        match changed {
            "missing"=>{target.data.remove("template0");},
            "role"=>target.roles[1].oid="16385".into(),
            "locale"=>target.databases.last_mut().unwrap().collate="different".into(),
            "rows"=>target.data.get_mut("nginx_web_ui").unwrap().tables.get_mut("public.users").unwrap().sha256="d".repeat(64),
            _=>target.roles[1].password_sha256=Some("e".repeat(64)),
        }
        assert!(source.require_equivalent(&target).is_err(),"{changed}");
    }
}
