import { daysLeft } from "@/components/certificates/certificate-vm";
import type { ConfigDiff } from "@/lib/config-diff";
import { toServerRow, type ServerRowVM, type ServerSourceRow } from "./server-vm";

/**
 * 서버 한 줄의 **상태 판정**(Operations 목록·개요의 「상태」「TLS」 칸). 전부 실제 데이터에서만
 * 나온다 — 서버 행, 같은 출처의 인증서 목록, 미적용 diff. **저장된 설정과 인증서를 본 판정이지
 * 서버가 실제로 응답하는지 본 것이 아니다**(백엔드 헬스 체크는 없다) — 그래서 문제없는 서버의
 * 말은 「정상」이 아니라 「활성」이고, 모르는 것은 「확인 불가」로 둔다. 트래픽(요청 수·5xx)은 여기서 쓰지
 * 않는다: 호스트 → 서버 대응이 확실하지 않아 섞지 않기로 했다(접속 지표 화면으로 링크).
 *
 * 로컬 목록과 노드 목록이 **같은 함수**를 쓴다. 인증서 목록은 호출하는 쪽이 같은 출처(로컬이면
 * 게이트웨이, 노드면 그 노드)에서 읽어 넘긴다.
 */

export type Tone = "success" | "warning" | "error" | "info" | "neutral";

/**
 * 「만료 임박」의 기준 일수. **API 의 규칙을 그대로 쓴다** — `/api/v1/status` 의
 * `certificates.expiringSoon`(apps/api/src/control/operations.rs, 30일)과 만료 알림
 * (notifications/queue.rs, 30일). 여기서 다른 숫자를 쓰면 개요의 개수와 목록의 표시가 어긋난다.
 */
export const EXPIRING_DAYS = 30;
const DAY_MS = 24 * 60 * 60 * 1000;

/**
 * 「만료 임박」— API 와 **같은 경계**다: `expires_at <= now + 30일`(정확한 시각 비교, 이미 만료된 것
 * 포함). 남은 날수를 내림한 값으로 비교하면 30.9일 남은 인증서까지 들어와 개수가 어긋난다.
 */
export function isExpiringSoon(expiresAt: Date | null, now: number = Date.now()): boolean {
  return expiresAt !== null && expiresAt.getTime() <= now + EXPIRING_DAYS * DAY_MS;
}

export interface CertificateSummary {
  id: string;
  status: string;
  expiresAt: Date | null;
}

/**
 * TLS 칸이 읽는 인증서 목록. **`null` 은 「읽지 못했다」**다 — 인증서 읽기는 페이지의 필수
 * 읽기가 아니라서 실패해도 목록은 그린다. 그때 TLS 칸은 추측하지 않고 「확인 불가」로 둔다.
 */
export type CertificateIndex = ReadonlyMap<string, CertificateSummary> | null;

export function certificateIndex(items: readonly CertificateSummary[] | null): CertificateIndex {
  return items ? new Map(items.map((c) => [c.id, c])) : null;
}

export interface TlsVM {
  /** 칸에 쓰는 짧은 말(「HTTPS · 61일」「만료 6일 전」「인증서 삭제됨」…). */
  label: string;
  tone: Tone;
  /** 확인이 필요한 이유(상태 칸의 부연). 문제가 없거나 모르면 `null`. */
  problem: string | null;
  /** 판정할 근거가 없다(인증서 목록을 못 읽음, 만료일 없음, 모르는 상태). 「문제없음」이 아니다. */
  unknown: boolean;
}

const known = (label: string, tone: Tone, problem: string | null = null): TlsVM => ({ label, tone, problem, unknown: false });
const unknownTls = (label: string): TlsVM => ({ label, tone: "neutral", problem: null, unknown: true });

export function tlsOf(certificateId: string | null, certificates: CertificateIndex): TlsVM {
  if (!certificateId) return known("HTTP", "neutral");
  if (!certificates) return unknownTls("확인 불가");
  const cert = certificates.get(certificateId);
  // 서버가 가리키는 인증서가 목록에 없다 = 삭제됐다. 적용하면 이 서버는 TLS 없이 렌더된다.
  if (!cert) return known("인증서 삭제됨", "error", "인증서 삭제됨");
  switch (cert.status) {
    case "failed":
      return known("발급 실패", "error", "인증서 발급 실패");
    case "issuing":
      return known("발급 중", "warning", "인증서 발급 중");
    case "awaiting-dns":
      return known("TXT 대기", "warning", "DNS TXT 대기");
    case "issued": {
      // 발급됐는데 만료일이 없다 — 괜찮다고 말할 근거가 없다.
      if (!cert.expiresAt) return unknownTls("만료일 모름");
      if (cert.expiresAt.getTime() <= Date.now()) return known("만료됨", "error", "인증서 만료됨");
      const left = daysLeft(cert.expiresAt) ?? 0;
      if (isExpiringSoon(cert.expiresAt)) return known(`만료 ${left}일 전`, "warning", `인증서 만료 ${left}일 전`);
      return known(`HTTPS · ${left}일`, "success");
    }
    default:
      // API 가 새 상태를 더했다 — 옛 규칙으로 「문제없음」이라 하지 않는다.
      return unknownTls("상태 모름");
  }
}

