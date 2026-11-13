"use client";

import { useActionState, useEffect, useState } from "react";
import { useApiAction } from "@/lib/use-api-action";
import { Banner, Button, StatusBadge, Table, Td, Th, Tr } from "@repo/ui";
// 로컬·노드가 같은 패널을 쓰므로 §9 카드가 이 안에 있어야 한다 —
// `components/upstreams/upstream-form.tsx` 헤더와 같은 판단.
import { NodeErrorCard } from "@/components/node-error";
import { CancelIssuanceButton } from "./cancel-button";
import type {
  DnsChallengeAction,
  DnsChallengeRecordView,
  DnsChallengeState,
} from "./certificate-vm";
// 라우트 그룹의 시트를 그대로 들여온다 — 이유는 `certificate-list.tsx` 헤더.
import styles from "@/app/(app)/certificates/certificates.module.css";

const INITIAL: DnsChallengeState = { ok: false };

/** 값이 길어서 눈으로 옮기기 어렵다 — 복사가 사실상 기본 동작이다. */
function CopyValueCell({ value }: { value: string }) {
  const [copied, setCopied] = useState<"idle" | "done" | "failed">("idle");
  // 실패는 다음 성공까지 남긴다 — http 접속에서는 매번 실패하므로 안내가 사라지면 안 된다.
  useEffect(() => {
    if (copied !== "done") return;
    const t = window.setTimeout(() => setCopied("idle"), 2000);
    return () => window.clearTimeout(t);
  }, [copied]);
  return (
    <>
      <div className={styles.dnsValueCell}>
        <code className={styles.dnsCode}>{value}</code>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          onClick={() => {
            const done = navigator.clipboard?.writeText(value);
            if (!done) setCopied("failed");
            else
              done.then(
                () => setCopied("done"),
                () => setCopied("failed"),
              );
          }}
        >
          {copied === "done"
            ? "복사됨"
            : copied === "failed"
              ? "복사할 수 없음"
              : "복사"}
        </Button>
      </div>
      <p
        className={styles.dnsCopyFailed}
        role="status"
        hidden={copied !== "failed"}
      >
        브라우저가 클립보드를 사용할 수 없습니다. TXT 값을 직접 선택해
        복사하세요.
      </p>
    </>
  );
}

/**
 * DNS-01 대기 패널: 넣어야 할 TXT를 보여주고, 사용자가 추가한 뒤 검증을 시작한다.
 * `mode="renew"`는 이미 서빙 중인 인증서의 갱신이라 되돌릴 수 있어야 한다(취소).
 *
 * **로컬 상세와 노드 상세가 같은 것을 렌더한다** — 다른 것은 넘기는 액션과
 * 중단 경로뿐이다. 이 패널이 뜨는 단계(`awaiting-txt`)를 정하는 것은 상세
 * 화면이고, 그 판정은 `dnsOrder.submittedAt`(노드는 `dnsSubmittedAt`) 하나다.
 */
