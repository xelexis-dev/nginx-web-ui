//! LC_ALL=C pg_controldata output; warnings never authorize conversion.
use serde::{Serialize,Deserialize};
use super::{error::{Error,Result},state};
use std::collections::BTreeMap;

pub const COMPATIBILITY_FIELDS:[&str;12]=[
    "Maximum data alignment","Database block size","Blocks per segment of large relation",
    "WAL block size","Bytes per WAL segment","Maximum length of identifiers",
    "Maximum columns in an index","Maximum size of a TOAST chunk","Size of a large-object chunk",
    "Date/time type storage","Float8 argument passing","Data page checksum version",
];
#[derive(Debug,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Control {
    pub system_identifier:String,
    pub cluster_state:String,
    compatibility:BTreeMap<String,String>,
}
impl Control {
    pub fn setting(&self,label:&str)->Result<&str>{self.compatibility.get(label).map(String::as_str).ok_or_else(||Error::refusal("unsupported_layout"))}
    pub fn validate(&self)->Result<()> {
        if !state::identifier(&self.system_identifier)||self.cluster_state.is_empty()||self.compatibility.len()!=COMPATIBILITY_FIELDS.len()||COMPATIBILITY_FIELDS.iter().any(|field|self.compatibility.get(*field).is_none_or(|v|v.is_empty()||v.len()>128)){return Err(Error::refusal("unsupported_layout"));}Ok(())
    }
    pub fn parse(stdout:&str,stderr:&str)->Result<Self> {
        let refuse=||Error::refusal("unsupported_layout");
        if !stderr.trim().is_empty() || stdout.len()>65536 || stdout.contains('\0'){return Err(refuse());}
        let mut fields=BTreeMap::new();
        for line in stdout.lines() {
            let Some((label,value))=line.split_once(':') else {continue;};
            let label=label.trim();let value=value.trim();
            if !COMPATIBILITY_FIELDS.contains(&label) && !["Database system identifier","Database cluster state"].contains(&label){continue;}
            if value.is_empty() || fields.insert(label.to_owned(),value.to_owned()).is_some(){return Err(refuse());}
        }
        let system_identifier=fields.remove("Database system identifier").ok_or_else(refuse)?;
        if !state::identifier(&system_identifier){return Err(refuse());}
        let cluster_state=fields.remove("Database cluster state").ok_or_else(refuse)?;
        if COMPATIBILITY_FIELDS.iter().any(|field|!fields.contains_key(*field)){return Err(refuse());}
        Ok(Self{system_identifier,cluster_state,compatibility:fields})
    }
    pub fn compatible_with(&self,target:&Self)->Result<()> {
        if self.compatibility!=target.compatibility{return Err(Error::refusal("unsupported_layout"));}Ok(())
    }
    pub fn require_cold(&self)->Result<()> {
        if self.cluster_state!="shut down"{return Err(Error::refusal("unsupported_layout"));}Ok(())
    }
}
