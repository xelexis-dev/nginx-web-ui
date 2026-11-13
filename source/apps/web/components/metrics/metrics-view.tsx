"use client";

import { startVisibleInterval } from "@/lib/visible-interval";
import { useCallback, useEffect, useState } from "react";
import {
  Surface,
  StatusDot,
  Banner,
  Select,
  Table,
  Td,
  Th,
  Tr,
} from "@repo/ui";
import {
  SeriesKey,
  TimeSeriesChart,
} from "@/components/chart/time-series-chart";
import { RANGES, type RangeKey } from "@/lib/metrics-view";
import { collectionHealth } from "./health";
import styles from "./metrics.module.css";
import layout from "@/components/page-layout.module.css";

// 데이터는 클라이언트에서 가져온다. 서버 컴포넌트로 그리면 범위를 바꿀 때마다
// 전체 페이지가 다시 그려지고, 이 앱에는 Suspense/loading.tsx 관용이 아예 없다
// (루트에 Suspense 를 넣었다가 프리렌더가 깨진 전례가 있다).

const POLL_MS = 30_000;

interface GlobalPoint {
  t: number;
  active: number | null;
  requestsPerSec: number | null;
  connectionsPerSec: number | null;
  droppedPerSec: number | null;
}

interface DomainPayload {
  times: number[];
  series: { host: string; points: (number | null)[]; total: number }[];
  otherTotal: number;
  ranking: {
    host: string;
    requests: number;
    bytesSent: number;
    s4xx: number;
    s5xx: number;
  }[];
  source: "minutes" | "hours";
}

interface HealthPayload {
  lastSampleAt: number | null;
  lastBucketAt: number | null;
  unparsedCount: number;
  parseRatio: number | null;
}

interface Payload {
  range: RangeKey;
  global: GlobalPoint[];
  domains: DomainPayload;
  health: HealthPayload;
}

/** 실제 도메인이 아닌 표식을 사람 말로 바꾼다. 숨기지는 않는다 — 진짜 트래픽이다. */
const PSEUDO_HOSTS: Record<string, string> = {
  __no_host_field__: "(옛 형식 · 도메인 기록 없음)",
  __empty_host__: "(Host 헤더 없음)",
  __default_vhost__: "(기본 vhost)",
  __invalid_host__: "(호스트명이 아닌 값)",
  __other__: "(기타 요청)",
};

function hostLabel(host: string): string {
  return PSEUDO_HOSTS[host] ?? host;
}

