import type {
  AuditListView,
  CertificateListView,
  DashboardView,
  HistoryView,
  NginxGaugeView,
  NodesListView,
  ServerListView,
} from "@repo/contracts";
import { formatAuditTime } from "@/components/audit/audit-list";
import { readInstant } from "@/components/certificates/certificate-vm";
import {
  byHealth,
  certificateIndex,
  isExpiringSoon,
  pendingIndex,
  toOpsRow,
  type CertificateIndex,
  type ServerOpsRowVM,
  type Tone,
} from "@/components/reverse-proxy/server-status";
import { mapOptional, type Optional } from "./optional";

/**
 * 개요(Operations) 화면이 모으는 것. **API 를 바꾸지 않고** 이미 있는 뷰들을 읽어 합친다:
 *
 * | 칸 | 출처 | 필수 |
 * |---|---|---|
 * | 개수·nginx 수치 | `/` (DashboardView) | 예 — 실패하면 페이지 오류 |
 * | 서버 상태·미적용 | `/reverse-proxy` (ServerListView) | 아니오 |
 * | TLS·만료 임박 | `/certificates` (CertificateListView) | 아니오 |
 * | 노드 | `/nodes` (NodesListView) | 아니오 |
 * | 최근 이벤트 | `/audit` 첫 쪽 (AuditListView) | 아니오 |
 * | 마지막 적용 | `/history` (HistoryView) | 아니오 |
 *
 * 보조 칸은 각자 실패한다(`Optional`) — 한 칸이 못 읽혀도 나머지는 그린다. nginx -t 를 돌리는
 * `/api/v1/status` 는 읽지 않는다(페이지를 열 때마다 설정 검사를 돌리지 않는다). 폴링도 없다.
 * p95 와 「적용 실패」 이력은 출처가 없어 보여 주지 않는다.
 */

export interface OverviewSources {
  dashboard: DashboardView;
  servers: Optional<ServerListView>;
  certificates: Optional<CertificateListView>;
  nodes: Optional<NodesListView>;
  audit: Optional<AuditListView>;
  history: Optional<HistoryView>;
}

export interface OverviewNodeVM {
  id: string;
  name: string;
  tone: Tone;
  status: string;
  version: string | null;
}

export interface OverviewEventVM {
  id: string;
  time: string;
  action: string;
  target: string | null;
  actor: string | null;
}

export interface OverviewVM {
  /** 로그인한 사용자(원격 게이트웨이 세션처럼 사람이 아니면 `null`). */
  email: string | null;
  counts: { serverTotal: number; serverActive: number; streamTotal: number; streamActive: number; audit24h: number };
  /** stub_status 스냅샷. 수집기가 아직 값을 못 가졌으면 `null`. */
  gauge: NginxGaugeView | null;
  servers: Optional<{
    /** 확인 필요 → 주의 → 확인 불가 → 활성 → 비활성 순의 앞부분(저장된 설정·인증서 기준). */
    rows: ServerOpsRowVM[];
    total: number;
    attention: number;
    pendingCount: number;
    pendingHasBaseline: boolean;
    pendingItems: { key: string; label: string; status: string }[];
    canApply: boolean;
    canWrite: boolean;
    /** 인증서 목록을 못 읽었으면 true — TLS 칸이 「확인 불가」인 이유. */
    certificatesUnknown: boolean;
  }>;
  certificates: Optional<{ total: number; expiring: number; failed: number }>;
  nodes: Optional<OverviewNodeVM[]>;
  events: Optional<OverviewEventVM[]>;
  lastApply: Optional<{ time: string; label: string | null; actor: string | null } | null>;
}

/** 개요의 서버 상태 표에 보이는 줄 수. 나머지는 「전체 N개」로 목록에 넘긴다. */
export const OVERVIEW_ROWS = 8;
/** 최근 이벤트 줄 수. */
export const OVERVIEW_EVENTS = 7;

const NODE_STATUS: Record<string, { label: string; tone: Tone }> = {
  ok: { label: "응답함", tone: "success" },
  unreachable: { label: "응답 없음", tone: "error" },
  unauthorized: { label: "연결 끊김", tone: "error" },
  version_mismatch: { label: "버전 불일치", tone: "warning" },
  error: { label: "오류", tone: "error" },
};

export function toOverviewVM(src: OverviewSources): OverviewVM {
  const certs: CertificateIndex = src.certificates.ok
    ? certificateIndex(src.certificates.data.data.items.map((c) => ({ id: c.id, status: c.status, expiresAt: readInstant(c.expiresAt) })))
    : null;

  return {
    email: src.dashboard.email,
    counts: {
      serverTotal: src.dashboard.serverTotal,
      serverActive: src.dashboard.serverActive,
      streamTotal: src.dashboard.streamTotal,
      streamActive: src.dashboard.streamActive,
      audit24h: src.dashboard.audit24h,
    },
    gauge: src.dashboard.metrics ?? null,
    servers: mapOptional(src.servers, (view) => {
      const names = new Map(view.upstreams.map((u) => [u.id, u.name]));
      const pending = pendingIndex(view.pending.diff);
      const rows = view.servers.map((s) => toOpsRow(s, names, certs, pending)).sort(byHealth);
      return {
        rows: rows.slice(0, OVERVIEW_ROWS),
        total: rows.length,
        attention: rows.filter((r) => r.health.tone === "error" || r.health.tone === "warning").length,
        pendingCount: view.pending.count,
        pendingHasBaseline: view.pending.hasBaseline === true,
        pendingItems: [...view.pending.diff.servers, ...view.pending.diff.upstreams, ...view.pending.diff.streams]
          .filter((i) => i.status !== "skipped")
          .map((i) => ({ key: i.key, label: i.label, status: i.status })),
        canApply: view.canApply,
        canWrite: view.canWrite,
        certificatesUnknown: certs === null,
      };
    }),
    certificates: mapOptional(src.certificates, (view) => {
      const items = view.data.items;
      const expiring = items.filter((c) => isExpiringSoon(readInstant(c.expiresAt))).length;
      return { total: items.length, expiring, failed: items.filter((c) => c.status === "failed").length };
    }),
    nodes: mapOptional(src.nodes, (view) =>
      view.nodes.map((n) => {
        const status = n.lastStatus ? NODE_STATUS[n.lastStatus] : undefined;
        return {
          id: n.id,
          name: n.name,
          tone: status?.tone ?? "neutral",
          status: status?.label ?? (n.lastStatus === null ? "확인 전" : n.lastStatus),
          version: n.lastVersion ?? null,
        };
      }),
    ),
    events: mapOptional(src.audit, (view) =>
      view.items.slice(0, OVERVIEW_EVENTS).map((row) => ({
        id: row.id,
        time: formatAuditTime(new Date(row.createdAt)),
        action: row.action,
        target: row.action.startsWith("node:") ? "노드" : row.target ?? null,
        actor: row.actorEmail ?? row.actorLabel ?? null,
      })),
    ),
    lastApply: mapOptional(src.history, (view) => {
      const latest = view.data.items[0];
      return latest ? { time: formatAuditTime(new Date(latest.iso)), label: latest.label, actor: latest.actorEmail ?? null } : null;
    }),
  };
}
