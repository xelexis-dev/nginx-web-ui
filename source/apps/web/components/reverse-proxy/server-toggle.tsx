"use client";

import { useActionState, useState } from "react";
import { Button, Modal, Switch } from "@repo/ui";
import { NodeErrorCard } from "@/components/node-error";
import type { NodeActionState, NodeFormAction } from "@/lib/node-action-state";
import { useApiAction } from "@/lib/use-api-action";
import styles from "@/app/(app)/reverse-proxy/reverse-proxy.module.css";

const INITIAL: NodeActionState = { ok: false };

/**
 * 목록 한 줄의 활성 스위치. **상세 화면의 「활성화/비활성화」와 같은 명령**을 같은 방식(폼 + `id`)으로
 * 보낸다 — 로컬은 `reverse-proxy.toggleServer`, 노드는 `node.toggleNodeServer`(페이지가 넘긴다).
 * 성공하면 `useApiAction` 이 화면을 새로 읽어 스위치·미적용 표시·셸의 미적용 수가 실제 값으로 바뀐다.
 * 스위치는 **저장만** 한다 — nginx 반영은 「설정 적용」이다(상세와 같다).
 *
 * 실패는 행 안에 한 줄로 말한다. 노드 실패(`nodeError`)는 종류마다 할 일이 다르므로(버전 불일치면
 * 다시 연결, 인증 실패면 토큰, nginx -t 실패면 출력) 한 줄로 뭉개지 않고 「자세히」가 같은
 * `NodeErrorCard` 를 대화상자로 연다 — 표 안에 카드를 욱여넣지 않으면서 그 내용을 잃지 않는다.
 */
export function ServerToggle({
  id,
  serverName,
  enabled,
  action,
}: {
  id: string;
  serverName: string;
  enabled: boolean;
  action: NodeFormAction;
}) {
  const [state, formAction, pending] = useActionState(useApiAction<NodeActionState>(action), INITIAL);
  const [detailOpen, setDetailOpen] = useState(false);
  const summary = state.ok ? null : (state.error ?? state.nodeError?.failure.message ?? null);
  return (
    <form action={formAction} className={styles.rowToggle}>
      <input type="hidden" name="id" value={id} />
      <Switch size="sm" type="submit" checked={enabled} label={`${serverName} 활성`} disabled={pending} aria-busy={pending || undefined} />
      {summary ? (
        <div className={styles.rowToggleError} role="alert">
          <span>{summary}</span>
          {state.nodeError ? (
            <Button type="button" variant="ghost" size="sm" onClick={() => setDetailOpen(true)}>
              자세히
            </Button>
          ) : null}
        </div>
      ) : null}
      {state.nodeError ? (
        <Modal open={detailOpen} onClose={() => setDetailOpen(false)} title={`${serverName} — 반영하지 못했습니다`}>
          <NodeErrorCard {...state.nodeError} />
        </Modal>
      ) : null}
    </form>
  );
}
