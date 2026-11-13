"use client";

import type { ReactNode } from "react";
import { Button, StatusDot } from "@repo/ui";
import { ThemeToggle } from "@/components/theme-sync";
import styles from "./operations-shell.module.css";

/** Shared by the live shell and product previews; target data stays with the caller. */
export function HeaderActions({
  canApply,
  showPending = true,
  pendingCount,
  onApply,
  onReview = onApply,
  available = true,
  pendingHasBaseline = true,
  notifications,
  profile,
}: {
  canApply: boolean;
  showPending?: boolean;
  pendingCount: number;
  onApply: () => void;
  onReview?: () => void;
  available?: boolean;
  pendingHasBaseline?: boolean;
  notifications?: ReactNode;
  profile?: ReactNode;
}) {
  return (
    <>
      {!showPending ? null : !available ? (
        <span className={styles.pendingState} role="status">
          조회할 수 없음
        </span>
      ) : pendingCount > 0 ? (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className={styles.pendingLink}
          onClick={onReview}
          aria-haspopup="dialog"
          aria-label={`${pendingHasBaseline ? "" : "적용 상태 확인 필요, "}미적용 변경 ${pendingCount}건 보기`}
        >
          <StatusDot tone="warning" />
          <span className={styles.pendingText}>
            {pendingHasBaseline
              ? "미적용 변경"
              : "적용 상태 확인 필요 · 미적용 변경"}
          </span>
          <span>{pendingCount}</span>
        </Button>
      ) : (
        <span
          className={styles.pendingState}
          role="status"
          aria-label={
            pendingHasBaseline ? "모두 적용됨" : "적용 상태 확인 필요"
          }
          title={pendingHasBaseline ? "모두 적용됨" : "적용 상태 확인 필요"}
        >
          <StatusDot tone={pendingHasBaseline ? "success" : "warning"} />
          <span className={styles.pendingText}>
            {pendingHasBaseline ? "모두 적용됨" : "적용 상태 확인 필요"}
          </span>
        </span>
      )}
      {canApply ? (
        <Button
          type="button"
          size="sm"
          variant="secondary"
          data-shell-apply
          onClick={onApply}
        >
          적용
        </Button>
      ) : null}
      {showPending || canApply ? (
        <span className={styles.headerDivider} aria-hidden="true" />
      ) : null}
      {notifications}
      <ThemeToggle />
      {profile}
    </>
  );
}
