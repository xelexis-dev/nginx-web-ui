
    use crate::control::resources::*;
    use crate::{state::AppState,contracts::Actor};
    use crate::contracts::FormInput;
    use crate::control::{reserved_ports::ReservedPorts,validation as valid};
    use serde_json::{Value,json};
    use std::collections::BTreeMap;
    fn form(value:Option<Value>,mode:&str)->FormInput{
        let mut form=FormInput::default();for(key,value)in[("serverName","retry.test"),("listen","8081"),("targetMode","locations")]{form.fields.insert(key.into(),vec![value.into()]);}
        let mut location=json!({"path":"/api","mode":mode,"proxyPass":"http://127.0.0.1:9000","upstreamId":"up","directives":null});if let Some(value)=value{location["retryOtherInstances"]=value;}form.fields.insert("locations".into(),vec![json!([location]).to_string()]);form
    }
    #[test]
    fn sr_sa_optional_strict_bool_passes_forms_and_node_and_stores_true_only(){
        for mode in ["direct","upstream"]{for flag in [None,Some(json!(false)),Some(json!(true))]{let raw=form_body(Resource::Server,&form(flag.clone(),mode)).unwrap();assert_eq!(raw["locations"][0].get("retryOtherInstances"),flag.as_ref());for input in [raw.clone(),serde_json::from_slice(&serde_json::to_vec(&raw).unwrap()).unwrap()]{let row=parse_server(&input,&ReservedPorts::default()).unwrap();let stored=row["config"]["locations"][0].get("retryOtherInstances");if flag==Some(json!(true)){assert_eq!(stored,Some(&json!(true)));}else{assert!(stored.is_none());}assert!(row["config"]["locations"][0].get("directives").is_none());}}}
        for invalid in [Value::Null,json!("true"),json!(1),json!([]),json!({})]{let raw=form_body(Resource::Server,&form(Some(invalid),"direct")).unwrap();for input in [raw.clone(),serde_json::from_slice(&serde_json::to_vec(&raw).unwrap()).unwrap()]{let error=parse_server(&input,&ReservedPorts::default()).unwrap_err();assert_eq!(error.status,400);assert_eq!(error.field_errors.unwrap()["locations"],"location 1 (/api): 재시도 설정은 참 또는 거짓이어야 합니다.");}}
        let single=parse_server(&json!({"serverName":"single.test","listen":8081,"proxyPass":"http://127.0.0.1:9000","retryOtherInstances":true}),&ReservedPorts::default()).unwrap();assert!(single["config"]["locations"][0].get("retryOtherInstances").is_none());
    }
    #[test]
    fn sr_sa_editor_connect_fields_and_advanced_lines_win_without_allowlisting_retry(){
        for editor in [json!({"timeouts":{"connect":"4s"}}),json!({"advanced":"proxy_connect_timeout 4s;"})]{let mut f=form(Some(json!(true)),"direct");let mut locations:Value=serde_json::from_str(f.get("locations").unwrap()).unwrap();locations[0]["directives"]=editor;f.fields.insert("locations".into(),vec![locations.to_string()]);let row=parse_server(&form_body(Resource::Server,&f).unwrap(),&ReservedPorts::default()).unwrap();let mut cfg=row["config"].clone();cfg["serverId"]=json!("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");cfg["serverName"]=row["serverName"].clone();cfg["listen"]=row["listen"].clone();let text=crate::control::render::render_reverse_proxy(&cfg).unwrap();assert_eq!(text.matches("proxy_connect_timeout 4s;").count(),1);assert!(!text.contains("proxy_connect_timeout 2s;"));}
        for name in ["proxy_next_upstream","proxy_next_upstream_tries","proxy_next_upstream_timeout"]{assert!(!valid::LOCATION_DIRECTIVES.contains(&name));assert!(valid::assert_valid_directive(name,&["2".into()],valid::DirectiveContext::Location).is_err());}
    }
    async fn database()->(AppState,tokio_postgres::Client,tempfile::TempDir){
        async fn connect(url:&str)->tokio_postgres::Client{let(c,f)=tokio_postgres::connect(url,tokio_postgres::NoTls).await.unwrap();tokio::spawn(f);c}
        let admin=std::env::var("NWUI_TEST_DATABASE_URL").unwrap();let name=format!("sr_sa_{}",uuid::Uuid::new_v4().simple());connect(&admin).await.batch_execute(&format!("CREATE DATABASE {name}")).await.unwrap();let url=format!("{}/{name}",admin.rsplit_once('/').unwrap().0);let temp=tempfile::tempdir().unwrap();let key=temp.path().join("key");
        let mut config=crate::config::RuntimeConfig::read(|k|match k{"DATABASE_URL"=>Some(url.clone()),"AUTH_SECRET"=>Some("sr-sa-test-only".into()),"DATA_KEY_FILE"=>Some(key.display().to_string()),"API_DB_POOL_SIZE"=>Some("1".into()),_=>None}).unwrap();config.target_dns.descriptor_path=temp.path().join("dns.json");std::fs::write(&config.target_dns.descriptor_path,serde_json::to_vec(&crate::dns_guard::Descriptor::available(&config.target_dns)).unwrap()).unwrap();std::fs::write(&config.data_key_file,"A5".repeat(32)).unwrap();let state=AppState::new(config).unwrap();crate::migrations::run(&state.db,&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle")).await.unwrap();let client=connect(&url).await;
        client.batch_execute("INSERT INTO users(id,email,role,session_version,enabled,approved) VALUES('sr-user','sr@example.test','admin',1,true,true)").await.unwrap();client.execute("INSERT INTO gateway_tokens(id,name,token_hash,role,created_by_user_id) VALUES('sr-gateway','sr',$1,'editor','sr-user')",&[&crate::auth::token_digest("sr-gateway-test")]).await.unwrap();(state,client,temp)
    }
    async fn node(state:&AppState,method:&str,path:&str,value:&Value)->(u16,Value){
        use axum::{body::Body,extract::ConnectInfo,http::Request};use tower::ServiceExt;
        let request=Request::builder().method(method).uri(path).header("host","admin.example.test").header("authorization","Bearer sr-gateway-test").header("x-gateway-version",&state.config.version).header("content-type","application/json").extension(ConnectInfo("127.0.0.1:45000".parse::<std::net::SocketAddr>().unwrap())).body(if method=="GET"{Body::empty()}else{Body::from(serde_json::to_vec(value).unwrap())}).unwrap();let response=tokio::time::timeout(std::time::Duration::from_secs(5),crate::server::router(state.clone()).oneshot(request)).await.expect("single-connection read/edit must finish").unwrap();let status=response.status().as_u16();let body=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),65536).await.unwrap()).unwrap();(status,body)
    }
    async fn snapshot(state:&AppState)->crate::control::model::ConfigSnapshotData{let pool=state.db.get().await.unwrap();crate::control::model::load_snapshot(&pool).await.unwrap()}
    #[tokio::test]
    #[ignore="real DB: NWUI_TEST_DATABASE_URL"]
    async fn sr_sa_on_off_on_node_edit_pending_snapshot_and_rollback_preserve_true_only(){
        let(state,client,_temp)=database().await;let actor=Actor{id:Some("sr-user".into()),label:None,ip:None};let up=mutate(&state,Resource::Upstream,Mutation::Create,None,json!({"name":"backend","servers":["127.0.0.1:9000"]}),&actor).await.unwrap();let uid=up["id"].as_str().unwrap();
        let session=crate::auth::principal::SessionPrincipal{user_id:"sr-user".into(),email:None,name:None,role:crate::contracts::Role::Admin,session_version:1,totp_enabled:false,requires_2fa:false,mfa_verified:false,expires_at:i64::MAX};
        for kind in 0..3{
            let body=|flag:Option<Value>|{let mut body=form_body(Resource::Server,&form(flag,if kind==2{"upstream"}else{"direct"})).unwrap();body["locations"][0]["upstreamId"]=json!(uid);if kind==1{body["locations"][0]["proxyPass"]=json!("http://app.test:9000/path/");body["locations"][0]["resolveAtRuntime"]=json!(true);}body};
            let row=mutate(&state,Resource::Server,Mutation::Create,None,body(Some(json!(true))),&actor).await.unwrap();let id=row["id"].as_str().unwrap().to_owned();let path=format!("/api/v1/servers/{id}");let mut off=None;
            for flag in [Some(json!(true)),Some(json!(false)),None,Some(json!(true))]{let desired=flag==Some(json!(true));let(status,response)=node(&state,"PATCH",&path,&body(flag)).await;assert_eq!(status,200,"{response}");let saved=get(&client,Resource::Server,&id).await.unwrap();let location=&saved["config"]["locations"][0];assert_eq!(location.get("retryOtherInstances"),desired.then_some(&json!(true)));assert!(location.get("directives").is_none(),"generated policy never becomes stored user directives");
                let view=crate::control::views::view(&state,&session,&format!("/reverse-proxy/{id}/edit"),&BTreeMap::new()).await.unwrap().unwrap();assert_eq!(view["server"]["config"]["locations"][0].get("retryOtherInstances"),location.get("retryOtherInstances"));let(status,read)=node(&state,"GET",&path,&Value::Null).await;assert_eq!(status,200);assert_eq!(read["server"]["config"]["locations"][0].get("retryOtherInstances"),location.get("retryOtherInstances"));if !desired{off=Some(snapshot(&state).await);}
            }
            let on=snapshot(&state).await;let pending=crate::control::pending::diff_config(off.as_ref(),&on);assert!(pending.has_changes);assert!(serde_json::to_string(&pending).unwrap().contains("retryOtherInstances"));
            for invalid in [Value::Null,json!("true"),json!(1)]{let raw=body(Some(invalid));let error=mutate(&state,Resource::Server,Mutation::Update,Some(&id),raw.clone(),&actor).await.unwrap_err();assert_eq!(error.status,400);assert!(error.field_errors.unwrap()["locations"].contains("재시도 설정"));let(status,error)=node(&state,"PATCH",&path,&raw).await;assert_eq!(status,400,"{error}");assert!(error["error"]["fieldErrors"]["locations"].as_str().unwrap().contains("재시도 설정"));assert_eq!(get(&client,Resource::Server,&id).await.unwrap()["config"],on.servers[0]["config"]);}
            mutate(&state,Resource::Server,Mutation::Update,Some(&id),body(Some(json!(false))),&actor).await.unwrap();let mut pool=state.db.get().await.unwrap();let tx=pool.transaction().await.unwrap();crate::publisher::rollback::replace_rows(&tx,&on).await.unwrap();tx.commit().await.unwrap();drop(pool);let restored=snapshot(&state).await;assert_eq!(restored.servers[0]["config"],on.servers[0]["config"]);let rendered=crate::control::model::render_preview_with_context(&state.config,&restored,&[],&crate::dns_guard::status(&state.config)).unwrap();assert!(rendered.skipped.is_empty());assert!(String::from_utf8_lossy(&rendered.rendered.conf[&format!("server-{id}.conf")]).contains("proxy_next_upstream_tries 2;"));
            let(status,read)=node(&state,"GET",&path,&Value::Null).await;assert_eq!(status,200);assert_eq!(read["server"]["config"]["locations"][0]["retryOtherInstances"],true);mutate(&state,Resource::Server,Mutation::Delete,Some(&id),Value::Null,&actor).await.unwrap();
        }
    }
    #[test]
    fn sr_sa_contract_boolean_is_optional_and_non_nullable_on_locations_only(){
        let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/contracts/openapi.json");let document:Value=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();let schemas=&document["components"]["schemas"];
        for name in ["NodeLocation","NodeLocationInput"]{let schema=&schemas[name];let field=&schema["properties"]["retryOtherInstances"];assert_eq!(field["type"],"boolean");let validate=jsonschema::validator_for(field).unwrap();for value in [json!(true),json!(false)]{assert!(validate.is_valid(&value));}for value in [Value::Null,json!(1),json!("true")]{assert!(!validate.is_valid(&value));}assert!(!schema["required"].as_array().unwrap().contains(&json!("retryOtherInstances")));}
        for name in ["NodeServerInput","NodeStreamInput"]{assert!(schemas[name]["properties"].get("retryOtherInstances").is_none());}
    }


    #[tokio::test]
    #[ignore="real DB: NWUI_TEST_DATABASE_URL"]
    async fn pf_a1_local_and_node_read_projection_keeps_storage_snapshots_and_pending_canonical(){
        use axum::{body::Body,extract::ConnectInfo,http::Request};use tower::ServiceExt;
        use crate::{config::RuntimeConfig,state::AppState,control::{model as apply,pending},publisher::rollback};
        async fn connect(url:&str)->tokio_postgres::Client{let(client,connection)=tokio_postgres::connect(url,tokio_postgres::NoTls).await.unwrap();tokio::spawn(connection);client}
        let admin=std::env::var("NWUI_TEST_DATABASE_URL").unwrap();let db=format!("pf_a1_{}",uuid::Uuid::new_v4().simple());connect(&admin).await.batch_execute(&format!("CREATE DATABASE {db}")).await.unwrap();let url=format!("{}/{db}",admin.rsplit_once('/').unwrap().0);
        let root=tempfile::tempdir().unwrap();let key=root.path().join("key");let secret="pf-a1-test-only";
        let mut config=RuntimeConfig::read(|k|match k{"DATABASE_URL"=>Some(url.clone()),"AUTH_SECRET"=>Some(secret.into()),"DATA_KEY_FILE"=>Some(key.display().to_string()),"API_DB_POOL_SIZE"=>Some("1".into()),_=>None}).unwrap();config.target_dns.descriptor_path=root.path().join("dns.json");std::fs::write(&config.data_key_file,"A5".repeat(32)).unwrap();let state=AppState::new(config).unwrap();
        crate::migrations::run(&state.db,&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle")).await.unwrap();let client=connect(&url).await;
        client.batch_execute("INSERT INTO users(id,email,role,session_version,enabled,approved) VALUES('pf-user','pf@example.test','editor',1,true,true),('pf-issuer','pf-issuer@example.test','admin',1,true,true); INSERT INTO upstreams(id,name,servers) VALUES('up','backend','[\"127.0.0.1:9000\"]');").await.unwrap();
        let config=json!({"locations":[{"path":"/","upstreamId":"up"}]});client.execute("INSERT INTO nginx_servers(id,server_name,listen,config,enabled) VALUES('server','example.test',8081,$1,true)",&[&config]).await.unwrap();
        client.execute("INSERT INTO gateway_tokens(id,name,token_hash,role,created_by_user_id) VALUES('gw','pf',$1,'viewer','pf-issuer')",&[&crate::auth::token_digest("pf-gateway")]).await.unwrap();
        let cookie=crate::auth::session::encode(json!({"id":"pf-user","sv":1}).as_object().unwrap(),secret,crate::auth::session::COOKIE_NAME_PLAIN,crate::auth::principal::now_unix_ms()/1000,3600).unwrap();
        for name in [Some("backend"),Some("renamed"),None]{
            if name==Some("renamed"){client.batch_execute("UPDATE upstreams SET name='renamed' WHERE id='up'").await.unwrap();}if name.is_none(){client.batch_execute("DELETE FROM upstreams WHERE id='up'").await.unwrap();}
            for(path,browser,list)in[("/api/ui/v1/views?path=%2Freverse-proxy",true,true),("/api/ui/v1/views?path=%2Freverse-proxy%2Fserver",true,false),("/api/ui/v1/views?path=%2Freverse-proxy%2Fserver%2Fedit",true,false),("/api/v1/servers",false,true),("/api/v1/servers/server",false,false)]{
                let mut request=Request::builder().uri(path).header("host","admin.example.test").extension(ConnectInfo("127.0.0.1:45000".parse::<std::net::SocketAddr>().unwrap()));
                if browser{request=request.header("cookie",format!("{}={cookie}",crate::auth::session::COOKIE_NAME_PLAIN));}else{request=request.header("authorization","Bearer pf-gateway").header("x-gateway-version",&state.config.version);}
                let response=tokio::time::timeout(std::time::Duration::from_secs(5),crate::server::router(state.clone()).oneshot(request.body(Body::empty()).unwrap())).await.expect("pool size one must not deadlock").unwrap();assert_eq!(response.status(),200,"{path}");let body:Value=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),65536).await.unwrap()).unwrap();let row=if list{&body["servers"][0]}else{&body["server"]};let location=&row["config"]["locations"][0];assert_eq!(location["upstreamId"],"up");assert_eq!(location.get("upstreamName").and_then(Value::as_str),name,"{path}: {body}");assert!(location.get("proxyPass").is_none());
            }
            let saved:Value=client.query_one("SELECT config FROM nginx_servers WHERE id='server'",&[]).await.unwrap().get(0);assert_eq!(saved,config);
            let mut pool=state.db.get().await.unwrap();let snapshot=apply::load_snapshot(&pool).await.unwrap();assert_eq!(snapshot.servers[0]["config"],config);assert!(!pending::diff_config(Some(&snapshot),&snapshot).has_changes);
            let mut bad=snapshot.clone();bad.servers[0]["config"]["locations"][0]["upstreamId"]=json!("missing");let tx=pool.transaction().await.unwrap();assert!(rollback::replace_rows(&tx,&bad).await.is_err());let count:i64=tx.query_one("SELECT count(*) FROM nginx_servers",&[]).await.unwrap().get(0);assert_eq!(count,1,"restore must refuse before any DELETE");tx.rollback().await.unwrap();
        }
    }
