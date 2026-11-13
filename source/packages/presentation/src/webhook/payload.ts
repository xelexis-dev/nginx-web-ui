/**
 * 웹훅으로 나가는 사건 1건과, 모든 형식이 공유하는 "일반(generic)" 페이로드.
 *
 * generic 페이로드가 **정본**이다: Discord embed 도 custom 템플릿 변수도 전부
 * 여기서 파생된다. 형식마다 재료를 따로 모으면 한쪽만 갱신되는 일이 생긴다.
 */

export type NotificationSeverity = "info" | "success" | "warning" | "error";

/** 인증서 사건이 싣는 부가 데이터. 알 수 없는 값은 넣지 않는다(없음 = 표시 안 함). */
export interface EventData {
  certificateId?: string | null;
  domains?: string[] | null;
  /** ISO 8601. */
  expiresAt?: string | null;
  daysLeft?: number | null;
  staging?: boolean | null;
  [key: string]: unknown;
}

export interface NotificationEvent {
  /** `notifications.type` 과 같은 문자열 (events.ts 참고). */
  event: string;
  severity: NotificationSeverity;
  title: string;
  message?: string | null;
  /** 앱 내부 경로("/certificates/…"). 절대 URL 은 baseUrl 과 합쳐 만든다. */
  path?: string | null;
  occurredAt: Date;
  data?: EventData | null;
}

export interface RenderContext {
  /**
   * 공개 앱 주소. 스윕은 요청 컨텍스트가 없어 스스로 알 수 없으므로 설정에서 온다.
   * 비어 있으면 링크는 상대경로로 나가고, Discord embed 는 링크를 아예 뺀다
   * (Discord 는 상대 URL 을 거부한다).
   */
  baseUrl?: string | null;
}

/** 절대 URL 로 만들 수 있으면 만들고, 아니면 경로 그대로. path 가 없으면 null. */
export function resolveLink(path: string | null | undefined, baseUrl: string | null | undefined): string | null {
  if (!path) return null;
  if (!baseUrl) return path;
  try {
    return new URL(path, baseUrl.endsWith("/") ? baseUrl : `${baseUrl}/`).toString();
  } catch {
    // 설정이 URL 이 아니면(운영자 오타) 링크를 잃는 것보다 상대경로가 낫다.
    return path;
  }
}

/** 값이 있는 키만 남긴다 — 수신 측에서 `null` 과 "모름"을 구분할 이유가 없다. */
function compactData(data: EventData | null | undefined): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  if (!data) return out;
  for (const [key, value] of Object.entries(data)) {
    if (value === null || value === undefined) continue;
    if (Array.isArray(value) && value.length === 0) continue;
    out[key] = value;
  }
  return out;
}

export interface GenericPayload {
  source: "nginx-web-ui";
  event: string;
  severity: NotificationSeverity;
  title: string;
  message: string | null;
  link: string | null;
  occurredAt: string;
  data: Record<string, unknown>;
}

export function buildGenericPayload(event: NotificationEvent, ctx: RenderContext = {}): GenericPayload {
  return {
    source: "nginx-web-ui",
    event: event.event,
    severity: event.severity,
    title: event.title,
    message: event.message ?? null,
    link: resolveLink(event.path, ctx.baseUrl),
    occurredAt: event.occurredAt.toISOString(),
    data: compactData(event.data),
  };
}

/**
 * 테스트 전송·템플릿 검증에 쓰는 표본. 실제 만기 알림과 같은 모양이라,
 * "테스트는 되는데 실제는 안 되는" 차이가 생기지 않는다.
 */
export function sampleEvent(now: Date = new Date()): NotificationEvent {
  const expires = new Date(now.getTime() + 12 * 24 * 60 * 60 * 1000);
  return {
    event: "cert.renewal_due",
    severity: "warning",
    title: "인증서 갱신 필요 (DNS-01)",
    message: "example.com, www.example.com — 약 12일 남음",
    path: "/certificates/00000000-0000-0000-0000-000000000000",
    occurredAt: now,
    data: {
      certificateId: "00000000-0000-0000-0000-000000000000",
      domains: ["example.com", "www.example.com"],
      expiresAt: expires.toISOString(),
      daysLeft: 12,
      staging: false,
      test: true,
    },
  };
}
