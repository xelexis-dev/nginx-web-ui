//! Read-only logical baselines; store only hashes/counts, never decrypted values.
use super::error::{Error,Result};
use crate::crypto::{SecretBox,is_sealed};
use serde::{Serialize,Deserialize};
use serde_json::Value;
use sha2::{Digest,Sha256};
use std::{collections::{BTreeMap,BTreeSet},path::Path};
use tokio_postgres::{Client,NoTls};
use futures_core::Stream;

pub fn quote(identifier:&str)->String{format!("\"{}\"",identifier.replace('"',"\"\""))}
fn unsupported()->Error{Error::refusal("unsupported_layout")}
fn failed()->Error{Error{code:"verification_failed",message:"이전 데이터 검증을 완료하지 못했습니다. 원본과 작업 사본을 보존했습니다.",exit:1,detail:None}}
pub fn check_sealed(value:&Value,key:Option<&SecretBox>)->Result<bool> {
    match value {
        Value::String(s) if is_sealed(s)=>{if key.and_then(|k|k.open(Some(s))).is_none(){return Err(Error::refusal("pair_mismatch"));}Ok(true)},
        Value::Array(items)=>{let mut found=false;for item in items{found|=check_sealed(item,key)?;}Ok(found)},
        Value::Object(items)=>{let mut found=false;for item in items.values(){found|=check_sealed(item,key)?;}Ok(found)},
        _=>Ok(false),
    }
}
pub struct Connection{pub client:Client,driver:tokio::task::JoinHandle<()>}
impl Drop for Connection{fn drop(&mut self){self.driver.abort();}}
pub fn connection_config(socket:&Path,db:&str,user:&str,password:Option<&str>)->tokio_postgres::Config {
    let mut config=tokio_postgres::Config::new();
    config.host_path(socket).port(50432).dbname(db).user(user).connect_timeout(std::time::Duration::from_secs(10));
    // Client startup options take precedence over source DB/role settings before library loading.
    config.options(if user=="postgres"{super::settings::ROOT_OPTIONS}else{super::settings::APP_OPTIONS});
    if let Some(password)=password{config.password(password);}config
}
/// Connect diagnostics are a closed static vocabulary, never PostgreSQL text or credentials.
pub(crate) fn connection_error(error:&tokio_postgres::Error)->Error {
    let detail=match error.code().map(|code|code.code()) {
        Some("28P01"|"28000")=>"db_auth_failed",
        None=>"db_unreachable",
        Some(code) if code.starts_with("08")=>"db_unreachable",
        _=>"db_connect_failed",
    };
    Error{detail:Some(detail),..Error::refusal("pair_mismatch")}
}
pub async fn connect(socket:&Path,db:&str,user:&str,password:Option<&str>)->Result<Connection> {
    let (client,connection)=connection_config(socket,db,user,password).connect(NoTls).await.map_err(|error|if user=="nginx"{connection_error(&error)}else{failed()})?;
    let driver=tokio::spawn(async move{let _=connection.await;});
    let c=Connection{client,driver};
    c.client.batch_execute("SET search_path=pg_catalog; SET TimeZone='UTC'; SET DateStyle='ISO,YMD'; SET extra_float_digits=3; SET statement_timeout='3600000'; SET lock_timeout='10000'; SET default_transaction_read_only=on").await.map_err(|_|failed())?;
    Ok(c)
}
#[derive(Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Table{pub rows:u64,pub sha256:String}
#[derive(Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct DatabaseSnapshot {
    pub tables:BTreeMap<String,Table>,pub sequences:BTreeMap<String,String>,pub schema_sha256:String,pub has_encrypted:bool,
}
// Released NWUI tables; other application relations require explicit qualification.
const TABLES:&[&str]=&["certificate_jobs","users","sessions","audit_logs","upstreams","nginx_servers","nginx_streams","config_snapshots","applied_config","certificates","acme_accounts","acme_challenges","app_settings","gateway_tokens","access_tokens","nodes","smtp_settings","notifications","notification_deliveries","mail_recipients","webhook_targets","nginx_metric_samples","metric_hosts","domain_metric_minutes","domain_metric_hours","metric_parse_state","oauth_providers","oauth_accounts"];
fn known_table(schema:&str,name:&str)->bool{(schema=="public"&&TABLES.contains(&name))||(schema=="drizzle"&&name=="__drizzle_migrations")}
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
struct LegacySignature {hash:String,created_at:i64}
#[derive(Deserialize)]
struct LegacyLedger {signatures:Vec<LegacySignature>}
pub async fn require_legacy_ledger(client:&Client)->Result<()> {
    let refused=||Error{code:"ledger_unknown",message:"변경 이력이 지원되는 1.19.11 이력과 일치하지 않습니다. 원본과 변경 이력을 확인하세요.",exit:2,detail:None};
    let released:LegacyLedger=serde_json::from_str(include_str!("legacy-1.19.11.json")).map_err(|_|failed())?;
    let ordered=released.signatures.into_iter().map(|entry|(entry.hash,entry.created_at)).collect::<Vec<_>>();
    let expected=ordered.iter().cloned().collect::<BTreeSet<_>>();
    if expected.len()!=28{return Err(failed());}
    // This precedes profile inspection; do not execute a history view or custom type.
    let plain_table:bool=client.query_one("SELECT EXISTS(SELECT 1 FROM ONLY pg_catalog.pg_class c JOIN ONLY pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='drizzle' AND c.relname='__drizzle_migrations' AND c.relkind='r' AND (SELECT count(*) FROM ONLY pg_catalog.pg_attribute a WHERE a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped AND ((a.attname='hash' AND a.atttypid='pg_catalog.text'::regtype) OR (a.attname='created_at' AND a.atttypid='pg_catalog.int8'::regtype)))=2)",&[]).await.map_err(|_|refused())?.get(0);
    if !plain_table{return Err(refused());}
    // ONLY avoids child/FDW execution before profile checks; distinct keeps legacy duplicates.
    let rows=client.query("SELECT DISTINCT CASE WHEN octet_length(hash)=64 THEN hash ELSE NULL END,created_at FROM ONLY drizzle.__drizzle_migrations LIMIT 29",&[]).await.map_err(|_|refused())?;
    let mut actual=BTreeSet::new();
    for row in rows {
        let hash:Option<String>=row.try_get(0).map_err(|_|refused())?;let time:Option<i64>=row.try_get(1).map_err(|_|refused())?;
        actual.insert((hash.ok_or_else(refused)?,time.ok_or_else(refused)?));
    }
    if actual==expected{return Ok(());}
    // Only a built-in id ordering may run before the schema/profile admission boundary.
    let plain_id:bool=client.query_one("SELECT EXISTS(SELECT 1 FROM ONLY pg_catalog.pg_attribute a JOIN ONLY pg_catalog.pg_class c ON c.oid=a.attrelid JOIN ONLY pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='drizzle' AND c.relname='__drizzle_migrations' AND a.attname='id' AND a.atttypid='pg_catalog.int4'::regtype AND a.attnum>0 AND NOT a.attisdropped)",&[]).await.map_err(|_|refused())?.get(0);
    if !plain_id{return Err(refused());}
    let rows=client.query("SELECT CASE WHEN octet_length(hash)=64 THEN hash ELSE NULL END,created_at FROM ONLY drizzle.__drizzle_migrations ORDER BY created_at,id LIMIT 29",&[]).await.map_err(|_|refused())?;
    let mut prefix=Vec::new();for row in rows{let hash:Option<String>=row.try_get(0).map_err(|_|refused())?;let time:Option<i64>=row.try_get(1).map_err(|_|refused())?;prefix.push((hash.ok_or_else(refused)?,time.ok_or_else(refused)?));}
    if old_ledger_prefix(&prefix,&ordered){return Err(Error{code:"source_too_old",message:"먼저 1.19.11로 올린 뒤 이 이미지로 업그레이드하세요. 원본은 보존했습니다.",exit:2,detail:None});}Err(refused())
}
fn old_ledger_prefix(rows:&[(String,i64)],expected:&[(String,i64)])->bool{
    !rows.is_empty()&&rows.len()<expected.len()&&expected.len()==28&&rows.iter().zip(expected).all(|(a,b)|a==b)
}
#[cfg(test)]
mod old_ledger_tests{
    use super::*;
    #[test]
    fn released_older_prefixes_are_proven_but_divergence_duplicates_gaps_and_empty_are_not(){
        #[derive(Deserialize)]#[serde(rename_all="camelCase")]struct Prefix{release:String,count:usize,ledger_sha256:String}
        let ledger:LegacyLedger=serde_json::from_str(include_str!("legacy-1.19.11.json")).unwrap();let expected=ledger.signatures.into_iter().map(|s|(s.hash,s.created_at)).collect::<Vec<_>>();
        let tagged:Vec<Prefix>=serde_json::from_str(include_str!("fixtures/old-release-prefixes.json")).unwrap();assert_eq!(tagged.iter().map(|p|p.count).collect::<Vec<_>>(),[14,15,18,19,20,24,25]);
        for tag in tagged{let rows=&expected[..tag.count];let mut digest=Sha256::new();for(hash,time)in rows{digest.update(format!("{time}:{hash}\n"));}assert_eq!(hex::encode(digest.finalize()),tag.ledger_sha256,"{}",tag.release);assert!(old_ledger_prefix(rows,&expected));}
        for change in ["hash","time","extra","gap","duplicate","reordered","empty","full"]{let mut rows=expected[..20].to_vec();match change{"hash"=>rows[2].0="f".repeat(64),"time"=>rows[2].1+=1,"extra"=>rows.push(("f".repeat(64),rows.last().unwrap().1+1)),"gap"=>{rows.remove(3);},"duplicate"=>rows.insert(3,rows[2].clone()),"reordered"=>rows.swap(2,3),"empty"=>rows.clear(),_=>rows=expected.clone()};assert!(!old_ledger_prefix(&rows,&expected),"{change}");}
    }
}
async fn validate_object_catalogs(client:&Client,application:bool,admitted:&Admitted)->Result<()> {
    let catalogs=client.query(OBJECT_CATALOGS_QUERY,&[]).await.map_err(|_|failed())?.into_iter().map(|r|r.get::<_,String>(0)).collect::<Vec<_>>();
    let query=object_catalog_query(&catalogs,application,admitted)?;
    let invalid:bool=client.query_one(&query,&[]).await.map_err(|_|failed())?.get(0);
    if invalid{return Err(unsupported());}Ok(())
}
fn object_catalog_query(catalogs:&[String],application:bool,admitted:&Admitted)->Result<String>{
    let namespaces="SELECT oid FROM ONLY pg_catalog.pg_namespace WHERE nspname IN('public','drizzle')";
    let relations=format!("SELECT oid FROM ONLY pg_catalog.pg_class WHERE relnamespace IN({namespaces})");
    let toast=format!("SELECT reltoastrelid FROM ONLY pg_catalog.pg_class WHERE oid IN({relations})");
    let mut queries=Vec::new();
    for name in catalogs {
        let name=name.as_str();
        // These catalogs are covered by table/type/constraint/default/enum/trigger snapshots.
        let covered=if name=="pg_namespace" {
            // Dropped temp tables leave namespaces behind; objects in them still fail other guards.
            Some(if application{"o.nspname IN('public','drizzle') OR o.nspname ~ '^pg_(toast_)?temp_[0-9]+$'"}else{"o.nspname ~ '^pg_(toast_)?temp_[0-9]+$'"}.into())
        }else if !application{None}else{match name {
            "pg_class"=>Some(format!("o.relnamespace IN({namespaces}) OR (o.relnamespace='pg_toast'::regnamespace AND (o.oid IN({toast}) OR o.oid IN(SELECT indexrelid FROM ONLY pg_catalog.pg_index WHERE indrelid IN({toast}))))")),
            "pg_type"=>Some(format!("o.typnamespace IN({namespaces})")),
            "pg_constraint"=>Some(format!("o.connamespace IN({namespaces}) AND o.conrelid IN({relations})")),
            "pg_attrdef"=>Some(format!("o.adrelid IN({relations})")),
            "pg_enum"=>Some(format!("o.enumtypid IN(SELECT oid FROM ONLY pg_catalog.pg_type WHERE typnamespace IN({namespaces}))")),
            // Only the built-in FK machinery behind the captured application constraints.
            "pg_trigger"=>Some(format!("(o.tgisinternal AND o.tgfoid<16384 AND o.tgrelid IN({relations}) AND o.tgconstraint IN(SELECT oid FROM ONLY pg_catalog.pg_constraint WHERE contype='f' AND connamespace IN({namespaces}))) OR o.oid IN({})",oid_list(&admitted.triggers))),
            "pg_proc" if !admitted.functions.is_empty()=>Some(format!("o.oid IN({})",oid_list(&admitted.functions))),
            _=>None,
        }};
        let filter=covered.map(|s|format!(" AND NOT ({s})")).unwrap_or_default();
        queries.push(format!("SELECT 1 FROM ONLY pg_catalog.{} o WHERE o.oid>=16384{filter}",quote(name)));
    }
    if queries.is_empty(){return Err(failed());}
    Ok(format!("SELECT EXISTS({})",queries.join(" UNION ALL ")))
}
pub(crate) const OBJECT_CATALOGS_QUERY:&str="SELECT c.relname FROM ONLY pg_catalog.pg_class c JOIN ONLY pg_catalog.pg_attribute a ON a.attrelid=c.oid WHERE c.relkind='r' AND c.relnamespace='pg_catalog'::regnamespace AND NOT c.relisshared AND a.attname='oid' AND a.attnum>0 AND NOT a.attisdropped ORDER BY c.relname";
pub(crate) const TYPES_QUERY:&str="SELECT n.nspname,t.typname,t.typtype::text,COALESCE(c.relname,''),COALESCE(e.typname,''),COALESCE(e.typtype::text,''),COALESCE(ec.relname,''),COALESCE(en.nspname,'') FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace LEFT JOIN pg_class c ON c.oid=t.typrelid LEFT JOIN pg_type e ON e.oid=t.typelem LEFT JOIN pg_namespace en ON en.oid=e.typnamespace LEFT JOIN pg_class ec ON ec.oid=e.typrelid WHERE n.nspname IN('public','drizzle')";
pub(crate) const ENUMS_QUERY:&str="SELECT t.typname,json_agg(e.enumlabel ORDER BY e.enumsortorder) FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace JOIN pg_enum e ON e.enumtypid=t.oid WHERE n.nspname='public' GROUP BY t.typname";
pub(crate) const RELATIONS_QUERY:&str="SELECT n.nspname,c.relname,c.relkind::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname IN('public','drizzle') AND c.relkind NOT IN('i','I','t') ORDER BY n.nspname,c.relname";
pub(crate) const INVALID_INDEX_QUERY:&str="SELECT EXISTS(SELECT 1 FROM pg_constraint c JOIN pg_namespace n ON n.oid=c.connamespace WHERE n.nspname IN('public','drizzle') AND NOT c.convalidated) OR EXISTS(SELECT 1 FROM pg_index i JOIN pg_class c ON c.oid=i.indrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname IN('public','drizzle') AND (NOT i.indisvalid OR NOT i.indisready))";
pub(crate) const LARGEST_INDEX_QUERY:&str="SELECT COALESCE(max(pg_relation_size(indexrelid)),0) FROM pg_index";
fn database_objects_query(functions:&str)->String{format!("SELECT EXISTS(SELECT 1 FROM pg_extension WHERE extname<>'plpgsql') OR EXISTS(SELECT 1 FROM pg_namespace WHERE nspname NOT IN('public','pg_catalog','information_schema') AND NOT($1 AND nspname='drizzle') AND nspname NOT LIKE 'pg_toast%' AND nspname NOT LIKE 'pg_temp_%') OR EXISTS(SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname IN('public','drizzle'){functions}) OR EXISTS(SELECT 1 FROM pg_largeobject_metadata) OR EXISTS(SELECT 1 FROM pg_subscription)")}

