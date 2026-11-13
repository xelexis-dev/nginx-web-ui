import { expect, it, vi } from "vitest";
import { isValidElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ServerDetail } from "./server-detail";
import { UpstreamDetail } from "@/components/upstreams/upstream-detail";
import { StreamDetail } from "@/components/streams/stream-detail";
import { WebSocketPresetControl, WebSocketPresetFeedback } from "./websocket-preset-control";
import { emptyDirectiveState } from "@repo/presentation/nginx/directive-state";
import { applyWebSocketPreset } from "@repo/presentation/nginx/websocket-preset";
const nodePage = vi.hoisted(() => ({ load: vi.fn() }));
vi.mock("@/lib/node-page", () => ({ loadNodePageAll: nodePage.load }));
vi.mock("./server-form", () => ({ ServerForm: () => null }));
import NewNodeServerPage from "@/app/(node)/nodes/[id]/reverse-proxy/new/page";
import EditNodeServerPage from "@/app/(node)/nodes/[id]/reverse-proxy/[sid]/edit/page";

const unavailable = { available: false, reason: "이 노드의 5354 포트가 사용 중입니다." };
const base = { id: "s", serverName: "app.test", listen: 80, enabled: true, upstreamId: null, certificateId: null, upstreams: [], cert: null, selfProxyNotice: false };

it.each([
  ["new", () => NewNodeServerPage({ params: Promise.resolve({ id: "target-node" }) })],
  ["edit", () => EditNodeServerPage({ params: Promise.resolve({ id: "target-node", sid: "server" }) })],
] as const)("node %s forwards the target node's name to the shared form", async (_, page) => {
  nodePage.load.mockResolvedValue({ ok: true, node: { id: "target-node", name: "서울 원격 노드" }, effectiveRole: "editor", data: {
    upstreams: [], certificates: [], values: { id: "server", serverName: "app.test", listen: 80, upstreamId: null, proxyPass: "http://origin.test" },
  } });
  const element = await page();
  if (!isValidElement<{ children: unknown[] }>(element)) throw Error("expected node page");
  const form = element.props.children.at(-1);
  if (!isValidElement<{ nodeName?: string }>(form)) throw Error("expected shared form");
  expect(form.props.nodeName).toBe("서울 원격 노드");
});

it.each(["local", "node"])("shows the saved mode and target-instance unavailability on %s resource detail pages", scope => {
  const root = scope === "node" ? "/nodes/n" : "";
  const server = renderToStaticMarkup(<ServerDetail basePath={`${root}/reverse-proxy`} configOrigin={scope === "node" ? "remote-node" : "this-instance"} write={null} runtimeDns={unavailable} data={{ ...base, config: { proxyPass: "http://px-web:3001", resolveAtRuntime: true } }} />);
  const upstream = renderToStaticMarkup(<UpstreamDetail basePath={`${root}/reverse-proxy/upstreams`} canWrite={false} deleteAction={{ id: "upstreams.deleteUpstream", args: [] }} runtimeDns={unavailable} data={{ id: "u", name: "pool", servers: ["px-web:3001"], resolveAtRuntime: true }} />);
  const stream = renderToStaticMarkup(<StreamDetail basePath={`${root}/stream`} canWrite={false} toggleAction={{ id: "stream.toggleStream", args: [] }} deleteAction={{ id: "stream.deleteStream", args: [] }} runtimeDns={unavailable} data={{ id: "l", listen: 15555, protocol: "tcp", enabled: true, proxyPass: "px-web:3001", resolveAtRuntime: true }} />);
  for (const html of [server, upstream, stream]) { expect(html).toContain("실행 중에 이름 확인"); expect(html).toContain(unavailable.reason); }
});

it("named server/location references display the selected group's DNS mode rather than their own flag", () => {
  const group = { id: "u", name: "pool", servers: ["px-web:3001"], resolveAtRuntime: false };
  const html = renderToStaticMarkup(<ServerDetail basePath="/reverse-proxy" configOrigin="this-instance" write={null} runtimeDns={{ available: true, reason: null }} data={{ ...base, upstreamId: "u", upstreams: [group], config: { resolveAtRuntime: true, locations: [{ path: "/group", proxyPass: "http://pool", resolveAtRuntime: true }] } }} />);
  expect(html).toContain("Upstream 설정 따름"); expect(html).toContain("정적 (설정 적용 시)");
});

it("canonical ID routes inherit group DNS and never fall through to a stale direct flag", () => {
  const group = { id: "u", name: "renamed_pool", servers: ["px-web:3001"], resolveAtRuntime: false };
  const html = renderToStaticMarkup(<ServerDetail basePath="/nodes/n/reverse-proxy" configOrigin="remote-node" write={null} runtimeDns={unavailable} data={{ ...base, upstreams: [group], config: { locations: [
    { path: "/id", upstreamId: "u", upstreamName: "renamed_pool", resolveAtRuntime: true },
  ] } }} />);
  expect(html).toContain("Upstream 설정 따름");
  expect(html).toContain("정적 (설정 적용 시)");
  expect(html).toMatch(/<td[^>]*>Upstream 설정 따름 · 정적 \(설정 적용 시\)<\/td>/);
});

it("WebSocket control is a non-submit action and conflict feedback requires an explicit choice", () => {
  const state = emptyDirectiveState(); state.headers = [{ name: "Connection", value: "upgrade" }];
  const before = structuredClone(state);
  const control = renderToStaticMarkup(<WebSocketPresetControl value={state} onChange={() => { throw Error("unexpected change during render"); }} />);
  expect(control).toMatch(/<button[^>]*type="button"[^>]*>WebSocket 헤더 채우기<\/button>/);
  const html = renderToStaticMarkup(<WebSocketPresetFeedback result={applyWebSocketPreset(state)} onReplace={() => { throw Error("unexpected replacement during render"); }} onDismiss={() => {}} />);
  expect(html).toContain("기존 WebSocket 설정과 충돌합니다"); expect(html).toContain("충돌한 설정 교체"); expect(html).toContain("기존 설정 유지");
  expect(state).toEqual(before);
});
