//! Shared fake-nginx/config fixtures for CLI integration and publisher kernel tests.
use super::{AppState,RuntimeConfig,RenderedConfig};
use std::{collections::BTreeMap,os::unix::fs::PermissionsExt,path::{Path,PathBuf},process::{Child,Command,Stdio},time::{Duration,Instant}};
/// A fake nginx that logs `<who> start|end <args>`, sleeping in between for the seconds in
/// `sleep-<who>` if that file exists. The guard process runs it with `NWUI_TEST_WHO=guard`; the
/// API side (this test process) as `api`.
pub struct Nginx {
    pub dir: PathBuf,
    pub bin: PathBuf,
    pub log: PathBuf,
}
impl Nginx {
    pub fn new(dir: &Path) -> Self {
        let bin = dir.join("nginx");
        let log = dir.join("calls");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\nif [ \"$1\" = -V ]; then printf '%s\\n' 'nginx version: nginx/1.30.6' 'configure arguments: --prefix={dir} --conf-path={dir}/nginx.conf' >&2; exit 0; fi\nwho=${{NWUI_TEST_WHO:-}}\nif [ -z \"$who\" ]; then case \"$(tr '\\000' ' ' < /proc/$PPID/cmdline)\" in *guard_worker*|*\" guard\"*) who=guard;; *) who=api;; esac; fi\nif [ \"$1\" = -T ]; then printf '# configuration file %s:\\n' '{dir}/nginx.conf'; fi\necho \"$who start $*\" >> '{log}'\necho $$ > '{dir}/pid-'$who\nif [ -f '{dir}/sleep-'$who ]; then sleep \"$(cat '{dir}/sleep-'$who)\"; fi\necho \"$who end $*\" >> '{log}'\nexit 0\n",
                log = log.display(),
                dir = dir.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self { dir: dir.into(), bin, log }
    }
    pub fn sleep(&self, who: &str, seconds: f32) {
        std::fs::write(self.dir.join(format!("sleep-{who}")), seconds.to_string()).unwrap();
    }
    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    /// Async: the API side of these tests runs on the same current-thread runtime.
    pub async fn wait_for(&self, line: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.calls().iter().any(|l| l == line) {
            assert!(Instant::now() < deadline, "never saw {line:?}: {:?}", self.calls());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    /// Every line of `first` comes before every line of `second`.
    pub fn assert_before(&self, first: &str, second: &str) {
        let calls = self.calls();
        let last = calls.iter().rposition(|l| l.starts_with(first));
        let next = calls.iter().position(|l| l.starts_with(second));
        assert!(last.is_some() && next.is_some(), "{calls:?}");
        assert!(last < next, "{first} and {second} interleaved: {calls:?}");
    }
    pub fn kill_leftover(&self, who: &str) {
        if let Ok(pid) = std::fs::read_to_string(self.dir.join(format!("pid-{who}"))) {
            let _ = Command::new("kill").arg(pid.trim()).status();
        }
    }
}

pub struct Paths {
    pub conf: PathBuf,
    pub stream: PathBuf,
    pub quarantine: PathBuf,
    pub main: PathBuf,
    pub certs: PathBuf,
}
pub fn paths(dir: &Path) -> Paths {
    let p = Paths {
        conf: dir.join("conf"),
        stream: dir.join("stream"),
        quarantine: dir.join("quarantine"),
        main: dir.join("nginx.conf"),
        certs: dir.join("certs"),
    };
    std::fs::create_dir_all(&p.conf).unwrap();
    std::fs::create_dir_all(&p.stream).unwrap();
    std::fs::write(&p.main, "listen 80;\nlisten 443 ssl;\nlisten 8080;\n").unwrap();
    let store_root=dir.join("publisher-state");
    std::fs::create_dir_all(&store_root).unwrap();
    nwui_api::publisher::test_support::prepare_store_fixture(&store_root).unwrap();
    p
}
pub fn api_config(nginx: &Nginx, p: &Paths, wait_secs: &str) -> Vec<(&'static str, String)> {
    vec![
        ("NGINX_BIN", nginx.bin.display().to_string()),
        ("NGINX_CONF", p.main.display().to_string()),
        ("NGINX_GUARD_QUARANTINE", p.quarantine.display().to_string()),
        ("NGINX_CONF_D", p.conf.display().to_string()),
        ("NGINX_STREAM_D", p.stream.display().to_string()),
        ("CERTS_DIR", p.certs.display().to_string()),
        ("NGINX_LOCK_WAIT_SECS", wait_secs.into()),
        ("MIGRATIONS_DIR", concat!(env!("CARGO_MANIFEST_DIR"), "/../../packages/db/drizzle").into()),
    ]
}
pub fn api_state(dir: &Path, nginx: &Nginx, p: &Paths, wait_secs: &str) -> AppState {
    let extra = api_config(nginx, p, wait_secs);
    let mut config = RuntimeConfig::read(|key| {
        extra.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone()).or_else(|| match key {
            "DATABASE_URL" => Some("postgres://test:test@127.0.0.1:1/test".into()),
            "AUTH_SECRET" => Some("test-secret".into()),
            "DATA_KEY_FILE" => Some(dir.join("key").display().to_string()),
            _ => None,
        })
    })
    .unwrap();
    config.target_dns.descriptor_path = dir.join("target-dns.json");
    std::fs::write(&config.data_key_file,"A5".repeat(32)).unwrap();let mut state=AppState::new(config).unwrap();
    super::nginx::witness(&mut state);
    state
}
pub fn rendered() -> RenderedConfig {
    RenderedConfig {
        conf: BTreeMap::from([
            ("upstreams.conf".into(), b"\n".to_vec()),
            ("server-new.conf".into(), b"server { listen 8081; }\n".to_vec()),
        ]),
        stream: BTreeMap::new(),
    }
}
pub fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    pub fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, &mut out);
    out
}
pub fn finish(child: Child) -> (bool, String) {
    let output = child.wait_with_output().unwrap();
    (output.status.success(), String::from_utf8_lossy(&output.stderr).into_owned())
}


pub fn guard_command(mut command:Command, nginx:&Nginx, p:&Paths, wait_secs:&str)->Command {
    let store_root=nginx.dir.join("publisher-state");
    command
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin",nginx.dir.display()))
        .env("NWUI_TEST_WHO", "guard")
        .env("NWUI_TEST_PUBLISHER_ROOT", &store_root)
        .env("NWUI_CONTAINER", "1")
        // The database port is one of the internal ports; the guard never connects.
        .env("DATABASE_URL", "postgres://unused@127.0.0.1:5432/unused")
        .env("AUTH_SECRET", "unused-by-the-guard")
        .env("NGINX_BIN", &nginx.bin)
        .env("NGINX_CONF", &p.main)
        .env("NGINX_CONF_D", &p.conf)
        .env("NGINX_STREAM_D", &p.stream)
        .env("NGINX_GUARD_QUARANTINE", &p.quarantine)
        .env("NGINX_LOCK_WAIT_SECS", wait_secs)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}
