//! P0b reads only port settings; AUTH_SECRET and the fresh DB password do not exist yet.
use super::StepError;

#[derive(Debug,PartialEq,Eq)]
pub(crate) struct StartupPorts {pub api:u16,pub web:u16,pub db:u16}
fn invalid(detail:impl ToString)->StepError {
    StepError {code:"api_port_conflict",message:"API 포트 환경 설정을 확인하세요.",detail:Some(detail.to_string().chars().take(512).collect())}
}
/// The runner passes these values to N2 preflight_ports, which owns listener-conflict checks.
pub(crate) fn startup_ports()->Result<StartupPorts,StepError> {
    checked_ports(crate::nonroot::startup_value)
}
fn checked_ports(get:impl Fn(&str)->Option<String>)->Result<StartupPorts,StepError> {
    let ports=read_ports(get)?;
    crate::runtime_boot::preflight_ports(ports.api,ports.web,ports.db)?;
    Ok(ports)
}
fn read_ports(get:impl Fn(&str)->Option<String>)->Result<StartupPorts,StepError> {
    let port=|name,default:&str| get(name).as_deref().unwrap_or(default).parse::<u16>().map_err(invalid);
    let api=port("API_PORT","3001")?;let web=port("PORT","3000")?;
    if web==0 {return Err(invalid("PORT must be nonzero"));}
    if api==0 || api==web {return Err(invalid("API_PORT must be nonzero and separate from the web port"));}
    let db=match get("DATABASE_URL") {
        Some(value)=>url::Url::parse(&value).map_err(invalid)?.port().unwrap_or(5432),
        None=>5432,
    };
    Ok(StartupPorts {api,web,db})
}

#[cfg(test)] mod tests {use super::*; #[test] fn startup_port_collisions_are_refused(){checked_ports(|_|None).unwrap();for port in [80,443,8080,5432]{assert_eq!(checked_ports(|name|(name=="API_PORT").then(||port.to_string())).unwrap_err().code,"api_port_conflict");}}}

#[cfg(test)]
mod restored_tests {
use super::*;
    #[test]
    fn w1_p0b_calls_the_shared_collision_core_before_runner_mutation() {
        checked_ports(|_|None).unwrap();
        for port in [80,443,8080,5432] {
            let error=checked_ports(|name|(name=="API_PORT").then(||port.to_string())).unwrap_err();
            assert_eq!(error.code,"api_port_conflict");
            assert_eq!(error.detail,crate::runtime_boot::preflight_ports(port,3000,5432).unwrap_err().detail);
        }
        assert!(checked_ports(|name|match name {"API_PORT"=>Some("5544".into()),"DATABASE_URL"=>Some("postgres://nginx@localhost:5544/fixture".into()),_=>None}).is_err());
    }
    #[test]
    fn w1_p0b_ports_need_no_auth_or_database_secrets() {
        let values=read_ports(|name|match name {"API_PORT"|"PORT"|"DATABASE_URL"=>None,_=>panic!("P0b read an unrelated setting")}).unwrap();
        assert_eq!(values,StartupPorts {api:3001,web:3000,db:5432});
    }
    #[test]
    fn w1_p0b_port_errors_match_runtime_config_before_mutation() {
        for (key,value) in [("API_PORT","0"),("API_PORT","3000"),("API_PORT",""),("API_PORT","bad"),("API_PORT","65536"),("API_PORT","-1"),("API_PORT"," 3001"),("PORT","0"),("PORT","bad"),("PORT","65536"),("PORT","3001")] {
            let get=|name:&str|if name==key {Some(value.to_owned())} else {None};
            let expected=crate::config::RuntimeConfig::read(get).err().expect("invalid ports must fail before missing secrets").to_string();
            let error=read_ports(get).unwrap_err();assert_eq!(error.code,"api_port_conflict");assert_eq!(error.detail.as_deref(),Some(expected.as_str()),"{key}");
        }
    }
    #[test]
    fn w1_p0b_database_url_is_parsed_only_when_set_and_never_logged() {
        let explicit=read_ports(|name|match name {"API_PORT"=>Some("+3011".into()),"PORT"=>Some("3010".into()),"DATABASE_URL"=>Some("postgres://nginx:fixture-secret@127.0.0.1:5440/fixture".into()),_=>None}).unwrap();
        assert_eq!(explicit,StartupPorts {api:3011,web:3010,db:5440});
        for value in ["", "not a URL with fixture-secret", "postgres://nginx:fixture-secret@[invalid"] {
            let error=read_ports(|name|(name=="DATABASE_URL").then(||value.to_owned())).unwrap_err();
            assert_eq!(error.code,"api_port_conflict");assert!(!format!("{error:?}").contains("fixture-secret"));
        }
    }
}
