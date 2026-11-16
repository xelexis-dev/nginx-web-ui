"use client";

import {
  Fragment,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Icon,
  EmptyState,
  Facts,
  Table,
  Td,
  Th,
  Tr,
  StatusBadge,
  type Tone,
  Button,
  FilterChips,
  SearchField,
  Select,
  SegmentedControl,
  Checkbox,
} from "@repo/ui";
import styles from "./logs.module.css";

type LogFile = "access" | "error";
type ViewMode = "table" | "raw";
type StatusClass = "" | "2xx" | "3xx" | "4xx" | "5xx";

const MAX_LINES = 2000;
const FLUSH_MS = 200;
const SCROLL_BOTTOM_THRESHOLD_PX = 24;
const RECONNECT_DELAY_MS = 2000;
const RECONNECT_DELAY_MAX_MS = 30_000;
// 노드 화면에서는 재연결 한 번이 세션 재검증 + nodes 조회 + 토큰 복호화 +
// 원격 TLS 핸드셰이크다 — 로컬처럼 "그냥 파일 다시 읽기"가 아니다. 그래서
// 2s 고정 간격으로 영원히 두드리면 안 된다: 실패마다 간격을 두 배로 늘리고
// (2s→4s→8s→16s→30s, 이후 30s 고정) MAX_RECONNECT_ATTEMPTS번 연속 실패하면
// 멈춘다. EventSource의 onerror는 HTTP 상태를 주지 않아 "토큰이 회수됐다"와
// "잠깐 끊겼다"를 구분할 수 없다 — 원인을 추정하는 대신 횟수로만 막는다.
const MAX_RECONNECT_ATTEMPTS = 6;

// --- parsing -------------------------------------------------------------

interface AccessRec {
  kind: "access";
  ip: string;
  /** epoch ms, 표시 시간대로 변환해 쓴다. 파싱 실패 시 null → raw 문자열 폴백. */
  atMs: number | null;
  rawTime: string;
  host: string;
  method: string;
  path: string;
  proto: string;
  status: number;
  statusRaw: string;
  bytes: string;
  referer: string;
  ua: string;
}
interface ErrorRec {
  kind: "error";
  atMs: number | null;
  rawTime: string;
  level: string;
  message: string;
}
interface RawRec {
  kind: "raw";
}
type Rec = AccessRec | ErrorRec | RawRec;

