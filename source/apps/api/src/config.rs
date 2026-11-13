use std::{
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
};

/// Runtime secrets deliberately do not implement Debug or Serialize.
#[derive(Clone)]
pub struct RuntimeConfig {
    pub target_dns: crate::dns_guard::Settings,
    pub version: String,
    pub git_sha: String,
    pub listen: SocketAddr,
    pub web_port: u16,
    pub database_url: String,
    pub database_pool_size: usize,
    pub password_concurrency: usize,
    pub data_key_file: PathBuf,
    pub migrations_dir: PathBuf,
    pub nginx_bin: PathBuf,
    pub nginx_main_conf: PathBuf,
    pub nginx_quarantine_dir: PathBuf,
    pub nginx_conf_dir: PathBuf,
    pub nginx_stream_dir: PathBuf,
    pub certs_dir: PathBuf,
    pub auth_log_file: PathBuf,
    pub nginx_access_log: PathBuf,
    pub nginx_error_log: PathBuf,
    pub nginx_status_url: String,
    pub auth_secrets: Vec<String>,
    pub auth_url: Option<String>,
    /// Running in the product image (`NWUI_CONTAINER`), where loopback is the container itself.
    pub in_container: bool,
    pub acme_directory_url: Option<String>,
    pub cloudflare_api_base: String,
    pub acme_timeout: std::time::Duration,
    pub dns_propagation_timeout: std::time::Duration,
    /// How long an apply or the nginx restart guard waits for the other to finish with the
    /// managed files (`control::nginx_lock`).
    pub nginx_lock_wait: std::time::Duration,
}

impl RuntimeConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::read(|name| std::env::var(name).ok())
    }

    pub fn read(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let default = |key, value: &str| get(key).unwrap_or_else(|| value.to_owned());
        let timeout = |key, fallback| -> anyhow::Result<std::time::Duration> {
            let seconds: u64 = default(key, fallback).parse()?;
            anyhow::ensure!((1..=600).contains(&seconds), "{key} must be between 1 and 600 seconds");
            Ok(std::time::Duration::from_secs(seconds))
        };
        let origin = |name: &str| -> Option<String> {
            let value = get(name).map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())?;
            match reqwest::Url::parse(&value) {
                Ok(parsed) if matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some() => Some(value),
                _ => {
                    eprintln!("[config] warning: {name} must be an http/https URL with a host; treating it as unset");
                    None
                }
            }
        };
        let auth_url = origin("AUTH_URL").or_else(|| origin("NEXTAUTH_URL"));
        let port: u16 = default("API_PORT", "3001").parse()?;
        let web_port: u16 = default("PORT", "3000").parse()?;
        anyhow::ensure!(web_port != 0, "PORT must be nonzero");
        anyhow::ensure!(
            port != 0 && port != web_port,
            "API_PORT must be nonzero and separate from the web port"
        );
        let database_url = get("DATABASE_URL")
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("DATABASE_URL is required"))?;
        let database_pool_size: usize = default("API_DB_POOL_SIZE", "16").parse()?;
        anyhow::ensure!(
            (1..=64).contains(&database_pool_size),
            "API_DB_POOL_SIZE must be between 1 and 64"
        );
        let password_concurrency: usize = default("API_PASSWORD_CONCURRENCY", "2").parse()?;
        anyhow::ensure!(
            (1..=64).contains(&password_concurrency),
            "API_PASSWORD_CONCURRENCY must be between 1 and 64"
        );
        let mut auth_secrets = Vec::new();
        // Auth.js prepends numbered secrets: the highest numbered secret writes new cookies.
        for key in ["AUTH_SECRET_3", "AUTH_SECRET_2", "AUTH_SECRET_1", "AUTH_SECRET"] {
            if let Some(value) = get(key).filter(|s| !s.is_empty()) {
                auth_secrets.push(value);
            }
        }
        anyhow::ensure!(!auth_secrets.is_empty(), "AUTH_SECRET is required");
        let package: serde_json::Value =
            serde_json::from_str(include_str!("../../web/package.json"))?;
        Ok(Self {
            target_dns: crate::dns_guard::Settings::read(&get),
            version: package["version"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing product version"))?
                .to_owned(),
            git_sha: default("APP_GIT_SHA", ""),
            listen: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            web_port,
            database_url,
            database_pool_size,
            password_concurrency,
            auth_secrets,
            auth_url,
            in_container: matches!(get("NWUI_CONTAINER").as_deref(), Some("1" | "true")),
            acme_directory_url: get("ACME_DIRECTORY_URL"),
            cloudflare_api_base: default("CF_API_BASE", "https://api.cloudflare.com/client/v4"),
            acme_timeout: timeout("API_ACME_TIMEOUT_SECS", "120")?,
            dns_propagation_timeout: timeout("API_DNS_PROPAGATION_TIMEOUT_SECS", "180")?,
            nginx_lock_wait: timeout("NGINX_LOCK_WAIT_SECS", "60")?,
            data_key_file: default("DATA_KEY_FILE", "/var/lib/nginx-web-ui/secrets/data_key")
                .into(),
            migrations_dir: default("MIGRATIONS_DIR", "packages/db/drizzle").into(),
            nginx_bin: default("NGINX_BIN", "nginx").into(),
            nginx_main_conf: default("NGINX_CONF", "/etc/nginx/nginx.conf").into(),
            nginx_quarantine_dir: default("NGINX_GUARD_QUARANTINE", "/var/lib/nginx-web-ui/quarantine").into(),
            nginx_conf_dir: default("NGINX_CONF_D", "/etc/nginx/conf.d").into(),
            nginx_stream_dir: default("NGINX_STREAM_D", "/etc/nginx/stream.d").into(),
            certs_dir: default("CERTS_DIR", "/var/lib/nginx-web-ui/certs").into(),
            auth_log_file: default("AUTH_LOG", "/var/log/nginx-web-ui/auth.log").into(),
            nginx_access_log: default("NGINX_ACCESS_LOG", "/var/log/nginx/access.log").into(),
            nginx_error_log: default("NGINX_ERROR_LOG", "/var/log/nginx/error.log").into(),
            nginx_status_url: default("NGINX_STATUS_URL", "http://127.0.0.1:8080/nginx_status"),
        })
    }
}

