"use client";

import { useActionState, useRef, useState } from "react";
import { Banner, Button, buttonClassName, ConfirmDialog } from "@repo/ui";
import type { WebhookFormat } from "@repo/presentation/webhook";
import { useApiAction } from "@/lib/use-api-action";
import type { WebhookFormState } from "../form-types";
import Link from "next/link";
import resourceStyles from "@/components/resource-layout.module.css";

const INITIAL: WebhookFormState = { ok: false };

/** actions.ts 의 KEEP_SECRET 과 같은 값 — 저장된 URL 을 그대로 쓰라는 표식. */
const KEEP_SECRET = "__keep__";

/**
 * 상세 페이지의 액션 줄: 테스트 전송 + 삭제.
 *
 * 테스트는 저장된 값(URL·헤더)으로 보낸다 — 폼은 그것들을 되받지 못하므로 서버가
 * DB 에서 채운다. 형식과 템플릿만 함께 보내면 된다.
 */
export function WebhookDetailActions({
  id,
  targetName = id,
  format,
  bodyTemplate,
}: {
  id: string;
  targetName?: string;
  format: WebhookFormat;
  bodyTemplate: string | null;
}) {
  const [testState, testAction, testPending] = useActionState(
    useApiAction<WebhookFormState>({ id: "admin.webhooks.test", args: [] }),
    INITIAL,
  );
  const [deleteState, deleteAction, deletePending] = useActionState(
    useApiAction<WebhookFormState>({ id: "admin.webhooks.delete", args: [] }),
    INITIAL,
  );

  const deleteForm = useRef<HTMLFormElement>(null);
  const approvedId = useRef<string | null>(null);
  const [confirmId, setConfirmId] = useState<string | null>(null);

  return (
    <div className={resourceStyles.detailActions}>
      <div className={resourceStyles.actionRow}>
        <form action={testAction}>
          <input type="hidden" name="id" value={id} />
          <input type="hidden" name="format" value={format} />
          <input type="hidden" name="url" value={KEEP_SECRET} />
          {bodyTemplate ? (
            <input type="hidden" name="bodyTemplate" value={bodyTemplate} />
          ) : null}
          <Button
            type="submit"
            variant="secondary"
            disabled={testPending || deletePending}
          >
            {testPending ? "보내는 중..." : "테스트 전송"}
          </Button>
        </form>
        <form ref={deleteForm} action={deleteAction} onSubmit={(event) => {
          if (testPending || deletePending || approvedId.current !== id) {
            event.preventDefault();
            if (!(testPending || deletePending)) setConfirmId(id);
          }
          approvedId.current = null;
        }}>
          <input type="hidden" name="id" value={id} />
          <Button
            type="submit"
            variant="dangerQuiet"
            disabled={testPending || deletePending}
          >
            {deletePending ? "삭제 중..." : "삭제"}
          </Button>
        </form>
        <Link
          href={`/admin/notifications/webhooks/${id}/edit`}
          className={buttonClassName({ variant: "secondary" })}
        >
          수정
        </Link>
      </div>
      <ConfirmDialog
        open={confirmId === id}
        onClose={() => setConfirmId(null)}
        title="삭제"
        description="이 Webhook 수신처를 삭제하시겠습니까?"
        target={targetName}
        confirmLabel="삭제"
        pending={testPending || deletePending}
        onConfirm={() => {
          if (confirmId !== id || testPending || deletePending) return;
          approvedId.current = id;
          setConfirmId(null);
          deleteForm.current?.requestSubmit();
        }}
      />

      {testState.error ? (
        <Banner
          tone={"error"}
          title={<>테스트 전송 실패 — {testState.error}</>}
          role="alert"
        ></Banner>
      ) : null}
      {testState.ok && testState.message ? (
        <Banner
          tone={"success"}
          title={<>{testState.message}</>}
          role="status"
        ></Banner>
      ) : null}
      {deleteState.error ? (
        <Banner tone="error" role="alert" title={deleteState.error} />
      ) : null}
    </div>
  );
}
