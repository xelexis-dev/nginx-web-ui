import type {
  CacheConfig,
  CacheInactive,
  CacheMaxSize,
  CacheValid,
  GzipConfig,
  GzipTypesPreset,
} from "./types";

/**
 * 리버스 프록시 **캐시·gzip** 의 순수 계약 — 기본값, 프리셋, 파서, 충돌 검증.
 *
 * `@repo/presentation/nginx/performance` 서브패스의 브라우저용 순수 함수다.
 * 허용된 프리셋과 범위만 받아 폼·상세 화면의 캐시·압축 설정을 일관되게 표시한다.
 *
 * 저장된 값과 nginx 적용 전의 검증은 Rust API가 담당한다. 캐시는 다른 사용자의
 * 응답을 내줄 수 있는 설정이므로 화면 검증만으로 서버 검증을 대신하지 않는다.
 */

export const CACHE_MAX_SIZES: readonly CacheMaxSize[] = ["100m", "500m", "1g", "5g"];
export const CACHE_INACTIVES: readonly CacheInactive[] = ["10m", "60m", "24h"];
export const CACHE_VALIDS: readonly CacheValid[] = ["1m", "10m", "1h", "24h"];
export const GZIP_TYPES_PRESETS: readonly GzipTypesPreset[] = ["text", "text-and-fonts"];

export const GZIP_MIN_LENGTH_MAX = 1048576;
export const GZIP_COMP_LEVEL_MIN = 1;
export const GZIP_COMP_LEVEL_MAX = 9;

export const DEFAULT_CACHE_CONFIG: Readonly<CacheConfig> = Object.freeze({
  enabled: false,
  maxSize: "1g",
  inactive: "60m",
  valid: "10m",
  useStale: false,
  lock: true,
  statusHeader: true,
});

export const DEFAULT_GZIP_CONFIG: Readonly<GzipConfig> = Object.freeze({
  enabled: false,
  types: "text",
  minLength: 1024,
  compLevel: 5,
  vary: true,
  proxied: true,
});

// ---------------------------------------------------------------------------
// parsers
// ---------------------------------------------------------------------------

/**
 * 오류 문구에 쓰는 사용자 라벨(스펙 §사용자 동작의 폼 라벨). 폼이 이 메시지를
 * 그대로 보여주므로 키만 적으면 운영자가 어느 칸인지 모른다 — 「라벨(키)」로 병기.
 */
const FIELD_LABELS: Readonly<Record<string, string>> = Object.freeze({
  enabled: "서버 기본 켜기",
  maxSize: "최대 저장 용량",
  inactive: "미사용 보관 기간",
  valid: "응답 유효기간",
  useStale: "오류 시 오래된 캐시 사용",
  lock: "동시 요청 잠금",
  statusHeader: "X-Cache-Status 응답 헤더",
  types: "압축 대상",
  minLength: "최소 응답 크기",
  compLevel: "압축 수준",
  vary: "Vary 응답 헤더",
  proxied: "프록시 요청도 압축",
});

function fieldName(key: string): string {
  const label = FIELD_LABELS[key];
  return label ? `${label}(${key})` : key;
}

function asRecord(raw: unknown, what: string): Record<string, unknown> {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    throw new Error(`${what} 설정 형식이 올바르지 않습니다.`);
  }
  return raw as Record<string, unknown>;
}

/** 필드가 없으면 기본값, 있으면 **정확히** boolean 이어야 한다("false" 같은 문자열은 거절). */
function readBoolean(obj: Record<string, unknown>, key: string, fallback: boolean, what: string): boolean {
  const v = obj[key];
  if (v === undefined) return fallback;
  if (typeof v !== "boolean") throw new Error(`${what} 설정의 ${fieldName(key)} 값은 켜기/끄기(boolean)여야 합니다.`);
  return v;
}

