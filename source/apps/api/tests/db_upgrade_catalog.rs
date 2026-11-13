#[path="../src/db_upgrade/state.rs"] mod state;
#[path="../src/db_upgrade/error.rs"] mod error;
#[path="../src/db_upgrade/catalog.rs"] mod catalog;
fn database(name:&str)->catalog::Database {
    catalog::Database{name:name.into(),owner:if name.starts_with("nginx_"){"nginx".into()}else{"postgres".into()},encoding:"UTF8".into(),collate:"C".into(),ctype:"C".into(),provider:"c".into(),icu:None}
}
fn databases()->Vec<catalog::Database>{["postgres","template0","template1","nginx_web_ui"].map(database).into()}
fn roles()->Vec<catalog::Role>{vec![catalog::Role{name:"postgres".into(),oid:"10".into(),login:true,elevated:true,password_sha256:None},catalog::Role{name:"nginx".into(),oid:"16384".into(),login:true,elevated:false,password_sha256:Some("a".repeat(64))}]}
#[test]
fn supported_profile_requires_one_application_database_and_preserves_legacy_identity() {
    assert_eq!(catalog::validate(&databases(),&roles(),false).unwrap(),Some("nginx_web_ui".into()));
    let mut db=databases();db.last_mut().unwrap().name="nginx_control".into();
    assert_eq!(catalog::validate(&db,&roles(),false).unwrap(),Some("nginx_control".into()));
    assert!(catalog::validate(&db,&roles(),true).is_err());
    db.push(database("nginx_web_ui"));assert!(catalog::validate(&db,&roles(),false).is_err());
}
#[test]
fn empty_fresh_profile_can_resume_role_creation_but_never_adopts_foreign_objects_or_roles() {
    let mut db=databases();db.pop();let mut rs=roles();rs.pop();
    assert_eq!(catalog::validate(&db,&rs,true).unwrap(),None);
    assert!(catalog::validate(&db,&rs,false).is_err());rs=roles();assert!(catalog::validate(&db,&rs,true).is_ok());
    for bad in ["database","role","owner","privilege","identity","provider"] {
        let mut db=databases();let mut rs=roles();
        match bad {
            "database"=>db.push(database("foreign_database")),
            "role"=>rs.push(catalog::Role{name:"foreign_role".into(),..rs[1].clone()}),
            "owner"=>db.last_mut().unwrap().owner="postgres".into(),
            "privilege"=>rs[1].elevated=true,
            "identity"=>rs[1].oid="bad".into(),
            _=>db[0].provider="unknown".into(),
        }
        assert_eq!(catalog::validate(&db,&rs,false).unwrap_err().code,"unsupported_layout","{bad}");
    }
}
