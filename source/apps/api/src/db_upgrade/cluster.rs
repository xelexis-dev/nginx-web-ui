//! Private cluster inspection and logical comparison; source superusers remain trusted.
use super::{catalog::{self,Database,Role},inspect::{self,DatabaseSnapshot},error::{Error,Result}};
use crate::crypto::SecretBox;
use serde::{Serialize,Deserialize};
use std::{collections::BTreeMap,path::Path};
fn unsupported()->Error{Error::refusal("unsupported_layout")}
fn failed()->Error{Error{code:"verification_failed",message:"이전 데이터 검증을 완료하지 못했습니다. 원본과 작업 사본을 보존했습니다.",exit:1,detail:None}}
#[derive(Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Collation {pub schema:String,pub name:String,pub provider:String,pub locale:Option<String>,pub recorded:Option<String>,pub actual:Option<String>}
#[derive(Debug,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Snapshot {
    pub databases:Vec<Database>,pub roles:Vec<Role>,pub data:BTreeMap<String,DatabaseSnapshot>,
    pub collations:BTreeMap<String,Vec<Collation>>,pub largest_index_bytes:u64,
}
impl Snapshot {
    pub fn application(&self)->Result<String>{catalog::validate(&self.databases,&self.roles,false)?.ok_or_else(unsupported)}
    fn canonical(&self)->Result<serde_json::Value> {
        let app=self.application()?;
        if self.data.len()!=self.databases.len()||self.databases.iter().any(|db|!self.data.contains_key(&db.name)){return Err(unsupported());}
        let mut databases=self.databases.clone();for db in &mut databases{if db.name==app{db.name="application".into();}}databases.sort_by(|a,b|a.name.cmp(&b.name));
        let mut roles=self.roles.clone();roles.sort_by(|a,b|a.name.cmp(&b.name));
        let data=self.data.iter().map(|(name,value)|(if *name==app{"application"}else{name.as_str()},value)).collect::<BTreeMap<_,_>>();
        serde_json::to_value((databases,roles,data)).map_err(|_|failed())
    }
    pub fn require_equivalent(&self,target:&Self)->Result<()> {if self.canonical()?!=target.canonical()?{return Err(Error{code:"baseline_changed",message:"이전 기준과 데이터가 달라졌습니다. 전환 후 데이터 변경은 정상일 수 있습니다.",exit:1,detail:None});}Ok(())}
    pub fn key_required(&self)->bool{self.data.values().any(|db|db.has_encrypted)}
    pub fn table_counts(&self)->Result<BTreeMap<String,u64>>{Ok(self.data.get(&self.application()?).ok_or_else(unsupported)?.tables.iter().map(|(name,table)|(name.clone(),table.rows)).collect())}
}
async fn template0(client:&tokio_postgres::Client,enable:bool)->Result<()> {
    if enable {
        let allowed:bool=client.query_one("SELECT datallowconn FROM ONLY pg_catalog.pg_database WHERE datname='template0'",&[]).await.map_err(|_|failed())?.get(0);
        if allowed{return Err(unsupported());}
    }
    client.batch_execute("SET default_transaction_read_only=off").await.map_err(|_|failed())?;
    client.batch_execute(if enable{"ALTER DATABASE template0 ALLOW_CONNECTIONS true"}else{"ALTER DATABASE template0 ALLOW_CONNECTIONS false"}).await.map_err(|_|failed())
}
pub async fn inspect_cluster(socket:&Path,password:&str,key:Option<&SecretBox>,fresh:bool)->Result<Snapshot> {
    inspect_inner(socket,password,key,fresh,false,false).await
}
/// V2 inspects the application through its profile allowances, never strict NONE.
/// Keep the complete catalog/roles and all non-application snapshots for independent comparison.
pub(crate) async fn inspect_non_application(socket:&Path,password:&str,key:Option<&SecretBox>)->Result<Snapshot> {
    inspect_inner(socket,password,key,false,true,false).await
}
async fn inspect_inner(socket:&Path,password:&str,key:Option<&SecretBox>,fresh:bool,skip_application:bool,skip_template0:bool)->Result<Snapshot> {
    let root=inspect::connect(socket,"postgres","postgres",None).await?;
    let (databases,roles)=inspect::read_catalog(&root.client).await?;
    let application=catalog::validate(&databases,&roles,fresh)?;
    let invalid:bool=root.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_prepared_xacts) OR EXISTS(SELECT 1 FROM pg_catalog.pg_replication_slots) OR EXISTS(SELECT 1 FROM ONLY pg_catalog.pg_tablespace WHERE spcname NOT IN('pg_default','pg_global') OR spcowner<>10 OR pg_tablespace_location(oid)<>'') OR EXISTS(SELECT 1 FROM ONLY pg_catalog.pg_db_role_setting WHERE setconfig IS NOT NULL) OR EXISTS(SELECT 1 FROM ONLY pg_catalog.pg_auth_members a JOIN ONLY pg_catalog.pg_authid m ON m.oid=a.member JOIN ONLY pg_catalog.pg_authid r ON r.oid=a.roleid WHERE m.rolname='nginx' OR r.rolname='nginx') OR EXISTS(SELECT 1 FROM ONLY pg_catalog.pg_authid WHERE left(rolname,3)='pg_' AND (oid>=16384 OR rolcanlogin OR rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))",&[]).await.map_err(|_|failed())?.get(0);
    if invalid{return Err(unsupported());}
    if roles.iter().any(|role|role.name=="nginx") {
        let connection=inspect::connect(socket,application.as_deref().unwrap_or("postgres"),"nginx",Some(password)).await?;
        connection.client.query_one("SELECT 1",&[]).await.map_err(|error|inspect::connection_error(&error))?;
    }
    let mut data=BTreeMap::new();let mut collations=BTreeMap::new();let mut largest_index_bytes=0;
    for db in &databases {
        if skip_template0 && db.name=="template0"{continue;}
        if skip_application && application.as_deref()==Some(db.name.as_str()){continue;}
        if db.name=="template0"{template0(&root.client,true).await?;}
        let result=async {
            let connection=inspect::connect(socket,&db.name,"postgres",None).await?;
            let snapshot=inspect::snapshot_database(&connection.client,key,!fresh&&application.as_deref()==Some(db.name.as_str())).await?;
            let bytes:i64=connection.client.query_one("SELECT COALESCE(max(pg_relation_size(indexrelid)),0) FROM pg_index",&[]).await.map_err(|_|failed())?.get(0);
            let mut used=Vec::new();
            for row in connection.client.query("SELECT DISTINCT n.nspname,c.collname,c.collprovider::text,COALESCE(to_jsonb(c)->>'colllocale',to_jsonb(c)->>'colliculocale',c.collcollate),c.collversion,CASE WHEN c.collprovider='d' THEN NULL ELSE pg_collation_actual_version(c.oid) END FROM pg_index i JOIN pg_class t ON t.oid=i.indrelid JOIN pg_namespace tn ON tn.oid=t.relnamespace JOIN LATERAL unnest(i.indcollation) u(oid) ON true JOIN pg_collation c ON c.oid=u.oid JOIN pg_namespace n ON n.oid=c.collnamespace WHERE tn.nspname IN('public','drizzle') ORDER BY n.nspname,c.collname",&[]).await.map_err(|_|failed())? {
                let provider:String=row.get(2);if !matches!(provider.as_str(),"c"|"i"|"d"|"b"){return Err(unsupported());}
                used.push(Collation{schema:row.get(0),name:row.get(1),provider,locale:row.get(3),recorded:row.get(4),actual:row.get(5)});
            }
            Ok::<_,Error>((snapshot,u64::try_from(bytes).map_err(|_|unsupported())?,used))
        }.await;
        if db.name=="template0"{template0(&root.client,false).await?;}
        let (snapshot,bytes,used)=result?;largest_index_bytes=largest_index_bytes.max(bytes);data.insert(db.name.clone(),snapshot);collations.insert(db.name.clone(),used);
    }
    Ok(Snapshot{databases,roles,data,collations,largest_index_bytes})
}
