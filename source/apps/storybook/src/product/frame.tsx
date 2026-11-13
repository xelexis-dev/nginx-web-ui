// The frame around a product story: the @repo/ui AppShell with the app's real navigation
// (apps/web nav-links), real HeaderActions/ProfileMenu/NotificationBellView and fixture target and state.
// It stands in for OperationsShell's session/API adapter; the controls and page are real components.
//
// Nothing here saves or navigates the app: links open the matching product story or say the screen is
// not in the preview (preview-nav.tsx), and 「설정 적용」 says what the app would do instead.
import { useEffect, useMemo, useState, type ReactNode } from "react";
import type { Decorator } from "@storybook/react-vite";
import {
  AppShell,
  Banner,
  ContextBar,
  SectionNav,
  StatusBadge,
  type AppShellNavGroup,
} from "@repo/ui";
import type { Role } from "@repo/contracts";
import { PendingBanner } from "@/components/pending-changes-badge";
import { HeaderActions } from "@/components/operations/header-actions";
import shellStyles from "@/components/operations/operations-shell.module.css";
import { ProfileMenu } from "@/components/profile-menu";
import { APP_SEMVER, FULL_SHA, SHA_LABEL } from "@/lib/version";
import type { NotificationItem } from "@/components/notifications/notification-bell-view";
import { PreviewNotifications } from "./preview-notifications";
import { NAV_CATEGORIES, ADMIN_CATEGORIES } from "@/app/(app)/nav-links";
import Link from "./adapters/next-link";
import { nodeNavCategories } from "@/app/(node)/nodes/[id]/node-nav";
import { PreviewNavContext, type PreviewRole } from "./preview-nav";

// Permission filtering happens in ProductFrame for the selected preview role.
const NAV: AppShellNavGroup[] = NAV_CATEGORIES.map((group) => ({
  id: group.id,
  label: group.label,
  items: group.items.map((item) => ({
    href: item.href,
    label: item.label,
    icon: item.icon,
    match: item.href === "/" ? "exact" : "prefix",
  })),
}));

const APPLY_NOTICE =
  "미리보기에서는 설정을 적용하지 않습니다. 앱에서는 미적용 변경과 nginx -t 결과를 보여 주는 창이 열립니다.";

export interface ProductFrameProps {
  path: string;
  label: string;
  /** editor/admin: apply is shown; viewer: it is not, as in the app. */
  role: Role;
  storyId: string;
  viewport: string | undefined;
  pendingCount?: number;
  nodeName?: string;
  available?: boolean;
  pendingHasBaseline?: boolean;
  notificationItems?: NotificationItem[];
  unreadCount?: number;
  children: ReactNode;
}

