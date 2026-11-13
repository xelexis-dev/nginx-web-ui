"use client";

import Link from "next/link";
import { useActionState, useRef, useState } from "react";
import { Banner, Button, ConfirmDialog, buttonClassName } from "@repo/ui";
// 로컬·노드가 같은 액션 행을 쓰므로 §9 카드가 이 안에 있어야 한다 —
// `components/upstreams/upstream-form.tsx` 헤더와 같은 판단.
import { NodeErrorCard } from "@/components/node-error";
import type { NodeActionState, NodeFormAction } from "@/lib/node-action-state";
import { useApiAction } from "@/lib/use-api-action";
import { CancelIssuanceButton } from "./cancel-button";
// 라우트 그룹의 시트를 그대로 들여온다 — 이유는 `certificate-list.tsx` 헤더.
import styles from "@/app/(app)/certificates/certificates.module.css";

const INITIAL: NodeActionState = { ok: false };

export interface CertificateDetailActionsProps {
  id: string;
  targetName?: string;
  /**
   * 중단 버튼이 `POST` 할 경로. **여기서 `id` 로 조립하지 않는다** — 노드 상세는
   * 중계 라우트를 넘겨야 하고, 조립하면 노드 화면의 중단이 게이트웨이 자신의
   * 발급을 끊으러 간다(`CancelIssuanceButton` 문서).
   */
  cancelPath: string;
  /** 「설정 수정」이 갈 곳. 로컬/노드의 이 인증서 수정 화면. */
  editHref: string;
  /**
   * 「인증서 교체」(수동 등록 전용)가 갈 곳. `null` 이면 버튼이 사라지고 수동
   * 인증서에는 「삭제」만 남는다.
   *
   * **`boolean` 하나로 대신하지 않는다** — 「그릴까 말까」와 「어디로 보낼까」가
   * 갈라지면 아직 없는 화면에도 버튼이 그려지고 아무 검사도 그걸 못 잡는다
   * (`CertificateListWrite.importHref` 와 같은 규칙). 노드에 교체 화면이 없던
   * 시기에 이 자리를 지켜준 것이 그 규칙이다.
   */
  replaceHref: string | null;
  status: "issuing" | "issued" | "failed" | "awaiting-dns";
  linkedCount: number;
  source: "acme" | "manual";
  challengeType: "http-01" | "dns-01";
  /** DNS-01 TXT 를 provider API 로 자동 등록하는지 — 버튼 문구가 달라진다. */
  dnsAutomated: boolean;
  /** DNS-01 주문이 진행 중이면 갱신을 또 시작하지 않는다(패널이 이미 떠 있다). */
  dnsOrderInFlight: boolean;
  /** DNS-01 주문이 CA 검증 단계로 넘어간 상태 — TXT 패널이 없어 여기서 중단을 제공한다. */
  dnsVerifying: boolean;
  /**
   * 넷 다 로컬 액션도 노드 액션도 그대로 들어간다 — **`ActionState` 로 되돌리지
   * 마라**(`NodeFormAction` 문서). 되돌리면 노드가 죽어도 §9 카드가 안 나온다.
   */
  retryAction: NodeFormAction;
  renewAction: NodeFormAction;
  renewDnsAction: NodeFormAction;
  deleteAction: NodeFormAction;
}

/**
 * 상세 페이지 액션: 다시 발급(failed) / 지금 갱신(issued, reload 수반) / 중단
 * (진행 중일 때) / 삭제(연결 서버 0일 때만). 각 버튼은 독립 form +
 * useActionState — 기존 detail-actions 패턴과 동일. 수동 등록(source ===
 * "manual") 인증서는 ACME 재시도/갱신 대신 "인증서 교체" 링크를 보여준다 —
 * 삭제는 공통.
 *
 * **로컬 상세와 노드 상세가 같은 것을 렌더한다.** 다른 것은 넘기는 액션 넷과
 * 세 경로(`cancelPath`·`editHref`·`replaceHref`)뿐이다.
 */
