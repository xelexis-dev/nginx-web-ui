//! Authenticated apply/history commands and read-only history presentation.
use super::{model::{ApplyResult,RollbackKind},pending};
use crate::{
    auth::{
        principal::{
            SessionPrincipal, TwoFactorGate, db_unavailable, require_gateway, require_session,
        },
        rbac::{Action, can},
    },
    contracts::FormResult,
    error::ApiError,
    http_policy::{check_browser_origin, read_form},
    request_context::RequestContext,
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, Request, State},
    http::StatusCode,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const FORM_IDS: &[&str] = &["apply.applyConfig", "history.rollback"];
pub fn router() -> Router<AppState> {
    let mut router = Router::new()
        .route("/api/v1/status", get(node_status))
        .route("/api/v1/pending", get(node_pending))
        .route("/api/v1/apply", post(node_apply))
        .route("/api/v1/history", get(node_history))
        .route("/api/v1/history/{id}", get(node_detail))
        .route("/api/v1/history/{id}/rollback", post(node_rollback));
    for command in FORM_IDS {
        router = router.route(&format!("/api/ui/v1/forms/{command}"), post(form));
    }
    router
}
fn internal(error: anyhow::Error) -> ApiError {
    eprintln!("[nginx-operation] {error:#}");
    ApiError::internal()
}
fn publisher_error(error: anyhow::Error) -> ApiError {
    use crate::publisher_client::ClientError;
    match error.downcast_ref::<ClientError>() {
        Some(ClientError::Unavailable { .. }) => ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "publisher_unavailable", "설정 적용 서비스가 아직 준비되지 않았습니다. 잠시 후 다시 시도하세요."),
        Some(ClientError::Unconfirmed { .. }) => ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "apply_unconfirmed", "설정 적용 결과를 아직 확인하지 못했습니다. 적용 기록에서 결과를 확인한 뒤 필요하면 다시 시도하세요."),
        _ => internal(error),
    }
}
async fn node_status(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    require_gateway(&state, request.headers(), &ctx, &[Action::ConfigRead]).await?;
    let counts = {
        let client = state.db.get().await.map_err(|_| db_unavailable())?;
        let cutoff = chrono::Utc::now() + chrono::Duration::days(30);
        let row = client.query_one("SELECT (SELECT count(*) FROM nginx_servers), (SELECT count(*) FROM nginx_streams), (SELECT count(*) FROM upstreams), (SELECT count(*) FROM certificates), (SELECT count(*) FROM certificates WHERE expires_at <= $1)", &[&cutoff]).await.map_err(|_|db_unavailable())?;
        (
            row.get::<_, i64>(0),
            row.get::<_, i64>(1),
            row.get::<_, i64>(2),
            row.get::<_, i64>(3),
            row.get::<_, i64>(4),
        )
    };
    // Release the pool connection before pending obtains one (pool size may be one).
    let pending = pending::status(&state).await?;
    let nginx = match crate::publisher_client::validate_live(&state).await {
        Ok(crate::publisher_client::LiveValidation::Checked { ok, output }) => json!({"configOk":ok,"output":output}),
        Ok(crate::publisher_client::LiveValidation::Busy) => return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "nginx_busy", "설정 적용 중이거나 재시작 점검 중입니다. 잠시 후 새로고침하세요.")),
        Err(_) => return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "publisher_unavailable", "설정 적용 서비스가 아직 준비되지 않았습니다. 잠시 후 다시 시도하세요.")),
    };
    Ok(Json(
        json!({"semver":state.config.version,"counts":{"servers":counts.0,"streams":counts.1,"upstreams":counts.2},"certificates":{"total":counts.3,"expiringSoon":counts.4},"pending":{"count":pending.count,"hasBaseline":pending.has_baseline},"nginx":nginx}),
    ))
}
fn applied_response(result: ApplyResult) -> Result<Json<Value>, ApiError> {
    if !result.ok {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "apply_failed",
            result.output,
        ));
    }
    Ok(Json(json!({"ok":true,"output":result.output})))
}
async fn node_pending(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<pending::PendingStatus>, ApiError> {
    require_gateway(&state, request.headers(), &ctx, &[Action::ConfigRead]).await?;
    Ok(Json(pending::status(&state).await?))
}
async fn node_apply(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let caller = require_gateway(&state, request.headers(), &ctx, &[Action::NginxReload]).await?;
    let actor = caller.actor(&ctx);
    let label = format!("apply via {}", actor.label.as_deref().unwrap_or("null"));
    applied_response(
        crate::publisher_client::apply_saved(&state, &actor, Some(&label))
            .await
            .map_err(publisher_error)?,
    )
}
fn iso(time: chrono::DateTime<chrono::Utc>) -> String {
    time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
pub async fn history_list(state: &AppState) -> Result<Vec<Value>, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let rows=client.query("SELECT id,created_at,label,actor_id FROM config_snapshots ORDER BY created_at DESC,id DESC LIMIT 100",&[]).await.map_err(|_|db_unavailable())?;
    Ok(rows.into_iter().map(|r|json!({"id":r.get::<_,String>(0),"createdAt":iso(r.get(1)),"label":r.get::<_,Option<String>>(2),"actorId":r.get::<_,Option<String>>(3)})).collect())
}
async fn node_history(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    require_gateway(&state, request.headers(), &ctx, &[Action::ConfigRead]).await?;
    Ok(Json(json!({"snapshots":history_list(&state).await?})))
}
pub async fn history_detail(state: &AppState, id: &str) -> Result<Value, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let row = client
        .query_opt(
            "SELECT id,created_at,label,actor_id,data FROM config_snapshots WHERE id=$1",
            &[&id],
        )
        .await
        .map_err(|_| db_unavailable())?
        .ok_or_else(ApiError::not_found)?;
    Ok(
        json!({"id":row.get::<_,String>(0),"createdAt":iso(row.get(1)),"label":row.get::<_,Option<String>>(2),"actorId":row.get::<_,Option<String>>(3),"data":row.get::<_,Value>(4)}),
    )
}
async fn node_detail(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<String>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    require_gateway(&state, request.headers(), &ctx, &[Action::ConfigRead]).await?;
    Ok(Json(json!({"snapshot":history_detail(&state,&id).await?})))
}
async fn node_rollback(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<String>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let caller = require_gateway(
        &state,
        request.headers(),
        &ctx,
        &[Action::ConfigWrite, Action::NginxReload],
    )
    .await?;
    let result = crate::publisher_client::rollback_snapshot(&state, &id, &caller.actor(&ctx))
        .await
        .map_err(publisher_error)?;
    if matches!(result.kind, Some(RollbackKind::NotFound)) {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            result.result.output,
        ));
    }
    applied_response(result.result)
}
async fn form(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<FormResult>, ApiError> {
    check_browser_origin(request.headers(), &ctx)?;
    let command = request
        .uri()
        .path()
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_owned();
    let session = require_session(
        &state,
        request.headers(),
        &ctx,
        Some(Action::NginxReload),
        TwoFactorGate::Enforce,
    )
    .await?;
    if command == "history.rollback" && !can(session.role, Action::ConfigWrite) {
        return Err(ApiError::forbidden());
    }
    let input = read_form(request).await?;
    let invalid = || {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "잘못된 폼 인자입니다.",
        )
    };
    let result = match command.as_str() {
        "apply.applyConfig" => {
            if !input.args.is_empty() {
                return Err(invalid());
            }
            json!(
                crate::publisher_client::apply_saved(&state, &session.actor(&ctx), None)
                    .await
                    .map_err(publisher_error)?
            )
        }
        "history.rollback" => {
            if input.args.len() != 1 || input.args[0].is_empty() {
                return Err(invalid());
            }
            json!(
                crate::publisher_client::rollback_snapshot(&state, &input.args[0], &session.actor(&ctx))
                    .await
                    .map_err(publisher_error)?
            )
        }
        _ => return Err(ApiError::not_found()),
    };
    Ok(Json(FormResult {
        result: Some(result),
        refresh: Some(true),
        ..Default::default()
    }))
}

