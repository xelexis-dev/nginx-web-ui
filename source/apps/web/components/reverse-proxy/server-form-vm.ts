import type { NodeLocation, NodeLocationInput } from "@repo/contracts";
import type { CacheConfig, Directive, GzipConfig, IpAccess, RealIpConfig, TlsTuning } from "@repo/presentation/nginx";
import { parseCacheConfig, parseGzipConfig, parsePerformanceOverride } from "@repo/presentation/nginx/performance";
import { carryDirectives, type DirectiveSubmission } from "@repo/presentation/nginx/directive-carry";
// Editor conversions are pure presentation helpers shared by local and node forms.
import { directivesToState, seededLocationDirectiveState, type DirectiveEditorState } from "@repo/presentation/nginx/directive-state";
import { storedConfig, type ServerSourceRow } from "./server-vm";

/**
 * 리버스 프록시 **폼**이 로컬·노드 양쪽에서 공유하는 타입과 그 변환.
 *
 * `server-vm.ts` 와 나눠 둔 이유는 하나다: 저쪽은 **값 의존이 0** 인 파일이고
 * (그 헤더가 그렇게 약속한다), 이 파일은 `directivesToState` 같은 **값**을
 * 들여온다. 한 파일로 합치면 그 약속이 조용히 깨진다.
 *
 * **`LocationInput`·`CertificateOption` 의 정의가 여기로 옮겨 왔다**(원래
 * `app/(app)/reverse-proxy/types.ts`). 폼과 그 섹션들이 공유 컴포넌트가
 * 되면서, 라우트 그룹의 TS 를 가져오는 것이 금지이기 때문이다
 * (`components/upstreams/upstream-list.tsx` 헤더의 불변식) —
 * `StoredServerConfig` 가 `server-vm.ts` 로 옮겨간 것과 같은 이유·같은 방식이다.
 */

/**
 * One editable location row (client state; serialized to a hidden JSON field).
 *
 * 폼 컨테이너(`server-form.tsx`)가 아니라 여기 사는 이유는 예전과 같다 — 탭
 * 섹션들이 컨테이너를 거슬러 올라가지 않고 이 타입을 쓸 수 있어야 한다.
 */
export interface LocationInput extends Pick<NodeLocationInput, "path" | "resolveAtRuntime" | "retryOtherInstances"> {
  mode: NonNullable<NodeLocationInput["mode"]>;
  upstreamId: string;
  proxyPass: string;
  directives: DirectiveEditorState;
  /** null preserves a missing stored key; undefined is a new route. */
  originalDirectives?: Directive[] | null;
  upstreamError?: string;
  /** location 범위 IP allow/deny (whitelist/blacklist). 미설정 시 서버 레벨만 적용. */
  ipAccess?: IpAccess;
  cacheEnabled?: boolean;
  gzipEnabled?: boolean;
}

export const MISSING_UPSTREAM_MESSAGE = "연결된 Upstream을 찾을 수 없습니다(삭제됐거나 이 서버 범위에 없음). 다른 Upstream을 고르거나 직접 주소로 바꾸세요.";

export function newLocationInput(path = "/"): LocationInput {
  return { path, mode: "upstream", upstreamId: "", proxyPass: "", directives: seededLocationDirectiveState() };
}

/** The form accepts editor objects too; the REST shape is array/null after API normalisation. */
export type LocationSubmission = Pick<NodeLocationInput, "path" | "upstreamId" | "proxyPass" | "resolveAtRuntime" | "ipAccess" | "cacheEnabled" | "gzipEnabled" | "retryOtherInstances"> & {
  mode: NonNullable<NodeLocationInput["mode"]>;
  /** Browser form commands additionally accept editor objects before API normalisation. */
  directives: DirectiveSubmission;
};

