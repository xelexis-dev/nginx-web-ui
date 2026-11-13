//! Read-only classification precedes every secret, ownership and initdb mutation.
use super::{paths,key_file::{self,KeyFileError},mounts,files,pair::Pair,state::State,error::{Error,Result}};
use std::{fs::{self,File},io::Read,path::{Path,PathBuf}};
use std::os::unix::fs::{MetadataExt,OpenOptionsExt};

#[derive(Debug,Clone)]
pub struct Roots { pub db:PathBuf,pub app:PathBuf,pub architecture:String }
#[derive(Debug)]
pub enum Decision { Empty,Fresh(Pair),Ready(Pair) }
fn refuse()->Error { Error::refusal("unsupported_layout") }
fn mismatch()->Error { Error::refusal("pair_mismatch") }
fn exists(path:&Path)->Result<bool> { match fs::symlink_metadata(path){Ok(_)=>Ok(true),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>Ok(false),Err(_)=>Err(refuse())} }
fn entries(path:&Path)->Result<Vec<String>> { fs::read_dir(path).map_err(|_|refuse())?.map(|e|e.map_err(|_|refuse())?.file_name().into_string().map_err(|_|refuse())).collect() }
fn actual_directory(path:&Path)->Result<()> { files::real_path(path).map_err(|_|refuse())?;if !fs::symlink_metadata(path).map_err(|_|refuse())?.is_dir(){return Err(refuse());}Ok(()) }
fn lost_found(root:&Path)->Result<()> {
    let path=root.join("lost+found");if !exists(&path)?{return Ok(());}
    let m=fs::symlink_metadata(&path).map_err(|_|refuse())?;
    if !m.is_dir() || m.file_type().is_symlink() || m.uid()!=0 || !entries(&path)?.is_empty(){return Err(refuse());}Ok(())
}
fn empty(root:&Path)->Result<bool> { Ok(entries(root)?.iter().all(|name|name=="lost+found")) }
fn small(path:&Path,max:u64,mode:u32)->Result<String> {
    files::real_path(path).map_err(|_|mismatch())?;
    let file=fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path).map_err(|_|mismatch())?;
    let m=file.metadata().map_err(|_|mismatch())?;
    if !m.is_file() || m.uid()!=0 || m.mode()&0o7777!=mode || m.len()==0 || m.len()>max { return Err(mismatch()); }
    let mut s=String::new();file.take(max+1).read_to_string(&mut s).map_err(|_|mismatch())?;
    if s.len()as u64>max || s.contains('\0'){return Err(mismatch());}Ok(s)
}
fn version(path:&Path)->Result<String> {
    files::real_path(path).map_err(|_|refuse())?;let m=fs::symlink_metadata(path).map_err(|_|refuse())?;
    if !m.is_file() || m.len()>16{return Err(refuse());}
    fs::read_to_string(path).map(|s|s.trim().into()).map_err(|_|refuse())
}
fn private_material_dir(path:&Path)->Result<()> {
    actual_directory(path).map_err(|_|mismatch())?;let m=fs::symlink_metadata(path).map_err(|_|mismatch())?;
    if m.uid()!=0 || m.mode()&0o7777!=0o700{return Err(mismatch());}Ok(())
}
/// The pre-random-password 1.19 path never writes a replacement secret during upgrade.
pub(super) fn legacy_db_password_fallback(roots:&Roots)->Result<()> {
    let candidates=[roots.db.clone(),roots.db.join("pgdata"),roots.db.join("17/data")];
    for data in &candidates {
        if exists(&data.join(".db-random-pw"))? {
            return Err(Error{detail:Some("db_password_missing_random_marker"),..mismatch()});
        }
    }
    for data in &candidates {
        let file=data.join("PG_VERSION");
        if exists(&file)? && matches!(version(&file)?.as_str(),"15"|"17"){return Ok(());}
    }
    Err(mismatch())
}
fn app_material(roots:&Roots,required:bool,key_required:bool,legacy_modes:bool)->Result<()> {
    let secrets=roots.app.join("secrets");let tls=roots.app.join("tls");
    if exists(&secrets)? {private_material_dir(&secrets)?;}else if required{return Err(mismatch());}
    for name in ["auth_secret","db_password","admin_password"] {
        let path=secrets.join(name);if exists(&path)?{let mode=if legacy_modes&&fs::symlink_metadata(&path).map_err(|_|mismatch())?.mode()&0o7777==0o644{0o644}else{0o600};small(&path,65536,mode)?;}else if required && name!="admin_password"{if legacy_modes&&name=="db_password"{legacy_db_password_fallback(roots)?;}else{return Err(mismatch());}}
    }
    let key=paths::startup_data_key_file(&roots.app).map_err(|e|Error::refusal(e.code))?;
    match key_file::read_key_file(&key,key_file::canonical_policy(&roots.app,&key)) {
        Ok(_)=>{},Err(KeyFileError::Missing)if !key_required=>{},
        Err(KeyFileError::Missing)=>return Err(Error::refusal("key_missing")),Err(_)=>return Err(mismatch()),
    }
    if exists(&tls)? {private_material_dir(&tls)?;}else if required{return Err(mismatch());}
    let cert=tls.join("default.crt");let key=tls.join("default.key");let has_cert=exists(&cert)?;let has_key=exists(&key)?;
    if has_cert!=has_key || (required && !has_cert){return Err(mismatch());}
    if has_cert {let cert=small(&cert,1_048_576,0o644)?;let key=small(&key,1_048_576,0o600)?;crate::certificates::pem::parse_certificate_pem(&cert,&key).map_err(|_|mismatch())?;}
    Ok(())
}
fn fresh_app(roots:&Roots,pair:&Pair,has_journal:bool)->Result<()> {
    for name in entries(&roots.app)? {if ![".nwui","secrets","tls","lost+found"].contains(&name.as_str()) && !(has_journal && name==".nwui-auto"){return Err(mismatch());}}
    for (directory,allowed)in[(".nwui",&["pair-v1.json"][..]),("secrets",&["auth_secret","db_password","admin_password"][..]),("tls",&["default.crt","default.key"][..])] {
        let path=roots.app.join(directory);if exists(&path)? {actual_directory(&path)?;for name in entries(&path)?{if !allowed.contains(&name.as_str()){return Err(mismatch());}}}
    }
    app_material(roots,pair.fresh_step.as_deref()!=Some("secrets"),false,false)
}
// No content hashing at boot; refuse paths PostgreSQL could follow outside the paired tree.
pub(crate) fn regular_tree(path:&Path,depth:u8,count:&mut u64)->Result<()> {
    *count+=1;if depth>64||*count>1_000_000{return Err(refuse());}
    let metadata=match fs::symlink_metadata(path){Ok(value)=>value,Err(e)if depth>0&&e.kind()==std::io::ErrorKind::NotFound=>return Ok(()),Err(_)=>return Err(refuse())};
    if metadata.file_type().is_symlink() {
        // Fixed names/numeric OIDs only: never log a link target or an arbitrary storage name.
        if path.file_name().is_some_and(|name|name=="pg_wal") {eprintln!("[db-upgrade] symbolic link in cluster: pg_wal");}
        else if path.parent().and_then(Path::file_name).is_some_and(|name|name=="pg_tblspc") {
            if let Some(oid)=path.file_name().and_then(|name|name.to_str()).and_then(|name|name.parse::<u32>().ok()) {eprintln!("[db-upgrade] tablespace link: pg_tblspc/{oid}");}
            else {eprintln!("[db-upgrade] tablespace link in cluster");}
        }else {eprintln!("[db-upgrade] symbolic link in cluster");}
        return Err(refuse());
    }
    if !metadata.is_dir()&&!metadata.is_file(){return Err(refuse());}
    match files::real_path(path){Ok(())=>{},Err(e)if depth>0&&e.kind()==std::io::ErrorKind::NotFound=>return Ok(()),Err(_)=>return Err(refuse())}
    if metadata.is_dir() {
        let entries=match fs::read_dir(path){Ok(entries)=>entries,Err(e)if depth>0&&e.kind()==std::io::ErrorKind::NotFound=>return Ok(()),Err(_)=>return Err(refuse())};
        for entry in entries {regular_tree(&entry.map_err(|_|refuse())?.path(),depth+1,count)?;}
    }Ok(())
}
fn control_metadata(roots:&Roots)->Result<PathBuf> {
    let path=roots.db.join("17/data");actual_directory(&path)?;
    if version(&path.join("PG_VERSION"))?!="17"{return Err(refuse());}
    regular_tree(&path,0,&mut 0)?;
    files::real_path(&path.join("global/pg_control")).map_err(|_|mismatch())?;
    let metadata=fs::symlink_metadata(path.join("global/pg_control")).map_err(|_|mismatch())?;
    if !metadata.is_file() || metadata.len()==0{return Err(mismatch());}
    Ok(path)
}
pub(crate) fn control(roots:&Roots,read_id:&impl Fn(&Path)->Result<String>)->Result<String>{read_id(&control_metadata(roots)?)}
pub fn classify(roots:&Roots,read_id:&impl Fn(&Path)->Result<String>)->Result<Decision> {classify_for(None,roots,read_id)}
pub fn classify_locked(locks:&crate::upgrade::StorageLocks,roots:&Roots,read_id:&impl Fn(&Path)->Result<String>)->Result<Decision> {classify_for(Some(locks),roots,read_id)}
pub(super) fn classify_for(locks:Option<&crate::upgrade::StorageLocks>,roots:&Roots,read_id:&impl Fn(&Path)->Result<String>)->Result<Decision> {
    classify_impl(locks,roots,read_id,true)
}
/// Metadata-only classification before ownership repair; never claims a physical system identifier.
pub(crate) fn classify_metadata_locked(locks:&crate::upgrade::StorageLocks,roots:&Roots)->Result<Decision>{
    classify_impl(Some(locks),roots,&|_|Err(refuse()),false)
}
fn classify_impl(locks:Option<&crate::upgrade::StorageLocks>,roots:&Roots,read_id:&impl Fn(&Path)->Result<String>,verify_control:bool)->Result<Decision>{
    if locks.is_some_and(|l|!l.holds_d() || !l.matches_roots(&roots.db,&roots.app)){return Err(Error::refusal("storage_busy"));}
    paths::startup_data_key_file(&roots.app).map_err(|e|Error::refusal(e.code))?;
    validate_roots(roots)?;
    lost_found(&roots.db)?;lost_found(&roots.app)?;
    let journal=if exists(&roots.app.join(".nwui-auto"))? {
        private_material_dir(&roots.app.join(".nwui-auto")).map_err(|_|refuse())?;
        Some(crate::upgrade::Journal::open(&roots.app,locks.ok_or_else(refuse)?).map_err(|_|refuse())?.load().clone())
    }else{None};
    if exists(&roots.db.join("PG_VERSION"))? {return Err(Error::refusal(if version(&roots.db.join("PG_VERSION"))?=="15"{"pg15_requires_upgrade"}else{"unsupported_layout"}));}
    let db_empty=empty(&roots.db)?;
    // Only a new/interrupted fresh attempt may still have no DB pair after journal bootstrap.
    let app_empty=empty(&roots.app)? || journal.as_ref().is_some_and(|j|j.branch==crate::upgrade::Branch::Fresh)
        && entries(&roots.app)?.iter().all(|name|matches!(name.as_str(),".nwui-auto"|"lost+found"));
    if db_empty && app_empty{return Ok(Decision::Empty);}
    if db_empty{return Err(Error::existing_app());}
    for root in [&roots.db,&roots.app] {if exists(&root.join(".nwui"))?{actual_directory(&root.join(".nwui"))?;}}
    let state_path=roots.db.join(".nwui/migration-v1.json");
    let state:Option<State>=if exists(&state_path)? {let state:State=files::read_json(&state_path).map_err(|_|refuse())?;state.validate().map_err(|_|refuse())?;if !state.ready{return Err(Error::refusal("migration_incomplete"));}Some(state)}else{None};
    let db_pair=roots.db.join(".nwui/pair-v1.json");let app_pair=roots.app.join(".nwui/pair-v1.json");
    if !exists(&db_pair)? || !exists(&app_pair)? {
        if state.is_some(){return Err(Error::refusal("migration_incomplete"));}
        return Err(if exists(&db_pair)? || exists(&app_pair)? || exists(&roots.db.join("17/data/PG_VERSION"))?{mismatch()}else{refuse()});
    }
    let mut pair:Pair=files::read_json(&db_pair).map_err(|_|mismatch())?;let other:Pair=files::read_json(&app_pair).map_err(|_|mismatch())?;
    pair.validate()?;other.validate()?;
    if pair!=other {
        let mut matching=other.clone();matching.encryption_key_required=pair.encryption_key_required;
        if pair!=matching || !pair.ready{return Err(mismatch());}
        let path=paths::startup_data_key_file(&roots.app).map_err(|e|Error::refusal(e.code))?;
        key_file::read_key_file(&path,key_file::canonical_policy(&roots.app,&path)).map_err(|e|if e==KeyFileError::Missing {Error::refusal("key_missing")} else {mismatch()})?;
        // Classification is read-only; r3 completes this monotonic flag write after key agreement.
        pair.encryption_key_required=true;
    }
    if pair.architecture!=roots.architecture{return Err(refuse());}
    if pair.mode=="fresh" && state.is_some(){return Err(mismatch());}
    let major=roots.db.join("17");if exists(&major)?{actual_directory(&major)?;if entries(&major)?.iter().any(|name|name!="data"){return Err(refuse());}}
    for name in entries(&roots.db)? {if ![".nwui","17",".nwui-work","lost+found"].contains(&name.as_str()){return Err(refuse());}}
    if pair.ready {
        if verify_control{if pair.system_identifier.as_deref()!=Some(control(roots,read_id)?.as_str()){return Err(mismatch());}}else{control_metadata(roots)?;}
        if pair.mode=="migration" {let state=state.ok_or_else(||Error::refusal("migration_incomplete"))?;if state.mode!=pair.mode || state.pair_id.as_deref()!=Some(pair.pair_id.as_str()) || state.target.system_identifier!=pair.system_identifier || state.target.architecture!=pair.architecture{return Err(mismatch());}}
        app_material(roots,true,pair.encryption_key_required,pair.mode=="migration")?;return Ok(Decision::Ready(pair));
    }
    if pair.mode=="migration"{return Err(Error::refusal("migration_incomplete"));}
    if state.is_some() || exists(&roots.db.join(".nwui-work"))?{return Err(refuse());}
    for name in entries(&roots.db.join(".nwui"))?{if name!="pair-v1.json"{return Err(refuse());}}
    fresh_app(roots,&pair,journal.is_some())?;
    let data=roots.db.join("17/data");
    if exists(&data)? {
        actual_directory(&data)?;
        if exists(&data.join("PG_VERSION"))? {
            if pair.fresh_step.as_deref()==Some("secrets"){return Err(mismatch());}
            if verify_control{let actual=control(roots,read_id)?;if pair.system_identifier.as_ref().is_some_and(|id|id!=&actual){return Err(mismatch());}}else{control_metadata(roots)?;}
        }else if !entries(&data)?.is_empty() || !matches!(pair.fresh_step.as_deref(),Some("secrets"|"initdb")){return Err(refuse());}
    }else if !matches!(pair.fresh_step.as_deref(),Some("secrets"|"initdb")){return Err(mismatch());}
    Ok(Decision::Fresh(pair))
}
pub fn prepare(roots:&Roots,read_id:&impl Fn(&Path)->Result<String>)->Result<Pair> {
    // Refuse aliased/unsafe roots before locking; recheck after acquiring both locks.
    classify(roots,read_id)?;
    let _locks=lock_roots(roots)?;
    prepare_under_lock(roots,read_id,None)
}

