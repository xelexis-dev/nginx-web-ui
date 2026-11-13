//! A private witness for the exact nginx configuration generation.
use crate::config::RuntimeConfig;
use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) const MARKER: &str = ".nwui-generation";
pub(crate) const UNCERTAIN: &str = ".nwui-apply-uncertain";
const HEADER: &str = "x-nwui-config-generation";
const MAX_BODY: usize = 16 * 1024;
const OPERATION: Duration = Duration::from_secs(280);
// Restoration keeps its 30 s validation, 15 s ACK and 20 s of the shared I/O headroom.
pub(crate) const RESTORE_RESERVE: Duration = Duration::from_secs(65);

fn marker_bytes(token: &str) -> Result<Vec<u8>> {
    let id = uuid::Uuid::parse_str(token)?;
    ensure!(
        id.get_version_num() == 4 && id.to_string() == token,
        "Invalid generation literal"
    );
    Ok(format!("add_header X-NWUI-Config-Generation \"{token}\";\n").into_bytes())
}
/// A pre-start reconciliation never signals or rewrites under a live master.
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub(crate) enum NginxState { Running, Starting, Stopped }

pub(crate) async fn ensure_nginx_stopped(config:&RuntimeConfig)->Result<()> {
    require_stopped(nginx_running(config).await?)
}
pub(super) fn require_stopped(state:NginxState)->Result<()> {
    ensure!(state==NginxState::Stopped,if state==NginxState::Starting {"nginx master is starting"}else{"nginx master is still running"});Ok(())
}
pub(super) async fn ensure_no_pending_start(config:&RuntimeConfig,guard_parent:Option<u32>)->Result<()> {
    // A new run publishes this before waiting for our locks; without it, keep legacy no-op recovery DB/config-blind.
    if group_pid(Path::new("/run/nginx-web-ui/nginx-process-group"))?.is_some() {
        require_stopped(nginx_state(config,guard_parent).await?)?;
    }
    Ok(())
}
pub(crate) async fn nginx_running(config:&RuntimeConfig)->Result<NginxState> {
    nginx_state(config,None).await
}
pub(super) async fn nginx_state(config:&RuntimeConfig,guard_parent:Option<u32>)->Result<NginxState> {
    let (_,pid_file)=configured_target(config)?;
    if let Some(state)=pid_state(&pid_file)? {return Ok(state);}
    let group=Path::new("/run/nginx-web-ui/nginx-process-group");
    if group_pid(group)?.is_none(){return Ok(NginxState::Stopped);}
    let admitted=crate::nonroot::admit_nginx(config).await?;
    nginx_state_with(&pid_file,group,&admitted.master_bin,Path::new("/etc/s6-overlay/s6-rc.d/nginx/run"),guard_parent)
}
fn live(pid:i32)->Result<bool> {
    if unsafe{libc::kill(pid,0)}==0{return Ok(true);}
    let error=std::io::Error::last_os_error();
    ensure!(error.raw_os_error()==Some(libc::ESRCH),"Cannot establish nginx master state: {error}");Ok(false)
}
fn pid_state(pid_file:&Path)->Result<Option<NginxState>> {
    let text=match std::fs::read_to_string(pid_file) {
        Ok(text)=>text,
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(None),
        Err(error)=>return Err(error.into()),
    };
    if text.is_empty(){return Ok(None);}
    let pid=text.trim().parse::<i32>().context("Cannot establish nginx master identity")?;
    ensure!(pid>1,"Cannot establish nginx master identity");
    Ok(if live(pid)?{Some(NginxState::Running)}else{None})
}
fn group_pid(group_file:&Path)->Result<Option<i32>> {
    let text=match std::fs::read_to_string(group_file) {
        Ok(text)=>text,
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(None),
        Err(error)=>return Err(error.into()),
    };
    let Ok(pid)=text.trim().parse::<i32>() else{return Ok(None);};
    if pid<=1{return Ok(None);}
    Ok(if live(pid)?{Some(pid)}else{None})
}
fn same_inode(a:&std::fs::Metadata,b:&std::fs::Metadata)->bool {
    use std::os::unix::fs::MetadataExt;
    a.dev()==b.dev()&&a.ino()==b.ino()
}
const MAX_RUN_SCRIPT_BYTES:usize=64*1024;
fn run_script_bytes(path:&Path)->Result<Option<Vec<u8>>> {
    use crate::db_upgrade::key_file::{validate_trusted_input,TrustedInput,KeyFileError};
    use std::{io::Read,os::unix::fs::MetadataExt};
    let file=match validate_trusted_input(path,TrustedInput::Config) {
        Ok(file)=>file,
        Err(KeyFileError::Missing|KeyFileError::Unsafe|KeyFileError::Invalid)=>return Ok(None),
        Err(error)=>return Err(error.into()),
    };
    let metadata=file.metadata()?;
    if metadata.uid()!=0||metadata.mode()&0o022!=0||metadata.len()>MAX_RUN_SCRIPT_BYTES as u64{return Ok(None);}
    let mut bytes=Vec::new();file.take(MAX_RUN_SCRIPT_BYTES as u64+1).read_to_end(&mut bytes)?;
    Ok((bytes.len()<=MAX_RUN_SCRIPT_BYTES).then_some(bytes))
}
pub(super) fn nginx_state_with(pid_file:&Path,group_file:&Path,master:&Path,run_script:&Path,guard_parent:Option<u32>)->Result<NginxState> {
    use std::os::unix::ffi::OsStrExt;
    if let Some(state)=pid_state(pid_file)?{return Ok(state);}
    let Some(pid)=group_pid(group_file)? else{return Ok(NginxState::Stopped);};
    let exe_path=format!("/proc/{pid}/exe");
    let metadata=|path:&Path|->Result<Option<std::fs::Metadata>>{match std::fs::metadata(path){Ok(value)=>Ok(Some(value)),Err(error)if error.kind()==std::io::ErrorKind::NotFound=>Ok(None),Err(error)=>Err(error.into())}};
    let Some(exe)=metadata(Path::new(&exe_path))? else{return Ok(NginxState::Stopped);};
    let master=std::fs::metadata(master)?;
    if same_inode(&exe,&master){return Ok(NginxState::Starting);}
    let shell=std::fs::metadata("/bin/sh")?;
    if !same_inode(&exe,&shell){return Ok(NginxState::Stopped);}
    let cmdline=match std::fs::read(format!("/proc/{pid}/cmdline")){Ok(value)=>value,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>return Ok(NginxState::Stopped),Err(error)=>return Err(error.into())};
    ensure!(!cmdline.is_empty(),"Cannot establish nginx startup identity");
    let script=match cmdline.split(|byte|*byte==0).nth(1) {
        Some(arg)=>{
            let arg=Path::new(std::ffi::OsStr::from_bytes(arg));
            let path=if arg.is_absolute(){arg.to_owned()}else{PathBuf::from(format!("/proc/{pid}/cwd")).join(arg)};
            Some(path)
        },
        None=>None,
    };
    let matches_script=match script {
        Some(path)=>match run_script_bytes(&path)? {Some(bytes)=>run_script_bytes(run_script)?.is_some_and(|expected|bytes==expected),None=>false},
        None=>false,
    };
    let Some(after)=metadata(Path::new(&exe_path))? else{return Ok(NginxState::Stopped);};
    // exec may happen between /proc reads; never mistake the new master for a stale shell.
    if same_inode(&after,&master){return Ok(NginxState::Starting);}
    ensure!(same_inode(&exe,&after),"nginx startup process changed identity");
    if matches_script {
        return Ok(if guard_parent==Some(pid as u32){NginxState::Stopped}else{NginxState::Starting});
    }
    Ok(NginxState::Stopped)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Master {
    pid: u32,
    start_ticks: u64,
}
impl Master {
    fn read(pid_file: &Path) -> Result<Self> {
        let pid = std::fs::read_to_string(pid_file)?.trim().parse::<u32>()?;
        ensure!(pid > 0, "nginx master PID is missing");
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
        let fields = stat.rsplit_once(") ").context("Invalid master stat")?.1;
        let start_ticks = fields
            .split_whitespace()
            .nth(19)
            .context("Missing master start ticks")?
            .parse()?;
        Ok(Self { pid, start_ticks })
    }
}
fn configured_target(config: &RuntimeConfig) -> Result<(url::Url, PathBuf)> {
    use super::guard_targets::{End, statements};
    let url = url::Url::parse(&config.nginx_status_url)?;
    let ip = url
        .host_str()
        .context("Missing status host")?
        .trim_matches(['[', ']'])
        .parse::<std::net::IpAddr>()?;
    ensure!(
        url.scheme() == "http"
            && ip.is_loopback()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/nginx_status",
        "Unowned nginx status target"
    );
    let text = std::fs::read_to_string(&config.nginx_main_conf)?;
    let marker = config.nginx_conf_dir.join(MARKER).display().to_string();
    let mut stack: Vec<Vec<String>> = Vec::new();
    let mut pid = PathBuf::from("/run/nginx.pid");
    let mut listens = Vec::<String>::new();
    let mut location = Vec::<Vec<String>>::new();
    let mut owned = 0;
    let mut witness_servers = 0;
    let port = url.port_or_known_default().unwrap();
    let address_matches = |listens: &[String]| {
        listens.iter().any(|raw| {
            raw.parse::<u16>().is_ok_and(|p| p == port && ip.is_ipv4())
                || raw.parse::<std::net::SocketAddr>().is_ok_and(|a| {
                    a.port() == port
                        && (a.ip() == ip
                            || a.ip().is_unspecified() && a.ip().is_ipv4() == ip.is_ipv4())
                })
        })
    };
    let mut unsafe_server = false;
    let mut candidates = 0;
    let mut unsafe_location = false;
    for statement in statements(&text) {
        let words = statement.words;
        match statement.end {
            End::Open => {
                if stack.len() == 1 && words == ["server"] {
                    listens.clear();
                    unsafe_server = false;
                    candidates = 0;
                }
                if stack.len() == 3 && stack[2] == ["location", "=", "/nginx_status"] {
                    unsafe_location = true;
                }
                if words == ["location", "=", "/nginx_status"] {
                    location.clear();
                    unsafe_location = false;
                }
                stack.push(words);
            }
            End::Semicolon => {
                if stack.is_empty() && words.first().is_some_and(|w| w == "pid") {
                    ensure!(
                        words.len() == 2 && Path::new(&words[1]).is_absolute(),
                        "Invalid nginx pid path"
                    );
                    pid = words[1].clone().into();
                }
                if stack.len() == 2 && stack[0] == ["http"] && stack[1] == ["server"] {
                    if words.first().is_some_and(|w| w == "listen") {
                        if let Some(raw) = words.get(1) {
                            listens.push(raw.clone());
                        }
                    }
                    if words
                        .first()
                        .is_some_and(|w| matches!(w.as_str(), "return" | "rewrite" | "include"))
                    {
                        unsafe_server = true;
                    }
                }
                if stack.len() == 3
                    && stack[0] == ["http"]
                    && stack[1] == ["server"]
                    && stack[2] == ["location", "=", "/nginx_status"]
                {
                    location.push(words);
                }
            }
            End::Close => {
                if stack.len() == 3
                    && stack[0] == ["http"]
                    && stack[1] == ["server"]
                    && stack[2] == ["location", "=", "/nginx_status"]
                {
                    let has = |want: &[&str]| {
                        location
                            .iter()
                            .any(|row| row.iter().map(String::as_str).eq(want.iter().copied()))
                    };
                    let safe = location
                        .iter()
                        .all(|row| match row.first().map(String::as_str) {
                            Some("stub_status") => row.len() == 1,
                            Some("allow") => {
                                row.len() == 2 && row[1] == "127.0.0.3"
                            }
                            Some("deny") => row == &["deny", "all"],
                            Some("include") => row.len() == 2 && row[1] == marker,
                            Some("access_log") => row == &["access_log", "off"],
                            _ => false,
                        });
                    if safe
                        && !unsafe_location
                        && address_matches(&listens)
                        && has(&["stub_status"])
                        && has(&["allow", "127.0.0.3"])
                        && has(&["deny", "all"])
                        && has(&["include", &marker])
                    {
                        candidates += 1;
                    }
                }
                if stack.len() == 2 && stack[0] == ["http"] && stack[1] == ["server"] {
                    if address_matches(&listens) {
                        witness_servers += 1;
                    }
                    if !unsafe_server {
                        owned += candidates;
                    }
                }
                ensure!(stack.pop().is_some(), "Unbalanced nginx configuration");
            }
        }
    }
    ensure!(
        stack.is_empty() && owned == 1 && witness_servers == 1,
        "nginx status listener is not uniquely owned"
    );
    Ok((url, pid))
}
fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .local_address(std::net::IpAddr::from([127,0,0,3]))
        .http1_only()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(0)
        .build()?)
}
async fn sync_parent(config: &RuntimeConfig) -> Result<()> {
    tokio::fs::File::open(&config.nginx_conf_dir)
        .await?
        .sync_all()
        .await?;
    Ok(())
}
pub(in crate::publisher) async fn publish(config: &RuntimeConfig, token: &str) -> Result<()> {
    super::files::atomic_write(
        &config.nginx_conf_dir.join(MARKER),
        &marker_bytes(token)?,
        0o644,
    )
    .await?;
    sync_parent(config).await
}
pub(in crate::publisher) async fn begin_uncertain(config: &RuntimeConfig) -> Result<()> {
    tokio::fs::create_dir_all(&config.nginx_conf_dir).await?;
    super::files::atomic_write(
        &config.nginx_conf_dir.join(UNCERTAIN),
        b"nginx generation is unconfirmed\n",
        0o600,
    )
    .await?;
    sync_parent(config).await
}
pub(crate) use crate::control::model::apply_uncertain as uncertain;
pub(in crate::publisher) async fn clear_uncertain(config: &RuntimeConfig) -> Result<()> {
    match tokio::fs::remove_file(config.nginx_conf_dir.join(UNCERTAIN)).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    sync_parent(config).await
}

