import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { HistoryDetail } from "./history-detail";
import { HistoryList, NODE_HISTORY_LIST_INFO } from "./history-list";
import { AuditList } from "../audit/audit-list";
import { AuditDetail } from "../audit/audit-detail";
import { ActionFilter } from "../audit/action-filter";
import { RollbackButton } from "@/app/(app)/history/rollback-button";
import type { NodeHistoryDetailData } from "@repo/contracts";
import NodeHistoryPage from "@/app/(node)/nodes/[id]/history/page";
import NodeHistoryDetailPage from "@/app/(node)/nodes/[id]/history/[sid]/page";
const mocks = vi.hoisted(() => ({ push: vi.fn(), actions: vi.fn(), read: vi.fn() }));
vi.mock("@/lib/api-page", () => ({ readPageView: mocks.read }));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: mocks.push }) }));
vi.mock("@/lib/use-api-action", () => ({
  useApiAction: (action: unknown) => {
    mocks.actions(action);
    return async () => null;
  },
}));
beforeEach(() => vi.clearAllMocks());
const rows = ["first", "second"].map((id) => ({
  id,
  iso: "2026-11-11T09:00:00Z",
  label: id,
  actorEmail: null,
  upstreamCount: 2,
  serverCount: 3,
  streamCount: 4,
}));
it.each([true, false])(
  "marks only the latest applied row; details=%s",
  (showDetails) => {
    const html = renderToStaticMarkup(
      <HistoryList
        info="기존 안내"
        basePath={showDetails ? "/history" : undefined}
        data={{ items: rows, showDetails }}
      />,
    );
    expect(html.match(/<table\b/g)).toHaveLength(1);
    expect(html).toMatch(/<table\b[^>]*data-density="comfortable"/);
    expect(html).toMatch(/<table\b[^>]*data-narrow="summary"/);
    expect(html.match(/data-current=""/g)).toHaveLength(1);
    expect(html).toContain("현재");
    expect(html).toContain(">적용 이력</h1>");
    expect(html).toContain("기존 안내");
    expect(html.indexOf('data-current=""')).toBeLessThan(html.indexOf("first"));
    if (showDetails) {
      expect(html).toContain('href="/history/first"');
      expect(html).toContain("system");
      expect(html).toContain("Upstreams");
    } else {
      expect(html).not.toContain("Upstreams");
      expect(html).not.toContain("system");
      expect(html).not.toContain('href="/history/');
    }
  },
);
it("keeps empty history unmarked", () => {
  const html = renderToStaticMarkup(
    <HistoryList info="안내" data={{ items: [], showDetails: true }} />,
  );
  expect(html).toContain("적용 이력이 없습니다.");
  expect(html).not.toContain("data-current");
  expect(html).toMatch(/<td[^>]*colSpan="6"[^>]*>.*적용 이력이 없습니다\./s);
});
it.each(["/audit", "/nodes/node-a/audit"])(
  "preserves page/action navigation at %s",
  (basePath) => {
    const data = {
      items: [
        {
          id: "event-a",
          time: "09:00 UTC",
          actorEmail: null,
          actorLabel: "gateway",
          action: "config.apply",
          target: null,
        },
      ],
      total: 3,
      page: 2,
      totalPages: 3,
      actions: ["config.apply"],
      actionFilter: "config.apply",
    };
    const html = renderToStaticMarkup(
      <AuditList data={data} basePath={basePath} />,
    );
    expect(html).toMatch(/<table\b[^>]*data-density="dense"/);
    expect(html).toMatch(/<table\b[^>]*data-narrow="summary"/);
    expect(html).toContain(`href="${basePath}?action=config.apply"`);
    expect(html).toContain(`href="${basePath}?action=config.apply&amp;page=3"`);
    expect(html).toContain(`href="${basePath}/event-a"`);
    expect(html).toContain("gateway");
    expect(html.indexOf("다음")).toBeLessThan(html.lastIndexOf("</section>"));
    const select = ActionFilter({
      actions: data.actions,
      selected: data.actionFilter,
      basePath,
    }).props.children;
    select.props.onChange("server update&x");
    expect(mocks.push).toHaveBeenLastCalledWith(
      `${basePath}?action=server+update%26x`,
    );
    select.props.onChange("");
    expect(mocks.push).toHaveBeenLastCalledWith(basePath);
  },
);
it.each([1, 3])("keeps disabled boundary paging at page %s", (page) => {
  const html = renderToStaticMarkup(
    <AuditList
      basePath="/audit"
      data={{
        items: [],
        total: 0,
        page,
        totalPages: 3,
        actions: [],
        actionFilter: "",
      }}
    />,
  );
  expect(html).toContain("감사 로그가 없습니다.");
  expect(html).toContain("disabled");
  expect(html).not.toContain(`page=${page === 1 ? 0 : 4}`);
});
it("keeps audit raw records and missing-value fallbacks in key-value sections", () => {
  const html = renderToStaticMarkup(
    <AuditDetail
      basePath="/nodes/n/audit"
      data={{
        action: "config.apply",
        time: "09:00 UTC",
        actorEmail: null,
        actorLabel: null,
        target: null,
        ip: null,
        beforeJson: '{"secretPlaceholder":false}',
        afterJson: null,
      }}
    />,
  );
  expect(html).toContain('data-columns="1"');
  expect(html).toContain('data-layout="grid"');
  expect(html).toContain("system");
  expect(html).toContain("secretPlaceholder");
  expect(html).toContain('href="/nodes/n/audit"');
});
it("keeps local rollback command bound to its snapshot and quiet on the page", () => {
  const html = renderToStaticMarkup(
    <RollbackButton snapshotId="snapshot-a" confirmLabel="운영" />,
  );
  expect(mocks.actions).toHaveBeenCalledWith({
    id: "history.rollback",
    args: ["snapshot-a"],
  });
  expect(html).toContain("secondary");
  expect(html).not.toContain("nwui-btn-primary");
});
const node = { id: "node-a", name: "운영" };
const nodeListPath = "/nodes/node-a/history";
const params = Promise.resolve({ id: node.id, sid: "snapshot-a" });
const nodeDetail: NodeHistoryDetailData = {
  snapshot: { id: "snapshot-a", label: "이전" },
  iso: rows[0]!.iso,
  upstreams: [],
  servers: [],
  streams: [],
  diff: { hasChanges: false, upstreams: [], servers: [], streams: [] },
  comparison: "compared",
  canRollback: true,
};

