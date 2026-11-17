mod auth_support;
use auth_support::*;
use nwui_api::{contracts::{Actor,FormInput},control::{resources::{self,Resource,Mutation},reserved_ports::ReservedPorts,validation},error::ApiError};
use serde_json::{Value,json};
fn server()->Value{json!({"serverName":"strict.test","listen":18081,"targetMode":"locations","locations":[{"path":"/","mode":"direct","proxyPass":"http://127.0.0.1:19000"}]})}
fn parse(v:&Value)->Result<Value,ApiError>{resources::parse_server(v,&ReservedPorts::default())}
fn bad(e:ApiError,field:&str){assert_eq!(e.status,400);assert!(e.field_errors.as_ref().is_some_and(|m|m.contains_key(field)),"{field}: {e:?}");}
fn form()->FormInput{let mut f=FormInput::default();for(k,v)in [("serverName","strict.test"),("listen","18081"),("targetMode","locations"),("locations",r#"[{"path":"/","mode":"direct","proxyPass":"http://127.0.0.1:19000"}]"#)]{f.fields.insert(k.into(),vec![v.into()]);}f}
#[test]
fn fx_a_json_syntax_errors_are_field_errors_in_local_and_node_forms(){
 let mut accepted=vec![];
 for field in ["ipAccess","realIp","tlsOptions","locations","cache","gzip","serverDirectives"]{
  let mut f=form();f.fields.insert(field.into(),vec!["{broken".into()]);
  match resources::form_body(Resource::Server,&f){Err(e)=>bad(e,field),Ok(_)=>accepted.push(field)}
  f.fields.insert("id".into(),vec!["server".into()]);f.args=vec!["node".into()];
  match nwui_api::nodes::relay::command("node.updateNodeServer","node",&f){Err(e)=>bad(e,field),Ok(_)=>accepted.push(field)}
 }
 assert!(accepted.is_empty(),"silently accepted malformed local/relay fields: {accepted:?}");
 for field in ["ipAccess","realIp","tlsOptions"]{let mut f=form();f.fields.insert(field.into(),vec!["  ".into()]);assert!(resources::form_body(Resource::Server,&f).unwrap()[field].is_null());}
}
#[test]
fn fx_a_server_and_route_acl_reject_wrong_shapes_instead_of_disabling(){
 for invalid in [json!("invalid"),json!(false),json!({}),json!({"mode":"whitelst","entries":["10.0.0.0/8"]}),json!({"mode":"whitelist","entries":"10.0.0.0/8"}),json!({"mode":"whitelist","entries":[]}),json!({"mode":"whitelist","entries":[12]}),json!({"mode":"whitelist","entries":[null]})]{
  let mut b=server();b["ipAccess"]=invalid.clone();bad(parse(&b).expect_err("invalid ACL accepted"),"ipAccess");
  b=server();b["locations"][0]["ipAccess"]=invalid;bad(parse(&b).expect_err("invalid route ACL accepted"),"locations");
 }
 for off in [Value::Null]{let mut b=server();b["ipAccess"]=off;assert!(parse(&b).unwrap()["config"].get("ipAccess").is_none());}
}
#[test]
fn fx_a_real_ip_preset_header_and_ranges_are_strict(){
 for invalid in [json!(false),json!({"preset":"cloudfare"}),json!({"preset":"custom","trustedRanges":["10.0.0.0/8"],"header":"X-Fowarded-For"}),json!({"preset":"custom","trustedRanges":"10.0.0.0/8"}),json!({"preset":"custom","trustedRanges":["10.0.0.0/8"],"recursive":"true"})]{assert!(validation::parse_real_ip_config(&invalid).is_err(),"accepted {invalid}");}
 assert!(validation::parse_real_ip_config(&Value::Null).unwrap().is_none());
 assert_eq!(validation::parse_real_ip_config(&json!({"preset":"custom","trustedRanges":["10.0.0.0/8"]})).unwrap().unwrap()["header"],validation::DEFAULT_REAL_IP_HEADER);
}
#[test]
fn fx_a_tls_options_and_certificate_id_never_silently_downgrade(){
 for invalid in [json!(false),json!([]),json!({"sslProtocols":"moden"}),json!({"sslCiphers":"unknown"}),json!({"hsts":"false"}),json!({"hsts":{"enabled":"true"}}),json!({"preferServerCiphers":"false"}),json!({"hsts":{"enabled":true,"includeSubDomains":"false"}})]{
  let mut b=server();b["certificateId"]=json!("cert");b["tls"]=json!({"tlsOptions":invalid});bad(parse(&b).expect_err("invalid TLS accepted"),"tlsOptions");
 }
 for invalid in [json!(0),json!(false),json!({}),json!([])]{let mut b=server();b["certificateId"]=invalid;bad(parse(&b).expect_err("nonstring certificate accepted"),"certificateId");}
 for off in [Value::Null,json!("")]{let mut b=server();b["certificateId"]=off;assert!(parse(&b).unwrap()["certificateId"].is_null());}
}
#[test]
fn fx_a_nul_strings_are_rejected_before_database_or_render(){
 for (field,value) in [("certificateId",json!("c\0id")),("unknown",json!("x\0y")),("locations",json!([{"path":"/","mode":"direct","proxyPass":"http://127.0.0.1:19000","directives":[{"name":"add_header","args":["X-Test","x\0y"]}]}]))]{let mut b=server();b[field]=value;bad(parse(&b).expect_err("NUL accepted"),field);}
 let mut f=form();f.fields.insert("certificateId".into(),vec!["c\0id".into()]);bad(resources::form_body(Resource::Server,&f).expect_err("form NUL accepted"),"certificateId");
 assert!(validation::assert_valid_directive("add_header",&["X-Test".into(),"x\0y".into()],validation::DirectiveContext::Server).is_err());
}
#[test]
fn fx_a_direct_proxy_targets_reject_interior_backslashes_and_controls(){
 for target in ["http://127.0.0.1:19000/a\\b","http://example.test\\@127.0.0.1:19000/","http://127.0.0.1:19000/a\u{7f}b"]{assert!(validation::assert_valid_proxy_target(target).is_err(),"accepted {target:?}");let mut b=server();b["locations"][0]["proxyPass"]=json!(target);bad(parse(&b).expect_err("bad proxy target accepted"),"locations");}
 assert!(validation::assert_valid_proxy_target("http://127.0.0.1:19000/$1?q=$arg_q").is_ok());
 assert!(validation::assert_valid_proxy_target("http://127.0.0.1:19000/a%5Cb").is_ok());
 assert!(validation::assert_valid_directive("rewrite",&["^/a\\.txt$".into(),"/b".into(),"last".into()],validation::DirectiveContext::Location).is_ok());
}
fn actor()->Actor{Actor{via: None,id:None,label:Some("fx-a".into()),ip:None}}
async fn write(db:&TestDb,op:Mutation,id:Option<&str>,v:Value)->Result<Value,ApiError>{resources::mutate(&db.state,Resource::Server,op,id,v,&actor()).await}
#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn fx_a_rejected_security_edits_leave_stored_acl_tls_and_hsts_unchanged(){
 let db=test_db().await;insert_user(&db.client,UserRow{role:"admin",..Default::default()},None).await;db.client.batch_execute("INSERT INTO certificates(id,domains,status) VALUES('cert','[\"strict.test\"]','issued')").await.unwrap();
 let mut original=server();original["certificateId"]=json!("cert");original["tls"]=json!({"tlsOptions":{"hsts":{"enabled":true}}});original["ipAccess"]=json!({"mode":"whitelist","entries":["10.0.0.0/8"]});original["realIp"]=json!({"preset":"custom","trustedRanges":["192.0.2.0/24"],"header":"X-Real-IP"});let row=write(&db,Mutation::Create,None,original.clone()).await.unwrap();let id=row["id"].as_str().unwrap();
 for (field,value)in [("ipAccess",json!({"mode":"whitelst","entries":["10.0.0.0/8"]})),("realIp",json!({"preset":"cloudfare"})),("tls",json!({"tlsOptions":{"hsts":{"enabled":"true"}}})),("certificateId",json!(12)),("certificateId",json!("x\0y"))]{let mut b=original.clone();b[field]=value;let e=write(&db,Mutation::Update,Some(id),b).await.expect_err("protection was silently changed");assert_eq!(e.status,400);assert_eq!(resources::get(&db.client,Resource::Server,id).await.unwrap(),row);}
 // Browser actions preserve the existing HTTP200 FormResult failure envelope.
 use tower::ServiceExt;
 let app=nwui_api::app::with_request_context(nwui_api::control::routes::router().with_state(db.state.clone()),None);
 for field in ["ipAccess","realIp","tlsOptions"] {
  let mut fields=vec![("id",id),("serverName","strict.test"),("listen","18081"),("targetMode","locations"),("locations",r#"[{"path":"/","mode":"direct","proxyPass":"http://127.0.0.1:19000"}]"#),("certificateId","cert"),("ipAccess",r#"{"mode":"whitelist","entries":["10.0.0.0/8"]}"#),("realIp",r#"{"preset":"custom","trustedRanges":["192.0.2.0/24"],"header":"X-Real-IP"}"#),("tlsOptions",r#"{"hsts":{"enabled":true}}"#)];
  fields.iter_mut().find(|(key,_)|*key==field).unwrap().1="{broken";
  let response=app.clone().oneshot(request("reverse-proxy.updateServer",&fields)).await.unwrap();assert_eq!(response.status(),200);
  let bytes=axum::body::to_bytes(response.into_body(),1<<20).await.unwrap();let result:Value=serde_json::from_slice(&bytes).unwrap();assert_eq!(result["result"]["ok"],false);assert!(result["result"]["fieldErrors"][field].is_string());assert_eq!(resources::get(&db.client,Resource::Server,id).await.unwrap(),row);
 }
 let mut cleared=server();cleared["certificateId"]=Value::Null;let after=write(&db,Mutation::Update,Some(id),cleared).await.unwrap();assert!(after["config"].get("ipAccess").is_none());assert!(after["certificateId"].is_null());
 bad(resources::get(&db.client,Resource::Server,"id\0bad").await.expect_err("NUL id reached DB"),"id");
}
#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn fx_a_certificate_selection_requires_issued_and_preserves_previous_binding(){
 let db=test_db().await;db.client.batch_execute("INSERT INTO certificates(id,domains,status) VALUES('issued','[\"strict.test\"]','issued'),('pending','[\"strict.test\"]','issuing'),('failed','[\"strict.test\"]','failed'),('dns','[\"strict.test\"]','awaiting-dns')").await.unwrap();let mut b=server();b["certificateId"]=json!("issued");let row=write(&db,Mutation::Create,None,b.clone()).await.unwrap();let id=row["id"].as_str().unwrap();
 for cert in ["pending","failed","dns","missing"]{b["certificateId"]=json!(cert);bad(write(&db,Mutation::Update,Some(id),b.clone()).await.expect_err("unissued certificate accepted"),"certificateId");assert_eq!(resources::get(&db.client,Resource::Server,id).await.unwrap(),row);}
}
fn cookie()->String{let now=nwui_api::auth::principal::now_unix_ms()/1000;let v=json!({"id":"u1","sub":"u1","role":"admin","sv":3,"requires2fa":false});let t=nwui_api::auth::session::encode(v.as_object().unwrap(),SECRET,nwui_api::auth::session::COOKIE_NAME_PLAIN,now,3600).unwrap();format!("authjs.session-token={t}")}
fn request(command:&str,fields:&[(&str,&str)])->axum::http::Request<axum::body::Body>{let mut s=url::form_urlencoded::Serializer::new(String::new());for(k,v)in fields{s.append_pair(k,v);}axum::http::Request::builder().method("POST").uri(format!("/api/ui/v1/forms/{command}")).header("host","admin.example.test").header("origin","http://admin.example.test").header("cookie",cookie()).header("content-type","application/x-www-form-urlencoded").extension(axum::extract::ConnectInfo("127.0.0.1:44000".parse::<std::net::SocketAddr>().unwrap())).body(axum::body::Body::from(s.finish())).unwrap()}
#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn fx_a_invalid_2fa_switch_cannot_disable_existing_policy(){
 use tower::ServiceExt;let db=test_db().await;insert_user(&db.client,UserRow{role:"admin",..Default::default()},None).await;db.client.batch_execute("INSERT INTO app_settings(id,require_2fa_oauth) VALUES('current',true) ON CONFLICT(id) DO UPDATE SET require_2fa_oauth=true").await.unwrap();
 let app=nwui_api::app::with_request_context(nwui_api::admin::router().with_state(db.state.clone()),None);
 for field in ["require2faOauth","require2faAdmin","require2faUser"]{for invalid in ["True","on","1","","false "]{let mut fields=vec![("require2faAdmin","false"),("require2faUser","false"),("require2faOauth","true")];fields.iter_mut().find(|(k,_)|*k==field).unwrap().1=invalid;let response=app.clone().oneshot(request("admin.security.updateTwoFactorPolicy",&fields)).await.unwrap();let row=db.client.query_one("SELECT require_2fa_admin,require_2fa_user,require_2fa_oauth FROM app_settings WHERE id='current'",&[]).await.unwrap();let after=(row.get::<_,bool>(0),row.get::<_,bool>(1),row.get::<_,bool>(2));assert_eq!(response.status(),400,"{field}={invalid}; policy after={after:?}");assert_eq!(after,(false,false,true));}}
}
#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn fx_a_node_relay_malformed_json_never_reaches_remote_write(){
 use axum::{Router,Json,extract::State,routing::patch};use std::sync::{Arc,atomic::{AtomicUsize,AtomicBool,Ordering}};use tower::ServiceExt;
 let db=test_db().await;insert_user(&db.client,UserRow{role:"admin",..Default::default()},None).await;
 let count=Arc::new(AtomicUsize::new(0));let protected=Arc::new(AtomicBool::new(true));let version=db.state.config.version.clone();
 let remote=Router::new().route("/api/v1/servers/server",patch(|State((count,protected,version)):State<(Arc<AtomicUsize>,Arc<AtomicBool>,String)>,Json(value):Json<Value>|async move{count.fetch_add(1,Ordering::SeqCst);if value["ipAccess"].is_null(){protected.store(false,Ordering::SeqCst);}([("x-node-version",version)],Json(json!({"server":{"id":"server"}})))})).with_state((count.clone(),protected.clone(),version));
 let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap();let task=tokio::spawn(async move{axum::serve(listener,remote).await.unwrap()});let sealed=db.state.secrets.seal("remote-token").unwrap();db.client.execute("INSERT INTO nodes(id,name,base_url,token_enc,token_role,instance_id) VALUES('n','remote',$1,$2,'admin','instance')",&[&format!("http://{addr}"),&sealed]).await.unwrap();
 let app=nwui_api::app::with_request_context(nwui_api::nodes::relay::router().with_state(db.state.clone()),None);
 let fields=[("_apiArgs",r#"["n"]"#),("id","server"),("serverName","strict.test"),("listen","18081"),("targetMode","locations"),("locations",r#"[{"path":"/","mode":"direct","proxyPass":"http://127.0.0.1:19000"}]"#),("ipAccess","{broken")];
 let response=app.oneshot(request("node.updateNodeServer",&fields)).await.unwrap();let bytes=axum::body::to_bytes(response.into_body(),1<<20).await.unwrap();let result:Value=serde_json::from_slice(&bytes).unwrap();
 assert_eq!(count.load(Ordering::SeqCst),0,"remote mutation happened: {result}");assert!(protected.load(Ordering::SeqCst));assert_eq!(result["result"]["ok"],false);assert!(result["result"]["fieldErrors"]["ipAccess"].is_string());task.abort();
}

#[test]
fn fx_a_a4_config_parameter_lengths_have_safe_byte_boundaries() {
    let dns253 = ["a".repeat(63),"b".repeat(63),"c".repeat(63),"d".repeat(61)].join(".");
    let mut body=server();body["serverName"]=json!(dns253.clone());assert!(parse(&body).is_ok());
    body["serverName"]=json!(dns253.clone()+"x");bad(parse(&body).expect_err("254-byte name stored"),"serverName");
    let total4096=vec![dns253;16].join(" ")+" "+&"a".repeat(32);
    assert_eq!(total4096.len(),4096);body=server();body["serverName"]=json!(total4096.clone());assert!(parse(&body).is_ok());
    body["serverName"]=json!(total4096+"x");bad(parse(&body).expect_err("oversize multi-name value stored"),"serverName");
    for length in [2048,2049] {
        let path="/".to_owned()+&"x".repeat(length-1);
        body=server();body["locations"][0]["path"]=json!(path);assert_eq!(parse(&body).is_ok(),length==2048);
        let prefix="http://127.0.0.1:19000/";let target=prefix.to_owned()+&"x".repeat(length-prefix.len());
        body=server();body["locations"][0]["proxyPass"]=json!(target);assert_eq!(parse(&body).is_ok(),length==2048);
        for (name,args) in [("add_header",vec!["X-Limit".to_owned(),"x".repeat(length)]),("rewrite",vec!["^".to_owned(),"x".repeat(length),"last".to_owned()])] {
            body=server();body["locations"][0]["directives"]=json!([{"name":name,"args":args}]);assert_eq!(parse(&body).is_ok(),length==2048);
        }
    }
    body=server();body["serverDirectives"]=json!([{"name":"add_header","args":["X-Limit","가".repeat(683)]}]);bad(parse(&body).expect_err("multibyte value exceeded byte limit"),"serverDirectives");
    for size in [46,47] {body=server();body["locations"][0]["directives"]=json!([{"name":"proxy_set_header","args":["X".repeat(size),"ok"]}]);assert_eq!(parse(&body).is_ok(),size==46);}
    for size in [46,47] {body=server();body["locations"][0]["directives"]=json!([{"name":"proxy_hide_header","args":["X".repeat(size)]}]);assert_eq!(parse(&body).is_ok(),size==46,"hidden header {size}");}
    body=server();body["locations"][0]["directives"]=json!([{"name":"add_header","args":["X".repeat(2048),"ok"]}]);assert!(parse(&body).is_ok(),"response headers do not use the proxy header hash");
    for size in [2041,2042] {assert_eq!(resources::parse_upstream(&json!({"name":"U".repeat(size),"servers":["127.0.0.1:19000"]})).is_ok(),size==2041);}
    let oversized="127.0.0.1:19000 weight=".to_owned()+&"0".repeat(2048)+"1";
    bad(resources::parse_upstream(&json!({"name":"up","servers":[oversized]})).expect_err("oversize upstream parameter stored"),"servers");
}

#[test]
fn fx_a_a4_save_limits_do_not_quarantine_valid_legacy_rendering() {
    let path="/".to_owned()+&"p".repeat(2999);
    let prefix="http://127.0.0.1:19000/";let target=prefix.to_owned()+&"t".repeat(3000-prefix.len());
    let legacy=json!({"serverName":"legacy.test","listen":18100,"locations":[{"path":path,"proxyPass":target,"directives":[{"name":"add_header","args":["X-Legacy","v".repeat(3000)]}]}]});
    let rendered=nwui_api::control::render::render_reverse_proxy(&legacy).expect("save-only bounds must not rewrite or reject legacy rendering");
    assert!(rendered.contains(&"v".repeat(3000)));
}

#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn fx_a_ops1_server_alias_and_tls_conflicts_are_scoped_and_atomic() {
    let db=test_db().await;
    db.client.batch_execute("INSERT INTO certificates(id,domains,status) VALUES('cert','[\"shared.test\"]','issued')").await.unwrap();
    let mut first=server();first["serverName"]=json!("Alpha.test SHARED.test");first["listen"]=json!(18110);first["certificateId"]=json!("cert");first["tls"]=json!({"tlsPort":18410});
    let original=write(&db,Mutation::Create,None,first.clone()).await.unwrap();let id=original["id"].as_str().unwrap();
    let mut other=first.clone();other["serverName"]=json!("other.test");let other_row=write(&db,Mutation::Create,None,other.clone()).await.unwrap();
    for (name,listen,tls,field) in [("shared.TEST extra.test",18110,18411,"listen"),("alpha.TEST",18111,18410,"tlsPort")] {
        let mut candidate=first.clone();candidate["serverName"]=json!(name);candidate["listen"]=json!(listen);candidate["tls"]["tlsPort"]=json!(tls);
        let error=write(&db,Mutation::Create,None,candidate).await.expect_err("overlapping listener accepted");
        assert!(error.field_errors.as_ref().unwrap()[field].contains("Alpha.test"));bad(error,field);
    }
    let mut changed=other.clone();changed["serverName"]=json!("alpha.test");
    bad(write(&db,Mutation::Update,other_row["id"].as_str(),changed).await.expect_err("conflicting edit accepted"),"listen");
    assert_eq!(resources::get(&db.client,Resource::Server,other_row["id"].as_str().unwrap()).await.unwrap(),other_row);
    assert!(write(&db,Mutation::Update,Some(id),first.clone()).await.is_ok(),"self edit conflicts with itself");
    let mut different=first.clone();different["listen"]=json!(18112);different["tls"]["tlsPort"]=json!(18412);assert!(write(&db,Mutation::Create,None,different).await.is_ok());
    let mut wildcard=server();wildcard["serverName"]=json!("*.Example.test");wildcard["listen"]=json!(18113);let wildcard_row=write(&db,Mutation::Create,None,wildcard.clone()).await.unwrap();wildcard["serverName"]=json!("*.example.TEST");
    let error=write(&db,Mutation::Create,None,wildcard).await.expect_err("duplicate wildcard accepted");assert!(error.field_errors.as_ref().unwrap()["listen"].contains("*.Example.test"));bad(error,"listen");
    // Old stored TLS rows without tlsPort still reserve the rendered default 443.
    let mut legacy=first.clone();legacy["serverName"]=json!("default-tls.test");legacy["listen"]=json!(18114);legacy["tls"]=json!({"tlsPort":18414});
    let legacy_row=write(&db,Mutation::Create,None,legacy.clone()).await.unwrap();
    db.client.execute("UPDATE nginx_servers SET config=config-'tlsPort' WHERE id=$1",&[&legacy_row["id"].as_str().unwrap()]).await.unwrap();
    legacy["listen"]=json!(18115);legacy["tls"]["tlsPort"]=json!(443);
    bad(write(&db,Mutation::Create,None,legacy).await.expect_err("legacy default TLS overlap accepted"),"tlsPort");
    // Existing duplicate rows retain their historical activation behavior.
    db.client.execute("UPDATE nginx_servers SET server_name='Alpha.test',enabled=false WHERE id=$1",&[&other_row["id"].as_str().unwrap()]).await.unwrap();
    resources::mutate(&db.state,Resource::Server,Mutation::Toggle,other_row["id"].as_str(),json!({}),&actor()).await.expect("legacy duplicate activation changed");
    assert_eq!(resources::get(&db.client,Resource::Server,other_row["id"].as_str().unwrap()).await.unwrap()["enabled"],true);
    // A separate node's database owns its own namespace.
    let node=test_db().await;let mut same=server();same["serverName"]=original["serverName"].clone();same["listen"]=original["listen"].clone();assert!(write(&node,Mutation::Create,None,same).await.is_ok());
    assert!(resources::get(&db.client,Resource::Server,wildcard_row["id"].as_str().unwrap()).await.is_ok());
}

#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn fx_a_ops1_stream_protocol_ports_and_self_edit_are_distinct() {
    let db=test_db().await;let raw=json!({"listen":18120,"protocol":"tcp","proxyPass":"127.0.0.1:19000"});
    let row=resources::mutate(&db.state,Resource::Stream,Mutation::Create,None,raw.clone(),&actor()).await.unwrap();
    let error=resources::mutate(&db.state,Resource::Stream,Mutation::Create,None,raw.clone(),&actor()).await.expect_err("duplicate TCP stream accepted");
    assert!(error.field_errors.as_ref().unwrap()["listen"].contains(row["id"].as_str().unwrap()));bad(error,"listen");
    assert!(resources::mutate(&db.state,Resource::Stream,Mutation::Update,row["id"].as_str(),raw.clone(),&actor()).await.is_ok());
    let mut udp=raw.clone();udp["protocol"]=json!("udp");assert!(resources::mutate(&db.state,Resource::Stream,Mutation::Create,None,udp.clone(),&actor()).await.is_ok());
    bad(resources::mutate(&db.state,Resource::Stream,Mutation::Create,None,udp,&actor()).await.expect_err("duplicate UDP stream accepted"),"listen");
    let mut http=server();http["listen"]=json!(18120);bad(write(&db,Mutation::Create,None,http).await.expect_err("HTTP/TCP stream overlap accepted"),"listen");
}

#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn fx_a_ops1_concurrent_duplicate_saves_cannot_both_commit() {
    let db=test_db().await;let mut b=server();b["serverName"]=json!("concurrent.test");b["listen"]=json!(18121);
    let (first,second)=tokio::join!(write(&db,Mutation::Create,None,b.clone()),write(&db,Mutation::Create,None,b.clone()));
    let (winner,loser)=if first.is_ok(){(first,second)}else{(second,first)};assert!(winner.is_ok());
    let error=loser.expect_err("both duplicates committed");assert!(matches!(error.status.as_u16(),400|503));
    bad(write(&db,Mutation::Create,None,b).await.expect_err("retry admitted duplicate"),"listen");
    let count:i64=db.client.query_one("SELECT count(*) FROM nginx_servers WHERE listen=18121",&[]).await.unwrap().get(0);assert_eq!(count,1);
}

