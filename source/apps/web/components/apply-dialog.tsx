"use client";

import { useActionState, useEffect, useId, useRef } from "react";
import { Button, Icon, Modal, StatusBadge } from "@repo/ui";
import type { OperationsTarget } from "@repo/presentation/operations-target";
import type { ConfigDiff } from "@/lib/config-diff";
import {
  DIFF_MARKERS,
  diffCounts,
  pendingIntro,
} from "@/lib/config-diff-summary";
import { useApiAction } from "@/lib/use-api-action";
import {
  applyResultToast,
  type ApplyBarAction,
  type ApplyBarResult,
} from "./apply-bar";
import { useToast } from "./notifications/toast-provider";
import { ConfigDiffView } from "./config-diff-view";
import styles from "./apply-dialog.module.css";

export interface ApplyDialogProps {
  open: boolean;
  onClose: () => void;
  canApply: boolean;
  target: OperationsTarget;
  targetLabel: string;
  pendingCount: number;
  pendingHasBaseline: boolean;
  pendingDiff: ConfigDiff;
  applyAction: ApplyBarAction;
}
export function ReviewResultToast({
  result,
  onSuccess,
}: {
  result: ApplyBarResult | null;
  onSuccess: () => void;
}) {
  const { toast } = useToast();
  const shown = useRef<ApplyBarResult | null>(null);
  useEffect(() => {
    if (!result || result === shown.current) return;
    shown.current = result;
    toast(applyResultToast(result));
    if (result.ok) onSuccess();
  }, [result, toast, onSuccess]);
  return null;
}
export function ApplyDialog({
  open,
  onClose,
  canApply,
  target,
  targetLabel,
  pendingCount,
  pendingHasBaseline,
  pendingDiff,
  applyAction,
}: ApplyDialogProps) {
  const [result, formAction, pending] = useActionState<
    ApplyBarResult | null,
    FormData
  >(useApiAction<ApplyBarResult | null>(applyAction), null);
  const id = useId();
  const empty = pendingHasBaseline && pendingCount === 0;
  const counts = diffCounts(pendingDiff);
  const label = targetLabel + "에 적용";
  return (
    <Modal
      open={open}
      onClose={onClose}
      size={empty ? "default" : "review"}
      dismissible={!pending}
      title={canApply ? "설정 적용" : "미적용 변경"}
      description={
        !empty ? (
          <div className={styles.meta}>
            <span className={styles.targetCaption}>대상</span>
            {target.kind === "node" ? (
              <span className={styles.node}>
                <Icon name="server" size={12} />
                원격 노드
              </span>
            ) : null}
            <strong className={styles.targetName}>{targetLabel}</strong>
            <span className={styles.divider} aria-hidden />
            <span>
              {pendingHasBaseline ? "미적용 " : "적용 상태 확인 필요 · "}
              <strong>{pendingCount}</strong>건
            </span>
            <ul className={styles.counts} aria-label="변경 종류별 건수">
              {(["added", "changed", "removed", "skipped"] as const).map(
                (kind) =>
                  counts[kind] ? (
                    <li key={kind} data-kind={kind}>
                      <span aria-hidden>{DIFF_MARKERS[kind].char}</span>{" "}
                      {kind === "skipped" ? "제외" : DIFF_MARKERS[kind].label}{" "}
                      {counts[kind]}
                    </li>
                  ) : null,
              )}
            </ul>
          </div>
        ) : undefined
      }
      footer={
        canApply && !empty ? (
          <>
            {pending ? (
              <span
                className={styles.progress}
                role="progressbar"
                aria-label="적용 중"
              />
            ) : null}
            <Button
              type="button"
              variant="secondary"
              disabled={pending}
              onClick={onClose}
            >
              취소
            </Button>
            <Button
              type="submit"
              form={id}
              disabled={pending}
              aria-busy={pending}
              title={label}
              className={styles.submit}
            >
              <span className={styles.submitLabel} data-pending={pending}>
                {label}
              </span>
              {pending ? (
                <span className={styles.busy}>
                  <span className={styles.spinner} aria-hidden />
                  적용 중
                </span>
              ) : null}
            </Button>
          </>
        ) : (
          <Button type="button" variant="secondary" onClick={onClose}>
            닫기
          </Button>
        )
      }
    >
      <div
        className={styles.body}
        aria-busy={pending}
        data-target-kind={target.kind}
        data-target-id={target.kind === "node" ? target.id : undefined}
      >
        <form id={id} action={formAction} />
        {empty ? (
          <>
            <StatusBadge tone="success">모두 적용됨</StatusBadge>
            <p className={styles.lead}>
              저장된 설정이 모두 nginx에 적용되어 있습니다. 적용할 변경이
              없습니다.
            </p>
          </>
        ) : (
          <>
            <p className={styles.lead}>
              {pendingIntro(pendingDiff, canApply, pendingHasBaseline)}
            </p>
            <div role="region" aria-label="미적용 변경 목록">
              <ConfigDiffView
                diff={pendingDiff}
                target={target}
                canEdit={canApply}
                onNavigate={onClose}
                disabled={pending}
              />
            </div>
          </>
        )}
      </div>
      {canApply ? (
        <ReviewResultToast result={result} onSuccess={onClose} />
      ) : null}
    </Modal>
  );
}
