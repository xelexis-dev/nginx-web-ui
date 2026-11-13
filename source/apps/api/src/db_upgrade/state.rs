use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

type Result<T> = std::result::Result<T, &'static str>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase { NotStarted, Validated, Copied, Checked, Converted, Verified, Ready }
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Operation { Idle, Running, Failed }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    pub major: u8,
    pub architecture: String,
    pub system_identifier: String,
    pub manifest_sha256: String,
    pub backup_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Target {
    pub major: u8,
    pub architecture: String,
    #[serde(deserialize_with = "Option::deserialize")] pub system_identifier: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Destination { Volume { source: String }, Bind { source: String } }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Destinations { pub db: Destination, pub app: Destination }
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Checks {
    pub source_unchanged: bool, pub role_password: bool, pub encrypted_data: bool,
    pub logical_data: bool, pub collations: bool, pub clean_shutdown: bool, pub pair_agreement: bool,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Summary {
    #[serde(deserialize_with = "Option::deserialize")] pub database_count: Option<u64>, #[serde(deserialize_with = "Option::deserialize")] pub table_counts: Option<BTreeMap<String, u64>>,
    #[serde(deserialize_with = "Option::deserialize")] pub source_db_bytes: Option<u64>, #[serde(deserialize_with = "Option::deserialize")] pub source_app_bytes: Option<u64>,
    #[serde(deserialize_with = "Option::deserialize")] pub largest_index_bytes: Option<u64>, #[serde(deserialize_with = "Option::deserialize")] pub required_bytes: Option<u64>, #[serde(deserialize_with = "Option::deserialize")] pub available_bytes: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LastError { pub code: String, pub message: String }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct State {
    pub schema_version: u8, #[serde(deserialize_with = "Option::deserialize")] pub attempt_id: Option<String>, #[serde(deserialize_with = "Option::deserialize")] pub pair_id: Option<String>,
    pub mode: String, pub phase: Phase, pub operation: Operation, #[serde(deserialize_with = "Option::deserialize")] pub next_phase: Option<Phase>,
    pub ready: bool, #[serde(deserialize_with = "Option::deserialize")] pub source: Option<Source>, pub target: Target,
    #[serde(deserialize_with = "Option::deserialize")] pub inputs_sha256: Option<String>, #[serde(deserialize_with = "Option::deserialize")] pub destinations: Option<Destinations>,
    pub checks: Checks, pub summary: Summary, #[serde(deserialize_with = "Option::deserialize")] pub last_error: Option<LastError>,
}
impl State {
    pub fn not_started(architecture: &str) -> Self {
        Self { schema_version: 1, attempt_id: None, pair_id: None, mode: "migration".into(),
            phase: Phase::NotStarted, operation: Operation::Idle, next_phase: None, ready: false,
            source: None, target: Target { major: 17, architecture: architecture.into(), system_identifier: None },
            inputs_sha256: None, destinations: None, checks: Checks::default(), summary: Summary::default(), last_error: None }
    }
    pub fn begin(&mut self, phase: Phase) -> Result<()> {
        self.validate()?;
        if self.operation != Operation::Idle || self.phase.next() != Some(phase) { return Err("invalid_transition"); }
        self.operation=Operation::Running;self.next_phase=Some(phase);self.ready=false;
        Ok(())
    }
    pub fn complete(&mut self, phase: Phase) -> Result<()> {
        if self.operation!=Operation::Running || self.next_phase!=Some(phase) || self.phase.next()!=Some(phase) { return Err("invalid_transition"); }
        let mut completed=self.clone();completed.phase=phase;completed.operation=Operation::Idle;
        completed.next_phase=None;completed.ready=phase==Phase::Ready;completed.last_error=None;
        completed.validate()?;*self=completed;Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema_version!=1 || !matches!(self.mode.as_str(),"migration"|"fresh") || self.target.major!=17 || !architecture(&self.target.architecture) { return Err("invalid_state"); }
        for id in [&self.attempt_id,&self.pair_id].into_iter().flatten() { if !uuid(id) { return Err("invalid_state"); } }
        if self.target.system_identifier.as_ref().is_some_and(|id|!identifier(id)) || self.inputs_sha256.as_ref().is_some_and(|hash|!sha256(hash)) { return Err("invalid_state"); }
        if let Some(source)=&self.source {
            if source.major!=15 || (source.architecture!=self.target.architecture && source.architecture!="unrecorded") || !identifier(&source.system_identifier) || !sha256(&source.manifest_sha256) || !backup_id(&source.backup_id) { return Err("invalid_state"); }
        }
        if let Some(destinations)=&self.destinations { destinations.db.validate()?;destinations.app.validate()?; }
        if self.mode=="fresh" && self.source.is_some() { return Err("invalid_state"); }
        if self.phase!=Phase::NotStarted && (self.attempt_id.is_none() || self.pair_id.is_none() || self.inputs_sha256.is_none() || self.destinations.is_none() || (self.mode=="migration" && self.source.is_none())) { return Err("invalid_state"); }
        match self.operation {
            Operation::Running if self.next_phase!=self.phase.next() || self.next_phase.is_none() || self.ready || self.last_error.is_some() => return Err("invalid_state"),
            Operation::Failed if self.next_phase.is_some() || self.ready || self.last_error.is_none() => return Err("invalid_state"),
            Operation::Idle if self.next_phase.is_some() || self.last_error.is_some() => return Err("invalid_state"),
            _ => {},
        }
        if let Some(error)=&self.last_error {
            if error.code.is_empty() || error.code.len()>64 || !error.code.bytes().next().is_some_and(|c|c.is_ascii_lowercase()) || !error.code.bytes().all(|c|c.is_ascii_lowercase()||c.is_ascii_digit()||c==b'_') || error.message.is_empty() || error.message.chars().count()>512 || error.message.chars().any(char::is_control) { return Err("invalid_state"); }
        }
        if self.ready != (self.phase==Phase::Ready && self.operation==Operation::Idle) { return Err("invalid_state"); }
        if self.ready && (self.target.system_identifier.is_none() || !self.checks.all() || self.attempt_id.is_none() || self.pair_id.is_none() || self.inputs_sha256.is_none() || self.destinations.is_none()) { return Err("invalid_state"); }
        Ok(())
    }
    pub fn can_continue(&self) -> bool {
        self.validate().is_ok() && self.mode=="migration" && self.operation==Operation::Idle && matches!(self.phase,Phase::Checked|Phase::Ready)
    }
}

impl Phase {
    pub fn next(self)->Option<Self> { match self { Self::NotStarted=>Some(Self::Validated),Self::Validated=>Some(Self::Copied),Self::Copied=>Some(Self::Checked),Self::Checked=>Some(Self::Converted),Self::Converted=>Some(Self::Verified),Self::Verified=>Some(Self::Ready),Self::Ready=>None } }
}
impl Checks {
    pub fn all(&self)->bool { self.source_unchanged && self.role_password && self.encrypted_data && self.logical_data && self.collations && self.clean_shutdown && self.pair_agreement }
}
impl Destination {
    pub fn validate(&self)->Result<()> {
        match self {
            Self::Volume{source} if !source.is_empty() && source.len()<=255 && source.bytes().next().is_some_and(|c|c.is_ascii_alphanumeric()) && source.bytes().all(|c|c.is_ascii_alphanumeric()||b"_.-".contains(&c)) => Ok(()),
            Self::Bind{source} if source.len()>1 && source.len()<=4096 && source.starts_with('/') && !source.chars().any(char::is_control) && !source.split('/').skip(1).any(|c|c.is_empty()||c=="."||c=="..") => Ok(()),
            _=>Err("invalid_destination"),
        }
    }
}
pub fn architecture(value:&str)->bool { matches!(value,"arm64"|"amd64") }
pub fn identifier(value:&str)->bool { !value.starts_with('0') && !value.is_empty() && value.len()<=20 && value.bytes().all(|c|c.is_ascii_digit()) && value.parse::<u64>().is_ok() }
pub fn sha256(value:&str)->bool { value.len()==64 && value.bytes().all(|c|c.is_ascii_digit()||(b'a'..=b'f').contains(&c)) }
pub fn uuid(value:&str)->bool { uuid::Uuid::parse_str(value).is_ok_and(|id|id.get_version_num()==4 && id.to_string()==value) }
pub fn backup_id(value:&str)->bool { !value.is_empty() && value.len()<=128 && value.bytes().next().is_some_and(|c|c.is_ascii_alphanumeric()) && value.bytes().all(|c|c.is_ascii_alphanumeric()||b"_.:-".contains(&c)) }
