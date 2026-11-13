import { expect, userEvent, within, waitFor } from "storybook/test";
import type { Meta, StoryObj } from "@storybook/react-vite";
import type { NodeView, HistoryDetailView } from "@repo/contracts";
import { PageHeader } from "@repo/ui";
import { HistoryList, type HistoryListVM } from "@/components/history/history-list";
import { HistoryDetail } from "@/components/history/history-detail";
import { AuditList, type AuditListVM } from "@/components/audit/audit-list";
import { AuditDetail, type AuditDetailVM } from "@/components/audit/audit-detail";
import { NodeList } from "@/app/(app)/nodes/node-list";
import { NodeNewView } from "@/app/(app)/nodes/node-new-view";
import { NodeOverview, type NodeOverviewView } from "@/app/(node)/nodes/[id]/node-overview";
import { NodeEditView } from "@/app/(node)/nodes/[id]/edit/node-edit-view";
import { NodeHistoryActions } from "@/app/(node)/nodes/[id]/history/rollback-button";
import { NodeErrorCard } from "@/components/node-error";
import { ProductFrame } from "./frame";
import layout from "@/components/page-layout.module.css";

const node: NodeView = { id: "p3-node", name: "서울 운영 노드", baseUrl: "https://node.example.test", tokenRole: "admin", allowInsecureTls: false, instanceId: "node-instance", gatewayName: "운영 게이트웨이", createdByUserId: "admin", createdAt: "2026-10-01T00:00:00Z", lastSeenAt: "2026-10-31T12:00:00Z", lastStatus: "ok", lastVersion: "2.0.0", lastError: null };
const history: HistoryListVM = { showDetails: true, items: [{ id: "snap-2", iso: "2026-10-31T10:00:00Z", label: "운영 리버스 프록시 설정 적용", actorEmail: "ops@example.test", upstreamCount: 2, serverCount: 3, streamCount: 1 }, { id: "snap-1", iso: "2026-10-30T10:00:00Z", label: null, actorEmail: null, upstreamCount: 1, serverCount: 2, streamCount: 0 }] };
const historyDetail: HistoryDetailView = { snapshot: { id: "snap-2", createdAt: history.items[0]!.iso, label: history.items[0]!.label, actorEmail: "ops@example.test", actorId: "admin", data: { servers: [], upstreams: [], streams: [] } }, iso: history.items[0]!.iso, upstreams: [], servers: [], streams: [], canRollback: true, isFirst: false, diff: { hasChanges: false, upstreams: [], servers: [], streams: [] } };
const audit: AuditListVM = { items: [{ id: "event-1", time: "2026. 10. 31. 19:00 KST", actorEmail: "ops@example.test", actorLabel: null, action: "nginx:apply", target: "로컬 서버" }], total: 1, page: 1, totalPages: 1, actions: ["nginx:apply"], actionFilter: "" };
const auditDetail: AuditDetailVM = { ...audit.items[0]!, ip: "127.0.0.1", beforeJson: '{"enabled":false}', afterJson: '{"enabled":true}' };
const status: Extract<NodeOverviewView, { ok: true }>["data"] = { semver: "2.0.0", counts: { servers: 3, streams: 1, upstreams: 2 }, pending: { count: 0, hasBaseline: true }, certificates: { total: 2, expiringSoon: 0 }, nginx: { configOk: true, output: "syntax is ok" } };
const paths = { history: "/history", historyDetail: "/history/snap-2", audit: "/audit", auditDetail: "/audit/event-1", nodes: "/nodes", nodeNew: "/nodes/new", node: "/nodes/p3-node", nodeEdit: "/nodes/p3-node/edit", nodeHistory: "/nodes/p3-node/history", nodeAudit: "/nodes/p3-node/audit", nodeAuditDetail: "/nodes/p3-node/audit/event-1" };
type Kind = keyof typeof paths;
function Pages({ kind, available = true, viewer = false }: { kind: Kind; available?: boolean; viewer?: boolean }) {
  const base = kind.startsWith("node") ? "/nodes/p3-node" : "";
  if (!available && ["nodeHistory", "nodeAudit", "nodeAuditDetail"].includes(kind)) return <section className={layout.page}><PageHeader title={kind === "nodeHistory" ? "History" : "감사 로그"} /><NodeErrorCard nodeId={node.id} retryHref={paths[kind]} listHref="/nodes" failure={{ kind: "unreachable", message: "노드에 연결할 수 없습니다." }} /></section>;
  switch (kind) {
    case "history": return <HistoryList data={history} basePath="/history" info="설정 적용(apply) 시점마다 저장된 스냅샷 이력을 확인합니다." />;
    case "historyDetail": return <HistoryDetail data={{ ...historyDetail, canRollback: !viewer }} />;
    case "audit": case "nodeAudit": return <AuditList data={audit} basePath={base + "/audit"} />;
    case "auditDetail": case "nodeAuditDetail": return <AuditDetail data={auditDetail} basePath={base + "/audit"} />;
    case "nodes": return <NodeList nodes={[node, { ...node, id: "offline-node", name: "응답을 기다리는 노드", lastStatus: "unreachable", lastSeenAt: null }]} canWrite={!viewer} unpairWarningText={null} />;
    case "nodeNew": return <NodeNewView defaultGatewayName="운영 게이트웨이" />;
    case "node": return <NodeOverview id={node.id} result={available ? { ok: true, node, canManage: !viewer, data: status } : { ok: false, node, canManage: !viewer, nodeError: { nodeId: node.id, retryHref: paths.node, listHref: "/nodes", failure: { kind: "unreachable", message: "노드에 연결할 수 없습니다." } } }} />;
    case "nodeEdit": return <NodeEditView node={node} />;
    case "nodeHistory": return <HistoryList data={{ ...history, showDetails: false }} info="이 노드에 적용된 설정 스냅샷 이력입니다." notice="스냅샷 내용은 노드에서만 볼 수 있습니다." renderRowAction={row => <NodeHistoryActions nodeId={node.id} nodeName={node.name} snapshotId={row.id} label={row.label} iso={row.iso} retryHref={paths.nodeHistory} nodeUrl={node.baseUrl} canRollback={!viewer} />} />;
  }
}
const meta = { id: "ui-impl-history-nodes", title: "Product/구현/이력과 노드", component: Pages, tags: ["!autodocs"], parameters: { layout: "fullscreen" }, args: { kind: "history" }, decorators: [(Story, context) => <ProductFrame path={paths[context.args.kind as Kind]} label="이력과 노드" role={context.args.viewer ? "viewer" : "admin"} storyId={context.id} viewport={undefined} nodeName={node.name} available={context.args.available !== false}><Story /></ProductFrame>] } satisfies Meta<typeof Pages>;
export default meta;
type Story = StoryObj<typeof meta>;
export const History: Story = {};
export const HistorySnapshot: Story = { args: { kind: "historyDetail" } };
export const Audit: Story = { args: { kind: "audit" } };
export const AuditRecord: Story = { args: { kind: "auditDetail" } };
export const Nodes: Story = { args: { kind: "nodes" } };
export const NodeNew: Story = { args: { kind: "nodeNew" } };
export const Node: Story = { args: { kind: "node" } };
export const NodeEdit: Story = { args: { kind: "nodeEdit" } };
export const NodeHistory: Story = { args: { kind: "nodeHistory" } };
export const NodeAudit: Story = { args: { kind: "nodeAudit" } };
export const NodeAuditRecord: Story = { args: { kind: "nodeAuditDetail" } };
export const Offline: Story = { args: { kind: "node", available: false } };
export const OfflineHistory: Story = { args: { kind: "nodeHistory", available: false } };
export const OfflineAudit: Story = { args: { kind: "nodeAudit", available: false } };
export const OfflineAuditRecord: Story = { args: { kind: "nodeAuditDetail", available: false } };
export const ViewerHistory: Story = { args: { kind: "nodeHistory", viewer: true } };