function formatBytes(n: number): string {
  if (n >= 1024 ** 3) return `${(n / 1024 ** 3).toFixed(1)} GB`;
  if (n >= 1024 ** 2) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${n} B`;
}

function formatRate(n: number): string {
  if (n >= 100) return n.toFixed(0);
  if (n >= 10) return n.toFixed(1);
  return n.toFixed(2);
}

export function MetricsView({
  initialRange,
  dataPath = "/api/metrics",
  now = Date.now,
}: {
  initialRange: RangeKey;
  /** 지표를 받아올 경로. 로컬 화면은 기본값(`/api/metrics`)을 그대로 쓰고,
   * 노드 화면은 게이트웨이 중계 라우트(`/api/nodes/[id]/metrics`)를 넘긴다
   * (이 컴포넌트는 클라이언트에서 스스로 fetch 하므로
   * `loadNodePage` 가 데이터를 넘겨줄 수 없다). */
  dataPath?: string;
  /** 수집 상태를 판정할 현재 시각. 미리보기가 고정 표본으로 같은 화면을 그리려고 쓴다. */
  now?: () => number;
}) {
  const [range, setRange] = useState<RangeKey>(initialRange);
  const [data, setData] = useState<Payload | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(
    async (key: RangeKey, signal?: AbortSignal) => {
      try {
        const res = await fetch(`${dataPath}?range=${key}`, {
          signal,
          cache: "no-store",
        });
        if (!res.ok) throw new Error(`서버가 ${res.status} 를 돌려줬습니다.`);
        const payload: Payload = await res.json();
        setData(payload);
        setError(null);
      } catch (err) {
        if ((err as Error).name === "AbortError") return;
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        setLoading(false);
      }
    },
    [dataPath],
  );

  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    void load(range, controller.signal);
    // 데이터가 분당 한 번만 바뀌므로 30초면 충분하다(알림 벨과 같은 주기).
    const stop = startVisibleInterval(
      () => void load(range, controller.signal),
      POLL_MS,
    );
    return () => {
      controller.abort();
      stop();
    };
  }, [range, load]);

  // URL 에 범위를 남긴다 — 새로고침·공유에도 같은 화면이 나오게(audit 화면 관용).
  useEffect(() => {
    const url = new URL(window.location.href);
    url.searchParams.set("range", range);
    window.history.replaceState(null, "", url.toString());
  }, [range]);

  const globalTimes = data?.global.map((p) => p.t) ?? [];
  const ratioOff =
    data?.health.parseRatio !== null && data?.health.parseRatio !== undefined
      ? data.health.parseRatio < 0.5 || data.health.parseRatio > 1.5
      : false;

  return (
    <div className={layout.section}>
      <div className={styles.toolbar}>
        <div className={styles.rangePicker}>
          <Select
            options={RANGES.map((r) => ({ value: r.key, label: r.label }))}
            value={range}
            onChange={(v) => setRange(v as RangeKey)}
            aria-label="표시 범위"
          />
        </div>
        {data &&
          (() => {
            const health = collectionHealth(
              data.health.lastSampleAt,
              now(),
              (t) => new Date(t).toLocaleTimeString("ko-KR"),
            );
            return (
              <div className={styles.health} data-state={health.state}>
                <StatusDot
                  tone={health.state === "ok" ? "success" : "warning"}
                />
                {health.text}
              </div>
            );
          })()}
      </div>

      {error && (
        <Banner
          tone="warning"
          title={<>지표를 불러오지 못했습니다: {error}</>}
        />
      )}
      {loading && !data && <p className={styles.loading}>불러오는 중…</p>}

      {data && (
        <>
          {ratioOff && (
            <Banner tone="warning" title="집계 상태 확인">
              <span>
                도메인별 집계가 전체 요청 수와 어긋납니다(비율{" "}
                {data.health.parseRatio!.toFixed(2)}, 1.00 이 정상).{" "}
                {data.health.parseRatio! < 0.5
                  ? "로그 로테이션 후 nginx 가 이전 파일에 계속 쓰고 있을 수 있습니다 — logrotate 설정의 postrotate 에 `nginx -s reopen` 이 있는지 확인하세요."
                  : "관리 UI 트래픽이 nginx 를 두 번 지나 이중으로 집계되고 있을 수 있습니다 — 관리자 설정에서 해당 도메인을 제외 목록에 넣으세요."}
              </span>
            </Banner>
          )}

          <div className={styles.panels}>
            <Surface
              title={<>활성 연결</>}
              info={
                <>
                  nginx 인스턴스 전체 합계입니다(도메인별 분해는 stub_status 에
                  없습니다).
                </>
              }
              as="div"
              className={styles.chartSection}
            >
              <TimeSeriesChart
                times={globalTimes}
                area
                series={[
                  {
                    key: "active",
                    label: "활성 연결",
                    points: data.global.map((p) => p.active),
                    slot: 0,
                  },
                ]}
                ariaLabel="활성 연결 추이"
                emptyLabel="아직 표본이 없습니다. 수집은 1분마다 이뤄집니다."
              />
            </Surface>

            <Surface
              title={<>초당 요청 / 연결</>}
              info={
                <>
                  누적 카운터의 증분입니다. nginx 재시작 구간은 선이 끊깁니다.
                </>
              }
              as="div"
              className={styles.chartSection}
            >
              <TimeSeriesChart
                times={globalTimes}
                format={formatRate}
                series={[
                  {
                    key: "req",
                    label: "요청/초",
                    points: data.global.map((p) => p.requestsPerSec),
                    slot: 0,
                  },
                  {
                    key: "conn",
                    label: "연결/초",
                    points: data.global.map((p) => p.connectionsPerSec),
                    slot: 2,
                  },
                ]}
                ariaLabel="초당 요청 및 연결 추이"
              />
            </Surface>

            <Surface
              title={<>드롭된 연결</>}
              info={
                <>
                  accepts − handled. 0 이 아니면 워커 연결 한도에 닿고 있다는
                  뜻입니다.
                </>
              }
              as="div"
              className={`${styles.chartSection} ${styles.panelWide}`}
            >
              <TimeSeriesChart
                times={globalTimes}
                format={formatRate}
                series={[
                  {
                    key: "drop",
                    tone: "error",
                    label: "드롭/초",
                    points: data.global.map((p) => p.droppedPerSec),
                    slot: 1,
                  },
                ]}
                area
                ariaLabel="드롭된 연결 추이"
              />
            </Surface>

            <Surface
              title={<>도메인별 요청 수</>}
              info={
                <>
                  access.log 를 서버에서 집계한 값입니다. 상위{" "}
                  {data.domains.series.length}개 도메인만 그리고, 나머지는 아래
                  표에 전부 있습니다
                  {data.domains.source === "hours"
                    ? " (30일이 넘는 범위는 시간 단위 롤업을 봅니다)"
                    : ""}
                  .
                </>
              }
              as="div"
              className={`${styles.chartSection} ${styles.panelWide}`}
            >
              <TimeSeriesChart
                times={data.domains.times}
                series={data.domains.series.map((s, i) => ({
                  key: s.host,
                  label: hostLabel(s.host),
                  points: s.points,
                  slot: i,
                }))}
                ariaLabel="도메인별 요청 수 추이"
                emptyLabel="아직 집계된 요청이 없습니다."
              />

              {data.domains.ranking.length > 0 && (
                <Table label="도메인별 요청 수">
                  <thead>
                    <tr>
                      <Th>도메인</Th>
                      <Th align="end">요청</Th>
                      <Th align="end">전송량</Th>
                      <Th align="end">4xx</Th>
                      <Th align="end">5xx</Th>
                    </tr>
                  </thead>
                  <tbody>
                    {data.domains.ranking.map((row, i) => {
                      const pseudo = row.host in PSEUDO_HOSTS;
                      return (
                        <Tr key={row.host}>
                          <Td cell="primary">
                            <span className={styles.hostCell}>
                              {/* The chart's slot i: the same colour and line pattern as its line. */}
                              {i < data.domains.series.length && (
                                <SeriesKey slot={i} />
                              )}
                              <span
                                className={`${styles.hostLabel}${pseudo ? ` ${styles.pseudoHost}` : ""}`}
                                title={
                                  row.host === "__other__"
                                    ? "등록한 서버 이름에 맞지 않는 요청(관리 화면 접속 포함)과 도메인 수 상한을 넘은 요청을 합산합니다."
                                    : hostLabel(row.host)
                                }
                              >
                                {hostLabel(row.host)}
                              </span>
                            </span>
                          </Td>
                          <Td numeric label="요청">
                            {row.requests.toLocaleString("ko-KR")}
                          </Td>
                          <Td numeric label="전송량">
                            {formatBytes(row.bytesSent)}
                          </Td>
                          <Td numeric label="4xx">
                            {row.s4xx.toLocaleString("ko-KR")}
                          </Td>
                          <Td numeric label="5xx">
                            {row.s5xx.toLocaleString("ko-KR")}
                          </Td>
                        </Tr>
                      );
                    })}
                  </tbody>
                </Table>
              )}
            </Surface>
          </div>
        </>
      )}
    </div>
  );
}
