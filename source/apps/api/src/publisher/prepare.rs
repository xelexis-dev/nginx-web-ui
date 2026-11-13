//! Read one saved snapshot, then validate an immutable generation without touching live files.
use super::{boot::StagedPayload, certificates, store::{PublisherStore,Origin,digest}};
use crate::{config::RuntimeConfig,control::{model::{self,CertificateRef,ConfigSnapshotData,RenderOutcome},model_render::{self,RenderContext}},crypto::SecretBox,upgrade::StepError};
use anyhow::{Context,Result,ensure};
use std::{collections::BTreeMap,path::Path,time::Duration};

fn error(error:impl std::fmt::Display)->StepError {super::store::step_error("publish_failed",error)}

pub(super) async fn prepare(
    mut config:RuntimeConfig,secrets:&SecretBox,pool:deadpool_postgres::Pool,store:PublisherStore,admitted:&crate::nonroot::Admitted,mode:super::boot::BootMode,
)->std::result::Result<StagedPayload,StepError> {
    // Use the process-cached effective nginx inputs; the environment mismatch is advisory only.
    config.nginx_bin=admitted.control_bin.clone();
    config.nginx_main_conf=admitted.conf_path.clone();
    let _store_owner=store.lock(Duration::from_secs(30)).await.map_err(error)?;
    let mut client=pool.get().await.map_err(error)?;
    let tx=client.build_transaction().isolation_level(tokio_postgres::IsolationLevel::RepeatableRead).start().await.map_err(error)?;
    let data=model::load_snapshot(&tx).await.map_err(error)?;
    let previous=model::load_applied_snapshot(&tx).await.map_err(error)?;
    let mut refs=model::load_certificate_refs(&tx).await.map_err(error)?;
    let mut materials=BTreeMap::new();let mut digests=BTreeMap::new();
    for cert in &mut refs {
        if cert.status!="issued" {continue;}
        match certificates::read(&tx,&cert.id,secrets,None).await {
            Ok(value)=>{
                digests.insert(cert.id.clone(),value.content_digest().to_owned());
                materials.insert(cert.id.clone(),value.into_material());cert.usable=true;
            },
            Err(certificates::Error::Invalid)=>cert.usable=false,
            Err(other)=>return Err(error(other)),
        }
    }
    // Root boot holds its own RR snapshot through render/staging and applied-state recording.
    let id=uuid::Uuid::new_v4().to_string();
    let generation=store.directory("payloads").map_err(stage_error)?.join(&id);
    let staged_certs=generation.join("certs");
    let render_config=config.clone();let render_data=data.clone();let render_refs=refs.clone();
    let (mut live,staged)=crate::control::internal_targets::blocking(move || {
        let dns=crate::dns_guard::status(&render_config);
        render_pair(&render_config,&render_data,&render_refs,&dns,&staged_certs,
            &crate::control::internal_targets::system_resolve,&crate::control::internal_targets::is_local_addr)
    }).await.map_err(error)?.map_err(error)?;
    super::vocab::validate(&config,&live.rendered.conf,&live.rendered.stream).map_err(error)?;
    let model_digest=digest(&serde_json::to_vec(&data).map_err(error)?);
    let previous_files=match store.manifest().map_err(error)?.current {
        Some(current)=>store.payload(&current).map_err(error)?.files,
        None=>super::files::capture_payload_files(&config).await.map_err(error)?,
    };
    let a8_clear=!super::confirmation::uncertain(&config).await;
    let stage=|id:&str,live:&RenderOutcome,staged:&RenderOutcome|->std::result::Result<_,StepError> {
        let generation=store.directory("payloads").map_err(stage_error)?.join(id);
        let view=generation.join(".test");
    let referenced=crate::publisher::files::certificate_consumers(&config,&live.rendered).map_err(error)?;
    let mut payload=BTreeMap::new();let mut used_digests=BTreeMap::new();
    for (prefix,files) in [("conf",&live.rendered.conf),("stream",&live.rendered.stream)] {
        payload.extend(files.iter().map(|(name,bytes)|(format!("{prefix}/{name}"),bytes.clone())));
    }
    for cert in referenced.keys() {
        let material=materials.get(cert).context("Rendered certificate material missing").map_err(error)?;
        payload.insert(format!("certs/{cert}/fullchain.pem"),material.cert_pem.as_bytes().to_vec());
        payload.insert(format!("certs/{cert}/privkey.pem"),material.key_pem.as_bytes().to_vec());
        used_digests.insert(cert.clone(),digests.get(cert).context("Certificate digest missing").map_err(error)?.clone());
    }
    let mut capacity=super::capacity::StageBudget::planned(&config,&id,&previous_files,a8_clear).map_err(stage_error)?;
    capacity.validation_dirs=vec![view.clone(),view.join("conf"),view.join("stream")];
    for(prefix,files)in [("conf",&staged.rendered.conf),("stream",&staged.rendered.stream)] {
        for(name,bytes)in files {capacity.validation.push(super::capacity::Write{path:view.join(prefix).join(name),bytes:bytes.len() as u64,inodes:1});}
    }
    capacity.validation.push(super::capacity::Write{path:view.join("marker"),bytes:view_marker(&id).len() as u64,inodes:1});
    capacity.validation.extend(super::staging::capacity_writes(&admitted.conf_path,&config,&generation,&id).map_err(error)?);
    let payload=store.stage_with_budget(&id,Origin::Publisher,Some(model_digest.clone()),used_digests,payload,&capacity).map_err(super::capacity::step)?;
    create_view(&view,&staged,&id).map_err(|error|super::capacity::step(super::capacity::stage_error(error)))?;
        Ok(payload)
    };
    let mut payload=stage(&id,&live,&staged)?;
    let initial=async {
        match super::staging::prepare_main(&admitted.conf_path,&config,&generation,&id).map_err(stage_error)? {
            super::staging::PreparedMain::Staged(main)=>{
                let mut view_config=view_config(&config,&generation);view_config.nginx_main_conf=main.path.clone();
                let report=super::containment::contain_staged(&view_config,&staged.rendered,&main.path,&store,&data,&config,mode).await.map_err(stage_error);
                main.remove().map_err(stage_error)?;
                let report=report?;
                if !report.validation.ok && mode==super::boot::BootMode::Conversion {return Err(super::command::test_failed_in(&report.validation.output,&prefixes(&view_config,&config)));}
                Ok((report.actions,!report.validation.ok))
            },
            super::staging::PreparedMain::LiveFallback(notice)=>{
                eprintln!("[publisher] {notice}");
                super::command::validate_includes(store.clone(),config.nginx_bin.clone(),None,Duration::from_secs(30)).await?;
                Ok((Vec::new(),false))
            },
        }
    }.await;
    remove_view(&generation)?;
    let (actions,incomplete)=initial?;
    let mut selected_id=id;
    if !actions.is_empty() || incomplete {
        for action in &actions {super::containment::set_unit(&mut live.rendered,&action.unit,None).map_err(stage_error)?;}
        super::apply::extend_containment(&mut live,&data,&actions);
        selected_id=uuid::Uuid::new_v4().to_string();
        let corrected_generation=store.directory("payloads").map_err(stage_error)?.join(&selected_id);
        let corrected=rebase(&config,&live,&refs,&corrected_generation.join("certs")).map_err(stage_error)?;
        payload=stage(&selected_id,&live,&corrected)?;
        let final_validation=async {
            match super::staging::prepare_main(&admitted.conf_path,&config,&corrected_generation,&selected_id).map_err(stage_error)? {
                super::staging::PreparedMain::Staged(main)=>{
                    let mut final_config=view_config(&config,&corrected_generation);final_config.nginx_main_conf=main.path.clone();
                    let result=super::command::validate_includes_in(store.clone(),config.nginx_bin.clone(),Some(main.path.clone()),Duration::from_secs(30),&prefixes(&final_config,&config)).await;
                    main.remove().map_err(stage_error)?;
                    result?;
                },
                super::staging::PreparedMain::LiveFallback(_)=>return Err(super::command::test_failed("격리한 설정의 전용 검증 파일을 만들지 못했습니다.")),
            }
            Ok::<_,StepError>(())
        }.await;
        remove_view(&corrected_generation)?;
        final_validation?;
    }
    crate::publisher::apply::record_applied_state(&tx,&live.effective(&data,Some(&previous)),&live.skipped,None).await.map_err(error)?;
    tx.commit().await.map_err(error)?;
    drop(client);
    let budget=crate::publisher::confirmation::Budget::new();
    crate::publisher::apply::notify_skipped_pool(&pool,&live.skipped,&budget).await;
    crate::publisher::apply::notify_notices_pool(&pool,&live.notices,&budget).await;
    if !actions.is_empty(){eprintln!("[publisher] start excluded {}",model::skipped_summary(&live.skipped));}
    Ok(StagedPayload {id:selected_id,manifest_sha256:payload.manifest_sha256,skipped:live.skipped})
}