#[test]
fn fx_a_compat_legacy_real_ip_clear_values_remain_clear() {
    for value in [json!({}),json!({"preset":"none"})] {
        assert!(validation::parse_real_ip_config(&value).unwrap().is_none());
        let mut b=server();b["realIp"]=value;assert!(parse(&b).unwrap()["config"].get("realIp").is_none());
    }
}
#[test]
fn fx_a_compat_legacy_acl_clear_values_remain_clear() {
    for mode in ["","none","off"] {
        let mut b=server();b["ipAccess"]=json!({"mode":mode});b["locations"][0]["ipAccess"]=json!({"mode":mode});
        let cfg=parse(&b).unwrap();assert!(cfg["config"].get("ipAccess").is_none());assert!(cfg["config"]["locations"][0].get("ipAccess").is_none());
    }
}
#[test]
fn fx_a_compat_redirect_inputs_are_truthful() {
    for value in [json!("false"),json!("true"),json!("on"),Value::Null,json!(0)] {
        let mut b=server();b["certificateId"]=json!("cert");b["tls"]=json!({"redirectHttp":value});bad(parse(&b).expect_err("non-boolean JSON redirect accepted"),"redirectHttp");
    }
    for value in [true,false] {
        let mut b=server();b["certificateId"]=json!("cert");b["tls"]=json!({"redirectHttp":value});assert_eq!(parse(&b).unwrap()["config"]["redirectHttp"],value);
        let mut f=form();f.fields.insert("certificateId".into(),vec!["cert".into()]);f.fields.insert("redirectHttp".into(),vec![value.to_string()]);assert_eq!(resources::form_body(Resource::Server,&f).unwrap()["tls"]["redirectHttp"],value);
    }
    for value in ["on","","True","0","false "] {
        let mut f=form();f.fields.insert("redirectHttp".into(),vec![value.into()]);bad(resources::form_body(Resource::Server,&f).expect_err("invalid redirect form accepted"),"redirectHttp");
    }
}
#[test]
fn fx_a_compat_hsts_age_normalization_preserves_the_released_contract() {
    for value in [Value::Null,json!(""),json!(false),json!(true),json!(-1),json!(1.5),json!("1.5"),json!("123"),json!([]),json!({})] {
        let mut b=server();b["certificateId"]=json!("cert");b["tls"]=json!({"tlsOptions":{"hsts":{"enabled":true,"maxAge":value,"includeSubDomains":true,"preload":true}}});
        let out=parse(&b).expect("released HSTS contract normalizes invalid ages instead of refusing");
        assert_eq!(out["config"]["tlsOptions"]["hsts"]["maxAge"],63072000.0);
        assert_eq!(out["config"]["tlsOptions"]["hsts"]["preload"],false,"normalization must not grant preload");
    }
    for (age,expected,preload) in [(None,63072000.0,false),(Some(json!(0)),0.0,false),(Some(json!(31536000)),31536000.0,true)] {
        let mut b=server();let mut h=json!({"enabled":true,"includeSubDomains":true,"preload":true});if let Some(age)=age{h["maxAge"]=age;}
        b["certificateId"]=json!("cert");b["tls"]=json!({"tlsOptions":{"hsts":h}});let out=parse(&b).unwrap();assert_eq!(out["config"]["tlsOptions"]["hsts"]["maxAge"],expected);assert_eq!(out["config"]["tlsOptions"]["hsts"]["preload"],preload);
    }
}
#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn fx_a_compat_existing_duplicates_keep_identity_and_other_edits() {
    let db=test_db().await;let first=write(&db,Mutation::Create,None,server()).await.unwrap();let mut second=server();second["serverName"]=json!("other.test");let other=write(&db,Mutation::Create,None,second.clone()).await.unwrap();
    db.client.execute("UPDATE nginx_servers SET server_name='strict.test' WHERE id=$1",&[&other["id"].as_str().unwrap()]).await.unwrap();
    let before=resources::list(&db.client,Resource::Server).await.unwrap();second["serverName"]=json!("strict.test");second["serverDirectives"]=json!([{"name":"server_tokens","args":["off"]}]);
    write(&db,Mutation::Update,other["id"].as_str(),second.clone()).await.expect("unrelated edit of legacy duplicate refused");
    let after=resources::list(&db.client,Resource::Server).await.unwrap();assert_eq!(before.iter().map(|r|&r["id"]).collect::<Vec<_>>(),after.iter().map(|r|&r["id"]).collect::<Vec<_>>());
    assert_eq!(resources::get(&db.client,Resource::Server,first["id"].as_str().unwrap()).await.unwrap(),first);
    bad(write(&db,Mutation::Create,None,second.clone()).await.expect_err("new duplicate accepted"),"listen");
    let mut third=server();third["serverName"]=json!("third.test");third["listen"]=json!(18130);let third=write(&db,Mutation::Create,None,third).await.unwrap();
    second["serverName"]=json!("third.test");second["listen"]=json!(18130);bad(write(&db,Mutation::Update,other["id"].as_str(),second).await.expect_err("new edit conflict accepted"),"listen");
    assert_eq!(resources::get(&db.client,Resource::Server,third["id"].as_str().unwrap()).await.unwrap(),third);
}
#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn fx_a_compat_unchanged_long_values_survive_other_edits() {
    let db=test_db().await;let row=write(&db,Mutation::Create,None,server()).await.unwrap();let id=row["id"].as_str().unwrap();let mut body=server();body["serverName"]=json!("s".repeat(254));
    body["locations"][0]["path"]=json!("/".to_owned()+&"p".repeat(2999));body["locations"][0]["proxyPass"]=json!("http://127.0.0.1:19000/".to_owned()+&"t".repeat(3000));body["locations"][0]["directives"]=json!([{"name":"add_header","args":["X-Legacy","v".repeat(3000)]}]);
    let cfg=json!({"locations":[{"path":body["locations"][0]["path"],"proxyPass":body["locations"][0]["proxyPass"],"directives":body["locations"][0]["directives"]}]});
    db.client.execute("UPDATE nginx_servers SET server_name=$2,config=$3 WHERE id=$1",&[&id,&body["serverName"].as_str().unwrap(),&cfg]).await.unwrap();
    body["serverDirectives"]=json!([{"name":"server_tokens","args":["off"]}]);let updated=write(&db,Mutation::Update,Some(id),body.clone()).await.expect("unchanged stored long values refused");
    assert_eq!(updated["config"]["locations"],cfg["locations"]);
    let mut changed=body.clone();changed["serverName"]=json!("s".repeat(255));bad(write(&db,Mutation::Update,Some(id),changed).await.expect_err("changed long name accepted"),"serverName");
    let mut changed=body.clone();changed["locations"][0]["proxyPass"]=json!("http://127.0.0.1:19000/".to_owned()+&"n".repeat(3000));bad(write(&db,Mutation::Update,Some(id),changed).await.expect_err("changed long target accepted"),"locations");
    bad(write(&db,Mutation::Create,None,body.clone()).await.expect_err("new long values accepted"),"serverName");assert_eq!(resources::get(&db.client,Resource::Server,id).await.unwrap(),updated);
    // A new short alias does not turn the untouched old token into a new value.
    body["serverName"]=json!("s".repeat(254)+" alias.test");
    let with_alias=write(&db,Mutation::Update,Some(id),body.clone()).await.expect("unchanged long token rejected with new short alias");assert_eq!(with_alias["serverName"],body["serverName"]);
    // Header values are compared independently of a changed, short header name.
    body["locations"][0]["directives"][0]["args"][0]=json!("X-Updated");
    write(&db,Mutation::Update,Some(id),body.clone()).await.expect("unchanged long header value rejected on rename");
    body["locations"][0]["path"]=json!("/renamed");
    write(&db,Mutation::Update,Some(id),body).await.expect("unchanged long target/header rejected on path rename");
}

