// Runs only in a disposable single-image container with the candidate nginx files.
import http from "node:http";
import { spawn } from "node:child_process";
import { networkInterfaces } from "node:os";
import assert from "node:assert/strict";
const api = http.createServer((req,res)=>{
  if(req.url==="/api/events"){
    res.writeHead(200,{"content-type":"text/event-stream","cache-control":"no-cache"});
    res.write("data: first\n\n");const timer=setTimeout(()=>res.end("data: last\n\n"),1500);res.on("close",()=>clearTimeout(timer));return;
  }
  res.setHeader("content-type","application/json");res.end(JSON.stringify({target:"api",path:req.url,scheme:req.headers["x-forwarded-proto"],ip:req.headers["x-real-ip"]}));
});
const web=http.createServer((req,res)=>{res.setHeader("content-type","application/json");res.end(JSON.stringify({target:"web",path:req.url,scheme:req.headers["x-forwarded-proto"]}));});
await Promise.all([new Promise(r=>api.listen(3001,"127.0.0.1",r)),new Promise(r=>web.listen(3000,"127.0.0.1",r))]);
const nginx=spawn("nginx",["-g","daemon off;"],{stdio:["ignore","ignore","inherit"]});
try{
  for(let i=0;i<40;i++){try{await fetch("http://127.0.0.1:8080/api/health");break;}catch{await new Promise(r=>setTimeout(r,50));}}
  assert.equal((await (await fetch("http://127.0.0.1:8080/api/health")).json()).target,"api");
  assert.equal((await (await fetch("http://127.0.0.1:8080/certificates")).json()).target,"web");
  const challenge=await (await fetch("http://127.0.0.1/.well-known/acme-challenge/probe-token")).json();assert.equal(challenge.target,"api");assert.equal(challenge.path,"/.well-known/acme-challenge/probe-token");
  const trusted=await(await fetch("http://127.0.0.1:8080/api/health",{headers:{"x-forwarded-proto":"https","x-real-ip":"198.51.100.2"}})).json();assert.equal(trusted.scheme,"https");assert.equal(trusted.ip,"198.51.100.2");
  const external=Object.values(networkInterfaces()).flat().find(a=>a.family==="IPv4"&&!a.internal)?.address;assert.ok(external);
  const forged=await(await fetch(`http://${external}:8080/api/health`,{headers:{"x-forwarded-proto":"https","x-real-ip":"198.51.100.2"}})).json();assert.equal(forged.scheme,"http");assert.notEqual(forged.ip,"198.51.100.2");
  const start=Date.now();const stream=await fetch("http://127.0.0.1:8080/api/events");const reader=stream.body.getReader();const first=await reader.read();assert.ok(new TextDecoder().decode(first.value).includes("data: first"));assert.ok(Date.now()-start<1200,"SSE must reach caller before upstream finishes");await reader.cancel();
  console.log("nginx runtime routing: API, SSR, HTTP-01, trusted TLS hop, spoof rejection, SSE passed (6/6)");
}finally{
  nginx.kill("SIGQUIT");await new Promise(r=>nginx.once("exit",r));api.closeAllConnections();web.closeAllConnections();await Promise.all([new Promise(r=>api.close(r)),new Promise(r=>web.close(r))]);
}