fn stage_error(error:anyhow::Error)->StepError {
    if let Some(step)=error.downcast_ref::<StepError>() {return StepError{code:step.code,message:step.message,detail:step.detail.clone()};}
    super::capacity::step(super::capacity::stage_error(error))
}
fn view_config(config:&RuntimeConfig,generation:&Path)->RuntimeConfig {
    let mut staged=config.clone();staged.nginx_conf_dir=generation.join(".test/conf");staged.nginx_stream_dir=generation.join(".test/stream");staged.certs_dir=generation.join("certs");staged
}
fn prefixes<'a>(view:&'a RuntimeConfig,live:&'a RuntimeConfig)->[(&'a Path,&'a Path);4] {
    [(&view.nginx_main_conf,&live.nginx_main_conf),(&view.nginx_conf_dir,&live.nginx_conf_dir),(&view.nginx_stream_dir,&live.nginx_stream_dir),(&view.certs_dir,&live.certs_dir)]
}
fn remove_view(generation:&Path)->std::result::Result<(),StepError> {
    let view=generation.join(".test");
    // Preserve the durable cleanup record if a sibling could not be removed.
    if !view.join("main-record.json").exists(){std::fs::remove_dir_all(&view).map_err(|cause|stage_error(cause.into()))?;std::fs::File::open(generation).and_then(|file|file.sync_all()).map_err(|cause|stage_error(cause.into()))?;}
    Ok(())
}
fn rebase(config:&RuntimeConfig,live:&RenderOutcome,certs:&[CertificateRef],staged_certs:&Path)->Result<RenderOutcome> {
    let mut staged=live.clone();
    for files in [&mut staged.rendered.conf,&mut staged.rendered.stream] {
        for bytes in files.values_mut() {
            let mut text=String::from_utf8(bytes.clone())?;
            for cert in certs.iter().filter(|cert|cert.usable && cert.status=="issued") {
                for (directive,leaf) in [("ssl_certificate","fullchain.pem"),("ssl_certificate_key","privkey.pem")] {
                    text=text.replace(&format!("{directive} {};",config.certs_dir.join(&cert.id).join(leaf).display()),&format!("{directive} {};",staged_certs.join(&cert.id).join(leaf).display()));
                }
            }
            *bytes=text.into_bytes();
        }
    }
    Ok(staged)
}

