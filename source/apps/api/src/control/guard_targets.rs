//! Internal targets in the managed files nginx is about to load.
//!
//! Apply refuses a reverse proxy, stream or upstream whose target reaches this container's web,
//! API or database port (`internal_targets`). nginx resolves names again every time it starts,
//! and s6 restarts it without an apply, so the guard in front of each start checks the rendered
//! files with the same rule. It has no database: it reads what apply wrote.
//!
//! What is removed follows apply's skip rules: an upstream block together with every server
//! that routes to it by name, a server file, a stream file. Everything else stays live.
use super::internal_targets::{Resolve, check_host_port_with, check_url_with};
use crate::config::RuntimeConfig;
use std::collections::{BTreeMap, BTreeSet};

pub const UPSTREAMS_FILE: &str = "upstreams.conf";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct TargetScan {
    /// Managed conf.d files to move out, including `upstreams.conf` when blocks were removed.
    pub conf: BTreeSet<String>,
    pub stream: BTreeSet<String>,
    /// `upstreams.conf` without the removed blocks, to write back once the original is saved.
    pub upstreams: Option<Vec<u8>>,
    pub removed_upstreams: Vec<String>,
}
impl TargetScan {
    pub fn is_empty(&self) -> bool {
        self.conf.is_empty() && self.stream.is_empty()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum End {
    Semicolon,
    Open,
    Close,
}
#[derive(Debug)]
pub(crate) struct Statement {
    pub(crate) words: Vec<String>,
    pub(crate) end: End,
    /// Byte offset of the first word (of the `}` for a close).
    start: usize,
    /// Byte offset just past the terminating `;`, `{` or `}`.
    stop: usize,
}

impl Statement {
    pub(crate) fn end_offset(&self) -> usize { self.stop }
    pub(crate) fn range(&self) -> std::ops::Range<usize> { self.start..self.stop }
}

/// Statements in nginx syntax. `#` at the start of a word starts a comment; quotes group a word and a
/// backslashes decode nginx escapes and preserve unknown escapes. The renderer quotes only header
/// values, but a comment or quoted value must never be taken for a target.
pub(crate) fn statements(text: &str) -> Vec<Statement> {
    let mut out = Vec::new();
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut start = None;
    let mut chars = text.char_indices().peekable();
    let finish = |word: &mut Option<String>, words: &mut Vec<String>| {
        if let Some(w) = word.take() {
            words.push(w);
        }
    };
    let escaped_char = |word: &mut String, escaped| {
        match escaped {
            'n' => word.push('\n'),
            'r' => word.push('\r'),
            't' => word.push('\t'),
            '\'' | '"' | '\\' => word.push(escaped),
            _ => { word.push('\\'); word.push(escaped); }
        }
    };
    while let Some((i, c)) = chars.next() {
        match c {
            '"' | '\'' if word.is_none() => {
                start.get_or_insert(i);
                let current = word.get_or_insert_with(String::new);
                while let Some((_, q)) = chars.next() {
                    match q {
                        '\\' => {
                            if let Some((_, escaped)) = chars.next() {
                                escaped_char(current, escaped);
                            }
                        }
                        q if q == c => break,
                        q => current.push(q),
                    }
                }
            }
            // Only at the start of a word, as in nginx: `a#b` is one word.
            '#' if word.is_none() => {
                finish(&mut word, &mut words);
                while chars.next_if(|(_, c)| *c != '\n').is_some() {}
            }
            // As in nginx: `${` continues a word (a variable), and `}` inside a word is a
            // character; only at the start of a word does `}` close a block.
            '{' if word.as_ref().is_some_and(|w| w.ends_with('$')) => {
                word.get_or_insert_with(String::new).push(c);
            }
            '}' if word.is_some() => word.get_or_insert_with(String::new).push(c),
            ';' | '{' | '}' => {
                finish(&mut word, &mut words);
                let end = match c {
                    ';' => End::Semicolon,
                    '{' => End::Open,
                    _ => End::Close,
                };
                out.push(Statement {
                    words: std::mem::take(&mut words),
                    start: start.take().unwrap_or(i),
                    stop: i + 1,
                    end,
                });
            }
            ' ' | '\t' | '\r' | '\n' => finish(&mut word, &mut words),
            '\\' => {
                start.get_or_insert(i);
                if let Some((_, escaped)) = chars.next() {
                    escaped_char(word.get_or_insert_with(String::new), escaped);
                }
            }
            c => {
                start.get_or_insert(i);
                word.get_or_insert_with(String::new).push(c);
            }
        }
    }
    out
}

fn listen_port(words:&[String])->Option<u16> {
    let raw=words.get(1)?;
    raw.parse::<u16>().ok().or_else(||raw.rsplit_once(':').and_then(|(_,p)|p.parse().ok())).filter(|p|*p>0)
}
/// Shared lexer keeps same-line/address-bound listeners visible to restart reservation checks.
pub(crate) fn listen_ports(text:&str,tcp_only:bool)->Vec<u16> {
    statements(text).into_iter().filter_map(|s| {
        if s.end!=End::Semicolon||s.words.first().map(String::as_str)!=Some("listen") {return None;}
        if tcp_only&&s.words.iter().any(|w|w=="udp"){return None;}
        listen_port(&s.words)
    }).collect()
}
pub(crate) fn http_listens(text:&str)->Vec<(u16,bool)> {
    statements(text).into_iter().filter_map(|s| {
        if s.end!=End::Semicolon||s.words.first().map(String::as_str)!=Some("listen") {return None;}
        Some((listen_port(&s.words)?,s.words.iter().any(|w|w=="ssl")))
    }).collect()
}

pub(crate) struct UpstreamBlock {
    pub(crate) name: String,
    servers: Vec<String>,
    /// Only exact down entries with their adjacent generated comment may bypass
    /// the live-target check (port 1 itself can be an internal service port).
    pub(crate) placeholders: BTreeSet<usize>,
    pub(crate) invalid_placeholder: bool,
    directives: Vec<Vec<String>>,
    /// Byte range of the whole block, through its closing brace and line end.
    pub(crate) range: std::ops::Range<usize>,
}
/// The comment is part of the unavailable-member grammar, not arbitrary text
/// that can exempt a live target. Blank/intervening lines break adjacency.
fn placeholder(text:&str,start:usize,words:&[String])->Option<bool> {
    let prefix=&text[..start];
    let (before,indent)=prefix.rsplit_once('\n').unwrap_or(("",prefix));
    let previous=before.rsplit('\n').next().unwrap_or("").trim();
    let marked=previous.starts_with("# runtime DNS unavailable");
    let down=words.get(1).is_some_and(|w|w=="127.0.0.1:1")&&words.iter().any(|w|w=="down");
    if !marked&&!down{return None;}
    let validate=|| {
        if !indent.chars().all(|c|c==' '||c=='\t'){return false;}
        let Some(original)=previous.strip_prefix("# runtime DNS unavailable: ") else{return false;};
        if !super::internal_targets::is_dns_endpoint(original){return false;}
        let member:Vec<_>=original.split_whitespace().collect();
        if member.len()>2{return false;}
        if member.len()==2&&!member[1].strip_prefix("weight=").is_some_and(|v|v.parse::<u32>().is_ok_and(|v|v>0)){return false;}
        let mut expected=vec!["server","127.0.0.1:1"];
        expected.extend(member.iter().skip(1).copied());expected.push("down");
        words.iter().map(String::as_str).eq(expected)
    };
    Some(validate())
}
pub(crate) fn upstream_blocks(text: &str) -> Vec<UpstreamBlock> {
    let mut blocks = Vec::new();
    let mut depth = 0usize;
    let mut open: Option<UpstreamBlock> = None;
    for s in statements(text) {
        match s.end {
            End::Open => {
                if depth == 0 && s.words.len() == 2 && s.words[0] == "upstream" {
                    open = Some(UpstreamBlock {
                        name: s.words[1].clone(),
                        servers: Vec::new(),
                        placeholders: BTreeSet::new(),
                        invalid_placeholder: false,
                        directives: Vec::new(),
                        range: s.start..s.stop,
                    });
                }
                depth += 1;
            }
            End::Close => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Some(mut block) = open.take() {
                        let stop = if text[s.stop..].starts_with('\n') { s.stop + 1 } else { s.stop };
                        block.range.end = stop;
                        blocks.push(block);
                    }
                }
            }
            End::Semicolon => {
                if depth == 1 {
                    if let Some(block)=open.as_mut() {
                        if s.words.len()>=2 && s.words[0]=="server" {
                            match placeholder(text,s.start,&s.words) {
                                Some(true)=>{block.placeholders.insert(block.servers.len());},
                                Some(false)=>block.invalid_placeholder=true,
                                None=>{},
                            }
                            block.servers.push(s.words[1..].join(" "));
                        }
                        block.directives.push(s.words);
                    }
                }
            }
        }
    }
    blocks
}