pub(crate) struct Witness {
    http: reqwest::Client,
    url: url::Url,
    pid_file: PathBuf,
}
impl Witness {
    pub(crate) fn from_config(config: &RuntimeConfig) -> Result<Self> {
        let (url, pid_file) = configured_target(config)?;
        Ok(Self {
            http: client()?,
            url,
            pid_file,
        })
    }
    async fn probe(&self, token: &str, deadline: Instant) -> Result<bool> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .context("nginx acknowledgement timed out")?;
        let operation = async {
            let mut response = self
                .http
                .get(self.url.clone())
                .header(reqwest::header::CONNECTION, "close")
                .timeout(remaining.min(Duration::from_secs(1)))
                .send()
                .await?;
            let headers = response.headers().get_all(HEADER);
            let mut values = headers.iter();
            let matched = response.status() == reqwest::StatusCode::OK
                && values
                    .next()
                    .is_some_and(|v| v.as_bytes() == token.as_bytes())
                && values.next().is_none();
            ensure!(
                response
                    .content_length()
                    .is_none_or(|n| n <= MAX_BODY as u64),
                "nginx 상태 응답이 너무 큽니다"
            );
            let mut size = 0;
            while let Some(chunk) = response.chunk().await? {
                size += chunk.len();
                ensure!(size <= MAX_BODY, "nginx 상태 응답이 너무 큽니다");
            }
            ensure!(Instant::now() < deadline, "nginx acknowledgement timed out");
            Ok(matched)
        };
        tokio::time::timeout(remaining.min(Duration::from_secs(1)), operation)
            .await
            .context("nginx status probe timed out")?
    }
    async fn observe(&self, token: &str, deadline: Instant, master: &Master) -> Result<()> {
        loop {
            ensure!(
                &Master::read(&self.pid_file)? == master,
                "nginx master changed during acknowledgement"
            );
            let observed = self.probe(token, deadline).await;
            ensure!(
                &Master::read(&self.pid_file)? == master,
                "nginx master changed during acknowledgement"
            );
            if observed.is_ok_and(|ok| ok) {
                return Ok(());
            }
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .context("nginx generation remains unconfirmed")?;
            tokio::time::sleep(remaining.min(Duration::from_millis(100))).await;
        }
    }
    #[cfg(test)]
    pub(crate) async fn confirm(
        &self,
        config: &RuntimeConfig,
        first: &str,
        deadline: Instant,
    ) -> Result<()> {
        let bin=config.nginx_bin.clone();
        self.confirm_with(config,first,deadline,move |remaining| {
            let process=super::process::NginxProcess::new(bin.clone()).with_timeout(remaining);
            async move {process.reload().await}
        },|_|Ok(())).await
    }
    pub(crate) async fn confirm_with<R,Fut,F>(
        &self,config:&RuntimeConfig,first:&str,deadline:Instant,mut reload:R,mut acknowledged:F,
    )->Result<()>
    where R:FnMut(Duration)->Fut,Fut:std::future::Future<Output=Result<()>>,F:FnMut(usize)->Result<()> {
        ensure!(
            tokio::fs::read(config.nginx_conf_dir.join(MARKER)).await? == marker_bytes(first)?,
            "Generation template changed after validation"
        );
        let master = Master::read(&self.pid_file)?;
        for phase in 0..2 {
            let token = if phase == 0 {
                first.to_owned()
            } else {
                ensure!(
                    tokio::fs::read(config.nginx_conf_dir.join(MARKER)).await?
                        == marker_bytes(first)?,
                    "Generation template changed after validation"
                );
                let next = uuid::Uuid::new_v4().to_string();
                publish(config, &next).await?;
                next
            };
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .context("nginx acknowledgement timed out")?;
            reload(remaining).await?;
            self.observe(&token, deadline, &master).await?;
            acknowledged(phase)?;
        }
        Ok(())
    }
}

