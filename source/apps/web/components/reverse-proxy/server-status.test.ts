import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ConfigDiff } from "@/lib/config-diff";
import {
  EXPIRING_DAYS,
  byHealth,
  certificateIndex,
  healthOf,
  isExpiringSoon,
  pendingIndex,
  tlsOf,
  toOpsRow,
  type CertificateSummary,
} from "./server-status";

/**
 * Operations 목록·개요의 상태 판정. 전부 실제 데이터(서버 행, 인증서 목록, 미적용 diff)에서만
 * 나오는지, 그리고 읽지 못한 것을 추측하지 않는지를 붙든다.
 */

const NOW = new Date("2026-09-27T00:00:00Z");
const day = (n: number) => new Date(NOW.getTime() + n * 24 * 60 * 60 * 1000);
const cert = (over: Partial<CertificateSummary> = {}): CertificateSummary => ({ id: "c1", status: "issued", expiresAt: day(90), ...over });

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
});
afterEach(() => vi.useRealTimers());

describe("tlsOf", () => {
  it("no certificate is plain HTTP, not a problem and not unknown", () => {
    expect(tlsOf(null, certificateIndex([]))).toEqual({ label: "HTTP", tone: "neutral", problem: null, unknown: false });
    // Even with the list unreadable: a server without a certificate needs no list.
    expect(tlsOf(null, null).unknown).toBe(false);
  });

  it("does not guess when the certificate list could not be read", () => {
    expect(tlsOf("c1", null)).toEqual({ label: "확인 불가", tone: "neutral", problem: null, unknown: true });
  });

  it("an issued certificate without an expiry date is unknown, not healthy", () => {
    expect(tlsOf("c1", certificateIndex([cert({ expiresAt: null })]))).toMatchObject({ label: "만료일 모름", tone: "neutral", unknown: true });
  });

  it("a status the API adds later is unknown, not read with the issued rules", () => {
    expect(tlsOf("c1", certificateIndex([cert({ status: "revoked", expiresAt: day(90) })]))).toMatchObject({ label: "상태 모름", unknown: true });
  });

  it("a certificate id missing from the list is a deleted certificate", () => {
    expect(tlsOf("gone", certificateIndex([cert()]))).toMatchObject({ label: "인증서 삭제됨", tone: "error" });
  });

  it("uses the API's exact boundary: expires_at <= now + 30 days", () => {
    const at = (ms: number) => new Date(NOW.getTime() + ms);
    const thirty = 30 * 86_400_000;
    expect(isExpiringSoon(at(thirty))).toBe(true);
    expect(isExpiringSoon(at(thirty + 1))).toBe(false);
    // Flooring the days left would count 30.5 days as expiring; the API does not.
    expect(isExpiringSoon(at(thirty + 12 * 3_600_000))).toBe(false);
    expect(tlsOf("c1", certificateIndex([cert({ expiresAt: at(thirty + 12 * 3_600_000) })]))).toMatchObject({ label: "HTTPS · 30일", tone: "success" });
    expect(isExpiringSoon(null)).toBe(false);
  });

  it("uses the API's 30-day rule for 'expiring'", () => {
    expect(EXPIRING_DAYS).toBe(30);
    expect(tlsOf("c1", certificateIndex([cert({ expiresAt: day(31) })]))).toMatchObject({ label: "HTTPS · 31일", tone: "success" });
    expect(tlsOf("c1", certificateIndex([cert({ expiresAt: day(30) })]))).toMatchObject({ label: "만료 30일 전", tone: "warning" });
    expect(tlsOf("c1", certificateIndex([cert({ expiresAt: day(-1) })]))).toMatchObject({ label: "만료됨", tone: "error" });
  });

  it("issuance states", () => {
    expect(tlsOf("c1", certificateIndex([cert({ status: "failed", expiresAt: null })]))).toMatchObject({ label: "발급 실패", tone: "error" });
    expect(tlsOf("c1", certificateIndex([cert({ status: "issuing", expiresAt: null })]))).toMatchObject({ label: "발급 중", tone: "warning" });
    expect(tlsOf("c1", certificateIndex([cert({ status: "awaiting-dns", expiresAt: null })]))).toMatchObject({ label: "TXT 대기", tone: "warning" });
  });
});

