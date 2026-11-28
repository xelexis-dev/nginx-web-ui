//! Certificate-only publication uses the publisher's durable live view, never saved drafts.
use crate::{contracts,control::model,state::AppState};
use super::{confirmation,journal,store::PublisherStore};

pub(super) async fn deploy(
    state:&AppState,store:&PublisherStore,operation:&mut journal::Operation,id:&str,digest:&str,
    actor:&contracts::Actor,budget:&confirmation::Budget,
)->anyhow::Result<model::ApplyResult> {
    deploy_core(state,store,operation,id,digest,actor,budget,#[cfg(test)] None).await
}
async fn deploy_core(
    state:&AppState,store:&PublisherStore,operation:&mut journal::Operation,id:&str,digest:&str,
    actor:&contracts::Actor,budget:&confirmation::Budget,
    #[cfg(test)] resolve:Option<&(dyn Fn(&str,u16)->Vec<std::net::IpAddr>+Send+Sync)>,
)->anyhow::Result<model::ApplyResult> {
    use super::{apply,files,journal::Phase,store::Origin};
    use anyhow::{Context,ensure};
    use std::{collections::BTreeMap,time::{Duration,Instant}};
    let mut client=budget.admit(state.db.get()).await??;
    let tx=client.build_transaction().isolation_level(tokio_postgres::IsolationLevel::RepeatableRead).start().await?;
    let mut _files_owner=None;
    let mut audit_submitted=false;let mut live_started=false;let mut sql_committed=false;let mut notices=Vec::new();
    let result=async {
        apply::lock_tables(&tx,budget).await?;
        _files_owner=Some(budget.admit(super::nginx_lock::acquire(&state.config)).await??);
        ensure!(operation.phase==Phase::Intent && operation.next.is_none(),"Certificate deploy already started");
        operation.previous_uncertain=confirmation::uncertain(&state.config).await;
        operation.save(store)?;
        let previous_uncertain=operation.previous_uncertain;
        let live=model::RenderedConfig {
            conf:files::managed_files(&state.config.nginx_conf_dir,files::ManagedKind::Conf).await?,
            stream:files::managed_files(&state.config.nginx_stream_dir,files::ManagedKind::Stream).await?,
        };
        let consumers=files::certificate_consumers(&state.config,&live)?;
        let requested=apply::certificate_material(&tx,state,id,Some(digest)).await?;
        let empty=std::collections::BTreeSet::new();
        let linked=consumers.get(id).unwrap_or(&empty);
        if linked.is_empty() {
            apply::certificate_activation_notice(&tx,id,digest,linked).await?;
            let applied=model::ApplyResult{ok:true,output:String::new(),rolled_back_to:None};
            save_result(store,operation,successful_result(&applied,previous_uncertain))?;
            apply::set_completion_timeout(&tx,budget).await?;commit(tx,budget).await?;
            operation.advance(store,Phase::Done)?;
            return Ok(applied);
        }
        let refusal = match super::vocab::validate(&state.config, &live.conf, &live.stream) {
            Err(error) => Some((error.to_string(), true)),
            Ok(()) if !matches!(confirmation::nginx_running(&state.config).await, Ok(confirmation::NginxState::Running)) =>
                Some(("nginx 프로세스를 확인하지 못해 인증서를 적용하지 않았습니다. 잠시 후 다시 시도하세요.".into(), false)),
            Ok(()) => None,
        };
        if let Some((output, vocabulary_rejected)) = refusal {
            apply::set_completion_timeout(&tx,budget).await?;
            let applied=model::ApplyResult{ok:false,output,rolled_back_to:None};
            apply::certificate_audit(&tx,actor,id,false,&applied.output).await?;audit_submitted=true;
            save_result(store,operation,successful_result(&applied,previous_uncertain))?;commit(tx,budget).await?;
            if vocabulary_rejected {
                operation.advance(store,Phase::Done)?;
            }
            return Ok(applied);
        }
        apply::certificate_activation_notice(&tx,id,digest,linked).await?;
        let previous=store.ensure_previous(&state.config,operation).await?;let old=store.payload(&previous)?;
        ensure!(files::capture_payload_files(&state.config).await?==old.files,"Live payload differs from its durable generation");
        let mut materials=vec![requested];let mut digests=BTreeMap::from([(id.to_owned(),digest.to_owned())]);
        for other in consumers.keys().filter(|other|other.as_str()!=id) {
            materials.push(apply::certificate_material(&tx,state,other,None).await?);
            let row=tx.query_one("SELECT cert_pem,key_pem FROM certificates WHERE id=$1 AND status='issued'",&[other]).await?;
            let cert:Option<String>=row.get(0);let key:Option<String>=row.get(1);
            digests.insert(other.clone(),model::certificate_content_digest(&cert.context("Missing certificate")?,&key.context("Missing certificate key")?));
        }
        budget.forward()?;
        let mut next_files=BTreeMap::new();
        for(prefix,entries)in [("conf",&live.conf),("stream",&live.stream)] {
            next_files.extend(entries.iter().map(|(name,bytes)|(format!("{prefix}/{name}"),bytes.clone())));
        }
        // The boot-owned loopback upstream file is outside managed_files and must stay byte-identical.
        if let Some(bytes)=old.files.get("conf/00-runtime-upstreams.conf") {
            next_files.insert("conf/00-runtime-upstreams.conf".into(),bytes.clone());
        }
        for material in materials {
            next_files.insert(format!("certs/{}/fullchain.pem",material.id),material.cert_pem.into_bytes());
            next_files.insert(format!("certs/{}/privkey.pem",material.id),material.key_pem.into_bytes());
        }
        let next=uuid::Uuid::new_v4().to_string();
        let capacity=super::capacity::StageBudget::new(&state.config,operation,&old.files,!previous_uncertain);
        store.stage_with_budget(&next,Origin::Publisher,old.manifest.model_digest,digests,next_files,&capacity)?;
        operation.previous=Some(previous);operation.next=Some(next.clone());operation.uncertain=true;
        operation.advance(store,Phase::PayloadStaged)?;budget.forward()?;
        operation.advance(store,Phase::LiveWriting)?;live_started=true;
        // write_live persists A8 before replacing any member of the complete payload.
        store.write_live(&state.config,operation).await?;
        let first=uuid::Uuid::new_v4().to_string();confirmation::publish(&state.config,&first).await?;
        let output=super::command::execute(store.clone(),state.config.nginx_bin.clone(),vec!["-t".into()],budget.remaining().saturating_sub(confirmation::RESTORE_RESERVE).min(Duration::from_secs(30))).await.map_err(command_error)?;
        operation.advance(store,Phase::Validated)?;
        let witness=confirmation::Witness::from_config(&state.config)?;
        witness.confirm_with(&state.config,&first,budget.ack_deadline()?,store,|phase|operation.advance(store,if phase==0{Phase::P}else{Phase::C})).await?;
        notices=target_notices_core(state,#[cfg(test)] resolve).await;
        let mut output=output;for notice in &notices{output.push_str(&format!("\n\n{notice}"));}
        let applied=model::ApplyResult{ok:true,output,rolled_back_to:None};
        save_result(store,operation,successful_result(&applied,previous_uncertain))?;
        apply::set_completion_timeout(&tx,budget).await?;
        apply::certificate_audit(&tx,actor,id,true,&applied.output).await?;audit_submitted=true;
        commit(tx,budget).await?;sql_committed=true;
        operation.uncertain=operation.previous_uncertain;operation.advance(store,Phase::DbCommitted)?;store.select(&next)?;
        if !operation.previous_uncertain {confirmation::clear_uncertain(&state.config).await?;}
        operation.advance(store,Phase::Done)?;
        Ok::<_,anyhow::Error>(applied)
    }.await;
    let result=match result {
        Err(error)=>{
            let output=error.to_string();
            let failed=if super::capacity::is_full(&error){super::capacity::result()}else if output=="stale_material"{super::protocol::OpResult::StaleMaterial}else{super::protocol::OpResult::Failed{code:"publish_failed".into(),output:output.clone()}};
            save_result(store,operation,failed)?;
            if sql_committed {
                // Known SQL commit can only finish; restoring would contradict its durable outcome.
                operation.uncertain=true;let _=operation.save(store);let _=confirmation::begin_uncertain(&state.config).await;
                Err(error)
            }else if live_started {
                if audit_submitted&&operation.phase==Phase::C {operation.advance(store,Phase::CommitUnknown)?;}
                let previous=operation.previous.clone().context("Missing durable previous certificate payload")?;
                operation.begin_restore(store,&previous,"certificate deployment failed")?;
                let restored=super::recovery::finish_restore(&state.config,store,operation,Instant::now()+budget.remaining().min(confirmation::RESTORE_RESERVE)).await;
                match restored {
                    Ok(())=>Err(error),
                    Err(restore)=>{
                        let failure=restore.context(format!("{error:#}; 이전 인증서의 동작을 확인하지 못했습니다."));
                        save_result(store,operation,super::protocol::OpResult::Failed{code:"publish_failed".into(),output:failure.to_string()})?;
                        Err(failure)
                    }
                }
            }else{
                if matches!(operation.phase,Phase::Intent|Phase::PayloadStaged) {
                    operation.advance(store,Phase::Done)?;
                }
                Err(error)
            }
        },
        other=>other,
    };
    if let Err(error)=&result {
        if !audit_submitted {
            // The prior RR transaction has rolled back; reuse this checkout (including pool size1).
            let audit=async {
                let tx=client.transaction().await?;tx.batch_execute("SET LOCAL statement_timeout='2000ms'; SET LOCAL lock_timeout='2000ms'").await?;
                apply::certificate_audit(&tx,actor,id,false,&format!("{error:#}")).await?;tx.commit().await?;Ok::<_,anyhow::Error>(())
            };
            let _=tokio::time::timeout(budget.remaining().min(Duration::from_secs(3)),audit).await;
        }
    }
    drop(client);
    if result.is_ok(){apply::notify_notices(state,&notices,budget).await;}
    result
}

fn successful_result(applied:&model::ApplyResult,uncertain:bool)->super::protocol::OpResult {
    super::protocol::OpResult::Ok{apply:Some(applied.clone()),skipped:Vec::new(),uncertain,phase:"done".into()}
}
fn save_result(store:&PublisherStore,operation:&mut journal::Operation,result:super::protocol::OpResult)->anyhow::Result<()> {
    operation.result=Some(serde_json::to_value(result)?);operation.save(store)
}

fn command_error(error:crate::upgrade::StepError)->anyhow::Error {
    anyhow::anyhow!(error.detail.unwrap_or_else(||format!("{}: {}",error.code,error.message)))
}
async fn commit(tx:deadpool_postgres::Transaction<'_>,budget:&confirmation::Budget)->anyhow::Result<()> {
    use anyhow::Context;
    tokio::time::timeout(budget.remaining().saturating_sub(confirmation::RESTORE_RESERVE),tx.commit()).await
        .context("설정 DB 커밋 결과를 확인하지 못했습니다.")?.context("설정 DB 커밋 결과를 확인하지 못했습니다.")?;Ok(())
}
#[cfg(test)]
async fn target_notices(state:&AppState)->Vec<String> {target_notices_core(state,#[cfg(test)] None).await}
async fn target_notices_core(state:&AppState,#[cfg(test)] resolve:Option<&(dyn Fn(&str,u16)->Vec<std::net::IpAddr>+Send+Sync)>)->Vec<String> {
    #[cfg(test)]
    let inspection=if let Some(resolve)=resolve {
        async {
            let live=model::RenderedConfig{conf:super::files::managed_files(&state.config.nginx_conf_dir,super::files::ManagedKind::Conf).await?,stream:super::files::managed_files(&state.config.nginx_stream_dir,super::files::ManagedKind::Stream).await?};
            let scan=crate::control::guard_targets::scan(&state.config,&live.conf,&live.stream,resolve);
            Ok::<_,anyhow::Error>((live,scan,Vec::<String>::new()))
        }.await
    }else{super::apply::inspect_targets(state.config.clone()).await};
    #[cfg(not(test))]
    let inspection=super::apply::inspect_targets(state.config.clone()).await;
    match inspection {
        Ok((live,scan,_)) if !scan.is_empty()=>vec![format!("인증서 배포 후 대상 재검사 경고: {}. 현재 설정 파일은 그대로 유지했습니다. 다음 일반 설정 적용·재시작에서 다시 검사합니다.",super::apply::target_hit_summary(&live,&scan))],
        Ok(_)=>Vec::new(),
        Err(error)=>vec![format!("인증서 배포 후 대상 재검사를 완료하지 못했습니다. 배포는 완료됐으며 다음 설정 적용에서 다시 확인하세요. ({error:#})")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{apply,files,store::{self,Origin},journal::{Admission,Operation,Phase}};
    use std::{collections::BTreeMap,path::Path,time::{Duration,Instant}};
    use serde_json::json;

    fn actor()->contracts::Actor {contracts::Actor{via: None,id:None,label:Some("certificate-fixture".into()),ip:None}}
    fn pair()->(String,String) {
        let pair=rcgen::generate_simple_self_signed(vec!["example.test".into()]).unwrap();
        (pair.cert.pem(),pair.signing_key.serialize_pem())
    }
    fn operation(store:&PublisherStore)->Operation {
        let Admission::New(operation)=Operation::admit(store,&uuid::Uuid::new_v4().to_string(),&"a".repeat(64)).unwrap() else{panic!()};operation
    }
    fn fake_state(temp:&Path,url:&str)->AppState {
        use super::super::test_helpers::restart_guard;
        let paths=restart_guard::paths(temp);let nginx=restart_guard::Nginx::new(temp);
        let extra=restart_guard::api_config(&nginx,&paths,"1");
        let config=crate::config::RuntimeConfig::read(|key|extra.iter().find(|(name,_)|*name==key).map(|(_,value)|value.clone()).or_else(||match key {
            "DATABASE_URL"=>Some(url.into()),"AUTH_SECRET"=>Some("cert-deploy-test-only".into()),
            "DATA_KEY_FILE"=>Some(temp.join("key").display().to_string()),"NWUI_CONTAINER"=>Some("1".into()),"API_DB_POOL_SIZE"=>Some("1".into()),_=>None,
        })).unwrap();
        std::fs::write(&config.data_key_file,"A5".repeat(32)).unwrap();let mut state=AppState::new(config).unwrap();super::super::test_helpers::nginx::witness(&mut state);
        // The existing double supplies fresh P/C; -T additionally exposes the trusted main.
        let bin=temp.join("nginx");let old=std::fs::read_to_string(&bin).unwrap();
        let prefix=format!("#!/bin/sh\nif [ \"$1\" = -T ]; then printf '# configuration file %s:\\n' '{}'; exit 0; fi\nif [ \"$1\" = -t ]; then printf 'fixture config ok\\n' >&2; fi\n",state.config.nginx_main_conf.display());
        std::fs::write(bin,old.replacen("#!/bin/sh\n",&prefix,1)).unwrap();state
    }
    async fn connect(url:&str)->tokio_postgres::Client {
        let(client,connection)=tokio_postgres::connect(url,tokio_postgres::NoTls).await.unwrap();tokio::spawn(connection);client
    }
    struct Fixture {
        temp:tempfile::TempDir,state:AppState,client:tokio_postgres::Client,store:PublisherStore,
        cert:String,digest:String,old:String,old_files:BTreeMap<String,Vec<u8>>,new_cert:String,new_key:String,
    }
    impl Fixture {
        async fn new(linked:bool)->Self {
            let admin=std::env::var("NWUI_TEST_DATABASE_URL").expect("NWUI_TEST_DATABASE_URL");let name=format!("nwui_cert_{}",uuid::Uuid::new_v4().simple());
            connect(&admin).await.batch_execute(&format!("CREATE DATABASE {name}")).await.unwrap();
            let(base,_)=admin.rsplit_once('/').unwrap();let url=format!("{base}/{name}");
            let temp=tempfile::tempdir().unwrap();let state=fake_state(temp.path(),&url);let client=connect(&url).await;
            crate::migrations::run(&state.db,Path::new(concat!(env!("CARGO_MANIFEST_DIR"),"/../../packages/db/drizzle"))).await.unwrap();
            let store=PublisherStore::open_test_root(&temp.path().join("publisher-state")).unwrap();
            let cert=uuid::Uuid::new_v4().to_string();let(old_cert,old_key)=pair();let(new_cert,new_key)=pair();let sealed=state.secrets.seal(&new_key).unwrap();
            client.execute("INSERT INTO certificates(id,domains,status,source,cert_pem,key_pem) VALUES($1,$2,'issued','manual',$3,$4)",&[&cert,&json!(["example.test"]),&new_cert,&sealed]).await.unwrap();
            let digest=model::certificate_content_digest(&new_cert,&sealed);
            let mut old_files=BTreeMap::new();
            if linked {
                let cert_path=state.config.certs_dir.join(&cert).join("fullchain.pem");let key_path=state.config.certs_dir.join(&cert).join("privkey.pem");
                old_files.insert("conf/server-live.conf".into(),format!("server {{ listen 18081 ssl; server_name example.test; ssl_certificate {}; ssl_certificate_key {}; location / {{ proxy_pass http://192.0.2.44:9000; }} }}\n",cert_path.display(),key_path.display()).into_bytes());
                old_files.insert("stream/stream-live.conf".into(),b"server { listen 18082; proxy_pass 192.0.2.44:9001; }\n".to_vec());
                old_files.insert(format!("certs/{cert}/fullchain.pem"),old_cert.into_bytes());old_files.insert(format!("certs/{cert}/privkey.pem"),old_key.into_bytes());
            }
            let old=uuid::Uuid::new_v4().to_string();store.stage(&old,Origin::Publisher,Some(store::digest(b"saved fixture")),BTreeMap::new(),old_files.clone()).unwrap();store.select(&old).unwrap();
            files::restore_payload_files(&state.config,&old_files).await.unwrap();
            Self{temp,state,client,store,cert,digest,old,old_files,new_cert,new_key}
        }
        async fn run(&self,digest:&str)->(anyhow::Result<model::ApplyResult>,Operation) {
            let _store=self.store.lock(Duration::from_secs(3)).await.unwrap();let _apply=self.state.runtime.apply_lock.lock().await;
            let mut op=operation(&self.store);let result=deploy(&self.state,&self.store,&mut op,&self.cert,digest,&actor(),&confirmation::Budget::new()).await;(result,op)
        }
        async fn audit(&self)->Vec<tokio_postgres::Row> {self.client.query("SELECT actor_label,\"after\" FROM audit_logs WHERE action='nginx:apply' AND target=$1",&[&self.cert]).await.unwrap()}
        fn reloads(&self)->usize {std::fs::read_to_string(self.temp.path().join("calls")).unwrap_or_default().lines().filter(|line|line.ends_with("start -s reload")).count()}
        async fn unchanged(&self) {assert!(files::capture_payload_files(&self.state.config).await.unwrap()==self.old_files,"live bytes changed");}
        async fn restart_preserves_result(&self,op:&Operation) {
            let result=Operation::load(&self.store,&op.id).unwrap().result;
            let _store=self.store.lock(Duration::from_secs(3)).await.unwrap();
            super::super::recovery::before_start(&self.state.config,&self.store).await.unwrap();
            let after=Operation::load(&self.store,&op.id).unwrap();
            assert_eq!(after.result,result,"restart must preserve the completed certificate outcome");
            assert_eq!(after.phase,Phase::Done);
        }

    }
    #[tokio::test]
    #[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
    async fn durable_deploy_preserves_live_only_routes_drafts_and_audit_parity() {
        let f=Fixture::new(true).await;
        f.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES('draft','draft.example.test',18090,$1)",&[&json!({"proxyPass":"http://192.0.2.88:9000"})]).await.unwrap();
        f.client.execute("INSERT INTO applied_config(id,data) VALUES('current',$1)",&[&json!({"fixture":"unchanged marker"})]).await.unwrap();
        let(result,op)=f.run(&f.digest).await;let result=result.unwrap();assert!(result.ok);assert_eq!(result.output,"fixture config ok\n");assert!(result.rolled_back_to.is_none());
        let live=files::capture_payload_files(&f.state.config).await.unwrap();
        for(name,bytes)in &f.old_files{if name.starts_with("conf/")||name.starts_with("stream/"){assert!(live.get(name)==Some(bytes),"config changed: {name}");}}
        assert!(live[&format!("certs/{}/fullchain.pem",f.cert)]==f.new_cert.as_bytes());assert!(live[&format!("certs/{}/privkey.pem",f.cert)]==f.new_key.as_bytes());
        assert!(!op.previous_uncertain,"confirmed baseline must be stamped before writes");assert_eq!(op.phase,Phase::Done);assert_eq!(op.previous.as_deref(),Some(f.old.as_str()));assert_eq!(f.store.manifest().unwrap().current,op.next);assert_eq!(f.reloads(),2);assert!(!confirmation::uncertain(&f.state.config).await);
        assert_eq!(f.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get::<_,serde_json::Value>(0),json!({"fixture":"unchanged marker"}));
        assert_eq!(f.client.query_one("SELECT count(*) FROM config_snapshots",&[]).await.unwrap().get::<_,i64>(0),0);
        assert_eq!(f.client.query_one("SELECT server_name FROM nginx_servers WHERE id='draft'",&[]).await.unwrap().get::<_,String>(0),"draft.example.test");
        let audit=f.audit().await;assert_eq!(audit.len(),1);assert_eq!(audit[0].get::<_,String>(0),"certificate-fixture");assert_eq!(audit[0].get::<_,serde_json::Value>(1),json!({"source":"certificate","certificateId":f.cert,"ok":true,"output":result.output}));
    }
    #[tokio::test]
    #[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
    async fn certificate_only_success_does_not_settle_prior_config_uncertainty() {
        for linked in [true,false] {
            let f=Fixture::new(linked).await;confirmation::begin_uncertain(&f.state.config).await.unwrap();
            let(result,op)=f.run(&f.digest).await;assert!(result.unwrap().ok);
            assert!(confirmation::uncertain(&f.state.config).await,"certificate deploy cannot settle a prior config DB commit");
            assert!(op.previous_uncertain);assert_eq!(op.result.as_ref().unwrap()["uncertain"],true);
            if linked {assert!(op.uncertain);assert_eq!(op.phase,Phase::Done);}
        }
    }
    #[tokio::test]
    #[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
    async fn stale_material_is_rejected_without_live_changes() {
        let f=Fixture::new(true).await;let(result,op)=f.run(&"0".repeat(64)).await;assert_eq!(result.unwrap_err().to_string(),"stale_material");f.unchanged().await;
        assert!(!Operation::load(&f.store,&op.id).unwrap().previous_uncertain,"Intent must carry the observed A8 even on an early refusal");assert!(op.next.is_none());assert_eq!(f.reloads(),0);assert!(!confirmation::uncertain(&f.state.config).await);
        f.restart_preserves_result(&op).await;
    }
    #[tokio::test]
    #[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
    async fn no_live_consumer_only_emits_the_existing_activation_notice() {
        let f=Fixture::new(false).await;f.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES('draft','example.test',18090,$1,$2)",&[&f.cert,&json!({"proxyPass":"http://192.0.2.44:9000"})]).await.unwrap();
        let(result,op)=f.run(&f.digest).await;assert!(result.unwrap().ok);f.unchanged().await;assert!(op.next.is_none());assert_eq!(f.reloads(),0);assert!(!confirmation::uncertain(&f.state.config).await);
        assert_eq!(f.client.query_one("SELECT count(*) FROM notifications WHERE type='config.apply-needed'",&[]).await.unwrap().get::<_,i64>(0),1);
        f.restart_preserves_result(&op).await;
    }
    #[tokio::test]
    #[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
    async fn vocabulary_refusal_is_false_and_audited_without_touching_live_bytes() {
        let mut f=Fixture::new(true).await;let bytes=f.old_files.get_mut("conf/server-live.conf").unwrap();bytes.extend_from_slice(b"include /fixture/not-generated;\n");
        files::restore_payload_files(&f.state.config,&f.old_files).await.unwrap();
        f.client.execute("INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES('unapplied','draft.test',18090,$1,$2)",&[&f.cert,&json!({"proxyPass":"http://192.0.2.44:9000"})]).await.unwrap();
        let(result,op)=f.run(&f.digest).await;let result=result.unwrap();assert!(!result.ok);assert!(result.output.contains("nginx vocabulary"));f.unchanged().await;assert!(op.next.is_none());assert_eq!(f.reloads(),0);
        let audit=f.audit().await;assert_eq!(audit.len(),1);assert_eq!(audit[0].get::<_,serde_json::Value>(1)["ok"],false);
        assert_eq!(f.client.query_one("SELECT count(*) FROM notifications WHERE type='config.apply-needed'",&[]).await.unwrap().get::<_,i64>(0),0,"rejected live vocabulary must not commit the rolled-back activation notice");
        f.restart_preserves_result(&op).await;
    }
    #[tokio::test]
    #[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
    async fn foreign_pid_refuses_apply_without_writing_or_fencing() {
        use super::super::{dispatcher::Dispatcher, protocol::{Actor, Op, OpResult, Request}};
        use std::{os::unix::process::CommandExt, sync::atomic::Ordering};
        let f = Fixture::new(true).await;
        let foreign = std::process::Command::new("/bin/sleep").arg("30").process_group(0).spawn().unwrap();
        let foreign = super::super::nginx_start_tests::Process(foreign);
        std::fs::write(f.temp.path().join(".fake-nginx.pid"), foreign.0.id().to_string()).unwrap();
        let before = crate::db_upgrade::files::snapshot(&f.store.root.join("payloads")).unwrap();
        let dispatcher = Dispatcher::new(f.state.config.as_ref().clone(), f.store.clone());
        dispatcher.ready.store(true, Ordering::Release);
        let id = uuid::Uuid::new_v4().to_string();
        let request = Request {
            v: 1, id: id.clone(), deadline_ms: 5000,
            op: Op::ApplySaved { label: None, expected_saved_digest: None },
            actor: Actor { via: None, user_id: None, label: Some("apply-fixture".into()), ip: None },
        };
        let state = f.state.clone();
        let store = f.store.clone();
        let result = dispatcher.dispatch(request, move |request, op, budget| async move {
            super::super::coordinator::execute(&state, &store, request, op, budget).await
        }).await;
        assert!(matches!(result, OpResult::Ok { apply: Some(result), uncertain: false, .. }
            if !result.ok && result.output == "nginx 프로세스를 확인하지 못해 설정을 적용하지 않았습니다. 잠시 후 다시 시도하세요."));
        let op = Operation::load(&f.store, &id).unwrap();
        assert!(op.terminal() && op.next.is_none());
        assert!(dispatcher.ready.load(Ordering::Acquire));
        assert_eq!(f.store.manifest().unwrap().current.as_deref(), Some(f.old.as_str()));
        assert_eq!(crate::db_upgrade::files::snapshot(&f.store.root.join("payloads")).unwrap(), before);
        assert_eq!(f.reloads(), 0);
        f.unchanged().await;
        assert_eq!(unsafe { libc::kill(foreign.0.id() as i32, 0) }, 0);
    }

    #[tokio::test]
    #[ignore = "requires disposable NWUI_TEST_DATABASE_URL"]
    async fn uncertain_nginx_refuses_before_staging_or_certificate_activation() {
        let f = Fixture::new(true).await;
        f.client.execute(
            "INSERT INTO nginx_servers(id,server_name,listen,certificate_id,config) VALUES('draft','draft.test',18090,$1,$2)",
            &[&f.cert, &json!({"proxyPass":"http://192.0.2.44:9000"})],
        ).await.unwrap();
        let foreign = super::super::identity_fixture::FakeMaster::spawn(&f.temp.path().join("foreign.pid"), true);
        std::fs::write(f.temp.path().join(".fake-nginx.pid"), foreign.pid().to_string()).unwrap();

        // With no selected previous payload, ensure_previous would create and select one.
        let mut manifest = f.store.manifest().unwrap();
        manifest.current = None;
        crate::db_upgrade::files::atomic_json(&f.store.root.join("manifest.json"), &manifest).unwrap();
        let manifest = std::fs::read(f.store.root.join("manifest.json")).unwrap();
        let payloads = crate::db_upgrade::files::snapshot(&f.store.root.join("payloads")).unwrap();
        let (result, op) = f.run(&f.digest).await;
        let result = result.unwrap();
        assert!(!result.ok);
        assert_eq!(result.output, "nginx 프로세스를 확인하지 못해 인증서를 적용하지 않았습니다. 잠시 후 다시 시도하세요.");
        assert_eq!(op.phase, Phase::Intent);
        assert!(op.next.is_none());
        f.unchanged().await;
        assert_eq!(std::fs::read(f.store.root.join("manifest.json")).unwrap(), manifest);
        assert_eq!(crate::db_upgrade::files::snapshot(&f.store.root.join("payloads")).unwrap(), payloads);
        assert_eq!(f.reloads(), 0);
        assert!(!confirmation::uncertain(&f.state.config).await);
        let audit = f.audit().await;
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].get::<_, serde_json::Value>(1)["ok"], false);
        assert_eq!(f.client.query_one("SELECT count(*) FROM notifications WHERE type='config.apply-needed'", &[]).await.unwrap().get::<_, i64>(0), 0);
    }

    #[tokio::test]
    #[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
    async fn report_only_target_scan_keeps_the_config_and_records_the_warning() {
        let mut f=Fixture::new(true).await;
        let bytes=f.old_files.get_mut("conf/server-live.conf").unwrap();*bytes=String::from_utf8(bytes.clone()).unwrap().replace("192.0.2.44:9000","100.100.100.200:9000").into_bytes();
        let generation=uuid::Uuid::new_v4().to_string();f.store.stage(&generation,Origin::Publisher,None,BTreeMap::new(),f.old_files.clone()).unwrap();f.store.select(&generation).unwrap();f.old=generation;
        files::restore_payload_files(&f.state.config,&f.old_files).await.unwrap();
        let(result,_)=f.run(&f.digest).await;let result=result.unwrap();assert!(result.ok);assert!(result.output.contains("대상 재검사 경고"));assert!(result.output.contains("server-live.conf"));assert_eq!(f.reloads(),2);
        assert!(std::fs::read(f.state.config.nginx_conf_dir.join("server-live.conf")).unwrap()==f.old_files["conf/server-live.conf"]);
        let audit=f.audit().await;assert_eq!(audit.len(),1);assert!(audit[0].get::<_,serde_json::Value>(1)["output"].as_str().unwrap().contains("100.100.100.200"));
        assert_eq!(f.client.query_one("SELECT count(*) FROM notifications WHERE type='config.apply-notice'",&[]).await.unwrap().get::<_,i64>(0),1);
    }
    #[tokio::test]
    async fn restore_must_not_clear_uncertainty_inherited_from_an_older_unknown_commit() {
        let temp=tempfile::tempdir().unwrap();let state=fake_state(temp.path(),"postgres://unused@127.0.0.1:1/unused");
        let store=PublisherStore::open_test_root(&temp.path().join("publisher-state")).unwrap();let old=uuid::Uuid::new_v4().to_string();let next=uuid::Uuid::new_v4().to_string();
        for id in [&old,&next]{store.stage(id,Origin::Publisher,None,BTreeMap::new(),BTreeMap::new()).unwrap();}store.select(&old).unwrap();
        let mut prior=operation(&store);prior.previous=Some(old.clone());prior.next=Some(next.clone());prior.restore_target=Some(old.clone());prior.phase=Phase::Done;prior.commit_unknown=true;prior.uncertain=true;prior.save(&store).unwrap();
        let mut op=operation(&store);op.previous_uncertain=true;op.previous=Some(old.clone());op.next=Some(next);op.phase=Phase::P;op.uncertain=true;op.save(&store).unwrap();op.begin_restore(&store,&old,"certificate deploy failed").unwrap();confirmation::begin_uncertain(&state.config).await.unwrap();
        let _store=store.lock(Duration::from_secs(3)).await.unwrap();let _files=super::super::nginx_lock::acquire(&state.config).await.unwrap();
        super::super::recovery::finish_restore(&state.config,&store,&mut op,Instant::now()+confirmation::RESTORE_RESERVE).await.unwrap();
        assert!(confirmation::uncertain(&state.config).await,"certificate-only restore cannot settle the earlier config DB commit");
    }
    #[tokio::test]
    #[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
    async fn frozen_validation_failure_restores_all_material_without_containment() {
        let f=Fixture::new(true).await;let old_pem=f.temp.path().join("previous.pem");std::fs::write(&old_pem,&f.old_files[&format!("certs/{}/fullchain.pem",f.cert)]).unwrap();
        let bin=f.temp.path().join("nginx");let script=std::fs::read_to_string(&bin).unwrap();let live=f.state.config.certs_dir.join(&f.cert).join("fullchain.pem");
        let guard=format!("#!/bin/sh\nif [ \"$1\" = -t ] && ! cmp -s '{}' '{}'; then echo bad-material >&2; exit 1; fi\n",live.display(),old_pem.display());std::fs::write(bin,script.replacen("#!/bin/sh\n",&guard,1)).unwrap();
        f.client.execute("INSERT INTO certificate_jobs(id,certificate_id,kind,mode,state,payload_enc,actor,deploy_pending) VALUES('deploy',$1,'http','renew','cleanup','','{}',true)",&[&f.cert]).await.unwrap();
        let(result,op)=f.run(&f.digest).await;assert!(result.is_err());f.unchanged().await;assert_eq!(op.phase,Phase::Done);assert_eq!(op.restore_target.as_deref(),Some(f.old.as_str()));assert_eq!(f.reloads(),2);assert!(!confirmation::uncertain(&f.state.config).await);assert!(!f.temp.path().join("quarantine").exists());
        assert!(f.client.query_one("SELECT deploy_pending FROM certificate_jobs WHERE id='deploy'",&[]).await.unwrap().get::<_,bool>(0));
        let saved=Operation::load(&f.store,&op.id).unwrap();let result:super::super::protocol::OpResult=serde_json::from_value(saved.result.unwrap()).unwrap();
        assert!(matches!(result,super::super::protocol::OpResult::Failed{output,..} if output.contains("bad-material")),"restore must never replay the intended success");
    }
    #[tokio::test]
    #[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
    async fn a8_creation_failure_prevents_the_first_live_material_write() {
        let f=Fixture::new(true).await;std::fs::create_dir(f.state.config.nginx_conf_dir.join(confirmation::UNCERTAIN)).unwrap();
        let(result,_)=f.run(&f.digest).await;assert!(result.is_err());f.unchanged().await;assert_eq!(f.reloads(),0);
    }
    #[tokio::test]
    #[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
    async fn every_live_certificate_is_validated_before_any_write() {
        let mut f=Fixture::new(true).await;let other=uuid::Uuid::new_v4().to_string();
        let pair=pair();f.client.execute("INSERT INTO certificates(id,domains,status,source,cert_pem,key_pem) VALUES($1,$2,'failed','manual',$3,$4)",&[&other,&json!(["other.test"]),&pair.0,&pair.1]).await.unwrap();
        let other_conf=String::from_utf8(f.old_files["conf/server-live.conf"].clone()).unwrap().replace(&f.cert,&other).replace("18081","18083");f.old_files.insert("conf/server-other.conf".into(),other_conf.into_bytes());
        f.old_files.insert(format!("certs/{other}/fullchain.pem"),pair.0.into_bytes());f.old_files.insert(format!("certs/{other}/privkey.pem"),pair.1.into_bytes());
        let generation=uuid::Uuid::new_v4().to_string();f.store.stage(&generation,Origin::Publisher,None,BTreeMap::new(),f.old_files.clone()).unwrap();f.store.select(&generation).unwrap();f.old=generation;files::restore_payload_files(&f.state.config,&f.old_files).await.unwrap();
        let(result,_)=f.run(&f.digest).await;assert!(result.is_err());f.unchanged().await;assert_eq!(f.reloads(),0);
    }
    #[tokio::test]
    async fn target_diagnostic_io_failure_is_a_notice_not_a_deploy_error() {
        let temp=tempfile::tempdir().unwrap();let mut state=fake_state(temp.path(),"postgres://unused@127.0.0.1:1/unused");
        let path=temp.path().join("not-a-directory");std::fs::write(&path,b"keep").unwrap();std::sync::Arc::get_mut(&mut state.config).unwrap().nginx_conf_dir=path.clone();
        let notices=target_notices(&state).await;assert_eq!(notices.len(),1);assert!(notices[0].contains("재검사를 완료하지 못했습니다"));assert_eq!(std::fs::read(path).unwrap(),b"keep");
    }
    #[tokio::test]
    async fn persisted_forward_and_restore_crash_images_never_adopt_mixed_material() {
        let phases=[Phase::Intent,Phase::PayloadStaged,Phase::LiveWriting,Phase::Validated,Phase::P,Phase::C,Phase::CommitUnknown,Phase::DbCommitted,Phase::RestoreIntent,Phase::RestoreWriting,Phase::RestoreValidated,Phase::R1,Phase::R2,Phase::Restored];
        for phase in phases {
            let(_temp,config,store,old,sample)=super::super::recovery::tests::durable_fixture();
            let mut next_files=store.payload(&old).unwrap().files;
            for(name,bytes)in store.payload(&sample).unwrap().files{if name.starts_with("certs/"){next_files.insert(name,bytes);}}
            let next=uuid::Uuid::new_v4().to_string();store.stage(&next,Origin::Publisher,None,BTreeMap::new(),next_files).unwrap();
            let mut op=super::super::recovery::tests::unfinished(&store,&old,&next,phase);
            if phase==Phase::CommitUnknown {op.commit_unknown=true;op.save(&store).unwrap();}
            let restore=matches!(phase,Phase::RestoreIntent|Phase::RestoreWriting|Phase::RestoreValidated|Phase::R1|Phase::R2|Phase::Restored);
            if restore {op.restore_target=Some(old.clone());op.restore_reason=Some("interrupted certificate deployment".into());op.save(&store).unwrap();}
            let mut live=store.payload(if matches!(phase,Phase::Intent|Phase::PayloadStaged){&old}else{&next}).unwrap().files;
            if matches!(phase,Phase::LiveWriting|Phase::RestoreWriting){let name=live.keys().find(|name|name.ends_with("privkey.pem")).unwrap().clone();live.insert(name.clone(),store.payload(&old).unwrap().files[&name].clone());}
            if !matches!(phase,Phase::Intent|Phase::PayloadStaged){confirmation::begin_uncertain(&config).await.unwrap();}
            files::restore_payload_files(&config,&live).await.unwrap();super::super::recovery::before_start(&config,&store).await.unwrap_or_else(|error|panic!("first recovery {phase:?}: {error:#}"));
            let expected=if matches!(phase,Phase::C|Phase::CommitUnknown|Phase::DbCommitted){&next}else{&old};
            assert!(files::capture_payload_files(&config).await.unwrap()==store.payload(expected).unwrap().files,"mixed payload at {phase:?}");assert_eq!(store.manifest().unwrap().current.as_ref(),Some(expected));
            super::super::recovery::before_start(&config,&store).await.unwrap_or_else(|error|panic!("second recovery {phase:?}: {error:#}"));
            assert_eq!(store.manifest().unwrap().current.as_ref(),Some(expected),"second recovery must not select a discarded staged payload: {phase:?}");
            assert!(files::capture_payload_files(&config).await.unwrap()==store.payload(expected).unwrap().files);
            if !matches!(phase,Phase::Intent|Phase::PayloadStaged){assert!(confirmation::uncertain(&config).await,"a fresh running-nginx acknowledgement has not happened");}
        }
    }

    #[tokio::test]
    #[ignore="requires disposable NWUI_TEST_DATABASE_URL"]
    async fn a29_real_pg_apply_rollback_boot_and_certificate_refuse_before_live_and_retry_after_space_freed() {
        use std::sync::Arc;
        use super::super::capacity::{self,Available};
        let mut f=Fixture::new(true).await;
        let server=uuid::Uuid::new_v4().to_string();
        f.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES($1,'capacity.example.test',18081,$2)",&[&server,&json!({"proxyPass":"http://127.0.0.1:19000"})]).await.unwrap();
        let saved={let client=f.state.db.get().await.unwrap();model::load_snapshot(&client).await.unwrap()};
        let snapshot=uuid::Uuid::new_v4().to_string();let data=serde_json::to_value(&saved).unwrap();
        f.client.execute("INSERT INTO config_snapshots(id,label,data) VALUES($1,'capacity snapshot',$2)",&[&snapshot,&data]).await.unwrap();
        let before=std::fs::read(f.store.root.join("manifest.json")).unwrap();let count=std::fs::read_dir(f.store.root.join("payloads")).unwrap().count();
        let a8=std::fs::read(f.state.config.nginx_conf_dir.join(".nwui-apply-uncertain")).ok();
        f.store.capacity_probe=Some(Arc::new(||Ok(Available{block_size:4096,bytes:0,inodes:Some(0)})));
        for command in [super::super::protocol::Op::ApplySaved{label:None,expected_saved_digest:None},super::super::protocol::Op::RollbackSnapshot{snapshot_id:snapshot.clone()}] {
            let _store=f.store.lock(Duration::from_secs(3)).await.unwrap();let _apply=f.state.runtime.apply_lock.lock().await;
            let op=operation(&f.store);let id=op.id.clone();
            let result=apply::mutate(&f.state,&f.store,op,command,&actor(),&confirmation::Budget::new()).await;
            assert!(matches!(result,super::super::protocol::OpResult::Failed{ref code,ref output}if code=="publisher_storage_full"&&output==capacity::OUTPUT));
            let closed=Operation::load(&f.store,&id).unwrap();assert_eq!(closed.phase,Phase::Done);assert_eq!(closed.result.unwrap()["code"],"publisher_storage_full");
            assert_eq!(f.client.query_one("SELECT count(*) FROM audit_logs WHERE id=$1",&[&id]).await.unwrap().get::<_,i64>(0),1);
            f.unchanged().await;
        }
        let admitted=crate::nonroot::Admitted{master_bin:f.state.config.nginx_bin.clone(),control_bin:f.state.config.nginx_bin.clone(),conf_path:f.state.config.nginx_main_conf.clone(),conf_mismatch:false};
        let error=super::super::prepare::prepare(f.state.config.as_ref().clone(),&f.state.secrets,f.state.db.clone(),f.store.clone(),&admitted,super::super::boot::BootMode::Conversion).await.unwrap_err();assert_eq!(error.code,"publisher_storage_full");
        let(result,op)=f.run(&f.digest).await;assert!(capacity::is_full(&result.err().expect("certificate refusal")));assert_eq!(op.phase,Phase::Done);assert_eq!(op.result.as_ref().unwrap()["code"],"publisher_storage_full");
        f.restart_preserves_result(&op).await;f.unchanged().await;assert_eq!(f.reloads(),0);assert_eq!(f.audit().await.len(),1);
        assert_eq!(std::fs::read(f.store.root.join("manifest.json")).unwrap(),before);assert_eq!(std::fs::read_dir(f.store.root.join("payloads")).unwrap().count(),count);
        assert_eq!(std::fs::read(f.state.config.nginx_conf_dir.join(".nwui-apply-uncertain")).ok(),a8);
        assert_eq!(f.client.query_one("SELECT count(*) FROM applied_config",&[]).await.unwrap().get::<_,i64>(0),0);
        assert_eq!(f.client.query_one("SELECT count(*) FROM config_snapshots",&[]).await.unwrap().get::<_,i64>(0),1);
        let after={let client=f.state.db.get().await.unwrap();model::load_snapshot(&client).await.unwrap()};assert_eq!(serde_json::to_value(after).unwrap(),data);
        f.store.capacity_probe=Some(Arc::new(||Ok(Available{block_size:4096,bytes:u64::MAX,inodes:None})));
        let _store=f.store.lock(Duration::from_secs(3)).await.unwrap();let _apply=f.state.runtime.apply_lock.lock().await;
        let result=apply::mutate(&f.state,&f.store,operation(&f.store),super::super::protocol::Op::ApplySaved{label:None,expected_saved_digest:None},&actor(),&confirmation::Budget::new()).await;
        assert!(matches!(result,super::super::protocol::OpResult::Ok{apply:Some(ref result),..}if result.ok));
        assert!(f.state.config.nginx_conf_dir.join(format!("server-{server}.conf")).exists());assert!(!confirmation::uncertain(&f.state.config).await);
    }

    #[path="g13_cert_deploy_tests.rs"] mod g13_dns;

}
