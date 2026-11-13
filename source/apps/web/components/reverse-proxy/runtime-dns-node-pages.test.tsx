import { expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
const raw = vi.hoisted(() => ({ payload: {} as unknown }));
vi.mock("@/lib/node-page", () => ({ loadNodePage: async (args: { toViewModel: (data: unknown) => unknown }) => ({ ok: true, effectiveRole: "editor", node: {}, data: args.toViewModel(raw.payload) }) }));
vi.mock("react", async original => ({ ...await original<typeof import("react")>(), useActionState: () => [{ ok: false }, () => {}, false] }));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }) }));
import EditUpstream from "@/app/(node)/nodes/[id]/reverse-proxy/upstreams/[upstreamId]/edit/page";
import UpstreamDetail from "@/app/(node)/nodes/[id]/reverse-proxy/upstreams/[upstreamId]/page";
import EditStream from "@/app/(node)/nodes/[id]/stream/[streamId]/edit/page";
import StreamDetail from "@/app/(node)/nodes/[id]/stream/[streamId]/page";

it("node upstream edit/detail preserve the node's saved runtime DNS flag", async () => {
  raw.payload = { upstream: { id: "u", name: "pool", servers: ["px-web:3001"], resolveAtRuntime: true } };
  const edit = renderToStaticMarkup(await EditUpstream({ params: Promise.resolve({ id: "n", upstreamId: "u" }) }));
  const detail = renderToStaticMarkup(await UpstreamDetail({ params: Promise.resolve({ id: "n", upstreamId: "u" }) }));
  expect(edit).toMatch(/name="resolveAtRuntime"[^>]*value="true"/); expect(detail).toContain("실행 중에 이름 확인");
});
it("node TCP/UDP stream edit/detail preserve the node's saved runtime DNS flag", async () => {
  for (const protocol of ["tcp", "udp"]) {
    raw.payload = { stream: { id: "l", listen: 15555, protocol, proxyPass: "px-web:3001", enabled: true, resolveAtRuntime: true } };
    const edit = renderToStaticMarkup(await EditStream({ params: Promise.resolve({ id: "n", streamId: "l" }) }));
    const detail = renderToStaticMarkup(await StreamDetail({ params: Promise.resolve({ id: "n", streamId: "l" }) }));
    expect(edit).toMatch(/name="resolveAtRuntime"[^>]*value="true"/); expect(detail).toContain("실행 중에 이름 확인");
  }
});
