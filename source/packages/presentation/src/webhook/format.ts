import { buildGenericPayload, type NotificationEvent, type RenderContext } from "./payload";

/**
 * 사건 하나를 수신처가 알아듣는 요청 본문으로 바꾼다.
 *
 * - `generic` — 앱이 정한 고정 스키마 그대로. 스키마가 안정적이어서 받는 쪽 코드를
 *   한 번 짜면 계속 쓸 수 있다.
 * - `discord`  — 같은 데이터를 embed 로. Discord 는 임의 JSON 을 받지 않는다.
 * - `dooray`   — 같은 데이터를 Dooray 인커밍 훅의 attachment 로.
 * - `custom`   — 운영자가 쓴 본문 템플릿에 `{{변수}}` 치환. 위 셋으로 안 맞는
 *   수신처(Slack, 사내 시스템)를 위한 탈출구.
 *
 * 전부 순수 함수다 — 네트워크는 send.ts 가 맡는다. 이 경계 덕분에 형식 로직은
 * 서버 없이 테스트된다.
 */
export type WebhookFormat = "generic" | "discord" | "dooray" | "custom";

export interface RenderedRequest {
  body: string;
  contentType: string;
}

export type RenderResult = { ok: true; request: RenderedRequest } | { ok: false; error: string };

const JSON_CONTENT_TYPE = "application/json";

// ---------------------------------------------------------------- generic

export function renderGeneric(event: NotificationEvent, ctx: RenderContext = {}): RenderedRequest {
  return { body: JSON.stringify(buildGenericPayload(event, ctx)), contentType: JSON_CONTENT_TYPE };
}

// ---------------------------------------------------------------- discord

/** severity → embed 색. 앱의 상태 색과 같은 계열로 맞춘다. */
const DISCORD_COLOR: Record<string, number> = {
  info: 0x3b82f6,
  success: 0x22c55e,
  warning: 0xf59e0b,
  error: 0xef4444,
};

// Discord 의 embed 상한. 넘기면 400 으로 통째로 거절당하므로 보내기 전에 자른다.
const TITLE_MAX = 256;
const DESCRIPTION_MAX = 4096;
const FIELD_VALUE_MAX = 1024;

function clip(text: string, max: number): string {
  return text.length <= max ? text : `${text.slice(0, max - 1)}…`;
}

interface DiscordField {
  name: string;
  value: string;
  inline?: boolean;
}

function discordFields(event: NotificationEvent): DiscordField[] {
  const data = event.data ?? {};
  const fields: DiscordField[] = [];

  const domains = Array.isArray(data.domains) ? data.domains : [];
  if (domains.length > 0) {
    fields.push({ name: "도메인", value: clip(domains.join("\n"), FIELD_VALUE_MAX) });
  }
  if (typeof data.daysLeft === "number") {
    fields.push({ name: "남은 기간", value: `${data.daysLeft}일`, inline: true });
  }
  if (typeof data.expiresAt === "string" && data.expiresAt) {
    fields.push({ name: "만료", value: data.expiresAt, inline: true });
  }
  if (data.staging === true) {
    fields.push({ name: "구분", value: "STAGING (테스트 발급)", inline: true });
  }
  return fields;
}

export function renderDiscord(event: NotificationEvent, ctx: RenderContext = {}): RenderedRequest {
  const payload = buildGenericPayload(event, ctx);
  // Discord 는 상대 URL 을 거부한다 — 앱 주소가 설정되지 않았으면 링크를 붙이지 않는다.
  const url = payload.link && /^https?:\/\//i.test(payload.link) ? payload.link : undefined;

  const embed: Record<string, unknown> = {
    title: clip(event.title, TITLE_MAX),
    color: DISCORD_COLOR[event.severity] ?? DISCORD_COLOR.info,
    timestamp: payload.occurredAt,
    footer: { text: `nginx-web-ui · ${event.event}` },
  };
  if (payload.message) embed.description = clip(payload.message, DESCRIPTION_MAX);
  if (url) embed.url = url;

  const fields = discordFields(event);
  if (fields.length > 0) embed.fields = fields;

  return {
    body: JSON.stringify({ username: "nginx-web-ui", embeds: [embed] }),
    contentType: JSON_CONTENT_TYPE,
  };
}