/// Complete strict non-application gates; no allowlist or transport-specific subset.
pub(crate) fn non_application_checks(catalogs:&[String])->Result<Vec<String>>{
    Ok(vec![object_catalog_query(catalogs,false,&Admitted::default())?,database_objects_query("").replace("$1","false"),format!("SELECT EXISTS({TYPES_QUERY})"),format!("SELECT EXISTS({ENUMS_QUERY})"),format!("SELECT EXISTS({RELATIONS_QUERY})"),INVALID_INDEX_QUERY.into()])
}
fn schema_hash(lines:&[String])->String{
    let mut hash=Sha256::new();for value in lines{hash.update((value.len()as u64).to_be_bytes());hash.update(value.as_bytes());}hex::encode(hash.finalize())
}
/// Called only after all strict object/type/relation gates pass; no table can be present.
pub(crate) fn non_application_snapshot(groups:&[Value])->Result<DatabaseSnapshot>{
    if groups.len()!=SCHEMA_QUERIES.len(){return Err(failed());}let mut lines=Vec::new();
    for group in groups{for line in group.as_array().ok_or_else(failed)?{lines.push(line.as_str().ok_or_else(failed)?.to_owned());}}
    Ok(DatabaseSnapshot{tables:BTreeMap::new(),sequences:BTreeMap::new(),schema_sha256:schema_hash(&lines),has_encrypted:false})
}
pub(crate) fn used_collations(rows:&Value)->Result<Vec<super::cluster::Collation>>{
    fn text(v:&Value)->Result<String>{v.as_str().map(str::to_owned).ok_or_else(failed)}
    fn optional(v:&Value)->Result<Option<String>>{if v.is_null(){Ok(None)}else{text(v).map(Some)}}
    rows.as_array().ok_or_else(failed)?.iter().map(|row|{let r=row.as_array().filter(|r|r.len()==6).ok_or_else(failed)?;let provider=text(&r[2])?;if !matches!(provider.as_str(),"c"|"i"|"d"|"b"){return Err(unsupported());}
        Ok(super::cluster::Collation{schema:text(&r[0])?,name:text(&r[1])?,provider,locale:optional(&r[3])?,recorded:optional(&r[4])?,actual:optional(&r[5])?})}).collect()
}
// Ordered query groups preserve the legacy length-prefixed schema hash.
pub(crate) const SCHEMA_QUERIES: &[&str] = &[
        "SELECT jsonb_build_array(n.nspname,pg_get_userbyid(n.nspowner),(SELECT jsonb_agg(jsonb_build_array(grantor::text,grantee::text,privilege_type,is_grantable) ORDER BY grantor,grantee,privilege_type,is_grantable) FROM aclexplode(COALESCE(n.nspacl,acldefault('n',n.nspowner)))))::text FROM pg_namespace n WHERE n.nspname IN('public','drizzle') ORDER BY n.nspname",
        "SELECT jsonb_build_array(n.nspname,c.relname,c.relkind::text,pg_get_userbyid(c.relowner),c.relacl::text)::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname IN('public','drizzle') ORDER BY n.nspname,c.relname",
        "SELECT jsonb_build_array(n.nspname,c.relname,a.attname,format_type(a.atttypid,a.atttypmod),a.attnotnull,pg_get_expr(d.adbin,d.adrelid))::text FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace LEFT JOIN pg_attrdef d ON d.adrelid=c.oid AND d.adnum=a.attnum WHERE n.nspname IN('public','drizzle') AND c.relkind='r' AND a.attnum>0 AND NOT a.attisdropped ORDER BY n.nspname,c.relname,a.attnum",
        "SELECT jsonb_build_array(n.nspname,c.conname,pg_get_constraintdef(c.oid))::text FROM pg_constraint c JOIN pg_namespace n ON n.oid=c.connamespace WHERE n.nspname IN('public','drizzle') ORDER BY n.nspname,c.conname",
        // pg_upgrade recreates internal FK trigger names from new OIDs; compare their semantics.
        "SELECT jsonb_build_array(n.nspname,c.relname,co.conname,p.proname,t.tgtype,t.tgenabled::text,t.tgdeferrable,t.tginitdeferred,t.tgattr::text,t.tgnargs,encode(t.tgargs,'hex'),pg_get_expr(t.tgqual,t.tgrelid))::text FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_constraint co ON co.oid=t.tgconstraint JOIN pg_proc p ON p.oid=t.tgfoid WHERE n.nspname IN('public','drizzle') ORDER BY n.nspname,c.relname,co.conname,p.proname,t.tgtype",
        "SELECT jsonb_build_array(n.nspname,c.relname,pg_get_indexdef(c.oid))::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname IN('public','drizzle') AND c.relkind='i' ORDER BY n.nspname,c.relname",
        "SELECT jsonb_build_array(n.nspname,t.typname,e.enumlabel)::text FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace JOIN pg_enum e ON e.enumtypid=t.oid WHERE n.nspname IN('public','drizzle') ORDER BY n.nspname,t.typname,e.enumsortorder",
        "SELECT jsonb_build_array(schemaname,sequencename,sequenceowner,data_type::text,start_value,min_value,max_value,increment_by,cycle,cache_size)::text FROM pg_sequences WHERE schemaname IN('public','drizzle') ORDER BY schemaname,sequencename",
    "SELECT jsonb_build_array(n.nspname,p.proname,pg_get_function_identity_arguments(p.oid),pg_get_function_result(p.oid),l.lanname,p.prosecdef,p.provolatile::text,p.proconfig::text,pg_get_userbyid(p.proowner),encode(sha256(convert_to(p.prosrc,'UTF8')),'hex'))::text FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace JOIN pg_language l ON l.oid=p.prolang WHERE n.nspname IN('public','drizzle') ORDER BY n.nspname,p.proname,pg_get_function_identity_arguments(p.oid)",
    "SELECT jsonb_build_array(n.nspname,c.relname,t.tgname,pg_get_triggerdef(t.oid))::text FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname IN('public','drizzle') AND NOT t.tgisinternal ORDER BY n.nspname,c.relname,t.tgname",
];
pub(crate) async fn schema_lines(client:&Client)->Result<Vec<String>> {
    let mut lines=Vec::new();
    for query in SCHEMA_QUERIES {for row in client.query(*query,&[]).await.map_err(|_|failed())? {lines.push(row.get(0));}}
    Ok(lines)
}

