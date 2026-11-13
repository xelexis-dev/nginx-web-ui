import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AuditListView, CertificateListView, DashboardView, HistoryView, NodesListView, ServerListView } from "@repo/contracts";
import { OVERVIEW_EVENTS, OVERVIEW_ROWS, toOverviewVM, type OverviewSources } from "./overview-data";

/**
 * 개요는 이미 있는 뷰들을 모아 그린다. 여기서는 (1) 보조 읽기의 실패가 그 칸에만 머무는지,
 * (2) 인증서를 못 읽으면 추측하지 않는지, (3) 숫자가 실제 데이터에서만 나오는지를 붙든다.
 */

const NOW = new Date("2026-09-27T00:00:00Z");
beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
});
afterEach(() => vi.useRealTimers());

const DASHBOARD = {
  email: "admin@example.test",
  serverTotal: 3,
  serverActive: 2,
  streamTotal: 1,
  streamActive: 1,
  audit24h: 5,
  metrics: { active: 3, accepts: 10, handled: 9, requests: 100, reading: 0, writing: 1, waiting: 2 },
} as DashboardView;

const server = (id: string, over: Record<string, unknown> = {}) => ({
  id,
  serverName: `${id}.example.test`,
  listen: 80,
  enabled: true,
  upstreamId: null,
  certificateId: null,
  config: {},
  ...over,
});

const SERVERS = {
  canWrite: true,
  canApply: true,
  upstreams: [],
  servers: [server("ok"), server("tls", { certificateId: "c-soon" }), server("off", { enabled: false })],
  pending: {
    count: 1,
    hasBaseline: true,
    diff: { servers: [{ key: "ok", label: "ok.example.test", status: "changed", changes: [] }], streams: [], upstreams: [], hasChanges: true },
  },
} as unknown as ServerListView;

const iso = (days: number) => new Date(NOW.getTime() + days * 86_400_000).toISOString();
const CERTS = {
  canWrite: true,
  data: {
    items: [
      { id: "c-soon", domains: ["tls.example.test"], alias: null, status: "issued", source: "acme", staging: false, expiresAt: iso(10), linkedCount: 1 },
      { id: "c-later", domains: ["x.example.test"], alias: null, status: "issued", source: "acme", staging: false, expiresAt: iso(90), linkedCount: 0 },
      { id: "c-fail", domains: ["y.example.test"], alias: null, status: "failed", source: "acme", staging: false, expiresAt: null, linkedCount: 0 },
    ],
  },
} as unknown as CertificateListView;

const sources = (over: Partial<OverviewSources> = {}): OverviewSources => ({
  dashboard: DASHBOARD,
  servers: { ok: true, data: SERVERS },
  certificates: { ok: true, data: CERTS },
  nodes: { ok: true, data: { nodes: [], canWrite: true, unpairWarningText: null } as unknown as NodesListView },
  audit: { ok: true, data: { items: [], total: 0, page: 1, totalPages: 1, actions: [], actionFilter: "" } as unknown as AuditListView },
  history: { ok: true, data: { data: { items: [], showDetails: true } } as unknown as HistoryView },
  ...over,
});

