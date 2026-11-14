import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync,mkdirSync,writeFileSync,readFileSync,rmSync,chmodSync,symlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {resolve,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {classifyGraph,hashFile,createRecord} from './build-inputs.mjs';
import {auditGraph} from './audit-graph.mjs';
import {checkLocks} from './check-locks.mjs';
const here=dirname(fileURLToPath(import.meta.url)),source=resolve(here,'../..');
function fixture(fn){const dir=mkdtempSync(resolve(tmpdir(),'nwui-pins-'));try{return fn(dir)}finally{rmSync(dir,{recursive:true,force:true})}}
const run=(script,args=[],env={})=>spawnSync('sh',[resolve(here,script),...args],{encoding:'utf8',env:{...process.env,...env}});
const executable=(file,text)=>{writeFileSync(file,'#!/bin/sh\n'+text+'\n');chmodSync(file,0o755)};
test('snapshot-only signed sources replace live mirrors; invalid input changes nothing',()=>fixture(root=>{
 mkdirSync(resolve(root,'etc/apt/sources.list.d'),{recursive:true});writeFileSync(resolve(root,'etc/apt/sources.list.d/debian.sources'),'URIs: http://deb.debian.org/debian\n');
 let p=run('configure-apt.sh',['bad','20261024T174259Z',root]);assert.notEqual(p.status,0);assert.match(readFileSync(resolve(root,'etc/apt/sources.list.d/debian.sources'),'utf8'),/deb.debian.org/);
 p=run('configure-apt.sh',['20261024T142624Z','20261024T174259Z',root]);assert.equal(p.status,0,p.stderr);
 const sources=readFileSync(resolve(root,'etc/apt/sources.list.d/nwui-snapshot.sources'),'utf8');assert.match(sources,/bookworm bookworm-updates/);assert.match(sources,/bookworm-security/);assert.equal((sources.match(/Check-Valid-Until: no/g)||[]).length,2);assert.equal((sources.match(/Signed-By:/g)||[]).length,2);
 const config=readFileSync(resolve(root,'etc/apt/apt.conf.d/80nwui-snapshot'),'utf8');assert.doesNotMatch(config,/Valid-Until|trusted|AllowInsecure/);assert.match(config,/Retries "5"/);
}));
test('origins come from base equality or exact transaction URI, including epoch; ambiguity fails closed',()=>fixture(root=>{
 const bin=resolve(root,'bin');mkdirSync(bin);
 executable(resolve(bin,'dpkg-query'),`cat '${root}/rows'`);executable(resolve(bin,'dpkg'),'echo arm64');executable(resolve(bin,'apt-cache'),'echo "policy proof"');
 writeFileSync(resolve(root,'rows'),'installed\tbase:arm64\tarm64\t1\tbase-src\t1\ninstalled\tdeb\tall\t2:1.2+3\tdeb-src\t2:1.2+3\ninstalled\tssl\tarm64\t3\tssl-src\t3\ninstalled\tnginx\tarm64\t4\tnginx\t4\nconfig-files\tremoved\tarm64\t1\tremoved\t1\n');
 writeFileSync(resolve(root,'base'),'base\tarm64\t1\tbase-src\t1\n');
 const tx="'http://snapshot.debian.org/archive/debian/20261024T142624Z/pool/deb.deb' deb_2%3a1.2+3_all.deb 1 SHA256:abc\n'http://snapshot.debian.org/archive/debian-security/20261024T174259Z/pool/ssl.deb' ssl_3_arm64.deb 1 SHA256:abc\n'https://nginx.org/packages/debian/pool/nginx.deb' nginx_4_arm64.deb 1 SHA256:abc\n";
 writeFileSync(resolve(root,'tx'),tx);
 const env={PATH:bin+':'+process.env.PATH};let p=run('inventory.sh',['runtime',root+'/actual',root+'/base',root+'/tx'],env);assert.equal(p.status,0,p.stderr);
 const rows=readFileSync(root+'/actual','utf8');assert.match(rows,/base-src\t1\tbase/);assert.match(rows,/deb-src\t2:1.2\+3\tdebian/);assert.match(rows,/ssl-src\t3\tdebian-security/);assert.match(rows,/nginx\t4\tnginx.org/);assert.doesNotMatch(rows,/removed/);
 writeFileSync(root+'/tx',tx+"'http://snapshot.debian.org/archive/debian-security/20261024T174259Z/pool/deb.deb' deb_1.2+3_all.deb 1 SHA256:abc\n");p=run('inventory.sh',['runtime',root+'/actual',root+'/base',root+'/tx'],env);assert.notEqual(p.status,0);assert.match(p.stderr,/고정한 입력/);
}));
test('normal inventory check cannot be disabled and rejects drift/missing inventory',()=>fixture(root=>{
 const bin=root+'/bin';mkdirSync(bin);executable(bin+'/dpkg','echo arm64');writeFileSync(root+'/actual','pkg\tarm64\t2\tsrc\t2\tdebian\n');writeFileSync(root+'/expected','pkg\tarm64\t1\tsrc\t1\tdebian\n');
 const env={PATH:bin+':'+process.env.PATH,SKIP_INVENTORY:'true'};assert.notEqual(run('verify-inventory.sh',['runtime',root+'/actual',root+'/expected'],env).status,0);assert.notEqual(run('verify-inventory.sh',['runtime',root+'/actual',root+'/missing'],env).status,0);
 writeFileSync(root+'/expected',readFileSync(root+'/actual'));assert.equal(run('verify-inventory.sh',['runtime',root+'/actual',root+'/expected'],env).status,0);
}));
function noAptFixture(fn){return fixture(root=>{
 const bin=root+'/bin';mkdirSync(bin);
 for(const tool of ['awk','mktemp','rm']){
  const found=spawnSync('sh',['-c','command -v '+tool],{encoding:'utf8'});assert.equal(found.status,0);
  symlinkSync(found.stdout.trim(),bin+'/'+tool);
 }
 // Use a private filesystem and PATH; real-image calibration runs the unchanged guard.
 executable(bin+'/dpkg-query',`while IFS= read -r line; do printf '%s\\n' "$line"; done < '${root}/rows'`);
 writeFileSync(root+'/rows','dpkg\tinstalled\n');
 const script=readFileSync(here+'/verify-no-apt.sh','utf8').replace(/(^|[\s"])\/(usr|lib64|lib|etc)(?=\/|[\s"])/gm,(_,prefix,path)=>prefix+root+'/'+path);
 writeFileSync(root+'/guard.sh',script);
 const check=()=>spawnSync('/bin/sh',[root+'/guard.sh'],{encoding:'utf8',env:{...process.env,PATH:bin}});
 fn({root,bin,check});
});}
test('runtime no-apt guard accepts retained packages and inert apt configuration',()=>noAptFixture(({root,check})=>{
 mkdirSync(root+'/etc/apt/apt.conf.d',{recursive:true});writeFileSync(root+'/etc/apt/apt.conf.d/70debconf','apt configuration');
 writeFileSync(root+'/rows','dpkg\tinstalled\ndebconf\tinstalled\napt\tnot-installed\n');
 const p=check();assert.equal(p.status,0,p.stderr);
}));
test('runtime no-apt guard rejects package state and failed enumeration independently of payload',()=>noAptFixture(({root,bin,check})=>{
 for(const name of ['apt','apt:arm64','libapt-pkg7.0:amd64','libapt-pkg8.0'])for(const status of ['installed','unpacked','config-files','half-configured']){
  writeFileSync(root+'/rows',name+'\t'+status+'\n');const p=check();assert.notEqual(p.status,0);assert.match(p.stderr,/package state/);
 }
 writeFileSync(root+'/rows','');let empty=check();assert.notEqual(empty.status,0);assert.match(empty.stderr,/empty package state/);
 executable(bin+'/dpkg-query','exit 17');const p=check();assert.notEqual(p.status,0);assert.match(p.stderr,/cannot enumerate/);
}));
test('runtime no-apt guard rejects executables, standard paths and dangling library/helper payload',()=>noAptFixture(({root,bin,check})=>{
 for(const name of ['apt','apt-get','apt-cache','apt-cdrom','apt-config','apt-mark']){
  executable(bin+'/'+name,'exit 0');const p=check();assert.notEqual(p.status,0);assert.match(p.stderr,/executable/);rmSync(bin+'/'+name);
 }
 for(const path of ['/usr/bin/apt','/usr/lib/apt/helper','/usr/lib/dpkg/methods/apt','/etc/cron.daily/apt-compat','/usr/lib/systemd/system/apt-daily.service','/etc/systemd/system/timers.target.wants/apt-daily.timer','/usr/lib/aarch64-linux-gnu/libapt-pkg.so.7.0','/usr/local/lib/libapt-private.so.0.0']){
  const file=root+path;mkdirSync(dirname(file),{recursive:true});symlinkSync('/missing-target',file);const p=check();assert.notEqual(p.status,0,p.stderr);assert.match(p.stderr,/remains/);rmSync(file);
  for(const directory of ['usr','etc'])rmSync(root+'/'+directory,{recursive:true,force:true});
  const clean=check();assert.equal(clean.status,0,clean.stderr);
 }
}));
test('runtime purge retains inventory proof and both no-apt build guards',()=>{
 const dockerfile=readFileSync(source+'/Dockerfile','utf8');
 const origin=dockerfile.indexOf('/opt/nwui-pins/inventory.sh runtime '),purge=dockerfile.indexOf('dpkg --purge apt libapt-pkg7.0'),reconcile=dockerfile.indexOf('cmp /tmp/nwui-post-apt-installed.tsv /tmp/nwui-post-apt-expected.tsv'),guard=dockerfile.indexOf('/opt/nwui-pins/verify-no-apt.sh');
 assert.ok(origin>=0&&purge>origin&&reconcile>purge&&guard>reconcile);
 assert.match(dockerfile,/\/bin\/bash -o pipefail \/opt\/nwui-pins\/base-inventory\.sh/);
 assert.match(dockerfile,/\/verify-pins\/verify-no-apt\.sh/);
 for(const arch of ['amd64','arm64']){
  const rows=readFileSync(here+'/inventory/runtime-'+arch+'.tsv','utf8').trim().split('\n');
  assert.equal(rows.length,130);assert.ok(rows.every(row=>!['apt','libapt-pkg7.0'].includes(row.split('\t')[0])));
 }
});
const graphLock=(...names)=>'version = 4\n'+names.map(name=>'[[package]]\nname = "'+name+'"\nversion = "1.0.0"\n').join('\n');
test('actual tree classifies build ancestry and shared crates; metadata-only rows stay locked-only',()=>{
 const tree=`root v1.0.0 (/private/checkout)
|-- normal v1.0.0
|   |-- shared v1.0.0
|   [build-dependencies]
|   \`-- build v1.0.0
|       \`-- shared v1.0.0
\`-- other v1.0.0
[build-dependencies]
\`-- build v1.0.0
    \`-- shared v1.0.0
`;
 assert.deepEqual(classifyGraph(tree,graphLock('root','normal','shared','build','other','dev','feature-extra')),{normal:['normal@1.0.0','other@1.0.0','root@1.0.0','shared@1.0.0'],build:['build@1.0.0','shared@1.0.0'],lockedOnly:['dev@1.0.0','feature-extra@1.0.0']});
});
test('tree classification fails closed on unknown, deduplicated, missing-lock or ambiguous rows',()=>{
 const lock=graphLock('root','normal');
 for(const tree of ['root v1.0.0\nnormal v1.0.0\n','root v1.0.0\n\`-- normal v1.0.0 (*)\n','root v1.0.0\n[dev-dependencies]\n\`-- normal v1.0.0\n','root v1.0.0\n    \`-- normal v1.0.0\n','root v1.0.0\n\`-- absent v1.0.0\n',''])assert.throws(()=>classifyGraph(tree,lock));
 assert.throws(()=>classifyGraph('root v1.0.0\n',graphLock('root','root')),/Duplicate/);
});
test('graph audit proves complete lock partition and rejects missing, overlap and membership drift',()=>{
 const graph={normal:['root@1.0.0','normal@1.0.0'],build:['normal@1.0.0'],lockedOnly:['dev@1.0.0']};
 const flat='root v1.0.0 (/path)\nnormal v1.0.0\nnormal v1.0.0 (*)\n',lock=graphLock('root','normal','dev');
 assert.equal(auditGraph(graph,flat,lock).partitionEqualsLock,true);
 assert.throws(()=>auditGraph({...graph,lockedOnly:[]},flat,lock),/complete Cargo.lock/);
 assert.throws(()=>auditGraph({...graph,lockedOnly:[...graph.lockedOnly,'root@1.0.0']},flat,lock),/overlaps/);
 assert.throws(()=>auditGraph({...graph,normal:graph.normal.slice(1)},flat,lock),/differs from actual/);
 assert.throws(()=>auditGraph({...graph,normal:[...graph.normal,'normal@1.0.0']},flat,lock),/Invalid graph set/);
});
test('actual bootstrap includes both Linux SRI pins, matches workspace Turbo and hashed Yarn',()=>{assert.equal(checkLocks(source),'2.10.9')});
test('download integrity rejects wrong bytes, absent hash, and malformed hash before use',()=>fixture(root=>{
 writeFileSync(root+'/file','checked bytes');const hash=hashFile(root+'/file');assert.equal(run('verify-file.sh',['runtime',root+'/file',hash]).status,0);writeFileSync(root+'/file','tampered bytes');
 for(const expected of [hash,'','not-a-hash']) {const result=run('verify-file.sh',['runtime',root+'/file',expected]);assert.notEqual(result.status,0);assert.match(result.stderr,/고정한 입력/)}
}));
test('reproduction ignores only origin and row order; version/source/architecture/add/remove are deltas',async()=>{
 const {differences,normalizedInventory}=await import('./reproduction.mjs');
 const header='binary\tarch\tversion\tsource\tsourceVersion';
 const before=header+'\na\tarm64\t1\tsrc\t1\nb\tall\t2\tsrc-b\t2\n';
 const after=header+'\torigin\nb\tall\t2\tsrc-b\t2\tbase\na\tarm64\t1\tsrc\t1\tdebian\n';
 assert.deepEqual(differences(before,after),[]);assert.equal(differences(before,after.replace('a\tarm64\t1\tsrc\t1','a\tarm64\t2\tsrc\t2')).length,1);
 assert.equal(differences(before,header+'\na\tamd64\t1\tsrc\t1\n').length,3);assert.throws(()=>normalizedInventory(before+'a\tarm64\t1\tsrc\t1\n'),/Duplicate/);
});
test('build record records actual hashes/platforms and omits raw Cargo source paths',()=>fixture(root=>{
 const input=root+'/input',records=root+'/records';mkdirSync(input);mkdirSync(records);
 for(const [from,to] of [['Dockerfile','Dockerfile'],['package.json','package.json'],['apps/web/package.json','web-package.json'],['yarn.lock','yarn.lock'],['apps/api/Cargo.lock','Cargo.lock'],['docker/turbo/package-lock.json','package-lock.json']])writeFileSync(input+'/'+to,readFileSync(source+'/'+from));
 for(const stage of ['runtime','api-builder','keyring'])writeFileSync(records+'/'+stage+'.tsv','binary\tarch\tversion\tsource\tsourceVersion\torigin\n');
 writeFileSync(records+'/cargo-tree.txt','nwui-api v0.1.0 (/private/secret-checkout)\n');writeFileSync(records+'/cargo-metadata.json',JSON.stringify({packages:[{id:'root',name:'root',version:'1',source:null,manifest_path:'/private/secret-checkout/Cargo.toml'}],resolve:{root:'root',nodes:[{id:'root',deps:[]}]}}));
 const env={TARGETARCH:'arm64',GIT_SHA:'a'.repeat(40)},record=createRecord({pinsPath:here,inputPath:input,recordPath:records,env});
 const schema=JSON.parse(readFileSync(here+'/build-inputs.schema.json','utf8'));assert.deepEqual(Object.keys(record).sort(),schema.required.sort());
 assert.equal(record.baseImages.node.childDigest,JSON.parse(readFileSync(here+'/base-images.json','utf8')).node.children.arm64);assert.equal(record.locks.cargoSha256,hashFile(input+'/Cargo.lock'));assert.equal(record.nodeVersions.openssl,process.versions.openssl);assert.equal(record.crateGraph.kind,'build-graph');assert.doesNotMatch(JSON.stringify(record),/secret-checkout|manifest_path/);
 assert.throws(()=>createRecord({pinsPath:here,inputPath:input,recordPath:records,env:{TARGETARCH:'arm64'}}),/GIT_SHA/);
 assert.throws(()=>createRecord({pinsPath:here,inputPath:input,recordPath:records,env:{...env,NODE_IMAGE:'node:latest'}}),/digest mismatch/);
}));
test('OS-W1 Trixie snapshot sources are signed, limited to three approved suites, and reject unknown suite before mutation',()=>fixture(root=>{
 mkdirSync(resolve(root,'etc/apt/sources.list.d'),{recursive:true});
 const file=resolve(root,'etc/apt/sources.list.d/debian.sources');writeFileSync(file,'original mirror');
 let p=run('configure-apt.sh',['20261024T142624Z','20261024T174259Z',root,'sid']);
 assert.notEqual(p.status,0);assert.equal(readFileSync(file,'utf8'),'original mirror');
 p=run('configure-apt.sh',['20261024T142624Z','20261024T174259Z',root,'trixie']);assert.equal(p.status,0,p.stderr);
 const sources=readFileSync(resolve(root,'etc/apt/sources.list.d/nwui-snapshot.sources'),'utf8');
 assert.match(sources,/Suites: trixie trixie-updates\n/);assert.match(sources,/Suites: trixie-security\n/);
 assert.equal((sources.match(/Signed-By: \/usr\/share\/keyrings\/debian-archive-keyring.gpg/g)||[]).length,2);
 assert.doesNotMatch(sources,/bookworm|pgdg|deb.debian.org|trusted=yes|AllowInsecure/);
}));

test('Trixie Dockerfile pins match signed provenance and keep helper origins narrow',()=>{
 const dockerfile=readFileSync(source+'/Dockerfile','utf8'),proof=JSON.parse(readFileSync(here+'/trixie-provenance.json','utf8'));
 const args=Object.fromEntries([...dockerfile.matchAll(/^ARG ([A-Z][A-Z0-9_]*)=(.*)$/gm)].map(m=>[m[1],m[2]]));
 assert.equal(args.NODE_IMAGE,proof.baseImages.node.image);assert.equal(args.RUST_IMAGE,proof.baseImages.rust.image);
 for(const [name,version] of Object.entries(proof.selectedDebianVersions))assert.equal(args[name],version);
 assert.equal(args.PG15_HELPER_VERSION,proof.pgdgPolicy.version);
 for(const arch of ['amd64','arm64'])for(const [pkg,key] of [['nginx','NGINX_DEB'],['postgresql-15','PG15_SERVER'],['postgresql-client-15','PG15_CLIENT']])assert.equal(args[key+'_'+arch.toUpperCase()+'_SHA256'],proof.approvedExternalPackages.find(p=>p.Package===pkg&&p.Architecture===arch).SHA256);
 assert.equal((dockerfile.match(/configure-apt.sh .* \/ trixie/g)||[]).length,3);
 assert.match(dockerfile,/zlib1g=\$ZLIB_VERSION/);
 assert.match(dockerfile,/libssl3t64=\$OPENSSL_VERSION/);assert.doesNotMatch(dockerfile,/libssl3=|packages\/debian bookworm/);
 assert.match(dockerfile,/postgresql-17=\$POSTGRES_SERVER_VERSION/);
 assert.equal(readFileSync(here+'/inventory-row.schema.json','utf8').includes('"pgdg"'),true);
});
test('only the two reviewed PGDG15 helper transactions receive pgdg origin',()=>fixture(root=>{
 const bin=root+'/bin';mkdirSync(bin);executable(bin+'/dpkg','echo arm64');executable(bin+'/apt-cache','echo proof');executable(bin+'/dpkg-query',`cat '${root}/rows'`);
 writeFileSync(root+'/base','');const env={PATH:bin+':'+process.env.PATH};
 for(const pkg of ['postgresql-15','postgresql-client-15','libpq5']){
  writeFileSync(root+'/rows',`installed\t${pkg}\tarm64\t15.20-1.pgdg13+1\tpostgresql-15\t15.20-1.pgdg13+1\n`);
  writeFileSync(root+'/tx',`'https://apt.postgresql.org/pub/repos/apt/pool/main/p/postgresql-15/${pkg}_15.20-1.pgdg13+1_arm64.deb' ${pkg}_15.20-1.pgdg13+1_arm64.deb 1 SHA256:abc\n`);
  const r=run('inventory.sh',['runtime',root+'/actual',root+'/base',root+'/tx'],env);
  if(pkg==='libpq5')assert.notEqual(r.status,0);else{assert.equal(r.status,0,r.stderr);assert.match(readFileSync(root+'/actual','utf8'),/\tpgdg\n/);}
 }
}));

test('PG15 artifact downloader checks both hashes and rejects unsupported inputs',()=>fixture(root=>{
 const bin=root+'/bin';mkdirSync(bin);writeFileSync(root+'/bytes','reviewed artifact');
 executable(bin+'/curl',`while [ "$#" -gt 0 ]; do if [ "$1" = -o ]; then cp '${root}/bytes' "$2"; exit; fi; shift; done; exit 9`);
 const env={PATH:bin+':'+process.env.PATH},hash=hashFile(root+'/bytes');
 let r=run('fetch-pg15.sh',['arm64','15.20-1.pgdg13+1',hash,hash,root+'/ok'],env);assert.equal(r.status,0,r.stderr);
 const tx=readFileSync(root+'/ok/transaction.txt','utf8');assert.equal(tx.trim().split('\n').length,2);assert.match(tx,/postgresql-client-15_15.20-1.pgdg13\+1_arm64.deb/);
 assert.notEqual(run('fetch-pg15.sh',['arm64','15.20-1.pgdg13+1','a'.repeat(64),hash,root+'/bad'],env).status,0);
 for(const version of ['15.19-1.pgdg13+2','16.0']){const refusal=run('fetch-pg15.sh',['arm64',version,hash,hash,root+'/unsupported'],env);assert.notEqual(refusal.status,0);assert.match(refusal.stderr,/pg15\/unsupported-artifact/);}
}));
