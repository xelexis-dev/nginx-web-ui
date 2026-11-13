import { carryDirectives, type OriginalDirectives } from "./directive-carry";
import type { DirectiveEditorState, HeaderRow } from "./directive-state";
import { stateToDirectives, emptyDirectiveState } from "./directive-state";
import type { DirectiveContext } from "./directive-validation";
import type { CacheConfig, GzipConfig, LocationConfig } from "./types";

export type SettingSummary = "켜짐" | "꺼짐" | "직접 설정";

/** Only names leave this helper: header values may contain credentials. */
export function directiveSummary(state: DirectiveEditorState, original?: OriginalDirectives, context: DirectiveContext = "location") {
  const carried = carryDirectives(original, state);
  let stored = Array.isArray(carried) ? carried : undefined;
  if (!stored) {
    // In-progress invalid text stays in the editor; the summary can still name structured headers.
    try { stored = carried === null ? [] : stateToDirectives(state, context); } catch { /* Not a validation boundary. */ }
  }
  const headers: HeaderRow[] = stored
    ? stored.filter((d) => d.name === "proxy_set_header").map((d) => ({ name: d.args[0] ?? "", value: d.args[1] ?? "" }))
    : state.headers;
  const responseNames = stored
    ? stored.filter((d) => d.name === "add_header").map((d) => d.args[0] ?? "")
    : state.addHeaders.map((h) => h.name);
  const upgrade = headers.filter((h) => h.name.trim().toLowerCase() === "upgrade");
  const connection = headers.filter((h) => h.name.trim().toLowerCase() === "connection");
  const value = (name: string, current: string) => stored ? stored.find((d) => d.name === name)?.args[0] ?? "" : current;
  const httpVersion = value("proxy_http_version", state.httpVersion);
  const webSocket: SettingSummary = httpVersion === "1.1" && upgrade.length === 1 && connection.length === 1 &&
    upgrade[0]!.value === "$http_upgrade" && connection[0]!.value === "$connection_upgrade"
    ? "켜짐"
    : upgrade.length || connection.length || /(?:^|[;\n])\s*proxy_set_header\s+(?:Upgrade|Connection)\b/i.test(state.advanced) ? "직접 설정" : "꺼짐";
  return {
    requestHeaderNames: headers.map((h) => h.name.trim()).filter(Boolean),
    responseHeaderNames: responseNames.map((name) => name.trim()).filter(Boolean),
    webSocket,
    timeouts: { read: value("proxy_read_timeout", state.timeouts.read), send: value("proxy_send_timeout", state.timeouts.send), connect: value("proxy_connect_timeout", state.timeouts.connect) },
    bodySize: value("client_max_body_size", state.bodySize),
    httpVersion,
    buffering: value("proxy_buffering", state.buffering),
    implicitHost: carried === null,
    advancedLineCount: state.advanced.split(/\r?\n/).filter((line) => line.trim() && !line.trim().startsWith("#")).length,
  };
}

export interface SummaryLocation extends Pick<LocationConfig, "path" | "resolveAtRuntime" | "cacheEnabled" | "gzipEnabled" | "ipAccess" | "retryOtherInstances"> {
  mode: "upstream" | "direct";
  upstreamId: string;
  proxyPass: string;
  directives: DirectiveEditorState;
  originalDirectives?: OriginalDirectives;
  upstreamError?: string;
}

