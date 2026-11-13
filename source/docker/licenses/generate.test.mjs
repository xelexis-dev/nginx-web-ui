import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync,mkdirSync,writeFileSync,readFileSync,rmSync,copyFileSync,symlinkSync,existsSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {check,inputs,inventory,render,sha256,licenceReference,prepare,readTexts,splitTexts,bundle,standardStatement,verifySnapshot} from './generate.mjs';
const encoding=text=>[{source:'LICENSE',encoding:'utf-8',archiveSha256:sha256('archive'),sha256:sha256(text),replacementCharacters:0}];
function fixture() {
  const base=mkdtempSync(join(tmpdir(),'nwui-licences-'));
  for(const p of ['docker/pins/inventory','docker/turbo','docker/licenses','apps/api'])mkdirSync(join(base,p),{recursive:true});
  const write=(p,v)=>writeFileSync(join(base,p),v);
  write('docker/licenses/spdx-lock.json',JSON.stringify({files:{MIT:{textSha256:sha256('Standard permission text.\n')}}}));
  write('package.json',JSON.stringify({packageManager:'yarn@4.0.0+sha224.abcdef'}));
  write('yarn.lock','"pkg@npm:1":\n  resolution: "pkg@npm:1.0.0"\n');
  write('docker/turbo/package-lock.json',JSON.stringify({packages:{}}));
  write('apps/api/Cargo.lock','version = 4\n');
  write('Dockerfile','ARG NODE_IMAGE=node:test@sha256:locked\nARG RUST_IMAGE=rust:test@sha256:locked\nARG S6_OVERLAY_VERSION=1.2.3\n');
  const text='Copyright upstream holder\nPermission granted for this synthetic test.\n';
  const texts=new Map(),entries=inventory(base).map(e=>prepare(e,{spdx:'MIT',archiveSha256:sha256('archive'),text,encodings:encoding(text)},texts));
  const evidence={schemaVersion:2,inputs:inputs(base),entries,collectionFailures:[]};
  write('docker/licenses/evidence.json',JSON.stringify(evidence));
  write('THIRD_PARTY_NOTICES.md',render(evidence.inputs,entries,texts));
  return {base,write,evidence,close:()=>rmSync(base,{recursive:true,force:true})};
}
test('offline regeneration is deterministic and preserves complete upstream text',()=>{
  const f=fixture();try{assert.equal(check(f.base),6);assert.equal(check(f.base),6);assert.match(readFileSync(join(f.base,'THIRD_PARTY_NOTICES.md'),'utf8'),/Copyright upstream holder/);}finally{f.close();}
});
for(const [name,mutate] of [
  ['changed licence text',f=>f.write('THIRD_PARTY_NOTICES.md',readFileSync(join(f.base,'THIRD_PARTY_NOTICES.md'),'utf8').replace('Copyright upstream holder','Changed holder'))],
  ['changed lockfile',f=>f.write('yarn.lock','changed\n')],
  ['new architecture inventory',f=>f.write('docker/pins/inventory/runtime-amd64.tsv','binary\tarch\tversion\tsource\tsourceVersion\torigin\n')],
  ['missing evidence',f=>{f.evidence.entries.pop();f.write('docker/licenses/evidence.json',JSON.stringify(f.evidence));}],
  ['unknown licence',f=>{f.evidence.entries[0].spdx='NOASSERTION';f.write('docker/licenses/evidence.json',JSON.stringify(f.evidence));}],
  ['extra uncontrolled text',f=>f.write('THIRD_PARTY_NOTICES.md',readFileSync(join(f.base,'THIRD_PARTY_NOTICES.md'),'utf8')+'uncontrolled\n')],
])test('fails closed: '+name,()=>{const f=fixture();try{mutate(f);assert.throws(()=>check(f.base));}finally{f.close();}});
test('known built-in Yarn patches retain their npm package; other sources fail closed',()=>{
  const f=fixture();try{
    f.write('yarn.lock','"pkg@patch":\n  resolution: "pkg@patch:pkg@npm%3A1.0.0#optional!builtin<compat/pkg>::version=1.0.0&hash=abcdef"\n');
    assert.ok(inventory(f.base).some(r=>r.id==='npm:pkg=1.0.0'));
    f.write('yarn.lock','"pkg@url":\n  resolution: "pkg@https://example.test/pkg.tgz"\n');
    assert.throws(()=>inventory(f.base),/Unsupported Yarn/);
  }finally{f.close();}
});

