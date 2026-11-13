//! A.22 immutable source identity and explicit pre/post-DDL verification.
use super::StepError;
use crate::db_upgrade::profiles::LedgerProfile;
use std::collections::BTreeSet;

#[derive(Clone,PartialEq,Eq)]
struct LedgerRow {id:i64,hash:String,created_at:i64}
struct LedgerState {rows:Vec<LedgerRow>,last_value:i64,is_called:bool}
fn ledger_error(rehearsal:bool,rule:u8)->StepError {StepError{code:if rehearsal{"rehearsal_failed"}else{"verification_failed"},message:"데이터 변경 이력 검증에 실패했습니다. 기존 데이터를 보존했습니다.",detail:Some(format!("ledger rule {rule}"))}}
fn check_ledger(before:&LedgerState,after:&LedgerState,profile:&LedgerProfile,packaged:&[(String,i64)],rehearsal:bool)->Result<(),StepError>{
    let refusal=|rule|ledger_error(rehearsal,rule);
    profile.check_packaged_prefix(packaged).map_err(|_|refusal(3))?;
    let source=profile.signatures.iter().map(|r|(r.hash.clone(),r.created_at)).collect::<BTreeSet<_>>();
    let before_ids=before.rows.iter().map(|r|r.id).collect::<BTreeSet<_>>();
    if !rehearsal && (before_ids.len()!=before.rows.len() || before.rows.iter().any(|r|!after.rows.contains(r))){return Err(refusal(1));}
    let suffix=&packaged[profile.signatures.len()..];
    let suffix_set=suffix.iter().cloned().collect::<BTreeSet<_>>();
    let mut new=after.rows.iter().filter(|r|!before_ids.contains(&r.id)).collect::<Vec<_>>();new.sort_by_key(|r|r.id);
    let max_old=before.rows.iter().map(|r|r.id).max().ok_or_else(||refusal(1))?;
    if new.len()!=suffix.len() || suffix_set.len()!=suffix.len() || !suffix_set.is_disjoint(&source)
        || new.iter().zip(suffix).any(|(row,signature)|row.id<=max_old || row.hash!=signature.0 || row.created_at!=signature.1){return Err(refusal(2));}
    let signatures=|rows:&[LedgerRow]|rows.iter().map(|r|(r.hash.clone(),r.created_at)).collect::<BTreeSet<_>>();
    let expected=source.union(&suffix_set).cloned().collect::<BTreeSet<_>>();
    if signatures(&before.rows)!=source || signatures(&after.rows)!=expected || after.rows.iter().map(|r|r.id).collect::<BTreeSet<_>>().len()!=after.rows.len(){return Err(refusal(3));}
    let max_id=after.rows.iter().map(|r|r.id).max().ok_or_else(||refusal(4))?;
    if !rehearsal && (!after.is_called || after.last_value<max_id || after.last_value<before.last_value){return Err(refusal(4));}
    Ok(())
}

use super::journal::Candidate;
use crate::{crypto::SecretBox,db_upgrade::{cluster::Snapshot,inspect::{self,Allowances}}};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest,Sha256};
use std::collections::BTreeMap;
use tokio_postgres::Client;
use futures_core::Stream;