#[cfg(test)]
mod audit_tests {
    use super::*;

    fn read(extra: &[(&str, &str)]) -> anyhow::Result<RuntimeConfig> {
        RuntimeConfig::read(|key| extra.iter().find(|(name, _)| *name == key).map(|(_, value)| (*value).to_owned()).or_else(|| match key {
            "DATABASE_URL" => Some("postgres://test@localhost/test".into()),
            "AUTH_SECRET" => Some("test-secret".into()),
            _ => None,
        }))
    }

    #[test]
    fn audit_s16_empty_auth_urls_are_unset() {
        for name in ["AUTH_URL", "NEXTAUTH_URL"] {
            assert!(read(&[(name, "")]).unwrap().auth_url.is_none(), "empty {name} must be unset");
        }
    }

    #[test]
    fn audit_s16_invalid_origins_are_treated_as_unset() {
        for name in ["AUTH_URL", "NEXTAUTH_URL"] {
            for value in ["admin.example.test", "ftp://admin.example.test", "https://", "file:///tmp/auth"] {
                let result = read(&[(name, value)]);
                assert!(result.unwrap().auth_url.is_none(), "invalid {name} must not stop boot");
            }
        }
        assert!(read(&[("AUTH_URL", "invalid"), ("API_PORT", "invalid")]).is_err(), "other configuration errors still stop boot");
    }

    #[test]
    fn audit_s16_valid_origins_keep_primary_precedence() {
        assert_eq!(read(&[("AUTH_URL", "http://localhost:1234"), ("NEXTAUTH_URL", "https://legacy.example.test")]).unwrap().auth_url.as_deref(), Some("http://localhost:1234"));
        assert_eq!(read(&[("AUTH_URL", "http://localhost:1234"), ("NEXTAUTH_URL", "invalid")]).unwrap().auth_url.as_deref(),Some("http://localhost:1234"));
        assert!(read(&[("AUTH_URL", "   ")]).unwrap().auth_url.is_none());
    }

    #[test]
    fn audit_s16_empty_primary_uses_valid_legacy_origin() {
        assert_eq!(read(&[("AUTH_URL", ""), ("NEXTAUTH_URL", "https://admin.example.test")]).unwrap().auth_url.as_deref(), Some("https://admin.example.test"));
        assert_eq!(read(&[("AUTH_URL", "invalid"), ("NEXTAUTH_URL", "https://admin.example.test")]).unwrap().auth_url.as_deref(), Some("https://admin.example.test"));
    }
    #[test]
    fn valid_primary_never_reads_legacy_url() {
        let config=RuntimeConfig::read(|key|match key {
            "AUTH_URL"=>Some("https://admin.example.test".into()),
            "NEXTAUTH_URL"=>panic!("legacy origin must not be read when AUTH_URL is valid"),
            "DATABASE_URL"=>Some("postgres://test@localhost/test".into()),
            "AUTH_SECRET"=>Some("test-secret".into()),
            _=>None,
        }).unwrap();
        assert_eq!(config.auth_url.as_deref(),Some("https://admin.example.test"));
    }

}
