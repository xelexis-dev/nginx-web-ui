use super::*;

#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn w11_dns_drift_after_certificate_reload_is_report_only() {
    let f=Fixture::new(true).await;
    let next=uuid::Uuid::new_v4().to_string();
    let mut original=f.old_files.clone();
    for value in original.values_mut(){if let Ok(text)=std::str::from_utf8(value){*value=text.replace("192.0.2.44:9000","flip.example.test:3001").into_bytes();}}
    f.store.stage(&next,Origin::Publisher,Some(store::digest(b"DNS drift fixture")),BTreeMap::new(),original.clone()).unwrap();f.store.select(&next).unwrap();files::restore_payload_files(&f.state.config,&original).await.unwrap();
    let saved_marker=json!({"upstreams":[],"servers":[{"id":"live","serverName":"example.test","listen":18081,"config":{"proxyPass":"http://flip.example.test:3001"}}],"streams":[]});
    f.client.execute("INSERT INTO applied_config(id,data) VALUES('current',$1)",&[&saved_marker]).await.unwrap();
    let marker:serde_json::Value=f.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get(0);
    let before=model::RenderedConfig{conf:files::managed_files(&f.state.config.nginx_conf_dir,files::ManagedKind::Conf).await.unwrap(),stream:files::managed_files(&f.state.config.nginx_stream_dir,files::ManagedKind::Stream).await.unwrap()};
    let public=|_:&str,_:u16|vec!["192.0.2.44".parse().unwrap()];
    assert!(crate::control::guard_targets::scan(&f.state.config,&before.conf,&before.stream,&public).is_empty());
    let internal=|_:&str,_:u16|vec!["127.0.0.1".parse().unwrap()];
    let _store=f.store.lock(Duration::from_secs(3)).await.unwrap();let _apply=f.state.runtime.apply_lock.lock().await;
    let mut operation=operation(&f.store);
    let result=deploy_core(&f.state,&f.store,&mut operation,&f.cert,&f.digest,&actor(),&confirmation::Budget::new(),Some(&internal)).await.unwrap();
    let response=json!({"applyError":if result.ok{serde_json::Value::Null}else{json!(result.output)}});
    assert!(response["applyError"].is_null(),"{response}");
    let after=model::RenderedConfig{conf:files::managed_files(&f.state.config.nginx_conf_dir,files::ManagedKind::Conf).await.unwrap(),stream:files::managed_files(&f.state.config.nginx_stream_dir,files::ManagedKind::Stream).await.unwrap()};
    assert_eq!(before.conf,after.conf);assert_eq!(before.stream,after.stream);
    assert_eq!(f.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get::<_,serde_json::Value>(0),marker);
    assert!(after.conf.contains_key("server-live.conf"),"DNS drift must not isolate a live server");
    let count:i64=f.client.query_one("SELECT count(*) FROM notifications WHERE severity='warning' AND body LIKE '%server-live.conf%'",&[]).await.unwrap().get(0);
    assert_eq!(count,1,"successful certificate reload must report the target hit");
    let audit:serde_json::Value=f.client.query_one("SELECT \"after\" FROM audit_logs WHERE action='nginx:apply' AND \"after\"->>'source'='certificate' AND target=$1",&[&f.cert]).await.unwrap().get(0);
    assert_eq!(audit["ok"],true);assert!(audit["output"].as_str().unwrap().contains("server-live.conf"));
    assert!(audit["output"].as_str().unwrap().contains("flip.example.test:3001"),"audit must name the configured destination");
}
