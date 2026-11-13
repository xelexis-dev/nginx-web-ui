/** Chart presentation types and range labels. Queries run in the API. */
export type RangeKey = "1h" | "6h" | "24h" | "7d" | "30d" | "90d" | "1y";

export const RANGES: { key: RangeKey; label: string; ms: number }[] = [
  { key: "1h", label: "최근 1시간", ms: 3600_000 },
  { key: "6h", label: "최근 6시간", ms: 6 * 3600_000 },
  { key: "24h", label: "최근 24시간", ms: 24 * 3600_000 },
  { key: "7d", label: "최근 7일", ms: 7 * 24 * 3600_000 },
  { key: "30d", label: "최근 30일", ms: 30 * 24 * 3600_000 },
  { key: "90d", label: "최근 90일", ms: 90 * 24 * 3600_000 },
  { key: "1y", label: "최근 1년", ms: 365 * 24 * 3600_000 },
];

export function isRangeKey(v: string | null): v is RangeKey {
  return v !== null && RANGES.some((r) => r.key === v);
}

export interface GlobalPoint {
  t: number;
  /** 활성 연결 평균. 표본이 없는 버킷은 null — 선을 끊는다(0 으로 채우면 거짓말). */
  active: number | null;
  /** 초당 요청. 카운터 리셋·공백 구간은 null. */
  requestsPerSec: number | null;
  /** 초당 신규 연결(accepts). */
  connectionsPerSec: number | null;
  /** 초당 드롭된 연결 = accepts − handled. 지금까지 파싱해놓고 버리던 값이다. */
  droppedPerSec: number | null;
}
export interface DomainSeries {
  host: string;
  points: (number | null)[];
  total: number;
}
export interface DomainSeriesResult {
  times: number[];
  series: DomainSeries[];
  /** 상위 계열에 들지 못한 나머지를 합친 값(있을 때만). */
  otherTotal: number;
  /** 표 뷰용 전체 순위. 라이트 모드 대비 보정(relief) 규칙을 이걸로 충족한다. */
  ranking: { host: string; requests: number; bytesSent: number; s4xx: number; s5xx: number }[];
  source: "minutes" | "hours";
}
export interface CollectorHealth {
  lastSampleAt: number | null;
  lastBucketAt: number | null;
  unparsedCount: number;
  /**
   * 검산: 같은 구간에서 stub_status 가 센 요청 증가분 대비 로그 파싱이 센 요청 수.
   *
   * 1.00 이어야 한다. 0 에 가까우면 파서가 로그를 못 읽고 있고(회전 후 nginx 가
   * 옛 inode 에 계속 쓰는 상태가 대표적), 2.00 에 가까우면 관리 UI 이중 계상이
   * 섞여 있다. 추가 저장·읽기 비용 없이 얻는 신호다.
   */
  parseRatio: number | null;
}
