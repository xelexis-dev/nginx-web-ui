use super::{state,error::{Error,Result}};
use serde::{Serialize,Deserialize};
use std::collections::BTreeSet;
#[derive(Debug,Clone,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Database {
    pub name:String,pub owner:String,pub encoding:String,pub collate:String,pub ctype:String,pub provider:String,pub icu:Option<String>,
}
#[derive(Debug,Clone,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Role {
    pub name:String,pub oid:String,pub login:bool,pub elevated:bool,pub password_sha256:Option<String>,
}
pub fn validate(databases:&[Database],roles:&[Role],fresh:bool)->Result<Option<String>> {
    let bad=||Error::refusal("unsupported_layout");let mut names=BTreeSet::new();let mut application=None;
    for db in databases {
        if !names.insert(db.name.as_str()) || db.encoding.is_empty() || db.collate.is_empty() || db.ctype.is_empty() || !matches!(db.provider.as_str(),"c"|"i") || (db.provider=="i"&&db.icu.as_deref().is_none_or(str::is_empty)) {return Err(bad());}
        if [&db.encoding,&db.collate,&db.ctype].into_iter().any(|s|s.chars().any(char::is_control)) || db.icu.as_ref().is_some_and(|s|s.chars().any(char::is_control)){return Err(bad());}
        match db.name.as_str() {
            "postgres"|"template0"|"template1" if db.owner=="postgres"=>{},
            "nginx_web_ui"|"nginx_control" if db.owner=="nginx" && application.is_none() && (!fresh||db.name=="nginx_web_ui")=>application=Some(db.name.clone()),
            _=>return Err(bad()),
        }
    }
    if !["postgres","template0","template1"].into_iter().all(|name|names.contains(name)){return Err(bad());}
    let mut names=BTreeSet::new();let mut nginx=false;
    for role in roles {
        if !names.insert(role.name.as_str()) || !state::identifier(&role.oid) || role.password_sha256.as_ref().is_some_and(|s|!state::sha256(s)){return Err(bad());}
        match role.name.as_str() {
            "postgres" if role.login&&role.elevated=>{},
            "nginx" if role.login&&!role.elevated&&role.password_sha256.is_some()=>nginx=true,
            _=>return Err(bad()),
        }
    }
    if !names.contains("postgres") || (application.is_some()&&!nginx) || (!fresh&&(application.is_none()||!nginx)){return Err(bad());}
    Ok(application)
}
