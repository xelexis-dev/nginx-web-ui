import { ApiRequestError, readView } from "./api-client";
import type { Optional } from "./optional";

export type { Optional } from "./optional";
export { mapOptional } from "./optional";

/**
 * 페이지의 **보조 읽기**(서버 전용). 한 화면이 여러 뷰를 모아 그릴 때(개요, 인증서 상태를 곁들인
 * 서버 목록) 필수 읽기는 `readPageView` 로 하고, 곁들이는 읽기는 이것으로 한다 — 실패가 그 칸에만
 * 머문다. 결과 모양은 `optional.ts`(순수)에 있다. 403 은 「권한 없음」, 그 밖의 실패(연결 실패,
 * 5xx, 404, 세션 만료 401 포함)는 그 칸의 오류다 — 세션 만료는 필수 읽기가 이미 로그인으로 보낸다.
 *
 * 이 함수는 던지지 않는다. 그래서 `Promise.all` 로 나란히 읽어도 하나의 실패가 페이지를 넘어뜨리지
 * 않는다.
 */
export async function readOptionalView<T>(path: string, query: Record<string, string | string[] | undefined> = {}): Promise<Optional<T>> {
  try {
    return { ok: true, data: await readView<T>(path, query) };
  } catch (error) {
    return optionalFailure(error);
  }
}

export function optionalFailure(error: unknown): Optional<never> {
  if (error instanceof ApiRequestError) {
    if (error.status === 403) return { ok: false, reason: "forbidden" };
    return { ok: false, reason: "error", message: error.message };
  }
  return { ok: false, reason: "error", message: "읽지 못했습니다." };
}