pub(crate) struct Allowances {pub tables:BTreeSet<String>,pub functions:BTreeSet<String>,pub triggers:BTreeSet<String>,pub unowned_sequences:BTreeSet<String>}
impl Allowances {pub(crate) const NONE:Self=Self{tables:BTreeSet::new(),functions:BTreeSet::new(),triggers:BTreeSet::new(),unowned_sequences:BTreeSet::new()};}
#[derive(Default)]
struct Admitted {tables:BTreeSet<(String,String)>,functions:BTreeSet<u32>,triggers:BTreeSet<u32>,unowned_sequences:BTreeSet<(String,String)>}
impl Admitted {fn table(&self,schema:&str,name:&str)->bool{known_table(schema,name)||self.tables.contains(&(schema.into(),name.into()))}}
fn oid_list(ids:&BTreeSet<u32>)->String {if ids.is_empty(){"0".into()}else{ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",")}}
fn object_name(line:&str)->Result<(String,String)> {let v:Value=serde_json::from_str(line).map_err(|_|failed())?;Ok((v.get(0).and_then(Value::as_str).ok_or_else(failed)?.into(),v.get(1).and_then(Value::as_str).ok_or_else(failed)?.into()))}
async fn admit_exact(client:&Client,application:bool,allow:&Allowances)->Result<Admitted>{
    let mut admitted=Admitted::default();
    if allow.tables.is_empty()&&allow.functions.is_empty()&&allow.triggers.is_empty()&&allow.unowned_sequences.is_empty(){return Ok(admitted);}
    if !application{return Err(unsupported());}
    for row in client.query(SCHEMA_QUERIES[1],&[]).await.map_err(|_|failed())? {
        let line:String=row.get(0);if allow.tables.contains(&line){let v:Value=serde_json::from_str(&line).map_err(|_|failed())?;if v.get(2).and_then(Value::as_str)!=Some("r"){return Err(unsupported());}admitted.tables.insert(object_name(&line)?);}
    }
    for row in client.query(SCHEMA_QUERIES[7],&[]).await.map_err(|_|failed())? {let line:String=row.get(0);if allow.unowned_sequences.contains(&line){admitted.unowned_sequences.insert(object_name(&line)?);}}
    for row in client.query(&SCHEMA_QUERIES[8].replacen("SELECT ","SELECT p.oid,",1),&[]).await.map_err(|_|failed())? {let line:String=row.get(1);if allow.functions.contains(&line){admitted.functions.insert(row.get(0));}}
    for row in client.query(&SCHEMA_QUERIES[9].replacen("SELECT ","SELECT t.oid,t.tgenabled::text,",1),&[]).await.map_err(|_|failed())? {let enabled:String=row.get(1);let line:String=row.get(2);if allow.triggers.contains(&line){if enabled!="O"{return Err(unsupported());}admitted.triggers.insert(row.get(0));}}
    Ok(admitted)
}
pub async fn snapshot_database(client:&Client,key:Option<&SecretBox>,application:bool)->Result<DatabaseSnapshot>{snapshot_database_with(client,key,application,&Allowances::NONE).await}
pub(crate) async fn snapshot_database_with(client:&Client,key:Option<&SecretBox>,application:bool,allow:&Allowances)->Result<DatabaseSnapshot> {
    if !application {
        if !allow.tables.is_empty()||!allow.functions.is_empty()||!allow.triggers.is_empty()||!allow.unowned_sequences.is_empty(){return Err(unsupported());}
        let catalogs=client.query(OBJECT_CATALOGS_QUERY,&[]).await.map_err(|_|failed())?.into_iter().map(|r|r.get::<_,String>(0)).collect::<Vec<_>>();
        for query in non_application_checks(&catalogs)?{if client.query_one(&query,&[]).await.map_err(|_|failed())?.get::<_,bool>(0){return Err(unsupported());}}
        let mut groups=Vec::new();for query in SCHEMA_QUERIES{groups.push(serde_json::to_value(client.query(*query,&[]).await.map_err(|_|failed())?.into_iter().map(|r|r.get::<_,String>(0)).collect::<Vec<_>>()).map_err(|_|failed())?);}
        return non_application_snapshot(&groups);
    }
    let admitted=admit_exact(client,application,allow).await?;
    validate_object_catalogs(client,application,&admitted).await?;
    let functions=if admitted.functions.is_empty(){String::new()}else{format!(" AND p.oid NOT IN({})",oid_list(&admitted.functions))};
    let query=database_objects_query(&functions);
    let invalid:bool=client.query_one(&query,&[&application]).await.map_err(|_|failed())?.get(0);
    if invalid{return Err(unsupported());}
    validate_types(client,application,&admitted).await?;
    let relations=client.query(RELATIONS_QUERY,&[]).await.map_err(|_|failed())?;
    let mut tables=BTreeMap::new();let mut sequences=BTreeMap::new();let mut has_encrypted=false;
    for rel in relations {
        let schema:String=rel.get(0);let name:String=rel.get(1);let kind:String=rel.get(2);
        if !application || (kind!="r"&&kind!="S") || (kind=="r"&&!admitted.table(&schema,&name)){return Err(unsupported());}
        let relation=format!("{}.{}",quote(&schema),quote(&name));
        if kind=="S" {
            let owned:bool=client.query_one("SELECT EXISTS(SELECT 1 FROM pg_depend d JOIN pg_class s ON s.oid=d.objid JOIN pg_namespace sn ON sn.oid=s.relnamespace JOIN pg_class t ON t.oid=d.refobjid JOIN pg_namespace tn ON tn.oid=t.relnamespace WHERE sn.nspname=$1 AND s.relname=$2 AND d.deptype IN('a','i') AND tn.nspname IN('public','drizzle'))",&[&schema,&name]).await.map_err(|_|failed())?.get(0);
            if !owned&&!admitted.unowned_sequences.contains(&(schema.clone(),name.clone())){return Err(unsupported());}
            let value:String=client.query_one(&format!("SELECT jsonb_build_array(last_value,is_called)::text FROM {relation}"),&[]).await.map_err(|_|failed())?.get(0);
            sequences.insert(format!("{schema}.{name}"),hex::encode(Sha256::digest(value.as_bytes())));continue;
        }
        // shortcut: rows above 16 MiB refuse; qualify a larger bounded stream before supporting them.
        let query=format!("SELECT encoded FROM (SELECT CASE WHEN octet_length(row_to_json(t)::text)>16777216 THEN NULL ELSE row_to_json(t)::text END AS encoded FROM ONLY {relation} t) data ORDER BY encoded COLLATE \"C\"");
        let stream=client.query_raw(&query,std::iter::empty::<&(dyn tokio_postgres::types::ToSql+Sync)>()).await.map_err(|_|failed())?;
        let mut stream=std::pin::pin!(stream);let mut hash=Sha256::new();let mut rows=0u64;
        while let Some(row)=std::future::poll_fn(|cx|stream.as_mut().poll_next(cx)).await {
            let value:String=row.map_err(|_|failed())?.get::<_,Option<String>>(0).ok_or_else(unsupported)?;
            if value.len()>16_777_216{return Err(unsupported());}
            let json:Value=serde_json::from_str(&value).map_err(|_|failed())?;has_encrypted|=check_sealed(&json,key)?;
            hash.update((value.len()as u64).to_be_bytes());hash.update(value.as_bytes());rows=rows.checked_add(1).ok_or_else(failed)?;
        }
        tables.insert(format!("{schema}.{name}"),Table{rows,sha256:hex::encode(hash.finalize())});
    }
    let invalid:bool=client.query_one(INVALID_INDEX_QUERY,&[]).await.map_err(|_|failed())?.get(0);
    if invalid{return Err(unsupported());}
    Ok(DatabaseSnapshot{tables,sequences,schema_sha256:schema_hash(&schema_lines(client).await?),has_encrypted})
}