export function routeRetryInfo(
  route: SummaryLocation,
  server: DirectiveEditorState = emptyDirectiveState(),
) {
  const own = directiveSummary(
    route.directives,
    route.originalDirectives,
  ).timeouts;
  const inherited = directiveSummary(server, undefined, "server").timeouts;
  const on = route.retryOtherInstances === true;
  const connect = own.connect || (on ? "2s" : inherited.connect);
  const source = own.connect
    ? "route"
    : on
      ? "retry"
      : inherited.connect
        ? "server"
        : "default";
  const connectDisplay =
    source === "retry"
      ? "2초 (재시도 설정)"
      : source === "route"
        ? `${connect} (경로 값)`
        : source === "server"
          ? `서버 값 사용 (${connect})`
          : "nginx 기본값";
  const summary = `재시도: 최대 2회 · 연결 ${own.connect ? `${own.connect}(경로 값)` : "2초"} · 3초 안에 재시도`;
  const parts = [...own.connect.matchAll(/(\d+)(ms|[wdhms])?/g)];
  const units: Record<string, number> = {
    w: 604800000,
    d: 86400000,
    h: 3600000,
    m: 60000,
    s: 1000,
    ms: 1,
  };
  const milliseconds =
    parts.map((part) => part[0]).join("") === own.connect
      ? parts.reduce(
          (total, part) =>
            total + Number(part[1]) * (units[part[2] ?? "s"] ?? 1000),
          0,
        )
      : 0;
  return {
    connect,
    connectDisplay,
    source,
    read: own.read || inherited.read || "60s",
    summary,
    replacedServer: on && !own.connect ? inherited.connect : "",
    warning:
      on && !!own.connect && milliseconds >= 3000
        ? `연결 대기(${own.connect})가 재시도 시작 한도 3초 이상이라, 연결이 맺어지지 않는 인스턴스에서는 다른 인스턴스로 넘어가지 않습니다.`
        : undefined,
  };
}

export type RetryInfo=ReturnType<typeof routeRetryInfo>;

export interface ServerSummaryInput {
  serverName: string;
  listen: string | number;
  locations: readonly SummaryLocation[];
  upstreams: readonly { id: string; name: string }[];
  serverDirectives: DirectiveEditorState;
  originalServerDirectives?: OriginalDirectives;
  cache?: CacheConfig;
  gzip?: GzipConfig;
}

export function serverSummary(input: ServerSummaryInput) {
  const names = input.serverName.split(/\s+/).filter(Boolean);
  const upstreams = new Map(input.upstreams.map((upstream) => [upstream.id, upstream.name]));
  const serverCarried = carryDirectives(input.originalServerDirectives, input.serverDirectives);
  const legacyGzip = Array.isArray(serverCarried)
    ? serverCarried.some((directive) => /^gzip(?:_|$)/.test(directive.name))
    : /(?:^|[;\n])\s*gzip(?:_[a-z_]+)?\s/m.test(input.serverDirectives.advanced);
  const gzip: SettingSummary = input.gzip ? (input.gzip.enabled ? "켜짐" : "꺼짐") : legacyGzip ? "직접 설정" : "꺼짐";
  return {
    names,
    listen: String(input.listen),
    locations: input.locations.map((location) => ({
      path: location.path,
      target: location.mode === "upstream" ? upstreams.get(location.upstreamId) ?? "Upstream 없음" : location.proxyPass || "주소 미입력",
      kind: location.mode,
      ...(location.mode === "direct" && location.resolveAtRuntime ? { resolveAtRuntime: true } : {}),
      ...(location.mode === "upstream" && !location.upstreamId && location.upstreamError ? { error: location.upstreamError } : {}),
      directives: directiveSummary(location.directives, location.originalDirectives),
      retryInfo: routeRetryInfo(location,input.serverDirectives),
      ...(location.retryOtherInstances===true?{retryOtherInstances:true as const}:{}),
      ...(location.cacheEnabled !== undefined ? { cacheEnabled: location.cacheEnabled } : {}),
      ...(location.gzipEnabled !== undefined ? { gzipEnabled: location.gzipEnabled } : {}),
      ...(location.ipAccess ? { ipEntryCount: location.ipAccess.entries.length } : {}),
    })),
    cache: input.cache?.enabled ? "켜짐" as const : "꺼짐" as const,
    gzip,
    cacheOverrides: input.locations.filter((location) => location.cacheEnabled !== undefined).length,
    gzipOverrides: input.locations.filter((location) => location.gzipEnabled !== undefined).length,
    ipOverrides: input.locations.filter((location) => location.ipAccess !== undefined).length,
    serverDirectives: directiveSummary(input.serverDirectives, input.originalServerDirectives, "server"),
  };
}

export type DirectiveSummary = ReturnType<typeof directiveSummary>;
export type ServerSummary = ReturnType<typeof serverSummary>;
