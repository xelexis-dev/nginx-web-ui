"use client";

import { useActionState, useRef, useState } from "react";
import Link from "next/link";
import { Button, Banner, buttonClassName, ConfirmDialog } from "@repo/ui";
import { NodeErrorCard } from "@/components/node-error";
import type { NodeActionState, NodeFormAction } from "@/lib/node-action-state";
import { useApiAction } from "@/lib/use-api-action";
// 로컬 리버스 프록시 화면들의 공용 시트를 그대로 쓴다 — 왜 복사하지 않고
// 들여오는지(그리고 공유 컴포넌트 CSS 규칙)는 `upstream-list.tsx` 헤더 참고.
import styles from "@/app/(app)/reverse-proxy/reverse-proxy.module.css";

const INITIAL: NodeActionState = { ok: false };

/**
 * 상세 화면의 삭제 액션. 로컬은 `deleteUpstream`, 노드는
 * `deleteNodeUpstream.bind(null, nodeId)` 가 들어온다 — 이 컴포넌트는 어느
 * 쪽인지 모른다.
 *
 * **성공하면 이 컴포넌트로 돌아오지 않는다** — 두 액션 다 목록으로
 * `redirect` 한다. 여기 그려지는 것은 실패뿐이다:
 *
 * - **사용 중인 upstream 삭제(409 `conflict`)** — 노드가 그 판정을 내고,
 *   `runNodeWrite` 가 `nodeError` 로 접어 보내면 `NodeErrorCard` 가 **배너**로
 *   그린다(카드가 아니다 — `node-error.tsx` 문서). 노드 상태 배지는 건드리지
 *   않는다: `statusOf(conflict) === null` 이라 `recordProbe` 가 DB 를 아예 안
 *   친다.
 * - 로컬은 같은 판정을 DB(FK 위반)로 하고 **같은 문구**를 쓴다 — 두 경로가
 *   `deleteUpstreamRow` 한 함수에서 문구를 가져오기 때문이다
 *   (`(app)/reverse-proxy/upstreams/service.ts`). 로컬은 `nodeError` 를 채우는
 *   일이 없어 아래 `state.error` 한 줄로 떨어진다(추출 전과 동일).
 */
export function UpstreamDetailActions({
  id,
  targetName = id,
  editHref,
  action,
}: {
  id: string;
  targetName?: string;
  editHref: string;
  action: NodeFormAction;
}) {
  const [deleteState, deleteAction, deletePending] = useActionState(
    useApiAction<NodeActionState>(action),
    INITIAL,
  );

  const deleteForm = useRef<HTMLFormElement>(null);
  const approvedId = useRef<string | null>(null);
  const [confirmId, setConfirmId] = useState<string | null>(null);

  return (
    <div className={styles.opsDetailActions}>
      <div className={styles.detailActionsRow}>
        <form ref={deleteForm} action={deleteAction} onSubmit={(event) => {
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
        description="이 Upstream을 삭제하시겠습니까?"
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
      {deleteState.nodeError ? (
        <NodeErrorCard {...deleteState.nodeError} />
      ) : deleteState.error ? (
        <Banner tone="error" role="alert" title={deleteState.error} />
      ) : null}
    </div>
  );
}
