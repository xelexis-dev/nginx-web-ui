use super::*;
use crate::{control::model::ConfigSnapshotData,publisher::{boot::BootMode,store::PublisherStore}};
use std::{path::Path,time::Duration,os::unix::fs::PermissionsExt};
use serde_json::json;

fn fixture(count:usize)->(tempfile::TempDir,RuntimeConfig,RuntimeConfig,RenderedConfig,ConfigSnapshotData,PublisherStore) {
    let temp=tempfile::tempdir().unwrap();let mut config=RuntimeConfig::read(|k|match k{"DATABASE_URL"=>Some("postgres://fixture@127.0.0.1/fixture".into()),"AUTH_SECRET"=>Some("boot-containment".into()),_=>None}).unwrap();
    config.nginx_conf_dir=temp.path().join("view/conf");config.nginx_stream_dir=temp.path().join("view/stream");config.certs_dir=temp.path().join("payload/certs");config.nginx_bin=temp.path().join("nginx");
    for d in [&config.nginx_conf_dir,&config.nginx_stream_dir]{std::fs::create_dir_all(d).unwrap();}
    let mut live_config=config.clone();live_config.nginx_conf_dir=temp.path().join("live/conf");live_config.nginx_stream_dir=temp.path().join("live/stream");live_config.certs_dir=temp.path().join("live/certs");
    let mut files=RenderedConfig::default();let mut data=ConfigSnapshotData::default();
    for n in 0..count {let id=uuid::Uuid::new_v4().to_string();data.servers.push(json!({"id":id,"serverName":format!("server-{n}"),"config":{"proxyPass":"http://absent-svc:80"}}));files.conf.insert(format!("server-{id}.conf"),b"server { BAD; }\n".to_vec());}
    files.conf.insert("00-runtime-upstreams.conf".into(),b"# product sentinel\n".to_vec());
    std::fs::write(config.nginx_conf_dir.join("00-runtime-upstreams.conf"),b"# product sentinel\n").unwrap();
    std::fs::write(config.nginx_conf_dir.join("server-foreign.conf"),b"# foreign sentinel\n").unwrap();
    std::fs::write(config.nginx_conf_dir.parent().unwrap().join("marker"),b"# marker sentinel\n").unwrap();
    let store=PublisherStore::at(temp.path().join("store"));
    (temp,config,live_config,files,data,store)
}
fn bin(config:&RuntimeConfig,body:&str){std::fs::write(&config.nginx_bin,format!("#!/bin/sh\n{body}\n")).unwrap();std::fs::set_permissions(&config.nginx_bin,std::fs::Permissions::from_mode(0o700)).unwrap();}
#[test]
fn fatal_reason_uses_attributed_line_normalizes_paths_and_redacts_secret_operands() {
    let raw="warning\nnginx: [alert] invalid port in upstream \"user:PASS@192.0.2.10:8081/path?token=QUERY#FRAG\" in /private/view/conf/server-x.conf:3\nnginx: [emerg] later secret\n";
    let line=reason(raw,&[(Path::new("/private/view/conf"),Path::new("/live/conf"))]);
    assert_eq!(line,"nginx: [alert] invalid port in upstream \"192.0.2.10:8081/path\" in /live/conf/server-x.conf:3");
    for secret in ["PASS","QUERY","FRAG","later"]{assert!(!line.contains(secret));}
    assert!(reason("nginx: [emerg] host not found in upstream \"absent-svc:80\"",&[]).contains("absent-svc:80"));
    assert!(!reason("nginx: [crit] bad https://user:PASS@example.test/p?QUERY",&[]).contains("PASS"));
    let bounded=reason(&format!("nginx: [emerg] {}","한".repeat(1000)),&[]);assert!(bounded.len()<=1024);assert!(!bounded.chars().any(char::is_control));
}
#[tokio::test]
async fn staged_rounds_remove_more_than_ten_bad_units_and_leave_product_and_live_files_untouched() {
    let (temp,config,live,files,data,store)=fixture(12);let main=temp.path().join("main");std::fs::write(&main,"events {}\n").unwrap();std::fs::set_permissions(&main,std::fs::Permissions::from_mode(0o600)).unwrap();
    bin(&config,&format!(r#"[ "$#" -eq 4 ] && [ "$1" = -t ] && [ "$2" = -T ] && [ "$3" = -c ] || exit 91
for file in '{}' server-never; do :; done
for file in '{}'/*.conf; do
 if grep -q BAD "$file"; then printf 'nginx: [emerg] host not found in upstream "absent-svc:80" in %s:1\n' "$file" >&2; exit 1; fi
done
printf '# configuration file %s:\n' "$4""#,config.nginx_conf_dir.display(),config.nginx_conf_dir.display()));
    let report=contain_staged(&config,&files,&main,&store,&data,&live,BootMode::Conversion).await.unwrap();
    assert!(report.validation.ok);assert_eq!(report.actions.len(),12);assert!(report.actions.iter().all(|a|!a.kept_previous));
    assert!(!live.nginx_conf_dir.exists());assert_eq!(std::fs::read(config.nginx_conf_dir.join("00-runtime-upstreams.conf")).unwrap(),b"# product sentinel\n");
    assert_eq!(std::fs::read(config.nginx_conf_dir.join("server-foreign.conf")).unwrap(),b"# foreign sentinel\n");assert_eq!(std::fs::read(config.nginx_conf_dir.parent().unwrap().join("marker")).unwrap(),b"# marker sentinel\n");
    for action in &report.actions{assert!(!action.reason.contains("/view/"));assert!(action.reason.contains("absent-svc:80"));}
}
#[test]
fn shared_upstream_removal_cascades_id_and_named_consumers_before_dns_can_win() {
    let (_temp,_config,_live,mut files,mut data,_store)=fixture(3);
    data.upstreams.push(json!({"id":"pool-id","name":"pool"}));files.conf.insert("upstreams.conf".into(),b"upstream pool { server absent:80; }\n".to_vec());
    data.servers[0]["config"]=json!({"locations":[{"upstreamId":"pool-id"}]});data.servers[1]["config"]=json!({"locations":[{"proxyPass":"http://POOL/path"}]});
    data.servers[2]["upstreamId"]=json!("pool-id");data.servers[2]["config"]=json!({});
    let mut units=vec![Unit::UpstreamBlock("pool".into())];boot_consumers(&data,&files,&mut units);assert_eq!(units.len(),4);
    for unit in &units {set_unit(&mut files,unit,None).unwrap();}assert_eq!(files.conf.len(),2);assert!(!String::from_utf8_lossy(&files.conf["upstreams.conf"]).contains("upstream pool"));
}
#[tokio::test]
async fn cutoff_and_initial_or_later_timeout_fallback_only_on_ready_with_original_and_removed_causes() {
    for mode in [BootMode::Conversion,BootMode::Ready] {for later in [false,true] {
        let(temp,config,live,files,data,store)=fixture(2);let main=temp.path().join("main");let marker=temp.path().join("calls");
        let first=files.conf.keys().find(|name|name.starts_with("server-")).unwrap();
        bin(&config,&if later {format!("if [ ! -e '{}' ]; then touch '{}'; echo 'nginx: [emerg] bad in {}/{}:1' >&2; exit 1; fi\nsleep 60",marker.display(),marker.display(),config.nginx_conf_dir.display(),first)}else{"sleep 60".into()});
        let mut boot=BootView{main:&main,store:&store,data:&data,live_config:&live,view_config:&config,mode,timeout:Some(Duration::from_millis(100))};
        let report=run(&config,files.clone(),Strategy::Previous(&RenderedConfig::default()),||Duration::ZERO,None,None,Some(&mut boot)).await.unwrap();
        assert!(!report.validation.ok);assert_eq!(report.actions.len(),if mode==BootMode::Ready{2}else{usize::from(later)});
        if mode==BootMode::Ready{assert!(report.actions.iter().any(|a|a.reason.contains("함께 제외")));}
        let report=run(&config,files.clone(),Strategy::Previous(&RenderedConfig::default()),||Duration::from_secs(60),None,None,Some(&mut boot)).await.unwrap();
        assert!(!report.validation.ok);assert_eq!(report.actions.len(),if mode==BootMode::Ready{2}else{0});
    }}
}
