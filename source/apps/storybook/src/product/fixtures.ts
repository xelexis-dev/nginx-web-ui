// Sample values for the product stories, typed with the production view models so a change to what
// the app renders breaks these instead of drifting. Addresses are *.example.test. These are sample
// values for Storybook only: the app itself reads everything from the API.
import type { OverviewVM } from "@/lib/overview-data";
import type { ServerOpsRowVM } from "@/components/reverse-proxy/server-status";
import type { ServerListVM } from "@/components/reverse-proxy/server-list";
import type { ServerDetailVM } from "@/components/reverse-proxy/server-detail";
import type { CertificateOption, ServerFormValues } from "@/components/reverse-proxy/server-form-vm";
import { emptyDirectiveState, seededLocationDirectiveState } from "@repo/presentation/nginx/directive-state";

const ok = { tone: "success", label: "활성", reason: null } as const;

export const SERVER_ROWS: ServerOpsRowVM[] = [
  {
    id: "srv-files",
    serverName: "files.example.test",
    listen: 80,
    enabled: true,
    targetLabel: "proxy_pass: http://files.internal.example.test:9100",
    targets: [{ path: null, label: "proxy_pass: http://files.internal.example.test:9100" }],
    certificateId: "cert-gone",
    tls: { label: "인증서 삭제됨", tone: "error", problem: "인증서 삭제됨", unknown: false },
    health: { tone: "error", label: "확인 필요", reason: "인증서 삭제됨" },
    pending: false,
  },
  {
    id: "srv-shop",
    serverName: "shop.example.test",
    listen: 80,
    enabled: true,
    targetLabel: "upstream: app_pool",
    targets: [{ path: null, label: "upstream: app_pool" }],
    certificateId: "cert-shop",
    tls: { label: "만료 6일 전", tone: "warning", problem: "인증서 만료 6일 전", unknown: false },
    health: { tone: "warning", label: "주의", reason: "인증서 만료 6일 전" },
    pending: true,
  },
  {
    id: "srv-api",
    serverName: "api.example.test www.api.example.test",
    listen: 80,
    enabled: true,
    targetLabel: "upstream: api_pool",
    targets: [{ path: null, label: "upstream: api_pool" }],
    certificateId: "cert-api",
    tls: { label: "HTTPS · 61일", tone: "success", problem: null, unknown: false },
    health: ok,
    pending: false,
  },
  {
    id: "srv-portal",
    serverName: "very-long-subdomain-for-layout-testing.customer-portal.internal.example.test",
    listen: 8081,
    enabled: true,
    targetLabel: "proxy_pass: http://10.0.6.20:8081",
    targets: [{ path: null, label: "proxy_pass: http://10.0.6.20:8081" }],
    certificateId: null,
    tls: { label: "HTTP", tone: "neutral", problem: null, unknown: false },
    health: ok,
    pending: false,
  },
  {
    id: "srv-legacy",
    serverName: "legacy.example.test",
    listen: 80,
    enabled: false,
    targetLabel: "proxy_pass: http://legacy.internal.example.test:8000",
    targets: [{ path: null, label: "proxy_pass: http://legacy.internal.example.test:8000" }],
    certificateId: null,
    tls: { label: "HTTP", tone: "neutral", problem: null, unknown: false },
    health: { tone: "neutral", label: "비활성", reason: null },
    pending: true,
  },
];

/** A server with locations: the 대상 column lists each path with its own target (not a dash). */
const GATEWAY_ROW: ServerOpsRowVM = {
  id: "srv-gateway",
  serverName: "gateway.example.test",
  listen: 80,
  enabled: true,
  targetLabel: "/ → upstream: app_pool · /api → upstream: api_pool · /static → proxy_pass: http://static-assets.internal.example.test:9100/public/ · /ws → proxy_pass: http://10.0.6.30:8088",
  targets: [
    { path: "/", label: "upstream: app_pool" },
    { path: "/api", label: "upstream: api_pool" },
    { path: "/static", label: "proxy_pass: http://static-assets.internal.example.test:9100/public/" },
    { path: "/ws", label: "proxy_pass: http://10.0.6.30:8088" },
  ],
  certificateId: null,
  tls: { label: "HTTP", tone: "neutral", problem: null, unknown: false },
  health: ok,
  pending: false,
};

export const SERVER_LIST: ServerListVM = {
  items: SERVER_ROWS,
  certificatesUnknown: false,
  canApply: true,
  pendingCount: 2,
  pendingHasBaseline: true,
  pendingDiff: {
    servers: [
      { key: "srv-shop", label: "shop.example.test:80", status: "changed", changes: [] },
      { key: "srv-legacy", label: "legacy.example.test:80", status: "changed", changes: [] },
    ],
    streams: [],
    upstreams: [],
    hasChanges: true,
  },
};

/** The list with a location-based server among the single-target ones. */
export const SERVER_LIST_LOCATIONS: ServerListVM = {
  ...SERVER_LIST,
  items: [...SERVER_ROWS.slice(0, 3), GATEWAY_ROW, ...SERVER_ROWS.slice(3)],
};

/** The same rows when the certificate read failed: TLS is 확인 불가, never 활성. */
export const SERVER_LIST_CERTS_UNKNOWN: ServerListVM = {
  ...SERVER_LIST,
  certificatesUnknown: true,
  items: SERVER_ROWS.map((row) =>
    row.certificateId
      ? { ...row, tls: { label: "확인 불가", tone: "neutral", problem: null, unknown: true }, health: { tone: "neutral", label: "확인 불가", reason: "TLS 확인 불가" } }
      : row,
  ),
};

