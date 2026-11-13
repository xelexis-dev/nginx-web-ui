import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { RollbackViewResult } from "@/lib/overview-views";
const mocks=vi.hoisted(() => ({actions:[] as Array<(state:RollbackViewResult|null,form:FormData)=>Promise<RollbackViewResult|null>>,result:null as RollbackViewResult|null,fetch:vi.fn(),push:vi.fn(),refresh:vi.fn()}));
vi.mock("react",async original=>({...await original<typeof import("react")>(),useActionState:(action:(state:RollbackViewResult|null,form:FormData)=>Promise<RollbackViewResult|null>)=>{mocks.actions.push(action);return[mocks.result,()=>undefined,false];}}));
vi.mock("next/navigation",()=>({useRouter:()=>({push:mocks.push,refresh:mocks.refresh})}));
import { RollbackButton } from "./rollback-button";
beforeEach(()=>{vi.clearAllMocks();mocks.actions.length=0;mocks.result=null;vi.stubGlobal("fetch",mocks.fetch);});
afterEach(()=>vi.unstubAllGlobals());
it("posts the bound snapshot through browser API transport and refreshes on success",async()=>{
  renderToStaticMarkup(<RollbackButton snapshotId="snap-id" confirmLabel="saved"/>);
  mocks.fetch.mockResolvedValue(Response.json({result:{ok:true,output:"reload passed"},refresh:true}));
  const form=new FormData();form.set("_apiArgs",'["forged"]');
  expect(await mocks.actions[0]!(null,form)).toEqual({ok:true,output:"reload passed"});
  const [url,init]=mocks.fetch.mock.calls[0]!;expect(url).toBe("/api/ui/v1/forms/history.rollback");expect(init.method).toBe("POST");expect(init.credentials).toBe("same-origin");expect(init.body.get("_apiArgs")).toBe('["snap-id"]');expect(mocks.refresh).toHaveBeenCalledOnce();
});
it("shows a transport failure without claiming any nginx state change",async()=>{
  renderToStaticMarkup(<RollbackButton snapshotId="snap-id" confirmLabel="saved"/>);mocks.fetch.mockRejectedValue(new Error("offline"));mocks.result=await mocks.actions[0]!(null,new FormData());
  const html=renderToStaticMarkup(<RollbackButton snapshotId="snap-id" confirmLabel="saved"/>);expect(html).toContain("롤백 요청 실패");expect(html).toContain("API 서버에 요청을 전달하지 못했습니다");expect(html).not.toContain("이전 설정을 유지합니다");expect(mocks.refresh).not.toHaveBeenCalled();
});