export function toLocationSubmission(location: LocationInput): LocationSubmission {
  return {
    path: location.path,
    mode: location.mode,
    ...(location.mode === "upstream" ? { upstreamId: location.upstreamId } : { proxyPass: location.proxyPass }),
    directives: carryDirectives(location.originalDirectives, location.directives),
    ...(location.retryOtherInstances===true?{retryOtherInstances:true}:{}),
    ...(location.mode === "direct" && location.resolveAtRuntime === true ? { resolveAtRuntime: true } : {}),
    ...(location.ipAccess != null ? { ipAccess: location.ipAccess } : {}),
    ...(location.cacheEnabled !== undefined ? { cacheEnabled: location.cacheEnabled } : {}),
    ...(location.gzipEnabled !== undefined ? { gzipEnabled: location.gzipEnabled } : {}),
  };
}

export function serializeLocations(locations: readonly LocationInput[]): string {
  return JSON.stringify(locations.map(toLocationSubmission));
}

/** 발급 완료(issued) 인증서만 폼에 내려온다 — TLS Select 후보. */
export interface CertificateOption {
  id: string;
  label: string;
  staging: boolean;
  domains: string[];
}

/**
 * 인증서 후보 하나의 **원본**. 로컬은 `certificates` 테이블 행, 노드는
 * `GET /api/v1/certificates` 의 한 항목(`api/v1/certificates/view.ts` 의
 * 허용목록)이 그대로 이 모양이다 — 폼이 필요로 하는 넷이 양쪽에 다 있다.
 */
export interface CertificateSourceRow {
  id: string;
  domains: unknown;
  staging: boolean;
  status: string;
  /** 사람이 붙인 이름. 별칭을 모르는 구 노드의 응답에는 키 자체가 없다 — 그래서 선택. */
  alias?: string | null;
}

/**
 * 인증서 목록 → TLS Select 후보. **`status === "issued"` 만 남긴다.**
 *
 * 로컬 화면은 같은 조건을 SQL 에서 이미 걸지만(`where: eq(c.status, "issued")`),
 * **노드는 목록 전체를 받는다** — 어느 인증서가 필요한지 알기 전에 경로 집합이
 * 정해지므로 `/api/v1/certificates` 를 통째로 받아 여기서 거른다. 그래서 이
 * 필터는 노드 쪽에서 실제로 일한다: 빼면 발급 전·실패한 인증서가 후보로 뜨고,
 * 고르면 노드가 저장은 하지만 `applyAll` 이 TLS 블록을 쓰지 않아 **HTTPS 가 안
 * 되는데 화면은 「연결됨」**이 된다.
 *
 * 라벨 규칙(별칭이 있으면 `별칭 (domains[0] ?? id)`, 없으면 `domains[0] ?? id`)도
 * 한 벌뿐이어야 한다 — 로컬과 노드가 갈리면 같은 인증서가 두 화면에서 다른
 * 이름으로 보인다.
 */
export function toCertificateOptions(rows: readonly CertificateSourceRow[]): CertificateOption[] {
  return rows
    .filter((c) => c.status === "issued")
    .map((c) => {
      const domains = Array.isArray(c.domains) ? (c.domains as string[]) : [];
      const primary = domains[0] ?? c.id;
      // 별칭이 있어도 대표 도메인은 괄호로 남긴다 — 어느 도메인 인증서인지는 계속 보여야 한다.
      const label = c.alias ? `${c.alias} (${primary})` : primary;
      return { id: c.id, label, staging: c.staging, domains };
    });
}

/**
 * `ServerForm` 의 `defaultValues` — **이름을 붙여 뒀다.** 수정 화면 둘(로컬·노드)이
 * 이 모양을 만들어 넘기므로, 폼 안에 익명 객체 타입으로 두면 그 변환 함수의
 * 반환 타입을 적을 수가 없다.
 */
