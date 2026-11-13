// Actual local/node resource forms; the failed action is synthetic and never sends a request.
import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import Link from "next/link";
import { Button, PageHeader, buttonClassName } from "@repo/ui";
import { ToastProvider, ToastViewport } from "@/components/notifications/toast-provider";
import { ServerForm } from "@/components/reverse-proxy/server-form";
import { UpstreamForm } from "@/components/upstreams/upstream-form";
import { StreamForm } from "@/components/streams/stream-form";
import type { NodeFormAction } from "@/lib/node-action-state";
import layout from "@/components/page-layout.module.css";
import { productFrame } from "./frame";

const message = "다른 사용자가 이 설정을 변경했습니다. 입력한 내용을 복사해 둔 뒤 페이지를 새로고침하고 다시 저장하세요.";
function ConflictForm({ kind, node, legacy, refreshed }: { kind: "server" | "upstream" | "stream"; node: boolean; legacy: boolean; refreshed: boolean }) {
  const [version, setVersion] = useState(refreshed ? "9007199254740994" : "9007199254740993");
  const base = node ? "/nodes/preview-node" : "";
  const cancelHref = `${base}/${kind === "server" ? "reverse-proxy" : kind === "upstream" ? "reverse-proxy/upstreams" : "stream"}/example`;
  const action: NodeFormAction = async (_state, form) => {
    window.dispatchEvent(new CustomEvent("nwui-preview-conflict", { detail: Object.fromEntries(form) }));
    const status = !legacy && form.get("version") === "9007199254740994" ? 200 : 409;
    window.dispatchEvent(new CustomEvent("nwui-preview-status", { detail: status }));
    if (status === 200) return { ok: false, error: "새 버전 저장 성공 (검증용)" };
    return { ok: false, error: message, ...(node ? { nodeError: {
      nodeId: "preview-node", retryHref: `${cancelHref}/edit`, listHref: base,
      failure: { kind: "conflict" as const, message },
    } } : {}) };
  };
  const target = node ? { kind: "node" as const, id: "preview-node", name: "미리보기 노드" } : { kind: "local" as const };
  const revision = legacy ? {} : { version };
  const runtimeDns = { available: true, reason: null };
  return (
    <section className={layout.page}>
      <PageHeader title={kind === "server" ? "서버 수정" : kind === "upstream" ? "Upstream 수정" : "Stream 수정"} actions={<Link href={cancelHref} className={buttonClassName({ variant: "secondary" })}>취소</Link>} />
      <Button type="button" variant="secondary" onClick={() => setVersion("9007199254740994")}>서버 데이터 새로고침 (검증용)</Button>
      {kind === "server" ? <ServerForm target={target} nodeName={node ? "미리보기 노드" : undefined} action={action} cancelHref={cancelHref} certificates={[]} upstreams={[]} runtimeDns={runtimeDns}
        defaultValues={{ id: "example", ...revision, serverName: "app.example.test", listen: 80, upstreamId: null, proxyPass: "http://origin.example.test:9000" }} /> : null}
      {kind === "upstream" ? <UpstreamForm target={target} action={action} cancelHref={cancelHref} runtimeDns={runtimeDns}
        defaultValues={{ id: "example", ...revision, name: "backend", servers: ["origin.example.test:9000"] }} /> : null}
      {kind === "stream" ? <StreamForm target={target} action={action} cancelHref={cancelHref} runtimeDns={runtimeDns}
        defaultValues={{ id: "example", ...revision, listen: 19000, protocol: "tcp", proxyPass: "origin.example.test:9000" }} /> : null}
    </section>
  );
}
const meta = {
  id: "product-audit-concurrency", title: "Product/동시 수정 충돌", component: ConflictForm, tags: ["!autodocs"],
  parameters: { layout: "fullscreen" },
  decorators: [(Story) => <ToastProvider><ToastViewport /><Story /></ToastProvider>, productFrame(args => ({ path: `${args.node ? "/nodes/preview-node" : ""}/${args.kind === "upstream" ? "reverse-proxy/upstreams" : args.kind === "stream" ? "stream" : "reverse-proxy"}/example/edit`, label: "설정 수정", role: "editor" }))],
  args: { kind: "server", node: false, legacy: false, refreshed: false },
} satisfies Meta<typeof ConflictForm>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Server: Story = {};
export const Upstream: Story = { args: { kind: "upstream" } };
export const Stream: Story = { args: { kind: "stream" } };
export const NodeServer: Story = { args: { node: true } };
export const NodeUpstream: Story = { args: { kind: "upstream", node: true } };
export const NodeStream: Story = { args: { kind: "stream", node: true } };
export const LegacyNodeServer: Story = { args: { node: true, legacy: true } };
export const LegacyNodeUpstream: Story = { args: { kind: "upstream", node: true, legacy: true } };
export const LegacyNodeStream: Story = { args: { kind: "stream", node: true, legacy: true } };