test('non-SPDX upstream labels are text references without choosing or relicensing terms',()=>{
  const body='Complete original upstream terms.\n';
  const ref='LicenseRef-'+sha256(body).slice(0,24);
  assert.equal(licenceReference({ecosystem:'cargo'},'MIT/Apache-2.0',body),ref);
  assert.equal(licenceReference({ecosystem:'deb'},'GPL-2+',body),ref);
  assert.equal(licenceReference({ecosystem:'npm'},'MIT OR Apache-2.0',body),'MIT OR Apache-2.0');
});

test('shared files and repeated HTML licence bodies appear once, with every applicable package',()=>{
  const rows=[{id:'npm:one=1',coverage:'build',arch:'all',spdx:'MIT',archiveSha256:sha256('one')},{id:'npm:two=1',coverage:'runtime',arch:'all',spdx:'MIT',archiveSha256:sha256('two')}];
  const text='File: COPYRIGHT.html\n\n<html><body><p>Copyright first holder</p><pre>Permission &lt;grant&gt; &amp; warranty.\n</pre><p>Copyright second holder</p><pre>Permission &lt;grant&gt; &amp; warranty.\n</pre><a href="https://example.test/licence">source</a></body></html>';
  const texts=new Map(),entries=rows.map(r=>prepare(r,{...r,text},texts)),output=render({},entries,texts);
  assert.equal(output.split('Permission <grant> & warranty.').length-1,1);
  assert.match(output,/Copyright first holder/);assert.match(output,/Copyright second holder/);
  assert.match(output,/source \(https:\/\/example.test\/licence\)/);
  assert.match(output,/one=1 — COPYRIGHT.html/);assert.match(output,/two=1 — COPYRIGHT.html/);
  assert.equal(readTexts(output).size,texts.size);
});
test('unapproved standard-text substitution is rejected',()=>{
  const f=fixture();try{f.evidence.entries[0].method='spdx-standard';f.write('docker/licenses/evidence.json',JSON.stringify(f.evidence));assert.throws(()=>check(f.base),/Unapproved/);}finally{f.close();}
});
test('approved standard text binds the declaration, author statement and pinned standard hash',()=>{
  const f=fixture();try{
    f.write('yarn.lock','"keyv@npm:4.5.4":\n  resolution: "keyv@npm:4.5.4"\n');
    const texts=new Map(),standard='Standard permission text.\n',author='none stated';
    const entries=inventory(f.base).map(row=>prepare(row,row.id==='npm:keyv=4.5.4'?{
      spdx:'MIT',archiveSha256:sha256('archive'),method:'spdx-standard',declaredLicense:'MIT',authorSha256:sha256(author),standardTextSha256:sha256(standard),encodings:encoding(standard),
      text:bundle([{source:'Package declaration',text:standardStatement(row,'MIT',author)},{source:'SPDX standard MIT',text:standard}]),
    }:{spdx:'MIT',archiveSha256:sha256('archive'),text:'Copyright upstream holder\nPermission granted.\n',encodings:encoding('upstream')},texts));
    const evidence={schemaVersion:2,inputs:inputs(f.base),entries,collectionFailures:[]};
    f.write('docker/licenses/evidence.json',JSON.stringify(evidence));f.write('THIRD_PARTY_NOTICES.md',render(evidence.inputs,entries,texts));
    assert.equal(check(f.base),6);
    const statement=[...texts.values()].find(t=>t.includes('Author stated in package metadata:'));
    assert.ok(statement.includes('none stated'));assert.ok(!/^Copyright/im.test(statement));
    entries.find(e=>e.method==='spdx-standard').standardTextSha256=sha256('wrong standard');f.write('docker/licenses/evidence.json',JSON.stringify(evidence));
    assert.throws(()=>check(f.base),/standard licence/);
  }finally{f.close();}
});

