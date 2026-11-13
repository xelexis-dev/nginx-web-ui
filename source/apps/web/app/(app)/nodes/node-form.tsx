"use client";

import { useActionState, useEffect, useRef, useState } from "react";
import Link from "next/link";
import {
  TextInput,
  buttonClassName,
  Button,
  Select,
  Banner,
  Checkbox,
  Field,
  Card,
  FormStack,
  FormBar,
  linkClassName,
  formFieldsClassName,
} from "@repo/ui";
import type { ActionState } from "./types";
import { useApiAction } from "@/lib/use-api-action";
import styles from "./nodes.module.css";

const INITIAL: ActionState = { ok: false };
const FIELD_IDS: Record<string, string> = {
  name: "node-name",
  baseUrl: "node-base-url",
  gatewayName: "node-gateway-name",
  role: "node-role",
  token: "node-token",
  allowInsecureTls: "node-allow-insecure-tls",
};

// Keep choices through native action resets, but never keep the one-request pairing credential.
function preservePairingForm(form: HTMLFormElement | null) {
  if (!form) return;
  const clearToken = () => {
    const token = form.querySelector<HTMLInputElement>('input[name="token"]');
    if (token) token.value = "";
  };
  form.onreset = (event) => {
    clearToken();
    event.preventDefault();
  };
  return () => {
    clearToken();
    form.onreset = null;
  };
}

export interface NodeFormProps {
  /**
   * `gatewayName` 필드의 기본값(`appSettings.publicBaseUrl` 의 호스트 부분,
   * `new/page.tsx` 가 `hostOfPublicBaseUrl` 로 계산해 넘긴다). 설정이 비어
   * 있으면 빈 문자열 — 그러면 사용자가 직접 채워야 한다(추측해서 채우지 않는다).
   */
  defaultGatewayName?: string;
}

/**
 * `/nodes/new` 의 페어링 폼: 이름 / 주소 / 이 게이트웨이의 이름 / 토큰 등급 /
 * 자체서명 허용 / **액세스 토큰 한 칸**.
 *
 * **그 토큰은 여기서 만드는 것이 아니다.** 연결할 **그 노드의** 관리자 패널 →
 * 액세스 토큰에서 `node:pair` 권한으로 발급한 서버 토큰을 붙여넣는다. 이
 * 게이트웨이의 `/admin/tokens` 에서 만든 토큰은 **다른 게이트웨이가 우리를 자기
 * 노드로 붙일 때** 쓰는 것이라 여기서는 아무 효과가 없다(그리고 그 값은 하필
 * 우리 서버의 제어권을 넘기는 자격증명이다). 방향은 `api/v1/pair/route.ts:46-52`
 * 가 정본이다 — 그 라우트는 **노드 쪽에서** 돈다.
 *
 * **「이름」과 「이 게이트웨이의 이름」은 반대 방향이다** — 전자는 우리가 이
 * 노드를 부르는 이름(`nodes.name`), 후자는 그 노드가 **우리를** 부를 이름
 * (`gatewayName`, 노드의 「연결된 게이트웨이」 목록·감사 기록에 찍힌다). 하나로
 * 합치면 노드 쪽 감사에 그 노드 자신의 이름을 가진 게이트웨이가 찍히는 사고가
 * 난다 — 반드시 별도 필드로 받는다.
 *
 * 성공하면 `pairNode` 가 새 노드의 상세 페이지로 리다이렉트한다 — 이 컴포넌트는
 * `ok: true` 를 볼 일이 없다. 필드 검증·페어링 실패만 `useActionState` 로
 * 돌아온다.
 *
 * ── 실패해도 입력을 잃지 않는다 ─────────────────────────────────────────────
 *
 * **`<form action={서버액션}>` 은 제출이 끝나면 React 가 폼을 리셋한다.** 그래서
 * 제어되지 않는 입력은 `defaultValue` 가 있든 없든 **전부 비워진다** — 연결이
 * 거부될 때마다 사용자가 이름·주소·게이트웨이 이름을 다시 치게 된다.
 * (예전에는 `baseUrl` 만 살아남았는데, 그건 http 경고 때문에 우연히 제어형이었을
 * 뿐이다.) 그래서 다시 칠 값들을 전부 여기서 들고 있는다.
 *
 * **토큰은 일부러 들고 있지 않는다** — 비밀번호·TOTP 를 들고 있지 않던 것과
 * **같은 논거이고, 토큰에서 더 무겁다.** 이 값은 페어링 요청 한 번에만 쓰이고
 * 어디에도 저장되지 않는다는 것이 이 화면의 약속인데(아래 안내 문구), 상태에
 * 남기면 그 평문이 실패한 뒤에도 브라우저 메모리와 리렌더되는 마크업에 계속
 * 살아 있는다. 서버 쪽에서 닫아 둔 평문 축을 화면에서 되여는 짓이다.
 * 그래서 **제어하지 않는다**(`value`/`onChange` 없음 → 제출 뒤 reset 처리에서 직접 비운다).
 * 대신 **다시 붙여넣어야 한다는 것을 화면이 말해 준다** — 안 그러면 「입력이
 * 지워졌다」는 같은 버그로 읽힌다.
 */