export const OVERVIEW: OverviewVM = {
  email: "editor@example.test",
  counts: { serverTotal: 5, serverActive: 4, streamTotal: 3, streamActive: 3, audit24h: 42 },
  gauge: { active: 1284, accepts: 18_420_900, handled: 18_420_900, requests: 18_420_771, reading: 3, writing: 41, waiting: 1240 },
  servers: {
    ok: true,
    data: {
      rows: SERVER_ROWS,
      total: SERVER_ROWS.length,
      attention: 2,
      pendingCount: 2,
      pendingHasBaseline: true,
      pendingItems: [
        { key: "srv-shop", label: "shop.example.test:80", status: "changed" },
        { key: "srv-legacy", label: "legacy.example.test:80", status: "changed" },
      ],
      canApply: true,
      canWrite: true,
      certificatesUnknown: false,
    },
  },
  certificates: { ok: true, data: { total: 6, expiring: 1, failed: 0 } },
  nodes: {
    ok: true,
    data: [
      { id: "node-a", name: "gateway-a.example.test", tone: "success", status: "응답함", version: "1.18.0" },
      { id: "node-c", name: "edge-c.example.test", tone: "error", status: "응답 없음", version: "1.17.2" },
    ],
  },
  events: {
    ok: true,
    data: [
      { id: "e1", time: "2026. 9. 27. 오전 10:38:02", action: "server.update", target: "legacy.example.test", actor: "ops@example.test" },
      { id: "e2", time: "2026. 9. 27. 오전 10:31:47", action: "server.update", target: "shop.example.test", actor: "ops@example.test" },
      { id: "e3", time: "2026. 9. 27. 오전 10:02:10", action: "config.apply", target: null, actor: "ops@example.test" },
    ],
  },
  lastApply: { ok: true, data: { time: "2026. 9. 27. 오전 10:02:10", label: "apply", actor: "ops@example.test" } },
};

/** A viewer: config:read only, so the same panels (nodes included) without write or apply. */
export const OVERVIEW_VIEWER: OverviewVM = {
  ...OVERVIEW,
  email: "viewer@example.test",
  servers: OVERVIEW.servers.ok ? { ok: true, data: { ...OVERVIEW.servers.data, canWrite: false, canApply: false } } : OVERVIEW.servers,
};

/**
 * Secondary reads failing on their own: certificates, nodes and events each show their reason, and the
 * rest still renders. (A forbidden read hides its panel instead; that branch is covered by the unit
 * tests, lib/overview-data.test.ts.)
 */
export const OVERVIEW_DEGRADED: OverviewVM = {
  ...OVERVIEW,
  gauge: null,
  certificates: { ok: false, reason: "error", message: "API 서버에 연결하지 못했습니다." },
  nodes: { ok: false, reason: "error", message: "API 서버에 연결하지 못했습니다." },
  events: { ok: false, reason: "error", message: "API 서버에 연결하지 못했습니다." },
  servers: OVERVIEW.servers.ok ? { ok: true, data: { ...OVERVIEW.servers.data, certificatesUnknown: true } } : OVERVIEW.servers,
};

export const SERVER_DETAIL: ServerDetailVM = {
  id: "srv-api",
  serverName: "api.example.test www.api.example.test",
  listen: 80,
  enabled: true,
  config: {
    proxyPass: undefined,
    locations: [
      { path: "/", proxyPass: "http://api_pool", directives: [], cacheEnabled: undefined, gzipEnabled: true },
      { path: "/static/", proxyPass: "http://10.0.5.10:8080", directives: [], cacheEnabled: true, gzipEnabled: undefined },
    ],
  } as ServerDetailVM["config"],
  upstreamId: "up-api",
  certificateId: "cert-api",
  upstreams: [{ id: "up-api", name: "api_pool", servers: ["10.0.1.10:3000", "10.0.1.11:3000"] }],
  cert: { id: "cert-api", domains: ["api.example.test", "www.api.example.test"], staging: false, issued: true, tlsReady: true },
  selfProxyNotice: false,
  preview: {
    config:
      "server {\n    listen 443 ssl;\n    server_name api.example.test www.api.example.test;\n    ssl_certificate /var/lib/nginx-web-ui/certs/cert-api/fullchain.pem;\n    ssl_certificate_key /var/lib/nginx-web-ui/certs/cert-api/privkey.pem;\n    location / {\n        proxy_pass http://api_pool;\n        gzip on;\n    }\n    location /static/ {\n        proxy_pass http://10.0.5.10:8080;\n        proxy_cache nwui_cache;\n    }\n}\n",
    error: null,
  },
};

export const UPSTREAMS = [
  { id: "up-api", name: "api_pool" },
  { id: "up-app", name: "app_pool" },
];

export const CERTIFICATES: CertificateOption[] = [
  { id: "cert-api", label: "api.example.test", staging: false, domains: ["api.example.test", "www.api.example.test"] },
  { id: "cert-shop", label: "shop.example.test", staging: false, domains: ["shop.example.test"] },
];

export const FORM_VALUES: ServerFormValues = {
  id: "srv-api",
  serverName: "api.example.test www.api.example.test",
  listen: 80,
  upstreamId: "up-api",
  proxyPass: null,
  serverDirectives: emptyDirectiveState(),
  locationDirectives: seededLocationDirectiveState(),
  certificateId: "cert-api",
  tlsPort: 443,
  redirectHttp: true,
};

export const PREVIEW_UNRESOLVED_PATHS = ["/api", "/long-" + "x".repeat(300)];