it("links node labels to the gateway detail and keeps the list free of row actions", async () => {
  mocks.read.mockResolvedValueOnce({ ok: true, node, data: { snapshots: [
    { id: "a/b", label: "이전", createdAt: rows[0]!.iso },
    { id: "unlabelled", label: null, createdAt: "2026-11-11T09:00:01Z" },
    { id: "blank", label: "", createdAt: "2026-11-11T09:00:02Z" },
  ] } });
  const html = renderToStaticMarkup(await NodeHistoryPage({ params }));
  expect(mocks.read).toHaveBeenCalledWith(nodeListPath, {});
  expect(html).toContain(NODE_HISTORY_LIST_INFO);
  expect(html).toContain('href="/nodes/node-a/history/a%2Fb"');
  expect(html).toMatch(/aria-label="이전 · [^"]*6:00:00"[^>]*>이전<\/a>/);
  expect(html).toMatch(/aria-label="— · [^"]*6:00:01"[^>]*>—<\/a>/);
  expect(html).toMatch(/aria-label="— · [^"]*6:00:02"[^>]*>—<\/a>/);

  expect(html).not.toContain("스냅샷 내용은 노드에서만 볼 수 있습니다.");
  expect(html).not.toContain("노드에서 열기");
  expect(html).not.toContain("이 스냅샷으로 롤백");
  expect(html).not.toContain("조작");
});

it.each([
  ["compared", "이전 스냅샷과 동일합니다. (변경 없음)"],
  ["first", "최초 스냅샷입니다. 비교할 이전 스냅샷이 없습니다."],
  ["previous_deleted", "이전 스냅샷은 노드에서 지워졌습니다. 비교할 수 없습니다."],
  ["outside_recent", "이 스냅샷 앞의 이력은 최근 100건 밖이라 확인할 수 없습니다. 비교할 수 없습니다."],
] as const)("renders the node comparison state %s", async (comparison, message) => {
  mocks.read.mockResolvedValueOnce({ ok: true, node, data: { ...nodeDetail, comparison } });
  const html = renderToStaticMarkup(await NodeHistoryDetailPage({ params }));
  expect(mocks.read).toHaveBeenCalledWith(`${nodeListPath}/snapshot-a`, {});
  expect(html).toContain(message);
  expect(html).toContain(`href="${nodeListPath}"`);
  expect(html).toContain("적용 이력 목록");
  expect(html).not.toContain("사용자");
  expect(html).not.toContain("노드에서 열기");
  expect(html.indexOf("<form")).toBeLessThan(html.indexOf("</header>"));
  expect(mocks.actions).toHaveBeenCalledWith({ id: "node.rollbackNodeSnapshot", args: [node.id, "snapshot-a"] });
});

it("renders a node diff with raw snapshots for a user who cannot roll back", async () => {
  const upstream = { id: "u1", name: "backend", servers: ["192.0.2.20:8080"], version: "7" };
  const diff = { hasChanges: true, upstreams: [{ key: "u1", label: "backend", status: "added", changes: [] }], servers: [], streams: [] };
  mocks.read.mockResolvedValueOnce({ ok: true, node, data: { ...nodeDetail, upstreams: [upstream], diff, canRollback: false } });
  const html = renderToStaticMarkup(await NodeHistoryDetailPage({ params }));
  expect(html).toContain("backend");
  expect(html).toContain("192.0.2.20:8080");
  expect(html).not.toContain("변경 없음");
  expect(html).not.toContain("<form");
  expect(mocks.actions).not.toHaveBeenCalled();
});

