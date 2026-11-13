import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync,mkdirSync,writeFileSync,readFileSync,chmodSync,symlinkSync,statSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';
const remove=['mount','umount','su','passwd','chfn','chsh','gpasswd','newgrp','chage','expiry'];
function program(){
 const dockerfile=readFileSync(new URL('../../Dockerfile',import.meta.url),'utf8'),runner=dockerfile.split(/^FROM runtime-content AS runner$/m)[1];assert.ok(runner,'final runner stage');
 const lines=runner.split('\n');
 for(let i=0;i<lines.length;i++)if(lines[i].startsWith('RUN ')){
  let instruction=lines[i];while(instruction.endsWith('\\'))instruction+='\n'+lines[++i];
  if(instruction.includes('chmod u-s,g-s'))return instruction.slice(4).replace(/\\\n[ \t]*/g,' ');
 }
 assert.fail('final runner must enforce H1 permission hardening');
}
function fixture(options,check){
 const root=mkdtempSync(join(tmpdir(),'nwui permissions '));
 try{
  mkdirSync(root+'/bin');mkdirSync(root+'/pkg');
  for(const name of [...remove,'unix_chkpwd','s6-overlay-suexec',...(options.extra?['unexpected']:[])]){
   if(name===options.missing)continue;writeFileSync(root+'/pkg/'+name,'fixture');chmodSync(root+'/pkg/'+name,name==='unix_chkpwd'?0o2755:name==='s6-overlay-suexec'||name==='unexpected'?0o4755:0o6755);
   symlinkSync(root+'/pkg/'+name,root+'/bin/'+name);
  }
  const shim='#!'+process.execPath+'\n'+`
const fs=require('node:fs'),path=require('node:path'),root=process.env.FIXTURE_ROOT,tool=path.basename(process.argv[1]),args=process.argv.slice(2);
fs.appendFileSync(root+'/calls',JSON.stringify({tool,args})+'\\n');
if(tool==='readlink'){const p=args.at(-1);console.log(p==='/usr/sbin/unix_chkpwd'?root+'/pkg/unix_chkpwd':p==='/command/s6-overlay-suexec'?root+'/pkg/s6-overlay-suexec':fs.realpathSync(p));}
if(tool==='chmod'){if(args[0]!=='u-s,g-s'||args.slice(1).some(p=>!p.startsWith(root+'/pkg/')))process.exit(99);for(const p of args.slice(1))fs.chmodSync(p,(fs.statSync(p).mode&0o7777)&~0o6000);}
if(tool==='find'){for(const n of fs.readdirSync(root+'/pkg'))if(fs.statSync(root+'/pkg/'+n).mode&0o6000)console.log(root+'/pkg/'+n);if(process.env.FAIL_FIND==='yes')process.exit(1);}
`;
  for(const name of ['readlink','chmod','find']){writeFileSync(root+'/bin/'+name,shim);chmodSync(root+'/bin/'+name,0o755);}symlinkSync('/usr/bin/sort',root+'/bin/sort');
  const r=spawnSync('/bin/sh',['-ec',program()],{encoding:'utf8',env:{...process.env,PATH:root+'/bin',FIXTURE_ROOT:root,FAIL_FIND:options.failFind?'yes':'no'}});
  const calls=readFileSync(root+'/calls','utf8').trim().split('\n').map(JSON.parse);check(r,calls,root);
 }finally{rmSync(root,{recursive:true,force:true});}
}
test('H1 strips only the ten resolved tools in one chmod and retains both helpers',()=>fixture({},(r,calls,root)=>{
 assert.equal(r.status,0,r.stderr);const chmod=calls.filter(x=>x.tool==='chmod');assert.equal(chmod.length,1);assert.deepEqual(chmod[0].args,['u-s,g-s',...remove.map(n=>root+'/pkg/'+n)]);
 for(const n of remove)assert.equal(statSync(root+'/pkg/'+n).mode&0o6000,0);assert.equal(statSync(root+'/pkg/unix_chkpwd').mode&0o6000,0o2000);assert.equal(statSync(root+'/pkg/s6-overlay-suexec').mode&0o6000,0o4000);
 assert.deepEqual(calls.find(x=>x.tool==='find').args,['/','-xdev','-type','f','-perm','/6000','-print']);
}));
test('H1 rejects an unexpected setuid file',()=>fixture({extra:true},r=>{assert.notEqual(r.status,0);assert.match(r.stderr,/H1|allowlist/);}));
test('H1 rejects a missing allowed helper',()=>fixture({missing:'unix_chkpwd'},r=>{assert.notEqual(r.status,0);assert.match(r.stderr,/H1|allowlist/);}));
test('H1 cannot turn a failed find into a successful allowlist check',()=>fixture({failFind:true},r=>{assert.notEqual(r.status,0);assert.match(r.stderr,/H1|allowlist/);}));