#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn fx_a_compat_unchanged_long_upstream_name_and_member_survive() {
    let db=test_db().await;let short=json!({"name":"legacy_up","servers":["127.0.0.1:19000"]});
    let row=resources::mutate(&db.state,Resource::Upstream,Mutation::Create,None,short,&actor()).await.unwrap();let id=row["id"].as_str().unwrap();
    let name="U".repeat(2100);let member="127.0.0.1:19000 weight=".to_owned()+&"0".repeat(2100)+"1";let servers=json!([member]);
    db.client.execute("UPDATE upstreams SET name=$2,servers=$3 WHERE id=$1",&[&id,&name,&servers]).await.unwrap();let body=json!({"name":name,"servers":servers});
    let saved=resources::mutate(&db.state,Resource::Upstream,Mutation::Update,Some(id),body.clone(),&actor()).await.expect("unchanged legacy upstream refused");
    let mut changed=body;changed["name"]=json!("V".repeat(2100));bad(resources::mutate(&db.state,Resource::Upstream,Mutation::Update,Some(id),changed,&actor()).await.expect_err("changed long upstream name accepted"),"name");
    assert_eq!(resources::get(&db.client,Resource::Upstream,id).await.unwrap(),saved);
    let mut linked=server();linked["serverName"]=json!("linked.test");linked["listen"]=json!(18170);
    let row=write(&db,Mutation::Create,None,linked).await.unwrap();let sid=row["id"].as_str().unwrap();
    db.client.execute("UPDATE nginx_servers SET upstream_id=$2,config='{}'::jsonb WHERE id=$1",&[&sid,&id]).await.unwrap();
    let single=json!({"serverName":"linked.test","listen":18170,"upstreamId":id});
    write(&db,Mutation::Update,Some(sid),single.clone()).await.expect("legacy single-ID binding treated derived URL as new oversized input");
    let mut another=single;another["serverName"]=json!("another-linked.test");
    write(&db,Mutation::Create,None,another).await.expect("existing legacy upstream name remains usable through its ID");
}