pub fn prepare_locked(locks:&crate::upgrade::StorageLocks,roots:&Roots,read_id:&impl Fn(&Path)->Result<String>)->Result<Pair> {
    if !locks.matches_roots(&roots.db,&roots.app){return Err(Error::refusal("storage_busy"));}
    prepare_under_lock(roots,read_id,Some(locks))
}
fn prepare_under_lock(roots:&Roots,read_id:&impl Fn(&Path)->Result<String>,locks:Option<&crate::upgrade::StorageLocks>)->Result<Pair> {
    match classify_for(locks,roots,read_id)? {
        Decision::Fresh(pair)|Decision::Ready(pair)=>Ok(pair),
        Decision::Empty=>{
            let pair=Pair::fresh(&roots.architecture);pair.validate()?;
            for root in [&roots.db,&roots.app]{files::private_dir(&root.join(".nwui")).map_err(|_|refuse())?;files::atomic_json(&root.join(".nwui/pair-v1.json"),&pair).map_err(|_|refuse())?;}
            for root in [&roots.db,&roots.app]{let saved:Pair=files::read_json(&root.join(".nwui/pair-v1.json")).map_err(|_|mismatch())?;if saved!=pair{return Err(mismatch());}}
            Ok(pair)
        }
    }
}

pub(crate) struct RootLocks(Vec<File>);
impl RootLocks {
    // Custody transfers the open descriptions; unlocking here would also unlock fdholder copies.
    pub(crate) fn into_files(mut self)->Vec<File> { std::mem::take(&mut self.0) }
}
impl Drop for RootLocks {
    fn drop(&mut self) {
        // A concurrent fork can retain CLOEXEC descriptors until exec; close alone keeps their flock.
        for file in &self.0 { let _=file.unlock(); }
    }
}

