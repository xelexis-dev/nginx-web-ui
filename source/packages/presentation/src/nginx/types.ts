/**
 * One nginx directive, normalized to a keyword + its argument tokens.
 * Both the structured form editors and the advanced free-form textarea
 * produce these, so render + validation have a single code path.
 * Examples: `{ name: "proxy_read_timeout", args: ["60s"] }`,
 * `{ name: "proxy_set_header", args: ["Host", "$host"] }`.
 */
export interface Directive {
  name: string;
  args: string[];
}

export interface IpAccess {
  mode: "whitelist" | "blacklist";
  entries: string[]; // 검증된 IPv4/IPv6/CIDR
}

/** Normalized canonical route; web VMs remove nullable wire overrides before editing. */
export interface LocationConfig {
  /** Opt-in HTTP retry policy. Persist/submit only when true; absent keeps nginx defaults. */
  retryOtherInstances?: boolean;
  /** Opt-in runtime DNS for a direct target; named references use their group. */
  resolveAtRuntime?: boolean;
  path: string;
  /** Canonical routes have exactly one target: a stable upstream ID or a direct URL. */
  upstreamId?: string;
  proxyPass?: string;
  /** Derived for response copies only; never persisted or submitted. */
  readonly upstreamName?: string;
  /**
   * Extra directives emitted inside this `location { }` block, after
   * `proxy_pass`. When `undefined` (legacy rows written before custom
   * directives existed), the renderer injects `proxy_set_header Host $host;`
   * for backward compatibility. An array (even empty) is rendered verbatim
   * with no implicit Host header.
   */
  directives?: Directive[];
  ipAccess?: IpAccess;
  /**
   * 이 location 의 캐시/압축 override. `undefined` = 서버 설정을 따른다,
   * `true`/`false` = 서버와 무관하게 켠다/끈다. 서버가 꺼져 있어도 location 이
   * 켤 수 있다(그 경우 캐시 존은 이 location 때문에 선언된다). 수치·대상은
   * 언제나 서버의 `cache`/`gzip` 을 공유한다 — location 은 on/off 만 낸다.
   */
  cacheEnabled?: boolean;
  gzipEnabled?: boolean;
}

/** Temporary render/validation input after an ID has been resolved to its current name. */
export interface ResolvedLocationConfig extends Omit<LocationConfig, "upstreamId" | "upstreamName" | "proxyPass"> {
  proxyPass: string;
}

/**
 * 캐시 프리셋 키. 렌더가 `proxy_cache_path`/`proxy_cache_valid` 인자로 **그대로**
 * 박는 값이라 자유 문자열이 아니라 닫힌 목록이다(목록과 파서는 `performance.ts`).
 */
export type CacheMaxSize = "100m" | "500m" | "1g" | "5g";
export type CacheInactive = "10m" | "60m" | "24h";
export type CacheValid = "1m" | "10m" | "1h" | "24h";

/**
 * 서버 단위 프록시 캐시 설정. `enabled` 가 서버 기본값이고 각 location 의
 * `cacheEnabled` 가 그것을 덮는다. 저장(jsonb)·DTO·폼·렌더가 전부
 * `parseCacheConfig` 로 같은 모양을 얻는다.
 */
export interface CacheConfig {
  enabled: boolean;
  /** `proxy_cache_path … max_size=` */
  maxSize: CacheMaxSize;
  /** `proxy_cache_path … inactive=` — 접근이 없으면 신선해도 지운다. */
  inactive: CacheInactive;
  /** `proxy_cache_valid 200 301 302 <valid>` — 업스트림 Cache-Control 이 있으면 그쪽이 우선. */
  valid: CacheValid;
  /** `proxy_cache_use_stale … updating` + `proxy_cache_background_update on` */
  useStale: boolean;
  /** `proxy_cache_lock on` */
  lock: boolean;
  /** server 레벨 `add_header X-Cache-Status $upstream_cache_status always` */
  statusHeader: boolean;
}

/** gzip 대상 MIME 프리셋 키. 실제 목록은 API 의 `control/performance.rs`. */
export type GzipTypesPreset = "text" | "text-and-fonts";

/** 서버 단위 gzip 설정. location 은 `gzipEnabled` 로 on/off 만 덮는다. */
export interface GzipConfig {
  enabled: boolean;
  types: GzipTypesPreset;
  /** `gzip_min_length` 바이트, 0..1048576 정수. Content-Length 가 있는 응답에만 적용된다(nginx 문서). */
  minLength: number;
  /** `gzip_comp_level` 1..9 정수. */
  compLevel: number;
  /** `gzip_vary on` */
  vary: boolean;
  /** `gzip_proxied any` — Via 헤더가 있는(다단 프록시 뒤) 요청도 압축. */
  proxied: boolean;
}

/** 실 IP 복원에 쓰는 real_ip_header 값 — allowlist(코드 상수). */
export type RealIpHeader = "CF-Connecting-IP" | "X-Forwarded-For" | "X-Real-IP";