pub async fn read_catalog(client:&Client)->Result<(Vec<super::catalog::Database>,Vec<super::catalog::Role>)> {
    use super::catalog::{Database,Role};
    let mut databases=Vec::new();let mut roles=Vec::new();
    for row in client.query("SELECT datname,pg_get_userbyid(datdba),pg_encoding_to_char(encoding),datcollate,datctype,datlocprovider::text,COALESCE(to_jsonb(d)->>'datlocale',to_jsonb(d)->>'daticulocale') FROM ONLY pg_catalog.pg_database d ORDER BY datname",&[]).await.map_err(|_|failed())? {
        databases.push(Database{name:row.get(0),owner:row.get(1),encoding:row.get(2),collate:row.get(3),ctype:row.get(4),provider:row.get(5),icu:row.get(6)});
    }
    for row in client.query("SELECT rolname,oid::text,rolcanlogin,CASE WHEN rolname='postgres' THEN rolsuper AND rolcreatedb AND rolcreaterole AND rolreplication AND rolbypassrls ELSE rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls OR rolconnlimit<>-1 OR rolvaliduntil IS NOT NULL END,rolpassword FROM ONLY pg_catalog.pg_authid WHERE left(rolname,3)<>'pg_' ORDER BY rolname",&[]).await.map_err(|_|failed())? {
        let password:Option<String>=row.get(4);
        roles.push(Role{name:row.get(0),oid:row.get(1),login:row.get(2),elevated:row.get(3),password_sha256:password.map(|p|hex::encode(Sha256::digest(p.as_bytes())))});
    }
    Ok((databases,roles))
}