type Columns=BTreeMap<(String,String),Vec<String>>;
#[derive(Clone,Copy,PartialEq,Eq,PartialOrd,Ord)]
enum Kind {Namespace,Relation,Attribute,Constraint,ForeignTrigger,Index,Enum,Sequence,Function,Trigger}
struct Object {kind:Kind,values:Vec<Value>}
fn fixture_error()->StepError {StepError{code:"verification_failed",message:"패키지의 데이터 검증 기준을 확인하지 못했습니다.",detail:Some("profile_fixture".into())}}
fn verification_error()->StepError {StepError{code:"verification_failed",message:"이전 데이터 검증에 실패했습니다. 기존 데이터를 보존했습니다.",detail:None}}
fn rehearsal_error()->StepError {StepError{code:"rehearsal_failed",message:"데이터 구조 변경 예행 검증에 실패했습니다. 기존 데이터를 보존했습니다.",detail:None}}
fn hash_lines(lines:&[String])->String {let mut h=Sha256::new();for line in lines{h.update((line.len()as u64).to_be_bytes());h.update(line.as_bytes());}hex::encode(h.finalize())}
fn text(v:&Value)->Result<&str,StepError>{v.as_str().filter(|s|!s.contains('\0')).ok_or_else(fixture_error)}
fn hex_hash(v:&str)->bool {v.len()==64&&v.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))}
impl Object {
    fn parse(line:&str)->Result<Self,StepError>{
        let values:Vec<Value>=serde_json::from_str(line).map_err(|_|fixture_error())?;
        if values.len()<3||!matches!(text(&values[0])?,"public"|"drizzle")||text(&values[1])?.is_empty(){return Err(fixture_error());}
        let kind=match values.len(){
            3 if !values[2].is_string()=>Kind::Namespace,
            3=>{let def=text(&values[2])?;if def.starts_with("CREATE "){Kind::Index}else if ["PRIMARY KEY ","UNIQUE ","FOREIGN KEY ","CHECK ","EXCLUDE "].iter().any(|p|def.starts_with(p)){Kind::Constraint}else{Kind::Enum}},
            5=>Kind::Relation,6=>Kind::Attribute,12=>Kind::ForeignTrigger,
            10 if values[4].is_number()=>Kind::Sequence,10=>Kind::Function,4=>Kind::Trigger,_=>return Err(fixture_error()),
        };
        if kind==Kind::Attribute {if text(&values[2])?.is_empty()||!values[3].is_string()||!values[4].is_boolean()||!(values[5].is_string()||values[5].is_null()){return Err(fixture_error());}}
        if kind==Kind::Relation&&!matches!(text(&values[2])?,"r"|"S"|"i"){return Err(fixture_error());}
        if kind==Kind::Function&&(!values[5].is_boolean()||!hex_hash(text(&values[9])?)){return Err(fixture_error());}
        Ok(Self{kind,values})
    }
    fn name(&self)->Result<(String,String),StepError>{Ok((text(&self.values[0])?.into(),text(&self.values[1])?.into()))}
    fn key(&self)->Result<(Kind,String,String,String,String,i64),StepError>{
        let (schema,name)=self.name()?;
        let sub=match self.kind{Kind::Function=>text(&self.values[2])?,Kind::ForeignTrigger|Kind::Trigger=>text(&self.values[2])?,_=>""};
        let more=if self.kind==Kind::ForeignTrigger{text(&self.values[3])?}else{""};
        let number=if self.kind==Kind::ForeignTrigger{self.values[4].as_i64().ok_or_else(fixture_error)?}else{0};
        Ok((self.kind,schema,name,sub.into(),more.into(),number))
    }
    fn detail(&self)->String {
        let label=match self.kind{Kind::Namespace=>"namespace",Kind::Relation=>"relation",Kind::Attribute=>"column",Kind::Constraint=>"constraint",Kind::ForeignTrigger|Kind::Trigger=>"trigger",Kind::Index=>"index",Kind::Enum=>"enum",Kind::Sequence=>"sequence",Kind::Function=>"function"};
        let mut names=self.values.iter().take(if matches!(self.kind,Kind::Attribute|Kind::ForeignTrigger|Kind::Trigger){3}else{2}).filter_map(Value::as_str).map(|v|v.chars().filter(|c|!c.is_control()).take(128).collect::<String>()).collect::<Vec<_>>();
        names.insert(0,label.into());names.join(" ").chars().take(512).collect()
    }
}
fn objects(lines:&[String])->Result<(Vec<Object>,Columns),StepError>{
    if lines.is_empty()||lines.iter().collect::<BTreeSet<_>>().len()!=lines.len(){return Err(fixture_error());}
    let mut parsed=Vec::new();let mut columns:Columns=BTreeMap::new();let mut previous=None;
    for line in lines {
        let object=Object::parse(line)?;let key=object.key()?;
        if previous.as_ref().is_some_and(|p|p>&key){return Err(fixture_error());}
        previous=Some(key);
        // shortcut: lines omit attnum/enumsortorder; preserve captured order and prove it by native drift/runtime hashes until the frozen format changes.
        if object.kind==Kind::Attribute {let cols=columns.entry(object.name()?).or_default();let col=text(&object.values[2])?.to_owned();if cols.contains(&col){return Err(fixture_error());}cols.push(col);}
        parsed.push(object);
    }
    Ok((parsed,columns))
}
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct SourceFile {version:u8,profile_id:String,ledger_sha256:String,postgres_major:u8,capture:Value,objects:Vec<String>}
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct TargetFile {version:u8,profile_id:String,postgres_major:u8,migrations_sha256:String,capture:Value,objects:Vec<String>,rules:Vec<Binding>,data_rules:Vec<DataRule>}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {object:String,rule:Rule}
#[derive(Deserialize)]
#[serde(tag="rule",rename_all="kebab-case",deny_unknown_fields)]
enum Rule {ColumnEquals{value:Value},RowVersion{tables:Vec<String>},SequenceAtLeastMax{sequence:String,columns:Vec<String>},TableEmpty,ObjectPresent}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Lookup {table:String,key:String,value:String}
#[derive(Deserialize)]
#[serde(rename_all="kebab-case")]
enum DataRuleKind {ColumnBackfillCoalesceLookup}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DataRule {migration:String,rule:DataRuleKind,table:String,column:String,lookup:Lookup,on:String}
fn source_column(source:&SourceProfile,table:&str,column:&str)->Result<(String,String),StepError>{
    let table=qualified(table,2)?;let name=(table[0].clone(),table[1].clone());
    if !source.columns.get(&name).is_some_and(|columns|columns.iter().any(|c|c==column)){return Err(fixture_error());}Ok(name)
}
fn validate_data_rules(rules:&[DataRule],source:&SourceProfile,packaged:&[crate::migrations::Migration])->Result<(),StepError>{
    let suffix=packaged.get(source.prefix_len..).ok_or_else(fixture_error)?;let mut seen=BTreeSet::new();
    for rule in rules {
        source_column(source,&rule.table,&rule.column)?;source_column(source,&rule.table,&rule.on)?;
        source_column(source,&rule.lookup.table,&rule.lookup.key)?;source_column(source,&rule.lookup.table,&rule.lookup.value)?;
        if !suffix.iter().any(|m|m.tag==rule.migration)||!seen.insert((&rule.table,&rule.column)){return Err(fixture_error());}
    }Ok(())
}
fn lookup_expression(rule:&DataRule)->Result<String,StepError>{
    let lookup=qualified(&rule.lookup.table,2)?;
    match rule.rule {DataRuleKind::ColumnBackfillCoalesceLookup=>Ok(format!("COALESCE(t.{column},(SELECT l.{value} FROM ONLY {table} l WHERE l.{key}=t.{on})) AS {column}",column=inspect::quote(&rule.column),value=inspect::quote(&rule.lookup.value),table=table_name(&lookup[0],&lookup[1]),key=inspect::quote(&rule.lookup.key),on=inspect::quote(&rule.on)))}
}
pub(crate) struct SourceProfile {id:String,sha256:String,objects:Vec<String>,schema_sha256:String,columns:Columns,prefix_len:usize}
pub(crate) struct TargetProfile {objects:Vec<String>,schema_sha256:String,rules:Vec<Binding>,allowances:Allowances,data_rules:Vec<DataRule>}
pub(crate) struct VerifiedSource {profile_id:String,profile_sha256:String}
impl VerifiedSource {pub(crate) fn profile_id(&self)->&str{&self.profile_id}pub(crate) fn profile_sha256(&self)->&str{&self.profile_sha256}}
impl SourceProfile {
    pub(crate) fn identity(&self)->(&str,&str){(&self.id,&self.sha256)}
    pub(crate) fn difference_detail(&self,actual:&[String])->String {
        let at=actual.iter().zip(&self.objects).position(|(a,b)|a!=b).unwrap_or(actual.len().min(self.objects.len()));
        actual.get(at).or_else(||self.objects.get(at)).and_then(|s|Object::parse(s).ok()).map(|o|o.detail()).unwrap_or_else(||"schema object".into())
    }
}
fn capture_tool(value:Option<&Value>)->bool {
    value.and_then(Value::as_str).and_then(|s|s.split_once('@')).is_some_and(|(path,sha)|path.starts_with("agent/scripts/profile-capture/")&&path.ends_with(".py")&&sha.len()==40&&sha.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)))
}
fn image_digest(value:Option<&Value>)->bool {value.and_then(Value::as_str).and_then(|s|s.strip_prefix("sha256:")).is_some_and(hex_hash)}
fn load_source(bytes:&[u8],ledger:&LedgerProfile)->Result<SourceProfile,StepError>{
    let file:SourceFile=serde_json::from_slice(bytes).map_err(|_|fixture_error())?;
    let releases=file.capture.get("releases").and_then(Value::as_array).ok_or_else(fixture_error)?;
    if file.version!=1||file.postgres_major!=15||file.profile_id!=ledger.id||file.ledger_sha256!=ledger.sha256||!file.capture.get("tool").is_some_and(Value::is_string)
        ||releases.len()!=ledger.releases.len()||releases.iter().zip(&ledger.releases).any(|(r,v)|r.get("version").and_then(Value::as_str)!=Some(v.as_str())||!matches!(r.get("method").and_then(Value::as_str),Some("released-image"|"built-from-tag"))){return Err(fixture_error());}
    let registry:Value=serde_json::from_str(include_str!("../db_upgrade/fixtures/released-profiles-v1.json")).map_err(|_|fixture_error())?;
    if !capture_tool(file.capture.get("tool"))||releases.iter().zip(registry["releases"].as_array().ok_or_else(fixture_error)?).any(|(actual,expected)|actual.get("commit")!=expected.get("commit")||!image_digest(actual.get("imageDigest"))||!capture_tool(actual.get("tool"))||!actual.get("serverVersion").and_then(Value::as_str).is_some_and(|s|s.starts_with("15."))){return Err(fixture_error());}
    let chain=&file.capture["chain"];let steps=chain["steps"].as_array().ok_or_else(fixture_error)?;
    if chain["versions"]!=serde_json::to_value(&ledger.releases).map_err(|_|fixture_error())?||!capture_tool(chain.get("tool"))||steps.len()!=releases.len()||steps.iter().zip(releases).any(|(step,release)|step.get("release")!=release.get("version")||["commit","method","imageDigest","serverVersion"].iter().any(|key|step.get(*key)!=release.get(*key))){return Err(fixture_error());}
    let (_,columns)=objects(&file.objects)?;
    Ok(SourceProfile{id:file.profile_id,sha256:hex::encode(Sha256::digest(bytes)),schema_sha256:hash_lines(&file.objects),objects:file.objects,columns,prefix_len:ledger.signatures.len()})
}
pub(crate) fn source_profile(ledger:&LedgerProfile)->Result<SourceProfile,StepError>{load_source(include_bytes!("../db_upgrade/fixtures/profile-legacy-28.json"),ledger)}
fn qualified(value:&str,count:usize)->Result<Vec<String>,StepError>{let v=value.split('.').map(str::to_owned).collect::<Vec<_>>();if v.len()!=count||v.iter().any(|s|s.is_empty()||!s.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_'))||!matches!(v[0].as_str(),"public"|"drizzle"){return Err(fixture_error());}Ok(v)}
fn load_target(bytes:&[u8],candidate:&Candidate,source:&SourceProfile,packaged:&[crate::migrations::Migration])->Result<TargetProfile,StepError>{
    let file:TargetFile=serde_json::from_slice(bytes).map_err(|_|fixture_error())?;
    if super::journal::migrations_sha256(packaged)!=candidate.migrations_sha256{return Err(fixture_error());}
    validate_data_rules(&file.data_rules,source,packaged)?;
    if !hex_hash(&candidate.migrations_sha256)||file.version!=1||file.postgres_major!=17||file.migrations_sha256!=candidate.migrations_sha256||file.profile_id!=format!("target-{}",&candidate.migrations_sha256[..12])||file.capture.get("method").and_then(Value::as_str)!=Some("fresh-packaged-migrations"){return Err(fixture_error());}
    if !capture_tool(file.capture.get("tool"))||!capture_tool(file.capture.get("captureTool"))||!image_digest(file.capture.get("imageDigest"))||!file.capture.get("serverVersion").and_then(Value::as_str).is_some_and(|s|s.starts_with("17.")){return Err(fixture_error());}
    let (parsed,columns)=objects(&file.objects)?;let old=source.objects.iter().collect::<BTreeSet<_>>();let new=file.objects.iter().collect::<BTreeSet<_>>();
    if !old.is_subset(&new){return Err(fixture_error());}
    let delta=new.difference(&old).copied().collect::<BTreeSet<_>>();let covered=file.rules.iter().map(|r|&r.object).collect::<BTreeSet<_>>();
    if covered.len()!=file.rules.len()||covered!=delta{return Err(fixture_error());}
    let mut allowances=Allowances::NONE;
    for (raw,object) in file.objects.iter().zip(&parsed){if old.contains(raw){continue;}match object.kind{Kind::Relation if text(&object.values[2])?=="r"=>{allowances.tables.insert(raw.clone());},Kind::Function=>{allowances.functions.insert(raw.clone());},Kind::Trigger=>{allowances.triggers.insert(raw.clone());},Kind::Sequence=>{allowances.unowned_sequences.insert(raw.clone());},_=>{}}}
    for binding in &file.rules {
        let object=Object::parse(&binding.object)?;let name=object.name()?;
        match &binding.rule {
            Rule::ColumnEquals{..}=>{if object.kind!=Kind::Attribute||!source.columns.contains_key(&name)||source.columns[&name].contains(&text(&object.values[2])?.to_owned()){return Err(fixture_error());}},
            Rule::RowVersion{tables}=>{if object.kind!=Kind::Attribute||text(&object.values[2])?!="row_version"||tables.is_empty(){return Err(fixture_error());}let mut seen=BTreeSet::new();for table in tables{let t=qualified(table,2)?;let key=(t[0].clone(),t[1].clone());if !seen.insert(key.clone())||!columns.get(&key).is_some_and(|c|c.iter().any(|c|c=="row_version")){return Err(fixture_error());}}if !seen.contains(&name){return Err(fixture_error());}},
            Rule::SequenceAtLeastMax{sequence,columns:cols}=>{let q=qualified(sequence,2)?;if name!=(q[0].clone(),q[1].clone())||!(object.kind==Kind::Sequence||object.kind==Kind::Relation&&text(&object.values[2])?=="S")||cols.is_empty(){return Err(fixture_error());}for column in cols{let q=qualified(column,3)?;if !columns.get(&(q[0].clone(),q[1].clone())).is_some_and(|c|c.contains(&q[2])){return Err(fixture_error());}}},
            Rule::TableEmpty=>{if object.kind!=Kind::Relation||text(&object.values[2])?!="r"||source.columns.contains_key(&name){return Err(fixture_error());}},
            Rule::ObjectPresent=>{if object.kind==Kind::Attribute&&source.columns.contains_key(&name)||object.kind==Kind::Sequence||object.kind==Kind::Relation&&text(&object.values[2])?!="i"{return Err(fixture_error());}},
        }
    }
    Ok(TargetProfile{schema_sha256:hash_lines(&file.objects),objects:file.objects,rules:file.rules,allowances,data_rules:file.data_rules})
}
pub(crate) fn target_profile(candidate:&Candidate,source:&SourceProfile,packaged:&[crate::migrations::Migration])->Result<TargetProfile,StepError>{load_target(include_bytes!("../db_upgrade/fixtures/target-profile.json"),candidate,source,packaged)}
pub(crate) fn verify_source(snapshot:&Snapshot,profile:&SourceProfile)->Result<VerifiedSource,StepError>{
    let name=snapshot.application().map_err(|_|verification_error())?;
    if snapshot.data.get(&name).is_none_or(|db|db.schema_sha256!=profile.schema_sha256){return Err(StepError{code:"object_unsupported",message:"저장된 데이터 구조가 출시된 구조와 다릅니다. 원본을 변경하지 않았습니다.",detail:None});}
    Ok(VerifiedSource{profile_id:profile.id.clone(),profile_sha256:profile.sha256.clone()})
}

#[derive(PartialEq,Eq)]
struct Projection {rows:u64,sha256:String,has_encrypted:bool}
pub(crate) struct V1Baseline {catalog:Value,others:BTreeMap<String,Value>,tables:BTreeMap<(String,String),Projection>,expected:BTreeMap<(String,String),Projection>,sequences:BTreeMap<String,String>,has_encrypted:bool,ledger:LedgerState,ledger_sequence:String}
fn table_name(schema:&str,table:&str)->String{format!("{}.{}",inspect::quote(schema),inspect::quote(table))}
fn table_failure(schema:&str,table:&str)->StepError{let mut e=verification_error();e.detail=Some(format!("table {schema}.{table}").chars().filter(|c|!c.is_control()).take(512).collect());e}
async fn projection(client:&Client,name:&(String,String),columns:&[String],key:Option<&SecretBox>)->Result<Projection,StepError>{
    projection_with(client,name,columns,key,&[]).await
}
async fn projection_with(client:&Client,name:&(String,String),columns:&[String],key:Option<&SecretBox>,rules:&[&DataRule])->Result<Projection,StepError>{
    if columns.is_empty(){return Err(fixture_error());}
    let list=columns.iter().map(|column|{
        if let Some(rule)=rules.iter().find(|rule|rule.column==*column){lookup_expression(rule)}else{Ok(format!("t.{}",inspect::quote(column)))}
    }).collect::<Result<Vec<_>,StepError>>()?.join(",");let relation=table_name(&name.0,&name.1);
    let sql=format!("SELECT encoded FROM (SELECT CASE WHEN octet_length(row_to_json(t)::text)>16777216 THEN NULL ELSE row_to_json(t)::text END encoded FROM (SELECT {list} FROM ONLY {relation} t) t) data ORDER BY encoded COLLATE \"C\"");
    let stream=client.query_raw(&sql,std::iter::empty::<&(dyn tokio_postgres::types::ToSql+Sync)>()).await.map_err(|_|table_failure(&name.0,&name.1))?;
    let mut stream=std::pin::pin!(stream);let mut rows=0u64;let mut hash=Sha256::new();let mut has_encrypted=false;
    while let Some(row)=std::future::poll_fn(|cx|stream.as_mut().poll_next(cx)).await {
        let row=row.map_err(|_|table_failure(&name.0,&name.1))?;
        let value:Option<String>=row.try_get(0).map_err(|_|verification_error())?;let value=value.filter(|v|v.len()<=16_777_216).ok_or_else(verification_error)?;
        let json:Value=serde_json::from_str(&value).map_err(|_|verification_error())?;
        has_encrypted|=inspect::check_sealed(&json,key).map_err(|_|verification_error())?;
        hash.update((value.len()as u64).to_be_bytes());hash.update(value.as_bytes());rows=rows.checked_add(1).ok_or_else(verification_error)?;
    }
    Ok(Projection{rows,sha256:hex::encode(hash.finalize()),has_encrypted})
}
fn identity(snapshot:&Snapshot)->Result<(String,Value,BTreeMap<String,Value>),StepError>{
    let application=snapshot.application().map_err(|_|verification_error())?;
    let mut databases=snapshot.databases.clone();for db in &mut databases{if db.name==application{db.name="application".into();}}databases.sort_by(|a,b|a.name.cmp(&b.name));
    let mut roles=snapshot.roles.clone();roles.sort_by(|a,b|a.name.cmp(&b.name));
    let catalog=serde_json::to_value((databases,roles)).map_err(|_|verification_error())?;
    let mut others=BTreeMap::new();
    for db in &snapshot.databases {if db.name!=application{others.insert(db.name.clone(),serde_json::to_value(snapshot.data.get(&db.name).ok_or_else(verification_error)?).map_err(|_|verification_error())?);}}
    if snapshot.data.keys().any(|name|name!=&application&&!others.contains_key(name)){return Err(verification_error());}
    Ok((application,catalog,others))
}
async fn ledger_state(app:&Client,rehearsal:bool)->Result<(LedgerState,String),StepError>{
    let failure=|rule|ledger_error(rehearsal,rule);
    let rows=app.query("SELECT id::bigint,CASE WHEN octet_length(hash)=64 THEN hash ELSE NULL END,created_at FROM ONLY drizzle.__drizzle_migrations ORDER BY id",&[]).await.map_err(|_|failure(3))?;
    let mut result=Vec::new();for row in rows{result.push(LedgerRow{id:row.try_get(0).map_err(|_|failure(3))?,hash:row.try_get::<_,Option<String>>(1).map_err(|_|failure(3))?.ok_or_else(||failure(3))?,created_at:row.try_get(2).map_err(|_|failure(3))?});}
    let owned=app.query("SELECT sn.nspname,s.relname FROM ONLY pg_catalog.pg_depend d JOIN ONLY pg_catalog.pg_class s ON s.oid=d.objid JOIN ONLY pg_catalog.pg_namespace sn ON sn.oid=s.relnamespace JOIN ONLY pg_catalog.pg_attribute a ON a.attrelid=d.refobjid AND a.attnum=d.refobjsubid WHERE d.refobjid='drizzle.__drizzle_migrations'::regclass AND a.attname='id' AND d.deptype IN('a','i') AND s.relkind='S'",&[]).await.map_err(|_|failure(4))?;
    if owned.len()!=1{return Err(failure(4));}let schema:String=owned[0].get(0);let name:String=owned[0].get(1);
    let value=app.query_one(&format!("SELECT last_value::bigint,is_called FROM {}",table_name(&schema,&name)),&[]).await.map_err(|_|failure(4))?;
    Ok((LedgerState{rows:result,last_value:value.try_get(0).map_err(|_|failure(4))?,is_called:value.try_get(1).map_err(|_|failure(4))?},format!("{schema}.{name}")))
}
pub(crate) async fn v1(source:&Snapshot,target:&Snapshot,target_app:&Client,profile:&SourceProfile,target_profile:&TargetProfile,key:Option<&SecretBox>)->Result<V1Baseline,StepError>{
    verify_source(source,profile).map_err(|_|verification_error())?;
    source.require_equivalent(target).map_err(|_|verification_error())?;
    let (name,catalog,others)=identity(source)?;let original=source.data.get(&name).ok_or_else(verification_error)?;
    let mut tables=BTreeMap::new();let mut expected=BTreeMap::new();let mut has_encrypted=false;
    for (name,columns) in &profile.columns{
        let value=projection(target_app,name,columns,key).await?;
        let original=original.tables.get(&format!("{}.{}",name.0,name.1)).ok_or_else(verification_error)?;
        if original.rows!=value.rows||original.sha256!=value.sha256{return Err(table_failure(&name.0,&name.1));}
        let rules=target_profile.data_rules.iter().filter(|rule|rule.table==format!("{}.{}",name.0,name.1)).collect::<Vec<_>>();
        if !rules.is_empty(){expected.insert(name.clone(),projection_with(target_app,name,columns,key,&rules).await?);}
        has_encrypted|=value.has_encrypted;tables.insert(name.clone(),value);
    }
    if has_encrypted!=original.has_encrypted{return Err(verification_error());}
    let (ledger,ledger_sequence)=ledger_state(target_app,false).await?;
    Ok(V1Baseline{catalog,others,tables,expected,sequences:original.sequences.clone(),has_encrypted,ledger,ledger_sequence})
}
pub(crate) async fn v2_rehearsal(app:&Client,target:&TargetProfile,ledger:&[(String,i64)])->Result<(),StepError>{
    let lines=inspect::schema_lines(app).await.map_err(|_|rehearsal_error())?;
    if lines!=target.objects{return Err(rehearsal_error());}
    inspect::snapshot_database_with(app,None,true,&target.allowances).await.map_err(|_|rehearsal_error())?;
    let (after,_)=ledger_state(app,true).await?;let profile=crate::db_upgrade::profiles::released().map_err(|_|rehearsal_error())?;
    let prefix=profile.signatures.iter().map(|r|(r.hash.clone(),r.created_at)).collect::<BTreeSet<_>>();
    let rows=after.rows.iter().filter(|r|prefix.contains(&(r.hash.clone(),r.created_at))).cloned().collect::<Vec<_>>();
    if rows.len()!=prefix.len(){return Err(ledger_error(true,3));}
    let seeded=LedgerState{rows,last_value:0,is_called:false};check_ledger(&seeded,&after,&profile,ledger,true)
}
async fn run_rules(app:&Client,target:&TargetProfile)->Result<(),StepError>{
    let mut groups=BTreeSet::new();let mut sequences=BTreeSet::new();
    for binding in &target.rules {
        let object=Object::parse(&binding.object)?;let (schema,table)=object.name()?;let relation=table_name(&schema,&table);
        let invalid=match &binding.rule{
            Rule::ColumnEquals{value}=>{let column=inspect::quote(text(&object.values[2])?);app.query_one(&format!("SELECT EXISTS(SELECT 1 FROM ONLY {relation} WHERE COALESCE(to_jsonb({column}),'null'::jsonb) IS DISTINCT FROM $1::jsonb)"),&[value]).await.map_err(|_|table_failure(&schema,&table))?.get(0)},
            Rule::RowVersion{tables}=>{
                if !groups.insert(tables.clone()){continue;}
                let mut selects=Vec::new();for table in tables{let q=qualified(table,2)?;selects.push(format!("SELECT row_version AS value FROM ONLY {}",table_name(&q[0],&q[1])));}
                app.query_one(&format!("WITH versions AS ({}) SELECT EXISTS(SELECT 1 FROM versions WHERE value IS NULL) OR EXISTS(SELECT 1 FROM versions GROUP BY value HAVING count(*)>1)",selects.join(" UNION ALL ")),&[]).await.map_err(|_|verification_error())?.get(0)
            },
            Rule::SequenceAtLeastMax{sequence,columns}=>{
                if !sequences.insert((sequence.clone(),columns.clone())){continue;}
                let q=qualified(sequence,2)?;let mut selects=Vec::new();for column in columns{let c=qualified(column,3)?;selects.push(format!("SELECT {} AS value FROM ONLY {}",inspect::quote(&c[2]),table_name(&c[0],&c[1])));}
                let maximum:Option<i64>=app.query_one(&format!("SELECT max(value)::bigint FROM ({}) values_at_cutover",selects.join(" UNION ALL ")),&[]).await.map_err(|_|verification_error())?.get(0);
                let state=app.query_one(&format!("SELECT last_value::bigint,is_called FROM {}",table_name(&q[0],&q[1])),&[]).await.map_err(|_|verification_error())?;let last:i64=state.get(0);let called:bool=state.get(1);
                maximum.is_some_and(|m|last<m||last==m&&!called)
            },
            Rule::TableEmpty=>app.query_one(&format!("SELECT EXISTS(SELECT 1 FROM ONLY {relation})"),&[]).await.map_err(|_|verification_error())?.get(0),
            Rule::ObjectPresent=>false,
        };
        if invalid{return Err(table_failure(&schema,&table));}
    }
    Ok(())
}
pub(crate) async fn v2(app:&Client,others:&Snapshot,baseline:&V1Baseline,source:&SourceProfile,target:&TargetProfile,ledger:&[(String,i64)],key:Option<&SecretBox>)->Result<(),StepError>{
    let (_,catalog,other_data)=identity(others)?;if catalog!=baseline.catalog||other_data!=baseline.others{return Err(verification_error());}
    let data=inspect::snapshot_database_with(app,key,true,&target.allowances).await.map_err(|_|verification_error())?;
    if data.schema_sha256!=target.schema_sha256||data.has_encrypted!=baseline.has_encrypted{return Err(verification_error());}
    for (name,columns) in &source.columns {if name.0=="drizzle"&&name.1=="__drizzle_migrations"{continue;}let current=projection(app,name,columns,key).await?;if baseline.expected.get(name).or_else(||baseline.tables.get(name))!=Some(&current){return Err(table_failure(&name.0,&name.1));}}
    for (name,value) in &baseline.sequences{if name!=&baseline.ledger_sequence&&data.sequences.get(name)!=Some(value){return Err(verification_error());}}
    let (current,sequence)=ledger_state(app,false).await?;if sequence!=baseline.ledger_sequence{return Err(ledger_error(false,4));}
    let profile=crate::db_upgrade::profiles::released().map_err(|_|verification_error())?;check_ledger(&baseline.ledger,&current,&profile,ledger,false)?;
    run_rules(app,target).await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile()->LedgerProfile {crate::db_upgrade::profiles::released().unwrap()}
    fn fixture(duplicates:bool)->(LedgerState,LedgerState,Vec<(String,i64)>){
        let profile=profile();let mut packaged=profile.signatures.iter().map(|r|(r.hash.clone(),r.created_at)).collect::<Vec<_>>();
        let rows=packaged.iter().enumerate().map(|(i,(hash,created_at))|LedgerRow{id:i as i64+1,hash:hash.clone(),created_at:*created_at}).collect::<Vec<_>>();
        let prefix=profile.signatures.len() as i64;let total:Value=serde_json::from_str(include_str!("../../../../packages/db/drizzle/meta/_journal.json")).unwrap();let suffix=total["entries"].as_array().unwrap().len() as i64-prefix;
        let mut before=LedgerState{rows,last_value:prefix,is_called:true};
        if duplicates{let mut row=before.rows[0].clone();row.id=prefix+1;before.rows.push(row);before.last_value=prefix+1;}
        let mut rows=before.rows.clone();for i in 0..suffix{let entry=(format!("{i:064x}"),packaged.last().unwrap().1+1);rows.push(LedgerRow{id:before.last_value+i+1,hash:entry.0.clone(),created_at:entry.1});packaged.push(entry);}
        let after=LedgerState{rows,last_value:before.last_value+suffix,is_called:true};(before,after,packaged)
    }
    #[test]fn valid_upgrade_passes_the_ledger_exception_instead_of_generic_row_equality(){let (b,a,p)=fixture(false);check_ledger(&b,&a,&profile(),&p,false).unwrap();}
    #[test]fn duplicate_legacy_signatures_are_preserved_with_their_own_ids(){let (b,a,p)=fixture(true);check_ledger(&b,&a,&profile(),&p,false).unwrap();}
    #[test]fn changed_preexisting_id_hash_or_timestamp_is_refused(){for field in ["id","hash","time"]{let (b,mut a,p)=fixture(true);match field{"id"=>a.rows[0].id=900,"hash"=>a.rows[0].hash="f".repeat(64),_=>a.rows[0].created_at+=1};let e=check_ledger(&b,&a,&profile(),&p,false).unwrap_err();assert_eq!(e.code,"verification_failed");assert_eq!(e.detail.as_deref(),Some("ledger rule 1"));}}
    #[test]fn suffix_missing_extra_reordered_or_duplicated_is_refused(){for bad in ["missing","extra","reordered","duplicate","low-id"]{let (b,mut a,p)=fixture(false);match bad{"missing"=>{a.rows.pop();},"extra"=>{let mut row=a.rows.last().unwrap().clone();row.id+=1;row.hash="e".repeat(64);a.rows.push(row)},"reordered"=>{let h=a.rows[b.rows.len()].hash.clone();let time=a.rows[b.rows.len()].created_at;a.rows[b.rows.len()].hash=a.rows[b.rows.len()+1].hash.clone();a.rows[b.rows.len()].created_at=a.rows[b.rows.len()+1].created_at;a.rows[b.rows.len()+1].hash=h;a.rows[b.rows.len()+1].created_at=time},"duplicate"=>{let mut row=a.rows.last().unwrap().clone();row.id+=1;a.rows.push(row)},_=>a.rows[b.rows.len()].id=0};let e=check_ledger(&b,&a,&profile(),&p,false).unwrap_err();assert_eq!(e.code,"verification_failed");assert!(e.detail.unwrap().starts_with("ledger rule "));}}
    #[test]fn ledger_sequence_must_be_called_and_cover_old_value_and_max_id(){for bad in ["uncalled","below-max","regressed"]{let (mut b,mut a,p)=fixture(false);match bad{"uncalled"=>a.is_called=false,"below-max"=>a.last_value=1,_=>{b.last_value=100;a.last_value=99}};let e=check_ledger(&b,&a,&profile(),&p,false).unwrap_err();assert_eq!(e.detail.as_deref(),Some("ledger rule 4"));}}
    #[test]fn rehearsal_checks_exact_suffix_and_set_with_its_own_code(){let (b,a,p)=fixture(false);check_ledger(&b,&a,&profile(),&p,true).unwrap();let mut broken=a;broken.rows.pop();assert_eq!(check_ledger(&b,&broken,&profile(),&p,true).unwrap_err().code,"rehearsal_failed");}
    #[tokio::test]
    #[ignore="capture tool invokes explicitly with owned output path"]
    async fn capture_packaged_identity() {
        if std::env::var_os("NWUI_CAPTURE_IDENTITY_OUTPUT").is_none(){return;}
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let directory=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");
        let migrations=crate::migrations::read_migrations(&directory).await.unwrap();
        let value=serde_json::json!({"migrationsSha256":crate::upgrade::journal::migrations_sha256(&migrations),"packaged":migrations.iter().map(|m|serde_json::json!({"tag":m.tag,"hash":m.hash,"createdAt":m.timestamp})).collect::<Vec<_>>()});
        let path=std::env::var_os("NWUI_CAPTURE_IDENTITY_OUTPUT").expect("owned capture output required");
        let mut f=std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).unwrap();
        serde_json::to_writer_pretty(&mut f,&value).unwrap();f.write_all(b"\n").unwrap();f.sync_all().unwrap();
    }

    fn dml_tags(packaged:&[crate::migrations::Migration],prefix:usize)->BTreeSet<String>{
        packaged[prefix..].iter().filter(|m|m.statements.iter().any(|statement|{
            let uncommented=statement.lines().map(|line|line.split_once("--").map_or(line,|(before,_)|before)).collect::<Vec<_>>().join("\n");
            uncommented.split_whitespace().next().is_some_and(|word|matches!(word.to_ascii_uppercase().as_str(),"UPDATE"|"INSERT"|"DELETE"|"MERGE"|"TRUNCATE"|"COPY"|"DO"|"WITH"|"CALL"|"EXECUTE"))
        })).map(|m|m.tag.clone()).collect()
    }
    #[test]
    fn dml_detector_covers_each_frozen_keyword_after_comments(){
        for keyword in ["update","INSERT","delete","MERGE","truncate","COPY","do","WITH","call","EXECUTE","ALTER","CREATE"]{
            let migration=crate::migrations::Migration{tag:"fixture".into(),timestamp:0,hash:String::new(),statements:vec![format!("-- leading comment\n  {keyword} fixture")]};
            assert_eq!(dml_tags(&[migration],0).contains("fixture"),!matches!(keyword,"ALTER"|"CREATE"));
        }
    }
    #[tokio::test]
    async fn every_suffix_dml_migration_has_exactly_one_closed_data_rule(){
        let directory=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");let packaged=crate::migrations::read_migrations(&directory).await.unwrap();
        let tags=dml_tags(&packaged,profile().signatures.len());let value:Value=serde_json::from_slice(include_bytes!("../db_upgrade/fixtures/target-profile.json")).unwrap();
        let empty=Vec::new();let rules=value.get("dataRules").and_then(Value::as_array).unwrap_or(&empty);
        for tag in &tags{assert_eq!(rules.iter().filter(|r|r["migration"].as_str()==Some(tag.as_str())).count(),1,"DML migration {tag} needs exactly one data rule");}
        for rule in rules{assert!(tags.contains(rule["migration"].as_str().unwrap()),"data rule must name a DML suffix migration");}
        assert_eq!(rules.len(),tags.len());
    }
    fn data_profile()->SourceProfile {source_profile(&profile()).unwrap()}
    fn target_candidate()->Candidate {
        let file:TargetFile=serde_json::from_slice(include_bytes!("../db_upgrade/fixtures/target-profile.json")).unwrap();
        Candidate{revision:"1".repeat(40),pins_sha256:"2".repeat(64),migrations_sha256:file.migrations_sha256}
    }
    #[tokio::test]async fn captured_target_has_exactly_one_closed_rule_per_actual_difference(){
        let directory=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");let packaged=crate::migrations::read_migrations(&directory).await.unwrap();
        let source=data_profile();let target=target_profile(&target_candidate(),&source,&packaged).unwrap();assert_eq!(target.rules.len(),target.objects.iter().filter(|line|!source.objects.contains(line)).count());
        assert_eq!(target.allowances.functions.len(),1);assert_eq!(target.allowances.triggers.len(),3);assert_eq!(target.allowances.unowned_sequences.len(),1);
        let mut missing:Value=serde_json::from_slice(include_bytes!("../db_upgrade/fixtures/target-profile.json")).unwrap();missing["rules"].as_array_mut().unwrap().pop();
        assert_eq!(load_target(&serde_json::to_vec(&missing).unwrap(),&target_candidate(),&source,&packaged).err().unwrap().detail.as_deref(),Some("profile_fixture"));
        let mut extra:Value=serde_json::from_slice(include_bytes!("../db_upgrade/fixtures/target-profile.json")).unwrap();let duplicate=extra["rules"][0].clone();extra["rules"].as_array_mut().unwrap().push(duplicate);assert!(load_target(&serde_json::to_vec(&extra).unwrap(),&target_candidate(),&source,&packaged).is_err());
    }
    #[test]fn source_loader_refuses_incomplete_wrong_or_disordered_capture_fixtures(){
        for bad in ["release","id","ledger","duplicate","order"]{
            let mut value:Value=serde_json::from_slice(include_bytes!("../db_upgrade/fixtures/profile-legacy-28.json")).unwrap();
            match bad{"release"=>{value["capture"]["releases"].as_array_mut().unwrap().pop();},"id"=>value["profileId"]="unknown".into(),"ledger"=>value["ledgerSha256"]="0".repeat(64).into(),"duplicate"=>{let duplicate=value["objects"][0].clone();value["objects"].as_array_mut().unwrap().push(duplicate)},_=>value["objects"].as_array_mut().unwrap().swap(0,1)};
            let error=load_source(&serde_json::to_vec(&value).unwrap(),&profile()).err().unwrap();assert_eq!(error.code,"verification_failed");assert_eq!(error.detail.as_deref(),Some("profile_fixture"));
        }
    }
    #[tokio::test]
    async fn data_rule_identifiers_suffix_membership_and_duplicates_are_closed(){
        let directory=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");let packaged=crate::migrations::read_migrations(&directory).await.unwrap();let source=data_profile();
        let file:Value=serde_json::from_slice(include_bytes!("../db_upgrade/fixtures/target-profile.json")).unwrap();let valid=file["dataRules"].as_array().unwrap();
        let rules:Vec<DataRule>=serde_json::from_value(file["dataRules"].clone()).unwrap();validate_data_rules(&rules,&source,&packaged).unwrap();
        for bad in ["table","column","on","lookup-table","lookup-key","lookup-value","prefix","duplicate","unknown-rule"]{
            let mut value=valid.clone();match bad{
                "table"=>value[0]["table"]="public.audit_logs; DROP TABLE users".into(),"column"=>value[0]["column"]="unknown".into(),"on"=>value[0]["on"]="unknown".into(),
                "lookup-table"=>value[0]["lookup"]["table"]="pg_catalog.pg_authid".into(),"lookup-key"=>value[0]["lookup"]["key"]="unknown".into(),"lookup-value"=>value[0]["lookup"]["value"]="unknown".into(),
                "prefix"=>value[0]["migration"]=packaged[0].tag.clone().into(),"duplicate"=>value.push(value[0].clone()),_=>value[0]["rule"]="arbitrary-sql".into(),
            }
            match serde_json::from_value::<Vec<DataRule>>(Value::Array(value)){
                Ok(rules)=>{let error=validate_data_rules(&rules,&source,&packaged).unwrap_err();assert_eq!(error.detail.as_deref(),Some("profile_fixture"));},Err(_)=>assert_eq!(bad,"unknown-rule"),
            }
        }
    }
    #[tokio::test]async fn captured_target_uses_the_shared_packaged_migration_identity(){
        let directory=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");let migrations=crate::migrations::read_migrations(&directory).await.unwrap();
        let mut candidate=target_candidate();assert_eq!(candidate.migrations_sha256,crate::upgrade::journal::migrations_sha256(&migrations));
        candidate.migrations_sha256="0".repeat(64);assert!(target_profile(&candidate,&data_profile(),&migrations).is_err());
    }
    #[test]
    fn complete_source_profile_provenance_and_byte_exact_digest(){let source=source_profile(&profile()).unwrap();assert_eq!(source.sha256,hex::encode(Sha256::digest(include_bytes!("../db_upgrade/fixtures/profile-legacy-28.json"))));}

    struct NativeData {
        app:crate::db_upgrade::inspect::Connection,
        root:crate::db_upgrade::inspect::Connection,
        pg:crate::db_upgrade::postgres::Private,
        _temp:tempfile::TempDir,
        migrations:Vec<crate::migrations::Migration>,
        key:SecretBox,
    }
    impl NativeData {
        async fn new(seeded:bool)->Self {
            use crate::db_upgrade::postgres as pg;
            use std::{process::Stdio,time::Duration};
            assert_eq!(std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref(),Ok("1"),"explicit isolated native image required");
            let temp=tempfile::tempdir().unwrap();pg::own(temp.path()).unwrap();let data=temp.path().join("data");let work=temp.path().join("work");pg::directory(&data).unwrap();pg::directory(&work).unwrap();
            let mut command=pg::command(17,"initdb").unwrap();command.arg("-D").arg(&data).args(["--encoding=UTF8","--no-locale","--auth-local=trust","--auth-host=reject"]).stdout(Stdio::null()).stderr(Stdio::null());pg::wait(command.spawn().unwrap(),Duration::from_secs(60)).unwrap();
            let pg=pg::Private::start(17,&data,&work).unwrap();let root=inspect::connect(&pg.socket,"postgres","postgres",None).await.unwrap();
            root.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();
            root.client.batch_execute("CREATE ROLE nginx LOGIN PASSWORD 'synthetic-profile-only'").await.unwrap();
            root.client.batch_execute("CREATE DATABASE nginx_web_ui OWNER nginx").await.unwrap();
            let app=inspect::connect(&pg.socket,"nginx_web_ui","postgres",None).await.unwrap();let directory=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");let migrations=crate::migrations::read_migrations(&directory).await.unwrap();
            let value=Self{app,root,pg,_temp:temp,migrations,key:SecretBox::from_key([23;32])};
            value.app.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();
            value.app.client.batch_execute("SET ROLE nginx; SET search_path=public; CREATE SCHEMA drizzle; CREATE TABLE drizzle.__drizzle_migrations(id serial primary key,hash text NOT NULL,created_at bigint)").await.unwrap();
            value.apply(0..profile().signatures.len()).await;
            if seeded {
            let sealed=value.key.seal("synthetic-profile-sealed").unwrap();
            value.app.client.execute("INSERT INTO public.users(id,email,totp_secret) VALUES('u1','profile@example.test',$1)",&[&sealed]).await.unwrap();
            value.app.client.batch_execute("INSERT INTO public.upstreams(id,name,servers) VALUES('up1','native-pool','[{\"server\":\"127.0.0.1:19000\"}]'); INSERT INTO public.nginx_servers(id,server_name,listen,config) VALUES('s1','native.internal',18080,'{\"proxyPass\":\"http://127.0.0.1:19000\"}'); INSERT INTO public.nginx_streams(id,listen,proxy_pass) VALUES('st1',18081,'127.0.0.1:19000')").await.unwrap();
            value.app.client.execute("INSERT INTO public.certificate_jobs(id,certificate_id,kind,mode,payload_enc,actor) VALUES('job1','c1','http','initial',$1,'{}')",&[&sealed]).await.unwrap();
            value.app.client.batch_execute("INSERT INTO public.audit_logs(id,actor_id,actor_label,action) VALUES('a-backfill','u1',NULL,'fixture'),('a-existing','u1','saved name','fixture'),('a-unmatched',NULL,NULL,'fixture'); INSERT INTO public.access_tokens(id,kind,name,token_hash,token_suffix,scopes) VALUES('token1','server','fixture','synthetic-token-hash','1234','[]'); INSERT INTO public.gateway_tokens(id,name,token_hash,role) VALUES('gateway1','fixture','synthetic-gateway-hash','viewer')").await.unwrap();
            value.app.client.execute("INSERT INTO public.nodes(id,name,base_url,token_enc,token_role,instance_id) VALUES('node1','fixture','https://node.invalid',$1,'viewer','synthetic-instance')",&[&sealed]).await.unwrap();
            // Preserve a real duplicate legacy signature with a distinct serial id.
            value.app.client.batch_execute("INSERT INTO drizzle.__drizzle_migrations(hash,created_at) SELECT hash,created_at FROM ONLY drizzle.__drizzle_migrations WHERE id=1; SET search_path=pg_catalog").await.unwrap();
            } else { value.app.client.batch_execute("SET search_path=pg_catalog").await.unwrap(); }
            value
        }
        async fn apply(&self,range:std::ops::Range<usize>){
            self.app.client.batch_execute("SET ROLE nginx; SET search_path=public; BEGIN").await.unwrap();
            for migration in &self.migrations[range]{for statement in &migration.statements{self.app.client.batch_execute(statement).await.unwrap();}self.app.client.execute("INSERT INTO drizzle.__drizzle_migrations(hash,created_at) VALUES($1,$2)",&[&migration.hash,&migration.timestamp]).await.unwrap();}
            self.app.client.batch_execute("COMMIT; RESET ROLE; SET search_path=pg_catalog").await.unwrap();
        }
        async fn others(&self,source:&Snapshot)->Snapshot{
            let mut result:Snapshot=serde_json::from_value(serde_json::to_value(source).unwrap()).unwrap();
            let (databases,roles)=inspect::read_catalog(&self.root.client).await.unwrap();result.databases=databases;result.roles=roles;
            for db in &result.databases {if db.name=="nginx_web_ui"{continue;}if db.name=="template0"{self.root.client.batch_execute("ALTER DATABASE template0 ALLOW_CONNECTIONS true").await.unwrap();}
                let connection=inspect::connect(&self.pg.socket,&db.name,"postgres",None).await.unwrap();let snapshot=inspect::snapshot_database(&connection.client,None,false).await.unwrap();drop(connection);result.data.insert(db.name.clone(),snapshot);
                if db.name=="template0"{self.root.client.batch_execute("ALTER DATABASE template0 ALLOW_CONNECTIONS false").await.unwrap();}
            }result
        }
    }
    #[tokio::test]
    #[ignore="requires explicit network-none native PG17 image; source-shaped data algorithm test"]
    async fn native_v1_v2_preserves_rows_and_refuses_each_unruled_mutation(){
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        let native=NativeData::new(true).await;let source=data_profile();let candidate=target_candidate();let target=target_profile(&candidate,&source,&native.migrations).unwrap();
        let baseline=crate::db_upgrade::cluster::inspect_source(&native.pg.socket,"synthetic-profile-only",Some(&native.key)).await.unwrap();
        assert_eq!(baseline.data["nginx_web_ui"].schema_sha256,source.schema_sha256);
        let proof=verify_source(&baseline,&source).unwrap();assert_eq!(proof.profile_sha256(),source.sha256);assert_eq!(proof.profile_id(),"legacy-28");
        let before=v1(&baseline,&baseline,&native.app.client,&source,&target,Some(&native.key)).await.unwrap();
        assert!(v1(&baseline,&baseline,&native.app.client,&source,&target,Some(&SecretBox::from_key([24;32]))).await.is_err());
        let mut changed:Snapshot=serde_json::from_value(serde_json::to_value(&baseline).unwrap()).unwrap();changed.data.get_mut("nginx_web_ui").unwrap().tables.get_mut("public.users").unwrap().rows+=1;
        assert_eq!(v1(&baseline,&changed,&native.app.client,&source,&target,Some(&native.key)).await.err().unwrap().code,"verification_failed");
        native.apply(profile().signatures.len()..native.migrations.len()).await;let others=native.others(&baseline).await;let ledger=native.migrations.iter().map(|m|(m.hash.clone(),m.timestamp)).collect::<Vec<_>>();
        v2(&native.app.client,&others,&before,&source,&target,&ledger,Some(&native.key)).await.unwrap();
        let mutations=[
            "UPDATE public.users SET email='changed@example.test' WHERE id='u1'",
            "ALTER TABLE public.upstreams DISABLE TRIGGER control_resource_version; UPDATE public.upstreams SET row_version=(SELECT row_version FROM public.nginx_servers WHERE id='s1'); ALTER TABLE public.upstreams ENABLE TRIGGER control_resource_version",
            "UPDATE public.certificate_jobs SET deploy_pending=true WHERE id='job1'",
            "UPDATE public.users SET passkey_user_handle=decode(repeat('01',32),'hex') WHERE id='u1'",
            "INSERT INTO public.user_passkeys(id,user_id,credential_id,public_key,algorithm,rp_id,origin,name,backup_eligible,backup_state) VALUES('unexpected','u1',decode('01','hex'),decode('01','hex'),-7,'localhost','http://localhost','unexpected',false,false)",
            "CREATE FUNCTION public.extra_function() RETURNS int LANGUAGE sql AS 'SELECT 1'",
            "CREATE TRIGGER extra_trigger BEFORE INSERT ON public.nginx_servers FOR EACH ROW EXECUTE FUNCTION public.assign_control_resource_version()",
            "UPDATE drizzle.__drizzle_migrations SET hash=repeat('f',64) WHERE id=1",
            "DELETE FROM drizzle.__drizzle_migrations WHERE id=(SELECT max(id) FROM ONLY drizzle.__drizzle_migrations)",
            "INSERT INTO drizzle.__drizzle_migrations(hash,created_at) SELECT hash,created_at FROM ONLY drizzle.__drizzle_migrations ORDER BY id DESC LIMIT 1",
        ];
        for sql in mutations{native.app.client.batch_execute("BEGIN").await.unwrap();native.app.client.batch_execute(sql).await.unwrap();let error=v2(&native.app.client,&others,&before,&source,&target,&ledger,Some(&native.key)).await.err().expect("mutation must refuse");assert_eq!(error.code,"verification_failed");native.app.client.batch_execute("ROLLBACK").await.unwrap();}
        // setval is not transactional; restore its exact state explicitly after each refusal.
        for sequence in before.sequences.keys().filter(|s|*s!=&before.ledger_sequence) {
            let q=qualified(sequence,2).unwrap();let saved=native.app.client.query_one(&format!("SELECT last_value::bigint,is_called FROM {}",table_name(&q[0],&q[1])),&[]).await.unwrap();let last:i64=saved.get(0);let called:bool=saved.get(1);let changed=last+1;
            native.app.client.query_one("SELECT setval($1::text::regclass,$2,true)",&[&sequence,&changed]).await.unwrap();
            assert_eq!(v2(&native.app.client,&others,&before,&source,&target,&ledger,Some(&native.key)).await.err().unwrap().code,"verification_failed");
            native.app.client.query_one("SELECT setval($1::text::regclass,$2,$3)",&[&sequence,&last,&called]).await.unwrap();
        }
        for sequence in ["public.control_resource_version_seq","drizzle.__drizzle_migrations_id_seq"]{
            let q=qualified(sequence,2).unwrap();let saved=native.app.client.query_one(&format!("SELECT last_value::bigint,is_called FROM {}",table_name(&q[0],&q[1])),&[]).await.unwrap();let last:i64=saved.get(0);let called:bool=saved.get(1);
            native.app.client.query_one("SELECT setval($1::text::regclass,1,true)",&[&sequence]).await.unwrap();assert_eq!(v2(&native.app.client,&others,&before,&source,&target,&ledger,Some(&native.key)).await.err().unwrap().code,"verification_failed");
            native.app.client.query_one("SELECT setval($1::text::regclass,$2,$3)",&[&sequence,&last,&called]).await.unwrap();
        }
        v2(&native.app.client,&others,&before,&source,&target,&ledger,Some(&native.key)).await.unwrap();
    }


    #[tokio::test]
    #[ignore="requires explicit network-none native PG17 image"]
    async fn native_rehearsal_and_fresh_target_drift_are_checked() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        let native=NativeData::new(false).await; native.apply(profile().signatures.len()..native.migrations.len()).await;
        let source=data_profile();let target=target_profile(&target_candidate(),&source,&native.migrations).unwrap();
        assert_eq!(inspect::schema_lines(&native.app.client).await.unwrap(),target.objects,"fresh packaged migrations must regenerate exact object bytes");
        let ledger=native.migrations.iter().map(|m|(m.hash.clone(),m.timestamp)).collect::<Vec<_>>();
        v2_rehearsal(&native.app.client,&target,&ledger).await.unwrap();
        for sql in ["DELETE FROM drizzle.__drizzle_migrations WHERE id=9", "INSERT INTO drizzle.__drizzle_migrations(hash,created_at) VALUES(repeat('e',64),0)"] {
            native.app.client.batch_execute("BEGIN").await.unwrap();native.app.client.batch_execute(sql).await.unwrap();
            assert_eq!(v2_rehearsal(&native.app.client,&target,&ledger).await.unwrap_err().code,"rehearsal_failed");
            native.app.client.batch_execute("ROLLBACK").await.unwrap();
        }
    }
    #[tokio::test]
    #[ignore="requires explicit network-none native PG17 image"]
    async fn native_backfill_preserves_nonnull_and_unmatched_labels_and_refuses_drift(){
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        let native=NativeData::new(true).await;let source=data_profile();let target=target_profile(&target_candidate(),&source,&native.migrations).unwrap();
        let original=crate::db_upgrade::cluster::inspect_source(&native.pg.socket,"synthetic-profile-only",Some(&native.key)).await.unwrap();
        let before=v1(&original,&original,&native.app.client,&source,&target,Some(&native.key)).await.unwrap();
        native.apply(profile().signatures.len()..native.migrations.len()).await;let others=native.others(&original).await;let ledger=native.migrations.iter().map(|m|(m.hash.clone(),m.timestamp)).collect::<Vec<_>>();
        v2(&native.app.client,&others,&before,&source,&target,&ledger,Some(&native.key)).await.unwrap();
        let row=native.app.client.query_one("SELECT actor_label FROM ONLY public.audit_logs WHERE id='a-backfill'",&[]).await.unwrap();assert_eq!(row.get::<_,String>(0),"profile@example.test");
        assert!(native.app.client.query_one("SELECT actor_label IS NULL FROM ONLY public.audit_logs WHERE id='a-unmatched'",&[]).await.unwrap().get::<_,bool>(0));
        for sql in ["UPDATE public.audit_logs SET actor_label='wrong' WHERE id='a-existing'","UPDATE public.audit_logs SET action='changed' WHERE id='a-backfill'","UPDATE public.audit_logs SET actor_label='wrong-email@example.test' WHERE id='a-backfill'"]{
            native.app.client.batch_execute("BEGIN").await.unwrap();native.app.client.batch_execute(sql).await.unwrap();
            let error=v2(&native.app.client,&others,&before,&source,&target,&ledger,Some(&native.key)).await.unwrap_err();assert_eq!(error.code,"verification_failed");assert_eq!(error.detail.as_deref(),Some("table public.audit_logs"));native.app.client.batch_execute("ROLLBACK").await.unwrap();
        }
        v2(&native.app.client,&others,&before,&source,&target,&ledger,Some(&native.key)).await.unwrap();
    }
    #[tokio::test]
    #[ignore="requires explicit network-none native PG17 image"]
    async fn native_source_extra_index_column_function_and_trigger_are_refused() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        let native=NativeData::new(false).await;let source=data_profile();
        let baseline=crate::db_upgrade::cluster::inspect_source(&native.pg.socket,"synthetic-profile-only",None).await.unwrap();
        for sql in ["CREATE INDEX extra_index ON public.users(email)","ALTER TABLE public.users ADD COLUMN extra_column text", "CREATE FUNCTION public.extra_function() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN RETURN NEW; END;$$", "CREATE FUNCTION public.extra_function() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN RETURN NEW; END;$$; CREATE TRIGGER extra_trigger BEFORE INSERT ON public.users FOR EACH ROW EXECUTE FUNCTION public.extra_function()"] {
            native.app.client.batch_execute("BEGIN").await.unwrap();native.app.client.batch_execute(sql).await.unwrap();
            let lines=inspect::schema_lines(&native.app.client).await.unwrap();
            let mut actual:Snapshot=serde_json::from_value(serde_json::to_value(&baseline).unwrap()).unwrap();actual.data.get_mut("nginx_web_ui").unwrap().schema_sha256=hash_lines(&lines);
            assert_eq!(verify_source(&actual,&source).err().unwrap().code,"object_unsupported");
            let detail=source.difference_detail(&lines);assert!(detail.len()<=512);assert!(!detail.contains("BEGIN RETURN"));
            native.app.client.batch_execute("ROLLBACK").await.unwrap();
        }
    }
}
