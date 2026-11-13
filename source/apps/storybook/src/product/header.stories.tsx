import { useEffect, useState } from "react";
import {
  ToastProvider,
  ToastViewport,
} from "@/components/notifications/toast-provider";
import { OperationsShell } from "@/components/operations/operations-shell";
import { StoryPathContext } from "./adapters/next-navigation";
import type { OperationsTarget } from "@repo/presentation/operations-target";
import type { Role } from "@repo/contracts";
import type { OperationsShellData } from "@/lib/operations-shell-data";
import {
  AppShell,
  Button,
  ContextBar,
  PageHeader,
  SectionNav,
  StatusBadge,
  Surface,
} from "@repo/ui";
import { HeaderActions } from "@/components/operations/header-actions";
import { NAV_CATEGORIES, ADMIN_CATEGORIES } from "@/app/(app)/nav-links";
import { nodeNavCategories } from "@/app/(node)/nodes/[id]/node-nav";
import shellStyles from "@/components/operations/operations-shell.module.css";
import pageLayout from "@/components/page-layout.module.css";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, waitFor, within } from "storybook/test";
import { ProductFrame, productViewports } from "./frame";

const meta = {
  id: "product-header",
  title: "Product/Header",
  component: ProductFrame,
  tags: ["!autodocs"],
  parameters: {
    layout: "fullscreen",
    viewport: {
      options: {
        ...productViewports,
        p320: {
          name: "Mobile 320",
          styles: { width: "320px", height: "640px" },
          type: "mobile",
        },
      },
    },
    docs: {
      description: {
        component:
          "48px 상단 바, 216/56px 레일, 768px 이하 드로어를 사용합니다. 페이지에 주 동작이 있으면 상단 적용은 보조 동작입니다. 노드 맥락은 전역 메뉴가 아닌 맥락 바와 경로에 표시합니다.",
      },
    },
  },
  args: {
    path: "/",
    label: "개요",
    role: "editor",
    storyId: "product-header--default",
    viewport: undefined,
    children: (
      <PageHeader
        title="헤더"
        description="페이지의 주 동작 유무에 따라 상단 적용의 강조가 바뀝니다."
      />
    ),
  },
} satisfies Meta<typeof ProductFrame>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const Unread: Story = {
  args: {
    unreadCount: 3,
    notificationItems: [
      {
        id: "example",
        type: "certificate",
        severity: "warning",
        title: "인증서 만료 예정",
        body: "미리보기 알림입니다.",
        link: "/certificates",
        createdAt: "2026-09-27T10:00:00",
      },
    ],
  },
};
export const ManyUnread: Story = { args: { ...Unread.args, unreadCount: 120 } };
export const Viewer: Story = { args: { role: "viewer" } };
export const Admin: Story = { args: { role: "admin" } };
export const Mobile: Story = {
  globals: { viewport: { value: "p390", isRotated: false } },
};
/** 미적용 0건: 요약에 색 점이 없다. */
export const NothingPending: Story = {
  name: "미적용 없음",
  args: { pendingCount: 0 },
};
/** 320px과 긴 페이지 이름: 요약은 둘째 줄, 이름은 말줄임, 가로 스크롤 없음. */
export const Narrow320: Story = {
  name: "320 · 긴 이름",
  args: { label: "edge-gateway-seoul-production-primary-01 리버스 프록시" },
  globals: { viewport: { value: "p320", isRotated: false } },
  play: async ({ canvasElement }) => {
    const breadcrumb = within(canvasElement).getByRole("navigation", { name: "경로" });
    await expect(breadcrumb.closest("header")).toHaveTextContent(
      "edge-gateway-seoul-production-primary-01 리버스 프록시",
    );
    const doc = canvasElement.ownerDocument.documentElement;
    await expect(doc.scrollWidth).toBeLessThanOrEqual(doc.clientWidth);
  },
};