// Directory flock leaves the strict pre-secret allowlist free of lock files.
pub(crate) fn lock_roots(roots:&Roots)->Result<RootLocks> { lock_modes(roots,false) }
pub(super) fn lock_key_roots(roots:&Roots)->Result<RootLocks> { lock_modes(roots,true) }
fn lock_modes(roots:&Roots,shared_pg:bool)->Result<RootLocks> {
    let mut locks=RootLocks(Vec::new());
    for (index,root) in [&roots.db,&roots.app].into_iter().enumerate() {
        actual_directory(root)?;
        let file=File::open(root).map_err(|_|refuse())?;
        if shared_pg && index==0{file.try_lock_shared()}else{file.try_lock()}.map_err(|_|Error::refusal("storage_busy"))?;
        locks.0.push(file);
    }
    Ok(locks)
}

pub(crate) fn validate_roots(roots:&Roots)->Result<()> {
    validate_roots_unlocked(roots)?;
    let db_meta=fs::metadata(&roots.db).map_err(|_|refuse())?;let app_meta=fs::metadata(&roots.app).map_err(|_|refuse())?;
    let postgres_uid=||fs::read_to_string("/etc/passwd").ok().and_then(|text|text.lines().find(|line|line.starts_with("postgres:")).and_then(|line|line.split(':').nth(2)).and_then(|uid|uid.parse::<u32>().ok()));
    let detail=if app_meta.uid()!=0 {"app_root_owner"} else if db_meta.uid()!=0 && Some(db_meta.uid())!=postgres_uid() {"db_root_owner"}
        else if app_meta.mode()&0o022!=0 {"app_root_mode"} else if db_meta.mode()&0o022!=0 {"db_root_mode"} else {return Ok(());};
    Err(Error{detail:Some(detail),..refuse()})
}
/// Pre-custody callers only (runner start, custody acquire): the owner/mode clause is checked under
/// both locks, after storage_modes has narrowed what it may (2.0.1 final.md F2).
pub(crate) fn validate_roots_unlocked(roots:&Roots)->Result<()> {
    actual_directory(&roots.db)?;actual_directory(&roots.app)?;
    if roots.db.starts_with(&roots.app) || roots.app.starts_with(&roots.db){return Err(mismatch());}
    let db_meta=fs::metadata(&roots.db).map_err(|_|refuse())?;let app_meta=fs::metadata(&roots.app).map_err(|_|refuse())?;
    if db_meta.dev()==app_meta.dev() && db_meta.ino()==app_meta.ino(){return Err(mismatch());}
    mounts::Table::read()?.distinct(&[&roots.db,&roots.app])?;
    Ok(())
}
pub(super) fn genuinely_empty(root:&Path)->Result<bool> {lost_found(root)?;empty(root)}

