"use client";

import { useActionState, useRef, useState } from "react";
import { Button, Banner, buttonClassName, ConfirmDialog } from "@repo/ui";
import type { BasicFormState as ActionState } from "@repo/contracts";
import { useApiAction } from "@/lib/use-api-action";
import Link from "next/link";
import resourceStyles from "@/components/resource-layout.module.css";

const INITIAL: ActionState = { ok: false };

/**
 * Detail-page action row for one user: 활성/비활성 토글 + 삭제. 본인 계정은 두 파괴적
 * 액션(자기 비활성화·자기 삭제)이 서버에서 막히므로, 클라이언트에서도 비활성화·삭제
 * 버튼을 비활성 처리한다(활성화는 자신에게 해당 없음 — 로그인=활성).
 */
export function UserDetailActions({
  id,
  targetName = id,
  isSelf,
  enabled,
}: {
  id: string;
  targetName?: string;
  isSelf: boolean;
  enabled: boolean;
}) {
  const [toggleState, toggleAction, togglePending] = useActionState(
    useApiAction<ActionState>({ id: "admin.users.toggle", args: [] }),
    INITIAL,
  );
  const [deleteState, deleteAction, deletePending] = useActionState(
    useApiAction<ActionState>({ id: "admin.users.delete", args: [] }),
    INITIAL,
  );

  const deleteForm = useRef<HTMLFormElement>(null);
  const approvedId = useRef<string | null>(null);
  const [confirmId, setConfirmId] = useState<string | null>(null);

  return (
    <div className={resourceStyles.detailActions}>
      <div className={resourceStyles.actionRow}>
        {isSelf && enabled ? (
          <Button
            type="button"
            variant="secondary"
            disabled
            title="본인 계정은 비활성화할 수 없습니다."
          >
            비활성화
          </Button>
        ) : (
          <form action={toggleAction}>
            <input type="hidden" name="id" value={id} />
            <Button type="submit" variant="secondary" disabled={togglePending}>
              {enabled ? "비활성화" : "활성화"}
            </Button>
          </form>
        )}
        {isSelf ? (
          <Button
            type="button"
            variant="dangerOutline"
            disabled
            title="본인 계정은 삭제할 수 없습니다."
          >
            삭제
          </Button>
        ) : (
          <form ref={deleteForm} action={deleteAction} onSubmit={(event) => {
          if (deletePending || isSelf || approvedId.current !== id) {
            event.preventDefault();
            if (!(deletePending || isSelf)) setConfirmId(id);
          }
          approvedId.current = null;
        }}>
            <input type="hidden" name="id" value={id} />
            <Button
              type="submit"
              variant="dangerOutline"
              disabled={deletePending}
            >
              삭제
            </Button>
          </form>
        )}
        <Link
          href={`/admin/users/${id}/edit`}
          className={buttonClassName({ variant: "secondary" })}
        >
          수정
        </Link>
      </div>
      <ConfirmDialog
        open={confirmId === id && !isSelf}
        onClose={() => setConfirmId(null)}
        title="삭제"
        description="이 사용자를 삭제하시겠습니까?"
        target={targetName}
        confirmLabel="삭제"
        pending={deletePending || isSelf}
        onConfirm={() => {
          if (confirmId !== id || deletePending || isSelf) return;
          approvedId.current = id;
          setConfirmId(null);
          deleteForm.current?.requestSubmit();
        }}
      />
      {toggleState.error ? (
        <Banner tone="error" role="alert" title={toggleState.error} />
      ) : null}
      {deleteState.error ? (
        <Banner tone="error" role="alert" title={deleteState.error} />
      ) : null}
    </div>
  );
}
