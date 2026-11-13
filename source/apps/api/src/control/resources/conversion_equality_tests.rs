use super::*;
#[test]
fn pf_a2_conversion_equality_form_preserves_arrays_and_explicit_absence(){
    let dirs=json!([{"name":"proxy_read_timeout","args":["60s"]},{"name":"proxy_set_header","args":["Host","$host"]}]);
    let mut form=FormInput::default();for(k,v)in[("serverName","example.test"),("listen","8081"),("targetMode","locations")]{form.fields.insert(k.into(),vec![v.into()]);}
    form.fields.insert("locations".into(),vec![json!([{"path":"/","mode":"direct","proxyPass":"http://app.test/","directives":dirs}]).to_string()]);
    let raw=form_body(Resource::Server,&form).unwrap();assert_eq!(raw["locations"][0]["directives"],dirs);
    let absent=parse_server(&json!({"serverName":"example.test","listen":8081,"proxyPass":"http://app.test/","locationDirectives":null}),&ReservedPorts::default()).unwrap();assert!(absent["upstreamId"].is_null());assert!(absent["config"]["locations"][0].get("directives").is_none());assert_eq!(absent["config"]["locations"][0]["proxyPass"],"http://app.test/");
}

fn render(cfg:&Value)->String{
    super::super::render::render_reverse_proxy_with_dns_and_resolve(cfg,Some("127.0.0.1:5354".parse().unwrap()),&|_,_|vec![]).unwrap()
}
fn carried_form(cfg:&Value,upstream:bool)->FormInput{
    let mut form=FormInput::default();let mut location=json!({"path":"/","mode":if upstream{"upstream"}else{"direct"},"directives":cfg.get("locationDirectives").cloned().unwrap_or(Value::Null)});
    if upstream{location["upstreamId"]=json!("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");}else{location["proxyPass"]=cfg["proxyPass"].clone();if cfg["resolveAtRuntime"]==true{location["resolveAtRuntime"]=json!(true);}}
    for(k,v)in[("serverName",cfg["serverName"].as_str().unwrap().to_owned()),("listen",cfg["listen"].to_string()),("targetMode","locations".into()),("locations",json!([location]).to_string())]{form.fields.insert(k.into(),vec![v]);}
    for key in ["serverDirectives","cache","gzip","realIp"]{if let Some(value)=cfg.get(key){form.fields.insert(key.into(),vec![value.to_string()]);}}
    if let Some(tls)=cfg.get("tls"){for(key,value)in[("certificateId","cert".into()),("tlsPort",tls["tlsPort"].to_string()),("redirectHttp",tls["redirectHttp"].to_string()),("tlsOptions",tls["tuning"].to_string())]{form.fields.insert(key.into(),vec![value]);}}
    form
}

