"use client";

import { useActionState, useRef, useState } from "react";
import Link from "next/link";
import { Button, Banner, buttonClassName, ConfirmDialog } from "@repo/ui";
import { NodeErrorCard } from "@/components/node-error";
import type { NodeActionState, NodeFormAction } from "@/lib/node-action-state";
import { useApiAction } from "@/lib/use-api-action";
import styles from "@/app/(app)/reverse-proxy/reverse-proxy.module.css";

const INITIAL: NodeActionState = { ok: false };

/**
 * Detail-page action row for one server: enable/disable toggle + delete.
 * Both stay on/leave the detail page via the server actions themselves
 * (`toggleServer` revalidates in place, `deleteServer` redirects to the
 * list) — this component only needs `useActionState` to surface an error
 * message inline if either ever returns one.
 *
 * **액션은 prop 으로 들어온다**(`components/streams/detail-actions.tsx` 와 같은
 * 모양). 로컬은 `toggleServer`/`deleteServer`, 노드는 그 노드용 액션이다 —
 * 이 컴포넌트는 어느 쪽인지 모른다. 그래서 노드 실패도 그릴 수 있어야 한다:
 *
 * - **노드에 닿지 못함·버전 불일치 등** — §9 카드(`NodeErrorCard`).
 * - **로컬 액션의 실패** — `nodeError` 를 채우는 일이 없어 아래 `state.error`
 *   빨간 한 줄로 그대로 떨어진다(추출 전과 동일).
 */
export function ServerDetailActions({
  id,
  editHref,
  serverName,
  enabled,
  toggleAction,
  deleteAction,
}: {
  id: string;
  editHref: string;
  /** 삭제 확인 창의 서버 이름. */
  serverName: string;
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
        {/* nginx 반영은 셸의 「설정 적용」에서 한다. */}
        <form action={toggleFormAction}>
          <input type="hidden" name="id" value={id} />
          <Button
            type="submit"
            variant="secondary"
            disabled={togglePending}
            aria-busy={togglePending || undefined}
          >
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
          <Button
            type="submit"
            variant="dangerOutline"
            disabled={deletePending}
            aria-busy={deletePending || undefined}
          >
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
        description="이 서버를 삭제하시겠습니까?"
        target={serverName}
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