describe("pendingIndex", () => {
  const diff = (servers: ConfigDiff["servers"]): ConfigDiff => ({ servers, streams: [], upstreams: [], hasChanges: servers.length > 0 });

  it("keys are server ids: added/changed are pending, skipped carries its reason, removed is neither", () => {
    const idx = pendingIndex(
      diff([
        { key: "a", label: "a.example.test", status: "added", changes: [] },
        { key: "b", label: "b.example.test", status: "changed", changes: [] },
        { key: "c", label: "c.example.test", status: "skipped", reason: "인증서 없음", changes: [] },
        { key: "d", label: "d.example.test", status: "removed", changes: [] },
      ]),
    );
    expect([...idx.changed]).toEqual(["a", "b"]);
    expect([...idx.skipped]).toEqual([["c", "인증서 없음"]]);
  });

  it("no diff means nothing pending", () => {
    expect(pendingIndex(null).changed.size).toBe(0);
  });
});

describe("healthOf", () => {
  const ok = { label: "HTTPS · 90일", tone: "success" as const, problem: null, unknown: false };
  it("disabled wins over everything", () => {
    expect(healthOf(false, { label: "만료됨", tone: "error", problem: "인증서 만료됨", unknown: false }, "x")).toEqual({ tone: "neutral", label: "비활성", reason: null });
  });
  it("an apply-skipped server needs attention, with the reason", () => {
    expect(healthOf(true, ok, "인증서 없음")).toEqual({ tone: "error", label: "확인 필요", reason: "적용 제외: 인증서 없음" });
  });
  it("TLS problems map to 확인 필요 / 주의; no problem is 활성 (saved config, not a runtime probe)", () => {
    expect(healthOf(true, { label: "인증서 삭제됨", tone: "error", problem: "인증서 삭제됨", unknown: false }, null)).toMatchObject({ label: "확인 필요", reason: "인증서 삭제됨" });
    expect(healthOf(true, { label: "만료 6일 전", tone: "warning", problem: "인증서 만료 6일 전", unknown: false }, null)).toMatchObject({ label: "주의" });
    expect(healthOf(true, ok, null)).toEqual({ tone: "success", label: "활성", reason: null });
  });

  it("unknown TLS never reads as 활성", () => {
    const unknown = tlsOf("c1", null);
    expect(healthOf(true, unknown, null)).toEqual({ tone: "neutral", label: "확인 불가", reason: "TLS 확인 불가" });
  });

  it("keeps the exact issuance semantics: failed is 확인 필요, issuing and TXT 대기 are 주의", () => {
    const idx = certificateIndex([cert({ id: "f", status: "failed", expiresAt: null }), cert({ id: "i", status: "issuing", expiresAt: null }), cert({ id: "d", status: "awaiting-dns", expiresAt: null })]);
    expect(healthOf(true, tlsOf("f", idx), null)).toMatchObject({ label: "확인 필요", reason: "인증서 발급 실패" });
    expect(healthOf(true, tlsOf("i", idx), null)).toMatchObject({ label: "주의", reason: "인증서 발급 중" });
    expect(healthOf(true, tlsOf("d", idx), null)).toMatchObject({ label: "주의", reason: "DNS TXT 대기" });
  });
});

describe("toOpsRow and byHealth", () => {
  const src = (id: string, over: Record<string, unknown> = {}) => ({
    id,
    serverName: `${id}.example.test`,
    listen: 80,
    enabled: true,
    upstreamId: null,
    certificateId: null,
    config: { proxyPass: "http://10.0.0.1:3000" },
    ...over,
  });

  it("joins the row with its certificate and pending state", () => {
    const row = toOpsRow(src("a", { certificateId: "gone" }), new Map(), certificateIndex([]), pendingIndex({ servers: [{ key: "a", label: "a", status: "changed", changes: [] }], streams: [], upstreams: [], hasChanges: true }));
    expect(row).toMatchObject({ id: "a", targetLabel: "proxy_pass: http://10.0.0.1:3000", pending: true, tls: { label: "인증서 삭제됨" }, health: { label: "확인 필요" } });
  });

  it("sorts 확인 필요 → 주의 → 확인 불가 → 활성 → 비활성, then by name", () => {
    const idx = certificateIndex([cert({ id: "soon", expiresAt: day(5) }), cert({ id: "noexp", expiresAt: null })]);
    const none = pendingIndex(null);
    const rows = [src("b"), src("off", { enabled: false }), src("a", { certificateId: "soon" }), src("c", { certificateId: "gone" }), src("u", { certificateId: "noexp" })]
      .map((s) => toOpsRow(s, new Map(), idx, none))
      .sort(byHealth)
      .map((r) => r.id);
    expect(rows).toEqual(["c", "a", "u", "b", "off"]);
  });
});
