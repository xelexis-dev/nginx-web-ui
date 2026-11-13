"use client";

import { startTransition, useActionState, useEffect, useRef, useState } from "react";
import { Banner, Button, ConfirmDialog, Icon } from "@repo/ui";
import type { RollbackViewResult } from "@/lib/overview-views";
import { useApiAction } from "@/lib/use-api-action";
import styles from "@/components/history/history.module.css";

type RollbackState = RollbackViewResult | null;

/** Browser command bound to the selected snapshot. */
export function RollbackButton({
  snapshotId,
  confirmLabel,
}: {
  snapshotId: string;
  confirmLabel: string;
}) {
  const rollbackAction = useApiAction<RollbackState>({
    id: "history.rollback",
    args: [snapshotId],
  });

  const [result, formAction, pending] = useActionState<RollbackState, FormData>(
    rollbackAction,
    null,
  );

  const formRef = useRef<HTMLFormElement>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  useEffect(() => { if (result !== null) setConfirmOpen(false); }, [result]);

  return (
    <div className={styles.rollbackCell}>
      <form
        action={formAction}
        ref={formRef}
        onSubmit={(event) => { event.preventDefault(); setConfirmOpen(true); }}
      >
        <Button type="submit" variant="secondary" disabled={pending}>
          <Icon name="rotateCcw" size={16} />
          {pending ? "롤백 중..." : "이 스냅샷으로 롤백"}
        </Button>
      </form>
      <ConfirmDialog open={confirmOpen} onClose={() => setConfirmOpen(false)} title="이 스냅샷으로 롤백"
        description={`"${confirmLabel}" 스냅샷으로 롤백하시겠습니까? 현재 설정을 덮어씁니다.`} target={confirmLabel} confirmLabel="이 스냅샷으로 롤백" pending={pending}
        onConfirm={() => { if (formRef.current && !pending) startTransition(() => formAction(new FormData(formRef.current!))); }} />

      {result ? (
        <Banner
          tone={result.ok ? "success" : "error"}
          title={
            <>
              {result.ok
                ? "롤백 완료 — nginx -t 통과, reload 완료"
                : result.error
                  ? "롤백 요청 실패"
                  : result.kind === "db_restore_failed"
                    ? "롤백 실패 — 이전 설정으로 되돌리는 것도 실패했습니다. DB 상태를 확인하세요."
                    : result.kind === "not_found"
                      ? "롤백 실패 — 스냅샷을 찾을 수 없습니다."
                      : "롤백 실패 — 이전 설정을 유지합니다."}
            </>
          }
          role="status"
          output={result.output}
        >
          {result.error ? <p>{result.error}</p> : null}
        </Banner>
      ) : null}
    </div>
  );
}
