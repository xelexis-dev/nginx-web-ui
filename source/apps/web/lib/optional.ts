/**
 * 보조 읽기의 결과 모양(순수 모듈 — 서버 전용 읽기와 분리). 개요 합성처럼 Storybook·테스트에서도
 * 부르는 코드가 이것만 가져가도록, 실제 읽기(`optional-read.ts`, `next/headers` 사슬)와 나눴다.
 *
 * - 성공: `{ ok: true, data }`.
 * - 권한 없음(403): `{ ok: false, reason: "forbidden" }` — 그 칸을 숨긴다. 오류가 아니다.
 * - 그 밖의 실패: `{ ok: false, reason: "error", message }` — 그 칸에 이유를 보인다.
 */
export type Optional<T> =
  | { ok: true; data: T }
  | { ok: false; reason: "forbidden" }
  | { ok: false; reason: "error"; message: string };

/** 성공한 값에만 변환을 건다. 실패는 그대로 나른다. 변환이 던지면(응답 모양이 다르면) 그 칸의 오류다. */
export function mapOptional<T, U>(value: Optional<T>, fn: (data: T) => U): Optional<U> {
  if (!value.ok) return value;
  try {
    return { ok: true, data: fn(value.data) };
  } catch (error) {
    return { ok: false, reason: "error", message: error instanceof Error ? error.message : "응답을 읽지 못했습니다." };
  }
}
