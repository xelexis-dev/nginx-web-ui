import type { IconName } from "@repo/ui";

export type NavItem = {
  href: string;
  label: string;
  icon?: IconName;
  /** Only rendered for role === "admin" (checked by Sidebar). */
  adminOnly?: boolean;
};

export type NavCategory = {
  id: string;
  label: string;
  icon?: IconName;
  items: NavItem[];
};

export const PROXY_TABS: NavItem[] = [
  { href: "/reverse-proxy", label: "서버" },
  { href: "/reverse-proxy/upstreams", label: "Upstreams" },
  { href: "/reverse-proxy/templates", label: "템플릿" },
];
export const PROXY_RESOURCES: NavItem[] = [
  { href: "/reverse-proxy", label: "리버스 프록시" },
  { href: "/reverse-proxy/upstreams", label: "Upstreams" },
  { href: "/stream", label: "Stream" },
  { href: "/certificates", label: "인증서" },
];
export const NAV_CATEGORIES: NavCategory[] = [
  {
    id: "monitoring",
    label: "모니터링",
    items: [
      { href: "/", label: "개요", icon: "overview" },
      { href: "/metrics", label: "접속 지표", icon: "activity" },
      { href: "/logs", label: "로그", icon: "scrollText" },
    ],
  },
  {
    id: "configuration",
    label: "구성",
    items: [
      { ...PROXY_RESOURCES[0]!, icon: "route" },
      { ...PROXY_RESOURCES[2]!, icon: "cable" },
      { ...PROXY_RESOURCES[3]!, icon: "shieldCheck" },
    ],
  },
  {
    id: "operations",
    label: "운영",
    items: [
      { href: "/history", label: "적용 이력", icon: "history" },
      { href: "/audit", label: "감사 로그", icon: "fileClock" },
      { href: "/nodes", label: "노드", icon: "server" },
    ],
  },
  {
    id: "settings",
    label: "설정",
    items: [
      { href: "/account", label: "계정", icon: "circleUserRound" },
      { href: "/admin", label: "관리자", icon: "settings", adminOnly: true },
    ],
  },
];

// Admin-panel nav (admin-only section at /admin).
export const ADMIN_CATEGORIES: NavCategory[] = [
  {
    id: "admin",
    label: "관리자",
    icon: "users",
    items: [
      { href: "/admin/users", label: "사용자" },
      { href: "/admin/pending", label: "승인 대기" },
      { href: "/admin/oauth", label: "OAuth 로그인" },
      { href: "/admin/security", label: "보안 (2FA)" },
      // 보안(2FA)의 「연결된 게이트웨이」와 **같은 관계의 두 단계**다 — 여기서 내준
      // 열쇠가 저기 한 줄이 된다. 그래도 별항인 이유는 할 수 있는 일이 다르기
      // 때문이다: 여기는 「앞으로 누가 붙을 수 있나」, 저기는 「지금 붙은 것을 끊기」.
      // 스펙 §8-2 도 별도 페이지로 둔다.
      { href: "/admin/tokens", label: "액세스 토큰" },
      { href: "/admin/secrets", label: "저장된 비밀값" },
    ],
  },
  // 알림은 "무엇을 누구에게"(채널), 사이트 설정은 "어떻게 보내는가"(수단)로 가른다.
  {
    id: "notifications",
    label: "알림",
    icon: "notifications",
    items: [
      { href: "/admin/notifications/mail", label: "메일" },
      { href: "/admin/notifications/webhooks", label: "Webhook" },
    ],
  },
  {
    id: "site",
    label: "사이트 설정",
    icon: "settings",
    items: [
      { href: "/admin/settings/general", label: "일반" },
      { href: "/admin/settings/smtp", label: "SMTP 설정" },
    ],
  },
];

/**
 * The active nav item is the one whose href is the LONGEST prefix of the
 * current pathname (exact match for "/"). This keeps nested routes like
 * /reverse-proxy/upstreams from also lighting up /reverse-proxy. Scoped to the
 * given categories so the main and admin sidebars each match their own nav.
 */
export function activeHref(
  pathname: string,
  categories: NavCategory[],
): string | null {
  let best: string | null = null;
  for (const cat of categories) {
    for (const item of cat.items) {
      const match =
        item.href === "/"
          ? pathname === "/"
          : pathname === item.href || pathname.startsWith(item.href + "/");
      if (match && (best === null || item.href.length > best.length)) {
        best = item.href;
      }
    }
  }
  return best;
}