/// Observe the owner's current generation without publishing or acknowledging a new one.
pub(crate) async fn observe_current(config: &RuntimeConfig, deadline: Instant) -> Result<()> {
    fn read_marker(path: &Path) -> Result<String> {
        use std::{io::Read, os::unix::fs::{OpenOptionsExt, MetadataExt}};
        let file=std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path)?;
        let metadata=file.metadata()?;
        ensure!(metadata.is_file() && metadata.uid()==0 && metadata.nlink()==1 && metadata.mode()&0o022==0 && metadata.len()<=128,"Unsafe generation marker");
        let mut bytes=Vec::new();file.take(129).read_to_end(&mut bytes)?;
        ensure!(bytes.len()<=128,"Generation marker is too large");
        let text=std::str::from_utf8(&bytes)?;
        let token=text.strip_prefix("add_header X-NWUI-Config-Generation \"")
            .and_then(|text|text.strip_suffix("\";\n")).context("Invalid generation marker")?;
        ensure!(marker_bytes(token)?==bytes,"Invalid generation marker");
        Ok(token.into())
    }
    let witness=Witness::from_config(config)?;
    let path=config.nginx_conf_dir.join(MARKER);
    loop {
        ensure!(Instant::now()<deadline,"nginx generation remains unconfirmed");
        let token=read_marker(&path)?;
        let before=Master::read(&witness.pid_file);
        let observed=if before.is_ok() {witness.probe(&token,deadline).await.is_ok_and(|matched|matched)} else {false};
        let after=Master::read(&witness.pid_file);
        let current=read_marker(&path)?;
        if observed && token==current && matches!((&before,&after),(Ok(before),Ok(after)) if before==after) {return Ok(());}
        let remaining=deadline.checked_duration_since(Instant::now()).context("nginx generation remains unconfirmed")?;
        tokio::time::sleep(remaining.min(Duration::from_millis(100))).await;
    }
}

