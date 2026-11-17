//! Fresh PG17 provisioning; paired intent precedes every storage mutation.
//! Private argv: `nwui-api storage-secrets` calls `secrets(roots, &Inputs::from_env()?,
//! /run/s6/container_environment)`; `nwui-api storage-provision` awaits `provision(roots)`.
//! Dispatch before RuntimeConfig/AppState; errors use their sanitized message and exit code.
use super::{cluster, files, inspect, layout::{self, Decision, Roots}, pair::Pair, postgres, error::{Error, Result}};
use std::{fs::{self, File, OpenOptions}, io::{Read, Write}, path::{Path, PathBuf}, time::Duration};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

#[derive(Default)]
pub struct Inputs { pub auth_secret: Option<String>, pub admin_password: Option<String> }
impl Inputs {
    pub fn from_env() -> Result<Self> {
        let get = |name| match std::env::var(name) {
            Ok(value) if !value.is_empty() => Ok(Some(value)),
            Ok(_) | Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => Err(mismatch()),
        };
        Ok(Self { auth_secret: get("AUTH_SECRET")?, admin_password: get("ADMIN_PASSWORD")? })
    }
}
fn mismatch() -> Error { Error::refusal("pair_mismatch") }
fn failed() -> Error { Error { code: "fresh_failed", message: "새 저장소 준비를 완료하지 못했습니다. 기존 값과 현재 상태는 보존했습니다. 제한된 작업 로그를 확인하세요.", exit:1,detail:None } }
fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) { Ok(_) => Ok(true), Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false), Err(_) => Err(failed()) }
}
fn identifier(path: &Path) -> Result<String> { Ok(postgres::read_control(17, path)?.system_identifier) }
fn text_valid(value: &str) -> bool { !value.is_empty() && value.len() <= 65536 && !value.contains('\0') }
fn secret(path: &Path, legacy: bool) -> Result<String> {
    files::real_path(path).map_err(|_| mismatch())?;
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).map_err(|_| mismatch())?;
    let m = file.metadata().map_err(|_| mismatch())?;
    if !m.is_file() || m.uid() != 0 || m.nlink() != 1 || !(m.mode() & 0o7777 == 0o600 || legacy && m.mode() & 0o7777 == 0o644) || m.len() > 65536 { return Err(mismatch()); }
    let mut value = String::new(); file.take(65537).read_to_string(&mut value).map_err(|_| mismatch())?;
    if !text_valid(&value) { return Err(mismatch()); } Ok(value)
}
fn atomic_material(path: &Path, bytes: &[u8], mode: u32, replace: bool) -> Result<()> {
    let parent = path.parent().ok_or_else(failed)?; files::real_path(parent).map_err(|_| mismatch())?;
    if exists(path)? {
        let m = fs::symlink_metadata(path).map_err(|_| failed())?;
        if !replace || !m.is_file() || m.file_type().is_symlink() || m.uid() != 0 || m.nlink() != 1 || m.mode() & 0o022 != 0 { return Err(mismatch()); }
    }
    let temp = parent.join(format!(".nwui-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new().write(true).create_new(true).mode(mode).open(&temp).map_err(|_| failed())?;
    file.write_all(bytes).and_then(|_| file.sync_all()).map_err(|_| failed())?;
    fs::rename(&temp, path).and_then(|_| File::open(parent)?.sync_all()).map_err(|_| failed())
}
fn random(bytes: usize) -> Result<String> {
    let mut data = vec![0; bytes]; getrandom::fill(&mut data).map_err(|_| failed())?; Ok(hex::encode(data))
}
fn ensure_secret(path: &Path, supplied: Option<&str>, bytes: usize) -> Result<()> {
    if exists(path)? { secret(path, false)?; return Ok(()); }
    let value = match supplied { Some(value) if text_valid(value) => value.to_owned(), Some(_) => return Err(mismatch()), None => random(bytes)? };
    atomic_material(path, value.as_bytes(), 0o600, false)
}
fn paired(roots: &Roots, pair: &Pair) -> Result<()> {
    pair.validate()?;
    let read = |root: &Path| -> Result<Pair> { files::read_json(&root.join(".nwui/pair-v1.json")).map_err(|_| mismatch()) };
    let old = read(&roots.db)?;
    if old != read(&roots.app)? || old.pair_id != pair.pair_id || old.mode != "fresh" { return Err(mismatch()); }
    for root in [&roots.db, &roots.app] { files::atomic_json(&root.join(".nwui/pair-v1.json"), pair).map_err(|_| failed())?; }
    if read(&roots.db)? != *pair || read(&roots.app)? != *pair { return Err(mismatch()); } Ok(())
}
fn environment(roots: &Roots, pair: &Pair, envdir: &Path) -> Result<()> {
    environment_material(roots,envdir,pair.ready && pair.mode=="migration",pair.ready)
}
/// File/material validation only: the runner separately verifies PG control and A.18 authority.
pub(crate) fn ready_environment_locked(locks:&crate::upgrade::StorageLocks,roots:&Roots,envdir:&Path)->Result<()> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app){return Err(mismatch());}
    let pair:Pair=files::read_json(&roots.app.join(".nwui/pair-v1.json")).map_err(|_|mismatch())?;pair.validate()?;
    if !pair.ready || pair.architecture!=roots.architecture{return Err(mismatch());}
    layout::ready_material(roots,&pair)?;environment(roots,&pair,envdir)
}
fn operator_material(path:&Path,name:&str,supplied:Option<&str>,legacy:bool)->Result<Option<String>> {
    let stored=if exists(path)?{Some(secret(path,legacy)?)}else{None};
    if let Some(value)=supplied {
        if !text_valid(value){return Err(mismatch());}
        if stored.as_deref()!=Some(value) {
            match atomic_material(path,value.as_bytes(),0o600,true) {
                Ok(())=>return secret(path,legacy).map(Some),
                Err(error) if error.code=="fresh_failed"=>{
                    eprintln!("[db-upgrade] operator {name} not persisted");
                    return Ok(Some(value.to_owned()));
                },
                Err(error)=>return Err(error),
            }
        }
    }
    Ok(stored)
}
fn environment_material(roots:&Roots,envdir:&Path,legacy:bool,operator_wins:bool)->Result<()> {
    files::real_path(envdir).map_err(|_| mismatch())?;
    let m = fs::symlink_metadata(envdir).map_err(|_| mismatch())?;
    if !m.is_dir() || m.uid() != 0 || m.mode() & 0o022 != 0 { return Err(mismatch()); }
    let directory = roots.app.join("secrets");
    let inputs=if operator_wins{Inputs::from_env()?}else{Inputs::default()};
    if inputs.auth_secret.as_deref().is_some_and(|s|!text_valid(s)) || inputs.admin_password.as_deref().is_some_and(|s|!text_valid(s)){return Err(mismatch());}
    let auth = operator_material(&directory.join("auth_secret"),"AUTH_SECRET",inputs.auth_secret.as_deref(),legacy)?.ok_or_else(mismatch)?;
    let password = if legacy&&!exists(&directory.join("db_password"))?{layout::legacy_db_password_fallback(roots)?;"nginx".to_owned()}else{secret(&directory.join("db_password"),legacy)?};
    let mut url = url::Url::parse("postgres://nginx@127.0.0.1:5432/nginx_web_ui").map_err(|_| failed())?;
    url.set_password(Some(&password)).map_err(|_| failed())?;
    atomic_material(&envdir.join("AUTH_SECRET"), auth.as_bytes(), 0o600, true)?;
    atomic_material(&envdir.join("DATABASE_URL"), url.as_str().as_bytes(), 0o600, true)?;
    if let Some(admin)=operator_material(&directory.join("admin_password"),"ADMIN_PASSWORD",inputs.admin_password.as_deref(),legacy)? {
        atomic_material(&envdir.join("ADMIN_PASSWORD"),admin.as_bytes(),0o600,true)?;
    }
    Ok(())
}
/// Publish existing values to s6; only a paired fresh intent may create material.
pub fn secrets(roots: &Roots, inputs: &Inputs, envdir: &Path) -> Result<()> {
    if inputs.auth_secret.as_deref().is_some_and(|s| !text_valid(s)) || inputs.admin_password.as_deref().is_some_and(|s| !text_valid(s)) { return Err(mismatch()); }
    if let Some(e) = postgres::interrupted() { return Err(e); }
    layout::prepare(roots, &identifier)?;
    let _locks = layout::lock_roots(roots)?;
    secrets_under_lock(roots, inputs, envdir, None)
}
pub fn secrets_locked(locks: &crate::upgrade::StorageLocks, roots: &Roots, inputs: &Inputs, envdir: &Path) -> Result<()> {
    if inputs.auth_secret.as_deref().is_some_and(|s| !text_valid(s)) || inputs.admin_password.as_deref().is_some_and(|s| !text_valid(s)) { return Err(mismatch()); }
    if let Some(e) = postgres::interrupted() { return Err(e); }
    layout::prepare_locked(locks, roots, &identifier)?;
    secrets_under_lock(roots, inputs, envdir, Some(locks))
}
fn secrets_under_lock(roots: &Roots, inputs: &Inputs, envdir: &Path, locks: Option<&crate::upgrade::StorageLocks>) -> Result<()> {
    let mut pair = match layout::classify_for(locks, roots, &identifier)? { Decision::Fresh(pair) | Decision::Ready(pair) => pair, Decision::Empty => return Err(mismatch()) };
    if !pair.ready && pair.fresh_step.as_deref() == Some("secrets") {
        let directory = roots.app.join("secrets"); files::private_dir(&directory).map_err(|_| mismatch())?;
        ensure_secret(&directory.join("auth_secret"), inputs.auth_secret.as_deref(), 32)?;
        ensure_secret(&directory.join("db_password"), None, 24)?;
        if let Some(admin) = inputs.admin_password.as_deref() { ensure_secret(&directory.join("admin_password"), Some(admin), 0)?; }
        let tls = roots.app.join("tls"); files::private_dir(&tls).map_err(|_| mismatch())?;
        if !exists(&tls.join("default.crt"))? && !exists(&tls.join("default.key"))? {
            // Keep the existing catch-all RSA-2048 policy without a secret-bearing subprocess.
            let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_RSA_SHA256).map_err(|_| failed())?;
            let mut params = rcgen::CertificateParams::new(vec!["unconfigured.invalid".into()]).map_err(|_| failed())?;
            params.distinguished_name.push(rcgen::DnType::CommonName, "unconfigured.invalid");
            params.not_before = std::time::SystemTime::now().into();
            params.not_after = (std::time::SystemTime::now() + Duration::from_secs(3650 * 86400)).into();
            let cert = params.self_signed(&key).map_err(|_| failed())?;
            atomic_material(&tls.join("default.key"), key.serialize_pem().as_bytes(), 0o600, false)?;
            atomic_material(&tls.join("default.crt"), cert.pem().as_bytes(), 0o644, false)?;
        }
        layout::ready_material(roots, &pair)?;
        pair.fresh_step = Some("initdb".into()); paired(roots, &pair)?;
    }
    layout::ready_material(roots, &pair)?;
    environment(roots, &pair, envdir)
}
fn workspace() -> Result<PathBuf> {
    let path = Path::new("/run").join(format!("nwui-fresh-{}", uuid::Uuid::new_v4()));
    postgres::directory(&path)?; Ok(path)
}
fn offline_identity(data: &Path, pair: &Pair) -> Result<String> {
    let control = postgres::read_control(17, data)?;
    if control.cluster_state != "shut down" || pair.system_identifier.as_ref().is_some_and(|id| id != &control.system_identifier) || exists(&data.join("postmaster.pid"))? { return Err(mismatch()); }
    Ok(control.system_identifier)
}
// Roles/databases are shared and checked separately; per-DB objects must be initdb built-ins.
async fn no_user_objects(socket: &Path, snapshot: &cluster::Snapshot) -> Result<()> {
    let root = inspect::connect(socket, "postgres", "postgres", None).await?;
    let invalid_template: bool = root.client.query_one("SELECT NOT datistemplate OR datallowconn FROM pg_catalog.pg_database WHERE datname='template0'", &[]).await.map_err(|_| failed())?.get(0);
    if invalid_template { return Err(mismatch()); }
    for database in &snapshot.databases {
        let template = database.name == "template0";
        if template {
            root.client.batch_execute("SET default_transaction_read_only=off").await.map_err(|_| failed())?;
            root.client.batch_execute("ALTER DATABASE template0 ALLOW_CONNECTIONS true").await.map_err(|_| failed())?;
        }
        let result = async {
            let connection = inspect::connect(socket, &database.name, "postgres", None).await?;
            let catalogs = connection.client.query("SELECT c.relname FROM pg_catalog.pg_class c JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid WHERE c.relkind='r' AND c.relnamespace='pg_catalog'::regnamespace AND NOT c.relisshared AND a.attname='oid' AND a.attnum>0 AND NOT a.attisdropped ORDER BY c.relname", &[]).await.map_err(|_| failed())?;
            let queries = catalogs.iter().map(|row| format!("SELECT 1 FROM pg_catalog.{} WHERE oid >= 16384", inspect::quote(row.get::<_, &str>(0)))).collect::<Vec<_>>();
            if queries.is_empty() { return Err(failed()); }
            let query = format!("SELECT EXISTS({})", queries.join(" UNION ALL "));
            Ok::<bool, Error>(connection.client.query_one(&query, &[]).await.map_err(|_| failed())?.get(0))
        }.await;
        // Restore the template gate even when inspection refuses or its connection fails.
        if template { root.client.batch_execute("ALTER DATABASE template0 ALLOW_CONNECTIONS false").await.map_err(|_| failed())?; }
        if result? { return Err(mismatch()); }
    }
    Ok(())
}
async fn inspect_empty(socket: &Path, password: &str) -> Result<cluster::Snapshot> {
    let snapshot = cluster::inspect_cluster(socket, password, None, true).await?;
    if snapshot.databases.iter().any(|db| db.encoding != "UTF8" || db.collate != "C" || db.ctype != "C" || db.provider != "c") || snapshot.data.values().any(|db| !db.tables.is_empty() || !db.sequences.is_empty() || db.has_encrypted) { return Err(mismatch()); }
    no_user_objects(socket, &snapshot).await?;
    Ok(snapshot)
}
fn marker(data: &Path, name: &str) -> Result<()> {
    let path = data.join(name);
    if exists(&path)? {
        files::real_path(&path).map_err(|_| mismatch())?;
        let m = fs::symlink_metadata(&path).map_err(|_| mismatch())?;
        if !m.is_file() || m.nlink() != 1 || m.len() != 0 || m.uid() != fs::metadata(data).map_err(|_| failed())?.uid() || m.mode() & 0o7777 != 0o600 { return Err(mismatch()); }
    } else { postgres::private_file(&path, b"")?; } Ok(())
}
/// Complete only the recorded fresh step. A ready installation is never provisioned again.
pub async fn provision(roots: &Roots) -> Result<()> {
    provision_using(roots, postgres::Private::start).await
}
pub(super) async fn provision_using(roots: &Roots, start: impl FnOnce(u8, &Path, &Path) -> Result<postgres::Private>) -> Result<()> {
    if let Some(e) = postgres::interrupted() { return Err(e); }
    layout::classify(roots, &identifier)?;
    let _locks = layout::lock_roots(roots)?;
    provision_under_lock(roots, start, None).await
}
pub async fn provision_locked(locks: &crate::upgrade::StorageLocks, roots: &Roots) -> Result<()> {
    if !locks.matches_roots(&roots.db, &roots.app) { return Err(Error::refusal("storage_busy")); }
    if let Some(e) = postgres::interrupted() { return Err(e); }
    provision_under_lock(roots, postgres::Private::start, Some(locks)).await
}
async fn provision_under_lock(roots: &Roots, start: impl FnOnce(u8, &Path, &Path) -> Result<postgres::Private>, locks: Option<&crate::upgrade::StorageLocks>) -> Result<()> {
    let mut pair = match layout::classify_for(locks, roots, &identifier)? {
        Decision::Ready(pair) => { layout::ready_material(roots, &pair)?; return Ok(()); },
        Decision::Fresh(pair) if pair.fresh_step.as_deref() != Some("secrets") => pair,
        _ => return Err(mismatch()),
    };
    let password = secret(&roots.app.join("secrets/db_password"), false)?;
    let work = workspace()?; let data = roots.db.join("17/data");
    if pair.fresh_step.as_deref() == Some("initdb") {
        postgres::own(&roots.db)?;
        if !exists(&roots.db.join("17"))? { postgres::directory(&roots.db.join("17"))?; }
        if !exists(&data)? { postgres::directory(&data)?; }
        if !exists(&data.join("PG_VERSION"))? {
            if fs::read_dir(&data).map_err(|_| failed())?.next().is_some() { return Err(mismatch()); }
            // An empty directory can survive interruption between mkdir and chown.
            postgres::own(&roots.db.join("17"))?; postgres::own(&data)?;
            let mut init = postgres::command(17, "initdb")?;
            init.arg("-D").arg(&data).args(["--encoding=UTF8", "--locale=C", "--auth-local=trust", "--auth-host=scram-sha-256"]);
            postgres::capture(init, &work, Duration::from_secs(300))?;
        }
        pair.system_identifier = Some(offline_identity(&data, &pair)?);
    }
    offline_identity(&data, &pair)?;
    let pg = start(17, &data, &work)?;
    let snapshot = inspect_empty(&pg.socket, &password).await?;
    let has_role = snapshot.roles.iter().any(|role| role.name == "nginx");
    let has_db = snapshot.databases.iter().any(|db| db.name == "nginx_web_ui");
    if matches!(pair.fresh_step.as_deref(), Some("initdb" | "role")) && has_db || matches!(pair.fresh_step.as_deref(), Some("database" | "verify")) && !has_role || pair.fresh_step.as_deref() == Some("verify") && !has_db { return Err(mismatch()); }
    // Do not adopt/advance an interrupted initdb until its catalogs have proved empty.
    if pair.fresh_step.as_deref() == Some("initdb") { pair.fresh_step = Some("role".into()); paired(roots, &pair)?; }
    let connection = inspect::connect(&pg.socket, "postgres", "postgres", None).await?;
    // A combined SET/CREATE batch would begin its implicit transaction read-only.
    connection.client.batch_execute("SET default_transaction_read_only=off").await.map_err(|_| failed())?;
    connection.client.batch_execute("SET standard_conforming_strings=on").await.map_err(|_| failed())?;
    if pair.fresh_step.as_deref() == Some("role") {
        if !has_role {
            connection.client.batch_execute(&format!("CREATE ROLE nginx LOGIN PASSWORD '{}'", password.replace('\'', "''"))).await.map_err(|_| failed())?;
        }
        pair.fresh_step = Some("database".into()); paired(roots, &pair)?;
    }
    if pair.fresh_step.as_deref() == Some("database") {
        if !has_db { connection.client.batch_execute("CREATE DATABASE nginx_web_ui OWNER nginx").await.map_err(|_| failed())?; }
        pair.fresh_step = Some("verify".into()); paired(roots, &pair)?;
    }
    inspect_empty(&pg.socket, &password).await?;
    drop(connection); pg.stop()?;
    offline_identity(&data, &pair)?;
    marker(&data, ".db-random-pw")?; marker(&data, ".migrated-nginx-web-ui")?;
    pair.ready = true; pair.fresh_step = None; paired(roots, &pair)?;
    if !matches!(layout::classify_for(locks, roots, &identifier)?, Decision::Ready(_)) { return Err(mismatch()); } Ok(())
}