pub(crate) fn legacy_material(roots:&Roots)->Result<()> {validate_roots(roots)?;app_material(roots,true,false,true)}

pub(super) fn ready_material(roots:&Roots,pair:&Pair)->Result<()> {validate_roots(roots)?;app_material(roots,true,pair.encryption_key_required,pair.ready&&pair.mode=="migration")}

#[cfg(test)]
mod unlocked_tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt,symlink};
    #[test]
    fn u14_unlocked_roots_skip_only_the_owner_and_mode_clause() {
        let t=tempfile::tempdir().unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
        for p in [&roots.db,&roots.app] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o775)).unwrap();}
        validate_roots_unlocked(&roots).unwrap();
        assert_eq!(validate_roots(&roots).unwrap_err().detail,Some("app_root_mode"));
        fs::set_permissions(&roots.app,fs::Permissions::from_mode(0o755)).unwrap();assert_eq!(validate_roots(&roots).unwrap_err().detail,Some("db_root_mode"));
        fs::set_permissions(&roots.db,fs::Permissions::from_mode(0o755)).unwrap();validate_roots(&roots).unwrap();
        let link=t.path().join("link");symlink(&roots.app,&link).unwrap();
        assert!(validate_roots_unlocked(&Roots{app:link,..roots.clone()}).is_err(),"symlinked root");
        fs::create_dir(roots.app.join("inner")).unwrap();
        assert_eq!(validate_roots_unlocked(&Roots{db:roots.app.join("inner"),..roots.clone()}).unwrap_err().code,"pair_mismatch","nested roots");
        assert_eq!(validate_roots_unlocked(&Roots{db:roots.app.clone(),..roots.clone()}).unwrap_err().code,"pair_mismatch","aliased roots");
    }
}
