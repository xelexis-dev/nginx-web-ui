"use client";

import { formatServerTime } from "@/lib/format-time";

import { startTransition, useActionState, useEffect, useRef, useState } from "react";
import { Banner, Button, ConfirmDialog, Icon } from "@repo/ui";
import { NodeErrorCard } from "@/components/node-error";
import type { NodeResult } from "@/lib/node-result";
import historyStyles from "@/components/history/history.module.css";
import { useApiAction } from "@/lib/use-api-action";
interface RollbackResponse { output: string }

type RollbackState = NodeResult<RollbackResponse> | { ok: false; error: string } | null;

function fallbackLabel(iso: string): string {
  return formatServerTime(new Date(iso));
}

interface NodeRollbackProps {
  nodeId: string;
  nodeName: string;
  snapshotId: string;
  label: string | null;
  iso: string;
  retryHref: string;
  listHref: string;
}

export function NodeRollbackButton({ nodeId, nodeName, snapshotId, label, iso, retryHref, listHref }: NodeRollbackProps) {
  const rollbackAction = useApiAction<RollbackState>({ id: "node.rollbackNodeSnapshot", args: [nodeId, snapshotId] });

  const [result, formAction, pending] = useActionState<RollbackState, FormData>(rollbackAction, null);

  const formRef = useRef<HTMLFormElement>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  useEffect(() => { if (result !== null) setConfirmOpen(false); }, [result]);

  const snapshotLabel = label ?? fallbackLabel(iso);
  const confirmText = `"${nodeName}" 을(를) "${snapshotLabel}" 스냅샷으로 되돌립니다. 현재 설정을 덮어쓰며 되돌릴 수 없습니다.`;

  return (
    <div className={historyStyles.rollbackCell}>
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
        description={confirmText} target={`${nodeName} · ${snapshotLabel}`} confirmLabel="이 스냅샷으로 롤백" pending={pending}
        onConfirm={() => { if (formRef.current && !pending) startTransition(() => formAction(new FormData(formRef.current!))); }} />

      {result && (result.ok ? (
        <Banner tone="success" role="status" title="롤백 완료 — nginx -t 통과, reload 완료" output={result.value.output} />
      ) : (
        <NodeErrorCard
          failure={"kind" in result ? result : { kind: "node_error", message: result.error }}
          nodeId={nodeId}
          retryHref={retryHref}
          listHref={listHref}
          subject="그 스냅샷을"
        />
      ))}
    </div>
  );
}