fn proxy_targets(text: &str) -> Vec<String> {
    statements(text)
        .into_iter()
        .filter(|s| s.end == End::Semicolon && s.words.len() >= 2 && s.words[0] == "proxy_pass")
        .map(|s| s.words[1].clone())
        .filter(|target|!super::internal_socket::is_unix_target(target))
        .collect()
}

/// A resolve token is never an exemption by itself. Only the generated supported
/// zone/resolver structure may defer a hostname to the filtered process.
#[derive(Clone,Copy)]
enum GroupKind { Named, Direct, Stream }
fn full_uuid(raw:&str)->bool {raw.len()==32&&raw.bytes().all(|c|c.is_ascii_hexdigit())}
fn valid_zone(block:&UpstreamBlock,zone:&str,kind:GroupKind)->bool {
    match kind {
        GroupKind::Named=>zone.strip_prefix("nwui_resolve_upstream_").is_some_and(full_uuid),
        GroupKind::Stream=>zone==block.name&&zone.strip_prefix("nwui_resolve_stream_").is_some_and(full_uuid),
        GroupKind::Direct=>zone==block.name&&zone.strip_prefix("nwui_resolve_").and_then(|s|s.rsplit_once('_')).is_some_and(|(id,index)|full_uuid(id)&&index.parse::<usize>().is_ok_and(|v|v.to_string()==index)),
    }
}
fn unsafe_block(config:&RuntimeConfig,block:&UpstreamBlock,resolve:Resolve,kind:GroupKind)->bool {
    if block.invalid_placeholder{return true;}
    if !block.placeholders.is_empty() {
        // Unavailable direct HTTP routes are return 502, never placeholder groups.
        if matches!(kind,GroupKind::Direct){return true;}
        let zones:Vec<_>=block.directives.iter().filter(|d|d.first().is_some_and(|w|w=="zone")).collect();
        if zones.len()!=1||zones[0].len()!=3||zones[0][2]!="64k"||!valid_zone(block,&zones[0][1],kind){return true;}
        if block.directives.iter().any(|d|!matches!(d.first().map(String::as_str),Some("zone"|"server"))){return true;}
    }
    let dynamic=block.servers.iter().any(|s|s.split_whitespace().any(|w|w=="resolve"));
    let has_resolver=block.directives.iter().any(|d|d.first().is_some_and(|w|w=="resolver"));
    if dynamic||has_resolver {
        let descriptor=crate::dns_guard::status(config);
        if !descriptor.available{return true;}
        let directives=|name:&str|block.directives.iter().filter(|d|d.first().is_some_and(|w|w==name)).collect::<Vec<_>>();
        let zones=directives("zone");let resolvers=directives("resolver");let timeouts=directives("resolver_timeout");
        if zones.len()!=1||resolvers.len()!=1||timeouts.len()!=1{return true;}
        let zone=zones[0];
        if zone.len()!=3||zone[2]!="64k"||!valid_zone(block,&zone[1],kind){return true;}
        let dns=resolvers[0];
        if dns.len()<3||dns.len()>4||dns[1]!=descriptor.endpoint.to_string()||dns[2]!="valid=10s" {return true;}
        if dns.len()==4&&(config.target_dns.ipv6||dns[3]!="ipv6=off"){return true;}
        if timeouts[0].as_slice()!=["resolver_timeout","2s"]{return true;}
        if block.directives.iter().any(|d|!matches!(d.first().map(String::as_str),Some("zone"|"resolver"|"resolver_timeout"|"server"))){return true;}
    }
    block.servers.iter().enumerate().any(|(index,server)| {
        if block.placeholders.contains(&index){return false;}
        let words:Vec<_>=server.split_whitespace().collect();
        if words.iter().any(|w|*w=="resolve") {
            if words.iter().filter(|w|**w=="resolve").count()!=1||!super::internal_targets::is_dns_endpoint(server){return true;}
            if words.iter().skip(1).any(|w|!matches!(*w,"resolve"|"down"|"backup")
                &&!w.strip_prefix("weight=").is_some_and(|v|v.parse::<u32>().is_ok_and(|v|v>0))){return true;}
            super::internal_targets::check_dynamic_host_port(config,server).is_err()
        }else{check_host_port_with(config,server,resolve).is_err()}
    })
}