/**
 * 엣지/프록시(Cloudflare 등) 뒤 배치 시 실 방문자 IP 복원 설정.
 * preset="cloudflare"는 내장 CF 대역 + CF-Connecting-IP를 쓴다(커스텀 필드 무시).
 * preset="custom"은 검증된 trustedRanges + allowlist 헤더를 쓴다.
 */
export interface RealIpConfig {
  preset: "cloudflare" | "custom";
  /** custom 전용. 검증된 IPv4/IPv6/CIDR. cloudflare일 때는 내장 상수 사용. */
  trustedRanges?: string[];
  /** custom 전용. allowlist 밖/누락이면 기본값. cloudflare는 CF-Connecting-IP 고정. */
  header?: RealIpHeader;
  /** real_ip_recursive on. 기본 off. custom에서만 의미. */
  recursive?: boolean;
}

export interface TlsTuning {
  /** Opt-in for this TLS server; omission means off. */
  http2?: boolean;
  hsts?: { enabled: boolean; maxAge: number; includeSubDomains: boolean; preload: boolean };
  sslProtocols?: "intermediate" | "modern";
  sslCiphers?: "intermediate" | "modern";
  preferServerCiphers?: boolean;
}

/**
 * TLS 서빙 설정. 사용자 입력이 아니라 applyAll이 연결된 인증서 row로부터
 * 조립하는 코드-생성 값(경로는 UUID 기반 고정 형식 — certFilePaths)이므로
 * render의 injection 표면이 아니다. ssl_* 디렉티브는 여전히 사용자 allowlist 밖.
 */
export interface TlsConfig {
  certPath: string;
  keyPath: string;
  tlsPort: number;
  redirectHttp: boolean;
  /** redirect 블록이 listen할 HTTP 포트(서버 row의 기존 listen 값). */
  httpPort: number;
  /** 서버별 TLS 튜닝(프리셋 키). 렌더는 이 키를 상수맵으로 resolve한다. */
  tuning?: TlsTuning;
}

export interface ReverseProxyConfig {
  /** Opt-in runtime DNS for the direct single target; omitted means static. */
  resolveAtRuntime?: boolean;
  serverName: string;
  listen: number;
  /**
   * Direct upstream URL (e.g. "http://127.0.0.1:3000"), used verbatim in
   * `proxy_pass`. Ignored when `upstreamName` is set.
   */
  proxyPass?: string;
  /**
   * Name of an nginx `upstream { }` block to proxy_pass to instead of a
   * direct URL (e.g. "backend" -> `proxy_pass http://backend;`).
   */
  upstreamName?: string;
  /** Render-side routes have already resolved any canonical upstream ID to a URL. */
  locations?: ResolvedLocationConfig[];
  /** Extra directives emitted directly inside `server { }` (before locations). */
  serverDirectives?: Directive[];
  /**
   * Directives for the implicit single `location /` used in single-target mode
   * (when `locations` is absent). Ignored when `locations` is set. When both
   * this and `locations` are absent, the synthesized `/` location falls back to
   * the legacy `proxy_set_header Host $host;` default.
   */
  locationDirectives?: Directive[];
  /** Present only when a linked certificate is issued — see TlsConfig. */
  tls?: TlsConfig;
  /** 서버 블록 레벨 IP 접근제어. */
  ipAccess?: IpAccess;
  /** 엣지/프록시 뒤 배치 시 실 IP 복원. 없으면 직접 노출(현행). */
  realIp?: RealIpConfig;
  /**
   * 이 서버의 DB id(UUID). 캐시 존 이름·디렉터리가 여기서 나온다
   * (`keys_zone=srv_<hex>`, `/var/cache/nginx/proxy-<hex>`) — 인스턴스 안에서
   * 유일해야 `nginx -t` 의 `duplicate zone` / `the same path name` 을 피한다.
   * 캐시가 실효 경로 하나라도 있는데 UUID 형식이 아니면 렌더가 던진다.
   */
  serverId?: string;
  /** 서버 단위 프록시 캐시. 없으면 캐시 없음(현행). */
  cache?: CacheConfig;
  /** 서버 단위 gzip. 없으면 고급 디렉티브의 legacy `gzip` 이 있는 그대로 동작한다. */
  gzip?: GzipConfig;
}

export interface StreamConfig {
  resolveAtRuntime?: boolean;
  listen: number;
  protocol: "tcp" | "udp";
  proxyPass: string; // host:port
}

export interface Upstream {
  resolveAtRuntime?: boolean;
  name: string;
  servers: string[];
}

/** A reverse-proxy server plus its DB id/enabled flag, as written/removed by writeReverseProxyFiles. */
export interface ManagedReverseProxyServer {
  id: string;
  enabled: boolean;
  config: ReverseProxyConfig;
}

/** A stream server plus its DB id/enabled flag, as written/removed by writeStreamFiles. */
export interface ManagedStream {
  id: string;
  enabled: boolean;
  config: StreamConfig;
}

export interface ValidationResult {
  ok: boolean;
  output: string;
}
