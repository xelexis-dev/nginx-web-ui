import { beforeEach, expect, it, vi } from "vitest";
// Role checks, fanout concurrency and remote-error precedence moved with execution
// into tests/node_{service,registry,relay}.rs. These tests cover SSR presentation.
import { isValidElement } from "react";
import { NodeErrorCard, type NodeErrorCardProps } from "@/components/node-error";
const mock = vi.hoisted(() => ({ read: vi.fn() }));
vi.mock("./api-page", () => ({ readPageView: mock.read }));
import { authorizeNodePage, loadNodePage, loadNodePageAll } from "./node-page";
const node = { id: "n1", name: "Node", baseUrl: "https://node.test" };
const args = { nodeId: "n1", action: "config:read" as const, path: "/api/v1/audit", retryHref: "/nodes/n1/audit?page=2", listHref: "/nodes/n1/audit" };
beforeEach(() => vi.clearAllMocks());
it.each([true, false, undefined])("preserves target runtime DNS status (%s) through every loader with one API read", async available => {
  const runtimeDns = available === undefined ? undefined : { available, reason: available ? null : "노드 DNS 포트 충돌" };
  for (const load of [
    () => authorizeNodePage(args),
    () => loadNodePage({ ...args, toViewModel: raw => raw }),
    () => loadNodePageAll({ ...args, paths: {}, subjects: {}, toViewModel: raw => raw }),
  ]) {
    mock.read.mockClear();
    mock.read.mockResolvedValue({ ok: true, node, effectiveRole: "editor", data: {}, ...(runtimeDns ? { runtimeDns } : {}) });
    const result = await load();
    expect(result).toMatchObject({ ok: true, runtimeDns });
    expect(mock.read).toHaveBeenCalledExactlyOnceWith("/nodes/n1/audit", { page: "2" });
  }
});
it("uses one local API view request and keeps query filters", async () => {
  mock.read.mockResolvedValue({ ok: true, node, effectiveRole: "viewer", data: { items: [1] } });
  expect(await loadNodePage({ ...args, toViewModel: (raw) => (raw as {items:number[]}).items })).toEqual({ok:true,node,effectiveRole:"viewer",data:[1]});
  expect(mock.read).toHaveBeenCalledExactlyOnceWith("/nodes/n1/audit", {page:"2"});
});
it("does not let legacy caller paths select remote requests", async () => {
  mock.read.mockResolvedValue({ok:true,node,effectiveRole:"viewer",data:{}});
  await loadNodePage({...args,path:"https://evil.test/secrets",toViewModel:raw=>raw});
  expect(mock.read).toHaveBeenCalledExactlyOnceWith("/nodes/n1/audit",{page:"2"});
});
it("passes aggregated named envelopes to the presentation converter once", async () => {
  const data={servers:{servers:[]},certificates:{certificates:[]}};
  mock.read.mockResolvedValue({ok:true,node,effectiveRole:"editor",data});
  const convert=vi.fn(raw=>raw);
  await loadNodePageAll({...args,paths:{servers:"ignored",certificates:"ignored"},subjects:{servers:"servers",certificates:"certificates"},toViewModel:convert});
  expect(convert).toHaveBeenCalledExactlyOnceWith(data);
  expect(mock.read).toHaveBeenCalledOnce();
});
it("renders remote failures with their subject and navigation context",async()=>{
  const nodeError={failure:{ok:false,kind:"not_found",message:"missing"},nodeId:"n1",retryHref:args.retryHref,listHref:args.listHref,subject:"certificate"};
  mock.read.mockResolvedValue({ok:false,nodeError});
  const convert=vi.fn();
  const result=await loadNodePage({...args,toViewModel:convert});
  expect(convert).not.toHaveBeenCalled();
  if(result.ok||!isValidElement<NodeErrorCardProps>(result.render))throw Error("expected card");
  expect(result.render.type).toBe(NodeErrorCard);
  expect(result.render.props).toEqual(nodeError);
});
it("converts malformed remote data to one error card within the SSR boundary",async()=>{
  mock.read.mockResolvedValue({ok:true,node,effectiveRole:"viewer",data:null});
  const result=await loadNodePage({...args,toViewModel:()=>{throw Error("unexpected");}});
  if(result.ok||!isValidElement<NodeErrorCardProps>(result.render))throw Error("expected card");
  expect(result.render.props.failure.kind).toBe("node_error");
  expect(result.render.props.listHref).toBe(args.listHref);
});
it("preserves local API navigation and outage errors",async()=>{
  const error=new Error("NEXT_NOT_FOUND");
  mock.read.mockRejectedValue(error);
  await expect(authorizeNodePage(args)).rejects.toBe(error);
});
it("uses the API authorization result for a creation page",async()=>{
  mock.read.mockResolvedValue({ok:true,node,effectiveRole:"editor",data:null});
  expect(await authorizeNodePage({...args,retryHref:"/nodes/n1/stream/new"})).toEqual({ok:true,node,effectiveRole:"editor"});
});
