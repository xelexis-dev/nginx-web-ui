import { expect, userEvent, waitFor, within } from "storybook/test";
import type { NodeFormAction } from "@/lib/node-action-state";
import type { Meta, StoryObj } from "@storybook/react-vite";
import Link from "next/link";
import { PageHeader, SectionNav } from "@repo/ui";
import { PROXY_TABS } from "@/app/(app)/nav-links";
import { SERVER_TEMPLATES } from "@/app/(app)/reverse-proxy/server-templates";
import { ServerList } from "@/components/reverse-proxy/server-list";
import { ServerDetail } from "@/components/reverse-proxy/server-detail";
import { ServerTemplates } from "@/components/reverse-proxy/server-templates";
import { UpstreamList } from "@/components/upstreams/upstream-list";
import { UpstreamDetail } from "@/components/upstreams/upstream-detail";
import { UpstreamForm } from "@/components/upstreams/upstream-form";
import { StreamList } from "@/components/streams/stream-list";
import { StreamDetail } from "@/components/streams/stream-detail";
import { StreamForm } from "@/components/streams/stream-form";
import layout from "@/components/page-layout.module.css";
import { SERVER_LIST_LOCATIONS, SERVER_DETAIL } from "./fixtures";
import { productFrame } from "./frame";

type Props = { kind: "proxy" | "upstream" | "stream" | "templates"; mode: "list" | "detail" | "new" | "edit"; node: boolean; viewer: boolean; empty: boolean; failure: boolean };
const FAILURE = "이 upstream을 사용하는 location이 있습니다. 해당 경로를 먼저 변경한 뒤 이름 변경·삭제를 다시 시도하세요.";
const nodeTarget = { kind: "node", id: "node-a", name: "미리보기 노드" } as const;
const runtimeDns = { available: true, reason: null };
const upstream = { id: "api-pool", name: "api_pool", servers: ["api-1.example.test:8080", "api-2.example.test:8080"], version: "1", resolveAtRuntime: true };
const stream = { id: "database", listen: 15432, protocol: "tcp" as const, proxyPass: "database.example.test:5432", enabled: true, version: "2", resolveAtRuntime: true,
  preview: { config: "server {\n    listen 15432;\n    proxy_pass database.example.test:5432;\n}\n", error: null } };
