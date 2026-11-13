import { useEffect, useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { Button, Modal } from "@repo/ui";
import type { Role } from "@repo/contracts";
import type { OperationsTarget } from "@repo/presentation/operations-target";
import type { ConfigDiff } from "@/lib/config-diff";
import { ProfileMenu } from "@/components/profile-menu";
import {
  PendingBanner,
  PendingChangesBadge,
} from "@/components/pending-changes-badge";
import { NotificationBellView } from "@/components/notifications/notification-bell-view";
import {
  ToastProvider,
  ToastViewport,
  useToast,
} from "@/components/notifications/toast-provider";
import { ApplyReview, applyResultToast } from "@/components/apply-bar";
import { ConfigDiffView } from "@/components/config-diff-view";
import { OperationsShell } from "@/components/operations/operations-shell";
import { StoryPathContext } from "./adapters/next-navigation";

const diff: ConfigDiff = {
  hasChanges: true,
  servers: [
    {
      key: "server-1",
      label: "서울 운영 api.example.test:443",
      status: "changed",
      changes: [
        {
          field: "proxy_pass",
          before: "http://old.example.test:8080",
          after: "http://new.example.test:8080",
        },
      ],
    },
    {
      key: "server-2",
      label: "설정을 확인해야 하는 서버",
      status: "skipped",
      reason: "예약된 포트는 사용할 수 없습니다.",
      changes: [],
    },
  ],
  streams: [
    { key: "stream-1", label: "TCP 5432", status: "added", changes: [] },
  ],
  upstreams: [],
};
const diagnostics = Array.from(
  { length: 15 },
  (_, i) =>
    `nginx: [emerg] ${i + 1} ${"very-long-upstream-name.".repeat(12)}test`,
).join("\n");
const build = {
  version: "2.0.0",
  sha: "6ab3e7f",
  fullSha: "6ab3e7f28dfe1587fd3a2564d01d2ee1763fdc19",
};

function Profile({
  role = "admin",
  long = false,
}: {
  role?: Role;
  long?: boolean;
}) {
  return (
    <div
      style={{
        display: "flex",
        justifyContent: "flex-end",
        alignItems: "flex-start",
        minHeight: 440,
      }}
    >
      <ProfileMenu
        email={
          long
            ? "아주긴운영관리자이름.서비스운영.보안관리@example-with-a-very-long-domain.test"
            : "admin@example.test"
        }
        role={role}
        {...build}
      />
    </div>
  );
}
function Pending({ count = 3 }: { count?: number }) {
  const [visible, setVisible] = useState(true);
  const [reviewed, setReviewed] = useState(false);
  return (
    <div style={{ display: "grid", gap: "var(--sp-4)" }}>
      {visible ? (
        <PendingBanner
          count={count}
          onReview={() => setReviewed(true)}
          onDismiss={() => setVisible(false)}
        />
      ) : null}
      <output>{reviewed ? "변경 내용 검토 요청됨" : "검토 전"}</output>
    </div>
  );
}
function Review({
  node = false,
  viewer = false,
}: {
  node?: boolean;
  viewer?: boolean;
}) {
  const [open, setOpen] = useState(true);
  return (
    <ToastProvider>
      <Modal open={open} onClose={() => setOpen(false)} title="설정 적용">
        <ApplyReview
          target={
            node
              ? { kind: "node", id: "node-1", name: "local" }
              : { kind: "local" }
          }
          targetLabel={node ? "local" : "로컬 서버"}
          canApply={!viewer}
          pendingCount={3}
          pendingHasBaseline
          pendingDiff={diff}
          applyAction={async () => ({ ok: false, output: diagnostics })}
          onCancel={() => setOpen(false)}
        />
      </Modal>
      <ToastViewport />
    </ToastProvider>
  );
}
function Notifications({
  empty = false,
  long = false,
}: {
  empty?: boolean;
  long?: boolean;
}) {
  const [read, setRead] = useState(false);
  const [href, setHref] = useState("");
  return (
    <div style={{ display: "grid", gap: "var(--sp-4)", minHeight: 440 }}>
      <div style={{ display: "flex", justifyContent: "flex-end" }}>
        <NotificationBellView
          unread={read ? 0 : 3}
          onOpen={() => setRead(true)}
          onNavigate={setHref}
          items={
            empty
              ? []
              : [
                  {
                    id: "n1",
                    type: "apply",
                    severity: "error",
                    title: long
                      ? "아주 긴 한국어 운영 서버 이름의 설정을 반영하지 못했습니다"
                      : "설정을 적용하지 못했습니다",
                    body: "nginx -t 검사 결과를 확인하세요.",
                    link: "/history/17",
                    createdAt: "2026-11-11T08:00:00Z",
                  },
                  {
                    id: "n2",
                    type: "certificate",
                    severity: "warning",
                    title: "인증서 만료 확인",
                    body: "인증서를 확인하세요.",
                    link: null,
                    createdAt: "2026-11-11T07:00:00Z",
                  },
                ]
          }
        />
      </div>
      <output>{href ? `이동: ${href}` : "알림 대기"}</output>
    </div>
  );
}
function ToastButtons() {
  const { toast } = useToast();
  return (
    <div style={{ display: "flex", flexWrap: "wrap", gap: "var(--sp-2)" }}>
      <Button
        variant="secondary"
        onClick={() =>
          toast({
            key: "success",
            severity: "success",
            title: "설정을 적용했습니다",
            body: "검사와 다시 불러오기가 끝났습니다.",
          })
        }
      >
        성공 알림
      </Button>
      <Button
        variant="secondary"
        onClick={() =>
          toast({
            key: "error",
            severity: "error",
            title: "설정을 적용하지 못했습니다",
            output: diagnostics,
            link: "/history/17",
            linkLabel: "기록 보기",
          })
        }
      >
        오류 알림
      </Button>
      <Button
        variant="secondary"
        onClick={() => toast({ severity: "error", title: "진단 결과", output: "nginx: configuration is valid" })}
      >
        짧은 진단 알림
      </Button>
      <Button
        variant="secondary"
        onClick={() =>
          toast(
            applyResultToast({
              ok: true,
              output:
                diagnostics +
                "\n\n새 변경을 적용하지 못한 설정 1개:\n- 리버스 프록시 api.example.test: nginx -t 실패",
            }),
          )
        }
      >
        부분 적용 알림
      </Button>
      <Button
        variant="secondary"
        onClick={() =>
          toast(
            applyResultToast({
              ok: false,
              nodeError: {
                nodeId: "preview-node",
                retryHref: "/nodes/preview-node",
                listHref: "/nodes",
                failure: {
                  kind: "version_mismatch",
                  message: "버전이 다릅니다",
                  nodeVersion: "1.0.0",
                },
              },
            }),
          )
        }
      >
        문장 오류 알림
      </Button>
      <Button
        variant="secondary"
        onClick={() =>
          toast({
            severity: "warning",
            title: "일부 설정을 적용하지 못했습니다",
            body: "적용에서 제외한 설정 2개:\n- api.test: nginx -t 실패\n- stream 53: 포트 충돌",
          })
        }
      >
        줄바꿈 알림
      </Button>
      <Button
        variant="secondary"
        onClick={() =>
          toast({
            severity: "warning",
            title: "일부 설정을 적용하지 못했습니다",
            body:
              "적용에서 제외한 설정 35개:\n" +
              Array.from(
                { length: 35 },
                (_, i) => `- api-${i}.test: nginx -t 실패`,
              ).join("\n"),
          })
        }
      >
        긴 본문 알림
      </Button>
      <Button
        variant="secondary"
        onClick={() =>
          toast({
            severity: "info",
            title: "이 노드가 연결을 끊었습니다",
            body: "다시 연결하세요.",
          })
        }
      >
        짧은 본문 알림
      </Button>
    </div>
  );
}
function Toasts() {
  return (
    <ToastProvider>
      <ToastButtons />
      <ToastViewport />
    </ToastProvider>
  );
}

/** Changes the real shell's data/context snapshot without copying its review-close logic. */
function ShellSafety() {
  const [state, setState] = useState({
    path: "/reverse-proxy",
    target: { kind: "local" } as OperationsTarget,
    role: "editor" as Role,
    available: true,
    canApply: true,
  });
  useEffect(() => {
    const update = (event: Event) =>
      setState((previous) => ({
        ...previous,
        ...(event as CustomEvent).detail,
      }));
    window.addEventListener("wp2b-context", update);
    return () => window.removeEventListener("wp2b-context", update);
  }, []);
  return (
    <ToastProvider>
      <StoryPathContext.Provider value={state.path}>
        <OperationsShell
          role={state.role}
          email="editor@example.test"
          target={state.target}
          notifications={false}
          data={
            state.available
              ? {
                  available: true,
                  serverTotal: 1,
                  serverActive: 1,
                  pendingCount: 3,
                  pendingHasBaseline: true,
                  pendingDiff: diff,
                  canApply: state.canApply,
                  applyAction: { id: "apply.applyConfig", args: [] },
                }
              : { available: false }
          }
        >
          <h1>적용 검토 대상 검증</h1>
        </OperationsShell>
      </StoryPathContext.Provider>
    </ToastProvider>
  );
}

const meta = {
  title: "Product/WP-2B Shell overlays",
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen" },
  decorators: [
    (Story) => (
      <main
        style={{
          maxWidth: 1200,
          margin: "0 auto",
          width: "100%",
          boxSizing: "border-box",
          padding: "var(--sp-6)",
        }}
      >
        <Story />
      </main>
    ),
  ],
} satisfies Meta;
export default meta;
type Story = StoryObj<typeof meta>;
const openProfile: Story["play"] = async ({ canvasElement }) => {
  await userEvent.click(
    within(canvasElement).getByRole("button", { name: /^계정 메뉴:/ }),
  );
};
const openNotifications: Story["play"] = async ({ canvasElement }) => {
  await userEvent.click(
    within(canvasElement).getByRole("button", { name: "알림" }),
  );
};
export const ProfileClosed: Story = { render: () => <Profile /> };
export const ProfileOpen: Story = {
  render: () => <Profile />,
  play: openProfile,
};
export const ProfileViewer: Story = {
  render: () => <Profile role="viewer" />,
  play: openProfile,
};
export const ProfileLong: Story = {
  render: () => <Profile long />,
  play: openProfile,
};
export const PendingChanges: Story = { render: () => <Pending /> };
export const PendingEmpty: Story = { render: () => <Pending count={0} /> };
export const PendingReview: Story = {
  render: () => <Pending />,
  play: async ({ canvasElement }) => {
    const c = within(canvasElement);
    await userEvent.click(c.getByRole("button", { name: "변경 내용 보기" }));
    await expect(c.getByText("변경 내용 검토 요청됨")).toBeVisible();
  },
};
export const Applied: Story = {
  render: () => (
    <PendingChangesBadge
      count={0}
      hasBaseline
      diff={{ ...diff, hasChanges: false }}
    />
  ),
};
export const UnknownBaseline: Story = {
  render: () => (
    <PendingChangesBadge count={0} hasBaseline={false} diff={diff} />
  ),
};
export const ReviewLocal: Story = { render: () => <Review /> };
export const ReviewNode: Story = { render: () => <Review node /> };
export const ReviewViewer: Story = { render: () => <Review viewer /> };
export const NotificationOpen: Story = {
  render: () => <Notifications />,
  play: openNotifications,
};
export const NotificationEmpty: Story = {
  render: () => <Notifications empty />,
  play: openNotifications,
};
export const NotificationLong: Story = {
  render: () => <Notifications long />,
  play: openNotifications,
};
export const ToastControls: Story = { render: () => <Toasts /> };
export const ToastSuccess: Story = {
  render: () => <Toasts />,
  play: async ({ canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByRole("button", { name: "성공 알림" }),
    );
  },
};
export const ToastError: Story = {
  render: () => <Toasts />,
  play: async ({ canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByRole("button", { name: "오류 알림" }),
    );
  },
};
export const Diff: Story = {
  render: () => <ConfigDiffView diff={diff} variant="panel" />,
};
export const TargetSafety: Story = {
  render: () => <ShellSafety />,
  decorators: [(Story) => <Story />],
};

export const ToastPartialApply: Story = {
  render: () => <Toasts />,
  play: async ({ canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByRole("button", { name: "부분 적용 알림" }),
    );
  },
};
export const ToastErrorSentence: Story = {
  render: () => <Toasts />,
  play: async ({ canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByRole("button", { name: "문장 오류 알림" }),
    );
  },
};

export const ToastSkippedBody: Story = {
  render: () => <Toasts />,
  play: async ({ canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByRole("button", { name: "줄바꿈 알림" }),
    );
  },
};
export const ToastLongBody: Story = {
  render: () => <Toasts />,
  play: async ({ canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByRole("button", { name: "긴 본문 알림" }),
    );
  },
};
export const ToastShortBody: Story = {
  render: () => <Toasts />,
  play: async ({ canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByRole("button", { name: "짧은 본문 알림" }),
    );
  },
};

export const ToastShortOutput: Story = {
  render: () => <Toasts />,
  play: async ({ canvasElement }) => {
    await userEvent.click(within(canvasElement).getByRole("button", { name: "짧은 진단 알림" }));
  },
};