export function CertificateDetailActions({
  id,
  targetName,
  cancelPath,
  editHref,
  replaceHref,
  status,
  linkedCount,
  source,
  challengeType,
  dnsAutomated,
  dnsOrderInFlight,
  dnsVerifying,
  retryAction,
  renewAction,
  renewDnsAction,
  deleteAction,
}: CertificateDetailActionsProps) {
  const [retryState, retryFormAction, retryPending] = useActionState(
    useApiAction<NodeActionState>(retryAction),
    INITIAL,
  );
  const [renewState, renewFormAction, renewPending] = useActionState(
    useApiAction<NodeActionState>(renewAction),
    INITIAL,
  );
  const [dnsRenewState, dnsRenewFormAction, dnsRenewPending] = useActionState(
    useApiAction<NodeActionState>(renewDnsAction),
    INITIAL,
  );
  const [deleteState, deleteFormAction, deletePending] = useActionState(
    useApiAction<NodeActionState>(deleteAction),
    INITIAL,
  );

  const deleteForm = useRef<HTMLFormElement>(null);
  const approvedId = useRef<string | null>(null);
  const [confirmId, setConfirmId] = useState<string | null>(null);

  return (
    <div className={styles.detailActions}>
      <div className={styles.actionsRow}>
        {source === "manual" ? (
          replaceHref ? (
            <Link
              href={replaceHref}
              className={buttonClassName({ variant: "secondary" })}
            >
              인증서 교체
            </Link>
          ) : null
        ) : (
          <>
            {status === "failed" ? (
              <form action={retryFormAction}>
                <input type="hidden" name="id" value={id} />
                <Button type="submit" disabled={retryPending}>
                  {retryPending ? "요청 중..." : "다시 발급"}
                </Button>
              </form>
            ) : null}
            {status === "issued" && challengeType === "http-01" ? (
              <form action={renewFormAction}>
                <input type="hidden" name="id" value={id} />
                <Button type="submit" disabled={renewPending}>
                  {renewPending ? "갱신 중..." : "지금 갱신"}
                </Button>
              </form>
            ) : null}
            {status === "issued" &&
            challengeType === "dns-01" &&
            !dnsOrderInFlight ? (
              <form action={dnsRenewFormAction}>
                <input type="hidden" name="id" value={id} />
                <Button type="submit" disabled={dnsRenewPending}>
                  {dnsRenewPending
                    ? "갱신 시작 중..."
                    : dnsAutomated
                      ? // 자동화면 사람이 TXT 를 넣을 일이 없으므로 문구에서 TXT 를 뺀다.
                        "지금 갱신 (자동)"
                      : "갱신 (새 TXT 발급)"}
                </Button>
              </form>
            ) : null}
            {/* 진행 중인 작업을 끊는다. HTTP-01 발급/검증은 상태로, "지금 갱신"은
                액션 pending으로 판단한다(갱신 중에는 status가 issued 그대로다). */}
            {status === "issuing" ? (
              <CancelIssuanceButton cancelPath={cancelPath} />
            ) : null}
            {status !== "issuing" && dnsVerifying ? (
              <CancelIssuanceButton cancelPath={cancelPath} label="검증 중단" />
            ) : null}
            {renewPending ? (
              <CancelIssuanceButton cancelPath={cancelPath} label="갱신 중단" />
            ) : null}
          </>
        )}
        <form
          ref={deleteForm}
          action={deleteFormAction}
          onSubmit={(e) => {
            if (linkedCount > 0 || deletePending || approvedId.current !== id) {
              e.preventDefault();
              if (linkedCount === 0 && !deletePending) setConfirmId(id);
            }
            approvedId.current = null;
          }}
        >
          <input type="hidden" name="id" value={id} />
          <Button
            type="submit"
            variant="dangerQuiet"
            disabled={deletePending || linkedCount > 0}
          >
            {linkedCount > 0
              ? "삭제 (연결 해제 필요)"
              : deletePending
                ? "삭제 중..."
                : "삭제"}
          </Button>
        </form>
        {/* 설정 수정은 ID를 유지하므로 연결된 서버가 끊기지 않는다. 발급 중에는
            값이 바뀌면 진행 중인 주문과 어긋나므로 숨긴다(액션도 거부한다). */}
        {source === "acme" && status !== "issuing" ? (
          <Link
            href={editHref}
            className={buttonClassName({ variant: "secondary" })}
          >
            설정 수정
          </Link>
        ) : null}
      </div>
      <ConfirmDialog
        open={confirmId === id && linkedCount === 0}
        onClose={() => setConfirmId(null)}
        title="삭제"
        description="이 인증서를 삭제하시겠습니까?"
        target={targetName ?? id}
        confirmLabel={deletePending ? "삭제 중..." : "삭제"}
        pending={deletePending}
        onConfirm={() => {
          if (confirmId !== id || linkedCount > 0 || deletePending) return;
          approvedId.current = id;
          setConfirmId(null);
          deleteForm.current?.requestSubmit();
        }}
      />
      {renewState.ok ? (
        <Banner
          tone="success"
          title="갱신 완료 — 인증서가 교체되고 nginx가 reload되었습니다."
        />
      ) : null}
      {/* 노드 액션이 낸 「닿지 못함」·「버전 불일치」 등은 빨간 한 줄이 아니라 §9
          카드다 — 어느 쪽을 올려야 하는지, 다시 연결하려면 어디로 가야 하는지를
          그 카드만 말해준다. 로컬 액션은 `nodeError` 를 채우는 일이 없어 지금까지와
          같은 배너로 그대로 떨어진다(동작 무변화). */}
      {[retryState, renewState, dnsRenewState, deleteState].map((s, i) =>
        s.nodeError ? (
          <NodeErrorCard key={i} {...s.nodeError} />
        ) : s.error ? (
          <Banner key={i} tone="error" title={s.error} />
        ) : null,
      )}
    </div>
  );
}