fn group_safety(config:&RuntimeConfig,files:&BTreeMap<String,Vec<u8>>,resolve:Resolve,stream:bool)->BTreeMap<String,bool> {
    let mut names=BTreeMap::new();
    for (file,bytes) in files {
        let kind=if stream{GroupKind::Stream}else if file==UPSTREAMS_FILE{GroupKind::Named}else{GroupKind::Direct};
        for block in upstream_blocks(&String::from_utf8_lossy(bytes)) {
            let key=block.name.to_ascii_lowercase();let unsafe_target=unsafe_block(config,&block,resolve,kind);
            // Duplicate groups are not a valid managed structure and must not trigger
            // the broad invalid-config fallback that quarantines unrelated vhosts.
            if names.insert(key.clone(),unsafe_target).is_some(){names.insert(key,true);}
        }
    }
    names
}

// Restart has no database; inspect only the certificate files the rendered server actually uses.
fn unavailable_certificate(text: &str) -> Option<String> {
    let parsed = statements(text);
    let names = parsed.iter().filter(|s| s.words.first().is_some_and(|w| w == "server_name"))
        .flat_map(|s| s.words.iter().skip(1).cloned()).collect::<Vec<_>>().join(" ");
    for statement in &parsed {
        if !statement.words.first().is_some_and(|w| matches!(w.as_str(), "ssl_certificate" | "ssl_certificate_key")) { continue; }
        let Some(path) = statement.words.get(1) else { continue; };
        if !std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0) {
            return Some(format!("서버 {names}: 인증서 파일 {path} 을(를) 읽을 수 없거나 비어 있습니다."));
        }
    }
    None
}

// Check each consumer's ports, since a shared group can loop for one vhost but not another.
fn listener_loop(config: &RuntimeConfig, text: &str, groups: &[UpstreamBlock], resolve: Resolve, stream: bool) -> bool {
    if !config.in_container { return false; }
    let own: Vec<u16> = statements(text).iter().filter(|s| s.words.first().is_some_and(|w| w == "listen"))
        .filter_map(|s| s.words.get(1)).filter_map(|s| s.rsplit(':').next()?.parse().ok()).collect();
    proxy_targets(text).iter().any(|target| {
        let name = if stream { Some(target.as_str()) } else { super::internal_targets::target_authority(target) };
        let group = groups.iter().find(|b| name.is_some_and(|name| b.name.eq_ignore_ascii_case(name)));
        if let Some(group) = group {
            group.servers.iter().enumerate().filter(|(i, _)| !group.placeholders.contains(i)).any(|(_, member)| {
                let empty = |_: &str, _: u16| Vec::new();
                let resolver: Resolve = if member.split_whitespace().any(|w| w == "resolve") { &empty } else { resolve };
                super::internal_targets::check_listener_host_port(member, &own, resolver).is_err()
            })
        } else if stream {
            super::internal_targets::check_listener_host_port(target, &own, resolve).is_err()
        } else {
            url::Url::parse(target).ok().is_some_and(|url| match (url.host_str(), url.port_or_known_default()) {
                (Some(host), Some(port)) => super::internal_targets::check_listener_endpoint(host, port, &own, resolve).is_err(),
                _ => false,
            })
        }
    })
}

/// Decide which managed files must not reach nginx. HTTP and stream namespaces
/// are checked independently, including groups in each owning server/stream file.
pub fn scan(config:&RuntimeConfig,conf:&BTreeMap<String,Vec<u8>>,stream:&BTreeMap<String,Vec<u8>>,resolve:Resolve)->TargetScan {
    let mut scan=TargetScan::default();
    let names=group_safety(config,conf,resolve,false);
    let groups: Vec<_> = conf.values().flat_map(|bytes| upstream_blocks(&String::from_utf8_lossy(bytes))).collect();
    if let Some(bytes)=conf.get(UPSTREAMS_FILE) {
        let text=String::from_utf8_lossy(bytes);let blocks=upstream_blocks(&text);let mut kept=text.to_string();
        for block in blocks.iter().rev() {
            if names.get(&block.name.to_ascii_lowercase()).copied().unwrap_or(true) {
                kept.replace_range(block.range.clone(),"");scan.removed_upstreams.push(block.name.clone());
            }
        }
        if !scan.removed_upstreams.is_empty(){scan.removed_upstreams.reverse();scan.conf.insert(UPSTREAMS_FILE.into());scan.upstreams=Some(kept.into_bytes());}
    }
    for (name,bytes) in conf.iter().filter(|(name,_)|name.as_str()!=UPSTREAMS_FILE) {
        let text=String::from_utf8_lossy(bytes);
        let certificate_error = unavailable_certificate(&text);
        if let Some(error) = &certificate_error { eprintln!("[nginx-guard] {name}: {error}"); }
        let refused=certificate_error.is_some()||listener_loop(config,&text,&groups,resolve,false)||upstream_blocks(&text).iter().any(|b|names.get(&b.name.to_ascii_lowercase())==Some(&true))
            ||proxy_targets(&text).iter().any(|target| match super::internal_targets::target_authority(target).and_then(|host|names.get(&host.to_ascii_lowercase())) {
                Some(bad) => *bad,
                None => check_url_with(config,target,resolve).is_err(),
            });
        if refused{scan.conf.insert(name.clone());}
    }
    let stream_names=group_safety(config,stream,resolve,true);
    let groups: Vec<_> = stream.values().flat_map(|bytes| upstream_blocks(&String::from_utf8_lossy(bytes))).collect();
    for (name,bytes) in stream {
        let text=String::from_utf8_lossy(bytes);
        let refused=listener_loop(config,&text,&groups,resolve,true)||upstream_blocks(&text).iter().any(|b|stream_names.get(&b.name.to_ascii_lowercase())==Some(&true))
            ||proxy_targets(&text).iter().any(|target|check_host_port_with(config,target,resolve).is_err()
                ||stream_names.get(&target.to_ascii_lowercase()).is_some_and(|bad|*bad));
        if refused{scan.stream.insert(name.clone());}
    }
    scan
}

