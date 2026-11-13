import {readFileSync,writeFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
export const hashFile=path=>createHash('sha256').update(readFileSync(path)).digest('hex');
export function lockedPackages(lock) {
  const packages=new Set();
  const blocks=lock.split(/^\[\[package\]\]\s*$/m).slice(1);
  if(!blocks.length)throw new Error('Empty Cargo lock inventory');
  for(const block of blocks) {
    const names=[...block.matchAll(/^name = "([A-Za-z0-9_-]+)"$/gm)];
    const versions=[...block.matchAll(/^version = "([^"\s]+)"$/gm)];
    if(names.length!==1||versions.length!==1)throw new Error('Unsupported Cargo lock package');
    const key=names[0][1]+'@'+versions[0][1];
    if(packages.has(key))throw new Error('Duplicate Cargo lock identity: '+key);
    packages.add(key);
  }
  return packages;
}
export function classifyGraph(tree,lock) {
  const locked=lockedPackages(lock),sets={normal:new Set(),build:new Set()};
  const ancestry=[],buildSections=new Set();let roots=0;
  for(const line of tree.trimEnd().split('\n')) {
    const section=line.match(/^((?:\|   |    )*)\[build-dependencies\]$/);
    if(section) {
      const depth=section[1].length/4;
      if(!ancestry[depth])throw new Error('Build section without a parent');
      ancestry.length=depth+1;
      for(const d of buildSections)if(d>depth)buildSections.delete(d);
      buildSections.add(depth);continue;
    }
    const branch=line.match(/^((?:\|   |    )*)(?:\|-- |`-- )(.*)$/);
    const depth=branch?branch[1].length/4+1:0;
    const text=branch?branch[2]:line;
    const row=text.match(/^([A-Za-z0-9_-]+) v([^\s]+)(?: \(.*\))?$/);
    if(!row||text.includes('(*)')||depth>ancestry.length)throw new Error('Unsupported Cargo tree row: '+line);
    if(depth===0&&roots++)throw new Error('Multiple Cargo tree roots');
    const key=row[1]+'@'+row[2];
    if(!locked.has(key))throw new Error('Cargo tree package missing from lock: '+key);
    for(const d of buildSections)if(d>=depth)buildSections.delete(d);
    const kind=depth>0&&(ancestry[depth-1]==='build'||buildSections.has(depth-1))?'build':'normal';
    ancestry.length=depth;ancestry.push(kind);sets[kind].add(key);
  }
  if(!roots)throw new Error('Empty Cargo tree');
  const present=new Set([...sets.normal,...sets.build]);
  return {normal:[...sets.normal].sort(),build:[...sets.build].sort(),lockedOnly:[...locked].filter(key=>!present.has(key)).sort()};
}
export function createRecord({pinsPath,inputPath,recordPath,env=process.env,nodeVersions=process.versions}) {
  const dockerfile=readFileSync(resolve(inputPath,'Dockerfile'),'utf8');
  const defaults=Object.fromEntries([...dockerfile.matchAll(/^ARG ([A-Z][A-Z0-9_]*)=(.*)$/gm)].map(m=>[m[1],m[2].replace(/^"|"$/g,'')]));
  const pin=name=>env[name]??defaults[name];
  const arch=pin('TARGETARCH');if(!['amd64','arm64'].includes(arch))throw new Error('Unknown architecture');
  const bases=JSON.parse(readFileSync(resolve(pinsPath,'base-images.json'),'utf8'));
  const baseImages=Object.fromEntries(['node','rust'].map(name=>{
    const declared=pin(name.toUpperCase()+'_IMAGE'),row=bases[name];
    if(declared!==row.image||row.digest!==declared.split("@")[1]||!/^sha256:[a-f0-9]{64}$/.test(row.children[arch]??""))throw new Error('Base platform digest mismatch');
    return [name,{image:declared,indexDigest:row.digest,childDigest:row.children[arch]}];
  }));
  const revision=pin('GIT_SHA');if(!/^[a-f0-9]{40}$/.test(revision??''))throw new Error('Build record requires exact GIT_SHA');
  const packageJson=JSON.parse(readFileSync(resolve(inputPath,'web-package.json'),'utf8'));
  const frontend=dockerfile.match(/^# syntax=(\S+)/)?.[1];if(!/@sha256:[a-f0-9]{64}$/.test(frontend??''))throw new Error('Frontend not pinned');
  return {schemaVersion:1,product:{version:packageJson.version,revision},architecture:arch,baseImages,
    debian:{snapshot:pin('DEBIAN_SNAPSHOT'),securitySnapshot:pin('DEBIAN_SECURITY_SNAPSHOT')},
    directPins:Object.fromEntries(Object.entries({nginx:'NGINX_VERSION',postgresMeta:'POSTGRES_META_VERSION',postgresServer:'POSTGRES_SERVER_VERSION',pg15Helper:'PG15_HELPER_VERSION',tzdata:'TZDATA_VERSION',openssl:'OPENSSL_VERSION',caCertificates:'CA_CERTIFICATES_VERSION',s6Overlay:'S6_OVERLAY_VERSION'}).map(([key,value])=>[key,pin(value)])),
    integrity:{frontend,nginxSigningKeySha256:pin('NGINX_SIGNING_KEY_SHA256'),nginxDebSha256:pin('NGINX_DEB_'+arch.toUpperCase()+'_SHA256'),pg15ServerSha256:pin('PG15_SERVER_'+arch.toUpperCase()+'_SHA256'),pg15ClientSha256:pin('PG15_CLIENT_'+arch.toUpperCase()+'_SHA256'),s6NoarchSha256:pin('S6_NOARCH_SHA256'),s6ArchSha256:pin('S6_'+arch.toUpperCase()+'_SHA256')},
    locks:{yarnSha256:hashFile(resolve(inputPath,'yarn.lock')),cargoSha256:hashFile(resolve(inputPath,'Cargo.lock')),turboSha256:hashFile(resolve(inputPath,'package-lock.json'))},
    packageManager:JSON.parse(readFileSync(resolve(inputPath,'package.json'),'utf8')).packageManager,nodeVersions,
    inventories:Object.fromEntries([['runtime','runtime'],['apiBuilder','api-builder'],['keyring','keyring']].map(([key,name])=>[key,{sha256:hashFile(resolve(recordPath,name+'.tsv'))}])),
    crateGraph:{kind:'build-graph',target:arch==='arm64'?'aarch64-unknown-linux-gnu':'x86_64-unknown-linux-gnu',treeSha256:hashFile(resolve(recordPath,'cargo-tree.txt')),...classifyGraph(readFileSync(resolve(recordPath,'cargo-tree.txt'),'utf8'),readFileSync(resolve(inputPath,'Cargo.lock'),'utf8'))}};
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href) {
  const record=createRecord({pinsPath:process.argv[2],inputPath:process.argv[3],recordPath:process.argv[4]});
  writeFileSync(process.argv[5],JSON.stringify(record,null,2)+'\n');
}
