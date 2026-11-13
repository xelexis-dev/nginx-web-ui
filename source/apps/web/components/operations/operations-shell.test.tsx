import { SectionNav, ContextBar } from "@repo/ui";
import { expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { ReactNode } from "react";
import type { OperationsShellData } from "@/lib/operations-shell-data";
const state = vi.hoisted(() => ({
  pathname: "/nodes/node-1/logs",
  opened: JSON.stringify([
    "/nodes/node-1/logs",
    "node",
    "node-1",
    "admin",
    true,
    true,
    "editor",
  ]),
}));
vi.mock("next/navigation", () => ({ usePathname: () => state.pathname }));
vi.mock("react", async (original) => {
  const react = await original<typeof import("react")>();
  return {
    ...react,
    useState: (initial: unknown) =>
      initial === null ? [state.opened, () => {}] : react.useState(initial),
  };
});
vi.mock("@repo/ui", async (original) => {
  const ui = await original<typeof import("@repo/ui")>();
  return {
    ...ui,
    Modal: ({ open, children }: { open: boolean; children: ReactNode }) =>
      open ? <section role="dialog">{children}</section> : null,
  };
});
vi.mock("@/components/apply-bar", () => ({
  ApplyReview: ({
    target,
    targetLabel,
  }: {
    target: { kind: string; id?: string };
    targetLabel: string;
  }) => (
    <span data-review-target={target.kind === "node" ? target.id : "local"}>
      {targetLabel} 검토 내용
    </span>
  ),
}));
vi.mock("@/components/profile-menu", () => ({
  ProfileMenu: () => <span>프로필</span>,
}));
vi.mock("@/components/notifications/notification-bell", () => ({
  NotificationBell: () => null,
}));
vi.mock("@/components/notifications/toast-provider", () => ({
  ToastViewport: () => null,
}));
import { OperationsShell, type OperationsShellProps } from "./operations-shell";
const data: OperationsShellData = {
  available: true,
  serverTotal: 0,
  serverActive: 0,
  pendingCount: 2,
  pendingHasBaseline: true,
  pendingDiff: { servers: [], streams: [], upstreams: [], hasChanges: true },
  canApply: true,
  applyAction: {
    id: "node.applyNodeConfig",
    args: ["node-1", "/nodes/node-1/reverse-proxy"],
  },
};
function render(props: Partial<OperationsShellProps> = {}) {
  return renderToStaticMarkup(
    <OperationsShell
      role="editor"
      email="e@example.test"
      target={{ kind: "node", id: "node-1", name: "admin" }}
      data={data}
      {...props}
    >
      <h1>로그</h1>
    </OperationsShell>,
  );
}
it("opens a review only for the exact route, typed target and current permission snapshot", () => {
  state.pathname = "/nodes/node-1/logs";
  expect(render()).toContain('role="dialog"');
  expect(render()).toContain('data-review-target="node-1">admin 검토 내용');
  for (const props of [
    { target: { kind: "local" } },
    { target: { kind: "node", id: "node-2", name: "admin" } },
    { target: { kind: "node", id: "node-1", name: "renamed" } },
    { data: { available: false } },
    { data: { ...data, canApply: false } },
    { role: "viewer" },
    { role: "admin" },
  ] as Partial<OperationsShellProps>[]) {
    expect(render(props)).not.toContain('role="dialog"');
  }
  state.pathname = "/nodes/node-1/metrics";
  expect(render()).not.toContain('role="dialog"');
  state.pathname = "/nodes/node-1/logs";
});
it("keeps a global rail and typed node label, including reserved-looking names", () => {
  const html = render();
  expect(html).toContain('title="admin"');
  const rail = html.slice(html.indexOf("<aside"), html.indexOf("</aside>"));
  expect(rail).toContain('href="/nodes"');
  expect(rail).not.toContain("/nodes/node-1/");
  expect(rail).not.toContain('title="admin"');
});
it("never hides unavailable or unknown-baseline state behind a zero count", () => {
  expect(render({ role: "viewer", data: { available: false } })).toContain(
    "조회할 수 없음",
  );
  expect(
    render({
      role: "viewer",
      data: { ...data, pendingCount: 0, pendingHasBaseline: false },
    }),
  ).toContain("적용 상태 확인 필요");
  expect(render({ role: "viewer" })).toContain("미적용 변경");
  expect(render({ role: "viewer" })).not.toContain("data-shell-apply");
});

it("section navigation keeps links and marks only the longest matching resource", () => {
  const html = renderToStaticMarkup(
    <SectionNav
      label="노드 메뉴"
      variant="tabs"
      currentPath="/nodes/a/reverse-proxy/upstreams/detail"
      items={[
        { href: "/nodes/a", label: "노드 요약" },
        { href: "/nodes/a/reverse-proxy", label: "리버스 프록시" },
        { href: "/nodes/a/reverse-proxy/upstreams", label: "Upstreams" },
      ]}
    />,
  );
  expect(html.match(/aria-current="page"/g) ?? []).toHaveLength(1);
  expect(html).toMatch(
    /href="\/nodes\/a\/reverse-proxy\/upstreams"[^>]*aria-current="page"/,
  );
  expect(html).not.toContain('role="tab"');
});
it("offline context keeps the supplied status and one explicit exit without mutation actions", () => {
  const html = renderToStaticMarkup(
    <ContextBar name="admin" offline status={<span>연결할 수 없음</span>} />,
  );
  expect(html).toContain("원격 노드");
  expect(html).toContain("연결할 수 없음");
  expect(html).toContain('href="/nodes"');
  expect(html).not.toContain('href="/"');
  expect(html).toContain("노드에서 나가기");
  expect(html.match(/<a /g)).toHaveLength(1);
  expect(html).not.toContain("<button");
  expect(html).not.toContain("<select");
});

it("renders the server breadcrumb slot unchanged, without client name replacement", () => {
  const html = render({
    crumbs: (
      <nav aria-label="경로">
        <ol>
          <li title="server-from-slot" aria-current="page">
            server-from-slot
          </li>
        </ol>
      </nav>
    ),
  });
  expect(html).toContain('title="server-from-slot"');
  expect(html).not.toContain('title="admin"');
  expect(html.match(/aria-label="경로"/g)).toHaveLength(1);
});
it.each([
  "/reverse-proxy",
  "/stream/tcp/edit",
  "/certificates/new",
  "/nodes/node-1/reverse-proxy/s/edit",
])("shows the pending strip on configuration page %s", (path) => {
  state.pathname = path;
  expect(render()).toContain("변경 내용 보기");
  state.pathname = "/nodes/node-1/logs";
});
it.each([
  "/",
  "/logs",
  "/admin/settings/general",
  "/reverse-proxy-extra",
  "/nodes/node-2/reverse-proxy",
])(
  "keeps the pending strip off nonconfiguration/mismatched-target page %s",
  (path) => {
    state.pathname = path;
    expect(render()).not.toContain("변경 내용 보기");
    state.pathname = "/nodes/node-1/logs";
  },
);
it("keeps account controls out of the rail and supplies the Apply marker only to writers", () => {
  const html = render();
  const rail = html.slice(html.indexOf("<aside"), html.indexOf("</aside>"));
  expect(rail).not.toContain("프로필");
  expect(html).toContain("data-shell-apply");
  expect(render({ role: "viewer" })).not.toContain("data-shell-apply");
});

it("preserves admin's no-snapshot top bar without Apply or pending counts", () => {
  state.pathname = "/admin/settings/general";
  const html = render({
    role: "admin",
    target: { kind: "local" },
    data: undefined,
  });
  expect(html).not.toContain("data-shell-apply");
  expect(html).not.toContain('aria-label="미적용 변경');
  expect(html).not.toContain("모두 적용됨");
  state.pathname = "/nodes/node-1/logs";
});
it("keeps the admin navigation out of a viewer's global rail", () => {
  const html = render({ role: "viewer" });
  const rail = html.slice(html.indexOf("<aside"), html.indexOf("</aside>"));
  expect(rail).not.toContain('href="/admin"');
});
