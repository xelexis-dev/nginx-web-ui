import type { ReactNode } from "react";
import Link from "next/link";
import { apiRead, readView } from "./api-client";
import { HistoryTime } from "@/components/history/history-time";

type Name =
  | "server"
  | "upstream"
  | "stream"
  | "certificate"
  | "history"
  | "audit"
  | "provider"
  | "webhook"
  | "mail"
  | "user"
  | "token"
  | "passkey";
interface BreadcrumbRoute {
  items: readonly (readonly [string, string | null])[];
  name?: Name;
  source?: string;
  action?: string;
  query?: boolean;
}
export interface BreadcrumbPageProps {
  params?: Promise<Record<string, string>>;
  searchParams?: Promise<Record<string, string | string[] | undefined>>;
}
export const breadcrumbRoutes = {
  "/account/password": {
    items: [
      ["설정", null],
      ["계정", "/account"],
    ],
    action: "비밀번호 변경",
  },
  "/account/2fa/backup-codes/new": {
    items: [
      ["설정", null],
      ["계정", "/account"],
      ["2단계 인증", "/account/2fa"],
    ],
    action: "백업 코드 재발급",
  },
  "/account/2fa/disable": {
    items: [
      ["설정", null],
      ["계정", "/account"],
      ["2단계 인증", "/account/2fa"],
    ],
    action: "해제",
  },
  "/account/2fa": {
    items: [
      ["설정", null],
      ["계정", "/account"],
      ["2단계 인증", "/account/2fa"],
    ],
  },
  "/account": {
    items: [
      ["설정", null],
      ["계정", "/account"],
    ],
  },
  "/account/passkeys/[id]/edit": {
    items: [
      ["설정", null],
      ["계정", "/account"],
      ["패스키", "/account"],
    ],
    name: "passkey",
    source: "/api/ui/v1/account/passkeys/[id]",
    action: "수정",
  },
  "/account/passkeys/[id]": {
    items: [
      ["설정", null],
      ["계정", "/account"],
      ["패스키", "/account"],
    ],
    name: "passkey",
    source: "/api/ui/v1/account/passkeys/[id]",
    action: "상세",
  },
  "/account/passkeys/new": {
    items: [
      ["설정", null],
      ["계정", "/account"],
      ["패스키", "/account"],
    ],
    action: "새로 만들기",
  },
  "/account/tokens/[id]": {
    items: [
      ["설정", null],
      ["계정", "/account"],
      ["액세스 토큰", "/account/tokens"],
    ],
    name: "token",
    source: "/api/ui/v1/account/tokens/[id]",
    action: "상세",
  },
  "/account/tokens/new": {
    items: [
      ["설정", null],
      ["계정", "/account"],
      ["액세스 토큰", "/account/tokens"],
    ],
    action: "새로 만들기",
  },
  "/account/tokens": {
    items: [
      ["설정", null],
      ["계정", "/account"],
      ["액세스 토큰", "/account/tokens"],
    ],
  },
  "/audit/[id]": {
    items: [
      ["운영", null],
      ["감사 로그", "/audit"],
    ],
    name: "audit",
    source: "/audit/[id]",
    action: "상세",
  },
  "/audit": {
    items: [
      ["운영", null],
      ["감사 로그", "/audit"],
    ],
  },
  "/certificates/[id]/edit": {
    items: [
      ["구성", null],
      ["인증서", "/certificates"],
    ],
    name: "certificate",
    source: "/certificates/[id]/edit",
    action: "수정",
  },
  "/certificates/[id]": {
    items: [
      ["구성", null],
      ["인증서", "/certificates"],
    ],
    name: "certificate",
    source: "/certificates/[id]",
    action: "상세",
    query: true,
  },
  "/certificates/[id]/replace": {
    items: [
      ["구성", null],
      ["인증서", "/certificates"],
    ],
    action: "교체",
  },
  "/certificates/import": {
    items: [
      ["구성", null],
      ["인증서", "/certificates"],
    ],
    action: "가져오기",
  },
  "/certificates/new": {
    items: [
      ["구성", null],
      ["인증서", "/certificates"],
    ],
    action: "새로 만들기",
  },
  "/certificates": {
    items: [
      ["구성", null],
      ["인증서", "/certificates"],
    ],
  },
  "/history/[id]": {
    items: [
      ["운영", null],
      ["적용 이력", "/history"],
    ],
    name: "history",
    source: "/history/[id]",
    action: "상세",
  },
  "/history": {
    items: [
      ["운영", null],
      ["적용 이력", "/history"],
    ],
  },
  "/logs": {
    items: [
      ["모니터링", null],
      ["로그", "/logs"],
    ],
  },
  "/metrics": {
    items: [
      ["모니터링", null],
      ["접속 지표", "/metrics"],
    ],
  },
  "/nodes/new": {
    items: [
      ["운영", null],
      ["노드", "/nodes"],
    ],
    action: "새로 만들기",
  },
  "/nodes": {
    items: [
      ["운영", null],
      ["노드", "/nodes"],
    ],
  },
  "/": {
    items: [
      ["모니터링", null],
      ["개요", "/"],
    ],
  },
  "/reverse-proxy/[id]/edit": {
    items: [
      ["구성", null],
      ["리버스 프록시", "/reverse-proxy"],
    ],
    name: "server",
    source: "/reverse-proxy/[id]/edit",
    action: "수정",
  },
  "/reverse-proxy/[id]": {
    items: [
      ["구성", null],
      ["리버스 프록시", "/reverse-proxy"],
    ],
    name: "server",
    source: "/reverse-proxy/[id]",
    action: "상세",
  },
  "/reverse-proxy/new": {
    items: [
      ["구성", null],
      ["리버스 프록시", "/reverse-proxy"],
    ],
    action: "새로 만들기",
  },
  "/reverse-proxy": {
    items: [
      ["구성", null],
      ["리버스 프록시", "/reverse-proxy"],
    ],
  },
  "/reverse-proxy/templates": {
    items: [
      ["구성", null],
      ["리버스 프록시", "/reverse-proxy"],
      ["템플릿", "/reverse-proxy/templates"],
    ],
  },
  "/reverse-proxy/upstreams/[id]/edit": {
    items: [
      ["구성", null],
      ["리버스 프록시", "/reverse-proxy"],
      ["Upstream", "/reverse-proxy/upstreams"],
    ],
    name: "upstream",
    source: "/reverse-proxy/upstreams/[id]/edit",
    action: "수정",
  },
  "/reverse-proxy/upstreams/[id]": {
    items: [
      ["구성", null],
      ["리버스 프록시", "/reverse-proxy"],
      ["Upstream", "/reverse-proxy/upstreams"],
    ],
    name: "upstream",
    source: "/reverse-proxy/upstreams/[id]",
    action: "상세",
  },
  "/reverse-proxy/upstreams/new": {
    items: [
      ["구성", null],
      ["리버스 프록시", "/reverse-proxy"],
      ["Upstream", "/reverse-proxy/upstreams"],
    ],
    action: "새로 만들기",
  },
  "/reverse-proxy/upstreams": {
    items: [
      ["구성", null],
      ["리버스 프록시", "/reverse-proxy"],
      ["Upstream", "/reverse-proxy/upstreams"],
    ],
  },
  "/stream/[id]/edit": {
    items: [
      ["구성", null],
      ["Stream", "/stream"],
    ],
    name: "stream",
    source: "/stream/[id]/edit",
    action: "수정",
  },
  "/stream/[id]": {
    items: [
      ["구성", null],
      ["Stream", "/stream"],
    ],
    name: "stream",
    source: "/stream/[id]",
    action: "상세",
  },
  "/stream/new": {
    items: [
      ["구성", null],
      ["Stream", "/stream"],
    ],
    action: "새로 만들기",
  },
  "/stream": {
    items: [
      ["구성", null],
      ["Stream", "/stream"],
    ],
  },
  "/admin/notifications/mail/[id]/edit": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["알림 메일", "/admin/notifications/mail"],
    ],
    name: "mail",
    source: "/api/ui/v1/admin/notifications/mail/[id]",
    action: "수정",
  },
  "/admin/notifications/mail/[id]": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["알림 메일", "/admin/notifications/mail"],
    ],
    name: "mail",
    source: "/api/ui/v1/admin/notifications/mail/[id]",
    action: "상세",
  },
  "/admin/notifications/mail/new": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["알림 메일", "/admin/notifications/mail"],
    ],
    action: "새로 만들기",
  },
  "/admin/notifications/mail": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["알림 메일", "/admin/notifications/mail"],
    ],
  },
  "/admin/notifications/webhooks/[id]/edit": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["Webhook", "/admin/notifications/webhooks"],
    ],
    name: "webhook",
    source: "/api/ui/v1/admin/notifications/webhooks/[id]",
    action: "수정",
  },
  "/admin/notifications/webhooks/[id]": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["Webhook", "/admin/notifications/webhooks"],
    ],
    name: "webhook",
    source: "/api/ui/v1/admin/notifications/webhooks/[id]",
    action: "상세",
  },
  "/admin/notifications/webhooks/new": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["Webhook", "/admin/notifications/webhooks"],
    ],
    action: "새로 만들기",
  },
  "/admin/notifications/webhooks": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["Webhook", "/admin/notifications/webhooks"],
    ],
  },
  "/admin/oauth/[id]/edit": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["OAuth/OIDC", "/admin/oauth"],
    ],
    name: "provider",
    source: "/api/ui/v1/admin/oauth/[id]",
    action: "수정",
  },
  "/admin/oauth/[id]": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["OAuth/OIDC", "/admin/oauth"],
    ],
    name: "provider",
    source: "/api/ui/v1/admin/oauth/[id]",
    action: "상세",
  },
  "/admin/oauth/new": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["OAuth/OIDC", "/admin/oauth"],
    ],
    action: "새로 만들기",
  },
  "/admin/oauth": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["OAuth/OIDC", "/admin/oauth"],
    ],
  },
  "/admin": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
    ],
  },
  "/admin/pending": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["승인 대기", "/admin/pending"],
    ],
  },
  "/admin/secrets": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["저장된 비밀값", "/admin/secrets"],
    ],
  },
  "/admin/security": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["보안", "/admin/security"],
    ],
  },
  "/admin/settings/general": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["일반 설정", "/admin/settings/general"],
    ],
  },
  "/admin/settings/smtp": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["SMTP 설정", "/admin/settings/smtp"],
    ],
  },
  "/admin/smtp": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["SMTP 설정", "/admin/smtp"],
    ],
  },
  "/admin/tokens/[id]": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["서버 액세스 토큰", "/admin/tokens"],
    ],
    name: "token",
    source: "/api/ui/v1/admin/tokens/[id]",
    action: "상세",
  },
  "/admin/tokens/new": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["서버 액세스 토큰", "/admin/tokens"],
    ],
    action: "새로 만들기",
  },
  "/admin/tokens": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["서버 액세스 토큰", "/admin/tokens"],
    ],
  },
  "/admin/users/[id]/edit": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["사용자", "/admin/users"],
    ],
    name: "user",
    source: "/api/ui/v1/admin/users/[id]",
    action: "수정",
  },
  "/admin/users/[id]": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["사용자", "/admin/users"],
    ],
    name: "user",
    source: "/api/ui/v1/admin/users/[id]",
    action: "상세",
  },
  "/admin/users/new": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["사용자", "/admin/users"],
    ],
    action: "새로 만들기",
  },
  "/admin/users": {
    items: [
      ["설정", null],
      ["관리자", "/admin/settings/general"],
      ["사용자", "/admin/users"],
    ],
  },
  "/nodes/[id]/audit/[logId]": {
    items: [["감사 로그", "/nodes/[id]/audit"]],
    name: "audit",
    source: "/nodes/[id]/audit/[logId]",
    action: "상세",
  },
  "/nodes/[id]/audit": {
    items: [["감사 로그", "/nodes/[id]/audit"]],
  },
  "/nodes/[id]/certificates/[cid]/edit": {
    items: [["인증서", "/nodes/[id]/certificates"]],
    name: "certificate",
    source: "/nodes/[id]/certificates/[cid]/edit",
    action: "수정",
  },
  "/nodes/[id]/certificates/[cid]": {
    items: [["인증서", "/nodes/[id]/certificates"]],
    name: "certificate",
    source: "/nodes/[id]/certificates/[cid]",
    action: "상세",
  },
  "/nodes/[id]/certificates/[cid]/replace": {
    items: [["인증서", "/nodes/[id]/certificates"]],
    action: "교체",
  },
  "/nodes/[id]/certificates/import": {
    items: [["인증서", "/nodes/[id]/certificates"]],
    action: "가져오기",
  },
  "/nodes/[id]/certificates/new": {
    items: [["인증서", "/nodes/[id]/certificates"]],
    action: "새로 만들기",
  },
  "/nodes/[id]/certificates": {
    items: [["인증서", "/nodes/[id]/certificates"]],
  },
  "/nodes/[id]/edit": {
    items: [],
    action: "수정",
  },
  "/nodes/[id]/history/[sid]": {
    items: [["적용 이력", "/nodes/[id]/history"]],
    name: "history",
    source: "/nodes/[id]/history/[sid]",
    action: "상세",
  },
  "/nodes/[id]/history": {
    items: [["적용 이력", "/nodes/[id]/history"]],
  },
  "/nodes/[id]/logs": {
    items: [["로그", "/nodes/[id]/logs"]],
  },
  "/nodes/[id]/metrics": {
    items: [["접속 지표", "/nodes/[id]/metrics"]],
  },
  "/nodes/[id]": {
    items: [],
  },
  "/nodes/[id]/reverse-proxy/[sid]/edit": {
    items: [["리버스 프록시", "/nodes/[id]/reverse-proxy"]],
    name: "server",
    source: "/nodes/[id]/reverse-proxy/[sid]/edit",
    action: "수정",
  },
  "/nodes/[id]/reverse-proxy/[sid]": {
    items: [["리버스 프록시", "/nodes/[id]/reverse-proxy"]],
    name: "server",
    source: "/nodes/[id]/reverse-proxy/[sid]",
    action: "상세",
  },
  "/nodes/[id]/reverse-proxy/new": {
    items: [["리버스 프록시", "/nodes/[id]/reverse-proxy"]],
    action: "새로 만들기",
  },
  "/nodes/[id]/reverse-proxy": {
    items: [["리버스 프록시", "/nodes/[id]/reverse-proxy"]],
  },
  "/nodes/[id]/reverse-proxy/upstreams/[upstreamId]/edit": {
    items: [
      ["리버스 프록시", "/nodes/[id]/reverse-proxy"],
      ["Upstream", "/nodes/[id]/reverse-proxy/upstreams"],
    ],
    name: "upstream",
    source: "/nodes/[id]/reverse-proxy/upstreams/[upstreamId]/edit",
    action: "수정",
  },
  "/nodes/[id]/reverse-proxy/upstreams/[upstreamId]": {
    items: [
      ["리버스 프록시", "/nodes/[id]/reverse-proxy"],
      ["Upstream", "/nodes/[id]/reverse-proxy/upstreams"],
    ],
    name: "upstream",
    source: "/nodes/[id]/reverse-proxy/upstreams/[upstreamId]",
    action: "상세",
  },
  "/nodes/[id]/reverse-proxy/upstreams/new": {
    items: [
      ["리버스 프록시", "/nodes/[id]/reverse-proxy"],
      ["Upstream", "/nodes/[id]/reverse-proxy/upstreams"],
    ],
    action: "새로 만들기",
  },
  "/nodes/[id]/reverse-proxy/upstreams": {
    items: [
      ["리버스 프록시", "/nodes/[id]/reverse-proxy"],
      ["Upstream", "/nodes/[id]/reverse-proxy/upstreams"],
    ],
  },
  "/nodes/[id]/stream/[streamId]/edit": {
    items: [["Stream", "/nodes/[id]/stream"]],
    name: "stream",
    source: "/nodes/[id]/stream/[streamId]/edit",
    action: "수정",
  },
  "/nodes/[id]/stream/[streamId]": {
    items: [["Stream", "/nodes/[id]/stream"]],
    name: "stream",
    source: "/nodes/[id]/stream/[streamId]",
    action: "상세",
  },
  "/nodes/[id]/stream/new": {
    items: [["Stream", "/nodes/[id]/stream"]],
    action: "새로 만들기",
  },
  "/nodes/[id]/stream": {
    items: [["Stream", "/nodes/[id]/stream"]],
  },
} as const satisfies Record<string, BreadcrumbRoute>;

