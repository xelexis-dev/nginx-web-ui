"use client";

import { startTransition, useActionState, useEffect, useRef, useState } from "react";
import { Button, Banner, ConfirmDialog } from "@repo/ui";
import type { ActionState } from "../../../../(app)/nodes/types";
import { useApiAction } from "@/lib/use-api-action";

const INITIAL: ActionState = { ok: false };

/**
 * 연결 해제 버튼. 되돌릴 수 없으므로 `ConfirmDialog` 으로 확인한다 — 이 저장소의
 * 선례(`certificates/[id]/detail-actions.tsx:102`, `history/rollback-button.tsx:31`)를
 * 따른다: 이름을 타이핑하게 하는 무거운 확인이 아니라 브라우저 확인창 하나다.
 *
 * 문구에는 **무엇이 사라지는지**를 담는다 — 노드가 지워지는 게 아니라 이
 * 게이트웨이에서만 연결이 끊긴다는 것. 그게 사용자가 실제로 걱정하는 지점이다.
 *
 * `unpairNode` 서버 액션(`../../actions.ts`)이 노드 쪽 토큰 해지 실패 여부와
 * 무관하게 성공하면 항상 리다이렉트한다(`/nodes` 또는
 * `/nodes?unpairWarning=revoke_failed`) — 그 경고는 목록 화면이 그린다. 여기서
 * 다루는 실패는 게이트웨이 쪽 행이 이미 없는 드문 경우(`not_found`)뿐이다.
 */
export function UnpairButton({ id, name }: { id: string; name: string }) {
  const [state, formAction, pending] = useActionState(
    useApiAction<ActionState>({ id: "nodes.unpairNode", args: [] }),
    INITIAL,
  );

  const formRef = useRef<HTMLFormElement>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  useEffect(() => { if (state !== INITIAL) setConfirmOpen(false); }, [state]);

  return (
    <div>
      <form
        action={formAction}
        ref={formRef}
        onSubmit={(event) => { event.preventDefault(); setConfirmOpen(true); }}
      >
        <input type="hidden" name="id" value={id} />
        <Button type="submit" variant="dangerOutline" disabled={pending}>
          {pending ? "해제하는 중..." : "연결 해제"}
        </Button>
      </form>
      <ConfirmDialog open={confirmOpen} onClose={() => setConfirmOpen(false)} title="연결 해제"
        description={`"${name}" 연결을 해제하시겠습니까? 노드의 설정은 그대로 남고, 이 게이트웨이에서만 사라집니다.`} target={name} confirmLabel="연결 해제" pending={pending}
        onConfirm={() => { if (formRef.current && !pending) startTransition(() => formAction(new FormData(formRef.current!))); }} />
      {state.error ? (
        <Banner tone="error" role="alert" title={state.error} />
      ) : null}
    </div>
  );
}
