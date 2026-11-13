import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { ReactElement } from "react";
const mock=vi.hoisted(()=>({error:null as unknown,push:vi.fn(),refresh:vi.fn()}));
vi.mock("react",async original=>({...await original<typeof import("react")>(),
  useTransition:()=>[false,(run:()=>void)=>run()],
  useState:(initial:unknown)=>[initial===null?mock.error:initial,(value:unknown)=>{if(initial===null)mock.error=value;}],
}));
vi.mock("next/navigation",()=>({useRouter:()=>({push:mock.push,refresh:mock.refresh})}));
import { CancelIssuanceButton } from "./cancel-button";
beforeEach(()=>{mock.error=null;mock.push.mockClear();mock.refresh.mockClear();});
afterEach(()=>vi.unstubAllGlobals());
async function cancel(status:number,body:unknown){
  vi.stubGlobal("fetch",vi.fn(async()=>Response.json(body,{status})));
  const tree=CancelIssuanceButton({cancelPath:"/api/v1/certificates/test/cancel"}) as ReactElement<{children:ReactElement<{onClick:()=>Promise<void>}>[]}>;
  await tree.props.children[0]!.props.onClick();
}
it.each([403,500])("renders a structured HTTP %i error as text",async status=>{
  await cancel(status,{error:{code:"denied",message:"중단 권한이 없습니다"}});
  expect(mock.error).toBe("중단 권한이 없습니다");
  expect(renderToStaticMarkup(<CancelIssuanceButton cancelPath="/test"/>)).toContain("중단 권한이 없습니다");
});
it("keeps legacy string errors and a safe fallback for malformed errors",async()=>{
  await cancel(409,{error:"이미 완료되었습니다"});expect(mock.error).toBe("이미 완료되었습니다");
  await cancel(502,{error:{message:{unexpected:true}}});expect(mock.error).toBe("중단하지 못했습니다 (HTTP 502)");
});
it("sends an expired session to login",async()=>{
  await cancel(401,{error:{code:"unauthorized",message:"로그인 필요"}});
  expect(mock.push).toHaveBeenCalledWith("/login");expect(mock.error).toBeNull();
});