test('symlinked CLI on an invalid tree cannot silently succeed',()=>{
  const f=fixture(),link=f.base+'-link';try{
    copyFileSync(new URL('./generate.mjs',import.meta.url),join(f.base,'docker/licenses/generate.mjs'));
    f.evidence.entries.pop();f.write('docker/licenses/evidence.json',JSON.stringify(f.evidence));
    symlinkSync(f.base,link,'dir');
    const result=spawnSync(process.execPath,[join(link,'docker/licenses/generate.mjs'),'--check'],{encoding:'utf8'});
    assert.notEqual(result.status,0);assert.match(result.stderr,/Licence verification failed/);
  }finally{rmSync(link,{force:true});f.close();}
});
test('hash-shaped LicenseRef must match its package text',()=>{
  const f=fixture();try{
    f.evidence.entries[0].spdx='LicenseRef-'+'0'.repeat(24);f.write('docker/licenses/evidence.json',JSON.stringify(f.evidence));
    assert.throws(()=>check(f.base),/LicenseRef/);
  }finally{f.close();}
});
test('receipt binds both exact files and is never written for a differing upstream result',()=>{
  const f=fixture(),path=join(f.base,'receipt.json');try{
    const snapshot={notices:readFileSync(join(f.base,'THIRD_PARTY_NOTICES.md'),'utf8'),evidence:readFileSync(join(f.base,'docker/licenses/evidence.json'),'utf8'),counts:{components:6,distinctTextBlocks:1,unresolved:0}};
    const receipt=verifySnapshot(f.base,path,snapshot);
    assert.equal(receipt.schemaVersion,1);assert.equal(receipt.noticesSha256,sha256(snapshot.notices));assert.equal(receipt.evidenceSha256,sha256(snapshot.evidence));assert.ok(Number.isFinite(Date.parse(receipt.verifiedAt)));
    rmSync(path);assert.throws(()=>verifySnapshot(f.base,path,{...snapshot,notices:snapshot.notices+'tampered\n'}),/differs/);assert.equal(existsSync(path),false);
  }finally{f.close();}
});
test('upstream CLI recollects instead of trusting a consistent saved block',()=>{
  const f=fixture(),receipt=join(f.base,'receipt.json');try{
    assert.equal(check(f.base),6);
    copyFileSync(new URL('./generate.mjs',import.meta.url),join(f.base,'docker/licenses/generate.mjs'));
    f.write('docker/licenses/collect.mjs',`export function resetCollection() {}
export async function collect() {return {spdx:'MIT',archiveSha256:'${sha256('archive')}',text:'Different actual upstream text\\n',encodings:${JSON.stringify(encoding('different'))}};}
`);
    const result=spawnSync(process.execPath,[join(f.base,'docker/licenses/generate.mjs'),'--verify-upstream','--receipt',receipt],{encoding:'utf8'});
    assert.notEqual(result.status,0);assert.match(result.stderr,/Licence verification failed/);assert.equal(existsSync(receipt),false);
  }finally{f.close();}
});

test('importing from stdin does not attempt to resolve stdin as a script path',()=>{
  const url=new URL('./generate.mjs',import.meta.url).href;
  const result=spawnSync(process.execPath,['--input-type=module','-'],{input:`await import(${JSON.stringify(url)});console.log('imported');`,encoding:'utf8'});
  assert.equal(result.status,0,result.stderr);assert.equal(result.stdout,'imported\n');
});
