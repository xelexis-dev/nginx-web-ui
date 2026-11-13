import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
const mocks = vi.hoisted(() => ({ read: vi.fn(), optional: vi.fn() }));
vi.mock("@/lib/api-page", () => ({ readPageView: mocks.read }));
vi.mock("@/lib/optional-read", () => ({ readOptionalView: mocks.optional }));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }) }));
vi.mock("@repo/db", () => { throw new Error("page must not import DB runtime"); });
vi.mock("@repo/presentation/nginx", () => { throw new Error("page must not import nginx runtime"); });
vi.mock("@/app/(app)/history/rollback-button", () => ({ RollbackButton: ({ snapshotId }: { snapshotId: string }) => <button data-snapshot={snapshotId}>이 스냅샷으로 롤백</button> }));
import Dashboard from "@/app/(app)/page";
import Audit from "@/app/(app)/audit/page";
import AuditDetail from "@/app/(app)/audit/[id]/page";
import History from "@/app/(app)/history/page";
import HistoryDetail from "@/app/(app)/history/[id]/page";
beforeEach(() => vi.clearAllMocks());
it("dashboard renders API counts and unavailable metrics without database access; secondary panels fail on their own", async () => {
  mocks.read.mockResolvedValue({ email:"viewer@example.test",serverTotal:7,serverActive:3,streamTotal:2,streamActive:1,audit24h:1200,metrics:null });
  const servers = { canWrite:false, canApply:false, upstreams:[], servers:[{ id:"s1", serverName:"api.example.test", listen:80, enabled:true, upstreamId:null, certificateId:null, config:{ proxyPass:"http://10.0.0.1:3000" } }], pending:{ count:0, hasBaseline:true, diff:{ servers:[], streams:[], upstreams:[], hasChanges:false } } };
  const byPath: Record<string, unknown> = {
    "/reverse-proxy": { ok:true, data:servers },
    "/certificates": { ok:true, data:{ canWrite:false, data:{ items:[] } } },
    "/nodes": { ok:false, reason:"forbidden" },
    "/audit": { ok:false, reason:"error", message:"API 서버에 연결하지 못했습니다." },
    "/history": { ok:true, data:{ data:{ items:[], showDetails:true } } },
  };
  mocks.optional.mockImplementation(async (path: string) => byPath[path]);
  const html=renderToStaticMarkup(await Dashboard());
  // The required read is the dashboard view, once; the rest are secondary reads of existing views.
  expect(mocks.read).toHaveBeenCalledExactlyOnceWith("/");
  expect(mocks.optional.mock.calls.map((c) => c[0]).sort()).toEqual(["/audit", "/certificates", "/history", "/nodes", "/reverse-proxy"]);
  expect(html).toContain("3 / 7");expect(html).toContain("1,200");expect(html).toContain("nginx 수치를 아직 수집하지 못했습니다");
  // Server status: saved-config basis, the no-problem label is 활성 (not 정상), a viewer gets no 새 서버.
  expect(html).toContain("저장된 설정·인증서 기준");expect(html).toContain("api.example.test");expect(html).toContain(">활성<");expect(html).not.toContain("새 서버");
  // Forbidden panel is hidden; a failed one says why; the rest still render.
  expect(html).not.toContain("노드 관리");
  expect(html).toContain("읽지 못했습니다: API 서버에 연결하지 못했습니다.");
  expect(html).toContain("모든 변경이 nginx에 반영되어 있습니다.");
  expect(html).toContain("기록 없음");
});
it("audit forwards filters and formats API timestamps and before/after detail", async () => {
  const log={id:"a",createdAt:"2026-09-23T01:02:03.000Z",action:"node:pair",target:"edge",actorEmail:null,actorLabel:"token:issued-id",ip:null,before:null,after:{role:"viewer"}};
  mocks.read.mockResolvedValueOnce({items:[log],total:51,page:2,totalPages:2,actions:["node:pair"],actionFilter:"node:pair"});
  const html=renderToStaticMarkup(await Audit({searchParams:Promise.resolve({page:"2",action:"node:pair"})}));expect(mocks.read).toHaveBeenLastCalledWith("/audit",{page:"2",action:"node:pair"});expect(html).toContain("token:issued-id");expect(html).toContain((new Date(log.createdAt).toLocaleString("ko-KR", { dateStyle: "medium", timeStyle: "medium", timeZone: "UTC" }) + " UTC"));
  mocks.read.mockResolvedValueOnce({log});const detail=renderToStaticMarkup(await AuditDetail({params:Promise.resolve({id:"a/b"})}));expect(mocks.read).toHaveBeenLastCalledWith("/audit/a%2Fb");expect(detail).toContain("viewer");expect(detail).toContain("token:issued-id");
});
it("history renders authoritative snapshot diff and rollback permission", async () => {
  mocks.read.mockResolvedValueOnce({data:{items:[{id:"snap",iso:"2026-09-23T00:00:00.000Z",actorEmail:null,label:"saved",upstreamCount:1,serverCount:2,streamCount:3}],showDetails:true}});
  expect(renderToStaticMarkup(await History())).toContain("saved");expect(mocks.read).toHaveBeenLastCalledWith("/history");
  const view={snapshot:{id:"snap",label:"saved",actorEmail:null},iso:"2026-09-23T00:00:00.000Z",upstreams:[],servers:[],streams:[],diff:{servers:[],streams:[],upstreams:[],hasChanges:false},isFirst:true,canRollback:false};
  mocks.read.mockResolvedValue(view);let html=renderToStaticMarkup(await HistoryDetail({params:Promise.resolve({id:"snap"})}));expect(html).toContain("최초 스냅샷");expect(html).not.toContain("이 스냅샷으로 롤백");
  mocks.read.mockResolvedValue({...view,isFirst:false,canRollback:true});html=renderToStaticMarkup(await HistoryDetail({params:Promise.resolve({id:"snap"})}));expect(html).toContain("이전 스냅샷과 동일");expect(html).toContain("이 스냅샷으로 롤백");
});
