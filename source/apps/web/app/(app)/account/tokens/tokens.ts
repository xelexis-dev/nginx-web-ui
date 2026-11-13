import { formatServerTime } from "@/lib/format-time";
import type { TokenScope } from "@repo/contracts";

/**
 * 토큰 화면의 **순수 부분** — 라벨·시각 표기·만료 입력 해석.
 *
 * 화면(`token-list.tsx`·`token-detail.tsx`·`token-form.tsx`)과 서버 액션
 * (`actions.ts`)이 **같은 것을 여기서** 가져간다. 갈라 두면 폼이 보여주는
 * 만료 선택지와 액션이 실제로 계산하는 날짜가 어긋나는데, 그 어긋남은 화면에
 * 보이지 않는다(사용자는 「90일」을 골랐고 DB 에는 다른 값이 들어간다).
 *
 * **서버 전용 모듈을 import 하지 않는다.** `token-form.tsx` 가 "use client" 라
 * 여기 있는 것은 클라이언트 번들에도 실린다 — `@/lib/guard` 나 `getDb` 를 끌어오면
 * 그 순간 빌드가 깨지거나(운이 좋으면) 서버 코드가 브라우저로 간다.
 */

/** scope 옆에 붙는 사람 말. 어휘 자체(`config:read`)는 그대로 보여준다 — 사용자가 API 에 쓸 값이다. */
export const SCOPE_LABELS: Record<TokenScope, string> = {
  "config:read": "설정 읽기",
  "config:write": "설정 변경",
  "nginx:reload": "nginx 다시 읽기",
  "user:manage": "사용자 관리",
  "node:pair": "노드 연결",
};

/** 목록·상세가 함께 쓰는 상태. 판정은 API 가 한다. */
export type TokenStatus = "active" | "expired" | "revoked";

export const STATUS_LABELS: Record<TokenStatus, string> = {
  active: "유효",
  expired: "만료됨",
  revoked: "해지됨",
};

/**
 * 화면에 쓰는 시각 표기.
 *
 * ⚠ **`timeZone` 을 주지 않으므로 도는 곳의 TZ 를 쓴다. 그래서 서버에서만 불러야
 * 한다** — 이 저장소의 관용이 그렇고(`admin/users`·`admin/pending` 등 전부 서버
 * 컴포넌트가 날짜를 만든다), 컨테이너는 TZ 를 정하지 않으므로 UTC 다.
 *
 * **클라이언트 컴포넌트에서 부르면 실제로 틀린다.** 그 컴포넌트는 SSR 된 뒤 브라우저에서
 * **다시 실행**되고, 그때는 브라우저 TZ 로 계산된다. 실측(2026-08-30, 서버 `TZ=UTC` +
 * 브라우저 `Asia/Seoul`): 같은 토큰의 만료가 목록에서 `2026. 11. 28. AM 1:38`,
 * 상세에서 `2026. 11. 28. 오전 10:38` 로 **9시간 어긋났고**, 상세 첫 로드에서 React
 * 오류 #418(hydration text mismatch)이 났다.
 *
 * 그래서 `lib/token-api.ts` 의 `tokenPresentation` 이 **서버에서** 문자열로 만들어 내려보내고, 화면
 * 컴포넌트(`TokenList`·`TokenDetail`)는 `Date` 를 아예 받지 않는다 — 규칙이 아니라
 * **타입이** 이 함수를 클라이언트에서 못 부르게 막는다. 목록과 상세가 같은 값을
 * 보이는 것도 그래서 자동이다(둘 다 같은 함수가 만든다).
 */
export function formatDateTime(value: Date | null, fallback: string): string {
  if (value === null) return fallback;
  return formatServerTime(value, { dateStyle: "medium", timeStyle: "short" });
}

/**
 * 화면이 받는 한 줄. **`Date` 가 하나도 없다** — 시각은 이미 문자열이고 상태도 이미
 * 판정돼 있다. 위 `formatDateTime` 주석이 그 이유다.
 *
 * `revoked` 를 따로 두는 것도 의도다. 「해지 버튼을 그릴까」를 `revokedAt` **문자열**로
 * 되묻게 하면 그 판정이 문구("해지되지 않음")에 걸리고, 문구를 고치는 날 버튼이
 * 사라지거나 되살아난다.
 */
export interface TokenRowView {
  id: string;
  name: string;
  masked: string;
  scopes: string[];
  status: TokenStatus;
  createdAt: string;
  lastUsedAt: string;
  expiresAt: string;
  revokedAt: string;
  revoked: boolean;
}

/** 만료 선택지. `never` 는 **막지 않는다** — 경고만 띄운다(스펙 §5). */
export const EXPIRY_PRESETS = [
  { value: "30d", label: "30일" },
  { value: "90d", label: "90일" },
  { value: "1y", label: "1년" },
  { value: "custom", label: "직접 입력" },
  { value: "never", label: "무기한" },
] as const;

export type ExpiryPreset = (typeof EXPIRY_PRESETS)[number]["value"];
