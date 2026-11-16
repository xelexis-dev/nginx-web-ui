"use client";

import { useState } from "react";
import { Button, Icon, IconButton, StatusBadge } from "@repo/ui";
import type { ConfigDiff } from "@/lib/config-diff";
import { ApplyDialog } from "./apply-dialog";
import styles from "./pending-changes-badge.module.css";

/**
 * 적용 상태 배지. 미적용이 있으면 버튼이 되어 "무엇이 미적용인지"를 모달로 보여준다
 * (적용 전에 확인할 수 있어야 하므로 — 적용 후 확인은 /history/[id]가 담당).
 * count와 diff는 같은 서버 계산에서 나오므로 숫자와 목록이 어긋나지 않는다.
 */
export function PendingChangesBadge({
  count,
  hasBaseline,
  diff,
}: {
  count: number;
  hasBaseline: boolean;
  diff: ConfigDiff;
}) {
  const [open, setOpen] = useState(false);

  if (count === 0)
    return hasBaseline ? (
      <StatusBadge tone="success" size="sm" role="status">
        모두 적용됨
      </StatusBadge>
    ) : (
      <StatusBadge tone="warning" size="sm" dot={false} role="status">
        적용 상태 확인 필요
      </StatusBadge>
    );
  return (
    <>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        onClick={() => setOpen(true)}
        aria-haspopup="dialog"
      >
        <StatusBadge tone="warning" size="sm">
          {hasBaseline
            ? `미적용 변경 ${count}건`
            : `적용 상태 확인 필요 · ${count}건`}
        </StatusBadge>
      </Button>
      <ApplyDialog
        open={open}
        onClose={() => setOpen(false)}
        canApply={false}
        target={{ kind: "local" }}
        targetLabel="로컬 서버"
        pendingCount={count}
        pendingHasBaseline={hasBaseline}
        pendingDiff={diff}
        applyAction={async () => ({ ok: false })}
      />
    </>
  );
}

/** Placement, route gating and per-session dismissal belong to the shell. */
export function PendingBanner({
  count,
  onReview,
  onDismiss,
}: {
  count: number;
  onReview: () => void;
  onDismiss: () => void;
}) {
  if (count <= 0) return null;
  return (
    <div className={styles.banner} role="status">
      <Icon name="alert" size={16} />
      <p>미적용 변경 {count}건은 설정을 적용해야 nginx에 반영됩니다.</p>
      <button type="button" className={styles.review} onClick={onReview}>
        변경 내용 보기
      </button>
      <IconButton
        label="미적용 변경 알림 닫기"
        variant="ghost"
        size="sm"
        onClick={onDismiss}
      >
        <Icon name="close" size={16} />
      </IconButton>
    </div>
  );
}
