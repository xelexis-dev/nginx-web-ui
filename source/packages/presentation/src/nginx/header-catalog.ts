/**
 * 리버스 프록시 헤더 편집기(요청 `proxy_set_header` / 응답 `add_header`)의
 * 자동완성 카탈로그. **순수 데이터 + 순수 함수**다 — `directive-state.ts` 와
 * 같은 이유로 node 의존이 하나도 없어야 클라이언트 컴포넌트가
 * `@repo/presentation/nginx/header-catalog` 서브패스로 직접 import 할 수 있다(패키지 index 는
 * apply.ts/control.ts 를 통해 node:fs·child_process 를 끌고 온다).
 *
 * 입력 보조일 뿐이라 **저장·검증 경로가 아니다.** 목록 밖의 헤더도 사용자가
 * 그대로 입력할 수 있고, 실제 관문은 지금처럼 `assertValidDirective` 하나다.
 * 다만 우리가 제안한 값이 그 관문에서 튕기면 UI 가 사용자를 함정에 빠뜨리는
 * 셈이라, `header-catalog.test.ts` 가 모든 이름·제안값을 실제 검증기에 통과시킨다.
 *
 * IANA 전체 목록(수백 개)을 담지 않는다 — 리버스 프록시에서 실제로 쓰는 것만
 * 있어야 제안 목록이 쓸모 있다.
 */

export interface HeaderSpec {
  /** 정규 표기(제안·비교 모두 이 문자열). 조회는 대소문자를 무시한다. */
  name: string;
  /** 목록에 함께 보여줄 한 줄 설명. */
  hint?: string;
  /** 값 칸 제안. 비면 카탈로그의 `valueFallback` 으로 내려간다. */
  values?: string[];
}

export interface HeaderCatalog {
  headers: readonly HeaderSpec[];
  /**
   * 헤더 이름을 모를 때(=커스텀 헤더) 값 칸에 제안할 목록.
   * 요청 헤더는 값이 거의 항상 nginx 변수라 변수 목록을 주고, 응답 헤더는
   * 값이 헤더마다 완전히 달라서 아무것도 제안하지 않는다.
   */
  valueFallback: readonly string[];
}

/** 헤더 값으로 자주 쓰는 nginx 변수 — 커스텀 요청 헤더의 값 폴백. */
export const NGINX_VALUE_VARIABLES: readonly string[] = [
  "$host",
  "$http_host",
  "$remote_addr",
  "$proxy_add_x_forwarded_for",
  "$scheme",
  "$server_port",
  "$server_name",
  "$request_uri",
  "$request_id",
  "$http_upgrade",
  "$ssl_client_verify",
  "$ssl_protocol",
];

const REQUEST_HEADERS: readonly HeaderSpec[] = [
  { name: "Host", hint: "업스트림이 보게 될 호스트명", values: ["$host", "$http_host", "$proxy_host"] },
  { name: "X-Real-IP", hint: "실제 클라이언트 IP 한 개", values: ["$remote_addr"] },
  {
    name: "X-Forwarded-For",
    hint: "프록시 체인 IP 목록 — add 변수는 덮어쓰지 않고 뒤에 붙인다",
    values: ["$proxy_add_x_forwarded_for", "$remote_addr"],
  },
  { name: "X-Forwarded-Proto", hint: "원래 요청의 http/https", values: ["$scheme"] },
  { name: "X-Forwarded-Host", hint: "원래 요청의 Host", values: ["$host", "$http_host"] },
  { name: "X-Forwarded-Port", hint: "원래 요청이 들어온 포트", values: ["$server_port"] },
  { name: "X-Forwarded-Prefix", hint: "하위 경로로 마운트할 때 앱에 알려줄 접두사" },
  { name: "Upgrade", hint: "WebSocket — Connection: upgrade 와 짝", values: ["$http_upgrade"] },
  { name: "Connection", hint: "WebSocket 이면 upgrade, keepalive 면 빈 값", values: ["upgrade", "", "close"] },
  { name: "X-Request-ID", hint: "요청 추적 ID (로그 상관관계)", values: ["$request_id"] },
  { name: "Authorization", hint: "원본 인증 헤더를 그대로 전달", values: ["$http_authorization"] },
  { name: "Accept-Encoding", hint: "빈 값이면 업스트림이 압축하지 않아 본문 가공이 쉬워진다", values: ["", "gzip"] },
  { name: "Cookie", hint: "원본 쿠키를 그대로 전달", values: ["$http_cookie"] },
  { name: "Referer", values: ["$http_referer"] },
  { name: "User-Agent", values: ["$http_user_agent"] },
  { name: "Early-Data", hint: "TLS 1.3 0-RTT 요청 표시", values: ["$ssl_early_data"] },
  { name: "X-SSL-Client-Verify", hint: "클라이언트 인증서 검증 결과", values: ["$ssl_client_verify"] },
  { name: "X-SSL-Client-DN", hint: "클라이언트 인증서 주체", values: ["$ssl_client_s_dn"] },
];

