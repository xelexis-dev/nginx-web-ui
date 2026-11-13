import {readFileSync} from 'node:fs';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
const columns=['binary','arch','version','source','sourceVersion'];
export function normalizedInventory(text) {
  const lines=text.trimEnd().split('\n');const header=lines.shift().split('\t');
  if(header.join('\t')!==columns.join('\t')&&header.join('\t')!==[...columns,'origin'].join('\t'))throw new Error('Unexpected inventory header');
  const rows=new Map();
  for(const line of lines) {
    const fields=line.split('\t');
    if(fields.length!==header.length||fields.some(s=>!s||/[\r\n]/.test(s))||!['all','arm64','amd64'].includes(fields[1]))throw new Error('Malformed inventory row');
    if(header.length===6&&!['base','debian','debian-security','nginx.org','pgdg'].includes(fields[5]))throw new Error('Unknown inventory origin');
    const key=fields[0]+'\t'+fields[1];if(rows.has(key))throw new Error('Duplicate package '+key);rows.set(key,fields.slice(0,5).join('\t'));
  }
  return rows;
}
export function differences(expected,actual) {
  const before=normalizedInventory(expected),after=normalizedInventory(actual);
  return [...new Set([...before.keys(),...after.keys()])].sort().filter(key=>before.get(key)!==after.get(key)).map(key=>({key,expected:before.get(key)??null,actual:after.get(key)??null}));
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const delta=differences(readFileSync(process.argv[2],'utf8'),readFileSync(process.argv[3],'utf8'));
    console.log(JSON.stringify({matched:delta.length===0,differences:delta},null,2));process.exitCode=delta.length?1:0;
  }catch(error){console.error(error.message);process.exitCode=2;}
}