export function NodeForm({ defaultGatewayName = "" }: NodeFormProps) {
  const [state, formAction, pending] = useActionState(
    useApiAction<ActionState>({ id: "nodes.pairNode", args: [] }),
    INITIAL,
  );
  const [name, setName] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [gatewayName, setGatewayName] = useState(defaultGatewayName);
  const [role, setRole] = useState("editor");
  const [allowInsecureTls, setAllowInsecureTls] = useState(false);

  const showInsecureHttpWarning = /^http:\/\//i.test(baseUrl.trim());
  // 필드에 붙지 않는 실패(연결 거부·자기 자신 등록 등). 폼 **맨 위**에 그린다.
  const generalError = state.error ?? null;

  /**
   * **맨 위에 그리는 것만으로는 모자란다 — 스크롤을 옮겨야 한다.**
   *
   * 제출하려면 폼 아래쪽 버튼까지 내려가야 하는데(실측: 폼 935px, 창 552px),
   * 실패해도 스크롤은 그 자리에 남는다. 그래서 맨 위의 안내가 창 **위쪽 바깥**에
   * 놓인다 — 실측으로 `top: -211px` 였다. 사용자에게는 「입력만 지워지고 아무 말도
   * 없다」로 보이고, 그게 이 화면에 대해 실제로 들어온 신고다.
   *
   * ⚠ **의존성이 `attempt` 인 것이 요점이다. 오류 문구가 아니다.**
   * 문구를 의존성으로 쓰면 **같은 사유로 두 번째 실패할 때 효과가 돌지 않는다** —
   * `Object.is` 가 성립하기 때문이다. 그런데 틀린 토큰을 붙여넣고 또 붙여넣는 것이
   * 이 화면에서 가장 흔한 경로라, 하필 거기서 배너가 화면 밖에 남는다.
   * `attempt` 는 제출마다 달라지므로(`actions.ts`) 그 구멍이 닫힌다.
   *
   * 배너에 `key={state.attempt}` 를 준 것도 같은 이유다. `role="alert"` 은 DOM
   * 텍스트가 그대로면 **다시 읽어 주지 않으므로**, 매 제출마다 **새 노드**로
   * 갈아 끼워 스크린 리더가 새 알림으로 받게 한다. (이 저장소에는 스크린 리더로
   * 재보는 수단이 없다 — 보이는 쪽 스크롤은 실측했고, 이쪽은 표준 관용을 따랐다.)
   */
  const errorRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (generalError)
      errorRef.current?.scrollIntoView({ behavior: "smooth", block: "center" });
  }, [state.attempt]);

  return (
    <form
      action={formAction}
      ref={preservePairingForm}
      className={styles.formStack}
    >
      {generalError ? (
        <div key={state.attempt} ref={errorRef} data-wide>
          <Banner tone="error" role="alert" title="노드를 연결하지 못했습니다">
            <p>{generalError}</p>
            <p>
              입력값은 그대로 두었습니다. <strong>토큰 칸만 비웠습니다</strong>{" "}
              — 다시 붙여넣고 연결을 다시 시도하세요.
            </p>
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
        </div>
      ) : null}
      {/* Below, an error takes the hint's place, as before. */}
      <FormStack>
        <Banner
          tone="info"
          title={
            <>
              원격 nginx-web-ui 인스턴스를 노드로 연결합니다. 연결할 노드의
              관리자 패널 → 액세스 토큰에서 node:pair 권한으로 발급한 서버
              토큰이 필요합니다 — 이 게이트웨이에서 발급한 토큰이 아닙니다.
              연결이 끝나면 그 뒤로는 노드가 발급해 준 토큰으로 접속합니다.
            </>
          }
        />
        <Card
          id="node-connection"
          title="노드 연결"
          errorCount={
            Object.keys(state.fieldErrors ?? {}).filter(
              (key) => key !== "token",
            ).length
          }
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
                placeholder="office-node-1"
                maxLength={100}
                value={name}
                onChange={(e) => setName(e.target.value)}
                required
              />
            </Field>

            <Field
              label="주소"
              htmlFor="node-base-url"
              hint={
                state.fieldErrors
                  ?.baseUrl ? undefined : showInsecureHttpWarning ? (
                  <p className={styles.hintWarning}>
                    평문 http 주소입니다. 사내망 등 신뢰할 수 있는 경로가
                    아니라면 https 사용을 권장합니다.
                  </p>
                ) : (
                  "스킴(http:// 또는 https://)을 포함해 입력하세요."
                )
              }
              error={state.fieldErrors?.baseUrl}
            >
              <TextInput
                name="baseUrl"
                placeholder="https://node.example.com"
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                required
              />
            </Field>

            <Field
              label="이 게이트웨이의 이름"
              htmlFor="node-gateway-name"
              hint={
                state.fieldErrors?.gatewayName ? undefined : (
                  <p>
                    노드의 감사 기록과 「연결된 게이트웨이」 목록에 이 이름이
                    표시됩니다. 위 「이름」(우리가 그 노드를 부르는 이름)과는
                    반대 방향입니다.
                  </p>
                )
              }
              error={state.fieldErrors?.gatewayName}
            >
              <TextInput
                name="gatewayName"
                value={gatewayName}
                onChange={(e) => setGatewayName(e.target.value)}
                placeholder="nginx.example.com"
                maxLength={100}
                required
              />
            </Field>

            <Field
              label="토큰 등급"
              htmlFor="node-role"
              hint="이 게이트웨이가 노드에서 할 수 있는 최대 권한을 요청합니다. 노드가 실제로 발급하는 등급이 최종값입니다."
              error={state.fieldErrors?.role}
            >
              <Select
                name="role"
                value={role}
                onChange={setRole}
                options={[
                  { value: "admin", label: "admin" },
                  { value: "editor", label: "editor" },
                  { value: "viewer", label: "viewer" },
                ]}
                aria-label="토큰 등급"
              />
            </Field>

            <Checkbox
              style={{ gridColumn: "1 / -1" }}
              id="node-allow-insecure-tls"
              name="allowInsecureTls"
              value="true"
              checked={allowInsecureTls}
              onChange={(e) => setAllowInsecureTls(e.target.checked)}
              label="자체 서명 인증서 허용 (https 인증서를 검증하지 않음)"
            />
          </div>
        </Card>
        <Card
          id="node-credential"
          title="노드의 액세스 토큰"
          errorCount={state.fieldErrors?.token ? 1 : 0}
        >
          {/*
        토큰 칸은 전폭이다 — 값이 길어서(`nwu_s_` + base64url) 반 칸에 넣으면
        붙여넣은 것이 한눈에 안 들어온다.

        **`value`/`onChange` 가 없는 것이 의도다.** 위 문서의 평문 논거대로,
        제출 뒤 React 가 이 칸을 비우는 것을 그대로 둔다.
      */}
          <Field
            label="액세스 토큰"
            htmlFor="node-token"
            hint={
              <>
                {state.fieldErrors?.token ? null : (
                  <p>
                    연결할 노드의 관리자 패널 → 액세스 토큰에서 node:pair
                    권한으로 발급한 토큰을 붙여넣으세요. 이 게이트웨이의 액세스
                    토큰이 아니라 <strong>그 노드에서</strong> 발급한 것입니다.
                  </p>
                )}
                <p>
                  붙여넣은 토큰은 이 연결 요청에만 실려 나가고 이 게이트웨이
                  어디에도 저장되지 않습니다. 연결에 실패하면 이 칸은 비워지니
                  다시 붙여넣으세요.
                </p>
              </>
            }
            error={state.fieldErrors?.token}
            wide
          >
            <TextInput
              name="token"
              type="password"
              placeholder="nwu_s_…"
              autoComplete="off"
              spellCheck={false}
              required
            />
          </Field>
        </Card>
      </FormStack>
      <FormBar>
        <Link
          href="/nodes"
          className={buttonClassName({ variant: "secondary" })}
        >
          취소
        </Link>
        <Button type="submit" disabled={pending}>
          {pending ? "연결하는 중..." : "노드 연결"}
        </Button>
      </FormBar>
    </form>
  );
}
