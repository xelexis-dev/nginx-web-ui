import type { NodeLocation, NodeServer } from "@repo/contracts";
import type { IpAccess, RealIpConfig, ReverseProxyConfig, TlsTuning } from "@repo/presentation/nginx";

/**
 * 리버스 프록시 화면이 로컬·노드 양쪽에서 공유하는 **뷰모델과 그 변환**.
 *
 * 값 import 없이 API 응답을 화면용 타입과 값으로 변환한다.
 * 클라이언트·서버 컴포넌트 양쪽에서 사용할 수 있다.
 *
 * **`StoredServerConfig` 의 정의가 여기로 옮겨 왔다**(원래
 * `app/(app)/reverse-proxy/types.ts`). 공유 컴포넌트가 라우트 그룹의 TS 를
 * 가져오는 것은 금지고(`components/upstreams/upstream-list.tsx` 헤더의 불변식),
 * 그렇다고 같은 타입을 두 벌 적으면 `nginx_servers.config` 의 모양이 바뀌는 날
 * 한쪽만 고쳐진다. **여기가 정본이다** — `types.ts` 가 한동안 재export 했지만
 * 그 폴더에 소비자가 남지 않아 지웠다(계획 4b Task 5 수정 라운드 1).
 */

/**
 * The subset of the canonical `ReverseProxyConfig` persisted in the
 * `nginx_servers.config` jsonb column (server_name/listen/upstreamId have their
 * own columns). Single source of truth for the `server.config` cast on both the
 * detail and edit pages, kept in lockstep with the renderer's type.
 */
export type StoredServerConfig = Partial<
  Pick<ReverseProxyConfig, "resolveAtRuntime" | "proxyPass" | "serverDirectives" | "locationDirectives" | "cache" | "gzip">
> & {
  /** Stored/read-projected routes keep IDs; only the API's temporary render copy resolves URLs. */
  locations?: NodeLocation[];
  /** TLS 튜닝(HSTS/ssl_protocols/ssl_ciphers/prefer_server_ciphers) — 인증서 연결 시에만 유효. */
  tlsOptions?: TlsTuning;
  /** 서버 전체에 적용되는 IP allow/deny (whitelist/blacklist). */
  ipAccess?: IpAccess;
  /** 엣지/프록시 뒤 배치 실 IP 복원(Cloudflare/커스텀). */
  realIp?: RealIpConfig;
};

/**
 * Fields consumed from local and remote server API projections.
 * `config` remains unknown until the presentation converter reads its fields.
 */
export interface ServerSourceRow extends Pick<NodeServer, "id" | "serverName" | "listen" | "enabled" | "upstreamId" | "version"> {
  /** Legacy local row projections may omit this field. */
  certificateId?: NodeServer["certificateId"];
  config: unknown;
}

/** jsonb 컬럼 → `StoredServerConfig`. 객체가 아니면 빈 설정으로 본다(추출 전과 같은 규칙). */
export function storedConfig(raw: unknown): StoredServerConfig {
  return (raw && typeof raw === "object" ? raw : {}) as StoredServerConfig;
}

/**
 * 「대상」 열의 한 줄. `path` 가 `null` 이면 location 을 따로 두지 않은 서버(렌더가 만드는 암묵
 * `location /`)이고, 문자열이면 저장된 location 하나다.
 */
export interface ServerTargetVM {
  path: string | null;
  label: string;
  missingUpstream?: boolean;
}

/** 서버 표 한 줄. 로컬은 DB 행에서, 노드는 `/api/v1/servers` 응답에서 같은 모양을 만든다. */
export interface ServerRowVM {
  id: string;
  serverName: string;
  listen: number;
  enabled: boolean;
  /**
   * 대상을 한 줄로 — 검색어와 `title`, 한 줄뿐인 개요 표가 쓴다. location 이 있으면
   * `경로 → 대상` 을 ` · ` 로 이은 요약이고, 없으면 예전과 같은 라벨이다.
   */
  targetLabel: string;
  /** 대상 칸이 줄마다 그리는 것. 비어 있지 않다. */
  targets: ServerTargetVM[];
}

