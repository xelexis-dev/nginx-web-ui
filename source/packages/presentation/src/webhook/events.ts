/**
 * 웹훅이 구독할 수 있는 사건 목록.
 *
 * 값은 `notifications.type`(인앱 피드)과 **같은 문자열**이다 — 웹훅 발송이
 * `createNotification` 한 곳에 붙어 있어서, 여기 없는 type 은 아무 대상도 고르지
 * 못하고 조용히 지나간다. 새 알림을 추가할 때 이 목록에 함께 넣어야 한다.
 *
 * 사건 계열별로 그룹을 나누어 폼에 표시한다.
 */
export interface WebhookEventDef {
  type: string;
  label: string;
  /**
   * 라벨만으로 알 수 없는 것만 적는다. "발급 완료 = 발급에 성공했을 때" 같은
   * 되풀이는 체크박스 여섯 개를 읽을거리로 만들 뿐이다.
   */
  hint?: string;
}

export const CERT_EVENTS: readonly WebhookEventDef[] = [
  {
    type: "cert.renewal_due",
    label: "만기 도래 (수동 갱신 필요)",
    hint: "DNS-01 인증서, 만료 30일 전부터 7일 간격",
  },
  { type: "cert.issued", label: "발급 완료" },
  { type: "cert.renewed", label: "갱신 완료" },
  { type: "cert.failed", label: "발급 실패" },
  { type: "cert.renew_failed", label: "갱신 실패", hint: "기존 인증서는 만료 전까지 유효" },
  { type: "cert.dns_awaiting", label: "DNS TXT 추가 필요" },
];

export const SYSTEM_EVENTS: readonly WebhookEventDef[] = [
  { type: "system.db_upgrade_backup_kept", label: "PG15 백업을 보관하고 있습니다" },
  { type: "system.db_upgrade_activation_failed", label: "업그레이드 후 서비스 시작에 실패했습니다" },
];

/** 폼이 렌더하는 그룹. 계열이 늘면 여기에 추가한다. */
export const WEBHOOK_EVENT_GROUPS: readonly { label: string; events: readonly WebhookEventDef[] }[] = [
  { label: "인증서", events: CERT_EVENTS },
  { label: "시스템", events: SYSTEM_EVENTS },
];

export const WEBHOOK_EVENT_TYPES: readonly string[] = WEBHOOK_EVENT_GROUPS.flatMap((g) =>
  g.events.map((e) => e.type),
);

/** 새 대상의 기본 구독 — 전부. 필요 없는 것만 꺼서 쓰는 편이 반대보다 낫다. */
export const DEFAULT_EVENT_TYPES: readonly string[] = WEBHOOK_EVENT_TYPES;

export function eventLabel(type: string): string {
  for (const group of WEBHOOK_EVENT_GROUPS) {
    const found = group.events.find((e) => e.type === type);
    if (found) return `${group.label} · ${found.label}`;
  }
  return type;
}
