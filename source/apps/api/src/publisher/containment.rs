//! Attribute only known fatal nginx errors to managed units.
use super::files::RenderedConfig;
use crate::config::RuntimeConfig;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Unit {
    ConfFile(String),
    StreamFile(String),
    UpstreamBlock(String),
}

fn fatal(output: &str) -> Option<&str> {
    output.lines().find(|line| {
        ["[emerg]", "[alert]", "[crit]"]
            .iter()
            .any(|level| line.contains(level))
    })
}
/// Format the same attributed fatal line for apply, boot, and strict validation.
pub(super) fn reason(output:&str,prefixes:&[(&std::path::Path,&std::path::Path)])->String {
    let mut line=fatal(output).or_else(||output.lines().rev().find(|line|!line.trim().is_empty())).unwrap_or("").to_owned();
    for (from,to) in prefixes {line=line.replace(from.to_string_lossy().as_ref(),to.to_string_lossy().as_ref());}
    line.retain(|c|!c.is_control());
    fn operand(text:&str)->String {
        let end=text.find(['?','#']).unwrap_or(text.len());let text=&text[..end];
        let start=text.find("://").map_or(0,|at|at+3);
        let authority_end=text[start..].find('/').map_or(text.len(),|at|start+at);
        match text[start..authority_end].rfind('@') {Some(at)=>format!("{}{}",&text[..start],&text[start+at+1..]),None=>text.to_owned()}
    }
    let mut out=String::new();let mut chars=line.chars().peekable();
    while let Some(c)=chars.next() {
        if c=='"' || c=='\'' {
            out.push(c);let mut quoted=String::new();
            while chars.peek().is_some_and(|next|*next!=c) {quoted.push(chars.next().unwrap());}
            out.push_str(&operand(&quoted));if chars.peek()==Some(&c){out.push(chars.next().unwrap());}
        } else {
            let mut word=String::from(c);
            while chars.peek().is_some_and(|next|!next.is_whitespace() && *next!='"' && *next!='\'') {word.push(chars.next().unwrap());}
            if word.contains("://") {out.push_str(&operand(&word));}else{out.push_str(&word);}
        }
    }
    let mut end=out.len().min(1024);while !out.is_char_boundary(end){end-=1;}out.truncate(end);out
}

fn owned_file(kind: super::files::ManagedKind, name: &str) -> bool {
    let prefix = match kind {
        super::files::ManagedKind::Conf => "server-",
        super::files::ManagedKind::Stream => "stream-",
    };
    kind.matches(name)
        && name
            .strip_prefix(prefix)
            .and_then(|s| s.strip_suffix(".conf"))
            .is_some_and(|id| super::files::safe_id(id).is_ok())
}
pub(crate) fn attribute(
    output: &str,
    config: &RuntimeConfig,
    live: &RenderedConfig,
) -> Option<Vec<Unit>> {
    use super::{files::ManagedKind, guard_targets};
    use std::path::Path;
    let line = fatal(output)?.trim_end();
    if let Some((_, location)) = line.rsplit_once(" in ") {
        let (file, number) = location.rsplit_once(':')?;
        let number = number.parse::<usize>().ok()?.checked_sub(1)?;
        let path = Path::new(file);
        let name = path.file_name()?.to_str()?;
        if path.parent()? == config.nginx_conf_dir {
            if name == "upstreams.conf" {
                let text = std::str::from_utf8(live.conf.get(name)?).ok()?;
                let mut offset = 0;
                let target = text.split_inclusive('\n').nth(number)?;
                for l in text.split_inclusive('\n').take(number) {
                    offset += l.len();
                }
                let end = offset + target.trim_end_matches('\n').len();
                let matches: Vec<_> = guard_targets::upstream_blocks(text)
                    .into_iter()
                    .filter(|b| b.range.start < end && b.range.end > offset)
                    .collect();
                // A compressed line spanning several blocks cannot identify one safely.
                return (matches.len() == 1)
                    .then(|| vec![Unit::UpstreamBlock(matches[0].name.clone())]);
            }
            if owned_file(ManagedKind::Conf, name) && live.conf.contains_key(name) {
                return Some(vec![Unit::ConfFile(name.into())]);
            }
        } else if path.parent()? == config.nginx_stream_dir
            && owned_file(ManagedKind::Stream, name)
            && live.stream.contains_key(name)
        {
            return Some(vec![Unit::StreamFile(name.into())]);
        }
        return None;
    }
    for quoted in line.split('"').skip(1).step_by(2) {
        let path = Path::new(quoted);
        if !matches!(
            path.file_name().and_then(|n| n.to_str()),
            Some("fullchain.pem" | "privkey.pem")
        ) {
            continue;
        }
        let Some(parent) = path.parent() else {
            continue;
        };
        if parent.parent() != Some(config.certs_dir.as_path())
            || !parent
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|id| id.len() == 36 && uuid::Uuid::parse_str(id).is_ok())
        {
            continue;
        }
        let units: Vec<_> = live
            .conf
            .iter()
            .filter(|(name, bytes)| {
                owned_file(ManagedKind::Conf, name)
                    && std::str::from_utf8(bytes)
                        .is_ok_and(|text| guard_targets::references_certificate(text, quoted))
            })
            .map(|(name, _)| Unit::ConfFile(name.clone()))
            .collect();
        if !units.is_empty() {
            return Some(units);
        }
    }
    None
}