const ENUMS:&[(&str,&[&str])]=&[
    ("protocol",&["tcp","udp"]),("role",&["admin","editor","viewer"]),
    ("challenge_type",&["http-01","dns-01"]),("cert_status",&["issuing","issued","failed","awaiting-dns"]),
    ("cert_source",&["acme","manual"]),("notification_severity",&["info","success","warning","error"]),
    ("webhook_format",&["generic","discord","dooray","custom"]),("token_kind",&["user","server"]),
    ("node_status",&["ok","unreachable","unauthorized","version_mismatch","error"]),
];
async fn validate_types(client:&Client,application:bool,admitted:&Admitted)->Result<()> {
    let rows=client.query(TYPES_QUERY,&[]).await.map_err(|_|failed())?;
    for row in rows {
        let schema:String=row.get(0);let name:String=row.get(1);let kind:String=row.get(2);let relation:String=row.get(3);
        let element:String=row.get(4);let element_kind:String=row.get(5);let element_relation:String=row.get(6);let element_schema:String=row.get(7);
        let known=|kind:&str,name:&str,relation:&str|match kind{"e"=>schema=="public"&&ENUMS.iter().any(|(n,_)|*n==name),"c"=>admitted.table(&schema,relation),_=>false};
        if !application || !(known(&kind,&name,&relation)||(kind=="b"&&element_schema==schema&&name==format!("_{element}")&&known(&element_kind,&element,&element_relation))){return Err(unsupported());}
    }
    for row in client.query(ENUMS_QUERY,&[]).await.map_err(|_|failed())? {
        let name:String=row.get(0);let labels:Value=row.get(1);
        let expected=ENUMS.iter().find(|(n,_)|*n==name).ok_or_else(unsupported)?.1;
        let actual=labels.as_array().ok_or_else(unsupported)?.iter().map(|v|v.as_str().ok_or_else(unsupported)).collect::<Result<Vec<_>>>()?;
        if actual!=expected{return Err(unsupported());}
    }
    Ok(())
}

