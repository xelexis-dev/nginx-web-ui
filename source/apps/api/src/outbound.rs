//! One address policy for operator-configured HTTP and SMTP destinations.
use std::{io,net::{IpAddr,SocketAddr},time::Duration};
use reqwest::dns::{Addrs,Name,Resolve,Resolving};
use crate::config::RuntimeConfig;

fn check(ip:IpAddr,allow_loopback:bool)->io::Result<()> {
    if crate::dns_guard::policy::unsafe_address(ip) && !(allow_loopback && ip.is_loopback()) {
        Err(io::Error::new(io::ErrorKind::PermissionDenied,"unsafe outbound address"))
    } else {Ok(())}
}
fn check_url(url:&url::Url,allow_loopback:bool)->io::Result<()> {
    match url.host() {
        Some(url::Host::Ipv4(ip))=>check(ip.into(),allow_loopback),
        Some(url::Host::Ipv6(ip))=>check(ip.into(),allow_loopback),
        Some(url::Host::Domain(_))=>Ok(()),
        None=>Err(io::Error::new(io::ErrorKind::InvalidInput,"missing outbound host")),
    }
}
// Context-free form parsing permits local peers; the transport applies the actual internal ports.
pub(crate) fn validate_url(url:&url::Url)->io::Result<()> {check_url(url,true)}
fn internal_port(config:&RuntimeConfig,port:u16)->bool {
    crate::control::internal_targets::internal_ports(config).iter().any(|(p,_)|*p==port)
}
fn checked(addresses:Vec<SocketAddr>,allow_loopback:bool)->io::Result<Vec<SocketAddr>> {
    if addresses.is_empty(){return Err(io::Error::new(io::ErrorKind::NotFound,"no outbound addresses"));}
    for address in &addresses {check(address.ip(),allow_loopback)?;}
    Ok(addresses)
}
async fn resolve(host:&str,port:u16,allow_loopback:bool)->io::Result<Vec<SocketAddr>> {
    let host=host.strip_prefix('[').and_then(|s|s.strip_suffix(']')).unwrap_or(host);
    if let Ok(ip)=host.parse::<IpAddr>() {return checked(vec![SocketAddr::new(ip,port)],allow_loopback);}
    let addresses=tokio::time::timeout(Duration::from_secs(10),tokio::net::lookup_host((host,port))).await
        .map_err(|_|io::Error::new(io::ErrorKind::TimedOut,"outbound DNS timeout"))??;
    checked(addresses.collect(),allow_loopback)
}
pub(crate) async fn resolve_host(host:&str,port:u16,config:&RuntimeConfig)->io::Result<Vec<SocketAddr>> {
    resolve(host,port,!internal_port(config,port)).await
}
struct SafeResolver {allow_loopback:bool}
impl Resolve for SafeResolver {
    fn resolve(&self,name:Name)->Resolving {
        let allow=self.allow_loopback;
        Box::pin(async move {let addresses=resolve(name.as_str(),0,allow).await?;Ok(Box::new(addresses.into_iter()) as Addrs)})
    }
}

#[derive(Clone)]
pub struct HttpClient {strict:reqwest::Client,local:reqwest::Client,config:std::sync::Arc<RuntimeConfig>}
impl HttpClient {
    pub(crate) fn new(config:&RuntimeConfig,builder:impl Fn()->reqwest::ClientBuilder)->Result<Self,reqwest::Error> {
        let build=|allow_loopback|builder().dns_resolver(std::sync::Arc::new(SafeResolver{allow_loopback})).build();
        Ok(Self{strict:build(false)?,local:build(true)?,config:std::sync::Arc::new(config.clone())})
    }
    pub(crate) fn for_url(&self,url:&url::Url)->io::Result<&reqwest::Client> {
        let port=url.port_or_known_default().ok_or_else(||io::Error::new(io::ErrorKind::InvalidInput,"missing outbound port"))?;
        let allow=!internal_port(&self.config,port);check_url(url,allow)?;
        // Resolve only receives a hostname; select its policy before connecting, including DNS refreshes.
        Ok(if allow{&self.local}else{&self.strict})
    }
}
#[cfg(test)]
#[path="../tests/outbound_support.rs"]
pub(crate) mod test_support;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_runtime_ports_control_the_literal_exception() {
        let mut config=test_support::runtime_config();config.web_port=31000;config.listen.set_port(31001);config.database_url="postgresql://127.0.0.1:35432/nwui".into();
        let client=HttpClient::new(&config,reqwest::Client::builder).unwrap();
        for (port,_) in crate::control::internal_targets::internal_ports(&config) {
            assert!(client.for_url(&url::Url::parse(&format!("http://127.0.0.1:{port}")).unwrap()).is_err(),"internal {port}");
        }
        assert!(client.for_url(&url::Url::parse("http://127.0.0.1:3001").unwrap()).is_ok(),"the old default API port is now a local peer");
    }
    #[tokio::test]
    async fn live_dns_availability_is_checked_after_app_state_creation() {
        for initial in [false,true] {
            let root=tempfile::tempdir().unwrap();let mut config=test_support::runtime_config();
            config.data_key_file=root.path().join("key");config.target_dns.descriptor_path=root.path().join("target-dns.json");
            let descriptor=|settings:&crate::dns_guard::Settings,available|if available{crate::dns_guard::Descriptor::available(settings)}else{crate::dns_guard::Descriptor::unavailable(settings,"test disabled")};
            crate::dns_guard::write_descriptor(&config.target_dns,&descriptor(&config.target_dns,initial)).await.unwrap();
            std::fs::write(&config.data_key_file,"A5".repeat(32)).unwrap();
            let state=crate::state::AppState::new(config).unwrap();
            let url=url::Url::parse(&format!("http://127.0.0.1:{}",state.config.target_dns.port)).unwrap();
            assert_eq!(crate::dns_guard::status(&state.config).available,initial);
            assert_eq!(state.webhook_http.for_url(&url).is_err(),initial);
            crate::dns_guard::write_descriptor(&state.config.target_dns,&descriptor(&state.config.target_dns,!initial)).await.unwrap();
            assert_eq!(crate::dns_guard::status(&state.config).available,!initial);
            assert_eq!(state.webhook_http.for_url(&url).is_err(),!initial,"DNS availability changed after AppState construction");
        }
    }
    #[test]
    fn mixed_answers_are_rejected_before_any_address_is_used() {
        for allow in [false,true] {
            assert!(checked(vec!["10.213.1.2:443".parse().unwrap(),"169.254.169.254:443".parse().unwrap()],allow).is_err());
            assert!(checked(vec![],allow).is_err());
            assert!(checked(vec!["10.213.1.2:443".parse().unwrap(),"[fd00::2]:443".parse().unwrap()],allow).is_ok());
            assert!(checked(vec!["[::ffff:127.0.0.1]:443".parse().unwrap()],allow).is_err());
        }
        assert!(checked(vec!["127.0.0.1:443".parse().unwrap()],false).is_err());
        assert!(checked(vec!["127.0.0.1:443".parse().unwrap(),"[::1]:443".parse().unwrap()],true).is_ok());
    }
}