// nginx "combined" + 우리가 덧붙인 "$host"(docker/nginx.conf). host는 선택 —
// 이 변경 전에 기록된 줄에는 없으므로 없어도 파싱은 성공해야 한다.
const ACCESS_RE =
  /^(\S+) \S+ (\S+) \[([^\]]+)\] "([^"]*)" (\d{3}) (\S+) "([^"]*)" "([^"]*)"(?: "([^"]*)")?/;
// $time_local: 24/Jul/2026:10:31:18 +0000 — 오프셋이 들어 있어 시각을 정확히 복원할 수 있다.
const ACCESS_TIME_RE =
  /^(\d{2})\/(\w{3})\/(\d{4}):(\d{2}):(\d{2}):(\d{2}) ([+-])(\d{2})(\d{2})$/;
// nginx error: YYYY/MM/DD HH:MM:SS [level] pid#tid: *conn message
const ERROR_RE =
  /^(\d{4})\/(\d{2})\/(\d{2}) (\d{2}):(\d{2}):(\d{2}) \[(\w+)\] (.*)$/;

const MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

/** access 로그의 $time_local(오프셋 포함)을 epoch ms로. */
function parseAccessTime(value: string): number | null {
  const m = ACCESS_TIME_RE.exec(value.trim());
  if (!m) return null;
  const month = MONTHS.indexOf(m[2] as string);
  if (month < 0) return null;
  const offsetMinutes =
    (m[7] === "-" ? -1 : 1) * (Number(m[8]) * 60 + Number(m[9]));
  const asUtc = Date.UTC(
    Number(m[3]),
    month,
    Number(m[1]),
    Number(m[4]),
    Number(m[5]),
    Number(m[6]),
  );
  return asUtc - offsetMinutes * 60_000;
}

function parseLine(file: LogFile, text: string): Rec {
  if (file === "access") {
    const m = ACCESS_RE.exec(text);
    if (m) {
      const request = m[4] ?? "";
      const parts = request.split(" ");
      let method = "";
      let path = request;
      let proto = "";
      if (parts.length === 3) {
        [method, path, proto] = parts as [string, string, string];
      }
      const rawTime = m[3] ?? "";
      return {
        kind: "access",
        ip: m[1] ?? "",
        atMs: parseAccessTime(rawTime),
        rawTime,
        host: m[9] ?? "",
        method,
        path,
        proto,
        status: Number(m[5]),
        statusRaw: m[5] ?? "",
        bytes: m[6] ?? "",
        referer: m[7] ?? "",
        ua: m[8] ?? "",
      };
    }
  } else {
    const m = ERROR_RE.exec(text);
    if (m) {
      // error 로그는 오프셋을 적지 않는다. 이 이미지는 TZ를 설정하지 않아
      // 컨테이너가 UTC로 동작하므로 UTC로 읽는다(Dockerfile에 TZ 없음).
      const atMs = Date.UTC(
        Number(m[1]),
        Number(m[2]) - 1,
        Number(m[3]),
        Number(m[4]),
        Number(m[5]),
        Number(m[6]),
      );
      return {
        kind: "error",
        atMs: Number.isNaN(atMs) ? null : atMs,
        rawTime: `${m[4]}:${m[5]}:${m[6]}`,
        level: m[7] ?? "",
        message: m[8] ?? "",
      };
    }
  }
  return { kind: "raw" };
}

/**
 * 표시 시간대: 브라우저 OS 시간대, 못 찾으면 KST. History의 HistoryTime과 같은
 * 규칙이다(update/2026-07-23/history-local-timezone.md).
 */
const FALLBACK_TIME_ZONE = "Asia/Seoul";

function resolveTimeZone(): string {
  try {
    return (
      Intl.DateTimeFormat().resolvedOptions().timeZone || FALLBACK_TIME_ZONE
    );
  } catch {
    return FALLBACK_TIME_ZONE;
  }
}

/**
 * 로그 표는 밀도가 높아 `2026-07-24 19:31:18`로 고정 폭·정렬 가능한 형태로 쓴다
 * (History 상세는 ko-KR 서술형 — 용도가 달라 일부러 다르게 둔다).
 */
function makeStamper(timeZone: string): (atMs: number) => string {
  const fmt = new Intl.DateTimeFormat("en-CA", {
    timeZone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  });
  return (atMs: number) => {
    const parts = fmt.formatToParts(new Date(atMs));
    const get = (type: string) =>
      parts.find((p) => p.type === type)?.value ?? "";
    return `${get("year")}-${get("month")}-${get("day")} ${get("hour")}:${get("minute")}:${get("second")}`;
  };
}

interface LogLine {
  id: number;
  text: string;
  rec: Rec;
}

function statusClassOf(status: number): StatusClass {
  if (status >= 200 && status < 300) return "2xx";
  if (status >= 300 && status < 400) return "3xx";
  if (status >= 400 && status < 500) return "4xx";
  if (status >= 500) return "5xx";
  return "";
}

const STATUS_TONE: Record<Exclude<StatusClass, "">, Tone> = {
  "2xx": "success",
  "3xx": "neutral",
  "4xx": "warning",
  "5xx": "error",
};

function levelTone(level: string): Tone {
  const l = level.toLowerCase();
  if (l === "error" || l === "crit" || l === "alert" || l === "emerg")
    return "error";
  if (l === "warn") return "warning";
  if (l === "notice" || l === "info") return "info";
  return "neutral";
}

/** Raw-view only: color a whole error line by its level. */
function rawLevelClassName(file: LogFile, text: string): string | undefined {
  if (file !== "error") return undefined;
  if (
    text.includes("[error]") ||
    text.includes("[crit]") ||
    text.includes("[alert]") ||
    text.includes("[emerg]")
  ) {
    return styles.levelError;
  }
  if (text.includes("[warn]")) return styles.levelWarn;
  if (text.includes("[notice]")) return styles.levelNotice;
  return styles.levelMuted;
}

function isNoise(rec: Rec): boolean {
  return (
    rec.kind === "access" &&
    (rec.path.startsWith("/_next/static/") ||
      rec.path.startsWith("/service/rest/v1/user-telemetry/"))
  );
}

const METHOD_OPTIONS = [
  { value: "", label: "전체 메서드" },
  ...["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"].map((m) => ({
    value: m,
    label: m,
  })),
];
const STATUS_CHIPS: { value: StatusClass; label: string }[] = [
  { value: "", label: "전체" },
  { value: "2xx", label: "2xx" },
  { value: "3xx", label: "3xx" },
  { value: "4xx", label: "4xx" },
  { value: "5xx", label: "5xx" },
];
const LEVEL_CHIPS = [
  { value: "", label: "전체" },
  { value: "error", label: "error" },
  { value: "warn", label: "warn" },
  { value: "notice", label: "notice" },
  { value: "info", label: "info" },
];

/**
 * Live access/error log tail. Streams raw lines over SSE, parses each into a
 * structured record on arrival, and renders either a scannable Table (default)
 * or the Raw text.
 * Filters (search, status/level, method, noise-hide) narrow the Table view
 * only.
 *
 * 로컬 화면(`app/(app)/logs/page.tsx`)과 노드 화면(`app/(node)/nodes/[id]/logs/`)이
 * 이 컴포넌트를 공유한다 — `streamPath` 가 갈라지는 유일한 지점이다. 노드
 * 화면은 게이트웨이 중계 라우트(`/api/nodes/[id]/logs/stream`)를 넘긴다.
 * **쿼리 없는 기본 경로를 넘길 것** — `?file=...` 는 이 컴포넌트가 스스로
 * 붙인다(아래 `connect()`). prop 에 쿼리를 이미 담아 넘기면
 * `?file=access?file=error` 처럼 겹쳐 붙는다.
 */
export function LogViewer({ streamPath }: { streamPath: string }) {
  const [file, setFile] = useState<LogFile>("access");
  const [view, setView] = useState<ViewMode>("table");
  const [lines, setLines] = useState<LogLine[]>([]);
  const [autoScroll, setAutoScroll] = useState(true);
  // 시작값은 항상 true다 — 정상적인 첫 로드에서 onopen이 오기 전까지 배너가
  // 잠깐 번쩍이면 안 된다. onerror가 false로, onopen이 true로 되돌린다(아래
  // connect()) — 카운팅·타이머 없이 이 두 이벤트만으로 항상 정확하다.
  const [connected, setConnected] = useState(true);
  // MAX_RECONNECT_ATTEMPTS번 연속 실패해 자동 재연결을 멈춘 상태. true인 동안은
  // connect()가 스스로 다시 시도하지 않는다 — 사용자가 재연결 버튼으로
  // restartKey를 올려야 새 시도가 시작된다.
  const [retriesExhausted, setRetriesExhausted] = useState(false);
  // 재연결 버튼이 누르는 값. 값이 바뀌면 아래 SSE effect가 처음부터 다시
  // 붙는다 — file 전환과 달리 이미 받은 줄(lines)은 지우지 않는다.
  const [restartKey, setRestartKey] = useState(0);

  // filters
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState<StatusClass>("");
  const [method, setMethod] = useState("");
  const [level, setLevel] = useState("");
  const [hideNoise, setHideNoise] = useState(false);
  const [expanded, setExpanded] = useState<number | null>(null);
  const detailPrefix = useId();

  function toggleRow(id: number) {
    setExpanded((current) => (current === id ? null : id));
  }

  // SSR과 첫 클라이언트 렌더가 같은 값을 쓰도록 KST로 시작한 뒤 브라우저 시간대로 교체한다.
  const [timeZone, setTimeZone] = useState(FALLBACK_TIME_ZONE);
  useEffect(() => setTimeZone(resolveTimeZone()), []);
  const stamp = useMemo(() => makeStamper(timeZone), [timeZone]);
  const formatAt = useCallback(
    (rec: { atMs: number | null; rawTime: string }) =>
      rec.atMs === null ? rec.rawTime : stamp(rec.atMs),
    [stamp],
  );

  const viewportRef = useRef<HTMLDivElement | null>(null);
  const nextId = useRef(0);
  const pendingLines = useRef<LogLine[]>([]);
  const suppressScrollEvent = useRef(false);
  const fileRef = useRef(file);
  fileRef.current = file;

  // Each tab is an independent stream — switching tabs starts fresh.
  // connected/retriesExhausted도 같이 되돌린다 — 안 그러면 이전 탭에서 재연결을
  // 포기한 상태가 방금 새로 시작한 스트림 위에 그대로 남아 거짓 배너가 뜬다.
  useEffect(() => {
    pendingLines.current = [];
    setLines([]);
    setAutoScroll(true);
    setExpanded(null);
    setConnected(true);
    setRetriesExhausted(false);
  }, [file]);

  useEffect(() => {
    let es: EventSource | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
    let cancelled = false;
    // 연속 실패 횟수. 이 effect 실행(= 탭 전환 또는 재연결 버튼)마다 0에서
    // 시작하고, 연결에 성공하면 onopen이 다시 0으로 되돌린다 — 그래야 잠깐
    // 끊겼다 바로 회복하는 링크가 예전 장애의 긴 대기시간을 물려받지 않는다.
    let attempt = 0;

    function connect() {
      es = new EventSource(`${streamPath}?file=${file}`);
      es.onopen = () => {
        if (cancelled) return;
        attempt = 0;
        setConnected(true);
        setRetriesExhausted(false);
      };
      es.onmessage = (event) => {
        const id = nextId.current++;
        const rec = parseLine(fileRef.current, event.data);
        pendingLines.current.push({ id, text: event.data, rec });
        // A background tab's timer can stall while lines keep arriving.
        if (pendingLines.current.length > MAX_LINES)
          pendingLines.current.shift();
      };
      es.onerror = () => {
        es?.close();
        if (cancelled) return;
        // 이미 받은 줄은 지우지 않는다 — 끊긴 순간에도 지금까지의 로그는
        // 여전히 유효한 정보다. 원인은 모른다: 이 컴포넌트는 로컬 /logs와
        // 노드 화면이 공유하므로 "노드가 죽었다"처럼 노드 탓으로 단정하지
        // 않는다 — 로컬 화면에서는 그 말이 거짓이 된다.
        setConnected(false);
        attempt += 1;
        if (attempt >= MAX_RECONNECT_ATTEMPTS) {
          // 더 두드리지 않는다 — 죽은 연결이면 영원히 실패할 시도고, 살아있는
          // 연결이면 사용자가 재연결 버튼으로 되살릴 수 있다.
          setRetriesExhausted(true);
          return;
        }
        const delay = Math.min(
          RECONNECT_DELAY_MS * 2 ** (attempt - 1),
          RECONNECT_DELAY_MAX_MS,
        );
        reconnectTimer = setTimeout(connect, delay);
      };
    }
    connect();

    return () => {
      cancelled = true;
      es?.close();
      if (reconnectTimer) clearTimeout(reconnectTimer);
    };
    // streamPath는 페이지당 한 번 정해지고 리렌더에도 바뀌지 않는다(고정 문자열
    // 리터럴) — 그래도 exhaustive-deps를 그대로 따른다.
  }, [file, streamPath, restartKey]);

  // One render per interval, not one per line: each render walks up to MAX_LINES rows.
  useEffect(() => {
    const timer = setInterval(() => {
      if (pendingLines.current.length === 0) return;
      const batch = pendingLines.current;
      pendingLines.current = [];
      setLines((prev) => {
        const next = [...prev, ...batch];
        return next.length > MAX_LINES
          ? next.slice(next.length - MAX_LINES)
          : next;
      });
    }, FLUSH_MS);
    return () => {
      clearInterval(timer);
      pendingLines.current = [];
    };
  }, []);

  // "다시 연결" 배너 버튼. lines는 건드리지 않는다 — 탭 전환과 달리 이건
  // 새 스트림이 아니라 같은 스트림을 다시 잇는 것이다.
  const handleManualReconnect = useCallback(() => {
    setRetriesExhausted(false);
    setRestartKey((k) => k + 1);
  }, []);

  const { rows, hiddenNoise } = useMemo(() => {
    const q = search.trim().toLowerCase();
    let hidden = 0;
    const out: LogLine[] = [];
    for (const l of lines) {
      const r = l.rec;
      if (file === "access" && hideNoise && isNoise(r)) {
        hidden++;
        continue;
      }
      if (file === "access" && (status || method) && r.kind !== "access")
        continue;
      if (file === "access" && r.kind === "access") {
        if (status && statusClassOf(r.status) !== status) continue;
        if (method && r.method !== method) continue;
      }
      if (file === "error" && level && r.kind !== "error") continue;
      if (
        file === "error" &&
        level &&
        r.kind === "error" &&
        r.level.toLowerCase() !== level
      )
        continue;
      if (q && !l.text.toLowerCase().includes(q)) continue;
      out.push(l);
    }
    return { rows: out, hiddenNoise: hidden };
  }, [lines, file, search, status, method, level, hideNoise]);

  const shown = view === "table" ? rows : lines;

  // Auto-scroll to bottom on new content unless the user scrolled up.
  useEffect(() => {
    if (!autoScroll) return;
    const el = viewportRef.current;
    if (!el) return;
    suppressScrollEvent.current = true;
    el.scrollTop = el.scrollHeight;
  }, [shown, autoScroll]);

  const handleScroll = useCallback(() => {
    if (suppressScrollEvent.current) {
      suppressScrollEvent.current = false;
      return;
    }
    const el = viewportRef.current;
    if (!el) return;
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    setAutoScroll(distanceFromBottom <= SCROLL_BOTTOM_THRESHOLD_PX);
  }, []);

  return (
    <div className={styles.panel}>
      <div className={styles.toolbar}>
        <SegmentedControl
          size="sm"
          label="로그 파일"
          value={file}
          onChange={setFile}
          options={[
            { value: "access", label: "access.log" },
            { value: "error", label: "error.log" },
          ]}
        />
        <div className={styles.toolbarRight}>
          <StatusBadge tone={connected ? "success" : "warning"} pulse={connected}>{connected ? "연결됨" : "연결 끊김"}</StatusBadge>
          <SegmentedControl
            size="sm"
            label="보기 모드"
            value={view}
            onChange={setView}
            options={[
              { value: "table", label: "표" },
              { value: "raw", label: "Raw" },
            ]}
          />
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={() => {
              pendingLines.current = [];
              setLines([]);
            }}
          >
            Clear
          </Button>
        </div>
      </div>

      {/* onerror/onopen이 켜고 끈다(위 connect()) — 원인은 밝히지 않는다(로컬
          /logs와 공유하는 컴포넌트라 "노드가 죽었다"를 단정할 수 없다). 이미
          받은 줄은 아래 viewport에 그대로 남는다. retriesExhausted가 되면
          자동 재연결은 멈추고 사용자가 직접 눌러야 하는 상태로 바뀐다 — 조용히
          멈추면 "끊긴 채 멈춘 화면"이 "정상"과 구별이 안 된다. */}
      {!connected ? (
        <div className={styles.connectionBanner} role="status">
          {retriesExhausted ? (
            <>
              <span>연결이 끊어져 자동 재연결을 멈췄습니다.</span>
              {/* A quiet outline: a filled button would shout inside the warning strip. */}
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={handleManualReconnect}
              >
                다시 연결
              </Button>
            </>
          ) : (
            <span>연결이 끊어졌습니다. 자동으로 다시 연결하는 중입니다…</span>
          )}
        </div>
      ) : null}

      {view === "table" ? (
        <div className={styles.filterBar}>
          <div className={styles.search}>
            <SearchField
              size="sm"
              label="로그 검색"
              value={search}
              onChange={setSearch}
              placeholder={file === "access" ? "경로·IP·UA 검색" : "메시지 검색"}
            />
          </div>
          {file === "access" ? (
            <>
              <FilterChips
                label="상태 코드 필터"
                options={STATUS_CHIPS}
                value={status}
                onChange={(v) => setStatus(v as StatusClass)}
              />
              <div className={styles.methodSelect}>
                <Select
                  size="sm"
                  aria-label="메서드 필터"
                  value={method}
                  onChange={setMethod}
                  options={METHOD_OPTIONS}
                  placeholder="전체 메서드"
                />
              </div>
              {/* The toolbar's caption-size, muted text comes from .checkbox on the row. */}
              <Checkbox
                className={styles.checkbox}
                checked={hideNoise}
                onChange={(e) => setHideNoise(e.target.checked)}
                label={
                  <>
                    정적·텔레메트리 숨김
                    {hideNoise && hiddenNoise > 0 ? (
                      <span className={styles.hiddenCount}>
                        {hiddenNoise}건 숨김
                      </span>
                    ) : null}
                  </>
                }
              />
            </>
          ) : (
            <FilterChips
              label="레벨 필터"
              options={LEVEL_CHIPS}
              value={level}
              onChange={setLevel}
            />
          )}
          {/* 어느 시간대로 읽고 있는지 밝힌다 — 로그 시각은 오해하면 조사를 망친다. */}
          <span className={styles.timeZoneNote} suppressHydrationWarning>
            시간대 {timeZone}
          </span>
        </div>
      ) : null}

      <div
        className={
          view === "table"
            ? `${styles.viewport} ${styles.viewportTable}`
            : styles.viewport
        }
        data-log-view={view}
        ref={viewportRef}
        onScroll={handleScroll}
      >
        {shown.length === 0 ? (
          <div className={styles.empty}>
            <EmptyState description={lines.length === 0
              ? "대기 중... 새 로그 라인이 들어오면 여기에 표시됩니다."
              : "필터에 맞는 로그가 없습니다."} />
          </div>
        ) : view === "raw" ? (
          shown.map((line) => (
            <div
              key={line.id}
              className={`${styles.line} ${rawLevelClassName(file, line.text) ?? ""}`}
            >
              <time aria-hidden="true" className={styles.rawTime} title={line.rec.kind === "raw" ? undefined : formatAt(line.rec)}>{line.rec.kind === "raw" ? "—" : formatAt(line.rec).slice(-8)}</time>
              <span aria-hidden="true" className={styles.rawLevel}>{line.rec.kind === "error" ? line.rec.level : line.rec.kind === "access" ? line.rec.statusRaw : "—"}</span>
              <span className={styles.rawMessage}>{line.text}</span>
            </div>
          ))
        ) : file === "access" ? (
          <div className={styles.accessTable}><Table label="access.log">
            {/* Explicit tracks keep the path column's width when a detail or unparsed row spans
                all 8 columns on narrow screens (logs.module.css, .trackSize). */}
            <colgroup>
              <col className={styles.colTime} />
              <col className={styles.colHost} />
              <col className={styles.colIp} />
              <col className={styles.colMethod} />
              <col className={styles.colStatus} />
              <col className={styles.trackSize} />
              <col className={styles.trackPath} />
              <col className={styles.trackUa} />
            </colgroup>
            <thead>
              <Tr>
                <Th className={styles.colTime}>일시</Th>
                <Th className={styles.colHost}>도메인</Th>
                <Th className={styles.colIp}>IP</Th>
                <Th className={styles.colMethod}>메서드</Th>
                <Th className={styles.colStatus}>상태</Th>
                <Th className={`${styles.colBytes} ${styles.num}`}>크기</Th>
                <Th>경로</Th>
                <Th className={styles.colUa}>UA</Th>
              </Tr>
            </thead>
            <tbody>
              {shown.map((line) => {
                const r = line.rec;
                if (r.kind !== "access") {
                  return (
                    <Tr key={line.id}>
                      <Td colSpan={8} className={styles.rawCell}>
                        {line.text}
                      </Td>
                    </Tr>
                  );
                }
                const open = expanded === line.id;
                const sc = statusClassOf(r.status);
                return (
                  <Fragment key={line.id}>
                    <Tr
                      className={styles.row}
                      tabIndex={0}
                      onClick={() => toggleRow(line.id)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                          event.preventDefault();
                          toggleRow(line.id);
                        }
                      }}
                      aria-expanded={open}
                      aria-controls={
                        open ? `${detailPrefix}-${line.id}` : undefined
                      }
                    >
                      <Td className={`${styles.colTime} ${styles.mono}`}>
                        {formatAt(r)}
                      </Td>
                      <Td cell="primary" wrap
                        className={`${styles.colHost} ${styles.mono}`}
                        title={r.host}
                      >
                        {r.host || "—"}
                      </Td>
                      <Td className={`${styles.colIp} ${styles.mono}`}>
                        {r.ip}
                      </Td>
                      <Td className={styles.colMethod}>{r.method || "—"}</Td>
                      <Td cell="status" className={styles.colStatus}>
                        <StatusBadge
                          tone={sc ? STATUS_TONE[sc] : "neutral"}
                          size="sm"
                          dot={false}
                        >
                          {r.statusRaw}
                        </StatusBadge>
                      </Td>
                      <Td cell="hide-narrow"
                        className={`${styles.colBytes} ${styles.num} ${styles.mono}`}
                      >
                        {r.bytes}
                      </Td>
                      <Td label="경로" wrap
                        className={`${styles.pathCell} ${styles.mono}`}
                        title={r.path}
                      >
                        {r.path}
                      </Td>
                      <Td cell="hide-narrow"
                        className={`${styles.colUa} ${styles.uaCell}`}
                        title={r.ua}
                      >
                        {r.ua}
                      </Td>
                    </Tr>
                    {open ? (
                      <Tr
                        id={`${detailPrefix}-${line.id}`}
                        className={styles.detailRow}
                      >
                        <Td colSpan={8}>
                          <Facts
                              items={[
                                {
                                  term: "전체 경로",
                                  value: (
                                    <>
                                      {r.method} {r.path} {r.proto}
                                    </>
                                  ),
                                  mono: true,
                                },
                                {
                                  term: "도메인",
                                  value: r.host || "—",
                                  mono: true,
                                },
                                {
                                  term: "일시",
                                  value: (
                                    <>
                                      {formatAt(r)} ({timeZone})
                                    </>
                                  ),
                                  mono: true,
                                },
                                {
                                  term: "Referer",
                                  value: r.referer || "—",
                                  mono: true,
                                },
                                {
                                  term: "User-Agent",
                                  value: r.ua || "—",
                                  mono: true,
                                },
                                { term: "원문", value: line.text, mono: true },
                              ]}
                            />
                        </Td>
                      </Tr>
                    ) : null}
                  </Fragment>
                );
              })}
            </tbody>
          </Table></div>
        ) : (
          <Table label="error.log">
            <colgroup><col className={styles.colTime} /><col className={styles.colLevel} /><col /></colgroup>
            <thead>
              <Tr>
                <Th className={styles.colTime}>일시</Th>
                <Th className={styles.colLevel}>레벨</Th>
                <Th>메시지</Th>
              </Tr>
            </thead>
            <tbody>
              {shown.map((line) => {
                const r = line.rec;
                if (r.kind !== "error") {
                  return (
                    <Tr key={line.id}>
                      <Td colSpan={3} className={styles.rawCell}>
                        {line.text}
                      </Td>
                    </Tr>
                  );
                }
                const open = expanded === line.id;
                return (
                  <Fragment key={line.id}>
                    <Tr
                      className={styles.row}
                      tabIndex={0}
                      aria-expanded={open}
                      aria-controls={
                        open ? `${detailPrefix}-${line.id}` : undefined
                      }
                      onClick={() => toggleRow(line.id)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                          event.preventDefault();
                          toggleRow(line.id);
                        }
                      }}
                    >
                      <Td className={`${styles.colTime} ${styles.mono}`}>
                        {formatAt(r)}
                      </Td>
                      <Td cell="status" className={styles.colLevel}>
                        <StatusBadge
                          tone={levelTone(r.level)}
                          size="sm"
                          dot={false}
                        >
                          {r.level}
                        </StatusBadge>
                      </Td>
                      <Td cell="primary" wrap className={`${styles.msgCell} ${styles.mono}`}>
                        {r.message}
                      </Td>
                    </Tr>
                    {open ? (
                      <Tr
                        id={`${detailPrefix}-${line.id}`}
                        className={styles.detailRow}
                      >
                        <Td colSpan={3}>
                          <Facts
                            items={[
                              {
                                term: "일시",
                                value: (
                                  <>
                                    {formatAt(r)} ({timeZone})
                                  </>
                                ),
                              },
                              { term: "레벨", value: r.level },
                              { term: "전체 메시지", value: r.message },
                              { term: "원문", value: line.text },
                            ]}
                          />
                        </Td>
                      </Tr>
                    ) : null}
                  </Fragment>
                );
              })}
            </tbody>
          </Table>
        )}
      </div>

      {!autoScroll ? (
        <Button
          variant="secondary"
          type="button"
          className={styles.resumeButton}
          onClick={() => setAutoScroll(true)}
        >
          <Icon name="arrowDown" />
          자동 스크롤 재개
        </Button>
      ) : null}
    </div>
  );
}