#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL; native nginx: NWUI_FXA_NGINX=1"]
async fn fx_a_compat_stored_duplicates_and_long_values_still_render_for_apply() {
    use nwui_api::control::model::ConfigSnapshotData;
    let db=test_db().await;let name="s".repeat(254);let path="/".to_owned()+&"p".repeat(2999);
    let cfg=json!({"locations":[{"path":path,"proxyPass":"http://127.0.0.1:19000","directives":[{"name":"add_header","args":["X-Legacy","v".repeat(3000)]}]}]});
    let rows=vec![json!({"id":"11111111-1111-4111-8111-111111111111","enabled":true,"serverName":name,"listen":18150,"config":cfg}),json!({"id":"22222222-2222-4222-8222-222222222222","enabled":true,"serverName":name,"listen":18150,"config":cfg})];
    let data=ConfigSnapshotData{servers:rows,..Default::default()};let out=nwui_api::publisher::test_support::render_for_apply(&db.state.config,&data,&[]);
    assert!(out.skipped.is_empty(),"legacy rows quarantined: {:?}",out.skipped);assert_eq!(out.rendered.conf.len(),3);
    let files=out.rendered.conf.keys().filter(|name|name.starts_with("server-")).cloned().collect::<Vec<_>>();assert!(files[0].contains("11111111"));assert!(files[1].contains("22222222"));
    for file in &files {let text=String::from_utf8_lossy(&out.rendered.conf[file]);assert!(text.contains(&path));assert!(text.contains(&"v".repeat(3000)));}
    if std::env::var("NWUI_FXA_NGINX").as_deref()==Ok("1") {
        let root=tempfile::tempdir().unwrap();for (file,bytes) in &out.rendered.conf {std::fs::write(root.path().join(file),bytes).unwrap();}
        let conf=root.path().join("nginx.conf");std::fs::write(&conf,format!("pid {0}/nginx.pid; error_log stderr; events {{}} http {{access_log off; server_names_hash_bucket_size 512; upstream nwui_api {{server 127.0.0.1:3001;}} map $http_upgrade $connection_upgrade {{default upgrade; '' close;}} include {0}/server-*.conf;}}",root.path().display())).unwrap();
        let output=std::process::Command::new("/usr/sbin/nginx").args(["-t","-p"]).arg(root.path()).arg("-c").arg(&conf).output().unwrap();
        let diagnostic=String::from_utf8_lossy(&output.stderr);assert!(output.status.success(),"{diagnostic}");assert!(diagnostic.contains("conflicting server name"),"duplicate order not exercised: {diagnostic}");println!("legacy duplicate UUID file order + 254-byte name/3000-byte path/header: actual nginx -t passed");
    }
}

