"use client";

import { useActionState, useRef, useState } from "react";
import Link from "next/link";
import { Button, Banner, buttonClassName, ConfirmDialog } from "@repo/ui";
import { NodeErrorCard } from "@/components/node-error";
import type { NodeActionState, NodeFormAction } from "@/lib/node-action-state";
import { useApiAction } from "@/lib/use-api-action";
import styles from "./stream.module.css";

const INITIAL: NodeActionState = { ok: false };

/**
 * 상세 화면의 액션 줄: 활성/비활성 토글 + 삭제. 로컬은 `toggleStream`/
 * `deleteStream`, 노드는 `toggleNodeStream`/`deleteNodeStream` 을 `nodeId` 로
 * 묶은 것이 들어온다 — 이 컴포넌트는 어느 쪽인지 모른다.
 *
 * 토글은 제자리(액션이 `revalidate`), 삭제는 목록으로 `redirect` 한다. 그래서
 * 여기 그려지는 것은 실패뿐이다:
 *
 * - **노드에 닿지 못함·버전 불일치 등** — §9 카드(`NodeErrorCard`). 어느 쪽을
 *   올려야 하는지, 다시 연결하려면 어디로 가야 하는지는 그 카드만 말해준다.
 * - **로컬 액션의 실패** — `nodeError` 를 채우는 일이 없어 아래 `state.error`
 *   빨간 한 줄로 그대로 떨어진다(추출 전과 동일).
 */
export function StreamDetailActions({
  id,
  targetName = id,
  editHref,
  enabled,
  toggleAction,
  deleteAction,
}: {
  id: string;
  targetName?: string;
  editHref: string;
  enabled: boolean;
  toggleAction: NodeFormAction;
  deleteAction: NodeFormAction;
}) {
  const [toggleState, toggleFormAction, togglePending] = useActionState(
    useApiAction<NodeActionState>(toggleAction),
    INITIAL,
  );
  const [deleteState, deleteFormAction, deletePending] = useActionState(
    useApiAction<NodeActionState>(deleteAction),
    INITIAL,
  );

  const deleteForm = useRef<HTMLFormElement>(null);
  const approvedId = useRef<string | null>(null);
  const [confirmId, setConfirmId] = useState<string | null>(null);

  return (
    <div className={styles.opsDetailActions}>
      <div className={styles.detailActionsRow}>
        <form action={toggleFormAction}>
          <input type="hidden" name="id" value={id} />
          <Button type="submit" variant="secondary" disabled={togglePending}>
            {enabled ? "비활성화" : "활성화"}
          </Button>
        </form>
        <form ref={deleteForm} action={deleteFormAction} onSubmit={(event) => {
          if (deletePending || approvedId.current !== id) {
            event.preventDefault();
            if (!(deletePending)) setConfirmId(id);
          }
          approvedId.current = null;
        }}>
          <input type="hidden" name="id" value={id} />
          <Button type="submit" variant="dangerQuiet" disabled={deletePending}>
            삭제
          </Button>
        </form>
        <Link href={editHref} className={buttonClassName({ variant: "secondary" })}>
          수정
        </Link>
      </div>
      <ConfirmDialog
        open={confirmId === id}
        onClose={() => setConfirmId(null)}
        title="삭제"
        description="이 Stream을 삭제하시겠습니까?"
        target={targetName}
        confirmLabel="삭제"
        pending={deletePending}
        onConfirm={() => {
          if (confirmId !== id || deletePending) return;
          approvedId.current = id;
          setConfirmId(null);
          deleteForm.current?.requestSubmit();
        }}
      />
      {toggleState.nodeError ? (
        <NodeErrorCard {...toggleState.nodeError} />
      ) : toggleState.error ? (
        <Banner tone="error" role="alert" title={toggleState.error} />
      ) : null}
      {deleteState.nodeError ? (
        <NodeErrorCard {...deleteState.nodeError} />
      ) : deleteState.error ? (
        <Banner tone="error" role="alert" title={deleteState.error} />
      ) : null}
    </div>
  );
}
