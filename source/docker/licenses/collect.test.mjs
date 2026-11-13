import test from 'node:test';
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {mkdtempSync,writeFileSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {extract,licenceFiles,decodeSource} from './collect.mjs';
test('archive members are literal operands; an empty licence is missing evidence',()=>{
  const dir=mkdtempSync(join(tmpdir(),'nwui-licence-archive-'));
  try {
    writeFileSync(join(dir,'--version'),'literal member contents');
    writeFileSync(join(dir,'LICENSE'),'');
    const tar=execFileSync('tar',['-cf','-','-C',dir,'--','--version','LICENSE'],{stdio:['ignore','pipe','ignore']});
    assert.equal(extract(tar,['--version']),'literal member contents');
    assert.throws(()=>licenceFiles(tar,['LICENSE']),/No complete licence text/);
  }finally{rmSync(dir,{recursive:true,force:true});}
});

test('strict UTF-8 preserves upstream replacement characters and Latin-1 creates none',()=>{
  const latin=decodeSource(Buffer.from([0x43,0x61,0x66,0xe9]));
  assert.equal(latin.text,'Café');assert.equal(latin.encoding,'latin-1');assert.equal(latin.replacementCharacters,0);assert.ok(!latin.text.includes('\uFFFD'));
  const original='Original upstream \uFFFD';const utf=decodeSource(Buffer.from(original));
  assert.equal(utf.text,original);assert.equal(utf.encoding,'utf-8');assert.equal(utf.replacementCharacters,1);
});