#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn fx_a_compat_old_gateway_cannot_silently_drop_modern_route_fields() {
    let db=test_db().await;
    for field in ["resolveAtRuntime","retryOtherInstances"] {
        let row=write(&db,Mutation::Create,None,server()).await.unwrap();let id=row["id"].as_str().unwrap();
        let mut cfg=row["config"].clone();cfg["locations"][0][field]=json!(true);
        db.client.execute("UPDATE nginx_servers SET config=$2 WHERE id=$1",&[&id,&cfg]).await.unwrap();let before=resources::get(&db.client,Resource::Server,id).await.unwrap();
        let error=resources::mutate_for_gateway(&db.state,Resource::Server,Mutation::Update,Some(id),server(),&actor(),Some("1.19.11")).await.expect_err("old gateway silently lost a 2.0 route field");
        assert_eq!(error.status,409);assert_eq!(error.message,"이 항목은 2.0 기능을 쓰고 있어 1.19 게이트웨이에서 수정할 수 없습니다 — 게이트웨이를 2.0으로 올리세요");assert_eq!(resources::get(&db.client,Resource::Server,id).await.unwrap(),before);
        if field=="retryOtherInstances" {
            let mut preserved=server();preserved["locations"][0]["retryOtherInstances"]=json!(true);
            resources::mutate_for_gateway(&db.state,Resource::Server,Mutation::Update,Some(id),preserved,&actor(),Some("1.19.11")).await.expect("a preserving old-gateway save must remain allowed");
        }
        // The modern UI can intentionally clear it; the protection is version-specific.
        resources::mutate_for_gateway(&db.state,Resource::Server,Mutation::Update,Some(id),server(),&actor(),Some("2.0.0")).await.unwrap();
        resources::mutate(&db.state,Resource::Server,Mutation::Delete,Some(id),Value::Null,&actor()).await.unwrap();
    }
    let ordinary=resources::mutate_for_gateway(&db.state,Resource::Server,Mutation::Create,None,server(),&actor(),Some("1.19.11")).await.unwrap();
    resources::mutate_for_gateway(&db.state,Resource::Server,Mutation::Update,ordinary["id"].as_str(),server(),&actor(),Some("1.19.11")).await.unwrap();
}
#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn fx_a_compat_old_gateway_cannot_drop_upstream_or_stream_runtime_dns() {
    let db=test_db().await;
    for (kind,body) in [(Resource::Upstream,json!({"name":"dynamic","servers":["127.0.0.1:19000"]})),(Resource::Stream,json!({"protocol":"tcp","listen":18160,"proxyPass":"127.0.0.1:19000"}))] {
        let row=resources::mutate(&db.state,kind,Mutation::Create,None,body.clone(),&actor()).await.unwrap();let id=row["id"].as_str().unwrap();
        db.client.execute(&format!("UPDATE {} SET resolve_at_runtime=true WHERE id=$1",kind.table()),&[&id]).await.unwrap();let before=resources::get(&db.client,kind,id).await.unwrap();
        let error=resources::mutate_for_gateway(&db.state,kind,Mutation::Update,Some(id),body.clone(),&actor(),Some("1.19.11")).await.expect_err("old gateway cleared runtime DNS");assert_eq!(error.status,409);assert_eq!(resources::get(&db.client,kind,id).await.unwrap(),before);
        resources::mutate(&db.state,kind,Mutation::Update,Some(id),body,&actor()).await.unwrap();
    }
}

