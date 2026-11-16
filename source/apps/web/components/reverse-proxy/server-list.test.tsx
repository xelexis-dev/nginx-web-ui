import { expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

/**
 * The Operations server list, rendered. What must hold: write controls (row switch, 수정, 새 서버)
 * appear only with write permission; the switch reflects the real enabled state; 미적용 and TLS are
 * said in text; an unreadable certificate list is announced; the metrics explanation is non-interactive.
 */

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }),
}));

import { ServerList, type ServerListProps } from "./server-list";
import type { ServerOpsRowVM } from "./server-status";
import { toServerRow } from "./server-vm";

const row = (over: Partial<ServerOpsRowVM>): ServerOpsRowVM => ({
  id: "s1",
  serverName: "api.example.test www.example.test",
  listen: 443,
  enabled: true,
  targetLabel: "upstream: app_pool",
  targets: [{ path: null, label: "upstream: app_pool" }],
  certificateId: "c1",
  tls: {
    label: "만료 6일 전",
    tone: "warning",
    problem: "인증서 만료 6일 전",
    unknown: false,
  },
  health: { tone: "warning", label: "주의", reason: "인증서 만료 6일 전" },
  pending: true,
  ...over,
});

const props = (over: Partial<ServerListProps> = {}): ServerListProps => ({
  basePath: "/nodes/n1/reverse-proxy",
  data: {
    items: [
      row({}),
      row({
        id: "s2",
        serverName: "old.example.test",
        enabled: false,
        pending: false,
        tls: { label: "HTTP", tone: "neutral", problem: null, unknown: false },
        health: { tone: "neutral", label: "비활성", reason: null },
      }),
    ],
    certificatesUnknown: false,
    canApply: true,
    pendingCount: 1,
    pendingHasBaseline: true,
    pendingDiff: { servers: [], streams: [], upstreams: [], hasChanges: true },
  },
  write: {
    newHref: "/nodes/n1/reverse-proxy/new",
    templatesHref: null,
    toggleAction: { id: "node.toggleNodeServer", args: ["n1"] },
  },
  ...over,
});

it("with write permission: a switch per row in its real state, 수정 and 새 서버", () => {
  const html = renderToStaticMarkup(<ServerList {...props()} />);
  expect(html.match(/role="switch"/g)).toHaveLength(2);
  expect(html).toContain(
    'aria-checked="true" aria-label="api.example.test 활성"',
  );
  expect(html).toContain(
    'aria-checked="false" aria-label="old.example.test 활성"',
  );
  expect(html).toContain('aria-label="api.example.test 수정"');
  expect(html).toContain('href="/nodes/n1/reverse-proxy/new"');
  // 템플릿은 노드에 없다.
  expect(html).not.toContain("템플릿으로 생성");
});

it("viewer: no switch, no 수정, no 새 서버 — the table still reads", () => {
  const html = renderToStaticMarkup(<ServerList {...props({ write: null })} />);
  expect(html).not.toContain('role="switch"');
  expect(html).not.toContain("수정");
  expect(html).not.toContain("새 서버");
  expect(html).toContain("api.example.test");
});

it("says 미적용 and TLS in text, shows extra names as a count, keeps metrics guidance in the page tip", () => {
  const html = renderToStaticMarkup(<ServerList {...props()} />);
  expect(html).toContain('data-state="pending"');
  expect(html).toContain(">미적용<");
  expect(html).toContain("만료 6일 전");
  expect(html).toContain(" +1");
  const tip = html.match(/<div[^>]*role="tooltip"[^>]*hidden=""[^>]*>(.*?)<\/div>/)![1];
  expect(tip).toContain("트래픽은 접속 지표에서 봅니다.");
  expect(tip).not.toContain("<a");
  expect(html).not.toContain('href="/nodes/n1/metrics"');
  for (const [term, value] of [
    ["서버", "2개"],
    ["활성", "1"],
    ["미적용", "1"],
  ]) {
    expect(html).toMatch(
      new RegExp(`<dt[^>]*>${term}</dt>.*?<dd[^>]*>${value}</dd>`),
    );
  }
});

it("announces an unreadable certificate list", () => {
  const html = renderToStaticMarkup(
    <ServerList
      {...props({ data: { ...props().data, certificatesUnknown: true } })}
    />,
  );
  expect(html).toContain("인증서 상태를 읽지 못했습니다");
});