fn view_marker(id:&str)->Vec<u8> {format!("add_header X-NWUI-Config-Generation \"{id}\";\n").into_bytes()}
fn create_view(view:&Path,outcome:&RenderOutcome,id:&str)->Result<()> {
    crate::db_upgrade::files::private_dir(view)?;
    for (prefix,files) in [("conf",&outcome.rendered.conf),("stream",&outcome.rendered.stream)] {
        let directory=view.join(prefix);crate::db_upgrade::files::private_dir(&directory)?;
        for (name,bytes) in files {super::store::payload_path(&format!("{prefix}/{name}"))?;super::store::new_private_file(&directory.join(name),bytes)?;}
    }
    super::store::new_private_file(&view.join("marker"),&view_marker(id))?;
    Ok(())
}

fn render_pair(
    config:&RuntimeConfig,data:&ConfigSnapshotData,certs:&[CertificateRef],dns:&crate::dns_guard::Descriptor,staged_certs:&Path,
    resolve:crate::control::internal_targets::Resolve<'_>,is_local:crate::control::internal_targets::LocalAddr<'_>,
)->Result<(RenderOutcome,RenderOutcome)> {
    let answers=std::cell::RefCell::new(BTreeMap::new());
    let local=std::cell::RefCell::new(BTreeMap::new());
    let resolve=|host:&str,port:u16|answers.borrow_mut().entry((host.to_owned(),port)).or_insert_with(||resolve(host,port)).clone();
    let probe=|ip|*local.borrow_mut().entry(ip).or_insert_with(||is_local(ip));
    let context=RenderContext {runtime_dns:dns,resolve:&resolve,is_local_addr:&probe};
    let mut live=model_render::render_with_context(config,data,certs,&context);
    live.rendered.conf.insert("00-runtime-upstreams.conf".into(),format!(
        "# Managed by Rust boot; API_PORT/PORT select these loopback listeners.\nupstream nwui_api {{ server 127.0.0.1:{}; keepalive 16; }}\nupstream nwui_web {{ server 127.0.0.1:{}; keepalive 16; }}\n",config.listen.port(),config.web_port).into_bytes());
    let mut staged_config=config.clone();staged_config.certs_dir=staged_certs.into();
    let mut staged=model_render::render_with_context(&staged_config,data,certs,&context);
    staged.rendered.conf.insert("00-runtime-upstreams.conf".into(),live.rendered.conf["00-runtime-upstreams.conf"].clone());
    ensure!(live.skipped==staged.skipped,"Staged rendering changed skipped records");
    // Resolve Unix paths once for the pair, so the validation view makes the same isolation decision.
    let socket_actions=super::unix_targets::filter_outcome(data,&mut live);
    super::unix_targets::apply_actions(data,&mut staged,&socket_actions);
    for (original,rebased) in [(&live.rendered.conf,&staged.rendered.conf),(&live.rendered.stream,&staged.rendered.stream)] {
        ensure!(original.len()==rebased.len(),"Staged rendering changed the file set");
        for (name,bytes) in original {
            let mut expected=String::from_utf8(bytes.clone())?;
            for cert in certs.iter().filter(|cert|cert.usable && cert.status=="issued") {
                for (directive,leaf) in [("ssl_certificate","fullchain.pem"),("ssl_certificate_key","privkey.pem")] {
                    let from=format!("{directive} {};",config.certs_dir.join(&cert.id).join(leaf).display());
                    let to=format!("{directive} {};",staged_certs.join(&cert.id).join(leaf).display());
                    expected=expected.replace(&from,&to);
                }
            }
            ensure!(rebased.get(name).is_some_and(|bytes|bytes==expected.as_bytes()),"Staged rendering changed bytes outside certificate prefixes: {name}");
        }
    }
    Ok((live,staged))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn q23_boot_pair_reuses_one_unix_isolation_decision_without_dns_or_preview_io() {
        let mut config=RuntimeConfig::read(|key|match key {"DATABASE_URL"=>Some("postgres://fixture@127.0.0.1/fixture".into()),"AUTH_SECRET"=>Some("prepare-unix-fixture".into()),_=>None}).unwrap();config.in_container=true;
        let bad=uuid::Uuid::new_v4().to_string();let good=uuid::Uuid::new_v4().to_string();
        let data=ConfigSnapshotData {servers:vec![
            json!({"id":bad,"serverName":"internal.example.test","listen":18081,"enabled":true,"config":{"proxyPass":"http://unix:/run/nwui-publisher/control.sock"}}),
            json!({"id":good,"serverName":"public.example.test","listen":18082,"enabled":true,"config":{"proxyPass":"http://unix:/tmp/q23-public.sock:$request_uri"}}),
        ],..Default::default()};
        let dns=crate::dns_guard::Descriptor::available(&config.target_dns);
        let (live,staged)=render_pair(&config,&data,&[],&dns,Path::new("/private/q23/certs"),&|_,_|panic!("Unix target reached DNS"),&|_|panic!("Unix target reached TCP classification")).unwrap();
        assert_eq!(live.skipped.len(),1);assert_eq!(live.skipped,staged.skipped);assert_eq!(live.skipped[0].id,bad);assert!(live.skipped[0].reason.contains("unix"));
        assert!(!live.rendered.conf.contains_key(&format!("server-{bad}.conf")));assert!(live.rendered.conf.contains_key(&format!("server-{good}.conf")));
        assert_eq!(live.rendered.conf,staged.rendered.conf);assert_eq!(live.effective(&data,None).servers.len(),1);
    }
    #[test]
    fn both_renders_cache_the_same_dns_and_probe_observations_and_only_cert_prefixes_differ() {
        let config=RuntimeConfig::read(|key|match key {"DATABASE_URL"=>Some("postgres://fixture@127.0.0.1/fixture".into()),"AUTH_SECRET"=>Some("prepare-render".into()),_=>None}).unwrap();
        let server=uuid::Uuid::new_v4().to_string();let cert=uuid::Uuid::new_v4().to_string();
        let data=ConfigSnapshotData {servers:vec![json!({"id":server,"serverName":"public.example.test","listen":18081,"enabled":true,"certificateId":cert,"config":{"tlsPort":18443,"proxyPass":"http://admin.example.test:8080"}})],..Default::default()};
        let refs=vec![CertificateRef {id:cert,status:"issued".into(),usable:true}];
        let dns=crate::dns_guard::Descriptor::available(&config.target_dns);
        let queries=std::cell::Cell::new(0);let probes=std::cell::Cell::new(0);
        let (live,staged)=render_pair(&config,&data,&refs,&dns,Path::new("/private/generation/certs"),&|_,_|{queries.set(queries.get()+1);vec![std::net::Ipv4Addr::LOCALHOST.into()]},&|_|{probes.set(probes.get()+1);true}).unwrap();
        assert!(live.skipped.is_empty(),"{:?}",live.skipped);
        let name=format!("server-{server}.conf");
        assert_ne!(live.rendered.conf[&name],staged.rendered.conf[&name]);
        assert_eq!(queries.get(),1);assert_eq!(probes.get(),1);
        assert!(String::from_utf8(staged.rendered.conf[&name].clone()).unwrap().contains("/private/generation/certs/"));
    }
}