function base({ kind, node }: Props) { return `${node ? "/nodes/node-a" : ""}${kind === "stream" ? "/stream" : kind === "upstream" ? "/reverse-proxy/upstreams" : kind === "templates" ? "/reverse-proxy/templates" : "/reverse-proxy"}`; }
function path(args: Props) { const b = base(args); const id = args.kind === "upstream" ? upstream.id : args.kind === "stream" ? stream.id : SERVER_DETAIL.id; return args.mode === "list" ? b : args.mode === "new" ? `${b}/new` : `${b}/${id}${args.mode === "edit" ? "/edit" : ""}`; }
function ProxyPages(args: Props) {
  const { kind, mode, node, viewer, empty } = args;
  const b = base(args);
  const action = (local: string, remote: string): NodeFormAction => args.failure
    ? async () => node ? { ok: false, nodeError: { nodeId: nodeTarget.id, retryHref: path(args), listHref: b, failure: { kind: "conflict", message: FAILURE } } } : { ok: false, error: FAILURE }
    : { id: node ? `node.${remote}` : local, args: node ? [nodeTarget.id] : [] };
  const navigation = kind !== "stream" ? <SectionNav items={PROXY_TABS.filter(item => !node || item.href !== "/reverse-proxy/templates").map(item => ({ ...item, href: `${node ? "/nodes/node-a" : ""}${item.href}` }))} currentPath={b} label="프록시 구성" renderLink={link => <Link {...link} />} /> : undefined;
  if (kind === "templates") return <ServerTemplates canWrite={!viewer} navigation={navigation} items={SERVER_TEMPLATES.map(item => ({ ...item, preview: { config: "server {\n    listen 80;\n}\n", error: null } }))} />;
  if (mode === "list") {
    if (kind === "proxy") return <ServerList basePath={b} navigation={navigation} data={{ ...SERVER_LIST_LOCATIONS, items: empty ? [] : SERVER_LIST_LOCATIONS.items }} write={viewer ? null : { newHref: `${b}/new`, templatesHref: node ? null : `${b}/templates`, toggleAction: action("reverse-proxy.toggleServer", "toggleNodeServer") }} />;
    if (kind === "upstream") return <UpstreamList basePath={b} navigation={navigation} data={{ items: empty ? [] : [upstream], canWrite: !viewer }} />;
    return <StreamList basePath={b} data={{ items: empty ? [] : [stream, { ...stream, id: "dns", listen: 15353, protocol: "udp", enabled: false }], canWrite: !viewer }} />;
  }
  if (mode === "detail") {
    if (kind === "proxy") return <ServerDetail basePath={b} data={SERVER_DETAIL} runtimeDns={runtimeDns} configOrigin={node ? "remote-node" : "this-instance"} write={viewer ? null : { editHref: `${b}/${SERVER_DETAIL.id}/edit`, toggleAction: action("reverse-proxy.toggleServer", "toggleNodeServer"), deleteAction: action("reverse-proxy.deleteServer", "deleteNodeServer") }} />;
    if (kind === "upstream") return <UpstreamDetail basePath={b} data={upstream} runtimeDns={runtimeDns} canWrite={!viewer} deleteAction={action("upstreams.deleteUpstream", "deleteNodeUpstream")} />;
    return <StreamDetail basePath={b} data={stream} runtimeDns={runtimeDns} canWrite={!viewer} toggleAction={action("stream.toggleStream", "toggleNodeStream")} deleteAction={action("stream.deleteStream", "deleteNodeStream")} />;
  }
  const edit = mode === "edit";
  const cancel = edit ? `${b}/${kind === "upstream" ? upstream.id : stream.id}` : b;
  return <section className={layout.page}>
    <PageHeader title={kind === "upstream" ? edit ? "Upstream 수정" : "새 Upstream" : edit ? "Stream 수정" : "새 Stream"} />
    {kind === "upstream" ? <UpstreamForm runtimeDns={runtimeDns} nodeVersionFamily={node ? "2.0" : undefined} target={node ? nodeTarget : undefined} defaultValues={edit ? upstream : undefined} cancelHref={cancel} action={action(`upstreams.${edit ? "update" : "create"}Upstream`, `${edit ? "update" : "create"}NodeUpstream`)} /> : <StreamForm runtimeDns={runtimeDns} nodeVersionFamily={node ? "2.0" : undefined} target={node ? nodeTarget : undefined} defaultValues={edit ? stream : undefined} cancelHref={cancel} action={action(`stream.${edit ? "update" : "create"}Stream`, `${edit ? "update" : "create"}NodeStream`)} reservedListenPorts={node ? undefined : [80, 3000, 3001]} reservedUdpListenPorts={node ? undefined : []} />}
  </section>;
}
const meta = { id: "product-wp3b", title: "Product/WP-3B", component: ProxyPages, tags: ["!autodocs"], parameters: { layout: "fullscreen" },
  decorators: [productFrame(args => ({ path: path(args as Props), label: "구성", role: args.viewer ? "viewer" : "editor" }))],
  args: { kind: "proxy", mode: "list", node: false, viewer: false, empty: false, failure: false },
} satisfies Meta<typeof ProxyPages>;
export default meta;
type Story = StoryObj<typeof meta>;
export const ProxyList: Story = {};
export const ProxyEmpty: Story = { args: { empty: true } };
export const ProxyDetail: Story = { args: { mode: "detail" } };
export const Templates: Story = { args: { kind: "templates" } };
export const UpstreamListPage: Story = { args: { kind: "upstream" } };
export const UpstreamDetailPage: Story = { args: { kind: "upstream", mode: "detail" } };
export const UpstreamNew: Story = { args: { kind: "upstream", mode: "new" } };
export const UpstreamEdit: Story = { args: { kind: "upstream", mode: "edit" } };
export const StreamListPage: Story = { args: { kind: "stream" } };
export const StreamDetailPage: Story = { args: { kind: "stream", mode: "detail" } };
export const StreamNew: Story = { args: { kind: "stream", mode: "new" } };
export const StreamEdit: Story = { args: { kind: "stream", mode: "edit" } };
export const ProxyViewer: Story = { args: { viewer: true } };
export const ProxyDetailViewer: Story = { args: { viewer: true, mode: "detail" } };
export const UpstreamViewer: Story = { args: { kind: "upstream", viewer: true } };
export const UpstreamDetailViewer: Story = { args: { kind: "upstream", viewer: true, mode: "detail" } };
export const StreamViewer: Story = { args: { kind: "stream", viewer: true } };
export const StreamDetailViewer: Story = { args: { kind: "stream", viewer: true, mode: "detail" } };
export const NodeProxy: Story = { args: { node: true } };
export const NodeProxyDetail: Story = { args: { node: true, mode: "detail" } };
export const NodeUpstream: Story = { args: { node: true, kind: "upstream" } };
export const NodeUpstreamDetail: Story = { args: { node: true, kind: "upstream", mode: "detail" } };
export const NodeUpstreamEdit: Story = { args: { node: true, kind: "upstream", mode: "edit" } };
export const NodeStream: Story = { args: { node: true, kind: "stream" } };
export const NodeStreamDetail: Story = { args: { node: true, kind: "stream", mode: "detail" } };
export const NodeStreamEdit: Story = { args: { node: true, kind: "stream", mode: "edit" } };

async function failDelete({ canvasElement }: { canvasElement: HTMLElement }) {
  const canvas = within(canvasElement);
  await userEvent.click(canvas.getByRole("button", { name: "삭제" }));
  const dialog = await canvas.findByRole("dialog");
  await userEvent.click(within(dialog).getByRole("button", { name: "삭제" }));
  await waitFor(() => expect(canvas.getByText(FAILURE)).toBeVisible());
}
export const UpstreamFailure: Story = { args: { kind: "upstream", mode: "detail", failure: true }, play: failDelete };
export const NodeUpstreamFailure: Story = { args: { kind: "upstream", mode: "detail", node: true, failure: true }, play: failDelete };
export const StreamFailure: Story = { args: { kind: "stream", mode: "detail", failure: true }, play: failDelete };
export const NodeStreamFailure: Story = { args: { kind: "stream", mode: "detail", node: true, failure: true }, play: failDelete };

export const NodeUpstreamNew: Story = { args: { node: true, kind: "upstream", mode: "new" } };
export const NodeStreamNew: Story = { args: { node: true, kind: "stream", mode: "new" } };
