"use client";

import { useEffect, useState, type ReactNode } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import type { Role } from "@repo/contracts";
import {
  targetLabel,
  type OperationsTarget,
} from "@repo/presentation/operations-target";
import {
  AppShell,
  Modal,
  SectionNav,
  type AppShellNavGroup,
  type SectionNavProps,
} from "@repo/ui";
import {
  activeHref,
  NAV_CATEGORIES,
  ADMIN_CATEGORIES,
  type NavCategory,
} from "@/app/(app)/nav-links";
import { nodeNavCategories } from "@/app/(node)/nodes/[id]/node-nav";
import { APP_SEMVER, FULL_SHA, SHA_LABEL } from "@/lib/version";
import type { OperationsShellData } from "@/lib/operations-shell-data";
import { PendingBanner } from "@/components/pending-changes-badge";
import { ConfigDiffView } from "@/components/config-diff-view";
import { ApplyReview } from "@/components/apply-bar";
import { ProfileMenu } from "@/components/profile-menu";
import { HeaderActions } from "./header-actions";
import { NotificationBell } from "@/components/notifications/notification-bell";
import { ToastViewport } from "@/components/notifications/toast-provider";
import styles from "./operations-shell.module.css";

export interface OperationsShellProps {
  role: Role;
  email: string;
  target: OperationsTarget;
  contentHeader?: ReactNode;
  crumbs?: ReactNode;
  data?: OperationsShellData;
  notifications?: boolean;
  children: ReactNode;
}

export function OperationsSectionNav(
  props: Omit<SectionNavProps, "currentPath" | "renderLink">,
) {
  return (
    <SectionNav
      {...props}
      currentPath={usePathname()}
      renderLink={(link) => <Link {...link} />}
    />
  );
}

export function OperationsShell({
  role,
  email,
  target,
  contentHeader,
  crumbs,
  data,
  notifications = true,
  children,
}: OperationsShellProps) {
  const pathname = usePathname();
  const available = Boolean(data?.available);
  const canApply = Boolean(
    data?.available && data.canApply && role !== "viewer",
  );
  const reviewKey = JSON.stringify([
    pathname,
    target.kind,
    target.kind === "node" ? target.id : null,
    targetLabel(target),
    available,
    canApply,
    role,
  ]);
  const [reviewFor, setReviewFor] = useState<string | null>(null);
  // The identity comparison also prevents a stale review rendering before this effect runs.
  useEffect(() => setReviewFor(null), [reviewKey]);
  const visible = NAV_CATEGORIES.map((group) => ({
    ...group,
    items: group.items.filter((item) => !item.adminOnly || role === "admin"),
  })).filter((group) => group.items.length);
  const nav: AppShellNavGroup[] = visible.map((group) => ({
    ...group,
    items: group.items.map((item) => ({
      ...item,
      match: item.href === "/" ? "exact" : "prefix",
    })),
  }));
  const admin = pathname === "/admin" || pathname.startsWith("/admin/");
  const pageGroups: NavCategory[] =
    target.kind === "node"
      ? nodeNavCategories(target.id)
      : admin
        ? ADMIN_CATEGORIES
        : visible;
  const current = activeHref(pathname, pageGroups);
  const currentLabel =
    pageGroups
      .flatMap((group) => group.items)
      .find((item) => item.href === current)?.label ??
    (admin ? "관리자" : "개요");
  const label = targetLabel(target);
  const targetKey = target.kind === "node" ? `node:${target.id}` : "local";
  const [dismissed, setDismissed] = useState<Record<string, true>>({});
  useEffect(() => {
    try {
      if (
        sessionStorage.getItem(`nwui-pending-dismissed:${targetKey}`) === "true"
      )
        setDismissed((previous) => ({ ...previous, [targetKey]: true }));
    } catch {
      /* Storage is optional. */
    }
  }, [targetKey]);
  const configPath =
    target.kind === "node" && pathname.startsWith(`/nodes/${target.id}/`)
      ? pathname.slice(`/nodes/${target.id}`.length)
      : pathname;
  const configuration = ["/reverse-proxy", "/stream", "/certificates"].some(
    (path) => configPath === path || configPath.startsWith(`${path}/`),
  );
  const group = pageGroups.find((g) =>
    g.items.some((item) => item.href === current),
  )?.label;
  const fallback =
    target.kind === "node"
      ? ["운영", "노드", label, currentLabel]
      : admin
        ? ["설정", "관리자", currentLabel]
        : [group, currentLabel].filter(Boolean);
  return (
    <>
      <AppShell
        nav={nav}
        currentPath={pathname}
        renderLink={(link) => <Link {...link} />}
        contentHeader={contentHeader}
        version={APP_SEMVER}
        banner={
          data?.available &&
          data.pendingCount > 0 &&
          configuration &&
          !dismissed[targetKey] ? (
            <div data-shell-pending className={styles.pendingBand}>
              <div className={styles.shellColumn}>
                <PendingBanner
                  count={data.pendingCount}
                  onReview={() => setReviewFor(reviewKey)}
                  onDismiss={() => {
                    setDismissed((previous) => ({
                      ...previous,
                      [targetKey]: true,
                    }));
                    try {
                      sessionStorage.setItem(
                        `nwui-pending-dismissed:${targetKey}`,
                        "true",
                      );
                    } catch {
                      /* Storage is optional. */
                    }
                  }}
                />
              </div>
            </div>
          ) : null
        }
        topbarContext={
          <div className={styles.crumbs}>
            {crumbs ?? (
              <nav aria-label="경로">
                <ol>
                  {fallback.map((item, index) => (
                    <li
                      key={index}
                      title={item}
                      aria-current={
                        index === fallback.length - 1 ? "page" : undefined
                      }
                    >
                      {item}
                    </li>
                  ))}
                </ol>
              </nav>
            )}
          </div>
        }
        topbarActions={
          <HeaderActions
            canApply={canApply}
            pendingCount={data?.available ? data.pendingCount : 0}
            available={available}
            showPending={data !== undefined}
            pendingHasBaseline={
              data?.available ? data.pendingHasBaseline : false
            }
            onReview={() => setReviewFor(reviewKey)}
            profile={
              <ProfileMenu
                email={email}
                role={role}
                version={APP_SEMVER}
                sha={SHA_LABEL}
                fullSha={FULL_SHA}
              />
            }
            onApply={() => setReviewFor(reviewKey)}
            notifications={notifications ? <NotificationBell /> : null}
          />
        }
      >
        {children}
        {notifications ? <ToastViewport /> : null}
      </AppShell>
      {data?.available ? (
        <Modal
          open={reviewFor === reviewKey}
          onClose={() => setReviewFor(null)}
          title={canApply ? "설정 적용" : "미적용 변경"}
        >
          {canApply ? (
            <ApplyReview
              target={target}
              targetLabel={label}
              onCancel={() => setReviewFor(null)}
              canApply
              pendingCount={data.pendingCount}
              pendingHasBaseline={data.pendingHasBaseline}
              pendingDiff={data.pendingDiff}
              applyAction={data.applyAction}
            />
          ) : (
            <ConfigDiffView diff={data.pendingDiff} />
          )}
        </Modal>
      ) : null}
    </>
  );
}