#[cfg(test)]
mod containment_state_tests {
    use super::*;
    use std::{fs,os::unix::fs::PermissionsExt};
    use serde_json::json;
    use crate::publisher::boot::BootMode;

    #[test]
    fn corrected_cert_prefixes_use_the_new_generation_without_rendering_or_changing_live_bytes() {
        let config=RuntimeConfig::read(|key|match key{"DATABASE_URL"=>Some("postgres://fixture@127.0.0.1/fixture".into()),"AUTH_SECRET"=>Some("prefix-fixture".into()),_=>None}).unwrap();
        let cert=uuid::Uuid::new_v4().to_string();let refs=vec![CertificateRef{id:cert.clone(),status:"issued".into(),usable:true}];
        let mut live=RenderOutcome::default();live.rendered.conf.insert("server-fixture.conf".into(),format!("ssl_certificate {};\nssl_certificate_key {};\n# stable\n",config.certs_dir.join(&cert).join("fullchain.pem").display(),config.certs_dir.join(&cert).join("privkey.pem").display()).into_bytes());
        let before=live.rendered.conf.clone();let first=rebase(&config,&live,&refs,Path::new("/private/first/certs")).unwrap();let final_view=rebase(&config,&live,&refs,Path::new("/private/final/certs")).unwrap();
        assert_eq!(live.rendered.conf,before);assert_ne!(first.rendered.conf,final_view.rendered.conf);let text=String::from_utf8(final_view.rendered.conf["server-fixture.conf"].clone()).unwrap();assert!(text.contains("/private/final/certs/"));assert!(!text.contains("/private/first/"));assert!(text.ends_with("# stable\n"));
    }
    #[tokio::test]
    #[ignore="real PostgreSQL: NWUI_TEST_DATABASE_URL"]
    async fn contained_boot_publishes_one_corrected_origin_preserves_saved_rows_and_dedupes_then_recovers() {
        let Ok(admin)=std::env::var("NWUI_TEST_DATABASE_URL")else{return};
        async fn connect(url:&str)->tokio_postgres::Client {let(c,f)=tokio_postgres::connect(url,tokio_postgres::NoTls).await.unwrap();tokio::spawn(f);c}
        let db=format!("x1_{}",uuid::Uuid::new_v4().simple());connect(&admin).await.batch_execute(&format!("CREATE DATABASE {db}")).await.unwrap();let url=format!("{}/{db}",admin.rsplit_once('/').unwrap().0);
        let root=tempfile::tempdir().unwrap();let mut config=RuntimeConfig::read(|k|match k{"DATABASE_URL"=>Some(url.clone()),"AUTH_SECRET"=>Some("x1-db-only".into()),"DATA_KEY_FILE"=>Some(root.path().join("key").display().to_string()),"API_DB_POOL_SIZE"=>Some("2".into()),_=>None}).unwrap();
        config.nginx_conf_dir=root.path().join("live/conf");config.nginx_stream_dir=root.path().join("live/stream");config.certs_dir=root.path().join("live/certs");config.nginx_main_conf=root.path().join("nginx.conf");config.nginx_bin=root.path().join("nginx");config.target_dns.descriptor_path=root.path().join("dns.json");
        fs::write(&config.data_key_file,"A5".repeat(32)).unwrap();fs::set_permissions(&config.data_key_file,fs::Permissions::from_mode(0o600)).unwrap();
        for dir in [&config.nginx_conf_dir,&config.nginx_stream_dir,&config.certs_dir]{fs::create_dir_all(dir).unwrap();}
        fs::write(config.nginx_conf_dir.join(".nwui-apply-uncertain"),b"original A8").unwrap();
        fs::write(&config.nginx_main_conf,format!("events {{}} http {{ include {}/*.conf; server {{ listen 127.0.0.1:8080; location = /nginx_status {{ stub_status; allow 127.0.0.3; deny all; include {}/.nwui-generation; }} }} }}",config.nginx_conf_dir.display(),config.nginx_conf_dir.display())).unwrap();fs::set_permissions(&config.nginx_main_conf,fs::Permissions::from_mode(0o600)).unwrap();
        let app=root.path().join("app");fs::create_dir(&app).unwrap();fs::set_permissions(&app,fs::Permissions::from_mode(0o700)).unwrap();let store=super::super::test_support::open_store_fixture(&app).unwrap();
        fs::write(&config.nginx_bin,format!(r#"#!/bin/sh
[ "$#" -eq 4 ] && [ "$1" = -t ] && [ "$2" = -T ] && [ "$3" = -c ] || exit 91
id=${{4##*.nwui-staged-}}
if [ -e '{r}/fail-final' ]; then echo 'nginx: [emerg] broken main config' >&2; exit 1; fi
if [ ! -e '{r}/resolved' ]; then
 for file in '{store}/payloads/'"$id"'/.test/conf/'*.conf; do
  if grep -q absent-svc "$file"; then printf 'nginx: [emerg] host not found in upstream "absent-svc:80" in %s:1\n' "$file" >&2; exit 1; fi
 done
fi
if [ -e '{r}/arm-final' ]; then touch '{r}/fail-final'; fi
printf '# configuration file %s:\n' "$4"
"#,r=root.path().display(),store=store.root.display())).unwrap();fs::set_permissions(&config.nginx_bin,fs::Permissions::from_mode(0o700)).unwrap();
        let state=crate::state::AppState::new(config.clone()).unwrap();crate::migrations::run(&state.db,&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle")).await.unwrap();let client=connect(&url).await;
        let good=uuid::Uuid::new_v4().to_string();let bad=uuid::Uuid::new_v4().to_string();let consumer=uuid::Uuid::new_v4().to_string();let up=uuid::Uuid::new_v4().to_string();
        client.execute("INSERT INTO upstreams(id,name,servers)VALUES($1,'pool',$2)",&[&up,&json!(["absent-svc:80"])]).await.unwrap();
        for(id,name,listen,target)in[(&good,"good.example.test",18081,"http://192.0.2.10:80"),(&bad,"bad.example.test",18082,"http://absent-svc:80"),(&consumer,"consumer.example.test",18083,"http://pool")]{client.execute("INSERT INTO nginx_servers(id,server_name,listen,config)VALUES($1,$2,$3,$4)",&[id,&name,&listen,&json!({"proxyPass":target})]).await.unwrap();}
        let saved=serde_json::to_value(model::load_snapshot(&state.db.get().await.unwrap()).await.unwrap()).unwrap();
        let admitted=crate::nonroot::Admitted{master_bin:config.nginx_bin.clone(),control_bin:config.nginx_bin.clone(),conf_path:config.nginx_main_conf.clone(),conf_mismatch:false};
        for mode in [BootMode::Conversion,BootMode::Ready] {
            let before=fs::read_dir(store.directory("payloads").unwrap()).unwrap().map(|e|e.unwrap().file_name()).collect::<std::collections::BTreeSet<_>>();
            let staged=prepare(config.clone(),&state.secrets,state.db.clone(),store.clone(),&admitted,mode).await.unwrap();
            let payload=store.payload(&staged.id).unwrap();assert!(payload.manifest.origin==Origin::Publisher);assert!(payload.manifest.derived_from.is_none());assert!(payload.manifest.reason.is_none());assert_eq!(payload.manifest_sha256,staged.manifest_sha256);assert_eq!(staged.skipped.len(),3);assert!(staged.skipped.iter().all(|s|!s.kept_previous && s.reason.contains("absent-svc") && !s.reason.contains("/.test/")));
            assert!(payload.files.contains_key(&format!("conf/server-{good}.conf")));assert!(!payload.files.contains_key(&format!("conf/server-{bad}.conf")));assert!(!payload.files.contains_key(&format!("conf/server-{consumer}.conf")));assert!(!String::from_utf8_lossy(&payload.files["conf/upstreams.conf"]).contains("upstream pool"));
            let created=fs::read_dir(store.directory("payloads").unwrap()).unwrap().map(|e|e.unwrap().file_name()).filter(|id|!before.contains(id)).collect::<Vec<_>>();assert_eq!(created.len(),2);let original=created.iter().find(|id|id.to_string_lossy()!=staged.id).unwrap();let original=store.payload(&original.to_string_lossy()).unwrap();assert!(original.files.contains_key(&format!("conf/server-{bad}.conf")));assert!(original.manifest.origin==Origin::Publisher);
            let applied:serde_json::Value=client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get(0);assert_eq!(applied["skipped"],serde_json::to_value(&staged.skipped).unwrap());assert_eq!(applied["servers"].as_array().unwrap().len(),1);
        }
        assert_eq!(client.query_one("SELECT count(*) FROM notifications WHERE type='config.apply-skipped'",&[]).await.unwrap().get::<_,i64>(0),1);
        assert_eq!(serde_json::to_value(model::load_snapshot(&state.db.get().await.unwrap()).await.unwrap()).unwrap(),saved);assert_eq!(fs::read(config.nginx_conf_dir.join(".nwui-apply-uncertain")).unwrap(),b"original A8");
        // The second immutable generation gets a fresh capacity admission before any commit.
        for errno in [libc::ENOSPC,libc::EDQUOT] {
            let mut limited=store.clone();let probes=std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));let seen=probes.clone();
            limited.capacity_probe=Some(std::sync::Arc::new(move||if seen.fetch_add(1,std::sync::atomic::Ordering::SeqCst)==0 {Ok(super::super::capacity::Available{block_size:4096,bytes:u64::MAX,inodes:None})}else{Err(std::io::Error::from_raw_os_error(errno).into())}));
            let before:serde_json::Value=client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get(0);
            let failure=prepare(config.clone(),&state.secrets,state.db.clone(),limited,&admitted,BootMode::Ready).await.unwrap_err();assert_eq!(failure.code,"publisher_storage_full");assert!(probes.load(std::sync::atomic::Ordering::SeqCst)>=2);
            assert_eq!(client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get::<_,serde_json::Value>(0),before);assert_eq!(fs::read(config.nginx_conf_dir.join(".nwui-apply-uncertain")).unwrap(),b"original A8");
        }
        // A final command has its own budget and must succeed before metadata or notices change.
        fs::write(root.path().join("arm-final"),b"1").unwrap();let applied:serde_json::Value=client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get(0);
        assert_eq!(prepare(config.clone(),&state.secrets,state.db.clone(),store.clone(),&admitted,BootMode::Ready).await.unwrap_err().code,"nginx_test_failed");assert_eq!(client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get::<_,serde_json::Value>(0),applied);
        fs::remove_file(root.path().join("arm-final")).unwrap();fs::remove_file(root.path().join("fail-final")).unwrap();fs::write(root.path().join("resolved"),b"1").unwrap();
        let recovered=prepare(config.clone(),&state.secrets,state.db.clone(),store.clone(),&admitted,BootMode::Ready).await.unwrap();assert!(recovered.skipped.is_empty());assert!(store.payload(&recovered.id).unwrap().files.contains_key(&format!("conf/server-{bad}.conf")));assert_eq!(serde_json::to_value(model::load_snapshot(&state.db.get().await.unwrap()).await.unwrap()).unwrap(),saved);
        assert!(!config.nginx_conf_dir.join(format!("server-{good}.conf")).exists());assert_eq!(fs::read(config.nginx_conf_dir.join(".nwui-apply-uncertain")).unwrap(),b"original A8");
    }
}
