import {readFileSync} from 'node:fs';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
export function checkLocks(root, installedTurbo) {
  const manifest=JSON.parse(readFileSync(resolve(root,'package.json'),'utf8'));
  if(!/^yarn@\d+\.\d+\.\d+\+sha224\.[a-f0-9]{56}$/.test(manifest.packageManager??'')) throw new Error('packageManager integrity is missing');
  const lock=readFileSync(resolve(root,'yarn.lock'),'utf8');
  const turbo=[...lock.matchAll(/resolution: "turbo@npm:([^\"]+)"/g)].map(m=>m[1]);
  const bootstrap=JSON.parse(readFileSync(resolve(root,'docker/turbo/package-lock.json'),'utf8'));
  const version=bootstrap.packages['node_modules/turbo']?.version;
  if(turbo.length!==1||turbo[0]!==version||installedTurbo&&JSON.parse(readFileSync(installedTurbo,'utf8')).version!==version)throw new Error('Turbo bootstrap differs from yarn.lock');
  for(const name of ['turbo','@turbo/linux-64','@turbo/linux-arm64']) {
    const row=bootstrap.packages['node_modules/'+name];
    if(row?.version!==version||!/^sha512-[A-Za-z0-9+/]+=*$/.test(row.integrity??''))throw new Error('Turbo SRI/platform pin missing: '+name);
  }
  // Cargo permits exact git commits; floating git sources are never accepted.
  const cargo=readFileSync(resolve(root,'apps/api/Cargo.lock'),'utf8');
  for(const [,source] of cargo.matchAll(/^source = "([^\"]+)"/gm)) {
    if(source.startsWith('git+')&&!/#[a-f0-9]{40}$/.test(source))throw new Error('Unpinned Cargo git source');
  }
  for(const [,resolution] of lock.matchAll(/^  resolution: "([^\"]+)"/gm)) {
    if(/(?:https?:|git:|ssh:)/.test(resolution)&&!/(?:#commit=|#)[a-f0-9]{40,64}(?:[&"?]|$)/.test(resolution)) throw new Error('Unpinned Yarn URL/git source');
  }
  return version;
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href)console.log('checked Turbo '+checkLocks(process.argv[2],process.argv[3]));