async function cancelConfirmation(canvasElement: HTMLElement, label: string, target: string) {
  const old = window.__NWUI_PREVIEW_ACTION__;
  let calls = 0;
  window.__NWUI_PREVIEW_ACTION__ = async () => { calls++; return null; };
  try {
    const trigger = within(canvasElement).getAllByRole("button", { name: label })[0]!;
    await userEvent.click(trigger);
    const dialog = within(canvasElement.ownerDocument.body).getByRole("dialog", { name: label });
    await expect(dialog).toHaveTextContent(target);
    await expect(calls).toBe(0);
    await userEvent.click(within(dialog).getByRole("button", { name: "취소" }));
    await waitFor(() => expect(dialog).not.toBeVisible());
    await expect(calls).toBe(0);
    await expect(trigger).toHaveFocus();
    canvasElement.dataset.confirmTested = "true";
  } finally { window.__NWUI_PREVIEW_ACTION__ = old; }
}
export const LocalRollbackConfirm: Story = { args: { kind: "historyDetail" }, play: ({ canvasElement }) => cancelConfirmation(canvasElement, "이 스냅샷으로 롤백", "운영 리버스 프록시 설정 적용") };
export const NodeRollbackConfirm: Story = { args: { kind: "nodeHistory" }, play: ({ canvasElement }) => cancelConfirmation(canvasElement, "이 스냅샷으로 롤백", node.name) };
export const UnpairConfirm: Story = { args: { kind: "nodeEdit" }, play: ({ canvasElement }) => cancelConfirmation(canvasElement, "연결 해제", node.name) };
