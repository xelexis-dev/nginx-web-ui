import { createServer } from 'node:http';
import { open, readFile, writeFile } from 'node:fs/promises';
import { constants, writeSync, closeSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { parseStatus, projectJournalRefusal, isJournalRefusal, isJournalRefusalCode, isStep } from './status.mjs';
import { createHash } from 'node:crypto';
import { isAbsolute } from 'node:path';
const assets = new URL('./', import.meta.url);
const failure=code=>Object.assign(Error('status file unavailable'),{code});
function ioFailure(e,leaf=false){
  return failure(leaf&&e.code==='ENOENT'?'status_missing':['ELOOP','ENOTDIR','ENXIO'].includes(e.code)?'status_unsafe':({EROFS:'storage_read_only',ENOSPC:'insufficient_space',EDQUOT:'insufficient_space',EINTR:'interrupted'}[e.code]||'io_error'));
}
async function safeParent(file){
 const s=await file.stat();if(!s.isDirectory()||s.uid!==0||((s.mode&0o022)&&!((s.mode&0o002)&&(s.mode&0o1000))))throw failure('status_unsafe');
}
async function readRuntime(path,exactLink=false) {
 if(!isAbsolute(path)||path.includes('\0')||path.endsWith('/'))throw failure('status_unsafe');
 const parts=path.split('/').filter(Boolean),leaf=parts.pop();if(!leaf||parts.some(p=>p==='.'||p==='..')||leaf==='.'||leaf==='..')throw failure('status_unsafe');
 let parent,file;
 try{
  parent=await open('/',constants.O_RDONLY|constants.O_DIRECTORY);await safeParent(parent);
  for(const part of parts){const next=await open(`/proc/self/fd/${parent.fd}/${part}`,constants.O_RDONLY|constants.O_DIRECTORY|constants.O_NOFOLLOW);await parent.close();parent=next;await safeParent(parent);}
  try{file=await open(`/proc/self/fd/${parent.fd}/${leaf}`,constants.O_RDONLY|constants.O_NOFOLLOW|constants.O_NONBLOCK);}catch(e){throw ioFailure(e,true);}
  const stat=await file.stat();if(!stat.isFile()||stat.uid!==0||(stat.mode&0o7777)!==0o644||stat.nlink>1||(exactLink&&stat.nlink!==1))throw failure('status_unsafe');
  if(stat.size>65536)throw failure('status_invalid');
  const buffer=Buffer.alloc(65537);let count=0;
  while(count<buffer.length){const {bytesRead}=await file.read(buffer,count,buffer.length-count,null);if(!bytesRead)break;count+=bytesRead;}
  if(count>65536)throw failure('status_invalid');
  try{return {text:new TextDecoder('utf-8',{fatal:true,ignoreBOM:true}).decode(buffer.subarray(0,count)),updatedAt:stat.mtime.toISOString()};}catch{throw failure('status_invalid');}
 }catch(e){if(e.code?.startsWith('status_')||['storage_read_only','insufficient_space','interrupted','io_error'].includes(e.code))throw e;throw ioFailure(e);}
 finally{await file?.close();await parent?.close();}
}
export async function readStatus(path='/run/nwui-db-upgrade.json') {
  const {text}=await readRuntime(path);
  try{return parseStatus(text);}catch{throw failure('status_invalid');}
}

async function bootIdentity() {
  const [raw,stat]=await Promise.all([
    readFile('/proc/sys/kernel/random/boot_id','utf8'),readFile('/proc/1/stat','utf8'),
  ]);
  const boot=raw.trim(), open=stat.indexOf('('), close=stat.lastIndexOf(')');
  if(!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(boot)
    ||open<0||open>=close||stat.slice(0,open).trim()!=='1'||stat.includes('\0'))throw failure('status_invalid');
  const ticks=stat.slice(close+1).trim().split(/\s+/)[19];
  if(!/^\d+$/.test(ticks??'')||BigInt(ticks)>18446744073709551615n)throw failure('status_invalid');
  return createHash('sha256').update(`nwui-boot-v1\0${boot}\0${BigInt(ticks)}`).digest('hex');
}

function parseRuntime(text) {
  try {
    const value=JSON.parse(text);
    if(!value||typeof value!=='object'||Array.isArray(value))throw failure('status_invalid');
    return value;
  }catch{throw failure('status_invalid');}
}

async function readServedStatus(statusPath,runtime) {
  if(runtime){
    const saved=await readRuntime(runtime.marker).catch(error=>{
      if(error.code!=='status_missing')throw error;
      return null;
    });
    if(saved){
      const marker=parseRuntime(saved.text);
      if(typeof marker.bootIdentity!=='string'||!/^[a-f0-9]{64}$/.test(marker.bootIdentity))throw failure('status_invalid');
      if(marker.bootIdentity===await runtime.identity()){
        if(Object.keys(marker).length!==5||marker.version!==1||!['restarting','refused'].includes(marker.state)
          ||!isJournalRefusalCode(marker.code)||!isStep(marker.step))throw failure('status_invalid');
        if(marker.state==='restarting')throw failure('status_missing');
        const descriptor=parseRuntime((await readRuntime(runtime.descriptor,true).catch(()=>{throw failure('status_invalid');})).text);
        if(descriptor.version!==1||descriptor.epoch!==1||descriptor.outcome!=='refused-preflight'
          ||descriptor.branch!=='upgrade'||descriptor.pgdata!=='/var/lib/postgresql/data/17/data'
          ||descriptor.backupId!==null||descriptor.schemaProfile!=='none'||descriptor.keyBinding?.state!=='unverified')
          throw failure('status_invalid');
        return projectJournalRefusal(marker,saved.updatedAt);
      }
    }
  }
  const value=await readStatus(statusPath);
  if(isJournalRefusal(value))throw failure('status_missing');
  return value;
}
export function createStatusServer(statusPath,runtime) {
  return createServer(async (request,response) => {
    const headers={'Cache-Control':'no-store','X-Content-Type-Options':'nosniff','Referrer-Policy':'same-origin',
      'Content-Security-Policy':"default-src 'none'; script-src 'self'; style-src 'self'; font-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'"};
    if (!['GET','HEAD'].includes(request.method)) {response.writeHead(405,{...headers,Allow:'GET, HEAD'});response.end();return;}
    try {
      const path=request.url;
      if (path==='/status.json') {
        const body=JSON.stringify(await readServedStatus(statusPath,runtime),(_k,v)=>typeof v==='bigint'?JSON.rawJSON(v.toString()):v);
        response.writeHead(200,{...headers,'Content-Type':'application/json; charset=utf-8'});response.end(request.method==='HEAD'?undefined:body);return;
      }
      const routes={'/':'index.html','/status.mjs':'status.mjs','/status.css':'status.css','/tokens.css':'tokens.css','/button.css':'button.css'};
      const asset=routes[path] || (/^\/fonts\/(?:woff2(?:-dynamic-subset)?\/)?[A-Za-z0-9_.-]+\.(?:css|woff2)$/.test(path)?path.slice(1):null);
      if (!asset) {response.writeHead(404,headers);response.end();return;}
      const body=await readFile(new URL(asset,assets));
      const type=asset.endsWith('.mjs')?'text/javascript':asset.endsWith('.css')?'text/css':asset.endsWith('.woff2')?'font/woff2':'text/html';
      response.writeHead(200,{...headers,'Content-Type':type+'; charset=utf-8'});response.end(request.method==='HEAD'?undefined:body);
    } catch (e) {
      if(request.url==='/status.json'&&e.code==='status_missing'){response.writeHead(202,{...headers,'Content-Type':'application/json; charset=utf-8'});response.end(request.method==='HEAD'?undefined:JSON.stringify({starting:true}));return;}
      response.writeHead(request.url==='/status.json'?503:404,{...headers,'Content-Type':'application/json; charset=utf-8'});
      response.end(request.method==='HEAD'?undefined:JSON.stringify({error:'상태를 읽을 수 없습니다. 컨테이너 로그를 확인하세요.'}));
    }
  });
}
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const identity=await bootIdentity();
  const readyPath='/run/nwui-upgrade-status-ready';
  const alreadyReady=await readRuntime(readyPath).then(record=>record.text===identity,()=>false);
  const server=createStatusServer(undefined,{
    marker:'/run/nwui-unrecorded.json',descriptor:'/run/nwui-storage.json',identity:()=>identity,
  });
  const deadline=performance.now()+2000;
  const listen=()=>server.listen(8080,'0.0.0.0');
  server.on('error',error=>{
    if(!alreadyReady&&error.code==='EADDRINUSE'&&performance.now()<deadline){setTimeout(listen,100);return;}
    if(!alreadyReady){
      process.stderr.write(`[upgrade-status] port 8080 unavailable: ${error.code}\n`);
      process.exitCode=100;
      return;
    }
    throw error;
  });
  server.once('listening',async()=>{
    await writeFile(readyPath,identity,{flag:constants.O_WRONLY|constants.O_CREAT|constants.O_TRUNC|constants.O_NOFOLLOW,mode:0o644});
    writeSync(3,'\n');closeSync(3);
  });
  listen();
}
