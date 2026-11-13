import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { HistoryDetail } from "./history-detail";
import { HistoryList } from "./history-list";
import { AuditList } from "../audit/audit-list";
import { AuditDetail } from "../audit/audit-detail";
import { ActionFilter } from "../audit/action-filter";
import { RollbackButton } from "@/app/(app)/history/rollback-button";
import { NodeHistoryActions } from "@/app/(node)/nodes/[id]/history/rollback-button";
const mocks = vi.hoisted(() => ({ push: vi.fn(), actions: vi.fn() }));
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
  expect(html).toContain('data-columns="2"');
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
it.each([false, true])(
  "keeps node external link and viewer restriction; write=%s",
  (canRollback) => {
    const html = renderToStaticMarkup(
      <NodeHistoryActions
        nodeUrl="https://node.example.test"
        nodeId="node-a"
        nodeName="운영"
        snapshotId="snapshot-a"
        label="이전"
        iso={rows[0]!.iso}
        retryHref="/nodes/node-a/history"
        canRollback={canRollback}
      />,
    );
    expect(html).toContain(
      'href="https://node.example.test/history/snapshot-a"',
    );
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noopener noreferrer"');
    if (canRollback) {
      expect(mocks.actions).toHaveBeenCalledWith({
        id: "node.rollbackNodeSnapshot",
        args: ["node-a", "snapshot-a"],
      });
      expect(html).toContain("secondary");
    } else {
      expect(mocks.actions).not.toHaveBeenCalled();
      expect(html).not.toContain("<form");
      expect(html).not.toContain("이 스냅샷으로 롤백");
    }
  },
);

it.each([false, true])("preserves raw snapshot records and rollback permission; write=%s", canRollback => {
 const upstream = { id: "upstream-a", name: "backend", servers: ["192.0.2.20:8080"], resolveAtRuntime: true, version: "7" };
 const html = renderToStaticMarkup(<HistoryDetail data={{ snapshot: { id: "snapshot-a", label: null, actorId: null, actorEmail: null, createdAt: rows[0]!.iso, data: { upstreams: [upstream], servers: [], streams: [] } }, iso: rows[0]!.iso, upstreams: [upstream], servers: [], streams: [], diff: { hasChanges: false, upstreams: [], servers: [], streams: [] }, isFirst: false, canRollback }} />);
 expect(html).toContain("192.0.2.20:8080");expect(html).toContain("resolveAtRuntime");expect(html).toContain("version");expect(html).toContain("이전 스냅샷과 동일합니다. (변경 없음)");
 expect(html).toContain('data-columns="2"');
 if (canRollback) { expect(html.indexOf("<form")).toBeLessThan(html.indexOf("</header>"));expect(mocks.actions).toHaveBeenCalledWith({id:"history.rollback",args:["snapshot-a"]}); }
 else { expect(html).not.toContain("<form");expect(mocks.actions).not.toHaveBeenCalled(); }
});
