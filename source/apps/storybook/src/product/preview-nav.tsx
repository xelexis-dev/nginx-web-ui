// Where a click inside a product story goes. The real pages link to app routes, which do not exist in
// Storybook: a route that has a product story for the same role opens that story, any other route stays
// put and says so in the frame's notice. The role always wins: a viewer never lands on an editor story
// (it would show buttons a viewer does not have). The theme and viewport globals carry over, so a 390
// preview stays at 390 on the next story. Storybook only.
import { createContext, useContext } from "react";
import { addons } from "storybook/preview-api";
import { SELECT_STORY, UPDATE_GLOBALS } from "storybook/internal/core-events";

export type PreviewRole = "editor" | "viewer";

export interface PreviewNav {
  /** The story being shown, so a link to the same screen can say so instead of doing nothing. */
  storyId: string;
  role: PreviewRole;
  /** The viewport global in effect (a 390 story sets it on the story), kept for the next story. */
  viewport: string | undefined;
  notify: (message: string) => void;
}

export const PreviewNavContext = createContext<PreviewNav | null>(null);

type Variants = { editor: string; viewer?: string };

// Only routes whose story shows that exact screen. Row links to other servers are not here on
// purpose: the detail story is srv-api, and opening it from another row would show the wrong server.
const ROUTES: Record<string, Variants> = {
  "/metrics": { editor: "product-metrics--default" },
  "/nodes": { editor: "product-management-pages--nodes" },
  "/nodes/preview-node": { editor: "product-management-pages--node", viewer: "product-management-pages--node-viewer" },
  "/account": { editor: "product-management-pages--account" },
  "/admin/users": { editor: "product-management-pages--users" },
  "/reverse-proxy/upstreams": { editor: "product-resource-lists--upstream" },
  "/reverse-proxy/upstreams/api-pool": { editor: "product-resource-details--upstream" },
  "/stream/database": { editor: "product-resource-details--stream" },
  "/certificates": { editor: "product-resource-lists--certificate" },
  "/certificates/cert-api": { editor: "product-resource-details--certificate", viewer: "product-resource-details--certificate-viewer" },
  "/audit": { editor: "product-resource-lists--audit" },
  "/audit/audit-1": { editor: "product-resource-details--audit" },
  "/account/tokens": { editor: "product-resource-lists--token" },
  "/stream": { editor: "product-stream-list--default", viewer: "product-stream-list--viewer" },
  "/account/2fa": { editor: "product-account-2fa-management--default", viewer: "product-account-2fa-management--default" },
  "/account/2fa/backup-codes/new": { editor: "product-account-2fa-regenerate--default", viewer: "product-account-2fa-regenerate--default" },
  "/account/2fa/disable": { editor: "product-account-2fa-disable--default", viewer: "product-account-2fa-disable--default" },
  "/": { editor: "product-operations-overview--default", viewer: "product-operations-overview--viewer" },
  "/reverse-proxy": { editor: "product-operations-server-list--editor", viewer: "product-operations-server-list--viewer" },
  "/reverse-proxy/srv-api": { editor: "product-operations-server-detail--default", viewer: "product-operations-server-detail--viewer" },
  "/reverse-proxy/srv-api/edit": { editor: "product-operations-server-form--edit" },
  "/reverse-proxy/new": { editor: "product-operations-server-form--new" },
  // The sign-in factor screens (no app shell, no role): each links to the other.
  "/login/2fa": { editor: "product-login-2fa--default", viewer: "product-login-2fa--default" },
  "/login/2fa/backup": { editor: "product-login-backup--default", viewer: "product-login-backup--default" },
};

/** The story for `href` as seen by `role`, or null when the preview has none for that role. */
export function storyFor(href: string, role: PreviewRole): string | null {
  const route = ROUTES[href.split(/[?#]/)[0] ?? href];
  if (!route) return null;
  return (role === "viewer" ? route.viewer : route.editor) ?? null;
}

export function usePreviewNav(): (href: string) => void {
  const nav = useContext(PreviewNavContext);
  return (href) => {
    if (!nav) return;
    let target = storyFor(href, nav.role);
    // The OAuth-only account keeps its reauthentication method across its management stories.
    if (nav.storyId.endsWith("--o-auth-only") && (target === "product-account-2fa-management--default" || target === "product-account-2fa-regenerate--default")) {
      target = target.replace(/--default$/, "--o-auth-only");
    }
    if (target === nav.storyId) {
      nav.notify(`지금 보고 있는 화면입니다 (${href}).`);
    } else if (target) {
      const channel = addons.getChannel();
      if (nav.viewport) channel.emit(UPDATE_GLOBALS, { globals: { viewport: { value: nav.viewport, isRotated: false } } });
      channel.emit(SELECT_STORY, { storyId: target });
    } else if (nav.role === "viewer" && storyFor(href, "editor")) {
      nav.notify(`${href} 화면은 보기 전용 미리보기가 없어 이동하지 않았습니다.`);
    } else {
      nav.notify(`${href} 화면은 이 미리보기에 없어 이동하지 않았습니다.`);
    }
  };
}
