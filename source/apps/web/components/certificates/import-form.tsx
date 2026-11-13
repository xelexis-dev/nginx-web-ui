"use client";

import {
  buttonClassName,
  linkClassName,
  TextArea,
  Field,
  Banner,
  Card,
  formFieldsClassName,
  FormBar,
  FormStack,
} from "@repo/ui";

import { useActionState, useRef, useState, type ReactNode } from "react";
import Link from "next/link";
import { Button, FileInput } from "@repo/ui";
// 이 폼이 `NodeErrorCard` 를 들여오는 것은 **의도한 것이다** — 로컬·노드가 같은
// 컴포넌트를 쓰는 것이 이 계획의 방식이라, 노드 액션이 넘어올 자리를 로컬 화면이
// 그리는 폼도 그대로 갖고 있어야 한다(`certificate-form.tsx` 헤더와 같은 판단).
import { NodeErrorCard } from "@/components/node-error";
import type { NodeActionState, NodeFormAction } from "@/lib/node-action-state";
import { useApiAction } from "@/lib/use-api-action";
// 라우트 그룹의 시트를 그대로 들여온다 — 왜 복사하지 않는지는
// `certificate-list.tsx` 헤더(요약: `composes` 가 걸려 있고 로컬에 남는 형제
// 화면들이 같은 클래스를 쓴다 — 옮길 수 없다).
import {
  targetLabel,
  type OperationsTarget,
} from "@repo/presentation/operations-target";
import styles from "@/app/(app)/certificates/certificates.module.css";

const INITIAL: NodeActionState = { ok: false };
const ERROR_FIELDS: Record<string, string> = {
  certPem: "cert-pem",
  keyPem: "key-pem",
};

/**
 * 수동 PEM 가져오기/교체 폼. 텍스트 붙여넣기 + 파일 선택(클라이언트에서 읽어
 * textarea에 채움). `certId`를 주입하면 교체 플로우(hidden id)로 재사용된다.
 *
 * **네 화면이 같은 것을 렌더한다** — 로컬 가져오기(`(app)/certificates/import`)·
 * 로컬 교체(`(app)/certificates/[id]/replace`)·노드 가져오기·노드 교체. 다른
 * 것은 넘기는 액션과 두 경로뿐이다.
 *
 * ── ⚠ 이 폼이 나르는 것은 **개인키**다 ──────────────────────────────────────
 *
 * 다른 인증서 화면들은 「비밀값은 노드에 남는다」 방향인데(Cloudflare 토큰은
 * `hasDnsToken: boolean` 으로만 올라온다) **이 화면만 반대 방향**이다: 운영자가
 * 붙여넣은 키가 게이트웨이를 지나 노드로 간다. 그래서 게이트웨이 쪽에 **머무는
 * 자리를 만들지 않는 것**이 이 화면의 불변식이다.
 *
 * - **입력은 클라이언트 상태에만 있다.** 파일 선택도 브라우저의 `FileReader` 가
 *   읽어 textarea 를 채울 뿐, 파일이 서버로 올라가지 않는다.
 * - **폼이 서버로 보내는 것은 제출 한 번뿐이고**, 그걸 받는 노드 액션은
 *   Rust API 중계 계층의 감사에도 로그에도 PEM 을 싣지 않는다.
 * - **되돌아오는 오류에 PEM 이 실리지 않는다.** 노드의 거절 문구는 모양에 대한
 *   것뿐이다(Rust `certificates/pem.rs` 의 `parse_certificate_pem` — 「개인키 PEM을 해석할 수
 *   없습니다」 등). 그 문구를 여기서 다시 손대지 않는다.
 * - **실패해도 입력은 화면에 남는다**(state 가 살아 있다) — 서버가 값을
 *   되돌려주기 때문이 아니다. 되돌려주는 설계로 바꾸지 마라: 그 순간 키가
 *   게이트웨이의 API 응답에 실린다.
 */
