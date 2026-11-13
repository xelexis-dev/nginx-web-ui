"use client";

import { formatServerTime } from "@/lib/format-time";

import { startTransition, useActionState, useEffect, useRef, useState } from "react";
import { Banner, Button, ConfirmDialog, Icon, buttonClassName } from "@repo/ui";
import { NodeErrorCard } from "@/components/node-error";
import type { NodeResult } from "@/lib/node-result";
import historyStyles from "@/components/history/history.module.css";
import { useApiAction } from "@/lib/use-api-action";
interface RollbackResponse { output: string }

type RollbackState = NodeResult<RollbackResponse> | { ok: false; error: string } | null;

function fallbackLabel(iso: string): string {
  return formatServerTime(new Date(iso));
}

/**
 * "이 스냅샷으로 롤백" 버튼. 로컬(`(app)/history/rollback-button.tsx`)과 같은
 * `ConfirmDialog` 관용을 따르지만, 문구는 더 담는다 — 이 화면에는 로컬의
 * 상세 페이지(스냅샷 내용을 먼저 보는 곳)가 없다. **어느 노드인지·어느
 * 스냅샷인지·내용을 확인하지 못했다는 것** 셋을 문장에 넣는다.
 *
 * 성공 배너는 로컬과 같은 톤(`history.module.css` 의 `.bannerSuccess`)을
 * 공유한다. 실패는 `NodeErrorCard` 를 그대로 쓴다 — §9 표를 옮긴 그 컴포넌트가
 * `domain_failure`(422, nginx -t 실패) 를 이미 스크롤 가능한 `<pre>` 에 그대로
 * 담아 그리므로 여기서 다시 구현하지 않는다. `NodeErrorCard` 는 클라이언트
 * 컴포넌트에서 import 해도 안전하다(순수 presentation 헬퍼와 타입만 사용한다).
 */
interface NodeRollbackProps {
  nodeId: string;
  nodeName: string;
  snapshotId: string;
  label: string | null;
  iso: string;
  retryHref: string;
}

/** The same row composition is used by the node page and its Storybook preview. */
export function NodeHistoryActions({ nodeUrl, canRollback, ...props }: NodeRollbackProps & {
  nodeUrl: string;
  canRollback: boolean;
}) {
  return (
    <div className={historyStyles.rowActions}>
      <a href={`${nodeUrl}/history/${props.snapshotId}`} target="_blank" rel="noopener noreferrer"
        className={buttonClassName({ variant: "secondary", size: "sm" })}>
        노드에서 열기
      </a>
      {canRollback ? <NodeRollbackButton {...props} /> : null}
    </div>
  );
}

function NodeRollbackButton({ nodeId, nodeName, snapshotId, label, iso, retryHref }: NodeRollbackProps) {
  const rollbackAction = useApiAction<RollbackState>({ id: "node.rollbackNodeSnapshot", args: [nodeId, snapshotId] });

  const [result, formAction, pending] = useActionState<RollbackState, FormData>(rollbackAction, null);

  const formRef = useRef<HTMLFormElement>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  useEffect(() => { if (result !== null) setConfirmOpen(false); }, [result]);

  const snapshotLabel = label ?? fallbackLabel(iso);
  const confirmText = `"${nodeName}" 을(를) "${snapshotLabel}" 스냅샷으로 되돌립니다. 현재 설정을 덮어쓰며 되돌릴 수 없습니다. 이 화면에서는 스냅샷 내용을 확인할 수 없습니다 — 필요하면 "노드에서 열기"로 먼저 확인하세요.`;

  return (
    <>
      <form
        action={formAction}
        ref={formRef}
        onSubmit={(event) => { event.preventDefault(); setConfirmOpen(true); }}
      >
        <Button type="submit" variant="secondary" size="sm" disabled={pending}>
          <Icon name="rotateCcw" size={16} />
          {pending ? "롤백 중..." : "이 스냅샷으로 롤백"}
        </Button>
      </form>
      <ConfirmDialog open={confirmOpen} onClose={() => setConfirmOpen(false)} title="이 스냅샷으로 롤백"
        description={confirmText} target={`${nodeName} · ${snapshotLabel}`} confirmLabel="이 스냅샷으로 롤백" pending={pending}
        onConfirm={() => { if (formRef.current && !pending) startTransition(() => formAction(new FormData(formRef.current!))); }} />

      {result ? (
        <div className={historyStyles.rowFeedback}>
          {result.ok ? (
            <Banner tone="success" role="status" title="롤백 완료 — nginx -t 통과, reload 완료" output={result.value.output} />
          ) : (
            // 이 버튼은 히스토리 **목록** 화면의 행에 산다 — 「다시 시도」와
            // 「목록으로」가 같은 곳이라 같은 값을 넘긴다.
            <NodeErrorCard failure={"kind" in result ? result : { kind: "node_error", message: result.error }} nodeId={nodeId} retryHref={retryHref} listHref={retryHref} />
          )}
        </div>
      ) : null}
    </>
  );
}
