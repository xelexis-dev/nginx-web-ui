// 수집 상태 문구. 늦은 것을 색 점만으로 알리지 않는다 — 점은 aria-hidden 이고 색각에 따라
// 초록·주황이 같아 보인다. 그래서 상태가 문구 자체에 들어간다.

/** 마지막 표본이 이보다 오래되면 늦은 것이다. 수집 주기(1분)의 다섯 배. */
export const STALE_AFTER_MS = 5 * 60_000;

export type CollectionHealth =
  | { state: "none"; text: string }
  | { state: "ok" | "stale"; text: string; lastSampleAt: number };

export function collectionHealth(
  lastSampleAt: number | null,
  now: number,
  formatTime: (epochMs: number) => string,
): CollectionHealth {
  if (!lastSampleAt) return { state: "none", text: "아직 수집된 표본이 없습니다" };
  const stale = now - lastSampleAt >= STALE_AFTER_MS;
  return {
    state: stale ? "stale" : "ok",
    text: `${stale ? "수집 지연" : "수집 중"} · 마지막 표본 ${formatTime(lastSampleAt)}`,
    lastSampleAt,
  };
}