#[tokio::test]
async fn pf_a2_conversion_equality_rust_render_matrix_and_local_node_form_round_trip(){
    let sets=[None,Some(json!([])),Some(json!([{"name":"proxy_set_header","args":["Host","$host"]}])),Some(json!([{"name":"add_header","args":["X-First","one"]},{"name":"proxy_read_timeout","args":["60s"]},{"name":"proxy_set_header","args":["Host","$host"]}])),Some(json!([{"name":"return","args":["302","/next"]},{"name":"rewrite","args":["^/old$","/new","break"]}])),Some(json!([{"name":"proxy_set_header","args":["X-Empty",""]}])),Some(json!([{"name":"proxy_set_header","args":["X-Quoted","two words"]}]))];
    let names=std::collections::BTreeMap::from([("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(),"backend".into())]);let mut count=0;
    for target in 0..4{for dirs in &sets{for uri in ["http://app.test/","http://app.test:80","http://app.test/path/"]{for cache in [false,true]{for gzip in [false,true]{for real in [Value::Null,json!({"preset":"cloudflare"}),json!({"preset":"custom","trustedRanges":["10.0.0.0/8"],"header":"X-Forwarded-For","recursive":true})]{for tls in [false,true]{
        let upstream=target<2;let mut old=json!({"serverId":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","serverName":"example.test","listen":8081,"cache":{"enabled":cache},"gzip":{"enabled":gzip},"realIp":real,"serverDirectives":[{"name":"add_header","args":["X-Example","one"]},{"name":"proxy_read_timeout","args":["300s"]},{"name":"server_tokens","args":["off"]}]});
        if upstream{old["upstreamName"]=json!("backend");}else{old["proxyPass"]=json!(uri);if target==3{old["resolveAtRuntime"]=json!(true);}}
        if let Some(dirs)=dirs{old["locationDirectives"]=dirs.clone();}
        if tls{old["tls"]=json!({"tlsPort":8443,"httpPort":8081,"redirectHttp":false,"certPath":"/cert/fullchain.pem","keyPath":"/cert/privkey.pem","tuning":{"hsts":{"enabled":true,"maxAge":3600,"includeSubDomains":true}}});}
        let form=carried_form(&old,upstream);let raw=form_body(Resource::Server,&form).unwrap();
        for node in [false,true]{
            // The relay sends this exact JSON body to the node; its save parser runs again.
            let submitted=if node{serde_json::from_slice(&serde_json::to_vec(&raw).unwrap()).unwrap()}else{raw.clone()};
            let parsed=parse_server_for_save(submitted,ReservedPorts::default(),|cfg|valid::validate_server_config_with(cfg,&|_,_|vec![]).map(|_|()),None).await.unwrap();
            assert!(parsed["upstreamId"].is_null());assert!(parsed["config"].get("proxyPass").is_none());assert!(parsed["config"].get("locationDirectives").is_none());
            assert_eq!(parsed["config"]["locations"][0].get("directives"),dirs.as_ref());
            let mut converted=super::super::location_refs::resolve_locations(&parsed["config"],&names).unwrap();converted["serverId"]=json!("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");converted["serverName"]=json!("example.test");converted["listen"]=json!(8081);if tls{assert_eq!(parsed["certificateId"],"cert");let mut tls=old["tls"].clone();tls["tlsPort"]=converted["tlsPort"].clone();tls["httpPort"]=parsed["listen"].clone();tls["redirectHttp"]=converted["redirectHttp"].clone();tls["tuning"]=converted["tlsOptions"].clone();converted["tls"]=tls;}
            let group=if upstream{super::super::render::render_upstream_with_dns(&json!({"id":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","name":"backend","servers":["app.test:9000"],"resolveAtRuntime":target==1}),Some("127.0.0.1:5354".parse().unwrap())).unwrap()}else{String::new()};
            assert_eq!(format!("{group}{}",render(&old)),format!("{group}{}",render(&converted)),"target={target} uri={uri} cache={cache} gzip={gzip} tls={tls} node={node} dirs={dirs:?}");count+=1;
        }
    }}}}}}}
    assert_eq!(count,4032);println!("conversion equality matrix: {count} local/node byte comparisons");
}

#[test]
fn pf_a2_conversion_equality_runtime_absence_negative_controls(){
    for (cache,tls) in [(false,false),(true,false),(false,true)]{
        let mut cfg=json!({"serverId":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","serverName":"example.test","listen":8081,"proxyPass":"http://app.test/","resolveAtRuntime":true,"cache":{"enabled":cache},"serverDirectives":[{"name":"add_header","args":["X-Example","one"]}]});
        if tls{cfg["tls"]=json!({"tlsPort":8443,"httpPort":8081,"certPath":"/cert/fullchain.pem","keyPath":"/cert/privkey.pem","tuning":{"hsts":{"enabled":true}}});}
        let absent=render(&cfg);let location=absent.rfind("location /").unwrap();let lines=&absent[location..];assert!(lines.find("add_header X-Example").unwrap()<lines.find("proxy_set_header Host $host;").unwrap());
        let mut explicit=cfg.clone();explicit["locationDirectives"]=json!([{"name":"proxy_set_header","args":["Host","$host"]}]);assert_ne!(absent,render(&explicit));explicit["locationDirectives"]=json!([]);let empty=render(&explicit);assert_ne!(absent,empty);assert!(!empty[empty.rfind("location /").unwrap()..].contains("proxy_set_header Host $host;"));
    }
}

#[test]
fn pf_a2_conversion_equality_null_omission_arrays_and_allowlist_for_local_and_node(){
    for dirs in [None,Some(Value::Null),Some(json!([])),Some(json!([{"name":"proxy_read_timeout","args":["60s"]}]))]{
        for single in [false,true]{
            let mut raw=json!({"serverName":"example.test","listen":8081});if single{raw["proxyPass"]=json!("http://app.test/");if let Some(dirs)=&dirs{raw["locationDirectives"]=dirs.clone();}}else{raw["targetMode"]=json!("locations");let mut l=json!({"path":"/","mode":"direct","proxyPass":"http://app.test/"});if let Some(dirs)=&dirs{l["directives"]=dirs.clone();}raw["locations"]=json!([l]);}
            let mut form=FormInput::default();for(k,v)in[("serverName","example.test"),("listen","8081")]{form.fields.insert(k.into(),vec![v.into()]);}
            if single{form.fields.insert("mode".into(),vec!["direct".into()]);form.fields.insert("proxyPass".into(),vec!["http://app.test/".into()]);if let Some(dirs)=&dirs{form.fields.insert("locationDirectives".into(),vec![dirs.to_string()]);}}else{form.fields.insert("targetMode".into(),vec!["locations".into()]);form.fields.insert("locations".into(),vec![raw["locations"].to_string()]);}
            let local=form_body(Resource::Server,&form).unwrap();let forwarded:Value=serde_json::from_slice(&serde_json::to_vec(&local).unwrap()).unwrap();
            let expected=parse_server(&raw,&ReservedPorts::default()).unwrap();assert_eq!(parse_server(&local,&ReservedPorts::default()).unwrap()["config"]["locations"],expected["config"]["locations"]);assert_eq!(parse_server(&forwarded,&ReservedPorts::default()).unwrap()["config"]["locations"],expected["config"]["locations"]);
            let parsed=parse_server(&raw,&ReservedPorts::default()).unwrap();let actual=parsed["config"]["locations"][0].get("directives");if dirs==Some(Value::Null){assert!(actual.is_none());}else{assert_eq!(actual,Some(&dirs.clone().unwrap_or_else(||json!([]))));}
        }
    }
    for invalid in [json!(true),json!(1),json!("not an editor")]{assert!(form_directives(Some(&invalid),valid::DirectiveContext::Location).is_err());}
    let mut form=carried_form(&json!({"serverName":"example.test","listen":8081,"proxyPass":"http://app.test/","locationDirectives":[{"name":"include","args":["/tmp/evil"]}]}),false);assert!(form_body(Resource::Server,&form).is_err());
    form.fields.insert("locations".into(),vec![json!([{"path":"/","mode":"direct","proxyPass":"http://app.test/","directives":{"headers":[{"name":"Host","value":"$host"}]}}]).to_string()]);assert_eq!(form_body(Resource::Server,&form).unwrap()["locations"][0]["directives"],json!([{"name":"proxy_set_header","args":["Host","$host"]}]));
}

async fn database()->(AppState,tokio_postgres::Client,tempfile::TempDir){
    async fn connect(url:&str)->tokio_postgres::Client{let(c,f)=tokio_postgres::connect(url,tokio_postgres::NoTls).await.unwrap();tokio::spawn(f);c}
    let admin=std::env::var("NWUI_TEST_DATABASE_URL").unwrap();let name=format!("pf_a2_{}",uuid::Uuid::new_v4().simple());connect(&admin).await.batch_execute(&format!("CREATE DATABASE {name}")).await.unwrap();let url=format!("{}/{name}",admin.rsplit_once('/').unwrap().0);let temp=tempfile::tempdir().unwrap();let key=temp.path().join("key");
    let mut config=crate::config::RuntimeConfig::read(|k|match k{"DATABASE_URL"=>Some(url.clone()),"AUTH_SECRET"=>Some("pf-a2-test-only".into()),"DATA_KEY_FILE"=>Some(key.display().to_string()),"API_DB_POOL_SIZE"=>Some("1".into()),_=>None}).unwrap();config.target_dns.descriptor_path=temp.path().join("dns.json");std::fs::write(&config.data_key_file,"A5".repeat(32)).unwrap();let state=AppState::new(config).unwrap();crate::migrations::run(&state.db,&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle")).await.unwrap();(state,connect(&url).await,temp)
}
fn actor()->Actor{Actor{id:None,label:Some("pf-a2".into()),ip:None}}
#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn pf_a2_conversion_equality_local_and_node_save_resolution_preserves_bytes(){
    let (_state,client,_temp)=database().await;client.batch_execute("INSERT INTO upstreams(id,name,servers) VALUES('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','backend','[\"127.0.0.1:9000\"]')").await.unwrap();
    for upstream in [false,true]{for dirs in [None,Some(json!([])),Some(json!([{"name":"proxy_read_timeout","args":["60s"]},{"name":"proxy_set_header","args":["Host","$host"]}]))]{
        let mut old=json!({"serverId":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","serverName":"example.test","listen":8081,"serverDirectives":[{"name":"add_header","args":["X-Example","one"]}]});if upstream{old["upstreamName"]=json!("backend");}else{old["proxyPass"]=json!("http://app.test:80/path/");old["resolveAtRuntime"]=json!(true);}if let Some(dirs)=&dirs{old["locationDirectives"]=dirs.clone();}
        let raw=form_body(Resource::Server,&carried_form(&old,upstream)).unwrap();for node in [false,true]{let submitted=if node{serde_json::from_slice(&serde_json::to_vec(&raw).unwrap()).unwrap()}else{raw.clone()};let mut parsed=parse_server_for_save(submitted,ReservedPorts::default(),valid::assert_valid_server_config,None).await.unwrap();let upstreams=resolve_server(&client,&mut parsed).await.unwrap();assert!(parsed["config"]["locations"][0].get("upstreamName").is_none());assert_eq!(parsed["config"]["locations"][0].get("directives"),dirs.as_ref());if upstream{assert!(parsed["config"]["locations"][0].get("proxyPass").is_none());}
            let mut cfg=super::super::location_refs::resolve_locations(&parsed["config"],&super::super::location_refs::upstream_names(&upstreams)).unwrap();for key in ["serverId","serverName","listen"]{cfg[key]=old[key].clone();}assert_eq!(render(&old),render(&cfg));
        }
    }}
}

#[tokio::test]
#[ignore="real DB: NWUI_TEST_DATABASE_URL"]
async fn pf_a2_reference_integrity_scope_rename_delete_literal_policy_and_audit(){
    let(state,client,_temp)=database().await;let(foreign,_foreign_client,_foreign_temp)=database().await;
    let create_upstream=|name:&str|json!({"name":name,"servers":["127.0.0.1:9000"]});
    let up=mutate(&state,Resource::Upstream,Mutation::Create,None,create_upstream("backend"),&actor()).await.unwrap();let id=up["id"].as_str().unwrap();
    let foreign_up=mutate(&foreign,Resource::Upstream,Mutation::Create,None,create_upstream("foreign"),&actor()).await.unwrap();
    for missing in ["missing",foreign_up["id"].as_str().unwrap()]{let error=mutate(&state,Resource::Server,Mutation::Create,None,json!({"serverName":"bad.test","listen":8081,"targetMode":"locations","locations":[{"path":"/missing","upstreamId":missing}]}),&actor()).await.unwrap_err();assert_eq!(error.status,400);assert!(error.field_errors.unwrap()["locations"].contains("선택한 upstream을 찾을 수 없습니다"));}
    let row=mutate(&state,Resource::Server,Mutation::Create,None,json!({"serverName":"example.test","listen":8081,"upstreamId":id,"locationDirectives":null}),&actor()).await.unwrap();let sid=row["id"].as_str().unwrap();assert!(row["upstreamId"].is_null());assert_eq!(row["config"]["locations"][0]["upstreamId"],id);assert!(row["config"]["locations"][0].get("directives").is_none());
    let after:Value=client.query_one("SELECT after FROM audit_logs WHERE target=$1 AND action='reverseproxy:create'",&[&sid]).await.unwrap().get(0);assert_eq!(after["config"],row["config"]);assert!(after["config"]["locations"][0].get("upstreamName").is_none());assert!(after["config"]["locations"][0].get("proxyPass").is_none());
    mutate(&state,Resource::Upstream,Mutation::Update,Some(id),create_upstream("renamed"),&actor()).await.unwrap();let preview=super::super::views::render_preview(&state.config,Resource::Server,&row,&list(&client,Resource::Upstream).await.unwrap(),&[]);assert!(preview["config"].as_str().unwrap().contains("proxy_pass http://renamed;"));assert_eq!(mutate(&state,Resource::Upstream,Mutation::Delete,Some(id),Value::Null,&actor()).await.unwrap_err().status,409);
    mutate(&state,Resource::Server,Mutation::Delete,Some(sid),Value::Null,&actor()).await.unwrap();
    client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('legacy','legacy.test',8081,$1)",&[&json!({"locations":[{"path":"/","proxyPass":"http://renamed"}]})]).await.unwrap();
    for operation in [Mutation::Update,Mutation::Delete]{assert_eq!(mutate(&state,Resource::Upstream,operation,Some(id),create_upstream("next"),&actor()).await.unwrap_err().status,409);}
    // Selected and directly entered exact-name bytes deliberately share this next-save policy.
    for mode in ["upstream","direct"]{let updated=mutate(&state,Resource::Server,Mutation::Update,Some("legacy"),json!({"serverName":"legacy.test","listen":8081,"targetMode":"locations","locations":[{"path":"/","mode":mode,"upstreamId":id,"proxyPass":"http://renamed","upstreamName":"stale"}]}),&actor()).await.unwrap();let l=&updated["config"]["locations"][0];assert_eq!(l["upstreamId"],id);assert!(l.get("proxyPass").is_none());assert!(l.get("upstreamName").is_none());}
    for target in ["http://renamed/","https://renamed","http://RENAMED"]{let mut parsed=parse_server(&json!({"serverName":"near.test","listen":8081,"targetMode":"locations","locations":[{"path":"/","mode":"direct","proxyPass":target}]}),&ReservedPorts::default()).unwrap();resolve_server(&client,&mut parsed).await.unwrap();assert_eq!(parsed["config"]["locations"][0]["proxyPass"],target);assert!(parsed["config"]["locations"][0].get("upstreamId").is_none());}
    mutate(&state,Resource::Server,Mutation::Delete,Some("legacy"),Value::Null,&actor()).await.unwrap();client.execute("INSERT INTO nginx_servers(id,server_name,listen,upstream_id,config) VALUES('fk','fk.test',8081,$1,'{}')",&[&id]).await.unwrap();assert_eq!(mutate(&state,Resource::Upstream,Mutation::Delete,Some(id),Value::Null,&actor()).await.unwrap_err().status,409);
}

#[test]
fn pf_a2_contract_exposes_nullable_directives_readonly_name_and_deprecated_single_input(){
    let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/contracts/openapi.json");let document:Value=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();let schemas=&document["components"]["schemas"];
    for field in ["proxyPass","upstreamId","locationDirectives","resolveAtRuntime"]{assert_eq!(schemas["NodeServerInput"]["properties"][field]["deprecated"],true);}
    assert_eq!(schemas["NodeServerInput"]["properties"]["targetMode"]["oneOf"][0]["deprecated"],true);assert_eq!(schemas["NodeLocation"]["properties"]["upstreamName"]["readOnly"],true);assert_eq!(schemas["NodeLocation"]["oneOf"].as_array().unwrap().len(),2);
    for(schema,field)in[("NodeLocationInput","directives"),("NodeServerInput","locationDirectives"),("NodeServerInput","serverDirectives")]{assert!(schemas[schema]["properties"][field]["anyOf"].as_array().unwrap().iter().any(|variant|variant["type"]=="null"));}
}