export interface ServerFormValues {
  version?: string;
  resolveAtRuntime?: boolean;
  /** edit 시에만 존재 — 템플릿 프리필 생성(defaultValues만 있고 id 없음)엔 없다. */
  id?: string;
  serverName: string;
  listen: number;
  upstreamId: string | null;
  proxyPass: string | null;
  /** Canonical routes, including a converted single target. Optional for new/template callers. */
  locations?: LocationInput[];
  convertedFromSingle?: boolean;
  /** server{} directives editor state. */
  serverDirectives?: DirectiveEditorState;
  originalServerDirectives?: Directive[] | null;
  /** Implicit `location /` directives editor state (single-target mode). */
  locationDirectives?: DirectiveEditorState;
  /** TLS: 연결된 인증서 + HTTPS 포트/리다이렉트 옵션. */
  certificateId?: string | null;
  tlsPort?: number;
  redirectHttp?: boolean;
  /** TLS 튜닝(HSTS/ssl_protocols/ssl_ciphers/prefer_server_ciphers). */
  tlsOptions?: TlsTuning;
  /** 서버 전체에 적용되는 IP allow/deny (whitelist/blacklist). */
  ipAccess?: IpAccess;
  /** 엣지/프록시 뒤 배치 실 IP 복원. */
  realIp?: RealIpConfig;
  cache?: CacheConfig;
  gzip?: GzipConfig;
}

/** 수정 화면이 보는 서버 한 행. 로컬은 DB 행, 노드는 `GET /api/v1/servers/:id` 의 `server`. */
export type ServerFormSourceRow = ServerSourceRow & { certificateId: string | null };

/** Local and node edit pages share the same canonical per-route conversion and presence carry. */
function storedDirectives(value: Directive[] | undefined): Directive[] | null {
  if (value === undefined) return null;
  if (!Array.isArray(value)) throw new Error("디렉티브 데이터 형식이 올바르지 않습니다.");
  return value;
}

function runtimeDnsFlag(value: unknown): boolean {
  if (value === undefined) return false;
  if (typeof value !== "boolean") throw new Error("실행 중 이름 확인 값은 참 또는 거짓이어야 합니다.");
  return value;
}

function retryFlag(value: unknown): boolean {
  if(value===undefined)return false;
  if(typeof value!=="boolean")throw new Error("재시도 설정은 참 또는 거짓이어야 합니다.");
  return value;
}