// ---------------------------------------------------------------- dooray

/**
 * severity → Dooray attachment 색.
 *
 * Dooray 는 Discord 처럼 정수 색이 아니라 **문자열 색 이름**을 받는다(실제로 쓰이는
 * 값: `red`, `darkgreen`, `green`, `yellow`, `purple` — 전부 CSS 색 이름이다).
 * hex(`#ff0000`) 도 되는지는 공식 문서에서 확인하지 못해, 근거가 있는 이름만 쓴다.
 */
const DOORAY_COLOR: Record<string, string> = {
  info: "blue",
  success: "green",
  warning: "orange",
  error: "red",
};

/**
 * Dooray 는 길이 상한을 문서로 밝히지 않는다. 그래서 이 값들은 규격이 아니라
 * **우리가 스스로 거는 안전선**이다 — 장문 ACME 오류가 그대로 실려 나가 수신처를
 * 곤란하게 하지 않도록.
 */
const DOORAY_TITLE_MAX = 500;
const DOORAY_TEXT_MAX = 4000;

/** attachment 본문에 넣을 세부 줄. 값이 있는 것만 남긴다. */
function doorayDetailLines(event: NotificationEvent): string[] {
  const data = event.data ?? {};
  const domains = Array.isArray(data.domains) ? data.domains : [];
  return [
    domains.length > 0 ? `도메인: ${domains.join(", ")}` : null,
    typeof data.daysLeft === "number" ? `남은 기간: ${data.daysLeft}일` : null,
    typeof data.expiresAt === "string" && data.expiresAt ? `만료: ${data.expiresAt}` : null,
    data.staging === true ? `구분: STAGING (테스트 발급)` : null,
  ].filter((line): line is string => line !== null);
}

export function renderDooray(event: NotificationEvent, ctx: RenderContext = {}): RenderedRequest {
  const payload = buildGenericPayload(event, ctx);
  // Dooray 도 상대 URL 로는 이동할 수 없다 — 앱 주소가 없으면 링크를 붙이지 않는다.
  const url = payload.link && /^https?:\/\//i.test(payload.link) ? payload.link : undefined;

  const attachment: Record<string, unknown> = {
    // 요약 줄이 곧 링크가 되도록 title 에 메시지를 둔다. 메시지가 없는 사건은 제목으로 대신한다.
    title: clip(payload.message || event.title, DOORAY_TITLE_MAX),
    color: DOORAY_COLOR[event.severity] ?? DOORAY_COLOR.info,
  };
  if (url) attachment.titleLink = url;

  const detail = doorayDetailLines(event);
  if (detail.length > 0) attachment.text = clip(detail.join("\n"), DOORAY_TEXT_MAX);

  return {
    body: JSON.stringify({
      botName: "nginx-web-ui",
      text: clip(event.title, DOORAY_TITLE_MAX),
      attachments: [attachment],
    }),
    contentType: JSON_CONTENT_TYPE,
  };
}

// ---------------------------------------------------------------- custom

/**
 * 템플릿에서 쓸 수 있는 변수.
 *
 * `json` 만 원문 그대로 들어가고(= generic 페이로드 통째), 나머지는 전부
 * **JSON 문자열 안쪽으로 이스케이프**해서 넣는다. 도메인이나 오류 메시지에 큰따옴표가
 * 섞여도 본문이 깨지지 않게 하려는 것 — 이스케이프가 없으면 평소엔 되다가
 * 하필 사고가 난 날 알림만 실패한다.
 *
 * `daysLeft`/`staging` 은 따옴표 없이 써도 유효한 JSON 이 되도록 `null`/숫자/불리언
 * 리터럴로 렌더한다(`"days": {{daysLeft}}` 도 `"days": "{{daysLeft}}"` 도 동작).
 */
