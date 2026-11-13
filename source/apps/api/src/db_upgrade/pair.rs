use serde::{Serialize,Deserialize};
use super::{state,error::{Error,Result}};
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Pair {
    pub schema_version:u8,pub pair_id:String,pub mode:String,pub ready:bool,pub major:u8,pub architecture:String,
    #[serde(deserialize_with="Option::deserialize")]pub system_identifier:Option<String>,
    pub encryption_key_required:bool,
    #[serde(deserialize_with="Option::deserialize")]pub fresh_step:Option<String>,
}
impl Pair {
    pub fn fresh(architecture:&str)->Self { Self{schema_version:1,pair_id:uuid::Uuid::new_v4().to_string(),mode:"fresh".into(),ready:false,major:17,architecture:architecture.into(),system_identifier:None,encryption_key_required:false,fresh_step:Some("secrets".into())} }
    pub fn validate(&self)->Result<()> {
        let invalid=||Error::refusal("pair_mismatch");
        if self.schema_version!=1 || !state::uuid(&self.pair_id) || self.major!=17 || !state::architecture(&self.architecture) || self.system_identifier.as_ref().is_some_and(|id|!state::identifier(id)) { return Err(invalid()); }
        match self.mode.as_str() {
            "migration" if self.system_identifier.is_some() && self.fresh_step.is_none()=>{},
            "fresh" if self.ready && self.system_identifier.is_some() && self.fresh_step.is_none()=>{},
            "fresh" if !self.ready && !self.encryption_key_required =>match self.fresh_step.as_deref() {
                Some("secrets"|"initdb")=>{},
                Some("role"|"database"|"verify") if self.system_identifier.is_some()=>{},
                _=>return Err(invalid()),
            },
            _=>return Err(invalid()),
        }
        Ok(())
    }
}