const RESPONSE_HEADERS: readonly HeaderSpec[] = [
  {
    name: "Strict-Transport-Security",
    hint: "HTTPS 강제 — TLS 탭의 HSTS 스위치와 중복 설정하지 말 것",
    values: [
      "max-age=31536000",
      "max-age=31536000; includeSubDomains",
      "max-age=63072000; includeSubDomains; preload",
    ],
  },
  { name: "X-Frame-Options", hint: "클릭재킹 방지 — iframe 삽입 허용 범위", values: ["SAMEORIGIN", "DENY"] },
  { name: "X-Content-Type-Options", hint: "MIME 스니핑 차단", values: ["nosniff"] },
  {
    name: "Referrer-Policy",
    hint: "다른 사이트로 나갈 때 Referer 를 얼마나 흘릴지",
    values: ["no-referrer", "strict-origin-when-cross-origin", "same-origin", "no-referrer-when-downgrade"],
  },
  {
    name: "Content-Security-Policy",
    hint: "리소스 로드 출처 제한 — 잘못 걸면 페이지가 깨진다",
    values: ["default-src 'self'", "frame-ancestors 'self'", "upgrade-insecure-requests"],
  },
  {
    name: "Content-Security-Policy-Report-Only",
    hint: "차단하지 않고 위반만 보고 — CSP 도입 전 시험용",
    values: ["default-src 'self'"],
  },
  {
    name: "Permissions-Policy",
    hint: "브라우저 기능 사용 제한",
    values: ["geolocation=(), microphone=(), camera=()", "interest-cohort=()"],
  },
  { name: "Access-Control-Allow-Origin", hint: "CORS 허용 출처", values: ["*", "$http_origin"] },
  {
    name: "Access-Control-Allow-Methods",
    hint: "CORS 허용 메서드",
    values: ["GET, POST, OPTIONS", "GET, POST, PUT, PATCH, DELETE, OPTIONS"],
  },
  { name: "Access-Control-Allow-Headers", hint: "CORS 허용 요청 헤더", values: ["Authorization, Content-Type"] },
  { name: "Access-Control-Allow-Credentials", hint: "쿠키 동반 CORS 허용", values: ["true"] },
  { name: "Access-Control-Expose-Headers", hint: "브라우저 스크립트가 읽게 열어줄 응답 헤더", values: ["Content-Length, Content-Range"] },
  { name: "Access-Control-Max-Age", hint: "preflight 캐시 초", values: ["600", "86400"] },
  {
    name: "Cache-Control",
    hint: "캐시 정책",
    values: ["no-store", "no-cache", "private, no-cache", "public, max-age=31536000, immutable"],
  },
  { name: "Cross-Origin-Opener-Policy", hint: "다른 출처 창과의 분리", values: ["same-origin", "same-origin-allow-popups"] },
  { name: "Cross-Origin-Embedder-Policy", hint: "교차 출처 리소스 임베드 조건", values: ["require-corp", "credentialless"] },
  { name: "Cross-Origin-Resource-Policy", hint: "이 리소스를 누가 임베드할 수 있는지", values: ["same-origin", "same-site", "cross-origin"] },
  { name: "X-XSS-Protection", hint: "구식 필터 — 현대 브라우저에선 0(끄기)이 권장", values: ["0"] },
  { name: "X-Robots-Tag", hint: "검색엔진 색인 제어", values: ["noindex, nofollow", "noindex"] },
  { name: "Vary", hint: "캐시 분기 기준", values: ["Accept-Encoding", "Origin", "Accept-Encoding, Origin"] },
];

/** 요청 헤더(`proxy_set_header`) 카탈로그. */
export const REQUEST_HEADER_CATALOG: HeaderCatalog = {
  headers: REQUEST_HEADERS,
  valueFallback: NGINX_VALUE_VARIABLES,
};

/** 응답 헤더(`add_header`) 카탈로그. */
export const RESPONSE_HEADER_CATALOG: HeaderCatalog = {
  headers: RESPONSE_HEADERS,
  valueFallback: [],
};

/** 헤더 이름으로 스펙을 찾는다. 대소문자·앞뒤 공백 무시. 없으면 undefined. */
export function lookupHeader(catalog: HeaderCatalog, name: string): HeaderSpec | undefined {
  const key = name.trim().toLowerCase();
  if (key.length === 0) return undefined;
  return catalog.headers.find((h) => h.name.toLowerCase() === key);
}

/**
 * 값 칸에 제안할 목록. 아는 헤더면 그 헤더의 값들, 모르거나 값이 없으면
 * 카탈로그의 폴백(요청=nginx 변수, 응답=없음).
 */
export function suggestedValues(catalog: HeaderCatalog, name: string): string[] {
  const spec = lookupHeader(catalog, name);
  if (spec?.values && spec.values.length > 0) return [...spec.values];
  return [...catalog.valueFallback];
}
