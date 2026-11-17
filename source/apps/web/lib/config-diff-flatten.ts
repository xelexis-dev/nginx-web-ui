import type { ItemDiff } from "./config-diff";
import type { DiffGroup } from "./config-diff-summary";
export interface FieldRow {
  key: string;
  path: string[];
  before: string | null;
  after: string | null;
  beforeTitle?: string;
  afterTitle?: string;
  changed: boolean;
}
export const FIELD_LABELS: Record<string, string> = {
  serverName: "server_name",
  listen: "listen",
  proxyPass: "proxy_pass 주소",
  upstreamId: "Upstream",
  upstreamName: "Upstream",
  certificateId: "인증서",
  enabled: "사용",
  name: "이름",
  protocol: "프로토콜",
  servers: "서버",
  locations: "location",
  path: "경로",
  serverDirectives: "server 블록 디렉티브",
  locationDirectives: "응답 헤더",
  directives: "디렉티브",
  tlsPort: "HTTPS 포트",
  redirectHttp: "HTTP→HTTPS 리다이렉트",
  tlsOptions: "TLS",
  http2: "HTTP/2",
  sslProtocols: "ssl_protocols",
  sslCiphers: "ssl_ciphers",
  preferServerCiphers: "ssl_prefer_server_ciphers",
  hsts: "HSTS",
  maxAge: "HSTS max-age (초)",
  includeSubDomains: "includeSubDomains",
  preload: "preload",
  ipAccess: "접근 제어",
  mode: "IP 접근 제어 모드",
  entries: "IP / CIDR",
  realIp: "실 IP 복원",
  preset: "엣지 모드",
  trustedRanges: "신뢰 프록시 IP / CIDR",
  header: "실 IP 헤더",
  recursive: "real_ip_recursive",
  cache: "프록시 캐시",
  cacheEnabled: "프록시 캐시",
  maxSize: "최대 저장 용량",
  inactive: "미사용 캐시 보관 기간",
  valid: "응답 유효기간",
  useStale: "원본 오류 시 기존 캐시 사용",
  lock: "같은 응답의 중복 요청 대기",
  statusHeader: "캐시 상태 응답 헤더",
  gzip: "gzip",
  gzipEnabled: "gzip",
  types: "압축 대상",
  minLength: "최소 응답 크기 (바이트)",
  compLevel: "압축 수준",
  vary: "Vary 응답 헤더",
  proxied: "프록시를 거친 요청도 압축",
  resolveAtRuntime: "실행 중에 이름 확인",
  retryOtherInstances: "재시작 중 다른 인스턴스로 재시도",
};
type Obj = Record<string, unknown>;
function object(value: unknown): value is Obj {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function equal(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  if (Array.isArray(a) && Array.isArray(b))
    return a.length === b.length && a.every((v, i) => equal(v, b[i]));
  return (
    object(a) &&
    object(b) &&
    Object.keys(a).length === Object.keys(b).length &&
    Object.keys(a).every((k) => Object.hasOwn(b, k) && equal(a[k], b[k]))
  );
}
function scalar(value: unknown): string | null {
  if (value == null || value === "") return null;
  if (typeof value === "boolean") return value ? "켜짐" : "꺼짐";
  return String(value);
}
function args(value: unknown): string | null {
  if (!Array.isArray(value)) return scalar(value);
  return (
    value
      .map((v) =>
        v === ""
          ? '"" (빈 값)'
          : /\s/.test(String(v)) && !/^("[\s\S]*"|'[\s\S]*')$/.test(String(v))
            ? JSON.stringify(String(v))
            : String(v),
      )
      .join(" ") || "빈 값"
  );
}
export function flattenChanges(
  group: DiffGroup,
  item: ItemDiff,
  lookups: Record<string, string> = {},
  full = false,
): FieldRow[] {
  const rows: FieldRow[] = [];
  function push(
    key: string,
    path: string[],
    a: unknown,
    b: unknown,
    format = scalar,
  ) {
    const changed = !equal(a, b);
    if (!full && !changed) return;
    rows.push({ key, path, before: format(a), after: format(b), changed });
  }
  function walk(
    a: unknown,
    b: unknown,
    path: string[],
    key: string,
    field: string,
    parentA?: Obj,
    parentB?: Obj,
  ) {
    if (!full && equal(a, b)) return;
    if (field === "upstreamId" || field === "certificateId") {
      const format = (value: unknown, parent?: Obj) =>
        value == null || value === ""
          ? null
          : field === "upstreamId" && typeof parent?.upstreamName === "string"
            ? parent.upstreamName
            : (lookups[String(value)] ?? `ID ${String(value).slice(0, 8)}`);
      rows.push({
        key,
        path,
        before: format(a, parentA),
        after: format(b, parentB),
        beforeTitle: a == null ? undefined : String(a),
        afterTitle: b == null ? undefined : String(b),
        changed: !equal(a, b),
      });
      return;
    }
    if (Array.isArray(a) || Array.isArray(b)) {
      const before = Array.isArray(a) ? a : [],
        after = Array.isArray(b) ? b : [];
      const location = field === "locations";
      const directive = [
        "serverDirectives",
        "locationDirectives",
        "directives",
      ].includes(field);
      const members = group === "upstreams" && field === "servers";
      if (location || directive || members) {
        const entries = (values: unknown[]) => {
          const seen = new Map<string, number>();
          return values.map((value) => {
            const v = object(value) ? value : {};
            const name = location
              ? String(v.path)
              : members
                ? String(value).trim().split(/\s+/)[0]!
                : v.name === "add_header" && Array.isArray(v.args)
                  ? `add_header ${String(v.args[0])}`
                  : String(v.name);
            const n = (seen.get(name) ?? 0) + 1;
            seen.set(name, n);
            return {
              id: `${name}#${n}`,
              name: name + (n > 1 ? `#${n}` : ""),
              value,
            };
          });
        };
        const left = entries(before),
          right = entries(after);
        const ids = [
          ...right.map((v) => v.id),
          ...left
            .filter((v) => !right.some((r) => r.id === v.id))
            .map((v) => v.id),
        ];
        // nginx executes location/rewrite directives in sequence, so identity matching must retain order changes.
        const sensitive =
          directive &&
          (field === "directives" ||
            [...before, ...after].some(
              (v) =>
                object(v) && /^(rewrite|return|if|break)$/.test(String(v.name)),
            ));
        const commonLeft = left.filter((v) => right.some((r) => r.id === v.id));
        const commonRight = right.filter((v) =>
          left.some((l) => l.id === v.id),
        );
        const rawSequence = (values: unknown[]) =>
          values.map((v) =>
            object(v) ? JSON.stringify([v.name, v.args]) : String(v),
          );
        const rawBefore = rawSequence(before),
          rawAfter = rawSequence(after);
        const repeatedReorder =
          !equal(rawBefore, rawAfter) &&
          equal([...rawBefore].sort(), [...rawAfter].sort());
        if (
          sensitive &&
          (commonLeft.some((v, i) => v.id !== commonRight[i]?.id) ||
            repeatedReorder)
        ) {
          const sequence = (values: unknown) =>
            (values as typeof left)
              .map((v) =>
                object(v.value)
                  ? `${String(v.value.name)} ${args(v.value.args) ?? ""}`.trim()
                  : v.name,
              )
              .join(" › ");
          push(key + ":order", [...path, "순서 변경"], left, right, sequence);
        }
        for (const id of ids) {
          const l = left.find((v) => v.id === id),
            r = right.find((v) => v.id === id),
            entry = r ?? l!;
          const nextPath = location
            ? [...path.slice(0, -1), `location ${entry.name}`]
            : directive
              ? entry.name.startsWith("add_header ")
                ? [...path.slice(0, -1), "응답 헤더", entry.name.slice(11)]
                : [...path, entry.name]
              : [...path, entry.name];
          const nextKey = `${key}:${id}`;
          if (members) {
            const remainder = (value: unknown) =>
              value == null
                ? null
                : String(value).trim().split(/\s+/).slice(1).join(" ") ||
                  "있음";
            push(nextKey, nextPath, l?.value, r?.value, remainder);
          } else if (directive) {
            const val = (value: unknown) => {
              if (!object(value)) return null;
              const list = Array.isArray(value.args) ? value.args : [];
              return args(value.name === "add_header" ? list.slice(1) : list);
            };
            // Compare raw arguments before formatting, including explicit empty arguments.
            push(nextKey, nextPath, l?.value, r?.value, val);
          } else if ((!l || !r) && !full) {
            const summary = (value: unknown) =>
              !object(value)
                ? null
                : typeof value.upstreamName === "string"
                  ? `Upstream ${value.upstreamName}`
                  : value.upstreamId
                    ? `Upstream ${lookups[String(value.upstreamId)] ?? `ID ${String(value.upstreamId).slice(0, 8)}`}`
                    : (scalar(value.proxyPass) ?? "있음");
            push(nextKey, nextPath, l?.value, r?.value, summary);
            const row = rows.at(-1)!;
            const beforeLocation = l?.value,
              afterLocation = r?.value;
            if (
              object(beforeLocation) &&
              typeof beforeLocation.upstreamId === "string"
            )
              row.beforeTitle = beforeLocation.upstreamId;
            if (
              object(afterLocation) &&
              typeof afterLocation.upstreamId === "string"
            )
              row.afterTitle = afterLocation.upstreamId;
          } else walk(l?.value, r?.value, nextPath, nextKey, "location");
        }
        return;
      }
      if ([...before, ...after].every((v) => !object(v) && !Array.isArray(v))) {
        for (const v of new Set([...before, ...after])) {
          if (before.includes(v) && after.includes(v)) {
            if (full) push(`${key}:${String(v)}`, path, v, v);
          } else
            push(
              `${key}:${String(v)}`,
              path,
              before.includes(v) ? v : undefined,
              after.includes(v) ? v : undefined,
            );
        }
      } else
        for (let i = 0; i < Math.max(before.length, after.length); i++)
          walk(
            before[i],
            after[i],
            [...path.slice(0, -1), `${path.at(-1)} ${i + 1}`],
            `${key}:${i}`,
            "element",
          );
      return;
    }
    if (object(a) || object(b)) {
      const left = object(a) ? a : {},
        right = object(b) ? b : {};
      for (const k of new Set([...Object.keys(left), ...Object.keys(right)])) {
        const label =
          k === "enabled" && field === "hsts" ? "HSTS" : (FIELD_LABELS[k] ?? k);
        walk(
          left[k],
          right[k],
          [...path, label],
          `${key}.${k}`,
          k,
          left,
          right,
        );
      }
      return;
    }
    const format =
      field === "http2" ? (v: unknown) => v === true ? "켜기" : "끄기" :
      field === "enabled" && path.length === 1 && group !== "upstreams"
        ? (v: unknown) => (v == null ? null : v ? "활성" : "비활성")
        : field === "protocol"
          ? (v: unknown) => scalar(v)?.toUpperCase() ?? null
          : scalar;
    push(key, path, field === "http2" ? a === true : a, field === "http2" ? b === true : b, format);
  }
  for (const change of item.changes) {
    if (full && change.field !== "config") continue;
    const label =
      change.field === "enabled" && group !== "upstreams"
        ? "상태"
        : group === "streams" && change.field === "proxyPass"
          ? "보낼 곳"
          : (FIELD_LABELS[change.field] ?? change.field);
    walk(
      change.before,
      change.after,
      change.field === "config" ? [] : [label],
      change.field,
      change.field,
    );
  }
  return rows;
}
