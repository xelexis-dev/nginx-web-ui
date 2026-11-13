"use client";

import { useActionState } from "react";
import Link from "next/link";
import { Button, Switch, Banner, buttonClassName } from "@repo/ui";
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
  /** 스위치의 접근 가능한 이름(「api.example.test 활성」). */
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

  return (
    <div className={styles.opsDetailActions}>
      <div className={styles.detailActionsRow}>
        {/* 스위치는 저장만 한다 — nginx 반영은 셸의 「설정 적용」이다(목록의 행 스위치와 같다). */}
        <form action={toggleFormAction}>
          <input type="hidden" name="id" value={id} />
          <Switch
            type="submit"
            checked={enabled}
            label={`${serverName} 활성`}
            showState
            disabled={togglePending}
            aria-busy={togglePending || undefined}
          />
        </form>
        <form action={deleteFormAction}>
          <input type="hidden" name="id" value={id} />
          <Button
            type="submit"
            variant="dangerQuiet"
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