#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn fx_a_compat_gateway_header_reaches_guard_and_local_form_stays_unchanged() {
    use tower::ServiceExt;
    let db=test_db().await;insert_user(&db.client,UserRow{role:"admin",..Default::default()},None).await;
    db.client.execute("INSERT INTO gateway_tokens(id,name,token_hash,role,created_by_user_id) VALUES('compat','compat',$1,'editor','u1')",&[&nwui_api::auth::token_digest("compat-token")]).await.unwrap();
    let row=write(&db,Mutation::Create,None,server()).await.unwrap();let id=row["id"].as_str().unwrap();let mut cfg=row["config"].clone();cfg["locations"][0]["retryOtherInstances"]=json!(true);
    db.client.execute("UPDATE nginx_servers SET config=$2 WHERE id=$1",&[&id,&cfg]).await.unwrap();let before=resources::get(&db.client,Resource::Server,id).await.unwrap();
    // Isolate request forwarding from the separately owned mixed-version auth gate.
    let mut config=(*db.state.config).clone();config.version="1.19.11".into();let mut state=db.state.clone();state.config=std::sync::Arc::new(config);
    let app=nwui_api::app::with_request_context(nwui_api::control::routes::router().with_state(state),None);
    let node_request=||axum::http::Request::builder().method("PATCH").uri(format!("/api/v1/servers/{id}")).header("host","admin.example.test").header("authorization","Bearer compat-token").header("x-gateway-version","1.19.11").header("content-type","application/json").extension(axum::extract::ConnectInfo("127.0.0.1:44000".parse::<std::net::SocketAddr>().unwrap())).body(axum::body::Body::from(server().to_string())).unwrap();
    let response=app.clone().oneshot(node_request()).await.unwrap();assert_eq!(response.status(),409);let body:Value=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),1<<20).await.unwrap()).unwrap();assert_eq!(body["error"]["code"],"gateway_upgrade_required");assert_eq!(resources::get(&db.client,Resource::Server,id).await.unwrap(),before);
    // No gateway header on local UI forms: an explicit local edit can clear retry.
    let fields=[("id",id),("serverName","strict.test"),("listen","18081"),("targetMode","locations"),("locations",r#"[{"path":"/","mode":"direct","proxyPass":"http://127.0.0.1:19000"}]"#)];
    let response=app.clone().oneshot(request("reverse-proxy.updateServer",&fields)).await.unwrap();assert_eq!(response.status(),200);let body:Value=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),1<<20).await.unwrap()).unwrap();assert_eq!(body["result"]["ok"],true,"{body}");
    let after=resources::get(&db.client,Resource::Server,id).await.unwrap();assert!(after["config"]["locations"][0].get("retryOtherInstances").is_none());
    assert_eq!(app.oneshot(node_request()).await.unwrap().status(),200,"ordinary old-gateway save refused");
}

