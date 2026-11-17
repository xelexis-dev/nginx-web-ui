use nwui_api::control::{model::ConfigSnapshotData,resources::{self,Resource,Mutation},reserved_ports::ReservedPorts};
use serde_json::json;
mod auth_support;
mod g13_support;

#[test]
fn q23_saved_unix_urls_render_verbatim_without_admin_binding() {
    let config=nwui_api::config::RuntimeConfig::read(|key|match key{"DATABASE_URL"=>Some("postgres://test@127.0.0.1/test".into()),"AUTH_SECRET"=>Some("unix-target-test".into()),"NWUI_CONTAINER"=>Some("1".into()),_=>None}).unwrap();
    let mut rows=Vec::new();
    for (i,target) in ["http://unix:/var/run/app.sock:/api/","http://unix:/tmp/x.sock"].iter().enumerate() {
        let mut row=resources::parse_server(&json!({"serverName":format!("socket-{i}.test"),"listen":19080+i,"proxyPass":target}),&ReservedPorts::default()).unwrap();
        row["id"]=json!(format!("11111111-1111-4111-8111-{:012}",i));row["enabled"]=json!(true);rows.push(row);
    }
    let out=g13_support::render(&config,&ConfigSnapshotData{servers:rows,..Default::default()},&[]);
    assert!(out.skipped.is_empty(),"{:?}",out.skipped);
    for (i,target) in ["http://unix:/var/run/app.sock:/api/","http://unix:/tmp/x.sock"].iter().enumerate() {
        let text=String::from_utf8_lossy(&out.rendered.conf[&format!("server-11111111-1111-4111-8111-{i:012}.conf")]);
        assert!(text.contains(&format!("proxy_pass {target};")),"{text}");assert!(!text.contains("proxy_bind"));
    }
    if let Ok(path)=std::env::var("NWUI_Q23_RENDER_OUT") {std::fs::write(path,serde_json::to_vec(&json!({"conf":out.rendered.conf,"stream":out.rendered.stream})).unwrap()).unwrap();}
}

#[cfg(unix)]
#[tokio::test]
#[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
async fn q23_unix_targets_save_and_apply_with_legacy_unchanged_edit() {
    use std::os::unix::fs::PermissionsExt;
    let temp=tempfile::tempdir().unwrap();let bin=temp.path().join("nginx");std::fs::write(&bin,"#!/bin/sh\nexit 0\n").unwrap();std::fs::set_permissions(&bin,std::fs::Permissions::from_mode(0o700)).unwrap();
    let conf=temp.path().join("conf");let streams=temp.path().join("stream");let certs=temp.path().join("certs");
    let mut db=auth_support::test_db_with(&[("NGINX_BIN",bin.to_str().unwrap()),("NGINX_CONF_D",conf.to_str().unwrap()),("NGINX_STREAM_D",streams.to_str().unwrap()),("CERTS_DIR",certs.to_str().unwrap())]).await;g13_support::witness(&mut db.state);
    let actor=nwui_api::contracts::Actor{via: None,id:None,label:Some("q23".into()),ip:None};
    for (i,target) in ["http://unix:/var/run/app.sock:/api/","http://unix:/tmp/x.sock"].iter().enumerate() {
        let row=resources::mutate(&db.state,Resource::Server,Mutation::Create,None,json!({"serverName":format!("socket-{i}.test"),"listen":19080+i,"proxyPass":target}),&actor).await.unwrap();
        let id=row["id"].as_str().unwrap();let applied=nwui_api::publisher_client::apply_saved(&db.state,&actor,None).await.unwrap();assert!(applied.ok,"{}",applied.output);
        assert!(std::fs::read_to_string(conf.join(format!("server-{id}.conf"))).unwrap().contains(&format!("proxy_pass {target};")));
        db.client.execute("UPDATE nginx_servers SET config=$1 WHERE id=$2",&[&json!({"proxyPass":"http://unix:/run/nwui-publisher/control.sock:/"}),&id]).await.unwrap();
        resources::mutate(&db.state,Resource::Server,Mutation::Update,Some(id),json!({"serverName":format!("edited-{i}.test"),"listen":19080+i,"proxyPass":"http://unix:/run/nwui-publisher/control.sock:/"}),&actor).await.unwrap();
        // The root publisher owns isolation of historical internal sockets, not save-time compatibility.
        db.client.execute("UPDATE nginx_servers SET config=$1 WHERE id=$2",&[&json!({"proxyPass":target}),&id]).await.unwrap();
    }
    resources::mutate(&db.state,Resource::Upstream,Mutation::Create,None,json!({"name":"unix_member","servers":["unix:8080"]}),&actor).await.unwrap();
}

#[tokio::test]
async fn relative_unix_target_keeps_the_server_and_its_other_working_locations(){
    if std::env::var_os("NWUI_Q23_RELATIVE_LOCATIONS_CHILD").is_none() {
        for prefix in ["nwui-q23-locations-","s6-rc:q23-locations-"] {
            let temp=tempfile::Builder::new().prefix(prefix).tempdir_in("/run").unwrap();
            let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","relative_unix_target_keeps_the_server_and_its_other_working_locations","--nocapture","--test-threads=1"]).env("NWUI_Q23_RELATIVE_LOCATIONS_CHILD","1").current_dir(temp.path()).output().unwrap();
            assert!(output.status.success(),"{}\n{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
            assert!(String::from_utf8_lossy(&output.stdout).lines().any(|line|line.starts_with("test result: ok. 1 passed; 0 failed;")));
        }
        return;
    }
    let cwd=std::env::current_dir().unwrap();assert!(nwui_api::control::internal_socket::matches_lexical(&cwd,&[]).is_some());
    let config=nwui_api::config::RuntimeConfig::read(|key|match key{"DATABASE_URL"=>Some("postgres://test@127.0.0.1/test".into()),"AUTH_SECRET"=>Some("relative-unix-test".into()),"NWUI_CONTAINER"=>Some("1".into()),_=>None}).unwrap();
    let target="http://unix:8080";
    let raw=json!({"serverName":"relative.test","listen":19084,"targetMode":"locations","locations":[{"path":"/api","mode":"direct","proxyPass":target},{"path":"/healthy","mode":"direct","proxyPass":"http://127.0.0.1:19000"}]});
    let mut row=resources::parse_server(&raw,&ReservedPorts::default()).unwrap();row["id"]=json!("22222222-2222-4222-8222-222222222222");row["enabled"]=json!(true);
    let socket=nwui_api::control::internal_socket::http_socket_path(target).unwrap();assert!(nwui_api::control::internal_socket::matches_resolved(socket,&[]).unwrap().is_none());
    let out=g13_support::render(&config,&ConfigSnapshotData{servers:vec![row],..Default::default()},&[]);assert!(out.skipped.is_empty(),"{:?}",out.skipped);
    let text=String::from_utf8_lossy(&out.rendered.conf["server-22222222-2222-4222-8222-222222222222.conf"]);assert!(text.contains("location /api"));assert!(text.contains(&format!("proxy_pass {target};")));assert!(text.contains("location /healthy"));assert!(text.contains("proxy_pass http://127.0.0.1:19000;"));
}
