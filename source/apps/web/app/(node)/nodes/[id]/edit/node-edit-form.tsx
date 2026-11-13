"use client";

import { useActionState, useState, type ReactNode } from "react";
import Link from "next/link";
import {
  TextInput,
  Button,
  Checkbox,
  Field,
  Card,
  FormStack,
  FormBar,
  linkClassName,
  Banner,
  Facts,
  formFieldsClassName,
  buttonClassName,
} from "@repo/ui";
import { targetLabel } from "@repo/presentation/operations-target";
import type { Role } from "@repo/contracts";
import type { ActionState } from "../../../../(app)/nodes/types";
import { useApiAction } from "@/lib/use-api-action";
import styles from "../../../../(app)/nodes/nodes.module.css";

const INITIAL: ActionState = { ok: false };
const FIELD_IDS: Record<string, string> = {
  name: "node-name",
  baseUrl: "node-base-url",
  allowInsecureTls: "node-allow-insecure-tls",
};

// A stable native ref survives action commits (which suppress React events) and cleans up on detach.
function preserveEditForm(form: HTMLFormElement | null) {
  if (!form) return;
  form.onreset = (event) => event.preventDefault();
  return () => {
    form.onreset = null;
  };
}

export interface NodeEditFormProps {
  id: string;
  /** A separate destructive form must stay outside the settings form. */
  unpair?: ReactNode;
  defaults: {
    name: string;
    baseUrl: string;
    allowInsecureTls: boolean;
  };
  /** `nodes.tokenRole` — 노드가 발급한 값. 이 폼으로는 바꿀 수 없다(읽기 전용으로만 보여준다). */
  tokenRole: Role;
  /**
   * `nodes.gatewayName` — 페어링 때 노드에 등록한 「이 게이트웨이의 이름」.
   * 이 컬럼이 생기기(0022 마이그레이션) 전에 페어링된 행은 `null` 이다.
   */
  registeredGatewayName: string | null;
}

/**
 * `/nodes/[id]/edit` 의 수정 폼: 이름 / 주소 / 자체서명 허용, 셋뿐이다.
 *
 * **`node-form.tsx` 를 재사용하지 않는다.** 그 컴포넌트는 `pairNode` 에 묶여 있고
 * 노드 관리자 이메일·비밀번호·TOTP 필드를 품고 있다 — 갈 곳이 없는 비밀값 입력칸을
 * 이 화면에 들이지 않기 위해 별도로 만들었다.
 *
 * 토큰 등급과 「이 게이트웨이의 이름」은 이 폼이 제출하는 값에 들어가지 않는다
 * (읽기 전용 안내일 뿐이다) — `updateNode` 서버 액션도 `name`/`baseUrl`/
 * `allowInsecureTls` 세 필드만 읽는다(`../../form.ts` 의 `parseNodeUpdateForm`).
 */
export function NodeEditForm({
  id,
  defaults,
  tokenRole,
  registeredGatewayName,
  unpair,
}: NodeEditFormProps) {
  const [state, formAction, pending] = useActionState(
    useApiAction<ActionState>({ id: "nodes.updateNode", args: [] }),
    INITIAL,
  );

  const [name, setName] = useState(defaults.name);
  const [baseUrl, setBaseUrl] = useState(defaults.baseUrl);
  const [allowInsecureTls, setAllowInsecureTls] = useState(
    defaults.allowInsecureTls,
  );

  return (
    <>
      <form
        id="node-edit-form"
        action={formAction}
        ref={preserveEditForm}
        className={styles.formStack}
      >
        <input type="hidden" name="id" value={id} />

        {state.error ? (
          <Banner tone="error" role="alert" title={state.error}>
            {Object.entries(state.fieldErrors ?? {}).map(([field, message]) => (
              <div key={field}>
                <a
                  className={linkClassName()}
                  href={`#${FIELD_IDS[field] ?? "node-connection-heading"}`}
                >
                  {message}
                </a>
              </div>
            ))}
          </Banner>
        ) : null}
        <FormStack>
          {/* An error takes the hint's place, as before. */}
          <Card
            id="node-connection"
            title="연결 설정"
            errorCount={Object.keys(state.fieldErrors ?? {}).length}
          >
            <div className={formFieldsClassName()} data-rollout-form-grid>
              <Field
                label="이름"
                htmlFor="node-name"
                hint={
                  state.fieldErrors?.name
                    ? undefined
                    : "이 게이트웨이의 목록에서 이 노드를 부를 이름입니다."
                }
                error={state.fieldErrors?.name}
              >
                <TextInput
                  name="name"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  maxLength={100}
                  required
                />
              </Field>

              <Field
                label="주소"
                htmlFor="node-base-url"
                hint={
                  state.fieldErrors?.baseUrl
                    ? undefined
                    : "노드가 이사했다면 여기서 바꾸세요. 스킴(http:// 또는 https://)을 포함합니다."
                }
                error={state.fieldErrors?.baseUrl}
              >
                <TextInput
                  name="baseUrl"
                  value={baseUrl}
                  onChange={(event) => setBaseUrl(event.target.value)}
                  placeholder="https://node.example.com"
                  required
                />
              </Field>

              <Checkbox
                style={{ gridColumn: "1 / -1" }}
                id="node-allow-insecure-tls"
                name="allowInsecureTls"
                value="true"
                checked={allowInsecureTls}
                onChange={(event) => setAllowInsecureTls(event.target.checked)}
                label="자체 서명 인증서 허용 (https 인증서를 검증하지 않음)"
              />
            </div>
          </Card>
          <Card id="node-issued" title="토큰 등급">
            {/* Values the node issued: shown, not edited — facts, each with why it cannot change here. */}
            <div data-wide>
              <Facts
                columns={2}
                items={[
                  {
                    term: "토큰 등급",
                    value: (
                      <>
                        {tokenRole}
                        <p className={styles.hint}>
                          노드가 발급한 값이라 여기서 바꿀 수 없습니다. 등급을
                          바꾸려면 연결을 해제하고 다시 연결하세요.
                        </p>
                      </>
                    ),
                  },
                  {
                    term: "이 게이트웨이의 이름",
                    value: (
                      <>
                        {registeredGatewayName ??
                          "알 수 없음 — 다시 연결하면 기록됩니다."}
                        <p className={styles.hint}>
                          노드에 등록된 이름입니다. 바꾸려면 다시 연결하세요.
                        </p>
                      </>
                    ),
                  },
                ]}
              />
            </div>
          </Card>
        </FormStack>
      </form>
      {unpair}
      <FormBar
        note={
          <>
            저장 대상: {targetLabel({ kind: "node", id, name: defaults.name })}
          </>
        }
      >
        <Link
          href={`/nodes/${id}`}
          className={buttonClassName({ variant: "secondary" })}
        >
          취소
        </Link>
        <Button type="submit" form="node-edit-form" disabled={pending}>
          {pending ? "저장하는 중..." : "저장"}
        </Button>
      </FormBar>
    </>
  );
}