/** 미적용 diff 에서 서버별로 읽은 것. `key` 는 서버 id 다(apps/api/src/control/pending.rs). */
export interface PendingIndex {
  /** 추가·변경되어 적용을 기다리는 서버. */
  changed: ReadonlySet<string>;
  /** 적용이 이 서버를 뺐다(`skipped`) — 이유와 함께. 살아 있지도, 대기 중도 아니다. */
  skipped: ReadonlyMap<string, string>;
}

export function pendingIndex(diff: ConfigDiff | null): PendingIndex {
  const changed = new Set<string>();
  const skipped = new Map<string, string>();
  for (const item of diff?.servers ?? []) {
    if (item.status === "added" || item.status === "changed") changed.add(item.key);
    else if (item.status === "skipped") skipped.set(item.key, item.reason ?? "적용에서 제외됨");
  }
  return { changed, skipped };
}

export interface HealthVM {
  tone: Tone;
  /** 저장된 설정·인증서 기준의 말. 「활성」은 「문제를 찾지 못함」이지 「응답함」이 아니다. */
  label: "활성" | "주의" | "확인 필요" | "확인 불가" | "비활성";
  /** 왜 그런지 한 줄. 활성·비활성이면 `null`. */
  reason: string | null;
}

/**
 * 한 서버의 상태. 순서가 규칙이다 — 비활성이면 그것이 전부고, 적용에서 빠진 서버와 TLS 오류는
 * 「확인 필요」, 만료 임박·발급 대기는 「주의」, TLS 를 판정할 근거가 없으면 「확인 불가」다.
 */
export function healthOf(enabled: boolean, tls: TlsVM, skippedReason: string | null): HealthVM {
  if (!enabled) return { tone: "neutral", label: "비활성", reason: null };
  if (skippedReason) return { tone: "error", label: "확인 필요", reason: `적용 제외: ${skippedReason}` };
  if (tls.tone === "error") return { tone: "error", label: "확인 필요", reason: tls.problem };
  if (tls.tone === "warning") return { tone: "warning", label: "주의", reason: tls.problem };
  if (tls.unknown) return { tone: "neutral", label: "확인 불가", reason: `TLS ${tls.label}` };
  return { tone: "success", label: "활성", reason: null };
}

/** 개요·목록의 정렬: 확인 필요 → 주의 → 확인 불가 → 활성 → 비활성, 같은 급이면 이름순. */
const HEALTH_ORDER: Record<HealthVM["label"], number> = { "확인 필요": 0, 주의: 1, "확인 불가": 2, 활성: 3, 비활성: 4 };
export function byHealth<T extends { health: HealthVM; serverName: string }>(a: T, b: T): number {
  return HEALTH_ORDER[a.health.label] - HEALTH_ORDER[b.health.label] || a.serverName.localeCompare(b.serverName);
}

/** Operations 목록·개요의 한 줄: 기존 행(이름·포트·대상·활성)에 TLS·상태·미적용을 더한 것. */
export interface ServerOpsRowVM extends ServerRowVM {
  certificateId: string | null;
  tls: TlsVM;
  health: HealthVM;
  /** 저장됐지만 아직 nginx 에 적용되지 않은 변경이 있다. */
  pending: boolean;
}

export function toOpsRow(
  row: ServerSourceRow,
  upstreamNameById: Map<string, string>,
  certificates: CertificateIndex,
  pending: PendingIndex,
): ServerOpsRowVM {
  const certificateId = row.certificateId ?? null;
  const tls = tlsOf(certificateId, certificates);
  return {
    ...toServerRow(row, upstreamNameById),
    certificateId,
    tls,
    health: healthOf(row.enabled, tls, pending.skipped.get(row.id) ?? null),
    pending: pending.changed.has(row.id),
  };
}