pub async fn view(
    state: &AppState,
    session: &SessionPrincipal,
    path: &str,
    _query: &BTreeMap<String, String>,
) -> Result<Option<Value>, ApiError> {
    if path != "/history" && !path.starts_with("/history/") {
        return Ok(None);
    }
    if !can(session.role, Action::ConfigRead) {
        return Err(ApiError::forbidden());
    }
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    if path == "/history" {
        let rows=client.query("SELECT s.id,s.created_at,s.label,s.data,u.email FROM config_snapshots s LEFT JOIN users u ON u.id=s.actor_id ORDER BY s.created_at DESC,s.id DESC LIMIT 50",&[]).await.map_err(|_|db_unavailable())?;
        let items:Vec<Value>=rows.into_iter().map(|r| {
            let data=pending::snapshot_from_value(&r.get::<_,Value>(3));
            json!({"id":r.get::<_,String>(0),"iso":iso(r.get(1)),"label":r.get::<_,Option<String>>(2),"actorEmail":r.get::<_,Option<String>>(4),"upstreamCount":data.upstreams.len(),"serverCount":data.servers.len(),"streamCount":data.streams.len()})
        }).collect();
        return Ok(Some(json!({"data":{"items":items,"showDetails":true}})));
    }
    let id = path.strip_prefix("/history/").unwrap();
    if id.is_empty() || id.contains('/') {
        return Ok(None);
    }
    let tx = client
        .build_transaction()
        .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .map_err(|_| db_unavailable())?;
    let row=tx.query_opt("SELECT s.id,s.created_at,s.label,s.data,u.email,s.actor_id FROM config_snapshots s LEFT JOIN users u ON u.id=s.actor_id WHERE s.id=$1",&[&id]).await.map_err(|_|db_unavailable())?.ok_or_else(ApiError::not_found)?;
    let created: chrono::DateTime<chrono::Utc> = row.get(1);
    let previous=tx.query_opt("SELECT data FROM config_snapshots WHERE created_at<$1 ORDER BY created_at DESC,id DESC LIMIT 1",&[&created]).await.map_err(|_|db_unavailable())?;
    let before = previous.map(|r| pending::snapshot_from_value(&r.get::<_, Value>(0)));
    let data = pending::snapshot_from_value(&row.get::<_, Value>(3));
    let diff = pending::diff_config(before.as_ref(), &data);
    let iso = iso(created);
    let snapshot = json!({"id":row.get::<_,String>(0),"createdAt":iso,"label":row.get::<_,Option<String>>(2),"data":row.get::<_,Value>(3),"actorEmail":row.get::<_,Option<String>>(4),"actorId":row.get::<_,Option<String>>(5)});
    tx.commit().await.map_err(|_| db_unavailable())?;
    Ok(Some(
        json!({"snapshot":snapshot,"upstreams":data.upstreams,"servers":data.servers,"streams":data.streams,"diff":diff,"isFirst":before.is_none(),"canRollback":can(session.role,Action::ConfigWrite)&&can(session.role,Action::NginxReload),"iso":iso}),
    ))
}

