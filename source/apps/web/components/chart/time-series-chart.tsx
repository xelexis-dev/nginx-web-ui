"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { seriesDash } from "./series-style";
import styles from "./chart.module.css";

// 손수 만든 인라인 SVG 차트. 라이브러리를 넣지 않는 이유는 @repo/ui 가 런타임
// 의존성 0개이고, 하나 들이는 순간 이미지에 고정해야 할 구성요소가 하나 느는
// 것이기 때문이다.
//
// 단일·오류 계열은 의미 토큰, 다중 계열은 --chart-1..6을 쓴다. 슬롯은 엔티티에 고정한다 —
// 필터로 계열을 하나 빼도 남은 계열의 색이 바뀌면 안 된다.

export const CHART_SLOTS = 6;

/** 계열 색. 슬롯을 넘으면 호출부가 "기타"로 접어야 한다(색을 만들어 쓰지 않는다). */
export function chartColor(slot: number): string {
  return `var(--chart-${Math.min(slot, CHART_SLOTS - 1) + 1})`;
}

/** 범례·툴팁·표의 계열 표식: 선과 같은 색과 같은 무늬의 짧은 선. 색을 못 가려도 무늬로 짝을 찾는다. */
export function SeriesKey({
  slot,
  color = chartColor(slot),
}: {
  slot: number;
  color?: string;
}) {
  return (
    <svg
      className={styles.key}
      width={20}
      height={10}
      viewBox="0 0 20 10"
      aria-hidden
    >
      <line
        x1={2}
        y1={5}
        x2={18}
        y2={5}
        stroke={color}
        strokeDasharray={seriesDash(slot)}
      />
    </svg>
  );
}

export interface ChartSeries {
  key: string;
  label: string;
  /** times 와 같은 길이. null 은 **구멍**이고 선을 끊는다 — 보간하면 거짓말이 된다. */
  points: (number | null)[];
  /** 0-based 슬롯. 생략하면 배열 순서를 쓴다. */
  slot?: number;
  tone?: "error";
}

export interface TimeSeriesChartProps {
  /** 버킷 시작 시각(epoch ms), 오름차순. */
  times: number[];
  series: ChartSeries[];
  height?: number;
  /** y 축·툴팁 숫자 포맷. */
  format?: (value: number) => string;
  /** 계열이 하나일 때 면적을 채운다. */
  area?: boolean;
  /** y 축 최솟값을 0 으로 고정할지. 카운트 지표는 true 가 맞다. */
  zeroBased?: boolean;
  emptyLabel?: string;
  ariaLabel?: string;
}

const PADDING = { top: 8, right: 16, bottom: 24, left: 48 };
const FALLBACK_TZ = "Asia/Seoul";

function niceTicks(min: number, max: number, count: number): number[] {
  if (!(max > min)) return [min];
  const raw = (max - min) / count;
  const mag = Math.pow(10, Math.floor(Math.log10(raw)));
  const norm = raw / mag;
  const step = (norm >= 5 ? 10 : norm >= 2 ? 5 : norm >= 1 ? 2 : 1) * mag;
  const out: number[] = [];
  for (let v = Math.ceil(min / step) * step; v <= max + step * 0.001; v += step)
    out.push(v);
  return out;
}

function defaultFormat(v: number): string {
  if (Math.abs(v) >= 1_000_000) return `${(v / 1_000_000).toFixed(1)}M`;
  if (Math.abs(v) >= 1_000) return `${(v / 1_000).toFixed(1)}k`;
  return Number.isInteger(v) ? String(v) : v.toFixed(1);
}