/** Read only the fields the page already shows; never stringify an unknown API object. */
function field(value: unknown, path: string): unknown {
  for (const key of path.split(".")) {
    if (value === null || typeof value !== "object") return undefined;
    value = (value as Record<string, unknown>)[key];
  }
  return value;
}
function text(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}
function nameFrom(kind: Name, value: unknown): ReactNode {
  switch (kind) {
    case "server":
      return text(
        field(value, "server.serverName") ??
          field(value, "server.server.serverName"),
      )
        ?.trim()
        .split(/\s+/)[0];
    case "upstream":
      return text(field(value, "upstream.name"));
    case "stream": {
      const listen = field(value, "stream.listen");
      return typeof listen === "number" ? `listen ${listen}` : undefined;
    }
    case "certificate":
      return [
        "data.alias",
        "certificate.alias",
        "certificate.certificate.alias",
        "data.domains.primary",
        "certificate.domains.0",
        "certificate.certificate.domains.0",
      ]
        .map((path) => text(field(value, path)))
        .find(Boolean);
    case "history": {
      const label = text(field(value, "snapshot.label"));
      const iso = text(field(value, "iso"));
      return (
        label ??
        (iso && Number.isFinite(Date.parse(iso)) ? (
          <HistoryTime iso={iso} />
        ) : undefined)
      );
    }
    case "audit":
      return text(field(value, "log.action"));
    case "mail":
      return text(field(value, "label")) ?? text(field(value, "address"));
    case "user":
      return text(field(value, "email"));
    case "passkey":
      return text(field(value, "passkey.name"));
    default:
      return text(field(value, "name"));
  }
}
function expand(path: string, params: Record<string, string>): string {
  return path.replace(/\[([^\]]+)\]/g, (_, key: string) =>
    encodeURIComponent(params[key] ?? ""),
  );
}

