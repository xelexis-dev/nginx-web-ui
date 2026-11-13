// Explicit collection only: no CI network access, no licence text in logs/evidence.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { setDefaultAutoSelectFamilyAttemptTimeout } from 'node:net';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { root, sha256, inventory, approved, splitTexts, bundle, standardStatement } from './generate.mjs';
// Allow intercontinental TCP setup before trying the next address.
setDefaultAutoSelectFamilyAttemptTimeout(1000);
const nodeImage=readFileSync(join(root,'Dockerfile'),'utf8').match(/^ARG NODE_IMAGE=(.*)$/m)[1];
let archives=new WeakMap();
export function decodeSource(bytes) {
  try {const text=new TextDecoder('utf-8',{fatal:true,ignoreBOM:true}).decode(bytes);return {text,encoding:'utf-8',replacementCharacters:(text.match(/\uFFFD/g)??[]).length};}
  catch {return {text:bytes.toString('latin1'),encoding:'latin-1',replacementCharacters:0};}
}
function records(bytes) {
  if(!archives.has(bytes))archives.set(bytes,{sha256:sha256(bytes),files:new Map()});
  return archives.get(bytes);
}
function encodings(bytes) {return [...records(bytes).files.values()].sort((a,b)=>a.source<b.source?-1:a.source>b.source?1:0);}
function baseArchive(image,script) {
  return execFileSync('docker',['run','--rm','--pull=never','--read-only','--network','none','--memory','384m','--cpus','0.5','--label','nwui.task=licence-collect','--entrypoint','sh',image,'-c',script],{maxBuffer:64*1024*1024,stdio:['ignore','pipe','ignore']});
}
function baseLicences(image,script) {
  const bytes=baseArchive(image,script);
  const parts=files(bytes).filter(n=>!n.endsWith('/')).map(source=>({source,text:extract(bytes,[source])})).filter(p=>p.text.trim());
  return {text:bundle(parts),encodings:encodings(bytes)};
}
let common;
function commonLicences() {
  return common??=baseLicences(nodeImage,'cd /; tar --hard-dereference -hcf - usr/share/common-licenses/*');
}
const pending=new Map();
export function resetCollection() {pending.clear();common=undefined;archives=new WeakMap();}
const once=(key,fn)=>{if(!pending.has(key))pending.set(key,fn());return pending.get(key);};
async function get(url) {
  const host=new URL(url).hostname;
  if(!['registry.npmjs.org','static.crates.io','snapshot.debian.org','codeload.github.com','skarnet.org','nginx.org','apt.postgresql.org','www.bearssl.org'].includes(host))throw new Error('Unapproved evidence host');
  for(let attempt=0;attempt<3;attempt++) {
    try {
      const r=await fetch(url,{redirect:'error',signal:AbortSignal.timeout(30000)});
      if(r.ok)return Buffer.from(await r.arrayBuffer());
      if(r.status<500)break;
    }catch { /* Retry transient archive transport failures without logging URLs. */ }
  }
  throw new Error('Exact archive unavailable');
}
const json=async url=>JSON.parse(decodeSource(await get(url)).text);
function files(bytes) {
  const listing=execFileSync('tar',['-tf','-'],{input:bytes,maxBuffer:32*1024*1024,stdio:['pipe','pipe','ignore']});
  const names=decodeSource(listing).text.trimEnd().split('\n');
  if(names.some(n=>n.startsWith('/')||n.split('/').includes('..')||/[\r\x00]/.test(n)))throw new Error('Unsafe archive');
  return names;
}
export function extract(bytes,paths) {
  const raw=execFileSync('tar',['-xOf','-','--',...paths],{input:bytes,maxBuffer:64*1024*1024,stdio:['pipe','pipe','ignore']}),decoded=decodeSource(raw),archive=records(bytes);
  archive.files.set(paths.join(' | '),{archiveSha256:archive.sha256,source:paths.join(' | '),encoding:decoded.encoding,sha256:sha256(raw),replacementCharacters:decoded.replacementCharacters});
  return decoded.text;
}
export function licenceFiles(bytes,names) {
  const paths=names.filter(n=>!n.endsWith('/')&&(/(?:^|\/)(?:licen[sc]e(?:[._-].*)?|copying(?:[._-].*)?|copyright(?:[._-].*)?|notice(?:[._-].*)?)$/i.test(n)||/\/LICENSES\/[^/]+$/i.test(n)));
  const extracted=paths.map(p=>({path:p,text:extract(bytes,[p])})).filter(p=>p.text.trim());
  if(!extracted.length) {
    for(const name of names.filter(n=>/(?:readme(?:\.[^/]+)?|[^/]+\.(?:js|rs|c|h))$/i.test(n))) {
      const content=extract(bytes,[name]);
      if(!/(?:Permission is hereby granted|Redistribution and use in source and binary forms)/i.test(content)||!/(?:IN NO EVENT|ANY EXPRESS OR IMPLIED WARRANTIES)/i.test(content))continue;
      const comment=content.match(/\/\*[\s\S]*?\*\//g)?.find(t=>content.indexOf(t)<1024&&/(?:Permission is hereby granted|Redistribution and use in source and binary forms)/i.test(t)&&/(?:IN NO EVENT|ANY EXPRESS OR IMPLIED WARRANTIES)/i.test(t));
      if(comment)return `File: ${name} (complete licence comment)\n\n${comment}\n`;
      if(/readme/i.test(name))return `File: ${name} (includes the complete licence)\n\n${content.trimEnd()}\n`;
    }
    throw new Error('No complete licence text in archive');
  }
  return extracted.sort((a,b)=>a.path<b.path?-1:a.path>b.path?1:0).map(p=>`File: ${p.path}\n\n${p.text.trimEnd()}\n`).join('\n');
}

function repo(value) {
  const raw=typeof value==='string'?value:value?.url;
  if(!raw)throw new Error('Missing upstream repository');
  const url=new URL(raw.replace(/^git\+/,'').replace(/^git:\/\//,'https://'));
  if(url.protocol!=='https:'||url.hostname!=='github.com'||url.username||url.password||url.search||url.hash)throw new Error('Unapproved upstream repository');
  const path=url.pathname.replace(/\.git$/,'').replace(/\/$/,'');
  if(!/^\/[^/]+\/[^/]+$/.test(path)||path.startsWith('/xelexis-dev/'))throw new Error('Unapproved upstream repository');
  return path;
}
async function githubSource(repository,revision) {
  if(!/^[a-f0-9]{40}$/.test(revision??''))throw new Error('Missing exact upstream source revision');
  const path=repo(repository);
  return once(`upstream:${path}:${revision}`,async()=>{
    const bytes=await get(`https://codeload.github.com${path}/tar.gz/${revision}`);
    const names=files(bytes),rootNames=names.filter(n=>!n.endsWith('/'));
    return {text:licenceFiles(bytes,rootNames),archives:[{identity:`github:${path}=${revision}`,sha256:sha256(bytes)}],encodings:encodings(bytes)};
  });
}


function arMember(bytes,prefix) {
  if(bytes.subarray(0,8).toString()!=='!<arch>\n')throw new Error('Invalid Debian archive');
  for(let at=8;at+60<=bytes.length;) {
    const name=bytes.subarray(at,at+16).toString().trim().replace(/\/$/,'');
    const size=Number(bytes.subarray(at+48,at+58).toString().trim());
    if(!Number.isSafeInteger(size)||size<0||at+60+size>bytes.length)throw new Error('Invalid Debian member');
    if(name.startsWith(prefix))return bytes.subarray(at+60,at+60+size);
    at+=60+size+(size%2);
  }
  throw new Error('Missing Debian archive member');
}
async function binaryDeb(row) {
  if(row.name==='libcrypt-dev')row=inventory().find(r=>r.name==='libcrypt1'&&r.arch===row.arch&&r.sourceVersion===row.sourceVersion);
  const pins=Object.fromEntries([...readFileSync(join(root,'Dockerfile'),'utf8').matchAll(/^ARG ([A-Z][A-Z0-9_]*)=(.*)$/gm)].map(m=>[m[1],m[2]]));
  let bytes;
  if(row.name==='nginx'||['postgresql-15','postgresql-client-15'].includes(row.name)) {
    const arch=row.arch.toUpperCase();
    const key=row.name==='nginx'?`NGINX_DEB_${arch}_SHA256`:`PG15_${row.name==='postgresql-15'?'SERVER':'CLIENT'}_${arch}_SHA256`;
    const url=row.name==='nginx'?`https://nginx.org/packages/debian/pool/nginx/n/nginx/nginx_${row.version}_${row.arch}.deb`:`https://apt.postgresql.org/pub/repos/apt/pool/main/p/postgresql-15/${row.name}_${row.version}_${row.arch}.deb`;
    bytes=await get(url);if(sha256(bytes)!==pins[key])throw new Error('Debian pinned binary checksum mismatch');
  }else {
    const meta=await json(`https://snapshot.debian.org/mr/binary/${row.name}/${encodeURIComponent(row.version)}/binfiles`);
    if(meta.binary!==row.name||meta.binary_version!==row.version)throw new Error('Debian binary identity mismatch');
    const file=meta.result.find(r=>r.architecture===row.arch);
    if(!file)throw new Error('Debian binary architecture missing');
    bytes=await get(`https://snapshot.debian.org/file/${file.hash}`);
    if(createHash('sha1').update(bytes).digest('hex')!==file.hash)throw new Error('Debian binary checksum mismatch');
  }
  const control=arMember(bytes,'control.tar'),controlFile=files(control).find(n=>/(?:^|\/)control$/.test(n));
  const fields=Object.fromEntries([...extract(control,[controlFile]).matchAll(/^([A-Za-z]+): ([^\n]+)$/gm)].map(m=>[m[1],m[2]]));
  if(fields.Package!==row.name||fields.Version!==row.version||fields.Architecture!==row.arch)throw new Error('Debian binary control mismatch');
  const data=arMember(bytes,'data.tar');
  const copyright=files(data).find(n=>n.endsWith(`/share/doc/${row.name}/copyright`));
  if(!copyright)throw new Error('Debian binary copyright missing');
  const text=extract(data,[copyright]);
  if(!text.trim())throw new Error('Debian binary copyright is a link');
  const ids=[...new Set([...text.matchAll(/^License:\s*([^\n]+)/gm)].map(m=>m[1].trim()))].sort();
  return {spdx:ids.length?ids.join(' AND '):'LicenseRef-Debian-copyright',text:`File: ${copyright}\n\n${text}\nReferenced common licence texts are reproduced in the DEBIAN_COMMON_LICENSES block.\n`,archiveSha256:sha256(bytes),encodings:[...encodings(control),...encodings(data)]};
}

export async function collect(row) {
  if(row.ecosystem==='npm') return once(row.id,async()=>{
    const meta=await json(`https://registry.npmjs.org/${encodeURIComponent(row.name)}/${encodeURIComponent(row.version)}`);
    if(meta.name!==row.name||meta.version!==row.version||!meta.dist?.integrity)throw new Error('npm identity/integrity missing');
    const bytes=await get(meta.dist.tarball), [algorithm,expected]=meta.dist.integrity.split('-');
    if(!['sha512','sha256'].includes(algorithm)||createHash(algorithm).update(bytes).digest('base64')!==expected)throw new Error('npm integrity mismatch');
    const names=files(bytes), manifestPath=names.find(n=>/^[^/]+\/package\.json$/.test(n));
    if(!manifestPath)throw new Error('Missing npm manifest');
    const manifest=JSON.parse(extract(bytes,[manifestPath]));
    if(manifest.name!==row.name||manifest.version!==row.version)throw new Error('npm archive identity mismatch');
    const declared=manifest.license??meta.license;
    const spdx=typeof declared==='string'?declared:declared?.type;
    if(!spdx)throw new Error('Missing npm licence declaration');
    if(approved[row.id])return standardNpm(row,manifest,meta,spdx,bytes);
    let text,upstreamArchives=[],extraEncodings=[];
    try{text=licenceFiles(bytes,names);}catch {
      if(meta.gitHead) {
        const upstream=await githubSource(meta.repository,meta.gitHead);
        text=upstream.text;upstreamArchives=upstream.archives;extraEncodings=upstream.encodings;
      }else {
        const parent=row.name.startsWith('@next/')?'next':row.name.startsWith('@turbo/')?'turbo':null;
        if(!parent||!readFileSync(join(root,'yarn.lock'),'utf8').includes(`resolution: "${parent}@npm:${row.version}"`))throw new Error('Missing npm upstream licence binding');
        const parentMeta=await json(`https://registry.npmjs.org/${parent}/${row.version}`);
        if(repo(meta.repository)!==repo(parentMeta.repository)||parentMeta.license!==spdx)throw new Error('npm upstream licence identity mismatch');
        const parentRow={...row,id:`npm:${parent}=${row.version}`,name:parent};
        const result=await collect(parentRow);text=result.text;
        upstreamArchives=[{identity:parentRow.id,sha256:result.archiveSha256},...(result.upstreamArchives??[])];extraEncodings=result.encodings??[];
      }
    }
    return {spdx,text,archiveSha256:sha256(bytes),upstreamArchives,encodings:[...encodings(bytes),...extraEncodings]};
  });
  if(row.ecosystem==='cargo')return once(row.id,async()=>{
    const bytes=await get(`https://static.crates.io/crates/${row.name}/${row.name}-${row.version}.crate`);
    if(sha256(bytes)!==row.lockedChecksum)throw new Error('Cargo archive checksum mismatch');
    const names=files(bytes),manifest=extract(bytes,[`${row.name}-${row.version}/Cargo.toml`]);
    const section=manifest.split(/^\[package\]\s*$/m)[1]?.split(/^\[/m)[0];
    const spdx=section?.match(/^license\s*=\s*"([^"]+)"/m)?.[1];
    if(!spdx)throw new Error('Missing Cargo licence declaration');
    if(approved[row.id])return electedCrate(row,spdx,bytes,names);
    let text,upstreamArchives=[],extraEncodings=[];
    try{text=licenceFiles(bytes,names);}catch {
      const vcsPath=names.find(n=>n.endsWith('/.cargo_vcs_info.json'));
      if(!vcsPath)throw new Error('Missing Cargo upstream licence binding');
      const revision=JSON.parse(extract(bytes,[vcsPath])).git?.sha1;
      const repository=section?.match(/^repository\s*=\s*"([^"]+)"/m)?.[1];
      const upstream=await githubSource(repository,revision);text=upstream.text;upstreamArchives=upstream.archives;extraEncodings=upstream.encodings;
    }
    return {spdx,text,archiveSha256:sha256(bytes),upstreamArchives,encodings:[...encodings(bytes),...extraEncodings]};
  });
  if(row.ecosystem==='deb')return once(`deb-source:${row.source}=${row.sourceVersion}`,async()=>{
    const meta=await json(`https://snapshot.debian.org/mr/package/${encodeURIComponent(row.source)}/${encodeURIComponent(row.sourceVersion)}/srcfiles`);
    if(meta.package!==row.source||meta.version!==row.sourceVersion)throw new Error('Debian identity mismatch');
    const refs=await Promise.all(meta.result.map(async r=>({hash:r.hash,info:await json(`https://snapshot.debian.org/mr/file/${r.hash}/info`)})));
    const archive=refs.find(r=>r.info.result.some(f=>/\.debian\.tar\./.test(f.name)))??refs.find(r=>r.info.result.some(f=>/\.tar\.(?:gz|xz|bz2)$/.test(f.name)&&!f.name.includes('.orig.')));
    if(!archive)throw new Error('Debian copyright source archive missing');
    const bytes=await get(`https://snapshot.debian.org/file/${archive.hash}`);
    if(createHash('sha1').update(bytes).digest('hex')!==archive.hash)throw new Error('Debian archive checksum mismatch');
    const names=files(bytes),copyright=names.find(n=>/(?:^|\/)debian\/copyright$/.test(n));
    if(!copyright)throw new Error('Debian copyright missing');
    const text=extract(bytes,[copyright]);
    const ids=[...new Set([...text.matchAll(/^License:\s*([^\n]+)/gm)].map(m=>m[1].trim()))].sort();
    // Non-machine-readable terms retain their full text under a licence reference.
    if(!ids.length)ids.push('LicenseRef-Debian-copyright');
    const common=[...new Set([...text.matchAll(/\/usr\/share\/common-licenses\/([A-Za-z0-9.+-]+)/g)].map(m=>m[1]))];
    const appendix=common.length?'\nReferenced common licence texts are reproduced in the DEBIAN_COMMON_LICENSES block.\n':'';
    return {spdx:ids.join(' AND '),text:`File: ${copyright}\n\n${text}${appendix}`,archiveSha256:sha256(bytes),encodings:encodings(bytes)};
  }).catch(()=>once(`deb-binary:${row.name}=${row.version}:${row.arch}`,()=>binaryDeb(row)));
  if(row.name==='DEBIAN_COMMON_LICENSES')return {spdx:'LicenseRef-Debian-common-licenses',...commonLicences(),archiveSha256:row.version.split('sha256:')[1]};
  if(row.name==='NODE_IMAGE')return {spdx:'LicenseRef-Node-distribution',...baseLicences(row.version,'cd /; tar --hard-dereference -hcf - usr/local/LICENSE usr/local/lib/node_modules/npm/LICENSE usr/local/lib/node_modules/corepack/LICENSE.md'),archiveSha256:row.version.split('sha256:')[1]};
  if(row.name==='RUST_IMAGE')return {spdx:'LicenseRef-Rust-distribution',...baseLicences(row.version,'cd /; for d in usr/local/rustup/toolchains/*; do set -- "$@" "$d"/share/doc/cargo/LICENSE* "$d"/share/doc/rust/licenses/* "$d"/share/doc/rust/COPYRIGHT*; done; tar --hard-dereference -hcf - "$@"'),archiveSha256:row.version.split('sha256:')[1]};
  if(row.name==='S6_OVERLAY_VERSION') {
    const bytes=await get(`https://codeload.github.com/just-containers/s6-overlay/tar.gz/refs/tags/v${row.version}`);
    const names=files(bytes);let text=licenceFiles(bytes,names);
    const versions=extract(bytes,[`s6-overlay-${row.version}/conf/versions`]);
    const archives=[{identity:`s6-overlay-source=${row.version}`,sha256:sha256(bytes)}],sourceEncodings=[...encodings(bytes)];
    const packages={SKALIBS:'skalibs',EXECLINE:'execline',S6:'s6',S6_RC:'s6-rc',S6_LINUX_INIT:'s6-linux-init',S6_PORTABLE_UTILS:'s6-portable-utils',S6_LINUX_UTILS:'s6-linux-utils',S6_DNS:'s6-dns',S6_NETWORKING:'s6-networking'};
    for(const [key,name] of Object.entries(packages)) {
      const version=versions.match(new RegExp('^'+key+'_VERSION=v([0-9.]+)$','m'))?.[1];
      if(!version)throw new Error('Missing overlay dependency version');
      const archive=await get(`https://skarnet.org/software/${name}/${name}-${version}.tar.gz`);
      text+='\n'+licenceFiles(archive,files(archive));sourceEncodings.push(...encodings(archive));archives.push({identity:`${name}=${version}`,sha256:sha256(archive)});
    }
    const helpers=versions.match(/^S6_OVERLAY_HELPERS_VERSION=v([0-9.]+)$/m)?.[1];
    if(!helpers)throw new Error('Missing overlay helper version');
    const archive=await get(`https://codeload.github.com/just-containers/s6-overlay-helpers/tar.gz/refs/tags/v${helpers}`);
    text+='\n'+licenceFiles(archive,files(archive));sourceEncodings.push(...encodings(archive));archives.push({identity:`s6-overlay-helpers=${helpers}`,sha256:sha256(archive)});
    const bear=versions.match(/^BEARSSL_VERSION=([a-f0-9]{40})$/m)?.[1];
    if(!bear)throw new Error('Missing overlay BearSSL revision');
    const bearArchive=await get(`https://www.bearssl.org/gitweb/?p=BearSSL;a=snapshot;h=${bear};sf=tgz`);
    text+='\n'+licenceFiles(bearArchive,files(bearArchive));sourceEncodings.push(...encodings(bearArchive));archives.push({identity:`BearSSL=${bear}`,sha256:sha256(bearArchive)});
    const pins=Object.fromEntries([...readFileSync(join(root,'Dockerfile'),'utf8').matchAll(/^ARG ([A-Z][A-Z0-9_]*)=(.*)$/gm)].map(m=>[m[1],m[2]]));
    for(const [name,key] of [['noarch','S6_NOARCH_SHA256'],['x86_64','S6_AMD64_SHA256'],['aarch64','S6_ARM64_SHA256']]) {
      const response=await fetch(`https://github.com/just-containers/s6-overlay/releases/download/v${row.version}/s6-overlay-${name}.tar.xz`,{signal:AbortSignal.timeout(60000)});
      if(!response.ok)throw new Error('Missing pinned overlay artifact');
      const artifact=Buffer.from(await response.arrayBuffer());
      if(sha256(artifact)!==pins[key])throw new Error('Overlay artifact checksum mismatch');
      archives.push({identity:`s6-overlay-${name}=${row.version}`,sha256:sha256(artifact)});
    }
    return {spdx:'LicenseRef-s6-overlay-distribution',text,archiveSha256:sha256(bytes),upstreamArchives:archives,encodings:sourceEncodings};
  }
  // Node/Rust base aggregates and s6 need reviewed exact-archive evidence, not a
  // guessed licence label or an upstream text fetched from an unpinned branch.
  throw new Error('Base/overlay licence evidence requires reviewed pinned archives');
}

async function standardTexts() {
  return once('pinned-spdx',async()=>{
    const lock=JSON.parse(readFileSync(join(root,'docker/licenses/spdx-lock.json')));
    const bytes=await get(`https://codeload.github.com/spdx/license-list-data/tar.gz/${lock.commit}`);
    if(sha256(bytes)!==lock.archiveSha256)throw new Error('SPDX archive checksum mismatch');
    const result=new Map();
    for(const [id,pin] of Object.entries(lock.files)) {
      const raw=extract(bytes,[`license-list-data-${lock.commit}/${pin.path}`]);
      if(sha256(raw)!==pin.sha256)throw new Error('SPDX file checksum mismatch');
      const data=JSON.parse(raw);if(data.licenseId!==id)throw new Error('SPDX identity mismatch');
      const text=data.licenseText.replace(/\r\n?/g,'\n').split('\n').filter(l=>!/^Copyright[^\n]*<(?:year|owner|copyright holders)>/i.test(l)).join('\n').trim()+'\n';
      if(/<year>|<owner>|<copyright holders>/i.test(text)||sha256(text)!==pin.textSha256)throw new Error('SPDX text checksum mismatch');
      result.set(id,{text,encodings:encodings(bytes).filter(e=>e.source.endsWith('/'+pin.path))});
    }
    return result;
  });
}
async function standardNpm(row,manifest,meta,spdx,bytes) {
  const policy=approved[row.id];
  if(policy.method!=='spdx-standard'||spdx!==policy.declaration)throw new Error('Unapproved standard licence');
  const declared=manifest.author??meta.author;
  const author=(typeof declared==='string'?declared:(declared?.name||declared?.email||declared?.url))?.trim()||'none stated';
  if(/[\r\n]/.test(author))throw new Error('Invalid author metadata');
  const standard=(await standardTexts()).get(spdx),text=standard.text;
  return {spdx,method:'spdx-standard',declaredLicense:spdx,archiveSha256:sha256(bytes),authorSha256:sha256(author),standardTextSha256:sha256(text),encodings:[...encodings(bytes),...standard.encodings],
    text:bundle([{source:'Package declaration',text:standardStatement(row,spdx,author)},{source:`SPDX standard ${spdx}`,text}])};
}
function shippedCopyrights(bytes,names) {
  const parts=[];
  for(const path of names.filter(n=>/\.(?:rs|toml|md|txt|c|h)$/.test(n))) {
    const text=extract(bytes,[path]);
    for(const paragraph of text.split(/\n[ \t]*\n/))if(/copyright/i.test(paragraph))parts.push({source:path+' (shipped copyright statement)',text:paragraph});
  }
  return parts;
}
function electedCrate(row,spdx,bytes,names) {
  const policy=approved[row.id];
  if(policy.method!=='elected'||spdx!==policy.declaration)throw new Error('Unapproved licence election');
  const apache=splitTexts(commonLicences().text).find(p=>p.source.endsWith('/Apache-2.0'));
  if(!apache)throw new Error('Missing common Apache licence');
  const copyright=shippedCopyrights(bytes,names);
  return {spdx:'Apache-2.0',method:'elected',declaredLicense:spdx,archiveSha256:sha256(bytes),commonTextSha256:sha256(apache.text),encodings:[...encodings(bytes),...commonLicences().encodings],
    text:bundle([{source:'Package licence election',text:`The crate ${row.name}=${row.version} declares ${spdx}.\nApache-2.0 is elected from that declaration. Its full text is the shared common-licence block.\n`},...copyright,apache])};
}