/// A server-level cycle is only a warning: its routes need not loop the same request.
pub(crate) fn cycle_notices(config:&RuntimeConfig,conf:&BTreeMap<String,Vec<u8>>,resolve:Resolve)->Vec<String> {
    if !config.in_container {return Vec::new();}
    let groups:Vec<_>=conf.values().flat_map(|bytes|upstream_blocks(&String::from_utf8_lossy(bytes))).collect();
    let listeners:BTreeMap<_,_>=conf.iter().filter(|(name,_)|name.as_str()!=UPSTREAMS_FILE)
        .map(|(name,bytes)|(name.clone(),listen_ports(&String::from_utf8_lossy(bytes),true))).collect();
    let mut graph:BTreeMap<String,BTreeSet<String>>=BTreeMap::new();
    for (name,bytes) in conf.iter().filter(|(name,_)|listeners.contains_key(*name)) {
        let mut endpoints=Vec::new();
        for target in proxy_targets(&String::from_utf8_lossy(bytes)) {
            if let Some(group)=groups.iter().find(|group|super::internal_targets::target_authority(&target).is_some_and(|host|group.name.eq_ignore_ascii_case(host))) {
                for (index,member) in group.servers.iter().enumerate() {
                    if group.placeholders.contains(&index)||member.split_whitespace().any(|w|matches!(w,"resolve"|"down")){continue;}
                    if super::internal_socket::is_unix_target(member){continue;}
                    if let Some((host,port))=member.split_whitespace().next().and_then(|address|address.rsplit_once(':')).and_then(|(host,port)|port.parse::<u16>().ok().map(|port|(host,port))) {
                        endpoints.push((host.to_owned(),port));
                    }
                }
            }else if let Ok(url)=url::Url::parse(&target) {
                if let (Some(host),Some(port))=(url.host_str(),url.port_or_known_default()) {endpoints.push((host.to_owned(),port));}
            }
        }
        let edges=graph.entry(name.clone()).or_default();
        for (other,ports) in &listeners {
            if other!=name&&endpoints.iter().any(|(host,port)|super::internal_targets::loops_to_own_listener(host,*port,ports,resolve)) {edges.insert(other.clone());}
        }
    }
    let mut cyclic=Vec::new();
    for start in graph.keys() {
        let mut pending:Vec<_>=graph[start].iter().cloned().collect();
        let mut visited=BTreeSet::new();
        while let Some(next)=pending.pop() {
            if next==*start {cyclic.push(start.clone());break;}
            if visited.insert(next.clone()) {pending.extend(graph[&next].iter().cloned());}
        }
    }
    if cyclic.is_empty(){Vec::new()}else{vec![format!("서버 간 프록시 순환 가능성 ({}): 같은 요청이 순환하면 누구나 nginx 연결을 소진시킬 수 있습니다",cyclic.join(", "))]}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn container() -> (RuntimeConfig, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let mut config = RuntimeConfig::read(|key| match key {
            "DATABASE_URL" => Some("postgres://u@127.0.0.1:5432/db".into()),
            "AUTH_SECRET" => Some("s".into()),
            "NWUI_CONTAINER" => Some("1".into()),
            _ => None,
        })
        .unwrap();
        config.target_dns.descriptor_path = root.path().join("target-dns.json");
        (config, root)
    }
    fn files(entries: &[(&str, &str)]) -> BTreeMap<String, Vec<u8>> {
        entries.iter().map(|(n, c)| (n.to_string(), c.as_bytes().to_vec())).collect()
    }
    fn answers(map: &'static [(&'static str, &'static str)]) -> impl Fn(&str, u16) -> Vec<IpAddr> {
        move |host, _| {
            map.iter()
                .filter(|(name, _)| *name == host)
                .map(|(_, ip)| ip.parse().unwrap())
                .collect()
        }
    }

    #[test]
    fn statements_skip_comments_and_keep_quoted_words_whole() {
        let text = "# proxy_pass http://127.0.0.1:3001;\nadd_header X \"a; proxy_pass http://127.0.0.1:3001\";\n    proxy_pass http://ok:8080/x; # trailing\n    proxy_pass http://a#b;\n";
        assert_eq!(proxy_targets(text), vec!["http://ok:8080/x", "http://a#b"]);
        let quoted = "proxy_pass \"http://127.0.0.1:3001\";";
        assert_eq!(proxy_targets(quoted), vec!["http://127.0.0.1:3001"]);
        let variable = "location / {\n    proxy_pass https://${u}.example.test;\n}\n";
        assert_eq!(proxy_targets(variable), vec!["https://${u}.example.test"]);
    }

    #[test]
    fn statements_tokenize_paths_like_native_nginx() {
        for (input, expected) in [
            (r"include /tmp/line\nfile;", "/tmp/line\nfile"),
            (r"include '/tmp/line\rfile';", "/tmp/line\rfile"),
            (r#"include "/tmp/line\tfile";"#, "/tmp/line\tfile"),
            (r"include /tmp/unknown\z;", r"/tmp/unknown\z"),
            (r"include /tmp/escaped\ space;", r"/tmp/escaped\ space"),
            (r#"include /tmp/mid"quote;"#, "/tmp/mid\"quote"),
            ("include /tmp/non\u{a0}breaking;", "/tmp/non\u{a0}breaking"),
            (r#"include '/tmp/quote\"and\\slash';"#, "/tmp/quote\"and\\slash"),
        ] {
            let parsed = statements(input);
            assert_eq!(parsed.len(), 1, "{input:?}");
            assert_eq!(parsed[0].words, ["include", expected], "{input:?}");
        }
        let module=statements(r"load_module '/tmp/mod\nule.so';");
        assert_eq!(module[0].words,["load_module","/tmp/mod\nule.so"]);
    }

    #[test]
    fn shared_upstream_markers_report_valid_and_invalid_placeholders() {
        for (marked,invalid) in [("missing.test:80",false),("missing.test:80 weight=2",true)] {
            let text=format!("upstream pool {{\n    # runtime DNS unavailable: {marked}\n    server 127.0.0.1:1 down;\n}}\n");
            let blocks=upstream_blocks(&text);
            assert_eq!(blocks.len(),1);
            assert_eq!(blocks[0].name,"pool");
            assert_eq!(blocks[0].invalid_placeholder,invalid);
            assert_eq!(blocks[0].placeholders,if invalid {BTreeSet::new()} else {BTreeSet::from([0])});
        }
    }

    #[test]
    fn statement_range_keeps_utf8_quotes_and_intervening_comments() {
        let text=" # before\n  set # inside\n $x \"한;글\"; # between\n http {}";
        let parsed=statements(text);
        assert_eq!(&text[parsed[0].range()],"set # inside\n $x \"한;글\";");
        assert_eq!(parsed[0].range().end,parsed[0].end_offset());
        assert_eq!(&text[parsed[1].range()],"http {");
        assert_eq!(&text[parsed[2].range()],"}");
    }

    #[test]
    fn statement_end_offsets_are_original_utf8_byte_boundaries() {
        let text="set $x \"한;글\"; http {}";
        let parsed=statements(text);
        assert_eq!(&text[..parsed[0].end_offset()],"set $x \"한;글\";");
        assert_eq!(parsed.last().unwrap().end_offset(),text.len());
    }

    #[test]
    fn upstream_blocks_are_removed_by_range_and_comments_do_not_open_blocks() {
        let text = "# upstream fake {\nupstream a {\n    server 10.0.0.1:3001;\n}\n\nupstream b {\n    server 127.0.0.1:3001 weight=2;\n}\n\nupstream c {\n    server 10.0.0.5:8080;\n}\n";
        let conf = files(&[(UPSTREAMS_FILE, text)]);
        let scan = scan(&container().0, &conf, &BTreeMap::new(), &answers(&[("localhost", "127.0.0.1")]));
        assert_eq!(scan.removed_upstreams, vec!["b"]);
        assert_eq!(
            String::from_utf8(scan.upstreams.unwrap()).unwrap(),
            "# upstream fake {\nupstream a {\n    server 10.0.0.1:3001;\n}\n\n\nupstream c {\n    server 10.0.0.5:8080;\n}\n"
        );
    }

    /// The renderer's own output: named upstream routes and literal targets, with the management
    /// UI (8080) and an external host on an internal port number left alone.
    #[test]
    fn rendered_files_lose_only_what_reaches_an_internal_port() {
        let conf = files(&[
            (UPSTREAMS_FILE, "upstream api_up {\n    server [::1]:3001;\n}\n\nupstream Web_Ok {\n    server 10.1.2.3:3000;\n}\n"),
            ("server-a.conf", "server {\n    listen 80;\n    location / {\n        proxy_pass http://api_up;\n    }\n}\n"),
            ("server-b.conf", "server {\n    location / {\n        proxy_pass http://web_ok/x;\n    }\n}\n"),
            ("server-c.conf", "server {\n    location / {\n        proxy_pass http://127.1:3000/;\n    }\n}\n"),
            ("server-d.conf", "server {\n    location / {\n        proxy_pass http://127.0.0.1:8080;\n    }\n}\n"),
            ("server-e.conf", "server {\n    location / {\n        proxy_pass http://[::ffff:127.0.0.1]:5432;\n    }\n}\n"),
            ("server-f.conf", "server {\n    location /a {\n        proxy_pass http://example.test;\n    }\n    location /b {\n        proxy_pass https://rebound.test:3001;\n    }\n}\n"),
        ]);
        let stream = files(&[
            ("stream-a.conf", "server {\n    listen 9000;\n    proxy_pass localhost:3001;\n}\n"),
            ("stream-b.conf", "server {\n    listen 9001 udp;\n    proxy_pass 0.0.0.0:5432;\n}\n"),
            ("stream-c.conf", "server {\n    listen 9002;\n    proxy_pass db.example.test:5432;\n}\n"),
        ]);
        let resolve = answers(&[
            ("localhost", "127.0.0.1"),
            ("rebound.test", "127.0.0.1"),
            ("db.example.test", "192.0.2.10"),
        ]);
        let scan = scan(&container().0, &conf, &stream, &resolve);
        assert_eq!(scan.removed_upstreams, vec!["api_up"]);
        assert_eq!(
            scan.conf.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["server-a.conf", "server-c.conf", "server-e.conf", "server-f.conf", UPSTREAMS_FILE]
        );
        assert_eq!(
            scan.stream.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["stream-a.conf", "stream-b.conf"]
        );
    }

    /// A name that answered an outside address when it was applied and 127.0.0.1 now.
    #[test]
    fn a_changed_dns_answer_is_seen_on_the_next_scan() {
        let conf = files(&[
            (UPSTREAMS_FILE, "upstream pool {\n    server app.example.test:3001;\n}\n"),
            ("server-pool.conf", "location / {\n    proxy_pass http://pool;\n}\n"),
            ("server-direct.conf", "location / {\n    proxy_pass http://app.example.test:3000;\n}\n"),
            ("server-other.conf", "location / {\n    proxy_pass http://other.example.test:3000;\n}\n"),
        ]);
        let stream = files(&[("stream-x.conf", "proxy_pass app.example.test:5432;\n")]);
        let before = answers(&[("app.example.test", "203.0.113.7"), ("other.example.test", "203.0.113.8")]);
        assert_eq!(scan(&container().0, &conf, &stream, &before), TargetScan::default());
        let after = answers(&[("app.example.test", "127.0.0.1"), ("other.example.test", "203.0.113.8")]);
        let changed = scan(&container().0, &conf, &stream, &after);
        assert_eq!(
            changed.conf.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["server-direct.conf", "server-pool.conf", UPSTREAMS_FILE]
        );
        assert_eq!(changed.stream.len(), 1);
        let v6 = answers(&[("app.example.test", "::1"), ("other.example.test", "203.0.113.8")]);
        assert_eq!(scan(&container().0, &conf, &stream, &v6).conf.len(), 3);
    }

    /// The image's own upstream names reach its API and web app whatever the port, and so can a
    /// variable host or port. Generated files that use the names are not managed and never
    /// scanned; users' own upstreams and variables after a fixed authority are fine.
    #[test]
    fn runtime_upstream_names_and_variable_authorities_are_refused() {
        let conf = files(&[
            (UPSTREAMS_FILE, "upstream nwui_apix {\n    server 10.0.0.1:80;\n}\n"),
            ("server-a.conf", "location / {\n    proxy_pass http://nwui_api;\n}\n"),
            ("server-b.conf", "location /x {\n    proxy_pass http://NWUI_Web/x;\n}\n"),
            ("server-c.conf", "location / {\n    proxy_pass https://nwui_api:8443;\n}\n"),
            ("server-d.conf", "include /etc/nginx/snippets/acme-challenge.conf;\nlocation / {\n    proxy_pass http://nwui_apix;\n}\n"),
            // A variable host or port can become either name, or a loopback port, per request.
            ("server-e.conf", "location / {\n    proxy_pass http://$arg_u;\n}\n"),
            ("server-f.conf", "location / {\n    proxy_pass http://127.0.0.1:$arg_p/x;\n}\n"),
            ("server-g.conf", "location / {\n    proxy_pass https://${u}.example.test?x=1;\n}\n"),
            // Variables after a fixed authority keep the destination fixed.
            ("server-h.conf", "location ~ ^/a/(.*)$ {\n    proxy_pass http://10.0.0.5:8080/$1?q=$arg_q;\n}\n"),
            // Strict by design: a variable right after the host counts as part of the authority.
            ("server-i.conf", "location / {\n    proxy_pass http://nwui_apix$request_uri;\n}\n"),
        ]);
        let refused = scan(&container().0, &conf, &BTreeMap::new(), &answers(&[]));
        assert_eq!(
            refused.conf.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["server-a.conf", "server-b.conf", "server-c.conf", "server-e.conf", "server-f.conf", "server-g.conf", "server-i.conf"]
        );
        let (mut outside, _root) = container();
        outside.in_container = false;
        assert!(scan(&outside, &conf, &BTreeMap::new(), &answers(&[])).is_empty());
    }

    #[test]
    fn outside_the_container_nothing_is_refused() {
        let (mut config, _root) = container();
        config.in_container = false;
        let conf = files(&[("server-a.conf", "proxy_pass http://127.0.0.1:3001;")]);
        assert!(scan(&config, &conf, &BTreeMap::new(), &answers(&[])).is_empty());
    }

    #[test]
    fn multihop_named_groups_aliases_and_fan_in() {
        let (config,_root)=container();
        let mut conf=files(&[
            (UPSTREAMS_FILE,"upstream pool { server alias.test:18091 weight=2; }"),
            ("server-a.conf","server { listen 18090; location /api { proxy_pass http://POOL; } }"),
            ("server-b.conf","server { listen 18091 ssl; location /auth { proxy_pass http://[::ffff:127.0.0.1]:18090; } }"),
            ("server-fan-in.conf","server { listen 18092; location / { proxy_pass http://localhost:18090; } }"),
        ]);
        let resolve=answers(&[("alias.test","127.0.0.2")]);
        let notices=cycle_notices(&config,&conf,&resolve);
        assert_eq!(notices.len(),1);
        assert!(notices[0].contains("server-a.conf")&&notices[0].contains("server-b.conf"));
        assert!(!notices[0].contains("server-fan-in.conf"));
        conf.insert("server-b.conf".into(),b"server { listen 18091; location / { proxy_pass http://192.0.2.1:18090; } }".to_vec());
        assert!(cycle_notices(&config,&conf,&resolve).is_empty());
    }

    #[test]
    fn multihop_runtime_names_and_down_peers_never_enter_nss() {
        let (config,_root)=container();
        let conf=files(&[
            (UPSTREAMS_FILE,"upstream pool { server runtime.test:18091 resolve; server localhost:18091 down; }"),
            ("server-a.conf","server { listen 18090; location / { proxy_pass http://pool; } }"),
            ("server-b.conf","server { listen 18091; location / { proxy_pass http://127.0.0.1:18090; } }"),
        ]);
        assert!(cycle_notices(&config,&conf,&|_,_|panic!("runtime names must not use NSS")).is_empty());
    }

    #[test]
    fn multihop_three_servers_and_default_port_are_report_only() {
        let (config,_root)=container();
        let conf=files(&[
            ("server-a.conf","server { listen 80; location / { proxy_pass http://localhost:18091; } }"),
            ("server-b.conf","server { listen 18091; location / { proxy_pass http://127.0.0.2:18092; } }"),
            ("server-c.conf","server { listen 18092; location / { proxy_pass http://127.0.0.1; } }"),
        ]);
        assert!(scan(&config,&conf,&BTreeMap::new(),&answers(&[])).is_empty());
        let notices=cycle_notices(&config,&conf,&answers(&[]));
        assert_eq!(notices.len(),1);
        for name in conf.keys(){assert!(notices[0].contains(name));}
        let mut outside=config;outside.in_container=false;
        assert!(cycle_notices(&outside,&conf,&|_,_|panic!("outside image")).is_empty());
    }
}

#[cfg(test)]
mod dns_restart_tests {
    use super::*;
    use crate::dns_guard::Descriptor;
    fn configured()->(RuntimeConfig,tempfile::TempDir) {
        let root=tempfile::tempdir().unwrap();
        let mut c=RuntimeConfig::read(|key|match key {
            "DATABASE_URL"=>Some("postgres://test:test@127.0.0.1/test".into()),
            "AUTH_SECRET"=>Some("test-only".into()),"NWUI_CONTAINER"=>Some("1".into()),_=>None,
        }).unwrap();
        c.target_dns.descriptor_path=root.path().join("dns.json");
        std::fs::write(&c.target_dns.descriptor_path,serde_json::to_vec(&Descriptor::available(&c.target_dns)).unwrap()).unwrap();
        (c,root)
    }
    fn dynamic(name:&str)->String {
        format!("upstream {name} {{\n zone {name} 64k;\n resolver 127.0.0.1:5354 valid=10s;\n resolver_timeout 2s;\n server blocked.example:3001 resolve;\n}}\n")
    }
    #[test]
    fn valid_native_groups_defer_dns_in_named_http_direct_and_stream_files() {
        let (c,_root)=configured();let group="nwui_resolve_0123456789abcdef0123456789abcdef_0";
        let conf=BTreeMap::from([
            ("upstreams.conf".into(),dynamic("nwui_resolve_upstream_fedcba9876543210fedcba9876543210").replace("upstream nwui_resolve_upstream_fedcba9876543210fedcba9876543210","upstream named_backend").into_bytes()),
            ("server-live.conf".into(),format!("server {{listen 80;location / {{proxy_pass http://named_backend;}}}}\n").into_bytes()),
            ("server-direct.conf".into(),format!("{}server {{listen 81;location / {{proxy_pass http://{group};}}}}",dynamic(group)).into_bytes()),
        ]);
        let stream_group="nwui_resolve_stream_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let stream=BTreeMap::from([("stream-live.conf".into(),format!("{}server {{listen 19000;proxy_pass {stream_group};}}",dynamic(stream_group)).into_bytes())]);
        let poison=|_:&str,_:u16|vec!["127.0.0.1".parse().unwrap()];
        assert!(scan(&c,&conf,&stream,&poison).is_empty(),"already-filtered groups must not be quarantined because a name is absent or blocked");
    }
    #[test]
    fn token_only_resolve_wrong_resolver_and_stale_descriptor_isolate_only_affected_files() {
        let (c,_root)=configured();let group="nwui_resolve_0123456789abcdef0123456789abcdef_0";
        let good="server { listen 82; location / { proxy_pass http://172.18.0.2:8080; } }".as_bytes().to_vec();
        for bad in [dynamic(group).replace("blocked.example:3001", "127.0.0.1:3001"), dynamic(group).replace("blocked.example:3001", "nwui_api:3001"), dynamic(group).replace("zone nwui_resolve_0123456789abcdef0123456789abcdef_0", "zone nwui_resolve_bad_0"), dynamic(group).replace("127.0.0.1:5354","8.8.8.8"),dynamic(group).replace(&format!("zone {group} 64k;"),""),dynamic(group).replace("resolver_timeout 2s;","resolver_timeout 10s;")] {
            let conf=BTreeMap::from([("server-bad.conf".into(),bad.into_bytes()),("server-good.conf".into(),good.clone())]);
            let scan=scan(&c,&conf,&BTreeMap::new(),&|_,_|vec![]);
            assert!(scan.conf.contains("server-bad.conf"));assert!(!scan.conf.contains("server-good.conf"));
        }
        std::fs::write(&c.target_dns.descriptor_path,serde_json::to_vec(&Descriptor::unavailable(&c.target_dns,"saved UDP conflict")).unwrap()).unwrap();
        let conf=BTreeMap::from([("server-bad.conf".into(),dynamic(group).into_bytes()),("server-good.conf".into(),good)]);
        let scan=scan(&c,&conf,&BTreeMap::new(),&|_,_|vec![]);
        assert!(scan.conf.contains("server-bad.conf"));assert!(!scan.conf.contains("server-good.conf"));
    }
}

#[cfg(test)]
mod placeholder_tests {
    use super::*;
    fn config()->(RuntimeConfig,tempfile::TempDir) {
        let root=tempfile::tempdir().unwrap();
        let mut c=RuntimeConfig::read(|key|match key {"DATABASE_URL"=>Some("postgres://test:test@127.0.0.1/test".into()),"AUTH_SECRET"=>Some("test-only".into()),"NWUI_CONTAINER"=>Some("1".into()),_=>None}).unwrap();
        c.target_dns.descriptor_path=root.path().join("dns.json");
        std::fs::write(&c.target_dns.descriptor_path,serde_json::to_vec(&crate::dns_guard::Descriptor::unavailable(&c.target_dns,"saved UDP collision")).unwrap()).unwrap();
        (c,root)
    }
    fn group(member:&str)->String {
        format!("upstream legacy_backend {{\n zone nwui_resolve_upstream_0123456789abcdef0123456789abcdef 64k;\n {member}\n server 172.18.0.2:3001;\n}}\n")
    }
    #[test]
    fn unavailable_placeholder_requires_exact_adjacent_comment_and_original_weight() {
        let (c,_root)=config();
        let scan_group=|text:String| scan(&c,&BTreeMap::from([("upstreams.conf".into(),text.into_bytes())]),&BTreeMap::new(),&|_,_|vec![]);
        for good in ["# runtime DNS unavailable: peer.test:3001\n server 127.0.0.1:1 down;","# runtime DNS unavailable: peer.test:3001 weight=2\n server 127.0.0.1:1 weight=2 down;"] {
            assert!(scan_group(group(good)).is_empty(),"valid placeholder and safe literal peer must survive");
        }
        for bad in [
            "server 127.0.0.1:1 down;",
            "# runtime DNS unavailable: peer.test:3001\n\n server 127.0.0.1:1 down;",
            "# runtime DNS unavailable: peer.test:3001\n # unrelated\n server 127.0.0.1:1 down;",
            "# runtime DNS unavailable: peer.test:3001\n server 127.0.0.1:1;",
            "# runtime DNS unavailable: peer.test:3001\n server 127.0.0.1:2 down;",
            "# runtime DNS unavailable: 127.0.0.1:3001\n server 127.0.0.1:1 down;",
            "# runtime DNS unavailable: peer.test:3001 weight=2\n server 127.0.0.1:1 weight=3 down;",
            "# runtime DNS unavailable: peer.test:3001\n server 127.0.0.1:1 backup down;",
        ] {
            assert_eq!(scan_group(group(bad)).removed_upstreams,vec!["legacy_backend"],"reject forged placeholder: {bad}");
        }
    }
    #[test]
    fn placeholder_requires_generated_zone_and_is_never_a_direct_http_fallback() {
        let (c,_root)=config();
        let good=group("# runtime DNS unavailable: peer.test:3001\n server 127.0.0.1:1 down;");
        for forged in [good.replace(" zone nwui_resolve_upstream_0123456789abcdef0123456789abcdef 64k;\n", ""),good.replace("nwui_resolve_upstream_0123456789abcdef0123456789abcdef", "arbitrary_zone"),good.replace("64k;", "32k;")] {
            let conf=BTreeMap::from([("upstreams.conf".into(),forged.into_bytes())]);
            assert_eq!(scan(&c,&conf,&BTreeMap::new(),&|_,_|vec![]).removed_upstreams,vec!["legacy_backend"]);
        }
        let direct=good.replace("legacy_backend","nwui_resolve_0123456789abcdef0123456789abcdef_0").replace("nwui_resolve_upstream_0123456789abcdef0123456789abcdef","nwui_resolve_0123456789abcdef0123456789abcdef_0");
        let conf=BTreeMap::from([("server-direct.conf".into(),direct.into_bytes())]);
        assert!(scan(&c,&conf,&BTreeMap::new(),&|_,_|vec![]).conf.contains("server-direct.conf"));
    }
    #[test]
    fn exact_down_placeholder_is_safe_even_when_port_one_is_an_internal_service() {
        let (mut c,_root)=config();c.listen.set_port(1);
        let conf=BTreeMap::from([("upstreams.conf".into(),group("# runtime DNS unavailable: peer.test:3001\n server 127.0.0.1:1 down;").into_bytes())]);
        assert!(scan(&c,&conf,&BTreeMap::new(),&|_,_|vec![]).is_empty(),"exact down placeholder must never be mistaken for a live loopback target");
    }
    #[test]
    fn placeholder_rejection_preserves_unrelated_static_vhost_and_tcp_udp_namespaces() {
        let (c,_root)=config();
        let valid="# runtime DNS unavailable: peer.test:3001\n server 127.0.0.1:1 down;";
        let stream=group(valid).replace("legacy_backend","nwui_resolve_stream_0123456789abcdef0123456789abcdef").replace("nwui_resolve_upstream_","nwui_resolve_stream_");
        let conf=BTreeMap::from([("server-static.conf".into(),b"server { listen 8081; location / { proxy_pass http://172.18.0.2:3001; } }".to_vec())]);
        for listen in ["19001","19002 udp"] {
            let managed=format!("{stream}server {{ listen {listen}; proxy_pass nwui_resolve_stream_0123456789abcdef0123456789abcdef; }}");
            let good=BTreeMap::from([("stream-live.conf".into(),managed.as_bytes().to_vec())]);
            assert!(scan(&c,&conf,&good,&|_,_|vec![]).is_empty());
            let bad=BTreeMap::from([("stream-live.conf".into(),managed.replace("# runtime DNS unavailable:","# unrelated:").into_bytes())]);
            let result=scan(&c,&conf,&bad,&|_,_|vec![]);
            assert!(result.conf.is_empty());assert_eq!(result.stream.into_iter().collect::<Vec<_>>(),vec!["stream-live.conf"]);
        }
    }
}

// The existing tokenizer handles quoted paths and ignores commented references.
pub(crate) fn references_certificate(text: &str, path: &str) -> bool {
    statements(text).iter().any(|s| s.words.first().is_some_and(|name| matches!(name.as_str(), "ssl_certificate" | "ssl_certificate_key")) && s.words.get(1).is_some_and(|value| value == path))
}

#[cfg(test)]
mod fxb_security_tests {
    use super::*;
    #[test]
    fn fxb_restart_scan_quarantines_direct_and_named_self_edges() {
        let cfg=RuntimeConfig::read(|key|match key {"DATABASE_URL"=>Some("postgres://test@127.0.0.1/db".into()),"AUTH_SECRET"=>Some("test-only".into()),"NWUI_CONTAINER"=>Some("1".into()),_=>None}).unwrap();
        let conf=BTreeMap::from([("upstreams.conf".into(),b"upstream backend { server 127.0.0.1:18086; }".to_vec()),("server-direct.conf".into(),b"server { listen 18086; location / { proxy_pass http://127.0.0.1:18086; } }".to_vec()),("server-named.conf".into(),b"server { listen 18086; location / { proxy_pass http://backend; } }".to_vec()),("server-good.conf".into(),b"server { listen 18087; location / { proxy_pass http://backend; } }".to_vec())]);
        let stream=BTreeMap::from([("stream-loop.conf".into(),b"server { listen 19000; proxy_pass 127.0.0.1:19000; }".to_vec())]);
        let result=scan(&cfg,&conf,&stream,&|_,_|Vec::new());
        assert!(result.conf.contains("server-direct.conf"));assert!(result.conf.contains("server-named.conf"));assert!(!result.conf.contains("server-good.conf"));assert!(result.stream.contains("stream-loop.conf"));
    }
}

#[cfg(test)]
mod q23_tests {
    use super::*;
    #[test]
    fn q23_static_unix_groups_routes_and_streams_never_reach_dns() {
        let config=RuntimeConfig::read(|key|match key {"DATABASE_URL"=>Some("postgres://fixture@127.0.0.1/db".into()),"AUTH_SECRET"=>Some("fixture-only".into()),"NWUI_CONTAINER"=>Some("1".into()),_=>None}).unwrap();
        let conf=BTreeMap::from([("upstreams.conf".into(),b"upstream sockets { server unix:3001; }".to_vec()),("server-socket.conf".into(),b"server { listen 80; location / { proxy_pass http://unix:/tmp/x.sock; } location /api { proxy_pass http://sockets; } }".to_vec())]);
        let stream=BTreeMap::from([("stream-socket.conf".into(),b"server { listen 8080; proxy_pass unix:8080; }".to_vec())]);
        let no_lookup=|_:&str,_:u16|panic!("Unix scan reached DNS");let result=scan(&config,&conf,&stream,&no_lookup);assert!(result.is_empty(),"{result:?}");assert!(cycle_notices(&config,&conf,&no_lookup).is_empty());
    }
}
