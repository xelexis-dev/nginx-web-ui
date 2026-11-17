const outcomes = {
  upgrading:['데이터베이스를 준비하고 있습니다','완료될 때까지 컨테이너를 중지하거나 이미지를 바꾸지 마세요. 준비 중에는 프록시와 관리 화면을 사용할 수 없습니다.'],
  pg17:['데이터베이스 준비를 마쳤습니다','서비스가 시작되고 있습니다. 관리 화면으로 자동으로 이동합니다.'],
  restored:['원본 PG15 데이터를 검증 후 복원했습니다',''],
  'restore-incomplete':['원본 복원을 끝내지 못했습니다',''],
  'activation-failed':['업그레이드는 완료됐지만 서비스 시작에 실패했습니다',''],
  'refused-preflight':['업그레이드 전 검사에서 중단했습니다',''],
  'refused-fresh':['새 설치 준비를 끝내지 못했습니다',''],
  'refused-privilege':['PG17 데이터는 변경되지 않았습니다',''],
  postponed:['업그레이드를 보류했습니다','먼저 2.0.x 최신(2.0.3 이상) 이미지에서 이전을 완료한 뒤 2.1로 올리세요. 기존 데이터는 변경하지 않았습니다.'],
};
// Separate from `outcomes`: a Rust parity test reads that table's literal key:['title' shape.
const badges={upgrading:['진행 중','info'],pg17:['완료','success'],postponed:['보류됨','warning'],
  'refused-preflight':['중단됨','error'],'refused-fresh':['중단됨','error'],'refused-privilege':['중단됨','error'],
  'restore-incomplete':['중단됨','error'],'activation-failed':['중단됨','error']};
const attention=['확인 필요','warning'];
const steps={p0:'저장소 확인',p0b:'시작 설정 확인',f1:'새 저장소 준비',y1:'기존 저장소 확인',y2:'실행 준비',y3:'시작 준비',
  'p1-backup':'원본 보관','p2-preflight':'업그레이드 전 검사','p3-convert':'PG17 변환','p4-target':'변환 결과 검증',
  's4-prepare':'서비스 권한 준비','p5-publish':'실행 파일 준비','p5-guard':'설정 검사','p5-commit':'PG17 선택',
  's4-activate':'서비스 활성화',handoff:'서비스 시작','phase2-monitor':'시작 상태 확인','p6-cleanup':'이전 데이터 정리',
  'r-restore':'원본 복원','cmd-restore-pg15':'원본 복원','cmd-remove-backup':'보관 데이터 제거','cmd-remove-discarded':'보존 데이터 제거'};
