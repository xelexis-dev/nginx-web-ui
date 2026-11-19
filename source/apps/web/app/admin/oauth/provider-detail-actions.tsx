"use client";

import { useActionState, useRef, useState } from "react";
import Link from "next/link";
import { Banner, Button, ConfirmDialog, buttonClassName } from "@repo/ui";
import { useApiAction } from "@/lib/use-api-action";
import resourceStyles from "@/components/resource-layout.module.css";

export function ProviderDetailActions({ id, targetName = id }: { id: string; targetName?: string }) {
  const [state, action, pending] = useActionState(useApiAction<{ ok: boolean; error?: string }>({ id: "admin.oauth.delete", args: [] }), { ok: false });
  const deleteForm = useRef<HTMLFormElement>(null);
  const approvedId = useRef<string | null>(null);
  const [confirmId, setConfirmId] = useState<string | null>(null);
  return <div className={resourceStyles.detailActions}>
    <div className={resourceStyles.actionRow}>
      <form ref={deleteForm} action={action} aria-busy={pending} onSubmit={(event) => {
        if (pending || approvedId.current !== id) {
          event.preventDefault();
          if (!pending) setConfirmId(id);
        }
        approvedId.current = null;
      }}>
        <input type="hidden" name="id" value={id} />
        <Button type="submit" variant="dangerQuiet" disabled={pending}>공급자 삭제</Button>
      </form>
      <Link href={`/admin/oauth/${id}/edit`} className={buttonClassName({ variant: "secondary" })}>수정</Link>
    </div>
    <ConfirmDialog open={confirmId === id} onClose={() => setConfirmId(null)} title="삭제" description="이 공급자를 삭제하시겠습니까?" target={targetName} confirmLabel="삭제" pending={pending} onConfirm={() => {
      if (confirmId !== id || pending) return;
      approvedId.current = id;
      setConfirmId(null);
      deleteForm.current?.requestSubmit();
    }} />
    {state.error ? <Banner tone="error" role="alert" title={state.error} style={{ marginBlock: "var(--sp-2)" }} /> : null}
  </div>;
}
