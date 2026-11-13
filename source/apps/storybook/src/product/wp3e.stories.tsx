import type { Meta, StoryObj } from "@storybook/react-vite";
import type { HistoryDetailView } from "@/lib/overview-views";
import { HistoryList } from "@/components/history/history-list";
import { HistoryDetail } from "@/components/history/history-detail";
import { AuditList } from "@/components/audit/audit-list";
import { AuditDetail } from "@/components/audit/audit-detail";
import { NodeHistoryActions } from "@/app/(node)/nodes/[id]/history/rollback-button";
import { ProductFrame } from "./frame";

const iso = "2026-11-11T09:00:00Z";
const info =
  "설정 적용(apply) 시점마다 저장된 스냅샷 이력을 확인합니다. 최근 50건을 표시하며, 각 항목을 열면 스냅샷 내용과 롤백 버튼을 확인할 수 있습니다.";
const nodeInfo =
  "이 노드에 적용된 설정 스냅샷 이력입니다. 게이트웨이는 스냅샷 내용을 읽을 수 없어 개수·작성자 열은 표시하지 않습니다.";
const rows = [0, 1, 2].map((i) => ({
  id: `snapshot-${i}`,
  iso,
  label: i === 2 ? null : `운영 설정 ${3 - i}`,
  actorEmail: i === 1 ? null : "admin@example.test",
  upstreamCount: 2,
  serverCount: 4,
  streamCount: 1,
}));
const detail: HistoryDetailView = {
  snapshot: {
    id: "snapshot-0",
    label: "운영 설정 3",
    actorEmail: "admin@example.test",
    actorId: "admin",
    data: { upstreams: [], servers: [], streams: [] },
    createdAt: iso,
  },
  iso,
  upstreams: [],
  servers: [],
  streams: [],
  canRollback: true,
  isFirst: false,
  diff: {
    hasChanges: true,
    upstreams: [],
    streams: [],
    servers: [
      {
        key: "server-a",
        label: "api.example.test",
        status: "changed",
        changes: [
          {
            field: "proxy_pass",
            before: "http://old.test",
            after: "http://new.test",
          },
        ],
      },
    ],
  },
};
type Props = {
  mode: "history" | "history-detail" | "audit" | "audit-detail";
  node: boolean;
  viewer: boolean;
  empty: boolean;
  first: boolean;
  unchanged: boolean;
  page: number;
  filter: string;
  missing: boolean;
};
function base(args: Props) {
  return `${args.node ? "/nodes/wp3e-node" : ""}/${args.mode.startsWith("history") ? "history" : "audit"}`;
}
function Pages(args: Props) {
  const basePath = base(args);
  if (args.mode === "history")
    return (
      <HistoryList
        info={args.node ? nodeInfo : info}
        notice={
          args.node ? "스냅샷 내용은 노드에서만 볼 수 있습니다." : undefined
        }
        basePath={args.node ? undefined : basePath}
        data={{ items: args.empty ? [] : rows, showDetails: !args.node }}
        renderRowAction={
          args.node
            ? (row) => (
                <NodeHistoryActions
                  nodeUrl="https://node.example.test"
                  nodeId="wp3e-node"
                  nodeName="서울 운영 노드"
                  snapshotId={row.id}
                  label={row.label}
                  iso={row.iso}
                  retryHref={basePath}
                  canRollback={!args.viewer}
                />
              )
            : undefined
        }
      />
    );
  if (args.mode === "history-detail")
    return (
      <HistoryDetail
        data={{
          ...detail,
          canRollback: !args.viewer,
          isFirst: args.first,
          diff: args.unchanged
            ? { hasChanges: false, upstreams: [], servers: [], streams: [] }
            : detail.diff,
        }}
      />
    );
  if (args.mode === "audit-detail")
    return (
      <AuditDetail
        basePath={basePath}
        data={{
          action: "config.apply",
          time: "2026. 11. 11. 09:00:00 UTC",
          actorEmail: args.missing ? null : "admin@example.test",
          actorLabel: args.missing ? "운영 게이트웨이" : null,
          target: args.missing ? null : "nginx",
          ip: args.missing ? null : "192.0.2.10",
          beforeJson: args.missing ? null : '{\n  "enabled": false\n}',
          afterJson: args.missing ? null : '{\n  "enabled": true\n}',
        }}
      />
    );
  return (
    <AuditList
      basePath={basePath}
      data={{
        items: args.empty
          ? []
          : rows.map((row, i) => ({
              id: `audit-${i}`,
              time: "2026. 11. 11. 09:00:00 UTC",
              actorEmail: row.actorEmail,
              actorLabel: i === 1 ? "운영 게이트웨이" : null,
              action: ["config.apply", "auth.login", "server.update"][i]!,
              target: i === 2 ? null : "api.example.test",
            })),
        total: 123,
        page: args.page,
        totalPages: 3,
        actions: ["config.apply", "auth.login", "server.update"],
        actionFilter: args.filter,
      }}
    />
  );
}
const meta = {
  id: "product-wp3e",
  title: "Product/WP-3E",
  component: Pages,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen" },
  decorators: [
    (Story, context) => (
      <ProductFrame
        path={`${base(context.args as Props)}${context.args.mode.endsWith("detail") ? "/snapshot-0" : ""}`}
        label={
          context.args.mode.startsWith("history") ? "적용 이력" : "감사 로그"
        }
        role={context.args.viewer ? "viewer" : "admin"}
        storyId={context.id}
        viewport={undefined}
        nodeName="서울 운영 노드"
      >
        <Story />
      </ProductFrame>
    ),
  ],
  args: {
    mode: "history",
    node: false,
    viewer: false,
    empty: false,
    first: false,
    unchanged: false,
    page: 2,
    filter: "config.apply",
    missing: false,
  },
} satisfies Meta<typeof Pages>;
export default meta;
type Story = StoryObj<typeof meta>;
export const History: Story = {};
export const HistoryEmpty: Story = { args: { empty: true } };
export const NodeHistory: Story = { args: { node: true } };
export const NodeHistoryViewer: Story = { args: { node: true, viewer: true } };
export const NodeHistoryEmpty: Story = { args: { node: true, empty: true } };
export const HistoryDetailPage: Story = { args: { mode: "history-detail" } };
export const HistoryDetailViewer: Story = {
  args: { mode: "history-detail", viewer: true },
};
export const HistoryFirst: Story = {
  args: { mode: "history-detail", first: true },
};
export const HistoryUnchanged: Story = {
  args: { mode: "history-detail", unchanged: true },
};
export const Audit: Story = { args: { mode: "audit" } };
export const AuditFirst: Story = {
  args: { mode: "audit", page: 1, filter: "" },
};
export const AuditLast: Story = { args: { mode: "audit", page: 3 } };
export const AuditEmpty: Story = { args: { mode: "audit", empty: true } };
export const AuditEmptyUnfiltered: Story = {
  args: { mode: "audit", empty: true, filter: "" },
};
export const NodeAudit: Story = { args: { mode: "audit", node: true } };
export const NodeAuditViewer: Story = {
  args: { mode: "audit", node: true, viewer: true },
};
export const AuditDetailPage: Story = { args: { mode: "audit-detail" } };
export const NodeAuditDetail: Story = {
  args: { mode: "audit-detail", node: true },
};
export const AuditDetailMissing: Story = {
  args: { mode: "audit-detail", missing: true },
};