function readEnum<T extends string>(obj: Record<string, unknown>, key: string, allowed: readonly T[], fallback: T, what: string): T {
  const v = obj[key];
  if (v === undefined) return fallback;
  if (typeof v !== "string" || !(allowed as readonly string[]).includes(v)) {
    throw new Error(`${what} 설정의 ${fieldName(key)} 값이 허용 목록(${allowed.join(", ")}) 밖입니다.`);
  }
  return v as T;
}

function readInteger(obj: Record<string, unknown>, key: string, min: number, max: number, fallback: number, what: string): number {
  const v = obj[key];
  if (v === undefined) return fallback;
  if (typeof v !== "number" || !Number.isInteger(v) || v < min || v > max) {
    throw new Error(`${what} 설정의 ${fieldName(key)} 값은 ${min}-${max} 사이의 정수여야 합니다.`);
  }
  return v;
}

/**
 * 저장/와이어/폼에서 온 캐시 설정. `null`/`undefined` 는 「없음」. 객체면 아는
 * 필드는 검증하고(없으면 기본값) 모르는 필드는 무시한다 — 구버전 노드가 신버전
 * 필드를 만나도 저장을 거부하지 않게. 잘못된 값은 던진다(조용히 바꾸지 않는다).
 */
export function parseCacheConfig(raw: unknown): CacheConfig | undefined {
  if (raw === undefined || raw === null) return undefined;
  const obj = asRecord(raw, "캐시");
  return {
    enabled: readBoolean(obj, "enabled", DEFAULT_CACHE_CONFIG.enabled, "캐시"),
    maxSize: readEnum(obj, "maxSize", CACHE_MAX_SIZES, DEFAULT_CACHE_CONFIG.maxSize, "캐시"),
    inactive: readEnum(obj, "inactive", CACHE_INACTIVES, DEFAULT_CACHE_CONFIG.inactive, "캐시"),
    valid: readEnum(obj, "valid", CACHE_VALIDS, DEFAULT_CACHE_CONFIG.valid, "캐시"),
    useStale: readBoolean(obj, "useStale", DEFAULT_CACHE_CONFIG.useStale, "캐시"),
    lock: readBoolean(obj, "lock", DEFAULT_CACHE_CONFIG.lock, "캐시"),
    statusHeader: readBoolean(obj, "statusHeader", DEFAULT_CACHE_CONFIG.statusHeader, "캐시"),
  };
}

/** `parseCacheConfig` 와 같은 규칙의 gzip 설정 파서. */
export function parseGzipConfig(raw: unknown): GzipConfig | undefined {
  if (raw === undefined || raw === null) return undefined;
  const obj = asRecord(raw, "gzip");
  return {
    enabled: readBoolean(obj, "enabled", DEFAULT_GZIP_CONFIG.enabled, "gzip"),
    types: readEnum(obj, "types", GZIP_TYPES_PRESETS, DEFAULT_GZIP_CONFIG.types, "gzip"),
    minLength: readInteger(obj, "minLength", 0, GZIP_MIN_LENGTH_MAX, DEFAULT_GZIP_CONFIG.minLength, "gzip"),
    compLevel: readInteger(obj, "compLevel", GZIP_COMP_LEVEL_MIN, GZIP_COMP_LEVEL_MAX, DEFAULT_GZIP_CONFIG.compLevel, "gzip"),
    vary: readBoolean(obj, "vary", DEFAULT_GZIP_CONFIG.vary, "gzip"),
    proxied: readBoolean(obj, "proxied", DEFAULT_GZIP_CONFIG.proxied, "gzip"),
  };
}

/**
 * location 의 `cacheEnabled`/`gzipEnabled`. `null`/`undefined` = 서버를 따른다.
 * `true`/`false` 만 통과 — 그 외("false", 0, 1, "inherit")는 던진다.
 */
export function parsePerformanceOverride(raw: unknown): boolean | undefined {
  if (raw === undefined || raw === null) return undefined;
  if (typeof raw !== "boolean") throw new Error("경로별 캐시/압축 설정은 「서버 설정 따름/켜기/끄기」 중 하나여야 합니다.");
  return raw;
}