/**
 * 「대상」 열의 라벨. 로컬 목록과 노드 목록이 **같은 함수**를 쓴다 — 여기서
 * 갈리면 같은 설정이 게이트웨이와 노드에서 다르게 읽힌다.
 */
function targetLabel(upstreamId: string | null, proxyPass: string | null, upstreamNameById: Map<string, string>): string {
  if (upstreamId) return `upstream: ${upstreamNameById.get(upstreamId) ?? upstreamId}`;
  if (proxyPass) return `proxy_pass: ${proxyPass}`;
  return "—";
}

/**
 * location 하나의 라벨. upstream 을 고른 location 은 `http://<이름>` 으로 저장되므로 그 문자열이
 * **정확히** 같은 출처의 upstream 이름일 때만 upstream 으로 읽는다(`toServerFormValues` 와 같은 규칙).
 */
function locationLabel(proxyPass: unknown, upstreamNames: Set<string>): string {
  if (typeof proxyPass !== "string" || !proxyPass) return "—";
  const name = proxyPass.startsWith("http://") ? proxyPass.slice("http://".length) : null;
  return name !== null && upstreamNames.has(name) ? `upstream: ${name}` : `proxy_pass: ${proxyPass}`;
}

/** The projected name is authoritative for ID targets; never fall back to a stale URL or another scope's list. */
export function locationTarget(location: unknown, upstreamNames: Set<string>): Omit<ServerTargetVM, "path"> {
  const value = (location && typeof location === "object" ? location : {}) as { upstreamId?: unknown; upstreamName?: unknown; proxyPass?: unknown };
  if (value.upstreamId !== undefined) {
    return typeof value.upstreamId === "string" && value.upstreamId && typeof value.upstreamName === "string" && value.upstreamName
      ? { label: `upstream: ${value.upstreamName}` }
      : { label: "Upstream 없음", missingUpstream: true };
  }
  return { label: locationLabel(value.proxyPass, upstreamNames) };
}

/**
 * 서버 행 → 표 한 줄. `upstreamNameById` 는 **같은 출처의** upstream 목록에서
 * 만든 것이어야 한다 — 노드 화면이 게이트웨이 자신의 upstream 이름을 섞으면
 * 이름이 엉뚱하게 붙거나 id 가 그대로 보인다.
 *
 * 렌더(`apps/api/src/control/render.rs`)와 같은 순서로 읽는다: `locations` 가 비어 있지 않으면
 * location 마다 제 proxy_pass 를 쓰고 서버의 upstream·proxy_pass 는 쓰이지 않는다.
 */
export function toServerRow(row: ServerSourceRow, upstreamNameById: Map<string, string>): ServerRowVM {
  const cfg = storedConfig(row.config);
  const base = { id: row.id, serverName: row.serverName, listen: row.listen, enabled: row.enabled };
  if (Array.isArray(cfg.locations) && cfg.locations.length > 0) {
    const upstreamNames = new Set(upstreamNameById.values());
    const targets = cfg.locations.map((location: unknown) => {
      const l = (location && typeof location === "object" ? location : {}) as { path?: unknown; proxyPass?: unknown };
      return { path: typeof l.path === "string" ? l.path : "", ...locationTarget(l, upstreamNames) };
    });
    return { ...base, targets, targetLabel: targets.map((t) => `${t.path} → ${t.label}`).join(" · ") };
  }
  // upstream 이 연결돼 있으면 proxy_pass 는 렌더에 쓰이지 않는다 — 라벨도 같은 규칙.
  const proxyPass = row.upstreamId ? null : (cfg.proxyPass ?? null);
  const label = targetLabel(row.upstreamId, proxyPass, upstreamNameById);
  return { ...base, targetLabel: label, targets: [{ path: null, label }] };
}
