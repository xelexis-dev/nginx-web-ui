import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup as renderMarkup } from "react-dom/server";
import { ToastProvider } from "@/components/notifications/toast-provider";
import type { ReactNode } from "react";
const renderToStaticMarkup = (node: ReactNode) => renderMarkup(<ToastProvider>{node}</ToastProvider>);
const mock = vi.hoisted(() => ({ read: vi.fn() }));
vi.mock("@/lib/api-page", () => ({ readPageView: mock.read }));
vi.mock("react", async original => ({ ...await original<typeof import("react")>(), useActionState: () => [{ ok: false }, () => {}, false] }));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }) }));
import NewServer from "@/app/(node)/nodes/[id]/reverse-proxy/new/page";
import EditServer from "@/app/(node)/nodes/[id]/reverse-proxy/[sid]/edit/page";
import ServerDetail from "@/app/(node)/nodes/[id]/reverse-proxy/[sid]/page";
import NewUpstream from "@/app/(node)/nodes/[id]/reverse-proxy/upstreams/new/page";
import EditUpstream from "@/app/(node)/nodes/[id]/reverse-proxy/upstreams/[upstreamId]/edit/page";
import UpstreamDetail from "@/app/(node)/nodes/[id]/reverse-proxy/upstreams/[upstreamId]/page";
import NewStream from "@/app/(node)/nodes/[id]/stream/new/page";
import EditStream from "@/app/(node)/nodes/[id]/stream/[streamId]/edit/page";
import StreamDetail from "@/app/(node)/nodes/[id]/stream/[streamId]/page";

const upstream = { id: "u", name: "pool", servers: ["px-web:3001"], resolveAtRuntime: false };
const stream = { id: "l", listen: 15555, protocol: "tcp", proxyPass: "px-web:3001", enabled: true, resolveAtRuntime: false };
const server = { id: "s", serverName: "app.test", listen: 80, enabled: true, upstreamId: null, certificateId: null, config: { proxyPass: "http://px-web:3001", resolveAtRuntime: false } };
const grouped = { server: { server }, upstreams: { upstreams: [upstream] }, certificates: { certificates: [] } };
const cases = [
  ["server new", "/reverse-proxy/new", () => NewServer({ params: Promise.resolve({ id: "n" }) }), grouped],
  ["server edit", "/reverse-proxy/s/edit", () => EditServer({ params: Promise.resolve({ id: "n", sid: "s" }) }), grouped],
  ["server detail", "/reverse-proxy/s", () => ServerDetail({ params: Promise.resolve({ id: "n", sid: "s" }) }), grouped],
  ["upstream new", "/reverse-proxy/upstreams/new", () => NewUpstream({ params: Promise.resolve({ id: "n" }) }), null],
  ["upstream edit", "/reverse-proxy/upstreams/u/edit", () => EditUpstream({ params: Promise.resolve({ id: "n", upstreamId: "u" }) }), { upstream }],
  ["upstream detail", "/reverse-proxy/upstreams/u", () => UpstreamDetail({ params: Promise.resolve({ id: "n", upstreamId: "u" }) }), { upstream }],
  ["stream new", "/stream/new", () => NewStream({ params: Promise.resolve({ id: "n" }) }), null],
  ["stream edit", "/stream/l/edit", () => EditStream({ params: Promise.resolve({ id: "n", streamId: "l" }) }), { stream }],
  ["stream detail", "/stream/l", () => StreamDetail({ params: Promise.resolve({ id: "n", streamId: "l" }) }), { stream }],
] as const;
beforeEach(() => mock.read.mockReset());
it.each(cases)("%s uses only the target status from its single gateway view", async (_, route, page, data) => {
  for (const runtimeDns of [{ available: false, reason: "대상 노드 DNS 포트 충돌" }, { available: true, reason: null }, undefined]) {
    mock.read.mockClear();
    mock.read.mockResolvedValue({ ok: true, node: { id: "n", name: "Node" }, effectiveRole: "editor", data, ...(runtimeDns ? { runtimeDns } : {}) });
    const html = renderToStaticMarkup(await page());
    if (runtimeDns?.available === false) expect(html).toContain(runtimeDns.reason);
    else if (!runtimeDns) expect(html).toContain("확인할 수 없");
    else {
      expect(html).not.toContain("확인할 수 없");
      expect(html).not.toContain("대상 노드 DNS 포트 충돌");
    }
    expect(mock.read).toHaveBeenCalledExactlyOnceWith(`/nodes/n${route}`, {});
  }
});
