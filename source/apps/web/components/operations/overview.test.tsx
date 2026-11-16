import { Children, isValidElement, type ReactNode } from "react";
import styles from "./overview.module.css";
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { OverviewVM } from "@/lib/overview-data";
import { OperationsOverview } from "./overview";

// An editor's overview: every permission that could offer creating a reverse proxy.
const vm: OverviewVM = {
  email: "ops@example.test",
  counts: { serverTotal: 2, serverActive: 1, streamTotal: 0, streamActive: 0, audit24h: 3 },
  gauge: null,
  servers: {
    ok: true,
    data: { rows: [], total: 2, attention: 0, pendingCount: 0, pendingHasBaseline: true, pendingItems: [], canApply: true, canWrite: true, certificatesUnknown: false },
  },
  certificates: { ok: true, data: { total: 0, expiring: 0, failed: 0 } },
  nodes: { ok: true, data: [] },
  events: { ok: true, data: [] },
  lastApply: { ok: true, data: null },
};

describe("OperationsOverview", () => {
  // The overview summarises; it does not create. 「새 서버」 registered a reverse proxy, not a server,
  // so it is gone: creation stays on the reverse-proxy list.
  it("offers no new-server action, even to a user who may create", () => {
    const html = renderToStaticMarkup(<OperationsOverview vm={vm} />);
    expect(html).not.toContain("새 서버");
    expect(html).not.toMatch(/href="\/reverse-proxy\/new"/);
    expect(html).toMatch(/href="\/reverse-proxy"/);
  });
});

it.each([0,2])("overview never claims unconfirmed settings are applied with %s changes", (pendingCount) => {
  if (!vm.servers.ok) throw new Error("fixture");
  const servers = {ok: true as const, data: {...vm.servers.data, pendingCount, pendingHasBaseline: false}};
  const html = renderToStaticMarkup(<OperationsOverview vm={{...vm, servers}} />);
  expect(html).toContain("적용 상태 확인 필요");
  expect(html).not.toContain("모든 변경이 nginx에 반영되어 있습니다");
});

it("keeps one page heading and reports only the supplied attention counts", () => {
  if (!vm.servers.ok) throw new Error("fixture");
  const html = renderToStaticMarkup(<OperationsOverview vm={{...vm, servers: {ok:true, data:{...vm.servers.data, attention:1}}}} />);
  expect(html.match(/<h1\b/g)).toHaveLength(1);
  expect(html).toContain("저장된 설정·인증서 기준");
  expect(html).toContain("1개");
  expect(html).not.toContain("p95");
});

it("discloses attention items outside the supplied row slice", () => {
  if (!vm.servers.ok) throw new Error("fixture");
  const html = renderToStaticMarkup(<OperationsOverview vm={{...vm, servers:{ok:true, data:{...vm.servers.data, attention:3}}}} />);
  expect(html).toContain("외 3건");
  expect(html).toContain("전체 보기");
  expect(html).toContain("개요");
});

it("shows only the node count and list link, without node identities", () => {
  const nodes = [
    { id:"private-node-a", name:"private-node-name-a", tone:"success" as const, status:"private-node-status-a", version:"private-node-version-a" },
    { id:"private-node-b", name:"private-node-name-b", tone:"error" as const, status:"private-node-status-b", version:"private-node-version-b" },
  ];
  const html = renderToStaticMarkup(<OperationsOverview vm={{...vm, nodes:{ok:true, data:nodes}}} />);
  expect(html.replace(/<[^>]*>/g, "")).toContain("연결된 노드 2대");
  expect(html).toContain('href="/nodes"');
  expect(html).not.toMatch(/href="\/nodes\//);
  for (const node of nodes) for (const value of [node.id, node.name, node.status, node.version]) expect(html).not.toContain(value);
});

it("keeps empty, failed and forbidden node states", () => {
  const render = (nodes: OverviewVM["nodes"]) => renderToStaticMarkup(<OperationsOverview vm={{...vm, nodes}} />).replace(/<[^>]*>/g, "");
  expect(render({ok:true, data:[]})).toContain("연결된 노드가 없습니다.");
  expect(render({ok:false, reason:"error", message:"노드 목록 읽기 실패"})).toContain("읽지 못했습니다: 노드 목록 읽기 실패");
  expect(render({ok:false, reason:"forbidden"})).not.toContain("노드 관리");
});

it("groups overview panels into two independent content stacks", () => {
  const page = OperationsOverview({ vm });
  const grid = Children.toArray(page.props.children).find((child) => isValidElement<{className?: string}>(child) && child.props.className === styles.overviewGrid);
  if (!isValidElement<{children: ReactNode}>(grid)) throw new Error("Missing overview grid");
  const stacks = Children.toArray(grid.props.children);
  const headings = stacks.map((stack) => [...renderToStaticMarkup(stack).matchAll(/<h2[^>]*>(.*?)<\/h2>/g)].map((match) => match[1]));
  expect(headings).toEqual([["서버 상태", "최근 이벤트"], ["확인 필요", "미적용 변경 0건", "구성", "노드"]]);
});

it("moves available gauge context into a tip but keeps missing data visible", () => {
  const withoutTips = (html: string) => html.replace(/<div[^>]*role="tooltip"[^>]*hidden=""[^>]*>.*?<\/div>/g, "");
  const missing = renderToStaticMarkup(<OperationsOverview vm={vm} />);
  expect(withoutTips(missing)).toContain("nginx 수치를 아직 수집하지 못했습니다.");
  const ready = renderToStaticMarkup(<OperationsOverview vm={{ ...vm, gauge: {active: 1, requests: 1, waiting: 0, reading: 0, writing: 1, accepts: 1, handled: 1} }} />);
  expect(ready).toContain("nginx 수치는 이 화면을 연 시각의 stub_status 스냅샷입니다.");
  expect(withoutTips(ready)).not.toContain("stub_status 스냅샷");
});

it("uses a shared telemetry surface and configuration facts", () => {
  const html = renderToStaticMarkup(<OperationsOverview vm={vm} />);
  expect(html).toMatch(/<h2[^>]*>nginx 현황<\/h2>/);
  expect(html).toContain('data-layout="grid"');
  expect(html).toContain('data-columns="1"');
  expect(html).not.toContain("로컬 서버");
});