/** The slot has no navigation policy: failed reads leave all redirects/errors with the page. */
export async function renderBreadcrumbs(
  pattern: keyof typeof breadcrumbRoutes,
  props: BreadcrumbPageProps = {},
) {
  const route: BreadcrumbRoute = breadcrumbRoutes[pattern];
  const params = (await props.params) ?? {};
  const node = pattern.startsWith("/nodes/[id]");
  const items: { label: ReactNode; href?: string }[] = [];
  if (node) {
    items.push({ label: "운영" }, { label: "노드", href: "/nodes" });
    try {
      const value = await readView<{ node: { name: string } }>(
        `/nodes/${params.id}/layout`,
      );
      const name = text(value.node.name);
      if (name) items.push({ label: name, href: `/nodes/${params.id}` });
    } catch {
      /* The node layout owns its auth and error handling. */
    }
  }
  items.push(
    ...route.items.map(([label, href]) => ({
      label,
      href: href ? expand(href, params) : undefined,
    })),
  );
  let resource: ReactNode;
  if (route.source && route.name) {
    try {
      const path = expand(route.source, params);
      const value = path.startsWith("/api/")
        ? await apiRead<unknown>(path)
        : await readView<unknown>(
            path,
            route.query ? await props.searchParams : undefined,
          );
      resource = nameFrom(
        route.name,
        node
          ? field(value, "ok") === true
            ? field(value, "data")
            : undefined
          : value,
      );
    } catch {
      /* The recognized page still renders its own error, redirect or 404. */
    }
    if (resource)
      items.push({
        label: resource,
        href: expand(pattern.replace(/\/edit$/, ""), params),
      });
  }
  if (route.action && (!route.name || resource || pattern === "/nodes/[id]/history/[sid]"))
    items.push({ label: route.action });
  return (
    <nav aria-label="경로">
      <ol>
        {items.map((item, index) => (
          <li key={index}>
            {item.href && index < items.length - 1 ? (
              <Link
                href={item.href}
                title={typeof item.label === "string" ? item.label : undefined}
              >
                {item.label}
              </Link>
            ) : (
              <span
                aria-current={index === items.length - 1 ? "page" : undefined}
                title={typeof item.label === "string" ? item.label : undefined}
              >
                {item.label}
              </span>
            )}
          </li>
        ))}
      </ol>
    </nav>
  );
}
