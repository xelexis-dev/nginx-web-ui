import type { Meta, StoryObj } from "@storybook/react-vite";
import type { NodeView } from "@/lib/node-page";
import type { NodeFailure } from "@/lib/node-result";
import { NodeList } from "@/app/(app)/nodes/node-list";
import { NodeNewView } from "@/app/(app)/nodes/node-new-view";
import { NodeOverview, type NodeOverviewView } from "@/app/(node)/nodes/[id]/node-overview";
import { NodeEditView } from "@/app/(node)/nodes/[id]/edit/node-edit-view";
import { ProductFrame } from "./frame";

const node: NodeView = { id: "wp3f-node", name: "서울 운영 노드", baseUrl: "https://node.example.test", tokenRole: "admin", allowInsecureTls: false,
  instanceId: "fixture-instance", gatewayName: "운영 게이트웨이", createdByUserId: "fixture-admin", createdAt: "2026-10-01T00:00:00Z", lastSeenAt: "2026-11-11T09:00:00Z", lastStatus: "ok", lastVersion: "2.0.0", lastError: null };
const status: Extract<NodeOverviewView, { ok: true }>["data"] = { semver: "2.0.0", counts: { servers: 7, streams: 3, upstreams: 5 },
  pending: { count: 2, hasBaseline: true }, certificates: { total: 11, expiringSoon: 1 }, nginx: { configOk: true, output: "nginx: configuration file syntax is ok\nnginx: configuration file test is successful" } };
const failures: Record<string, NodeFailure> = {
  unreachable: { kind: "unreachable", message: "노드에 연결할 수 없습니다." },
  unauthorized: { kind: "unauthorized", message: "연결 토큰이 만료되었습니다." },
  forbidden: { kind: "forbidden", message: "권한이 없습니다." },
  mismatch: { kind: "version_mismatch", message: "버전 불일치", nodeVersion: "3.0.0" },
  old: { kind: "too_old", message: "이 노드는 노드 기능을 지원하지 않습니다." },
  missing: { kind: "not_found", message: "항목이 없습니다." },
  error: { kind: "node_error", message: "노드 응답 오류" },
  conflict: { kind: "conflict", message: "다른 작업이 진행 중입니다." },
  limited: { kind: "rate_limited", message: "잠시 뒤 다시 시도하세요." },
  domain: { kind: "domain_failure", message: "nginx: configuration file test failed" },
};
type Props = { mode: "list" | "new" | "overview" | "edit"; viewer: boolean; empty: boolean; failure: string; unknownBaseline: boolean; invalidConfig: boolean; legacy: boolean };
function path(args: Props) { return args.mode === "list" ? "/nodes" : args.mode === "new" ? "/nodes/new" : `/nodes/${node.id}${args.mode === "edit" ? "/edit" : ""}`; }
function NodePages(args: Props) {
  if (args.mode === "new") return <NodeNewView defaultGatewayName="운영 게이트웨이" />;
  if (args.mode === "edit") return <NodeEditView node={{ ...node, gatewayName: args.legacy ? null : node.gatewayName }} />;
  if (args.mode === "overview") return <NodeOverview id={node.id} result={args.failure ? { ok: false, node, canManage: !args.viewer,
    nodeError: { nodeId: node.id, retryHref: `/nodes/${node.id}`, listHref: "/nodes", failure: failures[args.failure]! } }
    : { ok: true, node, canManage: !args.viewer, data: { ...status, pending: { ...status.pending, hasBaseline: !args.unknownBaseline }, nginx: { ...status.nginx, configOk: !args.invalidConfig } } }} />;
  const rows: NodeView[] = [node, ...(["unreachable", "unauthorized", "version_mismatch", "error", null] as const).map((lastStatus, i) => ({ ...node,
    id: `wp3f-state-${i}`, name: `${lastStatus ?? "신규"} 노드`, lastStatus, lastSeenAt: i % 2 ? node.lastSeenAt : null, lastVersion: lastStatus ? "2.0.0" : null }))];
  return <NodeList nodes={args.empty ? [] : rows} canWrite={!args.viewer} unpairWarningText={args.failure ? "노드의 토큰을 해지하지 못했습니다." : null} />;
}
const meta = { id: "product-wp3f", title: "Product/WP-3F", component: NodePages, tags: ["!autodocs"], parameters: { layout: "fullscreen" },
  decorators: [(Story, context) => <ProductFrame path={path(context.args as Props)} label="노드" role={context.args.viewer ? "viewer" : "admin"}
    storyId={context.id} viewport={undefined} nodeName={node.name} available={!context.args.failure}><Story /></ProductFrame>],
  args: { mode: "list", viewer: false, empty: false, failure: "", unknownBaseline: false, invalidConfig: false, legacy: false },
} satisfies Meta<typeof NodePages>;
export default meta;
type Story = StoryObj<typeof meta>;
export const List: Story = {};
export const ListViewer: Story = { args: { viewer: true } };
export const Empty: Story = { args: { empty: true } };
export const UnpairWarning: Story = { args: { failure: "warning" } };
export const Pair: Story = { args: { mode: "new" } };
export const Edit: Story = { args: { mode: "edit" } };
export const EditLegacy: Story = { args: { mode: "edit", legacy: true } };
export const Overview: Story = { args: { mode: "overview" } };
export const OverviewViewer: Story = { args: { mode: "overview", viewer: true } };
export const UnknownBaseline: Story = { args: { mode: "overview", unknownBaseline: true } };
export const InvalidConfig: Story = { args: { mode: "overview", invalidConfig: true } };
export const Unreachable: Story = { args: { mode: "overview", failure: "unreachable" } };
export const Unauthorized: Story = { args: { mode: "overview", failure: "unauthorized" } };
export const Forbidden: Story = { args: { mode: "overview", failure: "forbidden", viewer: true } };
export const Mismatch: Story = { args: { mode: "overview", failure: "mismatch" } };
export const TooOld: Story = { args: { mode: "overview", failure: "old" } };
export const Missing: Story = { args: { mode: "overview", failure: "missing" } };
export const NodeError: Story = { args: { mode: "overview", failure: "error" } };
export const Conflict: Story = { args: { mode: "overview", failure: "conflict" } };
export const RateLimited: Story = { args: { mode: "overview", failure: "limited" } };
export const DomainFailure: Story = { args: { mode: "overview", failure: "domain" } };