#[test]
fn fx_a_compat_tls_null_switches_remain_off_and_malformed_values_are_refused() {
    for field in ["preferServerCiphers","enabled","includeSubDomains","preload"] {
        for value in [Value::Null,json!("true"),json!(1),json!({}),json!([])] {
            let mut options=json!({"hsts":{"enabled":true,"maxAge":31536000,"includeSubDomains":true,"preload":true}});
            if field=="preferServerCiphers" {options[field]=value.clone();} else {options["hsts"][field]=value.clone();}
            let mut body=server();body["certificateId"]=json!("cert");body["tls"]=json!({"tlsOptions":options});
            if value.is_null() {
                let out=parse(&body).expect("1.19 explicit null switch means off");let opts=&out["config"]["tlsOptions"];
                let actual=if field=="preferServerCiphers"{&opts[field]}else{&opts["hsts"][field]};assert!(actual.is_null() || actual.as_bool()==Some(false),"{field} null did not turn off: {actual}");
                if field=="includeSubDomains" {assert_eq!(opts["hsts"]["preload"],false);}
            } else {bad(parse(&body).expect_err("malformed TLS switch accepted"),"tlsOptions");}
        }
    }
    let mut body=server();body["certificateId"]=json!("cert");body["tls"]=json!({"tlsOptions":null});assert!(parse(&body).is_ok(),"whole optional options can still be cleared");
}