const numberSources=new WeakMap();
const object=value=>value!==null&&typeof value==='object'&&!Array.isArray(value);
const string=value=>typeof value==='string'&&value.isWellFormed();
const invalid=()=>{throw Object.assign(Error('invalid status'),{code:'status_invalid'});};
function uint(value,key,max) {
  const n=value?.[key],source=numberSources.get(value)?.get(key);
  if(source!==undefined && (!/^(0|[1-9][0-9]*)$/.test(source)||BigInt(source)>max))invalid();
  if(typeof n==='bigint'){if(n<0n||n>max)invalid();}
  else if(typeof n!=='number'||!Number.isInteger(n)||n<0||n>Number(max))invalid();
  return n;
}
function strings(value,keys) {
  if(!object(value)||!keys.every(k=>string(value[k])))invalid();
  return Object.fromEntries(keys.map(k=>[k,value[k]]));
}
const optional=(value,convert)=>value==null?null:convert(value);
const copy=value=>({...strings(value,['id','removeCommand']),bytes:uint(value,'bytes',18446744073709551615n)});
const warningCodes=['desktop_shared_folder_isolation','storage_modes_normalized'];
function warnings(value){
  if(value===undefined)return [];
  if(!Array.isArray(value)||value.length>warningCodes.length||new Set(value.map(w=>object(w)?w.code:w)).size!==value.length)invalid();
  return value.map(w=>{
    if(!object(w)||!warningCodes.includes(w.code)||!string(w.message)||!w.message||!Array.isArray(w.scopes)||!w.scopes.length||w.scopes.length>4||new Set(w.scopes).size!==w.scopes.length||!w.scopes.every(s=>['db','app','nginx-log','app-log'].includes(s)))invalid();
    return {code:w.code,message:w.message,scopes:[...w.scopes]};
  });
}
export function normalizeStatus(s) {
  if(!object(s)||uint(s,'version',255n)!==1||!string(s.outcome)||!s.outcome||!['upgrade','fresh','ready'].includes(s.branch)||!string(s.updatedAt))invalid();
  const step=s.step,storageWarnings=warnings(s.warnings);
  if(!object(step)||!Object.hasOwn(steps,step.id)||!string(step.startedAt)||!Array.isArray(s.locations)||!s.locations.every(string)||!Array.isArray(s.discarded)||typeof s.cleanupPending!=='boolean'||!object(s.retry))invalid();
  const failure=optional(s.failure,f=>{if(!['transient','deterministic'].includes(f?.class))invalid();return {...strings(f,['code','message','nextAction']),class:f.class};});
  const space=optional(s.space,p=>{if(!object(p))invalid();return Object.fromEntries(['requiredBytes','availableBytes','inodesRequired','inodesAvailable'].map(k=>[k,uint(p,k,18446744073709551615n)]));});
  const retry=Object.fromEntries(['nextEligibleAt','reason'].map(k=>[k,optional(s.retry[k],v=>{if(!string(v))invalid();return v;})]));
  return {version:1,updatedAt:s.updatedAt,outcome:s.outcome,branch:s.branch,
    step:{id:step.id,index:uint(step,'index',4294967295n),total:uint(step,'total',4294967295n),startedAt:step.startedAt,bytesDone:uint(step,'bytesDone',18446744073709551615n),bytesTotal:uint(step,'bytesTotal',18446744073709551615n)},
    failure,space,locations:s.locations,backup:optional(s.backup,copy),discarded:s.discarded.map(copy),cleanupPending:s.cleanupPending,retry,...(storageWarnings.length?{warnings:storageWarnings}:{})};
}
export function parseStatus(text) {
  const raw=JSON.parse(text,function(key,value,context){
    if(typeof value==='number'){
      let sources=numberSources.get(this);if(!sources)numberSources.set(this,sources=new Map());sources.set(key,context.source);
      if(/^(0|[1-9][0-9]*)$/.test(context.source)&&!Number.isSafeInteger(value))return BigInt(context.source);
    }
    return value;
  });
  return normalizeStatus(raw);
}
const size=bytes=>new Intl.NumberFormat('ko-KR',{maximumFractionDigits:1}).format(Number(bytes)/1024/1024)+' MiB';
export function formatElapsed(startedAt,now) {
  const started=Date.parse(startedAt);
  if(!Number.isFinite(started)||!Number.isFinite(now))return '확인할 수 없음';
  const total=Math.max(0,Math.floor((now-started)/1000)),hours=Math.floor(total/3600),minutes=Math.floor(total%3600/60),seconds=total%60;
  return hours?`${hours}시간 ${minutes}분 ${seconds}초`:minutes?`${minutes}분 ${seconds}초`:`${seconds}초`;
}
export function clockOffset(dateHeader,now) {
  const server=Date.parse(dateHeader);
  return Number.isFinite(server)?server-now:0;
}
// server.mjs answers /status.json only with direct JSON, so anything else is the app taking over.
export function returnDecision(response) {
  if(!response)return 'wait';
  const type=(response.headers.get('content-type')??'').split(';')[0].trim().toLowerCase();
  if(!response.redirected&&type==='application/json')return 'status';
  return response.status<500?'return':'wait';
}
export function statusView(raw) {
  const status=normalizeStatus(raw),step=status.step,name=steps[step.id];
  const labels=Object.hasOwn(outcomes,status.outcome)?outcomes[status.outcome]:[status.outcome,''];
  const inode=status.space&&(BigInt(status.space.inodesRequired)===0n&&BigInt(status.space.inodesAvailable)===0n?'inode: 파일 시스템이 개수를 제공하지 않아 확인하지 않음':`inode: ${status.space.inodesAvailable} / ${status.space.inodesRequired}`);
  const upgrading=status.outcome==='upgrading',counted=upgrading&&step.total>0;
  const percent=counted&&step.bytesTotal?Math.min(100,Math.floor(Number(step.bytesDone)/Number(step.bytesTotal)*100)):null;
  const [label,tone]=upgrading&&status.warnings?attention:Object.hasOwn(badges,status.outcome)?badges[status.outcome]:attention;
  const guidance=status.failure?.nextAction??status.retry.reason??labels[1];
  return {warnings:(status.warnings??[]).map(w=>w.message),title:labels[0],guidance,lead:status.failure?'':guidance,
    phase:counted?`${step.index} / ${step.total} 단계 · ${name}`:status.outcome==='pg17'?`완료 · ${name}`:upgrading?'단계 확인 중':'다음 조치를 기다리고 있습니다',
    badge:{label,tone},elapsedKind:counted?'step':'none',startedAt:step.startedAt,
    space:status.space?`${size(status.space.availableBytes)} / ${size(status.space.requiredBytes)} 필요 · ${inode}`:'아직 집계하지 않았습니다.',
    retained:[status.backup,...status.discarded].filter(Boolean).map(x=>`${x.id} · ${size(x.bytes)} · ${x.removeCommand}`),
    percent,progressLabel:percent===null?'':`현재 단계 ${percent}%`,
    bytes:!counted?'':step.bytesTotal?`${size(step.bytesDone)} / ${size(step.bytesTotal)}`:'전체 크기를 집계하지 않는 단계입니다.',
    updated:Number.isFinite(Date.parse(status.updatedAt))?new Date(status.updatedAt).toLocaleString('ko-KR'):'확인할 수 없음',
    failure:status.failure?[status.failure.message,status.failure.nextAction].filter(Boolean).join('\n\n'):null,
    cleanup:status.cleanupPending,locations:status.locations,returning:status.outcome==='pg17',
    // The guidance line or the failure block already carries a matching reason.
    retry:[status.retry.nextEligibleAt?`다음 재시도 가능: ${new Date(status.retry.nextEligibleAt).toLocaleString('ko-KR')}`:'',status.retry.reason===guidance?null:status.retry.reason].filter(Boolean).join(' · '),
    connection:status.outcome==='pg17'?'서비스 연결을 확인하고 있습니다. 연결되면 자동으로 이동합니다.':'상태를 2초마다 확인합니다.'};
}
export const startingView={title:'데이터베이스 준비를 시작하고 있습니다',lead:'준비 중에는 프록시와 관리 화면을 사용할 수 없습니다.',phase:'단계 확인 중',badge:{label:'진행 중',tone:'info'},elapsedKind:'none',startedAt:'',
  space:'확인 중',updated:'확인 중',bytes:'',percent:null,progressLabel:'',warnings:[],failure:null,cleanup:false,retry:'',locations:[],retained:[],returning:false,connection:'첫 상태 기록을 기다리고 있습니다.'};