#[derive(Debug, Clone)]
pub(crate) struct Action {
    pub unit: Unit,
    pub kept_previous: bool,
    pub reason: String,
}
pub(crate) struct Report {
    pub validation: super::process::ValidationResult,
    pub actions: Vec<Action>,
    pub quarantine_dirs: Vec<std::path::PathBuf>,
}
#[derive(Clone, Copy)]
enum Strategy<'a> {
    Previous(&'a RenderedConfig),
    Quarantine(&'a std::path::Path),
}

fn upstream_bytes(files: &RenderedConfig, name: &str) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(files.conf.get("upstreams.conf")?).ok()?;
    let block = super::guard_targets::upstream_blocks(text)
        .into_iter()
        .find(|b| b.name == name)?;
    Some(text.as_bytes()[block.range].to_vec())
}
fn previous_bytes(previous: &RenderedConfig, unit: &Unit) -> Option<Vec<u8>> {
    match unit {
        Unit::ConfFile(name) => previous.conf.get(name).cloned(),
        Unit::StreamFile(name) => previous.stream.get(name).cloned(),
        Unit::UpstreamBlock(name) => upstream_bytes(previous, name),
    }
}
pub(super) fn set_unit(
    live: &mut RenderedConfig,
    unit: &Unit,
    previous: Option<Vec<u8>>,
) -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    match unit {
        Unit::ConfFile(name) => {
            if let Some(bytes) = previous {
                live.conf.insert(name.clone(), bytes);
            } else {
                live.conf.remove(name);
            }
        }
        Unit::StreamFile(name) => {
            if let Some(bytes) = previous {
                live.stream.insert(name.clone(), bytes);
            } else {
                live.stream.remove(name);
            }
        }
        Unit::UpstreamBlock(name) => {
            let text = std::str::from_utf8(
                live.conf
                    .get("upstreams.conf")
                    .context("Missing upstream file")?,
            )?;
            let mut next = Vec::new();
            let mut cursor = 0;
            let mut found = false;
            for block in super::guard_targets::upstream_blocks(text)
                .into_iter()
                .filter(|b| b.name == *name)
            {
                next.extend_from_slice(&text.as_bytes()[cursor..block.range.start]);
                if !found && let Some(bytes) = &previous {
                    next.extend_from_slice(bytes);
                }
                cursor = block.range.end;
                found = true;
            }
            ensure!(found, "Rejected upstream is no longer present");
            next.extend_from_slice(&text.as_bytes()[cursor..]);
            live.conf.insert("upstreams.conf".into(), next);
        }
    }
    Ok(())
}
fn can_keep(config: &RuntimeConfig, live: &RenderedConfig, unit: &Unit, bytes: &[u8]) -> bool {
    use super::{
        guard_targets::listen_ports,
        reserved_ports::{ListenContext, find_reserved_port_conflict},
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    if matches!(unit, Unit::UpstreamBlock(_)) {
        return true;
    }
    let mut reserved = super::bootstrap::ports_for_render(config);
    crate::dns_guard::reserve_port(&mut reserved, &crate::dns_guard::status(config));
    for (name, bytes) in &live.stream {
        if matches!(unit, Unit::StreamFile(current) if current == name) {
            continue;
        }
        reserved
            .stream_tcp
            .extend(listen_ports(&String::from_utf8_lossy(bytes), true));
    }
    let context = match unit {
        Unit::ConfFile(current) => {
            for (name,bytes) in &live.conf {
                if name==current {continue;}
                for (port,tls) in super::guard_targets::http_listens(&String::from_utf8_lossy(bytes)) {
                    if tls {reserved.http_tls.push(port);} else {reserved.http_plain.push(port);}
                }
            }
            return super::guard_targets::http_listens(text).into_iter().all(|(port,tls)|
                super::validation::assert_valid_http_port(f64::from(port),tls,&reserved).is_ok());
        },
        Unit::StreamFile(_) => {
            for bytes in live.conf.values() {
                reserved
                    .http
                    .extend(listen_ports(&String::from_utf8_lossy(bytes), true));
            }
            if listen_ports(text, true).is_empty() {
                ListenContext::StreamUdp
            } else {
                ListenContext::StreamTcp
            }
        }
        Unit::UpstreamBlock(_) => unreachable!(),
    };
    listen_ports(text, false)
        .into_iter()
        .all(|port| find_reserved_port_conflict(port, context, &reserved).is_none())
}

#[cfg(test)]
pub(crate) async fn contain_rendered_until(
    config:&RuntimeConfig,desired:&RenderedConfig,previous:&RenderedConfig,latest:std::time::Instant,
)->anyhow::Result<Report> {
    let live=desired.clone();let start=std::time::Instant::now();
    run(config,live,Strategy::Previous(previous),move || {
        if std::time::Instant::now()>=latest {start.elapsed().max(std::time::Duration::from_secs(60))}else{start.elapsed()}
    },None,None,None).await
}
pub(super) async fn contain_durable_until(
    config:&RuntimeConfig,desired:&RenderedConfig,previous:&RenderedConfig,latest:std::time::Instant,mutation:&mut super::mutation::Mutation,
)->anyhow::Result<Report> {
    let start=std::time::Instant::now();
    run(config,desired.clone(),Strategy::Previous(previous),move || {
        if std::time::Instant::now()>=latest {start.elapsed().max(std::time::Duration::from_secs(60))}else{start.elapsed()}
    },Some(mutation),None,None).await
}
pub(crate) async fn contain_on_disk(
    config: &RuntimeConfig,
    quarantine: &std::path::Path,
) -> anyhow::Result<Report> {
    let live = RenderedConfig {
        conf: super::files::managed_files(&config.nginx_conf_dir, super::files::ManagedKind::Conf)
            .await?,
        stream: super::files::managed_files(
            &config.nginx_stream_dir,
            super::files::ManagedKind::Stream,
        )
        .await?,
    };
    let start = std::time::Instant::now();
    run(config, live, Strategy::Quarantine(quarantine), move || start.elapsed(),None,None,None).await
}
pub(super) async fn contain_on_disk_durable(config:&RuntimeConfig,quarantine:&std::path::Path,writer:&mut super::derivation::Derivation)->anyhow::Result<Report> {
    let live=RenderedConfig{conf:super::files::managed_files(&config.nginx_conf_dir,super::files::ManagedKind::Conf).await?,stream:super::files::managed_files(&config.nginx_stream_dir,super::files::ManagedKind::Stream).await?};
    let start=std::time::Instant::now();run(config,live,Strategy::Quarantine(quarantine),move||start.elapsed(),None,Some(writer),None).await
}
pub(crate) fn explain_unconfirmed_previous(report:&mut Report) {
    const WHY:&str="직전 적용 결과를 확인하지 못해 이전 파일을 유지하지 않았습니다.";
    if report.actions.iter().any(|action|!action.kept_previous) {
        for action in report.actions.iter_mut().filter(|action|!action.kept_previous) {action.reason.push_str(&format!(" {WHY}"));}
        if report.validation.ok {report.validation.output.push_str(&format!("\n{WHY}"));}
    }
}
fn finish(
    mut validation: super::process::ValidationResult,
    actions: std::collections::BTreeMap<Unit, Action>,
    quarantine_dirs: Vec<std::path::PathBuf>,
) -> Report {
    if !actions.is_empty() {
        if !validation.ok {
            validation
                .output
                .push_str("\nnginx 오류 격리를 완료하지 못했습니다. 이전 설정으로 복구합니다.");
        }
        if validation.ok {
        for action in actions.values() {
            validation.output.push_str(&format!(
                "\n{}: {} — {}",
                match &action.unit {Unit::ConfFile(name)|Unit::StreamFile(name)|Unit::UpstreamBlock(name)=>name},
                if action.kept_previous {
                    "직전 적용본을 유지합니다"
                } else {
                    "이 항목은 지금 동작하지 않습니다"
                },
                action.reason
            ));
        }
        }
    }
    Report {
        validation,
        actions: actions.into_values().collect(),
        quarantine_dirs,
    }
}
struct BootView<'a> {
    main:&'a std::path::Path,
    store:&'a super::store::PublisherStore,
    data:&'a crate::control::model::ConfigSnapshotData,
    live_config:&'a RuntimeConfig,
    view_config:&'a RuntimeConfig,
    mode:super::boot::BootMode,
    #[cfg(test)] timeout:Option<std::time::Duration>,
}
fn boot_units(data:&crate::control::model::ConfigSnapshotData,files:&RenderedConfig)->Vec<Unit> {
    let mut units=Vec::new();
    for (rows,prefix,stream) in [(&data.servers,"server-",false),(&data.streams,"stream-",true)] {
        for row in rows {if let Some(id)=row["id"].as_str(){let name=format!("{prefix}{id}.conf");if if stream {files.stream.contains_key(&name)}else{files.conf.contains_key(&name)} {units.push(if stream{Unit::StreamFile(name)}else{Unit::ConfFile(name)});}}}
    }
    for row in &data.upstreams {if let Some(name)=row["name"].as_str(){if upstream_bytes(files,name).is_some(){units.push(Unit::UpstreamBlock(name.into()));}}}
    units
}
fn boot_consumers(data:&crate::control::model::ConfigSnapshotData,files:&RenderedConfig,units:&mut Vec<Unit>) {
    let removed=units.iter().filter_map(|unit|if let Unit::UpstreamBlock(name)=unit{Some(name.clone())}else{None}).collect::<Vec<_>>();
    for row in &data.servers {
        let Some(id)=row["id"].as_str() else{continue};let name=format!("server-{id}.conf");if !files.conf.contains_key(&name){continue;}
        let mut cfg=row["config"].clone();
        if let Some(upstream)=crate::control::internal_targets::referenced_upstream(&data.upstreams,row["upstreamId"].as_str(),None) {cfg["upstreamName"]=upstream["name"].clone();}
        if crate::control::render::effective_locations(&cfg).iter().any(|location| {
            crate::control::internal_targets::referenced_upstream(&data.upstreams,location["upstreamId"].as_str(),location["proxyPass"].as_str())
                .and_then(|upstream|upstream["name"].as_str()).is_some_and(|name|removed.iter().any(|removed|removed==name))
        }) {units.push(Unit::ConfFile(name));}
    }
    units.sort();units.dedup();
}
async fn write_boot_view(config:&RuntimeConfig,live:&RenderedConfig,data:&crate::control::model::ConfigSnapshotData)->anyhow::Result<()> {
    for (directory,files,kind) in [(&config.nginx_conf_dir,&live.conf,super::files::ManagedKind::Conf),(&config.nginx_stream_dir,&live.stream,super::files::ManagedKind::Stream)] {
        let owned=|name:&str|{
            let (prefix,rows)=if matches!(kind,super::files::ManagedKind::Conf){("server-",&data.servers)}else{("stream-",&data.streams)};
            name.strip_prefix(prefix).and_then(|id|id.strip_suffix(".conf")).is_some_and(|id|rows.iter().any(|row|row["id"].as_str()==Some(id)))
        };
        for (name,bytes) in files {if owned(name) || (matches!(kind,super::files::ManagedKind::Conf)&&name=="upstreams.conf") {super::files::atomic_write(&directory.join(name),bytes,0o600).await?;}}
        let existing=super::files::managed_files(directory,kind).await?;
        for name in existing.keys().filter(|name|owned(name) && !files.contains_key(*name)) {tokio::fs::remove_file(directory.join(name)).await?;}
        std::fs::File::open(directory)?.sync_all()?;
    }
    Ok(())
}
fn finish_run(validation:super::process::ValidationResult,mut actions:std::collections::BTreeMap<Unit,Action>,quarantine_dirs:Vec<std::path::PathBuf>,live:&RenderedConfig,boot:Option<&BootView<'_>>)->Report {
    if !validation.ok {if let Some(boot)=boot {if boot.mode==super::boot::BootMode::Ready {
        let why=format!("시작 시 nginx 오류 격리를 끝내지 못해 함께 제외했습니다: {}",reason(&validation.output,&[(boot.main,boot.live_config.nginx_main_conf.as_path()),(boot.view_config.nginx_conf_dir.as_path(),boot.live_config.nginx_conf_dir.as_path()),(boot.view_config.nginx_stream_dir.as_path(),boot.live_config.nginx_stream_dir.as_path()),(boot.view_config.certs_dir.as_path(),boot.live_config.certs_dir.as_path())]));
        for unit in boot_units(boot.data,live) {actions.entry(unit.clone()).or_insert(Action{unit,kept_previous:false,reason:why.clone()});}
    }}}
    if boot.is_some(){Report{validation,actions:actions.into_values().collect(),quarantine_dirs}}else{finish(validation,actions,quarantine_dirs)}
}
pub(super) async fn contain_staged(
    config:&RuntimeConfig,files:&RenderedConfig,main:&std::path::Path,store:&super::store::PublisherStore,
    data:&crate::control::model::ConfigSnapshotData,live_config:&RuntimeConfig,mode:super::boot::BootMode,
)->anyhow::Result<Report> {
    let started=std::time::Instant::now();let mut boot=BootView{main,store,data,live_config,view_config:config,mode,#[cfg(test)] timeout:None};
    run(config,files.clone(),Strategy::Previous(&RenderedConfig::default()),move||started.elapsed(),None,None,Some(&mut boot)).await
}

async fn run(
    config: &RuntimeConfig,
    mut live: RenderedConfig,
    strategy: Strategy<'_>,
    elapsed: impl Fn() -> std::time::Duration + Send + Sync,
    mut mutation:Option<&mut super::mutation::Mutation>,
    mut derivation:Option<&mut super::derivation::Derivation>,
    mut boot:Option<&mut BootView<'_>>,
) -> anyhow::Result<Report> {
    use std::collections::BTreeMap;
    use std::time::Duration;
    let process = super::process::NginxProcess::new(&config.nginx_bin);
    let mut actions: BTreeMap<Unit, Action> = BTreeMap::new();
    let mut quarantine_dirs = Vec::new();
    let mut previous_failure = None;
    // Round 0 plus at most ten corrective rounds; an in-flight validation has its own 30 s limit.
    let last_round=boot.as_ref().map_or(10,|boot|boot_units(boot.data,&live).len());
    for round in 0..=last_round {
        if elapsed() >= Duration::from_secs(60) {
            let validation=previous_failure.take().unwrap_or(super::process::ValidationResult {
                ok:false,output:"nginx 오류 격리의 새 작업을 시작할 시간이 없습니다.".into(),
            });
            return Ok(finish_run(validation,actions,quarantine_dirs,&live,boot.as_deref()));
        }
        if let Some(boot)=boot.as_deref(){write_boot_view(config,&live,boot.data).await?;}else if let Some(writer)=mutation.as_deref_mut() {writer.write_candidate(config,&live).await?;}else if let Some(writer)=derivation.as_deref_mut(){writer.write(config,&live,"guard-rejected").await?;}else{super::files::write_rendered(config, &live).await?;}
        if elapsed() >= Duration::from_secs(60) {
            let validation = previous_failure
                .take()
                .unwrap_or(super::process::ValidationResult {
                    ok: false,
                    output: "nginx 오류 격리의 파일 쓰기 시간이 제한을 넘었습니다.".into(),
                });
            return Ok(finish_run(validation, actions, quarantine_dirs,&live,boot.as_deref()));
        }
        let mut timed_out=false;
        let validation = if let Some(boot)=boot.as_deref_mut(){
            let timeout=Duration::from_secs(30);
            #[cfg(test)] let timeout=boot.timeout.unwrap_or(timeout);
            match super::command::validate_candidate(boot.store.clone(),config.nginx_bin.clone(),Some(boot.main.into()),timeout).await? {
                super::command::CommandResult::Accepted(output)=>super::process::ValidationResult{ok:true,output},
                super::command::CommandResult::Rejected(output)=>super::process::ValidationResult{ok:false,output},
                super::command::CommandResult::TimedOut=>{timed_out=true;super::process::ValidationResult{ok:false,output:"nginx 설정 검증 명령의 제한 시간이 지났습니다.".into()}},
            }
        }else if let Some(writer)=mutation.as_deref_mut() {writer.validate(config,Duration::from_secs(30)).await}else if let Some(writer)=derivation.as_deref_mut(){writer.validate(config).await}else{process.validate().await};
        let mut units = if validation.ok || timed_out {
            None
        } else {
            attribute(&validation.output, config, &live)
        };
        if let (Some(boot),Some(units))=(boot.as_deref(),units.as_mut()) {
            let allowed=boot_units(boot.data,&live);
            if units.iter().all(|unit|allowed.contains(unit)){boot_consumers(boot.data,&live,units);}else{units.clear();}
        }
        let stop = validation.ok
            || timed_out
            || round == last_round
            || elapsed() >= Duration::from_secs(60)
            || units.as_ref().is_none_or(|units| {
                units.is_empty() || units
                    .iter()
                    .any(|u| actions.get(u).is_some_and(|a| !a.kept_previous))
            });
        if stop {
            return Ok(finish_run(validation, actions, quarantine_dirs,&live,boot.as_deref()));
        }
        previous_failure = Some(validation.clone());
        let units = units.unwrap();
        let prefixes=boot.as_deref().map(|boot|vec![(boot.main,boot.live_config.nginx_main_conf.as_path()),(config.nginx_conf_dir.as_path(),boot.live_config.nginx_conf_dir.as_path()),(config.nginx_stream_dir.as_path(),boot.live_config.nginx_stream_dir.as_path()),(config.certs_dir.as_path(),boot.live_config.certs_dir.as_path())]).unwrap_or_default();
        let reason = reason(&validation.output,&prefixes);
        if let Strategy::Quarantine(root) = strategy {
            let mut conf = BTreeMap::new();
            let mut stream = BTreeMap::new();
            for unit in &units {
                match unit {
                    Unit::ConfFile(name) => {
                        conf.insert(name.clone(), live.conf[name].clone());
                    }
                    Unit::StreamFile(name) => {
                        stream.insert(name.clone(), live.stream[name].clone());
                    }
                    Unit::UpstreamBlock(_) => {
                        conf.insert("upstreams.conf".into(), live.conf["upstreams.conf"].clone());
                    }
                }
            }
            if let Some(dir) =
                super::bootstrap::isolate_with_writer(config, root, "rejected", &conf, &stream,derivation.as_deref_mut()).await?
            {
                quarantine_dirs.push(dir);
            }
        }
        for unit in units {
            let previous = match strategy {
                Strategy::Previous(previous) if boot.is_none() && !actions.contains_key(&unit) => {
                    previous_bytes(previous, &unit)
                        .filter(|bytes| can_keep(config, &live, &unit, bytes))
                }
                _ => None,
            };
            let kept_previous = previous.is_some();
            set_unit(&mut live, &unit, previous)?;
            actions
                .entry(unit.clone())
                .and_modify(|a| a.kept_previous = kept_previous)
                .or_insert(Action {
                    unit,
                    kept_previous,
                    reason: reason.clone(),
                });
        }
        // Quarantine moved the original away; persist its corrected replacement before cutoff.
        if matches!(strategy, Strategy::Quarantine(_)) {
            if let Some(writer)=derivation.as_deref_mut(){writer.write(config,&live,"guard-rejected").await?;}else{super::files::write_rendered(config, &live).await?;}
        }
    }
    unreachable!()
}

// Keep eligibility is preloaded once by the caller; the loop never borrows a DB client.
pub(crate) fn eligible_previous(
    backup: &super::files::FileBackup,
    snapshot: &super::apply::ConfigSnapshotData,
) -> RenderedConfig {
    use std::collections::BTreeSet;
    let servers: BTreeSet<_> = snapshot
        .servers
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    let streams: BTreeSet<_> = snapshot
        .streams
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    let upstreams: BTreeSet<_> = snapshot
        .upstreams
        .iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    let mut eligible = RenderedConfig::default();
    for (name, bytes) in &backup.conf {
        if name == "upstreams.conf" {
            if let Ok(text) = std::str::from_utf8(bytes) {
                let mut selected = Vec::new();
                for block in super::guard_targets::upstream_blocks(text)
                    .into_iter()
                    .filter(|b| upstreams.contains(b.name.as_str()))
                {
                    selected.extend_from_slice(&text.as_bytes()[block.range]);
                }
                eligible.conf.insert(name.clone(), selected);
            }
        } else if name
            .strip_prefix("server-")
            .and_then(|n| n.strip_suffix(".conf"))
            .is_some_and(|id| servers.contains(id))
        {
            eligible.conf.insert(name.clone(), bytes.clone());
        }
    }
    for (name, bytes) in &backup.stream {
        if name
            .strip_prefix("stream-")
            .and_then(|n| n.strip_suffix(".conf"))
            .is_some_and(|id| streams.contains(id))
        {
            eligible.stream.insert(name.clone(), bytes.clone());
        }
    }
    eligible
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    fn config() -> RuntimeConfig {
        RuntimeConfig::read(|key| {
            Some(match key {
                "DATABASE_URL" => "postgres://test:test@127.0.0.1:1/test".into(),
                "AUTH_SECRET" => "test-secret".into(),
                "NGINX_CONF_D" => "/tmp/m1-real/conf".into(),
                "NGINX_STREAM_D" => "/tmp/m1-real/stream".into(),
                "CERTS_DIR" => "/tmp/m1-real/certs".into(),
                _ => return None,
            })
        })
        .unwrap()
    }
    fn fixtures() -> Vec<Value> {
        serde_json::from_str::<Value>(include_str!(
            "../../tests/fixtures/containment-nginx-1.30.6.json"
        ))
        .unwrap()["cases"]
            .as_array()
            .unwrap()
            .clone()
    }
    fn live(case: &Value) -> RenderedConfig {
        let mut live = RenderedConfig::default();
        for (name, body) in case["files"].as_object().unwrap() {
            let (dir, file) = name.split_once('/').unwrap();
            let map = if dir == "conf" {
                &mut live.conf
            } else {
                &mut live.stream
            };
            map.insert(file.into(), body.as_str().unwrap().as_bytes().to_vec());
        }
        live
    }
    #[test]
    fn native_fatal_formats_map_to_the_exact_managed_unit() {
        for f in fixtures() {
            let expected = match f["name"].as_str().unwrap() {
                "server-directive" | "cert-load" | "key-mismatch" => {
                    Some(vec![Unit::ConfFile("server-bad.conf".into())])
                }
                "upstream-host" => Some(vec![Unit::UpstreamBlock("broken_group".into())]),
                "duplicate-upstream" => Some(vec![Unit::UpstreamBlock("duplicate".into())]),
                "stream-value" => Some(vec![Unit::StreamFile("stream-bad.conf".into())]),
                "main-directive" => None,
                _ => panic!("unhandled native fixture"),
            };
            assert_eq!(
                attribute(f["stderr"].as_str().unwrap(), &config(), &live(&f)),
                expected,
                "{}",
                f["name"]
            );
        }
    }
    #[test]
    fn only_the_first_fatal_counts_and_warnings_never_select_units() {
        let f = fixtures().remove(0);
        let live = live(&f);
        let fatal = f["stderr"].as_str().unwrap();
        assert_eq!(
            attribute(
                &format!("nginx: [warn] harmless\n{fatal}"),
                &config(),
                &live
            ),
            Some(vec![Unit::ConfFile("server-bad.conf".into())])
        );
        assert_eq!(
            attribute(
                "nginx: [warn] in /tmp/m1-real/conf/server-bad.conf:1",
                &config(),
                &live
            ),
            None
        );
        assert_eq!(
            attribute(
                &format!("nginx: [emerg] global failure\n{fatal}"),
                &config(),
                &live
            ),
            None
        );
    }
    #[test]
    fn unknown_unsafe_outside_and_control_paths_are_never_containment_units() {
        let mut live = RenderedConfig::default();
        for name in [
            "server-bad.conf",
            "server-bad$.conf",
            "00-runtime-upstreams.conf",
            ".nwui-generation",
        ] {
            live.conf.insert(name.into(), b"server {}".to_vec());
        }
        for p in [
            "/tmp/m1-real/conf-other/server-bad.conf",
            "/tmp/m1-real/conf/server-missing.conf",
            "/tmp/m1-real/conf/server-../bad.conf",
            "/tmp/m1-real/conf/server-bad$.conf",
            "/tmp/m1-real/conf/00-runtime-upstreams.conf",
            "/tmp/m1-real/conf/.nwui-generation",
        ] {
            assert_eq!(
                attribute(&format!("nginx: [emerg] bad in {p}:1"), &config(), &live),
                None,
                "{p}"
            );
        }
    }
    #[test]
    fn upstream_line_outside_blocks_or_ambiguous_on_one_line_is_not_guessed() {
        let mut live = RenderedConfig::default();
        live.conf.insert(
            "upstreams.conf".into(),
            b"upstream a {}\n\nupstream b {}\n".to_vec(),
        );
        for n in [0, 2, 99] {
            assert_eq!(
                attribute(
                    &format!("nginx: [emerg] invalid in /tmp/m1-real/conf/upstreams.conf:{n}"),
                    &config(),
                    &live
                ),
                None
            );
        }
        live.conf.insert(
            "upstreams.conf".into(),
            b"upstream a {} upstream b {}\n".to_vec(),
        );
        assert_eq!(
            attribute(
                "nginx: [emerg] invalid in /tmp/m1-real/conf/upstreams.conf:1",
                &config(),
                &live
            ),
            None
        );
    }
    #[test]
    fn certificate_paths_match_all_exact_references_without_prefix_or_comment_matches() {
        let f = fixtures()
            .into_iter()
            .find(|v| v["name"] == "cert-load")
            .unwrap();
        let mut live = live(&f);
        let p = "/tmp/m1-real/certs/51f19cb6-336c-4fb0-a3cb-5cfe55ec98f0/fullchain.pem";
        live.conf.insert(
            "server-also.conf".into(),
            format!("server {{ ssl_certificate \"{p}\"; }}").into_bytes(),
        );
        live.conf.insert(
            "server-lookalike.conf".into(),
            format!("server {{ ssl_certificate {p}.backup; }}").into_bytes(),
        );
        live.conf.insert(
            "server-comment.conf".into(),
            format!("# ssl_certificate {p};\nserver {{}}").into_bytes(),
        );
        assert_eq!(
            attribute(f["stderr"].as_str().unwrap(), &config(), &live),
            Some(vec![
                Unit::ConfFile("server-also.conf".into()),
                Unit::ConfFile("server-bad.conf".into())
            ])
        );
    }
    async fn cutoff_fixture(kind: &str) -> (tempfile::TempDir, RuntimeConfig, RenderedConfig) {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let mut config = config();
        config.nginx_conf_dir = temp.path().join("conf");
        config.nginx_stream_dir = temp.path().join("stream");
        config.certs_dir = temp.path().join("certs");
        config.nginx_bin = temp.path().join("nginx");
        let log = temp.path().join("calls");
        let output = if kind == "good" { "exit 0".to_string() } else {
            format!("echo 'nginx: [emerg] invalid in {}/upstreams.conf:2'; exit 1", config.nginx_conf_dir.display())
        };
        std::fs::write(&config.nginx_bin, format!("#!/bin/sh\necho check >> '{}'\n{output}\n",log.display())).unwrap();
        std::fs::set_permissions(&config.nginx_bin,std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut live=RenderedConfig::default();
        live.conf.insert("upstreams.conf".into(),b"upstream good { server 192.0.2.1:80; }\nupstream bad { BAD; }\n".to_vec());
        (temp,config,live)
    }
    #[tokio::test]
    async fn a_quarantine_crossing_the_cutoff_leaves_the_corrected_upstream_file_on_disk() {
        let (temp,config,live)=cutoff_fixture("bad").await;
        let quarantine=temp.path().join("quarantine");
        let report=run(&config,live,Strategy::Quarantine(&quarantine),|| {
            if quarantine.exists() {std::time::Duration::from_secs(60)}else{std::time::Duration::ZERO}
        },None,None,None).await.unwrap();
        assert!(!report.validation.ok);
        let kept=std::fs::read_to_string(config.nginx_conf_dir.join("upstreams.conf")).expect("cutoff cannot leave the whole upstream file absent");
        assert!(kept.contains("upstream good"));assert!(!kept.contains("upstream bad"));
    }
    #[tokio::test]
    async fn no_validation_starts_at_the_sixty_second_cutoff() {
        let(temp,config,live)=cutoff_fixture("bad").await;let previous=RenderedConfig::default();
        let report=run(&config,live,Strategy::Previous(&previous),||std::time::Duration::from_secs(60),None,None,None).await.unwrap();
        assert!(!report.validation.ok);assert!(!temp.path().join("calls").exists());assert!(!config.nginx_conf_dir.join("upstreams.conf").exists(),"no write starts after the cutoff");
    }
    #[tokio::test]
    async fn a_validation_started_before_the_cutoff_may_finish_but_no_retry_starts() {
        let(temp,config,live)=cutoff_fixture("bad").await;let previous=RenderedConfig::default();let calls=temp.path().join("calls");
        let report=run(&config,live,Strategy::Previous(&previous),||if calls.exists(){std::time::Duration::from_secs(60)}else{std::time::Duration::from_millis(59999)},None,None,None).await.unwrap();
        assert!(!report.validation.ok);assert_eq!(std::fs::read_to_string(calls).unwrap().lines().count(),1);
    }
    #[tokio::test]
    async fn a_final_inflight_validation_can_confirm_its_healthy_set() {
        let(temp,config,live)=cutoff_fixture("good").await;let previous=RenderedConfig::default();let calls=temp.path().join("calls");
        let report=run(&config,live,Strategy::Previous(&previous),||if calls.exists(){std::time::Duration::from_secs(89)}else{std::time::Duration::from_millis(59999)},None,None,None).await.unwrap();
        assert!(report.validation.ok);assert_eq!(std::fs::read_to_string(calls).unwrap().lines().count(),1);
    }

    #[test]
    fn keeping_previous_http_bytes_respects_the_shared_plain_tls_port_rule() {
        let temp=tempfile::tempdir().unwrap();let mut config=config();config.target_dns.descriptor_path=temp.path().join("dns.json");
        let unit=Unit::ConfFile("server-old.conf".into());
        for (current,previous) in [("server { listen 18443 ssl; }","server { listen 18443; }"),("server { listen 18080; }","server { listen 18080 ssl; }")] {
            let mut live=RenderedConfig::default();live.conf.insert("server-other.conf".into(),current.as_bytes().to_vec());
            assert!(!can_keep(&config,&live,&unit,previous.as_bytes()),"old bytes must not reintroduce a plain/TLS collision");
        }
        assert!(!can_keep(&config,&RenderedConfig::default(),&unit,b"server { listen 8080; }"));
    }

}

#[cfg(test)]
#[path="g13_containment_kernel_tests.rs"]
mod g13_kernel;

#[cfg(all(test,target_os="linux"))]
#[path="boot_containment_tests.rs"]
mod boot_tests;