#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn fx_a_compat_old_gateway_can_rebind_upstreams_and_switch_to_direct() {
    let db=test_db().await;let mut ids=Vec::new();
    for name in ["pool_a","pool_b"] {let row=resources::mutate(&db.state,Resource::Upstream,Mutation::Create,None,json!({"name":name,"servers":["127.0.0.1:19000"]}),&actor()).await.unwrap();ids.push(row["id"].as_str().unwrap().to_owned());}
    let mut body=server();body["locations"][0]=json!({"path":"/","mode":"upstream","upstreamId":ids[0]});let row=write(&db,Mutation::Create,None,body.clone()).await.unwrap();let id=row["id"].as_str().unwrap();
    body["locations"][0]["upstreamId"]=json!(ids[1]);
    let changed=resources::mutate_for_gateway(&db.state,Resource::Server,Mutation::Update,Some(id),body,&actor(),Some("1.19.11")).await.expect("released 1.19 ID selection A to B must remain allowed");assert_eq!(changed["config"]["locations"][0]["upstreamId"],ids[1]);
    let direct=resources::mutate_for_gateway(&db.state,Resource::Server,Mutation::Update,Some(id),server(),&actor(),Some("1.19.11")).await.expect("released 1.19 upstream to direct switch must remain allowed");assert!(direct["config"]["locations"][0].get("upstreamId").is_none());assert_eq!(direct["config"]["locations"][0]["proxyPass"],"http://127.0.0.1:19000");
}
#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn fx_a_compat_old_gateway_cannot_drop_server_level_runtime_dns() {
    let db=test_db().await;let row=write(&db,Mutation::Create,None,server()).await.unwrap();let id=row["id"].as_str().unwrap();
    let mut cfg=row["config"].clone();cfg["resolveAtRuntime"]=json!(true);db.client.execute("UPDATE nginx_servers SET config=$2 WHERE id=$1",&[&id,&cfg]).await.unwrap();let before=resources::get(&db.client,Resource::Server,id).await.unwrap();
    let error=resources::mutate_for_gateway(&db.state,Resource::Server,Mutation::Update,Some(id),server(),&actor(),Some("1.19.11")).await.expect_err("legacy server-level runtime flag silently dropped");assert_eq!(error.status,409);assert_eq!(resources::get(&db.client,Resource::Server,id).await.unwrap(),before);
    write(&db,Mutation::Update,Some(id),server()).await.expect("no-header explicit edit must remain unchanged");
}