describe("toOverviewVM", () => {
  it("builds the server health rows from real rows, certificates and the pending diff", () => {
    const vm = toOverviewVM(sources());
    if (!vm.servers.ok) throw new Error("servers should be ok");
    expect(vm.servers.data.rows.map((r) => [r.id, r.health.label, r.pending])).toEqual([
      ["tls", "주의", false],
      ["ok", "활성", true],
      ["off", "비활성", false],
    ]);
    expect(vm.servers.data).toMatchObject({ total: 3, attention: 1, pendingCount: 1, certificatesUnknown: false });
  });

  it("counts expiring certificates with the API's 30-day rule and failed ones apart", () => {
    const vm = toOverviewVM(sources());
    expect(vm.certificates).toEqual({ ok: true, data: { total: 3, expiring: 1, failed: 1 } });
  });

  it("a failed certificate read stays in its panel and TLS is not guessed", () => {
    const vm = toOverviewVM(sources({ certificates: { ok: false, reason: "error", message: "API 서버에 연결하지 못했습니다." } }));
    expect(vm.certificates).toEqual({ ok: false, reason: "error", message: "API 서버에 연결하지 못했습니다." });
    if (!vm.servers.ok) throw new Error("servers should still be ok");
    expect(vm.servers.data.certificatesUnknown).toBe(true);
    const tls = vm.servers.data.rows.find((r) => r.id === "tls");
    expect(tls?.tls).toMatchObject({ label: "확인 불가", unknown: true });
    // The server with a certificate is 확인 불가, never 활성; the one without a certificate is unaffected.
    expect(tls?.health).toMatchObject({ label: "확인 불가", tone: "neutral" });
    expect(vm.servers.data.rows.find((r) => r.id === "ok")?.health.label).toBe("활성");
    expect(vm.servers.data.attention).toBe(0);
  });

  it("forbidden panels are passed on as forbidden, the rest still render", () => {
    const vm = toOverviewVM(sources({ nodes: { ok: false, reason: "forbidden" }, audit: { ok: false, reason: "forbidden" } }));
    expect(vm.nodes).toEqual({ ok: false, reason: "forbidden" });
    expect(vm.events).toEqual({ ok: false, reason: "forbidden" });
    expect(vm.servers.ok).toBe(true);
    expect(vm.counts.serverTotal).toBe(3);
  });

  it("labels only overview node-event targets generically and keeps action and actor", () => {
    const audit = {
      items: [
        { id:"node-event", createdAt:iso(0), action:"node:pair", target:"edge-02", actorEmail:"operator@example.test", actorLabel:null, ip:null },
        { id:"server-event", createdAt:iso(0), action:"server.update", target:"api.example.test", actorEmail:null, actorLabel:"token:test", ip:null },
      ], total:2, page:1, totalPages:1, actions:[], actionFilter:"",
    } as unknown as AuditListView;
    const vm = toOverviewVM(sources({ audit:{ok:true, data:audit} }));
    expect(vm.events.ok && vm.events.data.map(({action,target,actor}) => [action,target,actor])).toEqual([
      ["node:pair", "노드", "operator@example.test"],
      ["server.update", "api.example.test", "token:test"],
    ]);
    expect(audit.items[0]!.target).toBe("edge-02");
  });

  it("caps rows and events", () => {
    const many = { ...SERVERS, servers: Array.from({ length: 12 }, (_, i) => server(`s${i}`)) } as unknown as ServerListView;
    const audit = {
      items: Array.from({ length: 12 }, (_, i) => ({ id: `e${i}`, createdAt: iso(-i), action: "server.update", target: "x", actorEmail: "a@example.test", actorLabel: null, ip: null })),
      total: 12,
      page: 1,
      totalPages: 1,
      actions: [],
      actionFilter: "",
    } as unknown as AuditListView;
    const vm = toOverviewVM(sources({ servers: { ok: true, data: many }, audit: { ok: true, data: audit } }));
    expect(vm.servers.ok && vm.servers.data.rows.length).toBe(OVERVIEW_ROWS);
    expect(vm.servers.ok && vm.servers.data.total).toBe(12);
    expect(vm.events.ok && vm.events.data.length).toBe(OVERVIEW_EVENTS);
  });

  it("last apply is the newest history item, or null when there is none", () => {
    expect(toOverviewVM(sources()).lastApply).toEqual({ ok: true, data: null });
    const history = { data: { items: [{ id: "h1", iso: iso(0), label: "적용", actorEmail: "a@example.test", upstreamCount: 0, serverCount: 1, streamCount: 0 }], showDetails: true } } as unknown as HistoryView;
    const vm = toOverviewVM(sources({ history: { ok: true, data: history } }));
    expect(vm.lastApply.ok && vm.lastApply.data).toMatchObject({ label: "적용", actor: "a@example.test" });
  });
});

it.each([0,2])("overview adapter preserves uncertainty with %s changes", (count) => {
  const view = {...SERVERS, pending: {...SERVERS.pending, count, hasBaseline: false}};
  const vm = toOverviewVM(sources({servers: {ok: true, data: view}}));
  expect(vm.servers).toMatchObject({ok: true, data: {pendingCount: count, pendingHasBaseline: false, canApply: true}});
});