export function TimeSeriesChart({
  times,
  series,
  height = 192,
  format = defaultFormat,
  area = false,
  zeroBased = true,
  emptyLabel = "표시할 데이터가 아직 없습니다.",
  ariaLabel,
}: TimeSeriesChartProps) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [hover, setHover] = useState<number | null>(null);
  // 시간대는 mount 후에 정한다(서버는 브라우저 시간대를 모른다 — history-time.tsx 와 동일 관용).
  const [timeZone, setTimeZone] = useState(FALLBACK_TZ);

  useEffect(() => {
    setTimeZone(
      Intl.DateTimeFormat().resolvedOptions().timeZone || FALLBACK_TZ,
    );
  }, []);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const observer = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width ?? 0;
      setWidth(Math.round(w));
    });
    observer.observe(el);
    setWidth(Math.round(el.getBoundingClientRect().width));
    return () => observer.disconnect();
  }, []);

  const spanMs = times.length > 1 ? times[times.length - 1]! - times[0]! : 0;
  const tickFormatter = useMemo(() => {
    // 범위가 넓으면 날짜를, 좁으면 시:분을 보여준다.
    const opts: Intl.DateTimeFormatOptions =
      spanMs > 3 * 24 * 3600_000
        ? { month: "numeric", day: "numeric", timeZone }
        : { hour: "2-digit", minute: "2-digit", hour12: false, timeZone };
    return new Intl.DateTimeFormat("ko-KR", opts);
  }, [spanMs, timeZone]);

  const fullFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat("ko-KR", {
        month: "numeric",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
        hour12: false,
        timeZone,
      }),
    [timeZone],
  );

  const bounds = useMemo(() => {
    let max = Number.NEGATIVE_INFINITY;
    let min = Number.POSITIVE_INFINITY;
    for (const s of series) {
      for (const p of s.points) {
        if (p === null || !Number.isFinite(p)) continue;
        if (p > max) max = p;
        if (p < min) min = p;
      }
    }
    if (!Number.isFinite(max)) return null;
    if (zeroBased) min = Math.min(0, min);
    if (max === min) max = min + 1;
    return { min, max };
  }, [series, zeroBased]);

  // ⚠ 아래 파생값은 **렌더와 hover 가 함께 쓴다.** 예전에 오른쪽 여백을 둘이 따로 계산하다
  // 렌더만 넓어져 hover 기준선이 오른쪽으로 갈수록 밀린 적이 있다. 계산을 하나로 두어
  // 여백 규칙이 바뀌어도 두 곳이 갈라질 수 없게 한다. 오른쪽 여백은 계열 수와 무관하게 16px이다.
  const padRight = PADDING.right;
  const plotWidth = Math.max(0, width - PADDING.left - padRight);

  const onPointerMove = useCallback(
    (event: React.PointerEvent<SVGSVGElement>) => {
      if (times.length === 0 || width === 0) return;
      const rect = event.currentTarget.getBoundingClientRect();
      const x = event.clientX - rect.left;
      if (plotWidth <= 0) return;
      const ratio = (x - PADDING.left) / plotWidth;
      const index = Math.round(ratio * (times.length - 1));
      setHover(Math.max(0, Math.min(times.length - 1, index)));
    },
    [times.length, plotWidth],
  );

  const hasData = bounds !== null && times.length > 0;
  // 계열이 2개 이상이면 범례는 항상 둔다. 1개면 제목이 곧 이름이라 두지 않는다.
  const showLegend = series.length >= 2;
  const colorFor = (s: ChartSeries, index: number) =>
    s.tone === "error"
      ? "var(--danger)"
      : series.length === 1
        ? "var(--chart-line)"
        : chartColor(s.slot ?? index);
  // padRight / plotWidth 는 위에서 계산한다(hover 와 공유).
  // 색만으로 정체를 전달하지 않는다(DESIGN.md Chart palette): 범례와 툴팁이 이름을 글자로 보이고,
  // 슬롯 4–6의 선 무늬가 같은 무늬의 표식(SeriesKey)으로 선과 이름을 잇는다.
  const plotHeight = Math.max(0, height - PADDING.top - PADDING.bottom);

  const xAt = (i: number) =>
    PADDING.left +
    (times.length <= 1 ? plotWidth / 2 : (i / (times.length - 1)) * plotWidth);
  const yAt = (v: number) =>
    PADDING.top +
    plotHeight -
    ((v - bounds!.min) / (bounds!.max - bounds!.min)) * plotHeight;

  const yTicks = hasData ? niceTicks(bounds.min, bounds.max, 4) : [];
  const xTickIndexes = useMemo(() => {
    if (times.length === 0) return [];
    const wanted = Math.max(2, Math.min(6, Math.floor(plotWidth / 90) || 2));
    if (times.length <= wanted) return times.map((_, i) => i);
    const step = (times.length - 1) / (wanted - 1);
    return Array.from({ length: wanted }, (_, i) => Math.round(i * step));
  }, [times, plotWidth]);

  /** null 을 경계로 끊어 여러 path 로 만든다. 한 점짜리 조각은 점으로 그린다. */
  function segmentsOf(points: (number | null)[]): {
    d: string;
    dots: [number, number][];
  } {
    let d = "";
    const dots: [number, number][] = [];
    let runLength = 0;
    let runStart = 0;
    for (let i = 0; i <= points.length; i++) {
      const value = i < points.length ? points[i]! : null;
      if (value === null || !Number.isFinite(value)) {
        if (runLength === 1)
          dots.push([xAt(runStart), yAt(points[runStart] as number)]);
        runLength = 0;
        continue;
      }
      if (runLength === 0) {
        runStart = i;
        d += `M${xAt(i).toFixed(1)},${yAt(value).toFixed(1)}`;
      } else {
        d += `L${xAt(i).toFixed(1)},${yAt(value).toFixed(1)}`;
      }
      runLength++;
    }
    return { d, dots };
  }

  const singleAreaD = useMemo(() => {
    if (!area || !hasData || series.length !== 1) return null;
    const points = series[0]!.points;
    let d = "";
    let open = false;
    let firstX = 0;
    let lastX = 0;
    for (let i = 0; i < points.length; i++) {
      const v = points[i];
      if (v === null || !Number.isFinite(v)) {
        if (open) {
          d += `L${lastX.toFixed(1)},${(PADDING.top + plotHeight).toFixed(1)}L${firstX.toFixed(1)},${(PADDING.top + plotHeight).toFixed(1)}Z`;
          open = false;
        }
        continue;
      }
      const x = xAt(i);
      const y = yAt(v as number);
      if (!open) {
        firstX = x;
        d += `M${x.toFixed(1)},${y.toFixed(1)}`;
        open = true;
      } else {
        d += `L${x.toFixed(1)},${y.toFixed(1)}`;
      }
      lastX = x;
    }
    if (open) {
      d += `L${lastX.toFixed(1)},${(PADDING.top + plotHeight).toFixed(1)}L${firstX.toFixed(1)},${(PADDING.top + plotHeight).toFixed(1)}Z`;
    }
    return d;
  }, [area, hasData, series, plotHeight, plotWidth, bounds, times]);

  return (
    <div className={styles.wrap} ref={wrapRef}>
      {showLegend && (
        <ul className={styles.legend}>
          {series.map((s, i) => (
            <li key={s.key} className={styles.legendItem}>
              <SeriesKey slot={s.slot ?? i} color={colorFor(s, i)} />
              {s.label}
            </li>
          ))}
        </ul>
      )}

      {!hasData ? (
        <div className={styles.empty} style={{ height }}>
          {emptyLabel}
        </div>
      ) : (
        <>
          <svg
            className={styles.plot}
            width={width || undefined}
            height={height}
            viewBox={width ? `0 0 ${width} ${height}` : undefined}
            role="img"
            aria-label={ariaLabel ?? series.map((s) => s.label).join(", ")}
            onPointerMove={onPointerMove}
            onPointerLeave={() => setHover(null)}
          >
            {yTicks.map((t) => (
              <g key={`y${t}`}>
                <line
                  className={styles.grid}
                  x1={PADDING.left}
                  x2={width - padRight}
                  y1={yAt(t)}
                  y2={yAt(t)}
                />
                <text
                  className={styles.tick}
                  x={PADDING.left - 8}
                  y={yAt(t)}
                  textAnchor="end"
                  dominantBaseline="middle"
                >
                  {format(t)}
                </text>
              </g>
            ))}
            <line
              className={styles.axis}
              x1={PADDING.left}
              x2={width - padRight}
              y1={PADDING.top + plotHeight}
              y2={PADDING.top + plotHeight}
            />
            {xTickIndexes.map((i) => (
              <text
                key={`x${i}`}
                className={styles.tick}
                x={xAt(i)}
                y={height - 8}
                textAnchor={
                  i === 0 ? "start" : i === times.length - 1 ? "end" : "middle"
                }
              >
                {tickFormatter.format(new Date(times[i]!))}
              </text>
            ))}

            {singleAreaD && (
              <path
                className={styles.area}
                d={singleAreaD}
                fill={
                  series[0]!.tone === "error"
                    ? "var(--danger-bg)"
                    : "var(--chart-fill)"
                }
              />
            )}

            {series.map((s, i) => {
              const { d, dots } = segmentsOf(s.points);
              const color = colorFor(s, i);
              return (
                <g key={s.key}>
                  {d && (
                    <path
                      className={styles.line}
                      d={d}
                      stroke={color}
                      strokeDasharray={seriesDash(s.slot ?? i)}
                    />
                  )}
                  {dots.map(([cx, cy], k) => (
                    <circle
                      key={k}
                      className={styles.dot}
                      cx={cx}
                      cy={cy}
                      r={3}
                      fill={color}
                    />
                  ))}
                </g>
              );
            })}

            {hover !== null && (
              <>
                <line
                  className={styles.crosshair}
                  x1={xAt(hover)}
                  x2={xAt(hover)}
                  y1={PADDING.top}
                  y2={PADDING.top + plotHeight}
                />
                {series.map((s, i) => {
                  const v = s.points[hover];
                  if (v === null || v === undefined || !Number.isFinite(v))
                    return null;
                  return (
                    <circle
                      key={s.key}
                      className={styles.dot}
                      cx={xAt(hover)}
                      cy={yAt(v)}
                      r={4}
                      fill={colorFor(s, i)}
                    />
                  );
                })}
              </>
            )}
          </svg>

          {hover !== null && (
            <div
              className={styles.tooltip}
              style={{
                left: Math.max(
                  PADDING.left,
                  Math.min(width - padRight, xAt(hover)),
                ),
                top: PADDING.top,
              }}
              role="status"
            >
              <div className={styles.tooltipTime}>
                {fullFormatter.format(new Date(times[hover]!))}
              </div>
              {series.map((s, i) => {
                const v = s.points[hover];
                return (
                  <div key={s.key} className={styles.tooltipRow}>
                    <span className={styles.tooltipName}>
                      <SeriesKey slot={s.slot ?? i} color={colorFor(s, i)} />
                      {s.label}
                    </span>
                    <span className={styles.tooltipValue}>
                      {v === null || v === undefined || !Number.isFinite(v)
                        ? "—"
                        : format(v)}
                    </span>
                  </div>
                );
              })}
            </div>
          )}
        </>
      )}
    </div>
  );
}
