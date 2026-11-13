// 계열 선 모양. 색 말고도 계열을 가릴 수 있게 하는 장치다 — 색각 이상 시뮬레이션에서 슬롯 2–6,
// 3–5, 4–5, 2–5 쌍은 색이 거의 같아진다. 선과 범례·툴팁·표의 표식(SeriesKey)이 같은 무늬를 쓴다.

/**
 * 슬롯별 선 모양(stroke-dasharray). 1–3은 실선, 4–6은 서로 다른 무늬다. 색이 비슷해지는
 * 쌍(2–6, 3–5, 4–5, 2–5)은 늘 실선 대 무늬이거나 서로 다른 무늬가 된다. 선은 둥근 끝(2px)이라
 * 칸마다 2px 길어지고 틈은 2px 줄어든다: 4 = 8px 선·3px 틈, 5 = 2.5px 점·3px 틈, 6 = 긴 선과 점.
 */
const DASHES: (string | undefined)[] = [undefined, undefined, undefined, "6 5", "0.5 5", "9 5 0.5 5"];

export function seriesDash(slot: number): string | undefined {
  return DASHES[Math.max(0, Math.min(slot, DASHES.length - 1))];
}