export function toServerFormValues(
  server: ServerFormSourceRow,
  upstreams: readonly { id: string; name: string }[],
): ServerFormValues {
  const cfg = storedConfig(server.config);
  const resolveAtRuntime = runtimeDnsFlag(cfg.resolveAtRuntime);
  const proxyPass = server.upstreamId ? null : (cfg.proxyPass ?? null);

  const byId = new Map(upstreams.map((u) => [u.id, u]));
  const byUrl = new Map(upstreams.map((u) => [`http://${u.name}`, u.id]));
  const convertedFromSingle = !Array.isArray(cfg.locations) || cfg.locations.length === 0;
  const storedLocations: NodeLocation[] = convertedFromSingle
    ? [{ path: "/", ...(server.upstreamId ? { upstreamId: server.upstreamId } : { proxyPass: cfg.proxyPass ?? "" }),
        ...(cfg.locationDirectives !== undefined ? { directives: cfg.locationDirectives } : {}),
        ...(resolveAtRuntime ? { resolveAtRuntime: true } : {}) }]
    : cfg.locations!;
  const locations = storedLocations.map((location): LocationInput => {
    const original = storedDirectives(location.directives);
    const runtimeDns = runtimeDnsFlag(location.resolveAtRuntime);
    const retryOtherInstances=retryFlag(location.retryOtherInstances);
    const hasId = location.upstreamId !== undefined;
    if (hasId && typeof location.upstreamId !== "string") throw new Error("Upstream ID 형식이 올바르지 않습니다.");
    const id = hasId ? location.upstreamId : byUrl.get(location.proxyPass ?? "");
    const upstream = hasId || id !== undefined;
    const dangling = upstream && !byId.has(id ?? "");
    const cacheEnabled = parsePerformanceOverride(location.cacheEnabled);
    const gzipEnabled = parsePerformanceOverride(location.gzipEnabled);
    const _ipShape = location.ipAccess?.entries.join("\n");
    return {
      path: location.path,
      mode: upstream ? "upstream" : "direct",
      upstreamId: dangling ? "" : (id ?? ""),
      proxyPass: upstream ? "" : (location.proxyPass ?? ""),
      directives: directivesToState(original ?? []),
      originalDirectives: original,
      ...(retryOtherInstances?{retryOtherInstances:true}:{}),
      resolveAtRuntime: upstream ? false : runtimeDns,
      ...(dangling ? { upstreamError: MISSING_UPSTREAM_MESSAGE } : {}),
      ...(location.ipAccess != null ? { ipAccess: location.ipAccess } : {}),
      ...(cacheEnabled !== undefined ? { cacheEnabled } : {}),
      ...(gzipEnabled !== undefined ? { gzipEnabled } : {}),
    };
  });
  const originalServerDirectives = storedDirectives(cfg.serverDirectives);
  const serverDirectives = directivesToState(originalServerDirectives ?? []);
  const locationDirectives = directivesToState(storedDirectives(cfg.locationDirectives) ?? []);

  // `tlsPort`/`redirectHttp` 는 `StoredServerConfig` 에 없다(폼이 config jsonb 에
  // 같이 넣는 값이다) — 추출 전 로컬 수정 화면과 **같은 캐스트**를 그대로 쓴다.
  const cfgTls = cfg as { tlsPort?: number; redirectHttp?: boolean };

  // **여기부터가 이 함수가 「읽지 않고 그대로 폼에 넘기던」 값들이다.**
  // 노드에서 오는 jsonb 라 모양이 보장되지 않는데, 그대로 통과시키면 그 실패가
  // `loadNodePageAll` 의 try/catch **밖**, 폼 렌더 도중에 터진다 — 하드 로드에서
  // §9 카드가 아니라 빈 화면이다(`LoadNodePageAllArgs.toViewModel` 문서).
  // 그래서 **폼이 파고드는 깊이 그대로** 여기서 한 번 읽는다. 값을 바꾸지도,
  // 대체하지도 않는다 — 어긋나면 그냥 던지고, 던지는 자리가 관문 안이다.
  //
  // `tlsOptions` 는 읽을 것이 없다(측정한 것이지 넘겨짚은 것이 아니다):
  // `server-form.tsx:147-153` 의 일곱 읽기가 **전부** 옵셔널 체이닝 + `??` 라
  // 어떤 모양이 와도 던지지 않는다. 그 자리가 언젠가 `.map`/`.join` 을 쓰게
  // 되면 여기에 한 줄이 더 필요하다.
  //
  // `_` 접두사는 이 저장소의 「의도적으로 버린 값」 표시다
  // (`packages/config-eslint/base.mjs` 의 `varsIgnorePattern`).
  const _ipShape = cfg.ipAccess?.entries.join("\n"); // ← server-form.tsx:172 와 같은 호출
  const _realIpShape = cfg.realIp?.trustedRanges?.join("\n"); // ← server-form.tsx:182 와 같은 호출

  return {
    resolveAtRuntime: server.upstreamId ? false : resolveAtRuntime,
    id: server.id,
    version: server.version,
    serverName: server.serverName,
    listen: server.listen,
    upstreamId: server.upstreamId,
    proxyPass,
    locations,
    convertedFromSingle,
    serverDirectives,
    originalServerDirectives,
    locationDirectives,
    certificateId: server.certificateId,
    tlsPort: cfgTls.tlsPort ?? 443,
    redirectHttp: cfgTls.redirectHttp !== false,
    tlsOptions: cfg.tlsOptions,
    ipAccess: cfg.ipAccess,
    realIp: cfg.realIp,
    cache: parseCacheConfig(cfg.cache),
    gzip: parseGzipConfig(cfg.gzip),
  };
}
