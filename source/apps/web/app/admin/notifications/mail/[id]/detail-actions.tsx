"use client";

import { useActionState } from "react";
import { Banner, Button, buttonClassName } from "@repo/ui";
import { useApiAction } from "@/lib/use-api-action";
import type { MailRecipientState } from "../form-types";
import Link from "next/link";
import resourceStyles from "@/components/resource-layout.module.css";

const INITIAL: MailRecipientState = { ok: false };

/** 상세 페이지의 액션 줄: 테스트 발송 + 삭제. webhook 상세와 같은 모양이다. */
export function MailRecipientDetailActions({
  id,
  address,
}: {
  id: string;
  address: string;
}) {
  const [testState, testAction, testPending] = useActionState(
    useApiAction<MailRecipientState>({
      id: "admin.mailRecipients.test",
      args: [],
    }),
    INITIAL,
  );
  const [deleteState, deleteAction, deletePending] = useActionState(
    useApiAction<MailRecipientState>({
      id: "admin.mailRecipients.delete",
      args: [],
    }),
    INITIAL,
  );

  return (
    <div className={resourceStyles.detailActions}>
      <div className={resourceStyles.actionRow}>
        <form action={testAction}>
          <input type="hidden" name="address" value={address} />
          <Button
            type="submit"
            variant="secondary"
            disabled={testPending || deletePending}
          >
            {testPending ? "보내는 중..." : "테스트 발송"}
          </Button>
        </form>
        <form action={deleteAction}>
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
          href={`/admin/notifications/mail/${id}/edit`}
          className={buttonClassName({ variant: "secondary" })}
        >
          수정
        </Link>
      </div>

      {testState.error ? (
        <Banner
          tone={"error"}
          title={<>테스트 발송 실패 — {testState.error}</>}
          role="alert"
          style={{ marginTop: "var(--sp-3)" }}
        ></Banner>
      ) : null}
      {testState.ok && testState.message ? (
        <Banner
          tone={"success"}
          title={<>{testState.message}</>}
          role="status"
          style={{ marginTop: "var(--sp-3)" }}
        ></Banner>
      ) : null}
      {deleteState.error ? (
        <Banner tone="error" role="alert" title={deleteState.error} />
      ) : null}
    </div>
  );
}
