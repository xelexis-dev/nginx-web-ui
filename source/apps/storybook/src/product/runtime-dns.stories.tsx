// Real form components, in the existing operator shell. API actions use the Storybook stand-in.
import type { Meta, StoryObj } from "@storybook/react-vite";
import { PageHeader } from "@repo/ui";
import { seededLocationDirectiveState } from "@repo/presentation/nginx/directive-state";
import { ToastProvider, ToastViewport } from "@/components/notifications/toast-provider";
import { newLocationInput } from "@/components/reverse-proxy/server-form-vm";
import { ServerForm } from "@/components/reverse-proxy/server-form";
import { UpstreamForm } from "@/components/upstreams/upstream-form";
import { StreamForm } from "@/components/streams/stream-form";
import layout from "@/components/page-layout.module.css";
import { productFrame, productViewports } from "./frame";

function RuntimeDnsForm({ kind, availability, enabled, conflict, node }: {
  kind: "server" | "upstream" | "stream";
  availability: "ready" | "unavailable" | "unknown";
  enabled: boolean;
  conflict: boolean;
  node: boolean;
}) {
  const runtimeDns = availability === "unknown" ? undefined : { available: availability === "ready", reason: availability === "ready" ? null : "5354 포트가 기존 Stream과 겹칩니다. 운영자가 포트를 확인한 뒤 다시 시작하세요." };
  const directives = seededLocationDirectiveState();
  if (conflict) { directives.httpVersion = "1.0"; directives.headers.push({ name: "Connection", value: "keep-alive" }); }
  return <section className={layout.page}>
    <PageHeader title={kind === "server" ? "서버 수정" : kind === "upstream" ? "Upstream 수정" : "Stream 수정"} />
    {kind === "server" ? <ServerForm nodeName={node ? "미리보기 노드" : undefined} action={{ id: "reverse-proxy.updateServer", args: [] }} cancelHref="/reverse-proxy/example" certificates={[]} upstreams={[{ id: "u", name: "shared-pool", resolveAtRuntime: true }]} runtimeDns={runtimeDns} defaultValues={{ id: "example", serverName: "app.example.test", listen: 80, upstreamId: null, proxyPass: null, locations: [{ ...newLocationInput(), mode: "direct", proxyPass: "http://px-web:3001", resolveAtRuntime: enabled, directives }] }} /> : null}
    {kind === "upstream" ? <UpstreamForm action={{ id: "upstreams.updateUpstream", args: [] }} cancelHref="/reverse-proxy/upstreams/pool" runtimeDns={runtimeDns} defaultValues={{ id: "pool", name: "shared-pool", servers: ["px-web:3001", "standby:3001"], resolveAtRuntime: enabled }} /> : null}
    {kind === "stream" ? <StreamForm action={{ id: "stream.updateStream", args: [] }} cancelHref="/stream/example" runtimeDns={runtimeDns} defaultValues={{ id: "example", listen: 15555, protocol: "tcp", proxyPass: "px-web:3001", resolveAtRuntime: enabled }} /> : null}
  </section>;
}
const meta = {
  id: "product-runtime-dns", title: "Product/프록시 실행 중 이름 확인", component: RuntimeDnsForm, tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [(Story) => <ToastProvider><ToastViewport /><Story /></ToastProvider>, productFrame(args => ({ path: args.node ? "/nodes/preview-node/reverse-proxy/example/edit" : "/reverse-proxy/example/edit", label: "대상 이름 확인", role: "editor" }))],
  args: { kind: "server", availability: "ready", enabled: true, conflict: false, node: false },
} satisfies Meta<typeof RuntimeDnsForm>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Direct: Story = { name: "직접 대상 · 사용 가능" };
export const UnavailableSaved: Story = { name: "사용 불가 · 기존 선택 유지", args: { availability: "unavailable" } };
export const UnavailableNew: Story = { name: "사용 불가 · 새 활성화 차단", args: { availability: "unavailable", enabled: false } };
export const Unknown: Story = { name: "상태 없음", args: { availability: "unknown", enabled: false } };
export const Upstream: Story = { name: "Upstream", args: { kind: "upstream" } };
export const Stream: Story = { name: "Stream", args: { kind: "stream" } };
export const WebSocketConflict: Story = { name: "WebSocket 충돌 · 채우기 버튼으로 확인", args: { conflict: true } };
export const Narrow: Story = { name: "직접 대상 · 390", globals: { viewport: { value: "p390", isRotated: false } } };

export const NodeUnavailable: Story = { name: "노드 · 실행 중 이름 확인 불가", args: { node: true, availability: "unavailable" } };
export const NodeUnknown: Story = { name: "노드 · 상태 확인할 수 없음", args: { node: true, availability: "unknown" } };
