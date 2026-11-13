import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup as renderMarkup } from "react-dom/server";
import { ToastProvider } from "@/components/notifications/toast-provider";
import type { ReactNode } from "react";
const renderToStaticMarkup=(node:ReactNode)=>renderMarkup(<ToastProvider>{node}</ToastProvider>);
const mocks = vi.hoisted(() => ({ read: vi.fn() }));
vi.mock("@/lib/api-page", () => ({ readPageView: mocks.read }));
vi.mock("react", async original => ({ ...await original<typeof import("react")>(), useActionState: () => [{ ok: false }, () => {}, false] }));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }) }));
import NewServer from "@/app/(app)/reverse-proxy/new/page";
import EditServer from "@/app/(app)/reverse-proxy/[id]/edit/page";
import ServerPage from "@/app/(app)/reverse-proxy/[id]/page";
import NewUpstream from "@/app/(app)/reverse-proxy/upstreams/new/page";
import EditUpstream from "@/app/(app)/reverse-proxy/upstreams/[id]/edit/page";
import UpstreamPage from "@/app/(app)/reverse-proxy/upstreams/[id]/page";
import NewStream from "@/app/(app)/stream/new/page";
import EditStream from "@/app/(app)/stream/[id]/edit/page";
import StreamPage from "@/app/(app)/stream/[id]/page";
const runtimeDns = { available: false, reason: "5354 포트에 기존 Stream이 있습니다." };
const upstream = { id: "u", name: "pool", servers: ["px-web:3001"], resolveAtRuntime: true };
const server = { id: "s", serverName: "app.test", listen: 80, enabled: true, upstreamId: null, certificateId: null, config: { proxyPass: "http://px-web:3001", resolveAtRuntime: true } };
const stream = { id: "l", listen: 15555, protocol: "tcp", enabled: true, proxyPass: "px-web:3001", resolveAtRuntime: true };
beforeEach(() => mocks.read.mockReset());

it.each([
  ["server new", () => NewServer({ searchParams: Promise.resolve({}) })],
  ["server edit", () => EditServer({ params: Promise.resolve({ id: "s" }) })],
  ["server detail", () => ServerPage({ params: Promise.resolve({ id: "s" }) })],
  ["upstream new", () => NewUpstream()],
  ["upstream edit", () => EditUpstream({ params: Promise.resolve({ id: "u" }) })],
  ["upstream detail", () => UpstreamPage({ params: Promise.resolve({ id: "u" }) })],
  ["stream new", () => NewStream()],
  ["stream edit", () => EditStream({ params: Promise.resolve({ id: "l" }) })],
  ["stream detail", () => StreamPage({ params: Promise.resolve({ id: "l" }) })],
] as const)("%s forwards the instance's runtime DNS status to the shared UI", async (_, page) => {
  mocks.read.mockResolvedValue({ runtimeDns, upstream, upstreams: [upstream], server, stream, certificates: [], canWrite: false, canApply: false, selfProxyNotice: false, preview: { config: "# preview", error: null }, reservedPorts: { process: [3000, 3001, 5432], http: [80] } });
  const html = renderToStaticMarkup(await page());
  expect(html).toContain(runtimeDns.reason);
});
