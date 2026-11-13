// Offline checker: upstream texts occur only in THIRD_PARTY_NOTICES.md.
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, readdirSync, realpathSync, existsSync } from 'node:fs';
import { dirname, resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
export const sha256 = value => createHash('sha256').update(value).digest('hex');
const order = (a,b) => a < b ? -1 : a > b ? 1 : 0;
const read = (base,path) => readFileSync(join(base,path),'utf8');
export function inputs(base=root) {
  const paths=['docker/licenses/spdx-lock.json','package.json','yarn.lock','docker/turbo/package-lock.json','apps/api/Cargo.lock',
    ...readdirSync(join(base,'docker/pins/inventory')).filter(p=>p.endsWith('.tsv')).map(p=>'docker/pins/inventory/'+p)];
  const hashes=Object.fromEntries(paths.sort(order).map(p=>[p,sha256(read(base,p))]));
  // Other Dockerfile edits must not invalidate the report; all pinned ARG inputs do.
  hashes['Dockerfile:ARG']=sha256(read(base,'Dockerfile').split('\n').filter(l=>/^ARG [A-Z][A-Z0-9_]*=/.test(l)).join('\n')+'\n');
  return hashes;
}
export function inventory(base=root) {
  const rows=[];
  const yarn=read(base,'yarn.lock');
  for(const m of yarn.matchAll(/^  resolution: "([^"\n]+)"$/gm)) {
    let resolution=m[1];
    if(resolution.includes('@patch:')) {
      const patch=resolution.match(/^.+@patch:([^#]+)#optional!builtin<compat\/[a-z-]+>::version=[^&]+&hash=[a-f0-9]+$/);
      if(!patch)throw new Error('Unsupported Yarn patch source');
      resolution=decodeURIComponent(patch[1]);
    }
    if(resolution.includes('@workspace:'))continue;
    const pkg=resolution.match(/^(.+)@npm:([^:]+)$/);
    if(!pkg)throw new Error('Unsupported Yarn source');
    rows.push({id:`npm:${pkg[1]}=${pkg[2]}`,name:pkg[1],version:pkg[2],ecosystem:'npm',arch:'all',coverage:'source/build/runtime-superset'});
  }
  const manager=JSON.parse(read(base,'package.json')).packageManager;
  const yarnVersion=manager?.match(/^yarn@([^+]+)\+sha224\.[a-f0-9]+$/)?.[1];
  if(!yarnVersion)throw new Error('Missing pinned Yarn tool');
  rows.push({id:`npm:@yarnpkg/cli-dist=${yarnVersion}`,name:'@yarnpkg/cli-dist',version:yarnVersion,ecosystem:'npm',arch:'all',coverage:'build'});
  const npm=JSON.parse(read(base,'docker/turbo/package-lock.json'));
  for(const [path,v] of Object.entries(npm.packages)) if(path && !v.link) {
    const name=path.split('node_modules/').at(-1);
    rows.push({id:`npm:${name}=${v.version}`,name,version:v.version,ecosystem:'npm',arch:'all',coverage:'source/build/runtime-superset'});
  }
  for(const block of read(base,'apps/api/Cargo.lock').split('[[package]]').slice(1)) {
    const field=k=>block.match(new RegExp('^'+k+' = "([^"]+)"','m'))?.[1];
    if(field('source')) {
      if(!field('source').startsWith('registry+')||!field('checksum')) throw new Error('Unsupported Cargo source');
      rows.push({id:`cargo:${field('name')}=${field('version')}`,name:field('name'),version:field('version'),ecosystem:'cargo',arch:'all',coverage:'source/build/runtime-superset',lockedChecksum:field('checksum')});
    }
  }
  for(const file of readdirSync(join(base,'docker/pins/inventory')).filter(p=>p.endsWith('.tsv')).sort(order)) {
    for(const line of read(base,'docker/pins/inventory/'+file).trimEnd().split('\n').slice(1)) {
      const [name,arch,version,source,sourceVersion,origin]=line.split('\t');
      if(!origin)throw new Error('Invalid Debian inventory');
      rows.push({id:`deb:${file}:${name}=${version}`,name,arch,version,source,sourceVersion,ecosystem:'deb',coverage:file.startsWith('runtime-')?'runtime':'build'});
    }
  }
  const pins=Object.fromEntries([...read(base,'Dockerfile').matchAll(/^ARG ([A-Z][A-Z0-9_]*)=(.*)$/gm)].map(m=>[m[1],m[2]]));
  for(const key of ['NODE_IMAGE','RUST_IMAGE','S6_OVERLAY_VERSION']) {
    if(!pins[key])throw new Error('Missing image component pin');
    rows.push({id:`component:${key}=${pins[key]}`,name:key,version:pins[key],ecosystem:'component',arch:'amd64/arm64',coverage:key==='RUST_IMAGE'?'build':'build/runtime'});
  }
  rows.push({id:`component:DEBIAN_COMMON_LICENSES=${pins.NODE_IMAGE}`,name:'DEBIAN_COMMON_LICENSES',version:pins.NODE_IMAGE,ecosystem:'component',arch:'amd64/arm64',coverage:'build/runtime'});
  return [...new Map(rows.map(r=>[r.id,r])).values()].sort((a,b)=>order(a.id,b.id));
}
export const approved = Object.fromEntries([
  ['cargo:r-efi=5.3.0','elected','MIT OR Apache-2.0 OR LGPL-2.1-or-later','Apache-2.0'],
  ['cargo:r-efi=6.0.0','elected','MIT OR Apache-2.0 OR LGPL-2.1-or-later','Apache-2.0'],
  ['cargo:rustls-platform-verifier-android=0.1.1','elected','MIT OR Apache-2.0','Apache-2.0'],
  ...[['@tybys/wasm-util','0.10.3'],['client-only','0.0.1'],['keyv','4.5.4'],['natural-compare','1.4.0'],['stackback','0.0.2'],['uri-js-replace','1.0.1']].map(([n,v])=>[`npm:${n}=${v}`,'spdx-standard','MIT','MIT']),
  ['npm:@yarnpkg/cli-dist=4.14.1','spdx-standard','BSD-2-Clause','BSD-2-Clause'],
].map(([id,method,declaration,licence])=>[id,{method,declaration,licence}]));
const normal = text => text.replace(/\r\n?/g,'\n').trim()+'\n';
const decodeHtml = text => text.replace(/&(#x[0-9a-f]+|#\d+|amp|lt|gt|quot|apos|nbsp);/gi,(_,key)=>key[0]==='#'?String.fromCodePoint(parseInt(key.slice(key[1].toLowerCase()==='x'?2:1),key[1].toLowerCase()==='x'?16:10)):({amp:'&',lt:'<',gt:'>',quot:'"',apos:"'",nbsp:' '})[key.toLowerCase()]);
function htmlMetadata(text) {
  return decodeHtml(text.replace(/<a\b[^>]*href=(["'])(.*?)\1[^>]*>([\s\S]*?)<\/a>/gi,(_,quote,url,label)=>label+' ('+url+')').replace(/<![^>]*>/g,'').replace(/<\/?(?:p|div|h[1-6]|li|details|summary|br|ul|ol)\b[^>]*>/gi,'\n').replace(/<[^>]*>/g,'')).split('\n').map(l=>l.trim()).filter(Boolean).join('\n');
}
export function splitTexts(text) {
  const sections=[], markers=[...text.matchAll(/^File: ([^\n]+)\n\n/gm)];
  if(!markers.length)sections.push({source:'upstream licence',text});
  else {
    if(text.slice(0,markers[0].index).trim())sections.push({source:'upstream licence',text:text.slice(0,markers[0].index)});
    for(let i=0;i<markers.length;i++)sections.push({source:markers[i][1],text:text.slice(markers[i].index+markers[i][0].length,markers[i+1]?.index??text.length)});
  }
  const parts=[];
  for(const section of sections) {
    if(!/\.html(?: |$)/i.test(section.source)||!/^\s*(?:<!DOCTYPE[^>]*>\s*)?<html\b/i.test(section.text)) {parts.push({...section,text:normal(section.text)});continue;}
    let at=0;
    for(const m of section.text.matchAll(/<pre\b[^>]*>([\s\S]*?)<\/pre>/gi)) {
      const before=htmlMetadata(section.text.slice(at,m.index));
      if(before)parts.push({source:section.source,text:normal(before)});
      parts.push({source:section.source,text:normal(decodeHtml(m[1]))});at=m.index+m[0].length;
    }
    const after=htmlMetadata(section.text.slice(at));if(after)parts.push({source:section.source,text:normal(after)});
  }
  return parts.filter(p=>p.text.trim()).filter((p,i,all)=>all.findIndex(x=>x.source===p.source&&x.text===p.text)===i);
}
export const bundle = parts => parts.map(p=>`File: ${p.source}\n\n${normal(p.text)}`).join('\n');
export function licenceReference(row,spdx,text) {
  return row.ecosystem==='deb'||spdx.includes('/')||/^SEE LICENSE IN /i.test(spdx)||/[^A-Za-z0-9.()+ _-]/.test(spdx)?'LicenseRef-'+sha256(text).slice(0,24):spdx;
}
export function prepare(row,result,texts) {
  const parts=splitTexts(result.text);
  const references=parts.map(p=>{const hash=sha256(p.text);texts.set(hash,p.text);return {source:p.source,sha256:hash};});
  const text=bundle(parts),{text:discard,...metadata}=result;
  return {...row,...metadata,spdx:result.method?result.spdx:licenceReference(row,result.spdx,text),textSha256:sha256(text),parts:references};
}
export function readTexts(content) {
  const texts=new Map();
  const pattern=/<!-- nwui-text:([a-f0-9]{64}) -->\n(`{3,})text\n([\s\S]*?)\n\2\n<!-- \/nwui-text:\1 -->\n/g;
  for(const m of content.matchAll(pattern)) {
    const text=m[3]+'\n';
    if(texts.has(m[1])||sha256(text)!==m[1])throw new Error('Duplicate or changed licence text');texts.set(m[1],text);
  }
  return texts;
}
const prefix = hashes => '# Third-party notices\n\n'+
  'Generated from locked source/build dependencies and both architecture image inventories.\n'+
  'Package inventories are conservative supersets, not a list of only runtime software.\n'+
  'Some explicitly identified entries elect Apache-2.0 or use a pinned SPDX standard\n'+
  'text because the declared licence text was not shipped. No copyright line is composed.\n'+
  'The checker verifies evidence and deterministic content; it does not prove legal compliance.\n'+
  'Upstream holders and terms apply; the image is an aggregate. Debian source packages\n'+
  'are available from https://snapshot.debian.org/ at the source versions listed below.\n\n'+
  '## Input SHA-256\n\n'+Object.entries(hashes).map(([p,h])=>`- ${p}: ${h}\n`).join('')+'\n';
export function render(hashes,entries,texts) {
  const applies=new Map();
  for(const e of entries)for(const p of e.parts) {
    if(!texts.has(p.sha256)||sha256(texts.get(p.sha256))!==p.sha256)throw new Error('Missing licence text');
    if(!applies.has(p.sha256))applies.set(p.sha256,new Set());applies.get(p.sha256).add(`${e.id} — ${p.source}`);
  }
  if(applies.size!==texts.size)throw new Error('Unreferenced licence text');
  let output=prefix(hashes);
  const declarations=new Set(entries.filter(e=>e.method==='spdx-standard').map(e=>e.parts.find(p=>p.source==='Package declaration')?.sha256));
  const sequence=[];
  for(const hash of [...texts.keys()].sort(order)) {
    if(declarations.has(hash))continue;
    const before=entries.filter(e=>e.method==='spdx-standard'&&e.standardTextSha256===hash).map(e=>e.parts.find(p=>p.source==='Package declaration')?.sha256);
    sequence.push(...[...new Set(before)].sort(order),hash);
  }
  for(const hash of sequence) {
    const text=texts.get(hash),runs=text.match(/`+/g)??[],fence='`'.repeat(Math.max(3,...runs.map(r=>r.length+1)));
    output+=`## Text ${hash}\n\n<!-- nwui-text:${hash} -->\n${fence}text\n${text.trimEnd()}\n${fence}\n<!-- /nwui-text:${hash} -->\n\nApplies to:\n`+
      [...applies.get(hash)].sort(order).map(p=>`- ${p}\n`).join('')+'\n';
  }
  output+='## Package inventory\n\n';
  for(const e of entries)output+=`### ${e.id}\n\nCoverage: ${e.coverage}; architecture: ${e.arch}; licence: ${e.spdx}.\n`+
    `Evidence: ${e.method??'archive'}; source archive SHA-256: ${e.archiveSha256}.\n`+
    (e.declaredLicense?`Declared licence: ${e.declaredLicense}.\n`:'')+
    (e.source?`Debian source: ${e.source} ${e.sourceVersion}.\n`:'')+
    [...new Set(e.parts.map(p=>p.sha256))].map(h=>`- Text ${h}\n`).join('')+'\n';
  return output;
}
export function standardStatement(row,licence,author) {
  return `The package ${row.name}=${row.version} declares ${licence} and ships no licence text.\nAuthor stated in package metadata: ${author}.\nThe pinned SPDX standard text below is used without its placeholder copyright line.\nNo copyright line has been composed.\n`;
}
export function check(base=root) {
  const e=JSON.parse(read(base,'docker/licenses/evidence.json')),hashes=inputs(base),rows=inventory(base);
  if(e.schemaVersion!==2||e.collectionFailures?.length||e.entries.length!==rows.length)throw new Error('Incomplete licence evidence');
  if(JSON.stringify(e.inputs)!==JSON.stringify(hashes))throw new Error('Licence inputs changed');
  const content=read(base,'THIRD_PARTY_NOTICES.md'),texts=readTexts(content),spdx=JSON.parse(read(base,'docker/licenses/spdx-lock.json'));
  const common=e.entries.find(r=>r.name==='DEBIAN_COMMON_LICENSES');
  const apache=common?.parts.find(p=>p.source.endsWith('/Apache-2.0'))?.sha256;
  for(let i=0;i<rows.length;i++) {
    const entry=e.entries[i],row=rows[i];
    for(const [key,value] of Object.entries(row))if(entry[key]!==value)throw new Error('Licence inventory mismatch');
    if(!/^[a-f0-9]{64}$/.test(entry.archiveSha256)||(row.lockedChecksum&&entry.archiveSha256!==row.lockedChecksum)||!entry.spdx||/unknown|unlicensed|noassertion/i.test(entry.spdx)||/[^A-Za-z0-9.()+ _-]/.test(entry.spdx))throw new Error('Unknown licence evidence');
    if(!Array.isArray(entry.parts)||!entry.parts.length)throw new Error('Missing package licence parts');
    if(!Array.isArray(entry.encodings)||!entry.encodings.length||entry.encodings.some(e=>!['utf-8','latin-1'].includes(e.encoding)||!/^[a-f0-9]{64}$/.test(e.sha256)||!/^[a-f0-9]{64}$/.test(e.archiveSha256)||!Number.isInteger(e.replacementCharacters)||e.replacementCharacters<0))throw new Error('Missing text encoding evidence');
    const parts=entry.parts.map(p=>({source:p.source,text:texts.get(p.sha256)}));
    if(parts.some(p=>!p.text)||sha256(bundle(parts))!==entry.textSha256)throw new Error('Changed package licence bundle');
    const reference=entry.spdx.match(/^LicenseRef-([a-f0-9]+)$/);
    if(reference&&(![24,64].includes(reference[1].length)||reference[1]!==entry.textSha256.slice(0,reference[1].length)))throw new Error('LicenseRef does not match its text');
    const policy=approved[row.id];
    if(policy) {
      if(entry.method!==policy.method||entry.declaredLicense!==policy.declaration||entry.spdx!==policy.licence)throw new Error('Unapproved standard licence use');
      if(policy.method==='elected') {
        if(!apache||entry.commonTextSha256!==apache||!entry.parts.some(p=>p.sha256===apache))throw new Error('Missing elected common licence');
      }else {
        if(entry.standardTextSha256!==spdx.files[policy.licence]?.textSha256||!entry.parts.some(p=>p.sha256===entry.standardTextSha256))throw new Error('Changed standard licence');
        const statement=parts.find(p=>p.source==='Package declaration')?.text;
        const author=statement?.match(/^Author stated in package metadata: (.*)\.$/m)?.[1];
        if(!author||sha256(author)!==entry.authorSha256||statement!==standardStatement(row,policy.licence,author))throw new Error('Changed standard licence declaration');
      }
    }else if(entry.method&&entry.method!=='archive')throw new Error('Unapproved licence substitution');
  }
  if(render(hashes,e.entries,texts)!==content)throw new Error('Notices differ from deterministic regeneration');
  return e.entries.length;
}
export async function collectSnapshot() {
  const {collect,resetCollection}=await import('./collect.mjs');resetCollection();
  const entries=[],texts=new Map(),failures=[],rows=inventory();let next=0;
  // Every invocation starts from upstream; only identical downloads within this
  // invocation are shared by the collector, never saved notice blocks.
  await Promise.all(Array.from({length:3},async()=>{while(next<rows.length) {
    const row=rows[next++];
    try {
      const result=await collect(row);
      if(!result.spdx||/unknown|unlicensed|noassertion/i.test(result.spdx)||!result.text?.trim())throw new Error('Missing licence evidence');
      entries.push(prepare(row,result,texts));
    }catch {failures.push({id:row.id,reason:'Missing or unverified exact licence evidence'});}
    if((entries.length+failures.length)%100===0)console.log(`Collected ${entries.length}; unresolved ${failures.length}`);
  }}));
  entries.sort((a,b)=>order(a.id,b.id));failures.sort((a,b)=>order(a.id,b.id));const hashes=inputs();
  const evidence={schemaVersion:2,inputs:hashes,entries,collectionFailures:failures};
  return {notices:render(hashes,entries,texts),evidence:JSON.stringify(evidence,null,2)+'\n',counts:{components:entries.length,distinctTextBlocks:texts.size,unresolved:failures.length}};
}
export function verifySnapshot(base,receiptPath,snapshot) {
  if(snapshot.counts.unresolved)throw new Error('Incomplete upstream collection');
  const noticesSha256=sha256(snapshot.notices),evidenceSha256=sha256(snapshot.evidence);
  if(read(base,'THIRD_PARTY_NOTICES.md')!==snapshot.notices||read(base,'docker/licenses/evidence.json')!==snapshot.evidence)throw new Error('Upstream regeneration differs');
  const receipt={schemaVersion:1,noticesSha256,evidenceSha256,counts:snapshot.counts,verifiedAt:new Date().toISOString()};
  writeFileSync(receiptPath,JSON.stringify(receipt,null,2)+'\n');return receipt;
}
export async function main(args) {
  const verify=args.length===3&&args[0]==='--verify-upstream'&&args[1]==='--receipt'&&args[2];
  if(!verify&&(args.length!==1||!['--check','--collect'].includes(args[0])))throw new Error('Use --check, --collect, or --verify-upstream --receipt <path>');
  if(verify) {
    check();const snapshot=await collectSnapshot();verifySnapshot(root,args[2],snapshot);
    console.log(`Upstream licence verification passed: ${snapshot.counts.components} components`);return;
  }
  if(args[0]==='--collect') {
    const snapshot=await collectSnapshot();
    writeFileSync(join(root,'THIRD_PARTY_NOTICES.md'),snapshot.notices);
    writeFileSync(join(root,'docker/licenses/evidence.json'),snapshot.evidence);
  }
  console.log(`Licence evidence verified: ${check()} components`);
}
if(process.argv[1]&&existsSync(process.argv[1])&&realpathSync(fileURLToPath(import.meta.url))===realpathSync(process.argv[1]))main(process.argv.slice(2)).catch(()=>{console.error('Licence verification failed; no licence contents printed');process.exitCode=1;});
