import { expect, it, vi } from "vitest";
import { renderToStaticMarkup as renderMarkup } from "react-dom/server";
import { ToastProvider } from "@/components/notifications/toast-provider";
import type { ReactNode } from "react";
const renderToStaticMarkup=(node:ReactNode)=>renderMarkup(<ToastProvider>{node}</ToastProvider>);
import { emptyDirectiveState } from "@repo/presentation/nginx/directive-state";
import { ServerForm } from "./server-form";
import { UpstreamForm } from "@/components/upstreams/upstream-form";
import { StreamForm } from "@/components/streams/stream-form";

vi.mock("react", async original => ({ ...await original<typeof import("react")>(), useActionState: () => [{ ok: false }, () => {}, false] }));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }) }));
const status = { available: true, reason: null };
const defaults = { id: "s", serverName: "app.test", listen: 80, upstreamId: null, proxyPass: "http://px-web:3001", resolveAtRuntime: true };
const hidden = (html: string, name: string) => {
  const value = new RegExp(`name="${name}"[^>]*value="([^"]*)"`).exec(html)?.[1];
  return value?.replaceAll("&quot;", '"').replaceAll("&amp;", "&");
};

it("submits explicit boolean DNS values for both new and existing upstream/stream forms", () => {
  for (const enabled of [false, true]) {
    const upstream = renderToStaticMarkup(<UpstreamForm action={{ id: "upstreams.updateUpstream", args: [] }} cancelHref="/reverse-proxy/upstreams" runtimeDns={status} defaultValues={{ id: "u", name: "pool", servers: ["px-web:3001"], resolveAtRuntime: enabled }} />);
    const stream = renderToStaticMarkup(<StreamForm action={{ id: "stream.updateStream", args: [] }} cancelHref="/stream" runtimeDns={status} defaultValues={{ id: "l", listen: 15555, protocol: "tcp", proxyPass: "px-web:3001", resolveAtRuntime: enabled }} />);
    expect(hidden(upstream, "resolveAtRuntime")).toBe(String(enabled)); expect(hidden(stream, "resolveAtRuntime")).toBe(String(enabled));
  }
  const fresh = renderToStaticMarkup(<StreamForm action={{ id: "stream.createStream", args: [] }} cancelHref="/stream" runtimeDns={status} />);
  expect(hidden(fresh, "resolveAtRuntime")).toBe("false");
});

it("submits direct server DNS and suppresses a per-server override of a named group", () => {
  const direct = renderToStaticMarkup(<ServerForm action={{ id: "reverse-proxy.updateServer", args: [] }} cancelHref="/reverse-proxy" certificates={[]} upstreams={[]} runtimeDns={status} defaultValues={defaults} />);
  expect(JSON.parse(hidden(direct, "locations")!)[0].resolveAtRuntime).toBe(true); expect(direct).toContain("WebSocket 헤더 채우기");
  const group = renderToStaticMarkup(<ServerForm action={{ id: "reverse-proxy.updateServer", args: [] }} cancelHref="/reverse-proxy" certificates={[]} upstreams={[{ id: "u", name: "pool", resolveAtRuntime: true }]} runtimeDns={status} defaultValues={{ ...defaults, upstreamId: "u", proxyPass: null }} />);
  expect(JSON.parse(hidden(group, "locations")!)[0].resolveAtRuntime).toBeUndefined(); expect(group).toContain("Upstream 설정을 따릅니다");
});

it("round-trips location flags but never sends a named-reference DNS override", () => {
  const html = renderToStaticMarkup(<ServerForm action={{ id: "reverse-proxy.updateServer", args: [] }} cancelHref="/reverse-proxy" certificates={[]} upstreams={[{ id: "u", name: "pool", resolveAtRuntime: true }]} runtimeDns={status} defaultValues={{ ...defaults, locations: [
    { path: "/direct", mode: "direct", upstreamId: "", proxyPass: "http://px-web:3001", resolveAtRuntime: true, directives: emptyDirectiveState() },
    { path: "/group", mode: "upstream", upstreamId: "u", proxyPass: "", resolveAtRuntime: true, directives: emptyDirectiveState() },
  ] }} />);
  const locations = JSON.parse(hidden(html, "locations")!);
  expect(locations.map((location: { resolveAtRuntime: boolean }) => location.resolveAtRuntime)).toEqual([true, undefined]);
  expect(hidden(html, "resolveAtRuntime")).toBeUndefined();
});

it("never claims UDP is unrestricted now that the DNS service reserves TCP and UDP", () => {
  const html = renderToStaticMarkup(<StreamForm action={{ id: "stream.createStream", args: [] }} cancelHref="/stream" defaultValues={{ id: "l", listen: 15555, protocol: "udp", proxyPass: "backend:53" }} />);
  expect(html).not.toContain("UDP 는 겹치지 않아 제한이 없습니다"); expect(html).toContain("UDP"); expect(html).toContain("이름 확인 서비스");
});