#[cfg(test)]
pub(crate) mod publisher_caller_tests {
    use super::*;
    use crate::publisher_client::ClientError;

    #[test]
    fn publisher_unavailable_is_a_retryable_503() {
        let result=publisher_error(anyhow::Error::new(ClientError::Unavailable {detail:"fixture connection refused".into()}));
        assert_eq!(result.status,StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(result.code,"publisher_unavailable");
        assert_eq!(result.message,"설정 적용 서비스가 아직 준비되지 않았습니다. 잠시 후 다시 시도하세요.");
    }
    #[test]
    fn unconfirmed_apply_is_distinct_from_unavailable() {
        let result=publisher_error(anyhow::Error::new(ClientError::Unconfirmed {operation_id:uuid::Uuid::new_v4().to_string()}));
        assert_eq!(result.status,StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(result.code,"apply_unconfirmed");
        assert_eq!(result.message,"설정 적용 결과를 아직 확인하지 못했습니다. 적용 기록에서 결과를 확인한 뒤 필요하면 다시 시도하세요.");
    }
    #[test]
    fn protocol_and_writer_errors_keep_the_existing_internal_response() {
        for error in [anyhow::Error::new(ClientError::Protocol {detail:"fixture protocol mismatch".into()}),anyhow::anyhow!("fixture writer failure")] {
            let result=publisher_error(error);
            assert_eq!(result.status,StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(result.code,"internal_error");
            assert_eq!(result.message,ApiError::internal().message);
        }
    }
    use crate::publisher_client::{Bounds,Client,fake::{Server,Script}};
    use crate::publisher::protocol::OpResult;
    use std::{path::Path,sync::Arc,time::Duration};
    const SECRET:&str="publisher-caller-test-secret";
    const SNAPSHOT:&str="3d521ab4-34ad-44e8-b7eb-882c0feff693";

    pub(crate) fn state_with_client(client:Client,url:&str)->(tempfile::TempDir,AppState) {
        let root=tempfile::tempdir().unwrap();
        let mut config=crate::config::RuntimeConfig::read(|key|match key {
            "DATABASE_URL"=>Some(url.into()),"AUTH_SECRET"=>Some(SECRET.into()),
            "DATA_KEY_FILE"=>Some(root.path().join("key").display().to_string()),
            "AUTH_LOG"=>Some(root.path().join("auth.log").display().to_string()),
            "NGINX_CONF_D"=>Some(root.path().join("conf").display().to_string()),
            "NGINX_STREAM_D"=>Some(root.path().join("stream").display().to_string()),
            "CERTS_DIR"=>Some(root.path().join("certs").display().to_string()),
            "NGINX_BIN"=>Some(root.path().join("must-not-run-nginx").display().to_string()),
            "API_DB_POOL_SIZE"=>Some("1".into()),_=>None,
        }).unwrap();
        config.target_dns.descriptor_path=root.path().join("target-dns.json");
        std::fs::write(&config.data_key_file,"A5".repeat(32)).unwrap();let mut state=AppState::new(config).unwrap();
        Arc::get_mut(&mut state.runtime).unwrap().publisher=client;
        (root,state)
    }
    struct Fixture { _root:tempfile::TempDir,state:AppState,db:tokio_postgres::Client }
    async fn database(client:Client)->Fixture {
        let admin_url=std::env::var("NWUI_TEST_DATABASE_URL").expect("real DB: NWUI_TEST_DATABASE_URL");
        let (admin,connection)=tokio_postgres::connect(&admin_url,tokio_postgres::NoTls).await.unwrap();tokio::spawn(connection);
        let name=format!("nwui_callers_{}",uuid::Uuid::new_v4().simple());admin.batch_execute(&format!("CREATE DATABASE {name}")).await.unwrap();
        let base=admin_url.rsplit_once('/').unwrap().0;let url=format!("{base}/{name}");let(root,state)=state_with_client(client,&url);
        crate::migrations::run(&state.db,&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle")).await.unwrap();
        let(db,connection)=tokio_postgres::connect(&url,tokio_postgres::NoTls).await.unwrap();tokio::spawn(connection);
        db.batch_execute("INSERT INTO users(id,email,role,session_version,enabled,approved,totp_enabled) VALUES('caller-user','caller@example.test','admin',3,true,true,false)").await.unwrap();
        db.execute("INSERT INTO gateway_tokens(id,name,token_hash,role,created_by_user_id) VALUES('caller-gateway','caller-fixture',$1,'admin','caller-user')",&[&crate::auth::token_digest("caller-gateway-secret")]).await.unwrap();
        Fixture{_root:root,state,db}
    }
    async fn send(f:&Fixture,path:&str,args:&[&str])->(u16,Value) {
        use axum::body::Body;
        use tower::ServiceExt;
        let is_form=path.starts_with("/api/ui/");
        let mut request=axum::http::Request::builder().method(if path=="/api/v1/status"{"GET"}else{"POST"}).uri(path)
            .header("host","caller.example.test").header("origin","http://caller.example.test");
        let body=if is_form {
            let now=crate::auth::principal::now_unix_ms()/1000;let claims=json!({"id":"caller-user","sv":3,"mfaVerified":true});
            let token=crate::auth::session::encode(claims.as_object().unwrap(),SECRET,crate::auth::session::COOKIE_NAME_PLAIN,now,3600).unwrap();
            request=request.header("cookie",format!("authjs.session-token={token}")).header("content-type","application/x-www-form-urlencoded");
            url::form_urlencoded::Serializer::new(String::new()).append_pair("_apiArgs",&json!(args).to_string()).finish()
        } else {
            request=request.header("authorization","Bearer caller-gateway-secret").header("x-gateway-version",&f.state.config.version);String::new()
        };
        let context=RequestContext{ip:Some("198.51.100.42".into()),external_origin:"http://caller.example.test".into(),secure:false,origin_configured:false};
        let app=router().with_state(f.state.clone()).layer(Extension(context));
        let response=tokio::time::timeout(Duration::from_secs(5),app.oneshot(request.body(Body::from(body)).unwrap())).await.unwrap().unwrap();
        let status=response.status().as_u16();let bytes=axum::body::to_bytes(response.into_body(),65536).await.unwrap();
        (status,serde_json::from_slice(&bytes).unwrap())
    }
    fn success(ok:bool)->OpResult {
        OpResult::Ok{apply:Some(ApplyResult{ok,output:"publisher-fixture".into(),rolled_back_to:None}),skipped:vec![],uncertain:false,phase:"done".into()}
    }
    async fn audit_count(f:&Fixture)->i64 {f.db.query_one("SELECT count(*) FROM audit_logs",&[]).await.unwrap().get(0)}
    async fn socket_route(path:&str,args:&[&str],operation:&str) {
        let server=Server::start([Script::Reply(success(true))]).await.unwrap();
        let f=database(Client::socket_for_tests(server.path().to_owned(),Bounds::default())).await;let before=audit_count(&f).await;
        let(status,body)=send(&f,path,args).await;assert_eq!(status,200,"{body}");
        if operation=="ValidateLive" {assert_eq!(body["nginx"],json!({"configOk":true,"output":"publisher-fixture"}));}
        else if path.starts_with("/api/ui/"){assert_eq!(body["result"]["ok"],true);assert_eq!(body["refresh"],true);}
        else {assert_eq!(body["ok"],true);}
        let requests=server.requests.lock().await;assert_eq!(requests.len(),1);
        let wire=serde_json::to_value(&requests[0]).unwrap();assert_eq!(wire["op"]["op"],operation);
        if operation!="ValidateLive" {
            assert_eq!(wire["actor"]["ip"],"198.51.100.42");
            if path.starts_with("/api/ui/"){assert_eq!(wire["actor"]["userId"],"caller-user");}
            else{assert_eq!(wire["actor"]["label"],"gateway:caller-fixture");}
        }
        if operation=="ApplySaved" {
            assert!(wire["op"]["args"]["expected_saved_digest"].is_null());
            assert_eq!(wire["op"]["args"]["label"],if path.starts_with("/api/ui/"){Value::Null}else{json!("apply via gateway:caller-fixture")});
        }
        if operation=="RollbackSnapshot" {assert_eq!(wire["op"]["args"]["snapshot_id"],SNAPSHOT);}
        assert!(server.errors.lock().await.is_empty());assert_eq!(audit_count(&f).await,before,"API must not add writer audit rows");
    }
    #[tokio::test] #[ignore="real DB and socket client: NWUI_TEST_DATABASE_URL"]
    async fn node_status_uses_the_socket_validation_result(){socket_route("/api/v1/status",&[],"ValidateLive").await;}
    #[tokio::test] #[ignore="real DB and socket client: NWUI_TEST_DATABASE_URL"]
    async fn node_apply_uses_the_socket_result(){socket_route("/api/v1/apply",&[],"ApplySaved").await;}
    #[tokio::test] #[ignore="real DB and socket client: NWUI_TEST_DATABASE_URL"]
    async fn node_rollback_uses_the_socket_result(){socket_route(&format!("/api/v1/history/{SNAPSHOT}/rollback"),&[],"RollbackSnapshot").await;}
    #[tokio::test] #[ignore="real DB and socket client: NWUI_TEST_DATABASE_URL"]
    async fn form_apply_uses_the_socket_result(){socket_route("/api/ui/v1/forms/apply.applyConfig",&[],"ApplySaved").await;}
    #[tokio::test] #[ignore="real DB and socket client: NWUI_TEST_DATABASE_URL"]
    async fn form_rollback_uses_the_socket_result(){socket_route("/api/ui/v1/forms/history.rollback",&[SNAPSHOT],"RollbackSnapshot").await;}

    #[tokio::test] #[ignore="real DB and socket client: NWUI_TEST_DATABASE_URL"]
    async fn node_status_busy_and_every_error_fail_closed() {
        for(reply,expected)in [(OpResult::Busy,"nginx_busy"),(OpResult::Unavailable,"publisher_unavailable"),(OpResult::Unknown,"publisher_unavailable")] {
            let server=Server::start([Script::Reply(reply)]).await.unwrap();let f=database(Client::socket_for_tests(server.path().to_owned(),Bounds::default())).await;
            let(status,body)=send(&f,"/api/v1/status",&[]).await;assert_eq!(status,503,"{body}");assert_eq!(body["error"]["code"],expected);
            let requests=server.requests.lock().await;assert_eq!(requests.len(),1);assert!(matches!(requests[0].op,crate::publisher::protocol::Op::ValidateLive));assert!(server.errors.lock().await.is_empty());
        }
    }
    #[tokio::test] #[ignore="real DB: NWUI_TEST_DATABASE_URL"]
    async fn noncanonical_rollback_keeps_node_404_form_kind_and_sends_zero_requests() {
        let server=Server::start(std::iter::empty::<Script>()).await.unwrap();let f=database(Client::socket_for_tests(server.path().to_owned(),Bounds::default())).await;let before=audit_count(&f).await;
        for id in ["missing".to_owned(),SNAPSHOT.to_uppercase()] {
            let(status,body)=send(&f,&format!("/api/v1/history/{id}/rollback"),&[]).await;assert_eq!(status,404,"{body}");assert_eq!(body["error"]["code"],"not_found");
            let(status,body)=send(&f,"/api/ui/v1/forms/history.rollback",&[&id]).await;assert_eq!(status,200,"{body}");assert_eq!(body["result"]["kind"],"not_found");assert_eq!(body["result"]["ok"],false);
        }
        assert!(server.requests.lock().await.is_empty());assert!(server.errors.lock().await.is_empty());assert_eq!(audit_count(&f).await,before);
    }
    #[tokio::test] #[ignore="real DB and socket client: NWUI_TEST_DATABASE_URL"]
    async fn rollback_socket_kinds_preserve_node_status_and_form_json() {
        for(code,kind,node_status)in [("snapshot_not_found","not_found",404),("db_restore_failed","db_restore_failed",422)] {
            for form in [false,true] {
                let server=Server::start([Script::Reply(OpResult::Failed{code:code.into(),output:"rollback-fixture".into()})]).await.unwrap();let f=database(Client::socket_for_tests(server.path().to_owned(),Bounds::default())).await;
                let path=if form{"/api/ui/v1/forms/history.rollback".to_owned()}else{format!("/api/v1/history/{SNAPSHOT}/rollback")};let(status,body)=send(&f,&path,if form{&[SNAPSHOT]}else{&[]}).await;
                assert_eq!(status,if form{200}else{node_status},"{body}");if form{assert_eq!(body["result"]["kind"],kind);assert!(body["result"]["rolledBackTo"].is_null());}else{assert_eq!(body["error"]["code"],if node_status==404{"not_found"}else{"apply_failed"});}
                assert_eq!(server.requests.lock().await.len(),1);
            }
        }
    }
    #[tokio::test] #[ignore="real DB and socket client: NWUI_TEST_DATABASE_URL"]
    async fn empty_id_bad_request_mutation_is_internal_without_status_lookup() {
        let reply=crate::publisher::protocol::Response{v:1,id:String::new(),epoch:1,result:OpResult::BadRequest{code:"invalid_request".into()}};
        let body=serde_json::to_vec(&reply).unwrap();let mut frame=(body.len()as u32).to_be_bytes().to_vec();frame.extend(body);
        let server=Server::start([Script::Garbage(frame)]).await.unwrap();
        let f=database(Client::socket_for_tests(server.path().to_owned(),Bounds::default())).await;let before=audit_count(&f).await;
        let(status,body)=send(&f,"/api/v1/apply",&[]).await;
        let requests=server.requests.lock().await;
        let lookups=requests.iter().filter(|r|matches!(&r.op,crate::publisher::protocol::Op::OperationStatus{..})).count();
        assert_eq!((status,requests.len(),lookups),(500,1,0),"{body}");
        assert_eq!(body["error"]["code"],"internal_error");
        assert!(matches!(&requests[0].op,crate::publisher::protocol::Op::ApplySaved{..}));
        assert_eq!(server.connections.load(std::sync::atomic::Ordering::SeqCst),1);
        assert!(server.errors.lock().await.is_empty());assert_eq!(audit_count(&f).await,before);
    }
    #[tokio::test] #[ignore="real DB and socket client: NWUI_TEST_DATABASE_URL"]
    async fn mutation_route_errors_distinguish_unavailable_and_unconfirmed_without_api_audits() {
        for(unconfirmed,expected)in [(false,"publisher_unavailable"),(true,"apply_unconfirmed")] {
            let script=if unconfirmed{vec![Script::CloseAfterRead,Script::Reply(OpResult::Unknown)]}else{vec![Script::Reply(OpResult::Unavailable)]};
            let server=Server::start(script).await.unwrap();let f=database(Client::socket_for_tests(server.path().to_owned(),Bounds::default())).await;let before=audit_count(&f).await;
            let(status,body)=send(&f,"/api/v1/apply",&[]).await;assert_eq!(status,503,"{body}");assert_eq!(body["error"]["code"],expected);
            assert_eq!(server.requests.lock().await.len(),if unconfirmed{2}else{1});assert_eq!(audit_count(&f).await,before);
        }
    }

}