it.each([
  { kind: "not_found", title: "그 스냅샷을 노드에서 찾을 수 없습니다", link: nodeListPath },
  { kind: "unreachable", title: "노드에 연결할 수 없습니다", link: `${nodeListPath}/snapshot-a` },
  { kind: "node_error", title: "스냅샷 응답이 1 MiB를 초과해 열 수 없습니다.", link: `${nodeListPath}/snapshot-a` },
])("renders a node detail failure for $kind", async ({ kind, title, link }) => {
  mocks.read.mockResolvedValueOnce({ ok: false, nodeError: {
    nodeId: node.id, retryHref: `${nodeListPath}/snapshot-a`, listHref: nodeListPath,
    subject: "그 스냅샷을", failure: { kind, message: "원격 응답" },
    nodeErrorTitle: kind === "node_error" ? title : undefined,
  } });
  const html = renderToStaticMarkup(await NodeHistoryDetailPage({ params }));
  expect(html).toMatch(/<h1[^>]*>적용 이력<\/h1>/);
  expect(html).toContain(title);
  expect(html).toContain(`href="${link}"`);
  expect(html).not.toContain("노드의 로그를 확인하세요.");
  expect(html).not.toContain("<form");
});

it.each([false, true])("preserves raw snapshot records and rollback permission; write=%s", canRollback => {
 const upstream = { id: "upstream-a", name: "backend", servers: ["192.0.2.20:8080"], resolveAtRuntime: true, version: "7" };
 const html = renderToStaticMarkup(<HistoryDetail data={{ snapshot: { id: "snapshot-a", label: null, actorId: null, actorEmail: null, createdAt: rows[0]!.iso, data: { upstreams: [upstream], servers: [], streams: [] } }, iso: rows[0]!.iso, upstreams: [upstream], servers: [], streams: [], diff: { hasChanges: false, upstreams: [], servers: [], streams: [] }, isFirst: false, canRollback }} />);
 expect(html).toContain("192.0.2.20:8080");expect(html).toContain("resolveAtRuntime");expect(html).toContain("version");expect(html).toContain("이전 스냅샷과 동일합니다. (변경 없음)");
 expect(html).toContain('data-columns="1"');
 if (canRollback) { expect(html.indexOf("<form")).toBeLessThan(html.indexOf("</header>"));expect(mocks.actions).toHaveBeenCalledWith({id:"history.rollback",args:["snapshot-a"]}); }
 else { expect(html).not.toContain("<form");expect(mocks.actions).not.toHaveBeenCalled(); }
});

it.each([true, false])(
  "renders empty history inside a spanning table row; details=%s",
  (showDetails) => {
    const html = renderToStaticMarkup(
      <HistoryList
        info="안내"
        data={{ items: [], showDetails }}
      />,
    );
    expect(html).toMatch(
      new RegExp(
        `<td[^>]*colSpan="${showDetails ? 6 : 2}"[^>]*>.*적용 이력이 없습니다\\.`,
        "s",
      ),
    );
  },
);
it.each(["", "config.apply"])(
  "distinguishes filtered audit empty state; filter=%s",
  (actionFilter) => {
    const html = renderToStaticMarkup(
      <AuditList
        basePath="/audit"
        data={{
          items: [],
          total: 0,
          page: 1,
          totalPages: 1,
          actions: [],
          actionFilter,
        }}
      />,
    );
    expect(html).toMatch(/<td[^>]*colSpan="5"[^>]*>.*<p[^>]*>/s);
    expect(html).toContain(
      actionFilter
        ? "해당 액션의 감사 로그가 없습니다."
        : "감사 로그가 없습니다.",
    );
  },
);

it("renders the shared history diff table with explicit missing values", () => {
  const html = renderToStaticMarkup(<HistoryDetail data={{
    snapshot: { id: "snapshot-a", label: "운영", actorId: null, actorEmail: null, createdAt: rows[0]!.iso, data: { upstreams: [], servers: [], streams: [] } },
    iso: rows[0]!.iso, upstreams: [], servers: [], streams: [], isFirst: false, canRollback: false,
    diff: { hasChanges: true, upstreams: [], streams: [], servers: [{ key: "server-a", label: "api.example.test", status: "changed", changes: [{ field: "listen", before: null, after: 8080 }] }] },
  }} />);
  expect(html).toContain('data-variant="panel"');
  for (const title of ["설정", "이전", "이후"]) expect(html).toContain(`<th scope="col">${title}</th>`);
  expect(html).toContain(">없음<");
  expect(html).toContain("8080");
  expect(html).not.toContain("→");
});
