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
  Banner,
  PageHeader,
  Surface,
} from "@repo/ui";
import styles from "./logs.module.css";

type LogFile = "access" | "error";
type ViewMode = "table" | "raw";
type StatusClass = "" | "2xx" | "3xx" | "4xx" | "5xx";

const MAX_LINES = 2000;
// observability/logs.rs TAIL_LINES: only the reconnect prefix can replay this tail.
const RECONNECT_TAIL_LINES = 200;
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

function parseLine(file: LogFile, text: string, offset: number | null = null): Rec {
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
      // The writer reports seconds east of UTC; older nodes without metadata stay UTC.
      const atMs = Date.UTC(
        Number(m[1]),
        Number(m[2]) - 1,
        Number(m[3]),
        Number(m[4]),
        Number(m[5]),
        Number(m[6]),
      ) - (offset ?? 0) * 1000;
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
  offset: number | null;
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

function recordTone(rec: Rec): Tone {
  if (rec.kind === "error") return levelTone(rec.level);
  const status = rec.kind === "access" ? statusClassOf(rec.status) : "";
  return status ? STATUS_TONE[status] : "neutral";
}

function isNoise(rec: Rec): boolean {
  return (
    rec.kind === "access" &&
    (rec.path.startsWith("/_next/static/") ||
      rec.path.startsWith("/service/rest/v1/user-telemetry/"))
  );
}

function levelGroup(rec: Rec): string {
  if (rec.kind !== "error") return "other";
  const level = rec.level.toLowerCase();
  if (["crit", "alert", "emerg"].includes(level)) return "crit";
  return ["error", "warn", "notice", "info"].includes(level) ? level : "other";
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
  { value: "crit", label: "crit 이상" },
  { value: "error", label: "error" },
  { value: "warn", label: "warn" },
  { value: "notice", label: "notice" },
  { value: "info", label: "info" },
];

/**
 * Live access/error log tail. Streams raw lines over SSE, parses each into a
 * structured record on arrival, and renders either a scannable Table (default)
 * or the Raw text.
 * Filters apply to both the structured table and the raw text.
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
  const [connection, setConnection] = useState<"connecting" | "live" | "lost">("connecting");
  const [waiting, setWaiting] = useState(0);
  const [copyNotice, setCopyNotice] = useState("");
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
    if (expanded !== id) pause();
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
  const detailRef = useRef<HTMLDivElement | null>(null);
  const received = useRef<LogLine[]>([]);
  const nextId = useRef(0);
  const pendingLines = useRef<LogLine[]>([]);
  const following = useRef(true);
  const arrivalCount = useRef(0);
  const fileRef = useRef(file);
  fileRef.current = file;

  // Each tab is an independent stream — switching tabs starts fresh.
  // connection/retriesExhausted도 같이 되돌린다 — 안 그러면 이전 탭에서 재연결을
  // 포기한 상태가 방금 새로 시작한 스트림 위에 그대로 남아 거짓 배너가 뜬다.
  useEffect(() => {
    pendingLines.current = [];
    received.current = [];
    setLines([]);
    following.current = true;
    arrivalCount.current = 0;
    setWaiting(0);
    setAutoScroll(true);
    setExpanded(null);
    setConnection("connecting");
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

    function append(text: string, offset: number | null) {
      arrivalCount.current += 1;
      const line = { id: nextId.current++, text, offset, rec: parseLine(fileRef.current, text, offset) };
      received.current.push(line);
      if (received.current.length > MAX_LINES) received.current.shift();
      pendingLines.current.push(line);
      if (pendingLines.current.length > MAX_LINES) pendingLines.current.shift();
    }

    function connect() {
      const held = received.current.slice(-RECONNECT_TAIL_LINES);
      let candidates = held.map((_, index) => index);
      let replay: { text: string; offset: number | null }[] = [];
      let offset: number | null = null;
      let overlap = 0;
      // Match a contiguous suffix, not a set: identical new log lines must survive.
      let matching = held.length > 0;
      es = new EventSource(`${streamPath}?file=${file}`);
      es.addEventListener("offset", (event) => {
        if (cancelled || file !== "error" || !/^[+-]?\d+$/.test(event.data.trim())) return;
        const value = Number(event.data);
        if (Number.isInteger(value) && Math.abs(value) <= 86400) offset = value;
      });
      es.onopen = () => {
        if (cancelled) return;
        attempt = 0;
        setConnection("live");
        setRetriesExhausted(false);
      };
      es.onmessage = (event) => {
        if (cancelled) return;
        if (!matching) { append(event.data, offset); return; }
        replay.push({ text: event.data, offset });
        candidates = candidates.filter((start) => held[start + replay.length - 1]?.text === event.data);
        if (candidates.some((start) => start + replay.length === held.length)) overlap = replay.length;
        candidates = candidates.filter((start) => start + replay.length < held.length);
        if (candidates.length) return;
        matching = false;
        // A replay can supply metadata an old node never sent. Update only those exact records.
        const updates = new Map<number, LogLine>();
        held.slice(held.length - overlap).forEach((line, index) => {
          const nextOffset = replay[index]!.offset;
          if (nextOffset !== null && nextOffset !== line.offset) updates.set(line.id, {
            ...line, offset: nextOffset, rec: parseLine(file, line.text, nextOffset),
          });
        });
        if (updates.size) {
          const apply = (records: LogLine[]) => records.map((line) => updates.get(line.id) ?? line);
          received.current = apply(received.current);
          pendingLines.current = apply(pendingLines.current);
          setLines(apply);
        }
        replay.slice(overlap).forEach((line) => append(line.text, line.offset));
        replay = [];
      };
      es.onerror = () => {
        es?.close();
        if (cancelled) return;
        // 이미 받은 줄은 지우지 않는다 — 끊긴 순간에도 지금까지의 로그는
        // 여전히 유효한 정보다. 원인은 모른다: 이 컴포넌트는 로컬 /logs와
        // 노드 화면이 공유하므로 "노드가 죽었다"처럼 노드 탓으로 단정하지
        // 않는다 — 로컬 화면에서는 그 말이 거짓이 된다.
        setConnection("lost");
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
      if (!following.current) {
        setWaiting(arrivalCount.current);
        return;
      }
      arrivalCount.current = 0;
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
    setConnection("connecting");
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
      if (file === "error" && level && levelGroup(r) !== level) continue;
      if (q && !l.text.toLowerCase().includes(q)) continue;
      out.push(l);
    }
    return { rows: out, hiddenNoise: hidden };
  }, [lines, file, search, status, method, level, hideNoise]);

  const shown = rows;
  const otherCount = lines.filter(({ rec }) => levelGroup(rec) === "other").length;
  const levelChips = otherCount || level === "other" ? [...LEVEL_CHIPS, { value: "other", label: "기타" }] : LEVEL_CHIPS;
  const chips = (file === "access" ? STATUS_CHIPS : levelChips).map((chip) => ({
    ...chip,
    count: lines.filter(({ rec }) => !chip.value || (file === "access"
      ? rec.kind === "access" && statusClassOf(rec.status) === chip.value
      : levelGroup(rec) === chip.value)).length,
  }));

  function pause() {
    following.current = false;
    setAutoScroll(false);
  }

  function resume() {
    const batch = pendingLines.current;
    pendingLines.current = [];
    arrivalCount.current = 0;
    setLines((prev) => [...prev, ...batch].slice(-MAX_LINES));
    setWaiting(0);
    setExpanded(null);
    following.current = true;
    setAutoScroll(true);
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      setCopyNotice("원문을 복사했습니다.");
    } catch {
      setCopyNotice("복사하지 못했습니다. 원문을 선택해 복사해 주세요.");
    }
  }

  function clearFilters() {
    setSearch(""); setStatus(""); setMethod(""); setLevel(""); setHideNoise(false);
  }


  // Auto-scroll to bottom on new content unless the user scrolled up.
  useEffect(() => {
    if (!autoScroll) return;
    const el = viewportRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [shown, autoScroll, view]);

  useEffect(() => {
    const viewport = viewportRef.current;
    const detail = detailRef.current;
    if (expanded === null || view !== "table" || !viewport || !detail) return;
    const box = viewport.getBoundingClientRect();
    const top = box.top + viewport.clientTop + (viewport.querySelector("thead")?.offsetHeight ?? 0);
    const bottom = box.top + viewport.clientTop + viewport.clientHeight;
    const bounds = detail.getBoundingClientRect();
    const delta = bounds.height > bottom - top || bounds.top < top
      ? bounds.top - top : Math.max(0, bounds.bottom - bottom);
    if (delta) viewport.scrollBy({ top: delta });
  }, [expanded, view]);

  const handleScroll = () => {
    const el = viewportRef.current;
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight > SCROLL_BOTTOM_THRESHOLD_PX) pause();
  };

  const badge: { tone: Tone; text: string } = connection === "connecting" ? { tone: "info", text: "연결 중" }
    : connection === "lost" ? { tone: "warning", text: "연결 끊김" }
    : autoScroll ? { tone: "success", text: "실시간 수신 중" }
    : { tone: "neutral", text: "일시 정지" };
  const last = lines.at(-1)?.rec;
  const lastTime = last && last.kind !== "raw" ? formatAt(last).slice(-8) : null;
  const isFiltered = Boolean(search.trim() || (file === "access" ? status || method || hideNoise : level));
  const number = (value: number) => value.toLocaleString("en-US");
  const size = (value: string) => /^\d+$/.test(value) ? number(Number(value)) : value;
  const time = (rec: Rec) => {
    if (rec.kind === "raw") return "—";
    const value = formatAt(rec);
    const split = value.lastIndexOf(" ");
    return split < 0 ? value : <><span className={styles.date}>{value.slice(0, split)}</span> {value.slice(split + 1)}</>;
  };
  const empty = <EmptyState
    description={lines.length === 0 ? connection === "connecting" ? "연결하는 중…" : "대기 중입니다. 새 로그 줄이 들어오면 여기에 표시됩니다." : "필터에 맞는 로그가 없습니다."}
    action={lines.length > 0 && isFiltered ? <Button size="sm" variant="secondary" onClick={clearFilters}>필터 지우기</Button> : undefined}
  />;

  return <>
    <PageHeader title="로그" info="access.log / error.log를 실시간으로 확인합니다."
      status={<StatusBadge tone={badge.tone} pulse={connection === "live" && autoScroll}>{badge.text}</StatusBadge>} />
    <Surface>
      <div className={styles.toolbar}>
        <div className={styles.filters}>
          <SegmentedControl label="로그 파일" value={file} onChange={setFile}
            options={[{ value: "access", label: "access.log" }, { value: "error", label: "error.log" }]} />
          <div className={styles.search}><SearchField label="로그 검색" value={search} onChange={setSearch}
            placeholder={file === "access" ? "경로·IP·UA 검색" : "메시지 검색"} /></div>
          <FilterChips label={file === "access" ? "상태 코드 필터" : "레벨 필터"} options={chips}
            value={file === "access" ? status : level} onChange={(value) => file === "access" ? setStatus(value as StatusClass) : setLevel(value)} />
          {file === "access" ? <>
            <div className={styles.methodSelect}><Select aria-label="메서드 필터" value={method} onChange={setMethod} options={METHOD_OPTIONS} /></div>
            <Checkbox className={styles.checkbox} checked={hideNoise} onChange={(event) => setHideNoise(event.target.checked)}
              label={<>정적·텔레메트리 숨김{hideNoise && hiddenNoise > 0 ? ` (${number(hiddenNoise)}건 숨김)` : ""}</>} />
          </> : null}
        </div>
        <div className={styles.meta}><span>{`${number(shown.length)}줄`}</span>
          <SegmentedControl label="보기 모드" value={view} onChange={setView} options={[{ value: "table", label: "표" }, { value: "raw", label: "Raw" }]} />
        </div>
      </div>
      {connection === "lost" ? <div className={styles.banner}><Banner tone="warning"
        title={retriesExhausted ? "연결이 끊어져 자동 재연결을 멈췄습니다." : "연결이 끊어졌습니다. 자동으로 다시 연결하는 중입니다…"}
        action={retriesExhausted ? <Button size="sm" variant="secondary" onClick={handleManualReconnect}><Icon name="refresh" />다시 연결</Button> : undefined} /></div> : null}
      {view === "raw" ? <section className={styles.rawPane} aria-label="원문 로그">
        <div className={styles.rawHead}><div><h2>{file}.log</h2><p>원문 {number(shown.length)}줄 · 앞의 시각과 상태는 복사되지 않습니다</p></div>
          <Button size="sm" variant="secondary" onClick={() => copy(shown.map((line) => line.text).join("\n"))}><Icon name="copy" />보이는 줄 복사</Button></div>
        <div className={styles.rawViewport} data-log-view="raw" ref={viewportRef} onScroll={handleScroll} tabIndex={0} aria-label="원문 로그 줄">
          {shown.length === 0 ? empty : shown.map((line) => <div key={line.id} className={styles.line}>
            <time aria-hidden="true" className={styles.rawTime}>{line.rec.kind === "raw" ? "—" : formatAt(line.rec).slice(-8)}</time>
            <span aria-hidden="true" data-tone={recordTone(line.rec)} className={styles.rawLevel}>{line.rec.kind === "error" ? line.rec.level : line.rec.kind === "access" ? line.rec.statusRaw : "원문"}</span>
            <span className={styles.rawMessage}>{line.text}</span>
          </div>)}
        </div>
      </section> : <>
        {connection === "connecting" && !lines.length ? <div className={styles.progress} role="progressbar" aria-label="로그에 연결하는 중" /> : null}
        <div className={styles.viewport} aria-busy={connection === "connecting" && !lines.length} data-log-view="table" ref={viewportRef} onScroll={handleScroll}>
          <Table label={`${file}.log`} layout="fixed">
            <colgroup><col className={styles.colTime} /><col className={file === "error" ? styles.colLevel : styles.colStatus} />
              {file === "access" ? <><col className={styles.colMethod} /><col className={styles.colHost} /><col /><col className={styles.colBytes} /><col className={styles.colIp} /></> : <col />}
              <col className={styles.colAction} /></colgroup>
            <thead><Tr><Th>일시 <span className={styles.zone}>({timeZone})</span></Th><Th>{file === "access" ? "상태" : "레벨"}</Th>
              {file === "access" ? <><Th>메서드</Th><Th>도메인</Th><Th>경로</Th><Th align="end">크기</Th><Th>IP</Th></> : <Th>메시지</Th>}
              <Th>상세</Th></Tr></thead>
            <tbody>
              {!shown.length && connection === "connecting" ? Array.from({ length: 6 }, (_, row) => <Tr key={row} aria-hidden="true">{Array.from({ length: file === "access" ? 8 : 4 }, (_, cell) => <Td key={cell} cell={cell === 1 ? "status" : cell === (file === "access" ? 4 : 2) ? "primary" : cell === (file === "access" ? 7 : 3) ? "action" : "meta"}><span className={styles.skeleton} /></Td>)}</Tr>) : !shown.length ? <Tr><Td colSpan={file === "access" ? 8 : 4}><div className={styles.empty}>{empty}</div></Td></Tr> : shown.map((line) => {
                const r = line.rec;
                const open = expanded === line.id;
                const title = r.kind === "access" ? r.path : r.kind === "error" ? r.message : line.text;
                const facts = r.kind === "access" ? [
                  { term: "전체 경로", value: `${r.method} ${r.path} ${r.proto}`, wide: true },
                  { term: "도메인", value: r.host || "—" }, { term: "IP", value: r.ip },
                  { term: "일시", value: `${formatAt(r)} (${timeZone})` }, { term: "크기", value: `${size(r.bytes)} bytes` },
                  { term: "Referer", value: r.referer || "—", wide: true }, { term: "User-Agent", value: r.ua || "—", wide: true },
                ] : r.kind === "error" ? [{ term: "일시", value: `${formatAt(r)} (${timeZone})` }, { term: "레벨", value: r.level }, { term: "전체 메시지", value: r.message, wide: true }] : [];
                return <Fragment key={line.id}>
                  <Tr className={styles.row} data-open={open ? "" : undefined} tabIndex={0} aria-expanded={open}
                    aria-controls={open ? `${detailPrefix}-${line.id}` : undefined}
                    onClick={() => toggleRow(line.id)} onKeyDown={(event) => {
                      if (event.target === event.currentTarget && (event.key === "Enter" || event.key === " ")) { event.preventDefault(); toggleRow(line.id); }
                    }}>
                    <Td className={styles.time}>{time(r)}</Td>
                    <Td cell="status"><StatusBadge tone={recordTone(r)} variant={r.kind === "error" && levelGroup(r) === "crit" ? "solid" : "soft"} size="sm" dot={false}>
                      {r.kind === "access" ? r.statusRaw : r.kind === "error" ? r.level : "원문"}</StatusBadge></Td>
                    {file === "access" ? <><Td>{r.kind === "access" ? r.method || "—" : "—"}</Td><Td truncate title={r.kind === "access" ? r.host : undefined}>{r.kind === "access" ? r.host || "—" : "—"}</Td></> : null}
                    <Td cell="primary" truncate className={styles.message} title={title}>{title}</Td>
                    {file === "access" ? <><Td cell="hide-narrow" numeric>{r.kind === "access" ? size(r.bytes) : "—"}</Td><Td cell="hide-narrow">{r.kind === "access" ? r.ip : "—"}</Td></> : null}
                    <Td cell="action"><Button size="sm" variant="ghost" aria-expanded={open} aria-controls={open ? `${detailPrefix}-${line.id}` : undefined}
                      onClick={(event) => { event.stopPropagation(); toggleRow(line.id); }}>{open ? "접기" : "보기"}</Button></Td>
                  </Tr>
                  {open ? <Tr id={`${detailPrefix}-${line.id}`} className={styles.detailRow}><Td colSpan={file === "access" ? 8 : 4}>
                    <div ref={detailRef} className={styles.detail}><Facts columns={2} items={[...facts, { term: "원문", value: line.text, wide: true }]} />
                      <div><Button size="sm" variant="secondary" onClick={() => copy(line.text)}><Icon name="copy" />원문 복사</Button></div></div>
                  </Td></Tr> : null}
                </Fragment>;
              })}
            </tbody>
          </Table>
        </div>
      </>}
      <div className={styles.foot}>
        <p className={styles.tail}>{!autoScroll ? <><b>새 줄 {number(waiting)}개</b>가 기다리고 있습니다{lastTime ? ` · 화면의 마지막 줄 ${lastTime}` : ""}</>
          : lastTime ? <>마지막 줄 <b>{lastTime}</b></> : lines.length ? `받은 줄 ${number(lines.length)}개` : connection === "connecting" ? "연결하는 중…" : "아직 받은 줄이 없습니다"}</p>
        <div className={styles.actions}><Button size="sm" variant="ghost" onClick={() => {
          pendingLines.current = []; received.current = []; arrivalCount.current = 0; setWaiting(0); setLines([]); setExpanded(null);
        }}>화면 비우기</Button>
          {!autoScroll ? <Button size="sm" variant="secondary" onClick={resume}><Icon name="play" />이어 보기</Button>
            : connection === "live" ? <Button size="sm" variant="secondary" onClick={pause}><Icon name="pause" />일시 정지</Button> : null}
        </div>
      </div>
      {copyNotice ? <p role="status" className={styles.copyNotice}>{copyNotice}</p> : null}
    </Surface>
  </>;
}
