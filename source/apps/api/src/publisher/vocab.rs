//! Pure admission of statements emitted by the managed nginx renderer.
use crate::{config::RuntimeConfig, control::{guard_targets::{self as lexer, End, Statement}, performance, validation::{self, DirectiveContext}}};
use anyhow::{Result, bail, ensure};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Context { Http, Stream, HttpServer, StreamServer, Location, Upstream, Map }
#[derive(Debug)]
enum Node { Line(Vec<String>,Option<Vec<String>>), Block(Context, Vec<String>, Vec<Node>) }
struct File { name:String, blocks:BTreeMap<String,lexer::UpstreamBlock>, stream:bool, nodes:Vec<Node> }

fn child_context(parent:Context, words:&[String])->Result<Context> {
    match (parent, words.first().map(String::as_str)) {
        (Context::Http|Context::Stream,Some("upstream")) if words.len()==2=>Ok(Context::Upstream),
        (Context::Http,Some("server")) if words.len()==1=>Ok(Context::HttpServer),
        (Context::Stream,Some("server")) if words.len()==1=>Ok(Context::StreamServer),
        (Context::Http,Some("map")) if words.len()==3=>Ok(Context::Map),
        (Context::HttpServer,Some("location")) if words.len()==2=>Ok(Context::Location),
        _=>bail!("nginx vocabulary: misplaced block in {parent:?}"),
    }
}
fn parse(text:&str,tokens:&[Statement],cursor:&mut usize,context:Context,nested:bool)->Result<Vec<Node>> {
    let mut nodes=Vec::new();
    while let Some(statement)=tokens.get(*cursor) {
        *cursor+=1;
        match statement.end {
            End::Semicolon=>{
                ensure!(!statement.words.is_empty(),"nginx vocabulary: empty directive");
                nodes.push(Node::Line(statement.words.clone(),statement_spellings(text,statement)?));
            }
            End::Open=>{
                let child=child_context(context,&statement.words)?;
                nodes.push(Node::Block(child,statement.words.clone(),parse(text,tokens,cursor,child,true)?));
            }
            End::Close=>{
                ensure!(nested&&statement.words.is_empty(),"nginx vocabulary: unexpected closing block");
                return Ok(nodes);
            }
        }
    }
    ensure!(!nested,"nginx vocabulary: unterminated block");
    Ok(nodes)
}
fn lines(nodes:&[Node])->impl Iterator<Item=&[String]> {
    nodes.iter().filter_map(|node|match node {Node::Line(words,_)=>Some(words.as_slice()),_=>None})
}
fn hex32(value:&str)->bool {value.len()==32&&value.bytes().all(|c|c.is_ascii_digit()||(b'a'..=b'f').contains(&c))}
fn uuid(value:&str)->bool {uuid::Uuid::parse_str(value).is_ok_and(|id|id.to_string()==value)}
fn resource_file(name:&str,prefix:&str)->bool {
    name.strip_prefix(prefix).and_then(|id|id.strip_suffix(".conf")).is_some_and(|id|!id.is_empty()&&id.bytes().all(|c|c.is_ascii_alphanumeric()||b"_-".contains(&c)))
}
fn generated_group(name:&str,stream:bool)->bool {
    if stream {return name.strip_prefix("nwui_resolve_stream_").is_some_and(hex32);}
    name.strip_prefix("nwui_resolve_").and_then(|tail|tail.rsplit_once('_')).is_some_and(|(id,index)|hex32(id)&&index.parse::<usize>().is_ok_and(|i|i.to_string()==index))
}
fn zone_name(value:&str)->bool {value.strip_prefix("srv_").is_some_and(hex32)}
fn bypass(value:&str,zones:&BTreeSet<String>)->bool {value.strip_prefix('$').and_then(|v|v.strip_suffix("_bypass")).is_some_and(|zone|zones.contains(zone))}
fn certificate(config:&RuntimeConfig,value:&str,leaf:&str)->bool {
    std::path::Path::new(value).parent().and_then(|parent|parent.file_name()).and_then(|id|id.to_str()).is_some_and(|id|uuid(id)&&value==config.certs_dir.join(id).join(leaf).to_string_lossy())
}
// NUL is forbidden in payloads; tags preserve escape provenance while the shared lexer handles syntax.
fn statement_spellings(text:&str,statement:&Statement)->Result<Option<Vec<String>>> {
    if !statement.words.iter().any(|word|word.contains(['\n','\r','\t'])) {return Ok(None);}
    let mut raw=text[statement.range()].chars();let mut tagged=String::new();
    while let Some(c)=raw.next() {
        if c=='\\' {
            match raw.next() {
                Some(escaped @ ('n'|'r'|'t'))=>{tagged.push('\0');tagged.push(escaped);}
                Some(escaped)=>{tagged.push('\\');tagged.push(escaped);}
                None=>tagged.push('\\'),
            }
        }else{tagged.push(c);}
    }
    let tokens=lexer::statements(&tagged);
    ensure!(tokens.len()==1&&tokens[0].end==statement.end,"nginx vocabulary: ambiguous escape provenance");
    let words=&tokens[0].words;
    ensure!(words.len()==statement.words.len()&&words.iter().zip(&statement.words).all(|(tagged,decoded)|tagged.replace("\0n","\n").replace("\0r","\r").replace("\0t","\t")==*decoded),"nginx vocabulary: changed escape provenance");
    Ok(Some(words.iter().map(|word|word.replace("\0n",r"\n").replace("\0r",r"\r").replace("\0t",r"\t")).collect()))
}
fn allowed(name:&str,args:&[String],stored:&[String],context:DirectiveContext)->bool {
    if validation::assert_valid_directive(name,args,context).is_ok(){return true;}
    let mut stored=stored.to_vec();
    if validation::HEADER_DIRECTIVES.contains(&name)&&!args.is_empty(){stored[0]=args[0].clone();}
    validation::assert_valid_directive(name,&stored,context).is_ok()
}
fn stored_target(value:&str,stored:&str)->bool {
    validation::assert_valid_stored_proxy_target(value).is_ok()||validation::assert_valid_stored_proxy_target(stored).is_ok()
}
fn one(args:&[String],value:&str)->bool {args.len()==1&&args[0]==value}
fn port(value:&str)->bool {!value.is_empty()&&value.bytes().all(|c|c.is_ascii_digit())&&value.parse::<u16>().is_ok_and(|p|p>0)}
fn literal_authority(value:&str)->bool {
    !value.contains(['$','|','@','/','?','#','\\'])&&validation::assert_valid_stored_proxy_target(&format!("http://{value}")).is_ok()
}
fn canonical_admin(target:&str,groups:&BTreeMap<String,bool>)->bool {
    let Some(authority)=crate::control::internal_targets::target_authority(target) else{return false;};
    authority=="127.0.0.1:8080"||groups.get(&authority.to_ascii_lowercase())==Some(&true)
}
fn protected(nodes:&[Node],groups:&BTreeMap<String,bool>)->bool {
    let targets=lines(nodes).filter(|words|words.first().is_some_and(|w|w=="proxy_pass")).collect::<Vec<_>>();
    if targets.len()!=1||targets[0].len()!=2||!canonical_admin(&targets[0][1],groups){return false;}
    [("X-Real-IP","$remote_addr"),("X-Forwarded-Proto","$scheme")].into_iter().all(|(name,value)| {
        let headers=lines(nodes).filter(|words|words.first().is_some_and(|w|w=="proxy_set_header")&&words.get(1).is_some_and(|header|header.eq_ignore_ascii_case(name))).collect::<Vec<_>>();
        headers.len()==1&&headers[0].len()==3&&headers[0][2]==value
    })
}
fn assert_emitted_names(context:Context,nodes:&[Node])->Result<()> {
    for node in nodes {match node {
        Node::Line(words,_)=>ensure!(emitted_name(context,&words[0]),"nginx vocabulary: unknown statement in {context:?}"),
        Node::Block(child,_,body)=>assert_emitted_names(*child,body)?,
    }}
    Ok(())
}
fn validate_upstream(config:&RuntimeConfig,file:&File,name:&str,nodes:&[Node])->Result<()> {
    let runtime=file.name=="00-runtime-upstreams.conf";
    if runtime {
        let expected=match name {"nwui_api"=>config.listen.port(),"nwui_web"=>config.web_port,_=>bail!("nginx vocabulary: unknown runtime upstream")};
        let rows=lines(nodes).collect::<Vec<_>>();
        ensure!(rows.len()==2&&rows.iter().any(|w|*w==["keepalive","16"])&&rows.iter().any(|w|w.len()==2&&w[0]=="server"&&w[1]==format!("127.0.0.1:{expected}")),"nginx vocabulary: changed runtime upstream");
        return Ok(());
    }
    let named=!file.stream&&file.name=="upstreams.conf";
    if named {validation::assert_valid_upstream_name(name).map_err(|_|anyhow::anyhow!("nginx vocabulary: invalid upstream name"))?;}
    else {ensure!(generated_group(name,file.stream),"nginx vocabulary: invalid generated upstream");}
    let block=file.blocks.get(name).ok_or_else(||anyhow::anyhow!("nginx vocabulary: missing upstream block"))?;
    ensure!(!block.invalid_placeholder,"nginx vocabulary: invalid unavailable-DNS marker");
    let mut zone=false;let mut resolver=false;let mut timeout=false;let mut dynamic=false;let mut index=0;
    for words in lines(nodes) {
        let args=&words[1..];
        match words[0].as_str() {
            "server"=>{
                ensure!(!args.is_empty(),"nginx vocabulary: empty upstream member");
                let resolve=args.iter().any(|v|v=="resolve");let down=args.iter().any(|v|v=="down");
                ensure!(args.iter().filter(|v|v.as_str()=="resolve").count()<=1&&args.iter().filter(|v|v.as_str()=="down").count()<=1,"nginx vocabulary: duplicate member flags");
                let member=args.iter().filter(|v|!matches!(v.as_str(),"resolve"|"down")).cloned().collect::<Vec<_>>().join(" ");
                if !named&&!file.stream {
                    ensure!(args.len()==2&&resolve&&!down&&crate::control::render::OriginalProxyTarget::parse(&format!("http://{member}")).is_ok(),"nginx vocabulary: invalid direct DNS member");
                }else{
                    validation::assert_valid_upstream_server(&member).map_err(|_|anyhow::anyhow!("nginx vocabulary: invalid upstream member"))?;
                }
                ensure!(!down||(!resolve&&block.placeholders.contains(&index)),"nginx vocabulary: unowned down placeholder");
                if resolve {ensure!(crate::control::internal_targets::is_dns_endpoint(&member),"nginx vocabulary: invalid dynamic member");}
                dynamic|=resolve;index+=1;
            }
            "zone"=>{
                ensure!(!zone&&args.len()==2&&args[1]=="64k","nginx vocabulary: invalid zone");
                ensure!(if named {args[0].strip_prefix("nwui_resolve_upstream_").is_some_and(hex32)}else{args[0]==name},"nginx vocabulary: unpinned zone");zone=true;
            }
            "resolver"=>{ensure!(!resolver&&args==[config.target_dns.endpoint().to_string(),"valid=10s".into()],"nginx vocabulary: unpinned resolver");resolver=true;}
            "resolver_timeout"=>{ensure!(!timeout&&one(args,"2s"),"nginx vocabulary: unpinned resolver timeout");timeout=true;}
            _=>bail!("nginx vocabulary: unknown upstream directive"),
        }
    }
    ensure!((!resolver&&!timeout)||zone&&resolver&&timeout,"nginx vocabulary: incomplete resolver settings");
    ensure!(!dynamic||zone&&resolver&&timeout,"nginx vocabulary: incomplete dynamic group");
    ensure!(zone||named&&block.placeholders.is_empty(),"nginx vocabulary: missing generated zone");
    Ok(())
}
fn cache_path(args:&[String])->bool {
    if args.len()!=6 {return false;}
    let Some(id)=args[0].strip_prefix("/var/cache/nginx/proxy-").filter(|id|hex32(id)) else{return false;};
    args[1]=="levels=1:2"&&args[2]==format!("keys_zone=srv_{id}:10m")&&args[3].strip_prefix("max_size=").is_some_and(|v|performance::CACHE_MAX_SIZES.contains(&v))&&args[4].strip_prefix("inactive=").is_some_and(|v|performance::CACHE_INACTIVES.contains(&v))&&args[5]=="use_temp_path=off"
}
// Closed renderer vocabulary; user statements still go through the stored-apply allowlist.
fn emitted_name(context:Context,name:&str)->bool {
    let generated:&[&str]=match context {
        Context::Http=>&["proxy_cache_path"],
        Context::Stream=>&[],
        Context::HttpServer=>&["listen","server_name","include","ssl_certificate","ssl_certificate_key","ssl_protocols","ssl_ciphers","ssl_prefer_server_ciphers","real_ip_recursive","set_real_ip_from","real_ip_header","gzip_types","gzip_min_length","gzip_comp_level","gzip_vary","gzip_proxied","allow","deny"],
        Context::StreamServer=>&["listen","proxy_pass"],
        Context::Location=>&["allow","deny","proxy_pass","proxy_bind","proxy_ssl_name","gzip","proxy_cache","proxy_cache_key","proxy_cache_valid","proxy_cache_lock","proxy_cache_background_update","proxy_cache_bypass","proxy_no_cache","proxy_cache_use_stale","proxy_next_upstream","proxy_next_upstream_tries","proxy_next_upstream_timeout"],
        Context::Upstream=>&["server","keepalive","zone","resolver","resolver_timeout"],
        Context::Map=>&[":","default"],
    };
    generated.contains(&name)||match context {
        Context::HttpServer=>validation::SERVER_DIRECTIVES.contains(&name),
        Context::Location=>validation::LOCATION_DIRECTIVES.contains(&name),
        _=>false,
    }
}
fn validate_line(config:&RuntimeConfig,context:Context,words:&[String],stored:&[String],scope:&[Node],groups:&BTreeMap<String,bool>,zones:&BTreeSet<String>)->Result<()> {
    let name=words[0].as_str();let args=&words[1..];
    ensure!(emitted_name(context,name),"nginx vocabulary: unknown {name} in {context:?}");
    let accepted=match (context,name) {
        (Context::Http,"proxy_cache_path")=>cache_path(args),
        (Context::HttpServer|Context::StreamServer,"listen")=>!args.is_empty()&&port(&args[0])&&(args.len()==1||args.len()==2&&args[1]==if context==Context::HttpServer{"ssl"}else{"udp"}),
        (Context::HttpServer,"server_name")=>!args.is_empty()&&validation::assert_valid_server_name(&args.join(" ")).is_ok(),
        (Context::HttpServer,"include")=>args.len()==1&&crate::control::render::ACME_CHALLENGE_INCLUDE.strip_prefix("include ").and_then(|v|v.strip_suffix(';'))==Some(args[0].as_str()),
        (Context::HttpServer,"ssl_certificate")=>args.len()==1&&certificate(config,&args[0],"fullchain.pem"),
        (Context::HttpServer,"ssl_certificate_key")=>args.len()==1&&certificate(config,&args[0],"privkey.pem"),
        (Context::HttpServer,"ssl_protocols")=>args==["TLSv1.3"]||args==["TLSv1.2","TLSv1.3"],
        (Context::HttpServer,"ssl_ciphers")=>one(args,"ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-CHACHA20-POLY1305:ECDHE-RSA-CHACHA20-POLY1305:DHE-RSA-AES128-GCM-SHA256:DHE-RSA-AES256-GCM-SHA384:DHE-RSA-CHACHA20-POLY1305"),
        (Context::HttpServer,"ssl_prefer_server_ciphers"|"real_ip_recursive")=>one(args,"on"),
        (Context::HttpServer,"set_real_ip_from")=>args.len()==1&&validation::assert_valid_ip_or_cidr(&args[0]).is_ok(),
        (Context::HttpServer,"real_ip_header")=>args.len()==1&&validation::REAL_IP_HEADERS.contains(&args[0].as_str()),
        (Context::HttpServer,"gzip_types")=>[performance::GZIP_TEXT_TYPES,performance::GZIP_TEXT_AND_FONTS_TYPES].contains(&args.join(" ").as_str()),
        (Context::HttpServer,"gzip_min_length")=>args.len()==1&&args[0].parse::<u32>().is_ok_and(|v|v<=performance::GZIP_MIN_LENGTH_MAX),
        (Context::HttpServer,"gzip_comp_level")=>args.len()==1&&args[0].parse::<u32>().is_ok_and(|v|(performance::GZIP_COMP_LEVEL_MIN..=performance::GZIP_COMP_LEVEL_MAX).contains(&v)),
        (Context::HttpServer,"gzip_vary")=>one(args,"on")||one(args,"off"),
        (Context::HttpServer,"gzip_proxied")=>one(args,"any")||one(args,"off"),
        (Context::HttpServer|Context::Location,"allow"|"deny")=>args.len()==1&&(name=="deny"&&args[0]=="all"||validation::assert_valid_ip_or_cidr(&args[0]).is_ok()),
        (Context::Location,"proxy_pass")=>args.len()==1&&(stored_target(&args[0],&stored[1])||crate::control::internal_targets::target_authority(&args[0]).is_some_and(|authority|!authority.contains('$')&&groups.contains_key(&authority.to_ascii_lowercase()))),
        (Context::StreamServer,"proxy_pass")=>args.len()==1&&(validation::assert_valid_stream_proxy_pass(&args[0]).is_ok()||generated_group(&args[0],true)),
        (Context::Location,"proxy_bind")=>one(args,"127.0.0.2")&&protected(scope,groups),
        (Context::Location,"proxy_ssl_name")=>args.len()==1&&crate::control::render::OriginalProxyTarget::parse(&format!("https://{}",args[0])).is_ok_and(|original|original.hostname==args[0])&&lines(scope).any(|w|w.first().is_some_and(|v|v=="proxy_pass")&&w.get(1).is_some_and(|target|target.starts_with("https://nwui_resolve_"))),
        (Context::Location,"gzip")=>one(args,"on")||one(args,"off"),
        (Context::Location,"proxy_cache")=>one(args,"off")||args.len()==1&&zones.contains(&args[0]),
        (Context::Location,"proxy_cache_key")=>args.len()==1&&args[0].strip_prefix("$scheme|$host|").and_then(|v|v.strip_suffix("|$request_uri")).is_some_and(|v|v=="$proxy_host"||literal_authority(v)),
        (Context::Location,"proxy_cache_valid")=>args.len()==4&&args[..3]==["200","301","302"]&&performance::CACHE_VALIDS.contains(&args[3].as_str()),
        (Context::Location,"proxy_cache_lock"|"proxy_cache_background_update")=>one(args,"on"),
        (Context::Location,"proxy_cache_bypass"|"proxy_no_cache")=>args.len()==1&&bypass(&args[0],zones),
        (Context::Location,"proxy_cache_use_stale")=>args==["error","timeout","updating","http_500","http_502","http_503","http_504"],
        (Context::Location,"proxy_next_upstream")=>args==["error","timeout","http_502","http_503","http_504"],
        (Context::Location,"proxy_next_upstream_tries")=>one(args,"2"),
        (Context::Location,"proxy_next_upstream_timeout")=>one(args,"3s"),
        (Context::HttpServer,_)=>allowed(name,args,&stored[1..],DirectiveContext::Server),
        (Context::Location,_)=>allowed(name,args,&stored[1..],DirectiveContext::Location),
        _=>false,
    };
    ensure!(accepted,"nginx vocabulary: forbidden {name} in {context:?}");
    Ok(())
}
fn validate_nodes(config:&RuntimeConfig,file:&File,context:Context,nodes:&[Node],groups:&BTreeMap<String,bool>,zones:&BTreeSet<String>)->Result<()> {
    for node in nodes {
        match node {
            Node::Line(words,stored)=>validate_line(config,context,words,stored.as_deref().unwrap_or(words),nodes,groups,zones)?,
            Node::Block(Context::Upstream,head,body)=>validate_upstream(config,file,&head[1],body)?,
            Node::Block(Context::Map,head,body)=>{
                let zone=head[2].strip_prefix('$').and_then(|v|v.strip_suffix("_bypass"));
                ensure!(head[1]=="$http_authorization:$http_cookie"&&zone.is_some_and(|v|zones.contains(v)),"nginx vocabulary: unpinned map");
                let rows=lines(body).collect::<Vec<_>>();ensure!(rows.len()==2&&rows[0]==[":","0"]&&rows[1]==["default","1"],"nginx vocabulary: changed cache map");
            }
            Node::Block(child,head,body)=>{
                if *child==Context::Location {validation::assert_valid_location_path(&head[1]).map_err(|_|anyhow::anyhow!("nginx vocabulary: invalid location"))?;}
                validate_nodes(config,file,*child,body,groups,zones)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn validate(config:&RuntimeConfig,http:&BTreeMap<String,Vec<u8>>,stream:&BTreeMap<String,Vec<u8>>)->Result<()> {
    let mut files=Vec::new();let mut groups=BTreeMap::new();
    for (is_stream,map) in [(false,http),(true,stream)] {
        for (name,bytes) in map {
            ensure!(if is_stream {resource_file(name,"stream-")}else{name=="upstreams.conf"||name=="00-runtime-upstreams.conf"||resource_file(name,"server-")},"nginx vocabulary: unowned filename");
            let text=std::str::from_utf8(bytes)?;ensure!(!text.contains('\0'),"nginx vocabulary: NUL");
            let tokens=lexer::statements(text);
            let tail=&text[tokens.last().map_or(0,Statement::end_offset)..];
            ensure!(tail.lines().all(|line|{let line=line.trim_start_matches([' ','\t','\r']);line.is_empty()||line.starts_with('#')}),"nginx vocabulary: incomplete quote or dangling tokens");
            let context=if is_stream{Context::Stream}else{Context::Http};let mut cursor=0;let nodes=parse(text,&tokens,&mut cursor,context,false)?;
            assert_emitted_names(context,&nodes)?;
            if !is_stream {
                for node in &nodes {if let Node::Block(Context::Upstream,head,body)=node {
                    let members=lines(body).filter(|words|words.first().is_some_and(|w|w=="server")).collect::<Vec<_>>();
                    let admin=!members.is_empty()&&members.iter().all(|words|words.get(1).is_some_and(|v|v=="127.0.0.1:8080")&&!words.iter().any(|v|matches!(v.as_str(),"resolve"|"down")));
                    ensure!(groups.insert(head[1].to_ascii_lowercase(),admin).is_none(),"nginx vocabulary: duplicate HTTP upstream");
                }}
            }
            let mut blocks=BTreeMap::new();
            for block in lexer::upstream_blocks(text) {
                ensure!(blocks.insert(block.name.clone(),block).is_none(),"nginx vocabulary: duplicate upstream in file");
            }
            files.push(File{name:name.clone(),blocks,stream:is_stream,nodes});
        }
    }
    for file in &files {
        let mut zones=BTreeSet::new();
        for words in lines(&file.nodes).filter(|words|words.first().is_some_and(|w|w=="proxy_cache_path")) {
            ensure!(cache_path(&words[1..]),"nginx vocabulary: unpinned cache path");
            let zone=words[3].strip_prefix("keys_zone=").and_then(|v|v.strip_suffix(":10m")).unwrap();
            ensure!(zone_name(zone),"nginx vocabulary: invalid cache zone");zones.insert(zone.to_owned());
        }
        if file.name=="00-runtime-upstreams.conf" {ensure!(file.nodes.len()==2&&groups.contains_key("nwui_api")&&groups.contains_key("nwui_web"),"nginx vocabulary: incomplete runtime routing");}
        validate_nodes(config,file,if file.stream{Context::Stream}else{Context::Http},&file.nodes,&groups,&zones)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

    fn config() -> RuntimeConfig {
        RuntimeConfig::read(|key| match key {
            "DATABASE_URL" => Some("postgres://test@127.0.0.1/test".into()),
            "AUTH_SECRET" => Some("vocabulary-test-only".into()),
            "CERTS_DIR" => Some("/var/lib/nginx-web-ui/certs".into()),
            _ => None,
        }).unwrap()
    }

    fn http(text: &str) -> BTreeMap<String, Vec<u8>> {
        BTreeMap::from([(format!("server-{ID}.conf"), text.as_bytes().to_vec())])
    }

    #[test]
    fn refuses_root_loaded_paths_in_every_context() {
        for directive in [
            "include /etc/shadow;", "load_module /tmp/evil.so;", "error_log /tmp/root;",
            "access_log /tmp/root;", "ssl_engine /tmp/evil;", "ssl_certificate /etc/shadow;",
            "ssl_certificate_key /etc/shadow;", "proxy_ssl_trusted_certificate /etc/shadow;",
        ] {
            for text in [directive.to_owned(), format!("server {{ {directive} }}"), format!("server {{ location / {{ {directive} }} }}")] {
                assert!(validate(&config(), &http(&text), &BTreeMap::new()).is_err(), "accepted {text}");
            }
        }
        let stream = BTreeMap::from([("stream-test.conf".into(), b"server { listen 19000; proxy_pass unix:/etc/shadow; }".to_vec())]);
        assert!(validate(&config(), &BTreeMap::new(), &stream).is_err());
    }

    #[test]
    fn refuses_incomplete_syntax_and_misplaced_directives() {
        for text in [
            "server { listen 80;", "server { listen 80; }}", "server {} trailing",
            "server {} dangling; extra", "server { add_header X \"unterminated; }",
            "server { add_header X 'unterminated; }", r"server {} \",
            "proxy_pass http://peer:9000;", "location / { return 200; }",
            "server { upstream nested { server peer:9000; } }",
            "server { location / { listen 80; } }", "server { resolver 8.8.8.8; }",
            "server { location / { location /nested { return 200; } } }",
        ] {
            assert!(validate(&config(), &http(text), &BTreeMap::new()).is_err(), "accepted {text}");
        }
    }

    #[test]
    fn refuses_generated_directive_lookalikes() {
        for text in [
            "server { location / { proxy_pass http://peer:9000; proxy_bind 127.0.0.2; } }",
            "server { proxy_bind 127.0.0.2; }",
            "server { location / { proxy_pass http://127.0.0.1:8080; proxy_bind 0.0.0.0; } }",
            "server { location / { proxy_pass http://127.0.0.1:8080; proxy_bind 127.0.0.2 transparent; } }",
            "server { location / { proxy_pass http://peer:9000; proxy_cache /etc/shadow; } }",
            "proxy_cache_path /tmp/root keys_zone=attacker:10m; server {}",
            "upstream backend { zone arbitrary 64k; resolver 8.8.8.8 valid=10s; server peer:9000 resolve; }",
            "server { ssl_certificate /var/lib/nginx-web-ui/certs/../../etc/shadow; }",
        ] {
            assert!(validate(&config(), &http(text), &BTreeMap::new()).is_err(), "accepted {text}");
        }
    }

    #[test]
    fn validates_user_arguments_with_the_existing_allowlist() {
        for directive in ["gzip maybe;", "client_max_body_size 10mb;", "proxy_read_timeout nope;", "proxy_http_version 3;"] {
            let text = format!("server {{ location / {{ proxy_pass http://peer:9000; {directive} }} }}");
            assert!(validate(&config(), &http(&text), &BTreeMap::new()).is_err(), "accepted {text}");
        }
    }

    #[test]
    fn accepts_quoted_header_data_and_trailing_comments() {
        let text = "server { listen 8081; server_name example.test; include /etc/nginx/snippets/acme-challenge.conf; location / { proxy_pass http://peer:9000; add_header X-Text \"include /etc/shadow; load_module x;\"; } } # a comment with an unmatched quote \"\n # final comment";
        validate(&config(), &http(text), &BTreeMap::new()).unwrap();
        validate(&config(), &BTreeMap::new(), &BTreeMap::new()).unwrap();
    }

    #[test]
    fn accepts_generated_dns_stream_tls_cache_and_retry_families() {
        use crate::control::render;
        use serde_json::json;
        let config=config();
        let mut count=0;
        for runtime in [false,true] {for endpoint in [None,Some(config.target_dns.endpoint())] {
            let upstream=render::render_upstream_with_dns(&json!({"id":ID,"name":"backend","servers":["192.0.2.1:9000","app.test:9000 weight=2"],"resolveAtRuntime":runtime}),endpoint).unwrap();
            validate(&config,&BTreeMap::from([("upstreams.conf".into(),upstream.into_bytes())]),&BTreeMap::new()).unwrap();count+=1;
            for protocol in ["tcp","udp"] {
                let text=render::render_stream_with_dns(&json!({"id":ID,"listen":19000,"protocol":protocol,"proxyPass":"app.test:9000","resolveAtRuntime":runtime}),endpoint).unwrap();
                validate(&config,&BTreeMap::new(),&BTreeMap::from([(format!("stream-{ID}.conf"),text.into_bytes())])).unwrap();count+=1;
            }
            for tls in [false,true] {for cache in [false,true] {for retry in [false,true] {
                let mut input=json!({"serverId":ID,"serverName":"example.test","listen":8081,"cache":{"enabled":cache,"useStale":true},"gzip":{"enabled":true,"types":"text-and-fonts"},"realIp":{"preset":"cloudflare"},"ipAccess":{"mode":"whitelist","entries":["10.0.0.0/8"]},"locations":[{"path":"/","proxyPass":if tls{"https://app.test:9443/app/"}else{"http://app.test:9000/app/"},"resolveAtRuntime":runtime,"retryOtherInstances":retry,"ipAccess":{"mode":"blacklist","entries":["192.0.2.0/24"]}}]});
                if tls {input["tls"]=json!({"tlsPort":8443,"httpPort":8081,"redirectHttp":true,"certPath":config.certs_dir.join(ID).join("fullchain.pem").to_str().unwrap(),"keyPath":config.certs_dir.join(ID).join("privkey.pem").to_str().unwrap(),"tuning":{"sslProtocols":"intermediate","sslCiphers":"intermediate","preferServerCiphers":true,"hsts":{"enabled":true,"includeSubDomains":true,"preload":true}}});}
                let text=render::render_reverse_proxy_with_dns_and_resolve(&input,endpoint,&|_,_|vec![]).unwrap();
                validate(&config,&http(&text),&BTreeMap::new()).unwrap_or_else(|error|panic!("runtime={runtime} tls={tls} cache={cache} retry={retry} endpoint={endpoint:?}: {error}"));count+=1;
            }}}
        }}
        assert_eq!(count,44);
    }

    fn legacy_stream_tls(config:&RuntimeConfig,target:&str)->String {
        format!("server {{ listen 19000 ssl; ssl_certificate {}; ssl_certificate_key {}; proxy_pass {target}; }}\n",config.certs_dir.join(ID).join("fullchain.pem").display(),config.certs_dir.join(ID).join("privkey.pem").display())
    }
    #[test]
    fn accepts_complete_stream_renderer_matrix_and_refuses_nonproduced_tls() {
        use crate::control::render;
        use serde_json::json;
        let config=config();let mut count=0;let mut names=BTreeSet::new();
        for protocol in ["tcp","udp"] {for runtime in [false,true] {for endpoint in [None,Some(config.target_dns.endpoint())] {
            for target in ["192.0.2.44:9000","127.0.0.1:9000","backend.test:9000"] {
                if runtime&&target!="backend.test:9000" {continue;}
                let text=render::render_stream_with_dns(&json!({"id":ID,"listen":19000,"protocol":protocol,"proxyPass":target,"resolveAtRuntime":runtime}),endpoint).unwrap();
                for statement in lexer::statements(&text) {if statement.end==End::Semicolon {names.insert(statement.words[0].clone());}}
                validate(&config,&BTreeMap::new(),&BTreeMap::from([(format!("stream-{ID}.conf"),text.into_bytes())])).unwrap();count+=1;
            }
        }}}
        assert_eq!(count,16);
        assert_eq!(names,BTreeSet::from(["listen","proxy_pass","zone","resolver","resolver_timeout","server"].map(String::from)));
        for target in ["192.0.2.44:9000","backend.test:9000"] {
            assert!(validate(&config,&BTreeMap::new(),&BTreeMap::from([(format!("stream-{ID}.conf"),legacy_stream_tls(&config,target).into_bytes())])).is_err());
        }
    }
    #[test]
    fn nonproduced_stream_tls_and_extra_directives_are_refused() {
        let config=config();let text=legacy_stream_tls(&config,"192.0.2.44:9000");
        let cert=config.certs_dir.join(ID).join("fullchain.pem").display().to_string();
        let key=config.certs_dir.join(ID).join("privkey.pem").display().to_string();
        for changed in [
            text.replace(&cert,"/etc/shadow"),text.replace(&key,"/tmp/private.key"),
            text.replace(ID,"not-a-uuid"),text.replace(ID,"AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA"),
            text.replace(&cert,&format!("{}/../fullchain.pem",config.certs_dir.join(ID).display())),
            text.replace("ssl;","ssl udp;"),text.replace("ssl;","ssl reuseport;"),
            text.replace("ssl;","ssl; ssl_protocols TLSv1.3;"),text.replace("ssl;","ssl; ssl_session_cache shared:other:10m;"),
            text.replace("ssl;","ssl; proxy_timeout 1s;"),text.replace("ssl;","ssl; proxy_connect_timeout 1s;"),
            text.replace("ssl;","ssl; resolver 8.8.8.8;"),text.replace("ssl;","ssl; ssl_certificate /etc/shadow;"),
            text.replace("proxy_pass 192.0.2.44:9000;","proxy_pass arbitrary_upstream;"),
            text.replace("proxy_pass 192.0.2.44:9000;","proxy_pass [2001:db8::44]:9000;"),
        ] {
            assert!(validate(&config,&BTreeMap::new(),&BTreeMap::from([(format!("stream-{ID}.conf"),changed.clone().into_bytes())])).is_err(),"accepted {changed}");
        }
        for changed in [format!("ssl_certificate {cert};"),format!("upstream nwui_resolve_stream_{} {{ ssl_certificate {cert}; }}",ID.replace('-',""))] {
            assert!(validate(&config,&BTreeMap::new(),&BTreeMap::from([(format!("stream-{ID}.conf"),changed.into_bytes())])).is_err());
        }
    }

    #[test]
    fn q23_rendered_user_socket_payloads_and_legacy_relative_members_pass_vocabulary() {
        use crate::control::render;
        use serde_json::json;
        let config=config();
        for target in ["http://unix:/var/run/app.sock:/api/","http://unix:/tmp/x.sock"] {
            let input=json!({"serverId":ID,"serverName":"socket.example.test","listen":18081,"proxyPass":target});
            let rendered=render::render_reverse_proxy_with_dns_and_resolve(&input,None,&|_,_|panic!("Unix target must not resolve DNS")).unwrap();
            assert!(rendered.contains(&format!("proxy_pass {target};")));
            assert!(!rendered.contains("proxy_bind "));
            validate(&config,&http(&rendered),&BTreeMap::new()).unwrap_or_else(|error|panic!("{target}: {error}"));
        }
        let upstream=render::render_upstream(&json!({"name":"relative_socket","servers":["unix:8080"]})).unwrap();
        validate(&config,&BTreeMap::from([("upstreams.conf".into(),upstream.into_bytes())]),&BTreeMap::new()).unwrap();
        let stream=render::render_stream(&json!({"id":ID,"protocol":"tcp","listen":19000,"proxyPass":"unix:8080"})).unwrap();
        validate(&config,&BTreeMap::new(),&BTreeMap::from([(format!("stream-{ID}.conf"),stream.into_bytes())])).unwrap();
    }

    #[test]
    fn proxy_bind_needs_canonical_admin_target_and_both_forced_headers() {
        use crate::control::render;
        use serde_json::json;
        let config=config();
        let direct=json!({"serverId":ID,"serverName":"admin.test","listen":8081,"proxyPass":"http://127.0.0.1:8080/path/"});
        let text=render::render_reverse_proxy_with_dns_and_resolve(&direct,None,&|_,_|vec![]).unwrap();
        validate(&config,&http(&text),&BTreeMap::new()).unwrap();
        for changed in [text.replace("127.0.0.2;","127.0.0.3;"),text.replace("proxy_set_header X-Real-IP $remote_addr;","proxy_set_header X-Real-IP spoof;"),text.replace("proxy_set_header X-Forwarded-Proto $scheme;","")] {
            assert!(validate(&config,&http(&changed),&BTreeMap::new()).is_err());
        }
        let group=render::render_upstream(&json!({"name":"admin_ui","servers":["127.0.0.1:8080 weight=2"]})).unwrap();
        let cfg=json!({"serverId":ID,"serverName":"admin.test","listen":8081,"upstreamName":"admin_ui"});
        let text=render::render_reverse_proxy_with_admin_upstreams(&cfg,None,&BTreeSet::from(["admin_ui".into()])).unwrap();
        let mut files=http(&text);files.insert("upstreams.conf".into(),group.as_bytes().to_vec());validate(&config,&files,&BTreeMap::new()).unwrap();
        files.insert("upstreams.conf".into(),group.replace("127.0.0.1:8080 weight=2;","127.0.0.1:8080 weight=2; server 192.0.2.1:9000;").into_bytes());
        assert!(validate(&config,&files,&BTreeMap::new()).is_err());
    }

    #[test]
    fn accepts_only_fixed_runtime_routing_and_generated_certificate_paths() {
        let config=config();
        let text=format!("upstream nwui_api {{ server 127.0.0.1:{}; keepalive 16; }} upstream nwui_web {{ server 127.0.0.1:{}; keepalive 16; }}",config.listen.port(),config.web_port);
        let files=BTreeMap::from([("00-runtime-upstreams.conf".into(),text.as_bytes().to_vec())]);validate(&config,&files,&BTreeMap::new()).unwrap();
        let wrong=BTreeMap::from([("00-runtime-upstreams.conf".into(),text.replace("127.0.0.1:","192.0.2.1:").into_bytes())]);assert!(validate(&config,&wrong,&BTreeMap::new()).is_err());
        let text=format!("server {{ listen 8443 ssl; ssl_certificate {0}/{ID}/fullchain.pem; ssl_certificate_key {0}/{ID}/privkey.pem; location / {{ return 200; }} }}",config.certs_dir.display());
        validate(&config,&http(&text),&BTreeMap::new()).unwrap();
        for changed in [text.replace("fullchain.pem","other.pem"),text.replace(ID,"not-a-uuid"),text.replace(&format!("/{ID}/"),&format!("/../{ID}/"))] {assert!(validate(&config,&http(&changed),&BTreeMap::new()).is_err());}
    }

    #[test]
    fn accepts_proxy_form_matrix_and_retry_with_available_or_unavailable_dns() {
        use crate::{contracts::FormInput, control::{location_refs, render, resources::{self, Resource}, reserved_ports::ReservedPorts}};
        use serde_json::{Value,json};
        let config=config();
        let names=BTreeMap::from([("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(),"backend".into())]);
        let sets=[None,Some(json!([])),Some(json!([{"name":"proxy_set_header","args":["Host","$host"]}])),Some(json!([{"name":"add_header","args":["X-First","one"]},{"name":"proxy_read_timeout","args":["60s"]},{"name":"proxy_set_header","args":["Host","$host"]}])),Some(json!([{"name":"return","args":["302","/next"]},{"name":"rewrite","args":["^/old$","/new","break"]}])),Some(json!([{"name":"proxy_set_header","args":["X-Empty",""]}])),Some(json!([{"name":"proxy_set_header","args":["X-Quoted","two words"]}]))];
        let mut count=0;
        for target in 0..4 {for dirs in &sets {for uri in ["http://app.test/","http://app.test:80","http://app.test/path/"] {for cache in [false,true] {for gzip in [false,true] {for real in [Value::Null,json!({"preset":"cloudflare"}),json!({"preset":"custom","trustedRanges":["10.0.0.0/8"],"header":"X-Forwarded-For","recursive":true})] {for tls in [false,true] {for retry in [None,Some(false),Some(true)] {
            let upstream=target<2;
            let mut old=json!({"serverId":ID,"serverName":"example.test","listen":8081,"cache":{"enabled":cache},"gzip":{"enabled":gzip},"realIp":real,"serverDirectives":[{"name":"add_header","args":["X-Example","one"]},{"name":"proxy_read_timeout","args":["300s"]},{"name":"server_tokens","args":["off"]}]});
            if upstream {old["upstreamName"]=json!("backend");}else{old["proxyPass"]=json!(uri);if target==3{old["resolveAtRuntime"]=json!(true);}}
            if let Some(dirs)=dirs{old["locationDirectives"]=dirs.clone();}
            if tls{old["tls"]=json!({"tlsPort":8443,"httpPort":8081,"redirectHttp":false,"certPath":config.certs_dir.join(ID).join("fullchain.pem").to_str().unwrap(),"keyPath":config.certs_dir.join(ID).join("privkey.pem").to_str().unwrap(),"tuning":{"hsts":{"enabled":true,"maxAge":3600,"includeSubDomains":true}}});}
            if let Some(retry)=retry {let mut locations=render::effective_locations(&old);locations[0]["retryOtherInstances"]=json!(retry);old["locations"]=json!(locations);}
            let mut form=FormInput::default();
            let mut location=json!({"path":"/","mode":if upstream{"upstream"}else{"direct"},"directives":dirs.clone().unwrap_or(Value::Null)});
            if upstream{location["upstreamId"]=json!("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");}else{location["proxyPass"]=json!(uri);if target==3{location["resolveAtRuntime"]=json!(true);}}
            if let Some(retry)=retry{location["retryOtherInstances"]=json!(retry);}
            for (key,value) in [("serverName","example.test".into()),("listen","8081".into()),("targetMode","locations".into()),("locations",json!([location]).to_string())]{form.fields.insert(key.into(),vec![value]);}
            for key in ["serverDirectives","cache","gzip","realIp"]{form.fields.insert(key.into(),vec![old[key].to_string()]);}
            if tls{for (key,value) in [("certificateId",ID.into()),("tlsPort","8443".into()),("redirectHttp","false".into()),("tlsOptions",old["tls"]["tuning"].to_string())]{form.fields.insert(key.into(),vec![value]);}}
            let wire=resources::form_body(Resource::Server,&form).unwrap();
            for node in [false,true] {
                let raw=if node{serde_json::from_slice(&serde_json::to_vec(&wire).unwrap()).unwrap()}else{wire.clone()};
                let parsed=resources::parse_server(&raw,&ReservedPorts::default()).unwrap();
                let mut converted=location_refs::resolve_locations(&parsed["config"],&names).unwrap();converted["serverId"]=json!(ID);converted["serverName"]=json!("example.test");converted["listen"]=json!(8081);if tls{let mut material=old["tls"].clone();material["tlsPort"]=converted["tlsPort"].clone();material["httpPort"]=parsed["listen"].clone();material["redirectHttp"]=converted["redirectHttp"].clone();material["tuning"]=converted["tlsOptions"].clone();converted["tls"]=material;}
                for endpoint in [None,Some(config.target_dns.endpoint())] {
                    let group=if upstream{render::render_upstream_with_dns(&json!({"id":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","name":"backend","servers":["app.test:9000"],"resolveAtRuntime":target==1}),endpoint).unwrap()}else{String::new()};
                    let before=render::render_reverse_proxy_with_dns_and_resolve(&old,endpoint,&|_,_|vec![]).unwrap();
                    let after=render::render_reverse_proxy_with_dns_and_resolve(&converted,endpoint,&|_,_|vec![]).unwrap();
                    assert_eq!(before,after,"target={target} tls={tls} retry={retry:?} node={node}");
                    let mut files=http(&after);if upstream{files.insert("upstreams.conf".into(),group.into_bytes());}
                    validate(&config,&files,&BTreeMap::new()).unwrap_or_else(|error|panic!("target={target} tls={tls} cache={cache} gzip={gzip} retry={retry:?} node={node}: {error}"));count+=1;
                }
            }
        }}}}}}}}
        assert_eq!(count,24192);
    }


    fn decode(value:&serde_json::Value)->serde_json::Value {
        use serde_json::{Value,json};
        match value {
            Value::Object(values)=>Value::Object(values.iter().filter(|(_,v)|v.get("$undefined")!=Some(&json!(true))).map(|(k,v)|(k.clone(),decode(v))).collect()),
            Value::Array(values)=>Value::Array(values.iter().map(decode).collect()),
            _=>value.clone(),
        }
    }

    #[test]
    #[ignore = "golden vectors: set NWUI_CONTROL_FIXTURES"]
    fn accepts_every_render_golden_with_only_synthetic_certificate_paths_rebased() {
        use crate::control::render;
        use serde_json::{Value,json};
        let config=config();
        let fixture:Value=serde_json::from_slice(&std::fs::read(std::path::Path::new(&std::env::var("NWUI_CONTROL_FIXTURES").expect("set NWUI_CONTROL_FIXTURES")).join("nginx/render.json")).expect("render fixture")).unwrap();
        let mut count=0;let mut rejected=0;
        for vector in fixture["vectors"].as_array().unwrap() {
            let mut input=decode(&vector["args"][0]);
            if input["tls"].is_object(){
                input["tls"]["certPath"]=json!(config.certs_dir.join(ID).join("fullchain.pem").to_str().unwrap());
                input["tls"]["keyPath"]=json!(config.certs_dir.join(ID).join("privkey.pem").to_str().unwrap());
            }
            let function=vector["fn"].as_str().unwrap();
            let rendered=match function {
                "renderReverseProxy"=>render::render_reverse_proxy(&input),
                "renderUpstream"=>render::render_upstream(&input),
                "renderStream"=>render::render_stream(&input),
                _=>panic!("unknown corpus function {function}"),
            };
            match rendered {
                Ok(text)=>{
                    let (http,stream)=match function {
                        "renderReverseProxy"=>(http(&text),BTreeMap::new()),
                        "renderUpstream"=>(BTreeMap::from([("upstreams.conf".into(),text.into_bytes())]),BTreeMap::new()),
                        _=>(BTreeMap::new(),BTreeMap::from([(format!("stream-{ID}.conf"),text.into_bytes())])),
                    };
                    for (is_stream,files) in [(false,&http),(true,&stream)] {
                        for bytes in files.values() {
                            let tokens=lexer::statements(std::str::from_utf8(bytes).unwrap());let mut cursor=0;
                            let context=if is_stream{Context::Stream}else{Context::Http};
                            let nodes=parse(std::str::from_utf8(bytes).unwrap(),&tokens,&mut cursor,context,false).unwrap();
                            assert_emitted_names(context,&nodes).unwrap_or_else(|e|panic!("{}: {e}",vector["id"]));
                        }
                    }
                    if vector["id"]=="render.renderReverseProxy#0063" {
                        // This rollback-failure fixture intentionally renders `proxy_pass ;`; apply skips it.
                        assert!(validate(&config,&http,&stream).is_err());
                        let row=json!({"id":ID,"enabled":true,"serverName":input["serverName"],"listen":input["listen"],"config":input});
                        let outcome=crate::publisher::apply::render_config(&config,&crate::publisher::apply::ConfigSnapshotData{servers:vec![row],upstreams:vec![],streams:vec![]},&[]);
                        assert_eq!(outcome.skipped.len(),1);assert!(outcome.rendered.conf.get(&format!("server-{ID}.conf")).is_none());
                        validate(&config,&outcome.rendered.conf,&outcome.rendered.stream).unwrap();
                    }else{validate(&config,&http,&stream).unwrap_or_else(|e|panic!("{}: {e}",vector["id"]));}
                }
                Err(_)=>{assert!(vector.get("error").is_some()||vector["hardeningExpected"].get("error").is_some(),"unexpected render error {}",vector["id"]);rejected+=1;}
            }
            count+=1;
        }
        assert_eq!((count,rejected),(83,11));
        // Stream TLS is not renderer-produced, even with generated-looking certificate paths.
        let legacy=legacy_stream_tls(&config,"192.0.2.44:9000");
        assert!(validate(&config,&BTreeMap::new(),&BTreeMap::from([(format!("stream-{ID}.conf"),legacy.into_bytes())])).is_err());
    }

    #[test]
    fn accepts_stored_apply_compatibility_corpus_without_save_strictness() {
        use crate::control::render;
        use serde_json::{Value,json};
        let cases:Vec<Value>=serde_json::from_str(include_str!("../../tests/fixtures/legacy-apply-inputs.json")).unwrap();
        for case in cases {
            let mut input=case["config"].clone();
            if input.get("realIp").is_some(){input["realIp"]=validation::parse_stored_real_ip_config(&input["realIp"]).unwrap().unwrap_or(serde_json::Value::Null);}
            input["serverName"]=json!("legacy.test");input["listen"]=json!(18081);
            let text=render::render_reverse_proxy(&input).unwrap();
            assert_eq!(text,case["expected"].as_str().unwrap(),"{}",case["name"]);
            if case["config"]["proxyPass"].as_str().is_some_and(|v|v.ends_with('\\')) {
                assert!(validate(&config(),&http(&text),&BTreeMap::new()).is_err(),"trailing backslash remains forbidden");
            }else{validate(&config(),&http(&text),&BTreeMap::new()).unwrap_or_else(|e|panic!("{}: {e}",case["name"]));}
        }
        let text="server { server_tokens On; gzip On; client_max_body_size 500m; location / { proxy_pass http://peer:9000; proxy_read_timeout 600s; } }";
        validate(&config(),&http(text),&BTreeMap::new()).unwrap();
    }

    #[test]
    fn probe_only_tls_directives_are_not_renderer_vocabulary() {
        for directive in ["proxy_ssl_server_name on;","proxy_ssl_verify off;","proxy_ssl_certificate /etc/shadow;","proxy_ssl_certificate_key /etc/shadow;"] {
            let text=format!("server {{ location / {{ proxy_pass https://peer:9000; {directive} }} }}");
            assert!(validate(&config(),&http(&text),&BTreeMap::new()).is_err());
        }
    }

    #[test]
    fn named_numeric_and_reserved_prefix_legacy_groups_keep_nginx_resolution() {
        use crate::control::render;
        use serde_json::json;
        for name in ["09","123","nwui_resolve_legacy"] {
            let group=render::render_upstream(&json!({"name":name,"servers":["192.0.2.1:9000"]})).unwrap();
            let text=render::render_reverse_proxy(&json!({"serverName":"example.test","listen":8081,"upstreamName":name})).unwrap();
            let mut files=http(&text);files.insert("upstreams.conf".into(),group.into_bytes());
            validate(&config(),&files,&BTreeMap::new()).unwrap_or_else(|e|panic!("{name}: {e}"));
        }
    }
    #[test]
    fn unavailable_dns_placeholders_must_belong_to_the_shared_generated_grammar() {
        use crate::control::render;
        use serde_json::json;
        let text=render::render_upstream_with_dns(&json!({"id":ID,"name":"backend","servers":["app.test:9000 weight=2"],"resolveAtRuntime":true}),None).unwrap();
        let files=|text:String|BTreeMap::from([("upstreams.conf".into(),text.into_bytes())]);
        validate(&config(),&files(text.clone()),&BTreeMap::new()).unwrap();
        for changed in [text.replace("# runtime DNS unavailable: app.test:9000 weight=2", "# not a generated placeholder"),text.replace("127.0.0.1:1 weight=2 down", "127.0.0.1:2 weight=2 down"),text.replace("app.test:9000 weight=2", "192.0.2.1:9000 weight=2")] {
            assert!(validate(&config(),&files(changed),&BTreeMap::new()).is_err());
        }
    }
    #[test]
    fn stored_nginx_escapes_keep_apply_argument_acceptance_after_lexing() {
        use crate::control::render;
        use serde_json::json;
        for suffix in [r"a\nb",r"a\rb",r"a\tb"] {
            let target=format!("http://peer:9000/{suffix}");
            validation::assert_valid_stored_proxy_target(&target).unwrap();
            let regex=format!("^/{suffix}$");
            let args=vec![regex.clone(),"/next".into(),"break".into()];
            validation::assert_valid_directive("rewrite",&args,DirectiveContext::Location).unwrap();
            let text=render::render_reverse_proxy(&json!({"serverName":"example.test","listen":8081,"proxyPass":target,"locationDirectives":[{"name":"rewrite","args":args}]})).unwrap();
            validate(&config(),&http(&text),&BTreeMap::new()).unwrap_or_else(|e|panic!("{suffix}: {e}"));
        }
        for value in [r"a\nb",r"a\rb"] {
            let text=format!("server {{ location / {{ proxy_pass http://peer:9000; add_header X-Test \"{value}\"; }} }}");
            assert!(validate(&config(),&http(&text),&BTreeMap::new()).is_err());
        }
    }
    #[test]
    fn quoted_raw_controls_do_not_inherit_an_escape_elsewhere() {
        for control in ['\n','\r','\t'] {
            let double_escape=r"\\n";
            for text in [
                format!("server {{ location / {{ proxy_pass \"http://peer:9000/a{control}b\"; }} }}"),
                format!("server {{ location / {{ proxy_pass \"http://peer:9000/a{control}b\\nc\"; }} }}"),
                format!("server {{ location / {{ proxy_pass \"http://peer:9000/a{control}b{double_escape}c\"; }} }}"),
                format!("server {{ location / {{ proxy_pass http://peer:9000; rewrite \"^/a{control}b$\" /next break; }} }}"),
                format!("server {{ location / {{ proxy_pass http://peer:9000; rewrite \"^/a{control}b$\" /next\\n break; }} }}"),
                format!("server {{ location / {{ proxy_pass http://peer:9000; proxy_set_header \"X{control}Name\" value; }} }}"),
            ] {
                assert!(validate(&config(),&http(&text),&BTreeMap::new()).is_err(),"accepted raw {control:?}: {text:?}");
            }
        }
        for escaped in [r"X\nName",r"X\rName",r"X\tName"] {
            let text=format!("server {{ location / {{ proxy_pass http://peer:9000; proxy_set_header {escaped} value; }} }}");
            assert!(validate(&config(),&http(&text),&BTreeMap::new()).is_err());
        }
        assert!(validate(&config(),&http("server { location / { proxy_pass http://peer:9000/a\0n; } }"),&BTreeMap::new()).is_err());
    }
}
