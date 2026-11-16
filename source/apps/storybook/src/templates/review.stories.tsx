import { useState } from "react";
import { ToastProvider } from "@/components/notifications/toast-provider";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Banner, Button, PageHeader, Surface } from "@repo/ui";
import { ApplyResult, type OperationsTarget } from "@/components/apply-bar";
import { ApplyDialog } from "@/components/apply-dialog";
import { ConfigDiffView } from "@/components/config-diff-view";
import type { ConfigDiff } from "@/lib/config-diff";
import { ProductFrame, productViewports } from "../product/frame";
import { targetLabel } from "@repo/presentation/operations-target";
import styles from "./templates.module.css";

const REMOTE_NAME = "서울 운영 노드 · 장비 교체 후 확인 중인 아주 긴 서버 이름";

const diff: ConfigDiff = {
  hasChanges: true,
  servers: [
    { key: "a", label: "app.example.test", status: "added", changes: [] },
    { key: "b", label: "legacy.example.test", status: "removed", changes: [] },
  ],
  upstreams: [
    {
      key: "u",
      label: "app_pool",
      status: "changed",
      changes: [
        { field: "서버", before: "10.0.1.10:3000", after: "10.0.1.11:3000" },
      ],
    },
  ],
  streams: [],
};
function ReviewTemplate({
  remote = false,
  canApply = true,
  available = true,
  skipped,
}: {
  remote?: boolean;
  canApply?: boolean;
  available?: boolean;
  skipped?: "mixed" | "all";
}) {
  const [open, setOpen] = useState(true);
  const target: OperationsTarget = remote
    ? {
        kind: "node",
        id: "preview-node",
        name: REMOTE_NAME,
      }
    : { kind: "local" };
  const label = targetLabel(target);
  const pendingDiff: ConfigDiff = skipped
    ? {
        ...diff,
        servers: diff.servers.map((item, index) =>
          skipped === "all" || index === 0
            ? {
                ...item,
                status: "skipped",
                reason: "예약된 포트는 사용할 수 없습니다.",
              }
            : item,
        ),
        upstreams:
          skipped === "all"
            ? diff.upstreams.map((item) => ({
                ...item,
                status: "skipped",
                reason: "설정을 확인하세요.",
              }))
            : diff.upstreams,
      }
    : diff;
  return (
    <div className={[styles.page, styles.reviewBackdrop].join(" ")}>
      <PageHeader
        eyebrow={label}
        title="리버스 프록시"
        description="저장된 설정과 적용할 변경 사항을 확인합니다."
      />
      {available && !open ? (
        <div>
          <Button onClick={() => setOpen(true)}>검토 미리보기</Button>
        </div>
      ) : null}
      {!available ? (
        <Banner tone="error" title="노드에 연결할 수 없습니다">
          연결 상태를 확인하세요.
        </Banner>
      ) : null}
      {available ? (
        <ApplyDialog
          open={open}
          onClose={() => setOpen(false)}
          target={target}
          targetLabel={label}
          canApply={canApply}
          pendingCount={3}
          pendingHasBaseline
          pendingDiff={pendingDiff}
          applyAction={async () => ({
            ok: false,
            error: "미리보기에서는 설정을 적용하지 않습니다.",
          })}
        />
      ) : null}
    </div>
  );
}
const meta = {
  id: "templates-review",
  title: "Templates/적용 검토",
  component: ReviewTemplate,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [
    (Story) => (
      <ToastProvider>
        <Story />
      </ToastProvider>
    ),
    (Story, context) => (
      <ProductFrame
        path={
          context.args.remote
            ? "/nodes/preview-node/reverse-proxy"
            : "/reverse-proxy"
        }
        label="리버스 프록시"
        role={context.args.canApply === false ? "viewer" : "editor"}
        nodeName={REMOTE_NAME}
        available={context.args.available !== false}
        pendingCount={3}
        storyId={context.id}
        viewport={undefined}
      >
        <Story />
      </ProductFrame>
    ),
  ],
} satisfies Meta<typeof ReviewTemplate>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const RemoteLong: Story = {
  name: "긴 원격 대상",
  args: { remote: true },
};
export const Viewer: Story = { name: "보기 전용", args: { canApply: false } };
export const Offline: Story = {
  name: "오프라인 · 읽기 전용",
  args: { remote: true, available: false },
};
export const Dark: Story = { globals: { theme: "dark" } };

export const HistoryPanel: Story = {
  name: "이력 diff 패널",
  render: () => (
    <div className={styles.page}>
      <PageHeader title="변경 이력" description="저장된 스냅샷의 차이입니다." />
      <ConfigDiffView variant="panel" diff={diff} />
    </div>
  ),
};
export const ResultStates: Story = {
  name: "적용 결과와 노드 오류",
  render: () => (
    <div className={styles.page}>
      <PageHeader
        title="적용 결과"
        description="로컬 검사 결과와 노드 연결 오류를 구분합니다."
      />
      <Surface title="검사 통과">
        <ApplyResult
          result={{
            ok: true,
            output: "nginx: configuration file test is successful",
          }}
        />
      </Surface>
      <Surface title="검사 실패">
        <ApplyResult
          result={{ ok: false, output: "nginx: [emerg] unknown directive" }}
        />
      </Surface>
      <ApplyResult
        result={{
          ok: false,
          nodeError: {
            failure: {
              kind: "unreachable",
              message: "노드에 연결할 수 없습니다.",
            },
            nodeId: "preview-node",
            retryHref: "#retry",
            listHref: "#nodes",
          },
        }}
      />
    </div>
  ),
};

export const SkippedMixed: Story = {
  name: "제외 항목 포함",
  args: { skipped: "mixed" },
};
export const SkippedAll: Story = {
  name: "모든 항목 제외",
  args: { skipped: "all" },
};