export const LongNotifications: Story = {
  name: "알림 · 긴 제목과 본문",
  args: {
    unreadCount: 2,
    notificationItems: [
      {
        id: "long-title",
        type: "certificate",
        severity: "warning",
        title:
          "인증서 갱신 실패: productiongatewaywithoutwordbreaksfornotificationlayoutcheck.example.test",
        body: "인증서 갱신 요청을 완료하지 못했습니다. 연결된 리버스 프록시와 DNS 설정을 확인한 후 다시 시도하세요.",
        link: "/certificates",
        createdAt: "2026-09-28T10:00:00",
      },
      {
        id: "long-body",
        type: "apply",
        severity: "error",
        title: "설정 적용 실패",
        body: "upstream: https://upstream.example.test/averylongpathsegmentwithoutwordbreaksfornotificationlayoutverification",
        link: null,
        createdAt: "2026-09-28T10:01:00",
      },
    ],
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: "알림" }));
    const menu = canvas.getByRole("menu");
    await waitFor(() =>
      expect(menu.scrollWidth).toBeLessThanOrEqual(menu.clientWidth),
    );
    for (const item of within(menu).getAllByRole("menuitem")) {
      expect(item.scrollWidth).toBeLessThanOrEqual(item.clientWidth);
    }
  },
};
export const LongNotificationsMobile: Story = {
  ...LongNotifications,
  name: "알림 · 긴 내용 · 390",
  globals: Mobile.globals,
};
export const Profile: Story = {
  name: "상단 계정 메뉴",
  args: { role: "admin" },
  play: async ({ canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByRole("button", { name: /admin@example.test/ }),
    );
  },
};
export const ProfileMobile: Story = {
  ...Profile,
  name: "프로필 · 390",
  globals: Mobile.globals,
  // The account action remains in the top bar at this width.
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(
      canvas.getByRole("button", { name: /admin@example.test/ }),
    );
  },
};

export const Node: Story = {
  args: {
    path: "/nodes/example/logs",
    label: "로그",
    nodeName: "서울 운영 노드",
  },
};
export const NodeOffline: Story = {
  ...Node,
  args: { ...Node.args, available: false },
};
export const NodeLong: Story = {
  ...Node,
  args: {
    ...Node.args,
    nodeName: "productiongatewaywithoutwordbreaks-서울데이터센터",
  },
};
export const Unavailable: Story = { args: { available: false } };
export const BaselineUnknown: Story = {
  args: { pendingCount: 0, pendingHasBaseline: false },
};
export const DefaultDark: Story = { globals: { theme: "dark" } };
export const NodeOfflineDark: Story = {
  ...NodeOffline,
  globals: { theme: "dark" },
};

function ReviewBoundaryHarness() {
  const [path, setPath] = useState("/nodes/first/logs");
  const [target, setTarget] = useState<OperationsTarget>({
    kind: "node",
    id: "first",
    name: "서울 운영 노드",
  });
  const [role, setRole] = useState<Role>("editor");
  const [available, setAvailable] = useState(true);
  const [permitted, setPermitted] = useState(true);
  useEffect(() => {
    const change = (event: Event) => {
      const kind = (event as CustomEvent<string>).detail;
      if (kind === "route") setPath("/nodes/first/metrics");
      if (kind === "target")
        setTarget({ kind: "node", id: "second", name: "다른 운영 노드" });
      if (kind === "availability") setAvailable(false);
      if (kind === "permission") setPermitted(false);
      if (kind === "role") setRole("viewer");
      if (kind === "reset") {
        setPath("/nodes/first/logs");
        setTarget({ kind: "node", id: "first", name: "서울 운영 노드" });
        setRole("editor");
        setAvailable(true);
        setPermitted(true);
      }
    };
    window.addEventListener("p12-review-context", change);
    return () => window.removeEventListener("p12-review-context", change);
  }, []);
  const data: OperationsShellData = available
    ? {
        available: true,
        serverTotal: 0,
        serverActive: 0,
        pendingCount: 2,
        pendingHasBaseline: true,
        pendingDiff: {
          servers: [],
          streams: [],
          upstreams: [],
          hasChanges: true,
        },
        canApply: permitted,
        applyAction: {
          id: "node.applyNodeConfig",
          args: ["first", "/nodes/first/reverse-proxy"],
        },
      }
    : { available: false };
  return (
    <ToastProvider>
      <StoryPathContext.Provider value={path}>
        <OperationsShell
          target={target}
          role={role}
          email="editor@example.test"
          notifications={false}
          data={data}
        >
          <PageHeader
            title="리뷰 맥락 변경"
            description="Storybook에서만 대상·경로·연결·권한 변경을 주입합니다. 실제 요청은 보내지 않습니다."
          />
        </OperationsShell>
      </StoryPathContext.Provider>
      <ToastViewport />
    </ToastProvider>
  );
}
export const ReviewContextSafety: Story = {
  render: () => <ReviewBoundaryHarness />,
};
export const ReviewContextSafetyDark: Story = {
  ...ReviewContextSafety,
  globals: { theme: "dark" },
};

export const AdminSettings: Story = {
  args: { path: "/admin/security", label: "보안 (2FA)", role: "admin" },
};
export const AdminSettingsDark: Story = {
  ...AdminSettings,
  globals: { theme: "dark" },
};