#[cfg(test)]
mod profile_tests {
    use super::*;
    struct Native {client:Client,drivers:Vec<tokio::task::JoinHandle<()>>}
    impl Drop for Native {fn drop(&mut self){for h in &self.drivers{h.abort();}}}
    async fn native()->Native {
        let uri=std::env::var("NWUI_TEST_DATABASE_URL").expect("isolated native test PostgreSQL is required");
        let mut config:tokio_postgres::Config=uri.parse().unwrap();
        let (root,driver)=config.connect(NoTls).await.unwrap();let root_driver=tokio::spawn(async move{let _=driver.await;});
        let name=format!("nwui_inspect_{}",uuid::Uuid::new_v4().simple());root.batch_execute(&format!("CREATE DATABASE {}",quote(&name))).await.unwrap();
        config.dbname(&name);let (client,driver)=config.connect(NoTls).await.unwrap();let app_driver=tokio::spawn(async move{let _=driver.await;});
        client.batch_execute("SET search_path=pg_catalog; SET TimeZone='UTC'; SET DateStyle='ISO,YMD'; SET extra_float_digits=3; CREATE TABLE public.users(id text PRIMARY KEY,value jsonb)").await.unwrap();
        Native{client,drivers:vec![root_driver,app_driver]}
    }
    async fn line(client:&Client,query:usize,name:&str)->String {
        client.query(SCHEMA_QUERIES[query],&[]).await.unwrap().into_iter().map(|r|r.get::<_,String>(0)).find(|s|serde_json::from_str::<Value>(s).unwrap().get(1).and_then(Value::as_str)==Some(name)).unwrap()
    }
    #[tokio::test]
    #[ignore="requires disposable NWUI_TEST_DATABASE_URL; run explicitly"]
    async fn none_preserves_legacy_eight_query_bytes_and_encrypted_row_baseline() {
        let n=native().await;let key=SecretBox::from_key([9;32]);let sealed=key.seal("synthetic-inspector-fixture").unwrap();
        n.client.execute("INSERT INTO public.users VALUES('u1',$1)",&[&serde_json::json!({"nested":[sealed]})]).await.unwrap();
        let old=snapshot_database(&n.client,Some(&key),true).await.unwrap();let with=snapshot_database_with(&n.client,Some(&key),true,&Allowances::NONE).await.unwrap();assert_eq!(old,with);assert!(with.has_encrypted);
        let mut legacy=Sha256::new();for query in &SCHEMA_QUERIES[..8]{for row in n.client.query(*query,&[]).await.unwrap(){let value:String=row.get(0);legacy.update((value.len()as u64).to_be_bytes());legacy.update(value.as_bytes());}}
        assert_eq!(with.schema_sha256,hex::encode(legacy.finalize()));assert_eq!(with.tables["public.users"].rows,1);
        assert_eq!(snapshot_database_with(&n.client,None,true,&Allowances::NONE).await.unwrap_err().code,"pair_mismatch");
        n.client.batch_execute("CREATE FUNCTION public.foreign_function() RETURNS int LANGUAGE sql AS 'SELECT 1'").await.unwrap();
        assert_eq!(snapshot_database_with(&n.client,Some(&key),true,&Allowances::NONE).await.unwrap_err().code,"unsupported_layout");
    }
    #[tokio::test]
    #[ignore="requires disposable NWUI_TEST_DATABASE_URL; run explicitly"]
    async fn exact_function_body_and_trigger_definition_only_are_admitted() {
        let n=native().await;
        n.client.batch_execute("CREATE FUNCTION public.assign_value() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN RETURN NEW; END;$$; CREATE TRIGGER assigned BEFORE INSERT ON public.users FOR EACH ROW EXECUTE FUNCTION public.assign_value()").await.unwrap();
        let mut allow=Allowances::NONE;allow.functions.insert(line(&n.client,8,"assign_value").await);allow.triggers.insert(n.client.query(SCHEMA_QUERIES[9],&[]).await.unwrap()[0].get(0));
        assert_eq!(snapshot_database(&n.client,None,true).await.unwrap_err().code,"unsupported_layout");
        snapshot_database_with(&n.client,None,true,&allow).await.unwrap();
        n.client.batch_execute("ALTER TABLE public.users DISABLE TRIGGER assigned").await.unwrap();assert_eq!(snapshot_database_with(&n.client,None,true,&allow).await.unwrap_err().code,"unsupported_layout");
        n.client.batch_execute("ALTER TABLE public.users ENABLE TRIGGER assigned; CREATE OR REPLACE FUNCTION public.assign_value() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN NEW.id := 'changed'; RETURN NEW; END;$$").await.unwrap();
        assert_eq!(snapshot_database_with(&n.client,None,true,&allow).await.unwrap_err().code,"unsupported_layout");
    }
    #[tokio::test]
    #[ignore="requires disposable NWUI_TEST_DATABASE_URL; run explicitly"]
    async fn exact_new_table_and_unowned_sequence_do_not_admit_extra_objects() {
        let n=native().await;n.client.batch_execute("CREATE TABLE public.new_table(id integer); CREATE SEQUENCE public.new_sequence").await.unwrap();
        let mut allow=Allowances::NONE;allow.tables.insert(line(&n.client,1,"new_table").await);allow.unowned_sequences.insert(line(&n.client,7,"new_sequence").await);
        snapshot_database_with(&n.client,None,true,&allow).await.unwrap();assert_eq!(snapshot_database(&n.client,None,true).await.unwrap_err().code,"unsupported_layout");
        n.client.batch_execute("CREATE SEQUENCE public.extra_sequence").await.unwrap();assert_eq!(snapshot_database_with(&n.client,None,true,&allow).await.unwrap_err().code,"unsupported_layout");
        n.client.batch_execute("DROP SEQUENCE public.extra_sequence; CREATE TABLE public.extra_table(id integer)").await.unwrap();assert_eq!(snapshot_database_with(&n.client,None,true,&allow).await.unwrap_err().code,"unsupported_layout");
        assert_eq!(snapshot_database_with(&n.client,None,false,&allow).await.unwrap_err().code,"unsupported_layout");
    }
}