export function DnsChallengePanel({
  id,
  submitAction,
  cancelPath,
  records,
  mode,
  canWrite,
}: {
  id: string;
  /**
   * 「전파 확인 후 계속」. 로컬 액션도 노드 액션도 그대로 들어간다
   * (`DnsChallengeState` 문서). **`notPropagated` 는 실패가 아니다** — 노드가
   * 200 + 플래그로 낸다.
   */
  submitAction: DnsChallengeAction;
  /**
   * 중단 버튼이 `POST` 할 경로. **`id` 로 조립하지 않는다** — 노드 상세는 중계
   * 라우트를 넘긴다(`CancelIssuanceButton` 문서).
   */
  cancelPath: string;
  records: DnsChallengeRecordView[];
  mode: "initial" | "renew";
  /**
   * false면 표만 보여주고 버튼은 감춘다. TXT 값은 config:read가 봐도 되는 공개값이고
   * 무엇이 대기 중인지 아는 건 유용하지만, 눌러도 서버 액션이 거부하며
   * 로그인으로 리다이렉트되고 감사 로그에 거짓 rbac:denied만 남는다.
   */
  canWrite: boolean;
}) {
  const [submitState, formAction, submitPending] = useActionState(
    useApiAction(submitAction),
    INITIAL,
  );

  return (
    <section
      className={styles.panel}
      aria-label={
        mode === "renew"
          ? "갱신 진행 중 — DNS TXT 레코드 추가 필요"
          : "DNS TXT 레코드 추가 필요"
      }
    >
      <header className={styles.panelHead}>
        <h2>
          {mode === "renew"
            ? "갱신 진행 중 — DNS TXT 레코드 추가 필요"
            : "DNS TXT 레코드 추가 필요"}
        </h2>
        <StatusBadge tone="warning">조치 필요</StatusBadge>
      </header>
      <div className={`${styles.stack} ${styles.panelBody}`}>
        <p className={styles.dnsPanelBody}>
          아래 레코드를 DNS 공급자에 추가한 뒤{" "}
          <strong>전파 확인 후 계속</strong>을 누르세요. 전파에는 보통 수십
          초에서 몇 분이 걸립니다. 전파가 확인되기 전에는 인증기관에 검증을
          요청하지 않으므로 일찍 눌러도 실패로 소모되지 않습니다.
          {mode === "renew"
            ? " 갱신이 끝날 때까지 기존 인증서가 계속 서빙됩니다."
            : ""}
        </p>

        <Table label="추가할 DNS TXT 레코드">
          <thead>
            <tr>
              <Th>이름 (host)</Th>
              <Th>타입</Th>
              <Th>값</Th>
            </tr>
          </thead>
          <tbody>
            {records.map((record) => (
              <Tr key={record.domain}>
                <Td cell="primary" mono>
                  {record.recordName}
                </Td>
                <Td label="타입">TXT</Td>
                <Td label="값" wrap>
                  <CopyValueCell value={record.recordValue} />
                </Td>
              </Tr>
            ))}
          </tbody>
        </Table>

        {/* **세 갈래가 따로인 것이 요점이다.**
            - `nodeError` — 노드에 닿지 못했다·버전이 안 맞는다 → §9 카드.
            - `notPropagated` — **오류가 아니다.** 노드가 **200 + 플래그**로 낸다
              (`api/v1/certificates/[id]/dns-challenge/route.ts`: 「TXT 가 아직 안
              보이는 것은 더 기다리라는 뜻이다」). 아무것도 실패하지 않았고 CA 를
              부르지도 않았으므로 빨간 오류가 아니라 **경고 배너**로 그리고, 어느
              도메인이 아직인지 함께 보여준다. 이 갈래는 추출 전 로컬이 다른 실패와
              같은 빨간 한 줄로 그리던 자리다 — 계획이 「오류로 그리지 마라」로
              바로잡았다(의도된 변경).
            - 그 밖 — 진짜 실패라 빨간 한 줄. */}
        {submitState.nodeError ? (
          <NodeErrorCard {...submitState.nodeError} />
        ) : submitState.notPropagated ? (
          <Banner
            tone="warning"
            title="아직 전파되지 않았습니다 — 실패가 아닙니다"
          >
            {submitState.error}
          </Banner>
        ) : submitState.error ? (
          <Banner tone="error" role="alert" title={submitState.error} />
        ) : null}
        {canWrite ? (
          <div className={styles.actionsRow}>
            <form action={formAction}>
              <input type="hidden" name="id" value={id} />
              <Button type="submit" disabled={submitPending}>
                {submitPending ? "확인 중..." : "전파 확인 후 계속"}
              </Button>
            </form>
            {/* 초기 발급도 여기서 접을 수 있다 — 이전에는 삭제 말고는 빠져나갈 길이 없었다.
                갱신 중단은 주문만 버리고 서빙 중인 인증서를 그대로 둔다. */}
            <CancelIssuanceButton
              cancelPath={cancelPath}
              label={mode === "renew" ? "갱신 취소" : "발급 중단"}
            />
          </div>
        ) : (
          <p className={styles.hint}>
            이 레코드를 추가하고 발급을 이어가려면 설정 쓰기 권한이 필요합니다.
          </p>
        )}
      </div>
    </section>
  );
}