const shellInitial = {
  path: "/",
  target: { kind: "local" } as OperationsTarget,
  role: "editor" as Role,
  available: true,
  primary: false,
  disabled: false,
  closedDialog: false,
};
function ShellHarness({
  initial = {},
}: {
  initial?: Partial<typeof shellInitial>;
}) {
  const [state, setState] = useState({ ...shellInitial, ...initial });
  useEffect(() => {
    const change = (event: Event) =>
      setState((previous) => ({
        ...previous,
        ...(event as CustomEvent<Partial<typeof shellInitial>>).detail,
      }));
    window.addEventListener("wp2a-shell", change);
    return () => window.removeEventListener("wp2a-shell", change);
  }, []);
  const data: OperationsShellData = state.available
    ? {
        available: true,
        serverTotal: 2,
        serverActive: 1,
        pendingCount: 2,
        pendingHasBaseline: true,
        pendingDiff: {
          servers: [],
          streams: [],
          upstreams: [],
          hasChanges: true,
        },
        canApply: true,
        applyAction:
          state.target.kind === "node"
            ? {
                id: "node.applyNodeConfig",
                args: [state.target.id, state.path],
              }
            : { id: "apply.applyConfig", args: [] },
      }
    : { available: false };
  const node = state.target.kind === "node" ? state.target : null;
  const admin = state.path.startsWith("/admin");
  return (
    <ToastProvider>
      <StoryPathContext.Provider value={state.path}>
        <OperationsShell
          role={state.role}
          email="editor@example.test"
          target={state.target}
          data={admin ? undefined : data}
          notifications={false}
          crumbs={
            <nav aria-label="경로">
              <ol>
                <li>구성</li>
                <li title={node?.name ?? "리버스 프록시"}>
                  {node?.name ?? "리버스 프록시"}
                </li>
                <li aria-current="page">{state.primary ? "수정" : "개요"}</li>
              </ol>
            </nav>
          }
          contentHeader={
            node ? (
              <>
                <div className={shellStyles.nodeContextBand}>
                  <div className={shellStyles.shellColumn}>
                    <ContextBar
                      name={node.name}
                      status={<StatusBadge tone="success">연결됨</StatusBadge>}
                    />
                  </div>
                </div>
                <div className={shellStyles.shellColumn}>
                  <SectionNav
                    groups={nodeNavCategories(node.id)}
                    currentPath={state.path}
                    label="노드 메뉴"
                  />
                </div>
              </>
            ) : admin ? (
              <div className={shellStyles.shellColumn}>
                <SectionNav
                  groups={ADMIN_CATEGORIES}
                  currentPath={state.path}
                  label="관리자 설정"
                />
              </div>
            ) : undefined
          }
        >
          <div className={pageLayout.page}>
            <PageHeader
              title={state.primary ? "서버 수정" : "개요"}
              actions={
                state.primary ? (
                  <Button disabled={state.disabled}>저장</Button>
                ) : undefined
              }
            />
            <Surface title="현재 구성">
              <p>같은 서버에서 읽은 변경 상태를 표시합니다.</p>
            </Surface>
            {state.closedDialog ? (
              <dialog>
                <Button>닫힌 대화상자의 주 동작</Button>
              </dialog>
            ) : null}
          </div>
        </OperationsShell>
      </StoryPathContext.Provider>
    </ToastProvider>
  );
}
export const ShellOverview: Story = { render: () => <ShellHarness /> };
export const ShellEdit: Story = {
  render: () => (
    <ShellHarness
      initial={{ path: "/reverse-proxy/demo/edit", primary: true }}
    />
  ),
};
export const ShellNode: Story = {
  render: () => (
    <ShellHarness
      initial={{
        path: "/nodes/first/reverse-proxy",
        target: { kind: "node", id: "first", name: "서울 운영 노드" },
      }}
    />
  ),
};
export const ShellAdminTabs: Story = {
  render: () => (
    <ShellHarness initial={{ path: "/admin/settings/smtp", role: "admin" }} />
  ),
};
export const ShellViewer: Story = {
  render: () => (
    <ShellHarness initial={{ path: "/reverse-proxy", role: "viewer" }} />
  ),
};
export const ShellGeometry: Story = {
  render: () => (
    <AppShell
      nav={NAV_CATEGORIES}
      currentPath="/"
      version="2.0.0"
      renderLink={({ onClick, ...link }) => (
        <a
          {...link}
          onClick={(event) => {
            event.preventDefault();
            onClick?.();
          }}
        />
      )}
      topbarContext={<span>모니터링 / 개요</span>}
      topbarActions={
        <HeaderActions canApply pendingCount={2} onApply={() => {}} />
      }
    >
      <div className={pageLayout.page}>
        <PageHeader title="개요" />
        <Surface title="현재 구성">
          <p>공통 셸의 폭과 키보드 동작</p>
        </Surface>
      </div>
    </AppShell>
  ),
};