/// Auto-only complete collation collection; legacy manual readers keep their original policy.
pub(crate) async fn auto_collations(client:&Client)->Result<Vec<super::cluster::Collation>>{
    let rows=client.query(AUTO_COLLATIONS_QUERY,&[]).await.map_err(|_|failed())?;
    Ok(rows.into_iter().map(|r|super::cluster::Collation{schema:r.get(0),name:r.get(1),provider:r.get(2),locale:r.get(3),recorded:r.get(4),actual:r.get(5)}).collect())
}

/// The frozen schema queries stay unchanged. This auto-only guard includes non-indexed
/// attributes and every dependent class (including partition keys and domains).
pub(crate) const AUTO_COLLATIONS_QUERY:&str="WITH used AS (SELECT a.attcollation AS oid FROM ONLY pg_catalog.pg_attribute a JOIN ONLY pg_catalog.pg_class t ON t.oid=a.attrelid JOIN ONLY pg_catalog.pg_namespace n ON n.oid=t.relnamespace WHERE a.attnum>0 AND NOT a.attisdropped AND t.relpersistence IN('p','u') AND n.nspname NOT IN('pg_catalog','information_schema') UNION SELECT unnest(i.indcollation) FROM ONLY pg_catalog.pg_index i UNION SELECT d.refobjid FROM ONLY pg_catalog.pg_depend d WHERE d.refclassid='pg_catalog.pg_collation'::regclass) SELECT DISTINCT n.nspname,c.collname,c.collprovider::text,CASE WHEN c.collprovider='c' THEN CASE WHEN c.collctype IN('C','POSIX') THEN c.collcollate ELSE COALESCE(c.collctype,'unknown libc ctype') END ELSE COALESCE(to_jsonb(c)->>'colllocale',to_jsonb(c)->>'colliculocale',c.collcollate) END,c.collversion,CASE WHEN c.collprovider='d' THEN NULL ELSE pg_collation_actual_version(c.oid) END FROM used u JOIN ONLY pg_catalog.pg_collation c ON c.oid=u.oid JOIN ONLY pg_catalog.pg_namespace n ON n.oid=c.collnamespace WHERE u.oid<>0 ORDER BY n.nspname,c.collname";