export const TEMPLATE_VARIABLES = [
  "event",
  "severity",
  "title",
  "message",
  "link",
  "occurredAt",
  "domains",
  "daysLeft",
  "expiresAt",
  "certificateId",
  "staging",
  "json",
] as const;

export type TemplateVariable = (typeof TEMPLATE_VARIABLES)[number];

const PLACEHOLDER = /\{\{\s*([a-zA-Z][a-zA-Z0-9_]*)\s*\}\}/g;

/** JSON 문자열 리터럴 **안쪽**에 넣을 수 있는 형태로. 바깥 따옴표는 템플릿이 쓴다. */
function jsonInner(value: string): string {
  const quoted = JSON.stringify(value);
  return quoted.slice(1, -1);
}

function templateValues(event: NotificationEvent, ctx: RenderContext): Record<string, string> {
  const payload = buildGenericPayload(event, ctx);
  const data = event.data ?? {};
  const domains = Array.isArray(data.domains) ? data.domains : [];

  return {
    event: jsonInner(payload.event),
    severity: jsonInner(payload.severity),
    title: jsonInner(payload.title),
    message: jsonInner(payload.message ?? ""),
    link: jsonInner(payload.link ?? ""),
    occurredAt: jsonInner(payload.occurredAt),
    domains: jsonInner(domains.join(", ")),
    daysLeft: typeof data.daysLeft === "number" ? String(data.daysLeft) : "null",
    expiresAt: jsonInner(typeof data.expiresAt === "string" ? data.expiresAt : ""),
    certificateId: jsonInner(typeof data.certificateId === "string" ? data.certificateId : ""),
    staging: typeof data.staging === "boolean" ? String(data.staging) : "null",
    json: JSON.stringify(payload),
  };
}

export interface TemplateRender {
  body: string;
  /** 템플릿이 쓴, 우리가 모르는 변수 이름. 저장 시 오류로 되돌린다. */
  unknownVariables: string[];
}

export function renderTemplateBody(
  template: string,
  event: NotificationEvent,
  ctx: RenderContext = {},
): TemplateRender {
  const values = templateValues(event, ctx);
  const unknown = new Set<string>();
  const body = template.replace(PLACEHOLDER, (_match, name: string) => {
    const replacement = values[name];
    if (replacement !== undefined) return replacement;
    unknown.add(name);
    return "";
  });
  return { body, unknownVariables: [...unknown] };
}

export function renderCustom(
  template: string | null | undefined,
  event: NotificationEvent,
  ctx: RenderContext = {},
): RenderResult {
  if (!template || !template.trim()) return { ok: false, error: "본문 템플릿이 비어 있습니다." };
  const { body, unknownVariables } = renderTemplateBody(template, event, ctx);
  if (unknownVariables.length > 0) {
    return { ok: false, error: `알 수 없는 변수: ${unknownVariables.map((v) => `{{${v}}}`).join(", ")}` };
  }
  return { ok: true, request: { body, contentType: JSON_CONTENT_TYPE } };
}

export const DEFAULT_TEMPLATE = `{
  "text": "[{{severity}}] {{title}} — {{message}}",
  "event": "{{event}}",
  "domains": "{{domains}}",
  "daysLeft": {{daysLeft}},
  "link": "{{link}}"
}`;

// ---------------------------------------------------------------- dispatch

export function renderRequest(
  format: WebhookFormat,
  template: string | null | undefined,
  event: NotificationEvent,
  ctx: RenderContext = {},
): RenderResult {
  switch (format) {
    case "generic":
      return { ok: true, request: renderGeneric(event, ctx) };
    case "discord":
      return { ok: true, request: renderDiscord(event, ctx) };
    case "dooray":
      return { ok: true, request: renderDooray(event, ctx) };
    case "custom":
      return renderCustom(template, event, ctx);
  }
}
