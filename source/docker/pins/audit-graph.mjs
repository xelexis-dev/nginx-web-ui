import {readFileSync} from 'node:fs';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {lockedPackages} from './build-inputs.mjs';
export function auditGraph(graph,flatTree,lock) {
  const locked=lockedPackages(lock),tree=new Set();
  for(const line of flatTree.trimEnd().split('\n')) {
    const row=line.match(/^([A-Za-z0-9_-]+) v([^\s]+)(?: \(.*\))?(?: \(\*\))?$/);
    if(!row)throw new Error('Unsupported flat Cargo tree row: '+line);
    tree.add(row[1]+'@'+row[2]);
  }
  const sets={};
  for(const name of ['normal','build','lockedOnly']) {
    const values=graph[name];
    if(!Array.isArray(values)||values.some(x=>typeof x!=='string')||new Set(values).size!==values.length)throw new Error('Invalid graph set: '+name);
    sets[name]=new Set(values);
  }
  const present=new Set([...sets.normal,...sets.build]);
  const same=(a,b)=>a.size===b.size&&[...a].every(x=>b.has(x));
  if(!same(present,tree))throw new Error('Graph union differs from actual per-target Cargo tree');
  if([...tree].some(x=>!locked.has(x)))throw new Error('Tree is not a subset of Cargo.lock');
  if([...sets.lockedOnly].some(x=>present.has(x)))throw new Error('lockedOnly overlaps normal/build');
  if(!same(new Set([...present,...sets.lockedOnly]),locked))throw new Error('Tree plus lockedOnly differs from complete Cargo.lock');
  return {normal:sets.normal.size,build:sets.build.size,tree:tree.size,lockedOnly:sets.lockedOnly.size,lock:locked.size,treeSubsetOfLock:true,partitionEqualsLock:true,lockedOnlyDisjoint:true};
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href) {
  const record=JSON.parse(readFileSync(process.argv[2],'utf8'));
  console.log(JSON.stringify(auditGraph(record.crateGraph??record,readFileSync(process.argv[4],'utf8'),readFileSync(process.argv[3],'utf8')),null,2));
}