pub(crate) struct Budget {
    started: Instant,
    limit:Duration,
}
impl Budget {
    pub(crate) fn new() -> Self {
        Self::with_limit(OPERATION)
    }
    pub(crate) fn with_limit(limit:Duration)->Self {Self{started:Instant::now(),limit:limit.min(OPERATION)}}
    pub(crate) fn admission_remaining(&self) -> Result<Duration> {
        self.limit.min(Duration::from_secs(30))
            .checked_sub(self.started.elapsed())
            .context("설정 적용 대기가 30초를 넘었습니다. 잠시 후 다시 시도하세요.")
    }
    pub(crate) async fn admit<F: std::future::Future>(&self, future: F) -> Result<F::Output> {
        let result = tokio::time::timeout(self.admission_remaining()?, future)
            .await
            .context("설정 적용 대기가 30초를 넘었습니다. 잠시 후 다시 시도하세요.")?;
        self.admission_remaining()?;
        Ok(result)
    }
    pub(crate) fn containment_deadline(&self) -> Instant {
        self.started + self.limit.saturating_sub(RESTORE_RESERVE + Duration::from_secs(45))
    }
    pub(crate) fn forward(&self) -> Result<()> {
        ensure!(
            Instant::now() < self.containment_deadline(),
            "이전 설정 복구 시간을 남기기 위해 새 적용을 중단했습니다."
        );
        Ok(())
    }
    pub(crate) fn ack_deadline(&self) -> Result<Instant> {
        let last = self.started + self.limit.saturating_sub(RESTORE_RESERVE);
        ensure!(
            Instant::now() < last,
            "이전 설정 복구 시간을 남기기 위해 새 적용을 중단했습니다."
        );
        Ok((Instant::now() + Duration::from_secs(15)).min(last))
    }
    pub(crate) fn remaining(&self) -> Duration {
        self.limit.saturating_sub(self.started.elapsed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        task::JoinHandle,
    };

    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Good,
        Wrong,
        Status,
        Duplicate,
        Redirect,
        Reflect,
        Late,
        Length,
        Chunked,
        Boundary,
    }
    struct Server {
        url: url::Url,
        requests: Arc<Mutex<Vec<String>>>,
        accepted: Arc<AtomicUsize>,
        task: JoinHandle<()>,
        children: Arc<Mutex<Vec<JoinHandle<()>>>>,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
            for child in self.children.lock().unwrap().drain(..) {
                child.abort();
            }
        }
    }
    async fn server(mode: Mode, token: &str) -> Server {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let url = url::Url::parse(&(base.clone() + "/nginx_status")).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let accepted = Arc::new(AtomicUsize::new(0));
        let children = Arc::new(Mutex::new(Vec::new()));
        let (seen, count, tasks) = (requests.clone(), accepted.clone(), children.clone());
        let token = token.to_owned();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                count.fetch_add(1, Ordering::SeqCst);
                let seen = seen.clone();
                let token = token.clone();
                let base = base.clone();
                let child = tokio::spawn(async move {
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") && request.len() < 4096 {
                        let mut b = [0];
                        if socket.read(&mut b).await.unwrap_or(0) == 0 {
                            return;
                        }
                        request.push(b[0]);
                    }
                    let request = String::from_utf8(request).unwrap();
                    seen.lock().unwrap().push(request.clone());
                    if mode == Mode::Late {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                    }
                    let reflected = request
                        .lines()
                        .find_map(|l| l.strip_prefix("x-nwui-config-generation: "))
                        .unwrap_or("not-reflected");
                    let value = if mode == Mode::Wrong {
                        "stale"
                    } else if mode == Mode::Reflect {
                        reflected
                    } else {
                        &token
                    };
                    let status = if mode == Mode::Status {
                        "503 Unavailable"
                    } else if mode == Mode::Redirect
                        && !request.starts_with("GET /redirect-target ")
                    {
                        "302 Found"
                    } else {
                        "200 OK"
                    };
                    let mut headers = format!(
                        "HTTP/1.1 {status}\r\nX-NWUI-Config-Generation: {value}\r\nConnection: close\r\n"
                    );
                    if mode == Mode::Duplicate {
                        headers.push_str(&format!("X-NWUI-Config-Generation: {value}\r\n"));
                    }
                    if mode == Mode::Redirect {
                        headers.push_str(&format!("Location: {base}/redirect-target\r\n"));
                    }
                    if mode == Mode::Length {
                        let _ = socket
                            .write_all(
                                format!("{headers}Content-Length: {}\r\n\r\n", MAX_BODY + 1)
                                    .as_bytes(),
                            )
                            .await;
                        std::future::pending::<()>().await;
                    } else if mode == Mode::Chunked {
                        let _ = socket
                            .write_all(
                                format!(
                                    "{headers}Transfer-Encoding: chunked\r\n\r\n{MAX_BODY:x}\r\n"
                                )
                                .as_bytes(),
                            )
                            .await;
                        let _ = socket.write_all(&vec![b'x'; MAX_BODY]).await;
                        let _ = socket.write_all(b"\r\n1\r\nx\r\n0\r\n\r\n").await;
                    } else {
                        let body = if mode == Mode::Boundary {
                            vec![b'x'; MAX_BODY]
                        } else {
                            b"Active connections: 1\n".to_vec()
                        };
                        let _ = socket
                            .write_all(
                                format!("{headers}Content-Length: {}\r\n\r\n", body.len())
                                    .as_bytes(),
                            )
                            .await;
                        let _ = socket.write_all(&body).await;
                    }
                });
                tasks.lock().unwrap().push(child);
            }
        });
        Server {
            url,
            requests,
            accepted,
            task,
            children,
        }
    }
    fn config(root: &Path, status: &str) -> RuntimeConfig {
        RuntimeConfig::read(|key| {
            Some(match key {
                "DATABASE_URL" => "postgres://unused@127.0.0.1:1/test".into(),
                "AUTH_SECRET" => "test-secret".into(),
                "NGINX_CONF_D" => root.join("conf").display().to_string(),
                "NGINX_STREAM_D" => root.join("stream").display().to_string(),
                "CERTS_DIR" => root.join("certs").display().to_string(),
                "NGINX_CONF" => root.join("nginx.conf").display().to_string(),
                "NGINX_BIN" => root.join("nginx").display().to_string(),
                "NGINX_STATUS_URL" => status.into(),
                _ => return None,
            })
        })
        .unwrap()
    }
    #[tokio::test]
    async fn a36_no_live_master_without_a_live_marker_is_stopped_and_invalid_pid_files_refuse() {
        let root=tempfile::tempdir().unwrap();let cfg=observer_config(root.path(),"http://127.0.0.1:8080/nginx_status");let pid=root.path().join("nginx.pid");
        std::fs::write(&pid,b"").unwrap();
        assert_eq!(nginx_running(&cfg).await.unwrap(),NginxState::Stopped);assert!(pid.exists(),"the empty PID file must not be removed");assert!(Master::read(&pid).is_err(),"post-start witness stays strict");
        for invalid in ["\n","abc","1","0","-1"] {std::fs::write(&pid,invalid).unwrap();assert!(nginx_running(&cfg).await.is_err(),"must refuse {invalid:?}");}
        std::fs::write(&pid,format!("{}\n",std::process::id())).unwrap();assert_eq!(nginx_running(&cfg).await.unwrap(),NginxState::Running);
        let mut dead=std::process::Command::new("/bin/true").spawn().unwrap();let dead_pid=dead.id();dead.wait().unwrap();std::fs::write(&pid,format!("{dead_pid}\n")).unwrap();assert_eq!(nginx_running(&cfg).await.unwrap(),NginxState::Stopped);
        std::fs::remove_file(&pid).unwrap();assert_eq!(nginx_running(&cfg).await.unwrap(),NginxState::Stopped);
        std::fs::create_dir(&pid).unwrap();assert!(nginx_running(&cfg).await.is_err(),"non-NotFound read errors stay fatal");
    }

    fn witness(server: &Server, pid_file: &Path) -> Witness {
        Witness {
            http: client().unwrap(),
            url: server.url.clone(),
            pid_file: pid_file.into(),
        }
    }
    fn pid_file(root: &Path) -> PathBuf {
        let p = root.join("nginx.pid");
        std::fs::write(&p, std::process::id().to_string()).unwrap();
        p
    }

    fn observer_config(root:&Path,status:&str)->RuntimeConfig {
        let cfg=config(root,status);std::fs::create_dir_all(&cfg.nginx_conf_dir).unwrap();
        let url=url::Url::parse(status).unwrap();let pid=pid_file(root);
        std::fs::write(&cfg.nginx_main_conf,format!("pid {}; events {{}} http {{ server {{ listen 127.0.0.1:{}; location = /nginx_status {{ stub_status; allow 127.0.0.3; deny all; include {}/.nwui-generation; }} }} }}",pid.display(),url.port().unwrap(),cfg.nginx_conf_dir.display())).unwrap();
        std::fs::write(&cfg.nginx_bin,b"observer must never execute this file").unwrap();
        cfg
    }
    #[tokio::test]
    async fn current_observer_is_read_only_and_accepts_one_owned_served_token() {
        let token=uuid::Uuid::new_v4().to_string();let server=server(Mode::Good,&token).await;
        let root=tempfile::tempdir().unwrap();let cfg=observer_config(root.path(),server.url.as_str());
        let marker=cfg.nginx_conf_dir.join(MARKER);std::fs::write(&marker,marker_bytes(&token).unwrap()).unwrap();
        let before=std::fs::read(&marker).unwrap();
        let main=std::fs::read(&cfg.nginx_main_conf).unwrap();
        std::fs::write(cfg.nginx_conf_dir.join(UNCERTAIN),b"pre-existing uncertainty").unwrap();
        observe_current(&cfg,Instant::now()+Duration::from_secs(2)).await.unwrap();
        assert_eq!(server.accepted.load(Ordering::SeqCst),1);
        assert_eq!(std::fs::read(&marker).unwrap(),before);
        assert_eq!(std::fs::read(&cfg.nginx_bin).unwrap(),b"observer must never execute this file");
        assert_eq!(std::fs::read(cfg.nginx_conf_dir.join(UNCERTAIN)).unwrap(),b"pre-existing uncertainty");
        assert_eq!(std::fs::read(&cfg.nginx_main_conf).unwrap(),main);
    }
    #[tokio::test]
    async fn current_observer_retries_changed_token_and_changed_master() {
        for change_token in [true,false] {
            let root=tempfile::tempdir().unwrap();let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let cfg=observer_config(root.path(),&format!("http://{}/nginx_status",listener.local_addr().unwrap()));
            let first=uuid::Uuid::new_v4().to_string();let next=uuid::Uuid::new_v4().to_string();
            let marker=cfg.nginx_conf_dir.join(MARKER);std::fs::write(&marker,marker_bytes(&first).unwrap()).unwrap();
            let pid=root.path().join("nginx.pid");
            let task=tokio::spawn(async move {
                for attempt in 0..2 {
                    let (mut socket,_)=listener.accept().await.unwrap();let mut request=Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {let mut byte=[0];socket.read_exact(&mut byte).await.unwrap();request.push(byte[0]);}
                    let token=if attempt==0 {&first}else if change_token {&next}else {&first};
                    if attempt==0 {
                        if change_token {std::fs::write(&marker,marker_bytes(&next).unwrap()).unwrap();}
                        else {assert_ne!(std::process::id(),1);std::fs::write(&pid,b"1").unwrap();}
                    }
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nX-NWUI-Config-Generation: {token}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                }
            });
            observe_current(&cfg,Instant::now()+Duration::from_secs(3)).await.unwrap();task.await.unwrap();
            assert!(!cfg.nginx_conf_dir.join(UNCERTAIN).exists());
        }
    }
    #[tokio::test]
    async fn current_observer_refuses_unsafe_and_malformed_markers_without_a_probe() {
        use std::os::unix::fs::{symlink,PermissionsExt};
        let token=uuid::Uuid::new_v4().to_string();let server=server(Mode::Good,&token).await;
        let root=tempfile::tempdir().unwrap();let cfg=observer_config(root.path(),server.url.as_str());let marker=cfg.nginx_conf_dir.join(MARKER);
        for bytes in [b"not a marker".to_vec(),vec![b'x';129],marker_bytes(&token).unwrap().into_iter().chain(b"extra".iter().copied()).collect()] {
            std::fs::write(&marker,&bytes).unwrap();assert!(observe_current(&cfg,Instant::now()+Duration::from_secs(1)).await.is_err());assert_eq!(std::fs::read(&marker).unwrap(),bytes);
        }
        std::fs::write(&marker,marker_bytes(&token).unwrap()).unwrap();
        std::fs::hard_link(&marker,root.path().join("linked")).unwrap();assert!(observe_current(&cfg,Instant::now()+Duration::from_secs(1)).await.is_err());std::fs::remove_file(root.path().join("linked")).unwrap();
        std::fs::set_permissions(&marker,std::fs::Permissions::from_mode(0o666)).unwrap();assert!(observe_current(&cfg,Instant::now()+Duration::from_secs(1)).await.is_err());
        std::fs::remove_file(&marker).unwrap();symlink(root.path().join("nginx.conf"),&marker).unwrap();assert!(observe_current(&cfg,Instant::now()+Duration::from_secs(1)).await.is_err());
        assert_eq!(server.accepted.load(Ordering::SeqCst),0);assert!(!cfg.nginx_conf_dir.join(UNCERTAIN).exists());
    }

    #[test]
    fn marker_grammar_accepts_only_one_canonical_uuid_literal() {
        let t = uuid::Uuid::new_v4().to_string();
        assert_eq!(
            marker_bytes(&t).unwrap(),
            format!("add_header X-NWUI-Config-Generation \"{t}\";\n").as_bytes()
        );
        for bad in [
            "not-a-uuid",
            "00000000-0000-0000-0000-000000000000",
            "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
            "x\"; return 200; #",
        ] {
            assert!(marker_bytes(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn master_identity_is_the_pid_and_proc_start_ticks_not_a_constant() {
        let t = tempfile::tempdir().unwrap();
        let p = pid_file(t.path());
        let m = Master::read(&p).unwrap();
        assert_eq!(m.pid, std::process::id());
        assert!(m.start_ticks > 0);
        std::fs::write(&p, "not-a-pid").unwrap();
        assert!(Master::read(&p).is_err());
    }
    #[tokio::test]
    async fn fxb3_status_witness_uses_the_exclusive_loopback_source() {
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap();
        let server=tokio::spawn(async move {let(mut stream,peer)=listener.accept().await.unwrap();let mut bytes=[0;4096];stream.read(&mut bytes).await.unwrap();let status=if peer.ip().to_string()=="127.0.0.3"{"200 OK"}else{"403 Forbidden"};stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();peer.ip()});
        let response=client().unwrap().get(format!("http://{addr}/nginx_status")).send().await.unwrap();assert_eq!(response.status(),200);assert_eq!(server.await.unwrap().to_string(),"127.0.0.3");
    }
    #[test]
    fn fxb3_owned_status_acl_accepts_only_the_exclusive_source() {
        let temp=tempfile::tempdir().unwrap();let cfg=config(temp.path(),"http://127.0.0.1:18480/nginx_status");
        let base=format!("events {{}} http {{ server {{ listen 127.0.0.1:18480; location = /nginx_status {{ stub_status; allow 127.0.0.3; deny all; include {}/.nwui-generation; }} }} }}",cfg.nginx_conf_dir.display());
        std::fs::write(&cfg.nginx_main_conf,&base).unwrap();assert!(configured_target(&cfg).is_ok());
        for source in ["127.0.0.1","::1","all"] {std::fs::write(&cfg.nginx_main_conf,base.replace("allow 127.0.0.3;",&format!("allow {source};"))).unwrap();assert!(configured_target(&cfg).is_err(),"{source}");}
    }
    #[test]
    fn witness_target_requires_the_owned_exact_stub_status_location_and_literal_loopback() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let cfg = config(root, "http://127.0.0.1:18480/nginx_status");
        let pid = root.join("nginx.pid");
        let base = format!(
            "pid {}; events {{}} http {{ server {{ listen 127.0.0.1:18480; location = /nginx_status {{ stub_status; allow 127.0.0.3; deny all; include {}/.nwui-generation; }} }} }}",
            pid.display(),
            cfg.nginx_conf_dir.display()
        );
        std::fs::write(&cfg.nginx_main_conf, &base).unwrap();
        assert_eq!(configured_target(&cfg).unwrap().1, pid);
        for url in [
            "http://localhost:18480/nginx_status",
            "http://192.0.2.4:18480/nginx_status",
            "https://127.0.0.1:18480/nginx_status",
            "http://user:pass@127.0.0.1:18480/nginx_status",
            "http://127.0.0.1:18480/nginx_status?token=x",
            "http://127.0.0.1:18480/wrong",
            "http://127.0.0.1:18481/nginx_status",
        ] {
            let c = config(root, url);
            assert!(configured_target(&c).is_err(), "{url}");
        }
        for unsafe_main in [
            base.replace("stub_status;", "return 200;"),
            base.replace("deny all;", "allow all;"),
            base.replace(".nwui-generation", "other.conf"),
            base.replace("location = /nginx_status", "location /nginx_status"),
            base.replacen("http {", "http { server { listen 127.0.0.1:18480; }", 1),
            base.replace("stub_status;", "if ($arg_x) { return 200; } stub_status;"),
        ] {
            std::fs::write(&cfg.nginx_main_conf, unsafe_main).unwrap();
            assert!(configured_target(&cfg).is_err());
        }
    }
    #[tokio::test]
    async fn uncertainty_is_a_private_durable_receipt_and_can_be_cleared() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let cfg = config(t.path(), "http://127.0.0.1:18480/nginx_status");
        begin_uncertain(&cfg).await.unwrap();
        let p = cfg.nginx_conf_dir.join(UNCERTAIN);
        assert!(p.exists());
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        clear_uncertain(&cfg).await.unwrap();
        assert!(!p.exists());
    }
    #[tokio::test]
    async fn uncertainty_never_follows_an_unexpected_symlink() {
        let t = tempfile::tempdir().unwrap();
        let cfg = config(t.path(), "http://127.0.0.1:18480/nginx_status");
        std::fs::create_dir_all(&cfg.nginx_conf_dir).unwrap();
        let outside = t.path().join("outside");
        std::fs::write(&outside, "keep").unwrap();
        std::os::unix::fs::symlink(&outside, cfg.nginx_conf_dir.join(UNCERTAIN)).unwrap();
        assert!(begin_uncertain(&cfg).await.is_err());
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "keep");
    }
    #[tokio::test]
    async fn probes_use_new_connections_and_never_send_the_expected_token_or_credentials() {
        let token = uuid::Uuid::new_v4().to_string();
        let s = server(Mode::Good, &token).await;
        let t = tempfile::tempdir().unwrap();
        let p = pid_file(t.path());
        let w = witness(&s, &p);
        for _ in 0..2 {
            assert!(
                w.probe(&token, Instant::now() + Duration::from_secs(1))
                    .await
                    .unwrap()
            );
        }
        assert_eq!(s.accepted.load(Ordering::SeqCst), 2);
        let requests = s.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        for r in requests.iter() {
            let lower = r.to_lowercase();
            assert!(lower.contains("connection: close"));
            assert!(!r.contains(&token));
            assert!(!lower.contains("authorization:"));
            assert!(r.starts_with("GET /nginx_status HTTP/1.1"));
        }
    }
    #[tokio::test]
    async fn stale_duplicate_redirect_reflected_and_wrong_status_replies_are_not_acknowledgements()
    {
        let token = uuid::Uuid::new_v4().to_string();
        let t = tempfile::tempdir().unwrap();
        let p = pid_file(t.path());
        for mode in [
            Mode::Wrong,
            Mode::Status,
            Mode::Duplicate,
            Mode::Redirect,
            Mode::Reflect,
        ] {
            let s = server(mode, &token).await;
            let w = witness(&s, &p);
            assert!(
                !w.probe(&token, Instant::now() + Duration::from_secs(1))
                    .await
                    .unwrap()
            );
            let req = s.requests.lock().unwrap();
            assert!(!req.is_empty());
            assert!(req.iter().all(|r| !r.contains("/redirect-target")));
        }
    }
    #[tokio::test]
    async fn body_limits_reject_declared_and_chunked_oversize_but_accept_the_boundary() {
        let token = uuid::Uuid::new_v4().to_string();
        let t = tempfile::tempdir().unwrap();
        let p = pid_file(t.path());
        for mode in [Mode::Length, Mode::Chunked] {
            let s = server(mode, &token).await;
            let w = witness(&s, &p);
            let e = w
                .probe(&token, Instant::now() + Duration::from_secs(1))
                .await
                .expect_err("oversized witness body");
            assert!(format!("{e:#}").contains("너무 큽니다"));
        }
        let s = server(Mode::Boundary, &token).await;
        assert!(
            witness(&s, &p)
                .probe(&token, Instant::now() + Duration::from_secs(1))
                .await
                .unwrap()
        );
    }
    #[tokio::test]
    async fn a_reply_after_the_shared_deadline_never_counts() {
        let token = uuid::Uuid::new_v4().to_string();
        let s = server(Mode::Late, &token).await;
        let t = tempfile::tempdir().unwrap();
        let p = pid_file(t.path());
        let w = witness(&s, &p);
        let master = Master::read(&p).unwrap();
        assert!(
            w.observe(&token, Instant::now() + Duration::from_millis(30), &master)
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn replacing_the_master_fails_even_when_a_reply_carries_the_expected_token() {
        let token = uuid::Uuid::new_v4().to_string();
        let s = server(Mode::Good, &token).await;
        let t = tempfile::tempdir().unwrap();
        let p = pid_file(t.path());
        let w = witness(&s, &p);
        let mut master = Master::read(&p).unwrap();
        master.start_ticks += 1;
        assert!(
            w.observe(&token, Instant::now() + Duration::from_secs(1), &master)
                .await
                .is_err()
        );
    }
    #[derive(Clone, Copy)]
    enum AckMode {
        Normal,
        Tamper,
        SlowSecond,
    }
    const FIRST_ACK_DELAY: Duration = Duration::from_secs(1);
    const SECOND_ACK_DELAY: Duration = Duration::from_secs(2);
    struct AckFixture {
        _temp: tempfile::TempDir,
        cfg: RuntimeConfig,
        witness: Witness,
        task: JoinHandle<()>,
        first: String,
        trace: PathBuf,
    }
    impl Drop for AckFixture {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    async fn ack_fixture(mode: AckMode) -> AckFixture {
        use axum::{Router, routing::get};
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let cfg = config(temp.path(), &format!("http://{addr}/nginx_status"));
        std::fs::create_dir_all(&cfg.nginx_conf_dir).unwrap();
        let first = uuid::Uuid::new_v4().to_string();
        let marker = cfg.nginx_conf_dir.join(MARKER);
        let active = temp.path().join("active");
        let trace = temp.path().join("trace");
        std::fs::write(&trace, "").unwrap();
        std::fs::write(
            &marker,
            format!("add_header X-NWUI-Config-Generation \"{first}\";\n"),
        )
        .unwrap();
        std::fs::write(cfg.nginx_conf_dir.join("server-untouched.conf"), "KEEP").unwrap();
        std::fs::write(
            &cfg.nginx_bin,
            format!(
                r#"#!/bin/sh
[ "$1" = -t ] && exit 0
cp '{marker}' '{active}'
token=$(cut -d '"' -f 2 '{active}')
printf 'HUP %s\n' "$token" >> '{trace}'
exit 0
"#,
                marker = marker.display(),
                active = active.display(),
                trace = trace.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&cfg.nginx_bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let slow_started = Arc::new([std::sync::OnceLock::new(), std::sync::OnceLock::new()]);
        let (seen, read_active, read_marker, read_first, read_trace) = (
            count.clone(),
            active.clone(),
            marker.clone(),
            first.clone(),
            trace.clone(),
        );
        let app = Router::new().route(
            "/nginx_status",
            get(move || {
                let slow_started = slow_started.clone();
                let (count, active, marker, first, trace) = (
                    seen.clone(),
                    read_active.clone(),
                    read_marker.clone(),
                    read_first.clone(),
                    read_trace.clone(),
                );
                async move {
                    use std::io::Write;
                    let text = std::fs::read_to_string(&active).unwrap_or_default();
                    let actual = text.split('"').nth(1).unwrap_or("stale").to_owned();
                    let n = count.fetch_add(1, Ordering::SeqCst);
                    let mut token = if matches!(mode, AckMode::Normal) && n < 2 {
                        "stale".to_owned()
                    } else {
                        actual
                    };
                    if matches!(mode, AckMode::SlowSecond) {
                        let phase = usize::from(token != first);
                        let delay = [FIRST_ACK_DELAY, SECOND_ACK_DELAY][phase];
                        // Reply promptly: each probe has its own one-second timeout.
                        if slow_started[phase].get_or_init(Instant::now).elapsed() < delay {
                            token = "stale".to_owned();
                        }
                    }
                    if matches!(mode, AckMode::Tamper) && token == first {
                        let _ = std::fs::OpenOptions::new()
                            .append(true)
                            .open(marker)
                            .and_then(|mut f| f.write_all(b"return 200;\n"));
                    }
                    if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(trace) {
                        let _ = writeln!(file, "HTTP {token}");
                    }
                    ([(HEADER, token)], "Active connections: 1\n")
                }
            }),
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let pid = pid_file(temp.path());
        let witness = Witness {
            http: client().unwrap(),
            url: url::Url::parse(&format!("http://{addr}/nginx_status")).unwrap(),
            pid_file: pid,
        };
        AckFixture {
            _temp: temp,
            cfg,
            witness,
            task,
            first,
            trace,
        }
    }
    #[tokio::test]
    async fn confirmation_waits_for_p_before_c_signals_once_each_and_changes_only_the_uuid() {
        let f = ack_fixture(AckMode::Normal).await;
        let original = std::fs::read_to_string(f.cfg.nginx_conf_dir.join(MARKER)).unwrap();
        f.witness
            .confirm(&f.cfg, &f.first, Instant::now() + Duration::from_secs(2))
            .await
            .unwrap();
        let updated = std::fs::read_to_string(f.cfg.nginx_conf_dir.join(MARKER)).unwrap();
        let second = updated.split('"').nth(1).unwrap();
        assert_ne!(second, f.first);
        assert_eq!(uuid::Uuid::parse_str(second).unwrap().to_string(), second);
        assert_eq!(updated.replace(second, &f.first), original);
        let trace = std::fs::read_to_string(&f.trace).unwrap();
        let rows: Vec<_> = trace.lines().collect();
        assert_eq!(
            rows.iter().filter(|r| r.starts_with("HUP ")).count(),
            2,
            "no HUP per retry: {trace}"
        );
        let pos = |kind: &str, token: &str| {
            rows.iter()
                .position(|r| *r == format!("{kind} {token}"))
                .unwrap()
        };
        assert!(pos("HUP", &f.first) < pos("HTTP", &f.first));
        assert!(pos("HTTP", &f.first) < pos("HUP", second));
        assert!(pos("HUP", second) < pos("HTTP", second));
        assert_eq!(
            std::fs::read_to_string(f.cfg.nginx_conf_dir.join("server-untouched.conf")).unwrap(),
            "KEEP"
        );
    }
    #[tokio::test]
    async fn a_modified_marker_template_is_not_silently_replaced_with_c() {
        let f = ack_fixture(AckMode::Tamper).await;
        assert!(
            f.witness
                .confirm(&f.cfg, &f.first, Instant::now() + Duration::from_secs(1))
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(&f.trace)
                .unwrap()
                .lines()
                .filter(|r| r.starts_with("HUP "))
                .count(),
            1
        );
    }
    #[tokio::test]
    async fn both_observations_share_the_same_deadline_without_a_phase_reset() {
        let f = ack_fixture(AckMode::SlowSecond).await;
        // Each ACK fits alone; together they exceed the budget, with headroom for phase one under load.
        let budget = SECOND_ACK_DELAY + FIRST_ACK_DELAY / 2;
        let start = Instant::now();
        assert!(
            f.witness
                .confirm(&f.cfg, &f.first, start + budget)
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(&f.trace)
                .unwrap()
                .lines()
                .filter(|r| r.starts_with("HUP "))
                .count(),
            2,
            "must reach phase two before testing its remaining budget"
        );
        assert!(start.elapsed() < budget + Duration::from_secs(2));
    }
    #[tokio::test]
    async fn the_signal_command_is_clamped_to_the_shared_ack_deadline() {
        let f = ack_fixture(AckMode::Normal).await;
        std::fs::write(&f.cfg.nginx_bin, "#!/bin/sh\nexec sleep 2\n").unwrap();
        let start = Instant::now();
        assert!(
            f.witness
                .confirm(&f.cfg, &f.first, start + Duration::from_millis(50))
                .await
                .is_err()
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn admission_waits_share_one_thirty_second_budget() {
        let budget = Budget {
            started: Instant::now() - Duration::from_secs(29),
            limit:OPERATION,
        };
        budget
            .admit(tokio::time::sleep(Duration::from_millis(100)))
            .await
            .unwrap();
        assert!(
            budget
                .admit(tokio::time::sleep(Duration::from_millis(1100)))
                .await
                .is_err()
        );
        assert!(budget.started.elapsed() < Duration::from_secs(31));
    }
    #[test]
    fn forward_work_preserves_the_restore_and_io_reserve() {
        assert!(Budget::new().forward().is_ok());
        assert!(
            Budget {
                started: Instant::now() - Duration::from_secs(220),
                limit:OPERATION,
            }
            .forward()
            .is_err()
        );
    }
}

#[cfg(test)]
#[path="g13_confirmation_kernel_tests.rs"]
mod g13_kernel;