if (typeof document !== 'undefined') {
  // Unchanged text is not reassigned: live regions would re-announce and selections would clear on every poll.
  const el=id=>document.getElementById(id), text=(id,value)=>{if(el(id).textContent!==value)el(id).textContent=value;}, fill=(id,value)=>{text(id,value);el(id).hidden=!value;};
  try { const theme=localStorage.getItem('nwui-theme');if(['light','dark'].includes(theme))document.documentElement.dataset.theme=theme; } catch {}
  el('theme').onclick=()=>{
    const dark=document.documentElement.dataset.theme==='dark'||(!document.documentElement.dataset.theme&&matchMedia('(prefers-color-scheme: dark)').matches);
    const theme=dark?'light':'dark';document.documentElement.dataset.theme=theme;try{localStorage.setItem('nwui-theme',theme);}catch{}
  };
  const list=(id,items)=>{if([...el(id).children].map(li=>li.textContent).join('\0')!==items.join('\0'))el(id).replaceChildren(...items.map(value=>{const li=document.createElement('li');li.textContent=value;return li;}));el(id).closest('.notice,.detail-section').hidden=!items.length;};
  let anchor=null,offset=0;
  const tick=()=>{if(anchor!==null)text('elapsed',formatElapsed(anchor,Date.now()+offset));};
  setInterval(tick,1000);
  function render(v) {
    text('title',v.title);fill('guidance',v.lead);text('phase',v.phase);text('status-badge',v.badge.label);el('status-badge').dataset.tone=v.badge.tone;
    anchor=v.elapsedKind==='step'?v.startedAt:null;el('elapsed-fact').hidden=anchor===null;tick();
    text('updated',v.updated);text('space',v.space);
    el('progress-block').hidden=!v.bytes;fill('bytes',v.bytes);el('progress').hidden=v.percent===null;if(v.percent!==null)el('progress').value=v.percent;fill('progress-label',v.progressLabel);
    el('return-notice').hidden=!v.returning;list('storage-warnings',v.warnings);
    text('failure',v.failure??'');el('failure-notice').hidden=v.failure===null;
    el('cleanup').hidden=!v.cleanup;fill('retry',v.retry);list('locations',v.locations);list('retained',v.retained);text('connection',v.connection);
  }
  async function refresh() {
    try {
      const response=await fetch('/status.json',{cache:'no-store'}).catch(()=>null),decision=returnDecision(response);
      if(decision==='return'){location.replace('/');return;}
      if(decision==='wait'||!response.ok)throw Error();
      const body=await response.text();
      if(JSON.parse(body).starting)render(startingView);
      else{
        const v=statusView(parseStatus(body)),measured=clockOffset(response.headers.get('date'),Date.now());
        // Date has one-second resolution and arrives late: the largest reading is the closest and never steps the counter back.
        offset=v.startedAt===anchor?Math.max(offset,measured):measured;
        render(v);
      }
    } catch {text('connection','상태를 읽을 수 없습니다. 컨테이너 로그를 확인하세요. 서비스 전환 중이면 잠시 후 관리 화면에 다시 접속하세요.');}
    setTimeout(refresh,2000);
  }
  refresh();
}