export function ImportForm({
  info,
  action,
  certId,
  cancelHref,
  target,
}: {
  info?: ReactNode;
  /**
   * 로컬 액션도 노드 액션도 그대로 들어간다 — `NodeFormAction` 이 기존
   * `ActionState` 액션의 상위 계약이다(그 타입의 문서). **`ActionState` 로
   * 되돌리지 마라**: 컴파일은 통과하는데 아래 `state.nodeError` 가 타입에서
   * 사라져 §9 카드가 영영 안 그려진다.
   *
   * **기본값을 두지 않는다.** 추출 전에는 `importCertificate` 가 기본값이었지만
   * 공유 컴포넌트는 라우트 그룹의 TS 를 가져오지 않는다(`certificate-list.tsx`
   * 헤더의 불변식) — 네 화면이 각자 자기 액션을 명시한다.
   */
  action: NodeFormAction;
  /**
   * 있으면 **교체**(그 인증서를 새 PEM 으로 덮는다), 없으면 **가져오기**(새로
   * 등록). 버튼 문구와 hidden `id` 가 여기서 갈린다.
   */
  certId?: string;
  /** 「취소」가 갈 곳 — 로컬/노드의 인증서 목록. */
  cancelHref: string;
  target?: OperationsTarget;
}) {
  const [state, formAction, pending] = useActionState(
    useApiAction<NodeActionState>(action),
    INITIAL,
  );
  const [certPem, setCertPem] = useState("");
  const [keyPem, setKeyPem] = useState("");
  const certFile = useRef<HTMLInputElement>(null);
  const keyFile = useRef<HTMLInputElement>(null);

  function readInto(file: File | undefined, set: (v: string) => void) {
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => set(String(reader.result ?? ""));
    reader.readAsText(file);
  }

  return (
    <form action={formAction} className={styles.form}>
      {/* **둘은 배타적이다** — `certificate-form.tsx`·`upstream-form.tsx` 와 같은
            갈래다. `runNodeWrite` 는 실패 문구를 `error` 에 **항상** 싣고 카드가
            필요한 실패에만 `nodeError` 를 더하므로, 두 갈래를 나란히 그리면 같은
            문장이 카드 위에 한 번 더 나온다(실측에서 실제로 그랬다).

            어느 쪽으로 오는가:
            - **PEM 거절은 한 줄이다.** 노드는 모양 오류를 필드별(`certPem`/`keyPem`)로
              주기도 하고(DTO), 실제 해석 실패는 필드 없는 한 줄로 주기도 한다
              (`api/v1/certificates/import/route.ts` — `importCertificateRow` 의
              실패). 둘 다 `kind: "validation"` 이라 `runNodeWrite` 가 `nodeError` 를
              비운 채 돌려준다 — 「PEM 이 잘못됐다」는 §9 카드로 그릴 사건이 아니다.
            - **「닿지 못함」·「버전 불일치」·「교체 후 적용 실패」는 카드/배너다** —
              어느 쪽을 올려야 하는지, 다시 연결하려면 어디로 가야 하는지를 그
              카드만 말해준다.

            로컬 액션은 `nodeError` 를 채우는 일이 없어 지금까지와 같은 한 줄로
            그대로 떨어진다(동작 무변화). */}
      {state.nodeError ? (
        <NodeErrorCard {...state.nodeError} />
      ) : state.error || Object.keys(state.fieldErrors ?? {}).length > 0 ? (
        <Banner tone="error" role="alert" title={state.error}>
          {Object.entries(state.fieldErrors ?? {}).map(([name, message]) => (
            <div key={name}>
              <a
                className={linkClassName()}
                href={`#${ERROR_FIELDS[name] ?? "certificate-pem-heading"}`}
              >
                {message}
              </a>
            </div>
          ))}
        </Banner>
      ) : null}

      <FormStack>
        {info ? <Banner tone="info" title={info} /> : null}
        <Card
          id="certificate-pem"
          title="인증서 (PEM)"
          errorCount={
            Object.values(state.fieldErrors ?? {}).filter(Boolean).length
          }
        >
          <div className={formFieldsClassName({ columns: 2 })}>
            {certId ? <input type="hidden" name="id" value={certId} /> : null}
            <input type="hidden" name="certPem" value={certPem} />
            <input type="hidden" name="keyPem" value={keyPem} />

            {/* The text area is the labelled control; the file picker beside it fills it and names itself. */}
            <Field
              label="인증서 (fullchain PEM)"
              htmlFor="cert-pem"
              error={state.fieldErrors?.certPem}
              wide
            >
              {(control) => (
                <>
                  <TextArea
                    mono
                    {...control}
                    value={certPem}
                    onChange={(e) => setCertPem(e.target.value)}
                    placeholder={"-----BEGIN CERTIFICATE-----\n..."}
                  />
                  <FileInput
                    ref={certFile}
                    accept=".pem,.crt,.cer,.txt"
                    aria-label="인증서 파일 선택"
                    onChange={(e) => readInto(e.target.files?.[0], setCertPem)}
                  />
                </>
              )}
            </Field>

            <Field
              label="개인키 (PEM)"
              htmlFor="key-pem"
              error={state.fieldErrors?.keyPem}
              wide
            >
              {(control) => (
                <>
                  <TextArea
                    mono
                    {...control}
                    value={keyPem}
                    onChange={(e) => setKeyPem(e.target.value)}
                    placeholder={"-----BEGIN PRIVATE KEY-----\n..."}
                  />
                  <FileInput
                    ref={keyFile}
                    accept=".pem,.key,.txt"
                    aria-label="개인키 파일 선택"
                    onChange={(e) => readInto(e.target.files?.[0], setKeyPem)}
                  />
                </>
              )}
            </Field>
          </div>
        </Card>
      </FormStack>
      <FormBar
        note={
          target?.kind === "node"
            ? `저장 대상: ${targetLabel(target)}`
            : undefined
        }
      >
        <Link
          href={cancelHref}
          className={buttonClassName({ variant: "secondary" })}
        >
          취소
        </Link>
        <Button type="submit" disabled={pending}>
          {pending
            ? certId
              ? "교체하는 중..."
              : "가져오는 중..."
            : certId
              ? "교체"
              : "가져오기"}
        </Button>
      </FormBar>
    </form>
  );
}