export function ProductFrame({
  path,
  label,
  role,
  storyId,
  viewport,
  pendingCount = 2,
  nodeName = "미리보기 노드",
  available = true,
  pendingHasBaseline = true,
  notificationItems,
  unreadCount = 0,
  children,
}: ProductFrameProps) {
  const [notice, setNotice] = useState<string | null>(null);
  const [bannerDismissed, setBannerDismissed] = useState(false);
  useEffect(() => setBannerDismissed(false), [path]);
  const admin = path.startsWith("/admin");
  const nodeId = /^\/nodes\/(?!new(?:\/|$))([^/]+)/.exec(path)?.[1];
  const configPath = nodeId ? path.slice(`/nodes/${nodeId}`.length) : path;
  const configuration = ["/reverse-proxy", "/stream", "/certificates"].some(
    (base) => configPath === base || configPath.startsWith(`${base}/`),
  );
  const previewRole: PreviewRole = role === "viewer" ? "viewer" : "editor";
  const nav = useMemo(
    () => ({ storyId, role: previewRole, viewport, notify: setNotice }),
    [storyId, previewRole, viewport],
  );
  return (
    <PreviewNavContext.Provider value={nav}>
      <AppShell
        nav={NAV.map((group) => ({
          ...group,
          items: group.items.filter(
            (item) => item.href !== "/admin" || role === "admin",
          ),
        }))}
        currentPath={path}
        renderLink={({ href, className, children: text, onClick, ...rest }) => (
          <Link
            href={href}
            className={className}
            aria-current={rest["aria-current"]}
            title={rest.title}
            onClick={onClick}
          >
            {text}
          </Link>
        )}
        version={APP_SEMVER}
        banner={
          available && pendingCount > 0 && configuration && !bannerDismissed ? (
            <div className={shellStyles.pendingBand}>
              <div className={shellStyles.shellColumn}>
                <PendingBanner
                  count={pendingCount}
                  onReview={() => setNotice(APPLY_NOTICE)}
                  onDismiss={() => setBannerDismissed(true)}
                />
              </div>
            </div>
          ) : undefined
        }
        topbarContext={
          <div className={shellStyles.crumbs}>
            <nav aria-label="경로">
              <ol>
                {nodeId ? (
                  <>
                    <li>운영</li>
                    <li>노드</li>
                    <li title={nodeName}>{nodeName}</li>
                  </>
                ) : (
                  <li>
                    {admin
                      ? "설정"
                      : (NAV_CATEGORIES.find((g) =>
                          g.items.some((i) => i.href === path),
                        )?.label ?? "구성")}
                  </li>
                )}
                <li aria-current="page" title={label}>
                  {label}
                </li>
              </ol>
            </nav>
          </div>
        }
        topbarActions={
          <HeaderActions
            canApply={!admin && available && role !== "viewer"}
            showPending={!admin}
            pendingCount={pendingCount}
            available={available}
            pendingHasBaseline={pendingHasBaseline}
            profile={
              <ProfileMenu
                email={`${role}@example.test`}
                role={role}
                version={APP_SEMVER}
                sha={SHA_LABEL}
                fullSha={FULL_SHA}
              />
            }
            onApply={() => setNotice(APPLY_NOTICE)}
            notifications={
              admin ? null : (
                <PreviewNotifications
                  key={unreadCount}
                  items={notificationItems}
                  unreadCount={unreadCount}
                />
              )
            }
          />
        }
        contentHeader={
          nodeId ? (
            <>
              <div className={shellStyles.nodeContextBand}>
                <div className={shellStyles.shellColumn}>
                  <ContextBar
                    name={nodeName}
                    offline={!available}
                    status={
                      <StatusBadge
                        tone={available ? "success" : "error"}
                        size="sm"
                      >
                        {available ? "응답함" : "조회할 수 없음"}
                      </StatusBadge>
                    }
                    renderLink={(props) => <Link {...props} />}
                  />
                </div>
              </div>
              <div className={shellStyles.shellColumn}>
                <SectionNav
                  items={nodeNavCategories(nodeId).flatMap((g) => g.items)}
                  currentPath={path}
                  variant="tabs"
                  label="노드 메뉴"
                  renderLink={(props) => <Link {...props} />}
                />
              </div>
            </>
          ) : admin ? (
            <div className={shellStyles.shellColumn}>
              <SectionNav
                groups={ADMIN_CATEGORIES}
                variant="tabs"
                currentPath={path}
                label="관리자 설정"
                renderLink={(props) => <Link {...props} />}
              />
            </div>
          ) : undefined
        }
      >
        {notice ? (
          <Banner
            tone="info"
            title="미리보기"
            onDismiss={() => setNotice(null)}
          >
            {notice}
          </Banner>
        ) : null}
        {children}
      </AppShell>
    </PreviewNavContext.Provider>
  );
}

/** A story decorator: the frame for `path`, with the role worked out from the story's args. */
export function productFrame(
  frame: (args: Record<string, unknown>) => {
    path: string;
    label: string;
    role: Role;
  },
): Decorator {
  return (Story, context) => {
    const { path, label, role } = frame(context.args);
    const viewport = (
      context.globals.viewport as { value?: string } | undefined
    )?.value;
    return (
      <ProductFrame
        path={path}
        label={label}
        role={role}
        storyId={context.id}
        viewport={viewport}
      >
        <Story />
      </ProductFrame>
    );
  };
}

/** Viewports for the product stories: `globals: { viewport: { value: "p390" } }`. */
export const productViewports = {
  p390: {
    name: "Mobile 390",
    styles: { width: "390px", height: "844px" },
    type: "mobile",
  },
  p1440: {
    name: "Desktop 1440",
    styles: { width: "1440px", height: "900px" },
    type: "desktop",
  },
} as const;

/**
 * A story decorator for the sign-in screens, which have no app shell: the same link handling as
 * ProductFrame (a link to a screen with a story opens it, keeping theme and viewport; any other link
 * says so here), with the notice shown above the screen.
 */
export function authFrame(): Decorator {
  return (Story, context) => {
    const viewport = (
      context.globals.viewport as { value?: string } | undefined
    )?.value;
    return (
      <AuthFrameNav storyId={context.id} viewport={viewport}>
        <Story />
      </AuthFrameNav>
    );
  };
}

function AuthFrameNav({
  storyId,
  viewport,
  children,
}: {
  storyId: string;
  viewport: string | undefined;
  children: ReactNode;
}) {
  const [notice, setNotice] = useState<string | null>(null);
  const nav = useMemo(
    () => ({ storyId, role: "editor" as const, viewport, notify: setNotice }),
    [storyId, viewport],
  );
  return (
    <PreviewNavContext.Provider value={nav}>
      {notice ? (
        <div
          style={{
            position: "fixed",
            insetInline: "var(--sp-4)",
            top: "var(--sp-4)",
            zIndex: 10,
          }}
        >
          <Banner
            tone="info"
            title="미리보기"
            onDismiss={() => setNotice(null)}
          >
            {notice}
          </Banner>
        </div>
      ) : null}
      {children}
    </PreviewNavContext.Provider>
  );
}