#[cfg(test)]
mod operator_tests {
    use super::*;
    #[test]
    #[ignore="A37 real full tmpfs persistence failure; isolated native container"]
    fn native_g37_operator_persist_full_exports_supplied_without_rewriting_stored(){
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        let app=Path::new("/run/g37-full");assert!(app.is_dir());let directory=app.join("secrets");files::private_dir(&directory).unwrap();
        let auth=directory.join("auth_secret");atomic_material(&auth,b"g37-stored-auth-secret",0o600,false).unwrap();atomic_material(&directory.join("db_password"),b"g37-stored-db-password",0o600,false).unwrap();
        let before=fs::read(&auth).unwrap();let mut fill=File::create(app.join("fill")).unwrap();let block=[0u8;65536];loop{if let Err(error)=fill.write_all(&block){assert_eq!(error.raw_os_error(),Some(libc::ENOSPC));break;}}drop(fill);
        let env=Path::new("/run/g37-operator-env");files::private_dir(env).unwrap();let roots=Roots{db:"/not-used-by-strict-material".into(),app:app.into(),architecture:"arm64".into()};
        environment_material(&roots,env,false,true).unwrap();assert_eq!(fs::read(&auth).unwrap(),before);assert!(fs::read_to_string(env.join("AUTH_SECRET")).unwrap()==std::env::var("AUTH_SECRET").unwrap());
        assert!(!env.join("ADMIN_PASSWORD").exists());assert!(fs::read_to_string(env.join("DATABASE_URL")).unwrap().contains("g37-stored-db-password"));
    }
}