it("no servers: an empty state with 새 서버 only for writers", () => {
  const empty = { ...props().data, items: [] };
  expect(
    renderToStaticMarkup(<ServerList {...props({ data: empty })} />),
  ).toContain("등록된 서버가 없습니다");
  expect(
    renderToStaticMarkup(
      <ServerList {...props({ data: empty, write: null })} />,
    ),
  ).not.toContain("새 서버");
});

it("a server with locations lists every path and its own target instead of —", () => {
  const multi = row({
    id: "s3",
    serverName: "multi.example.test",
    targetLabel:
      "/api → upstream: api_pool · /static → proxy_pass: http://static.internal.example.test:8080",
    targets: [
      { path: "/api", label: "upstream: api_pool" },
      {
        path: "/static",
        label: "proxy_pass: http://static.internal.example.test:8080",
      },
    ],
  });
  const html = renderToStaticMarkup(
    <ServerList
      {...props({ data: { ...props().data, items: [row({}), multi] } })}
    />,
  );
  const cell =
    /<td[^>]*title="\/api → [^"]*"[^>]*>[\s\S]*?<\/td>/.exec(html)?.[0] ?? "";
  expect(cell).toContain("/api");
  expect(cell).toContain("upstream: api_pool");
  expect(cell).toContain("/static");
  expect(cell).toContain(
    "proxy_pass: http://static.internal.example.test:8080",
  );
  expect(cell).not.toContain("—");
  // Wraps rather than cutting a location off; the single-target cell keeps its one truncated line.
  expect(cell).not.toContain("data-truncate");
  expect(cell).toContain("data-wrap");
  expect(html).toMatch(
    /<td[^>]*data-truncate=""[^>]*title="upstream: app_pool"|<td[^>]*title="upstream: app_pool"[^>]*data-truncate=""/,
  );
});

it.each([0, 2])(
  "uncertain list status remains visible with %s changes",
  (pendingCount) => {
    const data = { ...props().data, pendingCount, pendingHasBaseline: false };
    expect(renderToStaticMarkup(<ServerList {...props({ data })} />)).toContain(
      "적용 상태 확인 필요",
    );
  },
);

it("shows projected upstream names and a missing-upstream badge in local and node lists", () => {
  for (const basePath of ["/reverse-proxy", "/nodes/n1/reverse-proxy"]) {
    const target = toServerRow(
      {
        id: "s1",
        serverName: "api.test",
        listen: 80,
        enabled: true,
        upstreamId: null,
        config: {
          locations: [
            { path: "/id", upstreamId: "up-api", upstreamName: "renamed_pool" },
            { path: "/missing", upstreamId: "foreign-id" },
          ],
        },
      },
      new Map(),
    );
    const html = renderToStaticMarkup(
      <ServerList
        {...props({
          basePath,
          write: null,
          data: { ...props().data, items: [row(target)] },
        })}
      />,
    );
    expect(html).toContain("upstream: renamed_pool");
    expect(html).toMatch(/<span[^>]*>Upstream 없음<\/span>/);
    expect(html).not.toContain("foreign-id");
  }
});

it("places supplied local family tabs below the PageHeader without changing node links", () => {
  const html = renderToStaticMarkup(
    <ServerList
      {...props()}
      navigation={
        <nav aria-label="프록시 구성">
          <a href="/reverse-proxy">서버</a>
        </nav>
      }
    />,
  );
  expect(html.indexOf('aria-label="프록시 구성"')).toBeGreaterThan(
    html.indexOf("</header>"),
  );
  expect(html).toContain('href="/nodes/n1/reverse-proxy/s1"');
});

it.each(["/reverse-proxy", "/nodes/n1/reverse-proxy"])("labels both stacked row controls on %s", (basePath) => {
  const html = renderToStaticMarkup(<ServerList {...props({ basePath })} />);
  const cells = [...html.matchAll(/<td\b[^>]*data-cell="action"[^>]*>[\s\S]*?<\/td>/g)].map(match => match[0]);
  expect(cells).toHaveLength(4);
  for (let index = 0; index < cells.length; index += 2) {
    expect(cells[index]).toContain('data-label="활성"');
    expect(cells[index]).toContain('role="switch"');
    expect(cells[index + 1]).toContain('data-label="작업"');
    expect(cells[index + 1]).toContain(`href="${basePath}/`);
  }
});
