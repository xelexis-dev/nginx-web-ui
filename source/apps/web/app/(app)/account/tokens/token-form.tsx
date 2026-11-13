"use client";
import { Card, FormStack, FormBar, formFieldsClassName } from "@repo/ui";
import resourceStyles from "@/components/resource-layout.module.css";
import { FormErrors } from "@/components/form-errors";
import { useActionState, useState } from "react";
import Link from "next/link";
import {
  TextInput,
  buttonClassName,
  Button,
  Select,
  Banner,
  Surface,
  Checkbox,
  Field,
  FieldGroup,
} from "@repo/ui";
import type { TokenScope } from "@repo/contracts";
type TokenKind = "user" | "server";
import type { IssueTokenState } from "./form-types";
import { useApiAction, type FormAction } from "@/lib/use-api-action";
import { EXPIRY_PRESETS, SCOPE_LABELS, type ExpiryPreset } from "./tokens";
import styles from "./tokens.module.css";

const INITIAL: IssueTokenState = { ok: false };

// Only non-secret inputs are present; success replaces this form with the one-time result panel.
function preserveTokenForm(form: HTMLFormElement | null) {
  if (!form) return;
  form.onreset = (event) => event.preventDefault();
  return () => {
    form.onreset = null;
  };
}

/**
 * 복사 버튼의 상태.
 *
 * ⚠ **`navigator.clipboard` 는 secure context(https·localhost) 에서만 존재한다.**
 * 이 제품은 흔히 `http://<host>:8080` 에 놓이고, 거기서는 그 객체가 `undefined` 다 —
 * 옵셔널 체이닝으로 조용히 넘기면 「눌렀는데 아무 일도 없다」가 되는데, 그 값은
 * **다시 볼 수 없는 값**이다. 그래서 실패를 상태로 만들어 **화면에 말한다**.
 */
export const COPY_IDLE = "idle";
type CopyStatus = typeof COPY_IDLE | "copied" | "failed";

/**
 * 평문을 클립보드에 넣고 **무슨 일이 일어났는지** 낸다.
 *
 * 컴포넌트 밖에 있는 이유는 이것이 유일하게 검사 가능한 자리이기 때문이다 —
 * 클릭 핸들러 안에 두면 서버 렌더만 하는 이 저장소의 검사 하네스로는 절대
 * 실행되지 않고, 실제로 1차 수정에서 「옵셔널 체이닝으로 되돌리기」 변이가
 * **살아남았다**.
 *
 * `navigator.clipboard?.…` 로 넘기지 않는 것이 요점이다. secure context 밖에서는
 * 그 객체가 없어서 체인이 통째로 끊기는데, 그러면 예외도 없고 화면도 안 바뀐다 —
 * 사용자에게는 「눌렀는데 아무 일도 없다」이고 그 값은 **다시 볼 수 없는 값**이다.
 */
export async function copyTokenToClipboard(
  token: string,
): Promise<"copied" | "failed"> {
  try {
    const clipboard = globalThis.navigator?.clipboard;
    if (!clipboard) return "failed";
    await clipboard.writeText(token);
    return "copied";
  } catch {
    // 권한 거부도 여기로 온다 — 놓치면 unhandled rejection 이 될 뿐 사용자는
    // 여전히 아무것도 못 본다.
    return "failed";
  }
}

/**
 * 종류마다 다른 것은 **이름 예시**뿐이다.
 *
 * ⚠ **`server` 는 「이 토큰을 줄 게이트웨이」다.** 서버 토큰은 다른 게이트웨이가 이
 * 인스턴스를 자기 노드로 붙일 때 제시하는 자격증명이라(스펙 §8-4 · `api/v1/pair`),
 * 이름에 실리는 정체는 **받는 쪽**의 것이다. 노드 호스트명을 예시로 두면 방향이
 * 거꾸로 읽히고, 그 오해는 관리자가 자기 서버의 제어권을 넘기는 값을 만드는 것으로
 * 끝난다. 이 항목의 소비자는 `/admin/tokens/new` 하나뿐이다.
 */
const NAME_PLACEHOLDER: Record<TokenKind, string> = {
  user: "노트북 CLI",
  server: "gw.example.com 연결용",
};

export interface TokenFormProps {
  /**
   * 체크박스로 그릴 scope 목록.
   *
   * ⚠ **호출부가 `issuableScopes(kind, role)` 로 만들어 넘긴다.** 이 컴포넌트는
   * 받은 것을 그대로 그린다 — 여기서 다시 거르면 「어디가 상한을 정하는가」가 둘이
   * 되고, 둘이 어긋나는 날 한쪽만 고쳐진다. 사용자 화면에서 `node:pair` 는
   * 비활성화된 항목이 아니라 **목록에 없어야** 한다(스펙 §4).
   */
  scopes: readonly TokenScope[];
  /** 서버 액션. Task 6 은 자기 액션(서버 토큰용)을 넘긴다. */
  action: FormAction<IssueTokenState>;
  /** 목록 경로 — 「취소」와 발급 후 「완료」가 가는 곳. */
  basePath: string;
  kind: TokenKind;
  /** 만료 기본 선택. 종류마다 알맞은 기본이 다를 수 있어 열어 둔다. */
  defaultPreset?: ExpiryPreset;
}

/**
 * 발급 폼(`/account/tokens/new`) — **별도 페이지**다(프로젝트 규칙: 생성은
 * modal·인라인 금지).
 *
 * ── 발급에 성공하면 리다이렉트하지 않는다 ────────────────────────────────
 *
 * 성공한 응답이 평문을 들고 오고 **이 컴포넌트가 그 자리에서 그린다.**
 * 리다이렉트하면 평문을 URL·쿼리스트링·쿠키 중 하나에 실어 날라야 하는데 전부
 * 로그와 히스토리에 남는다. 그래서 성공하면 폼을 **결과 패널로 갈아 끼우고**,
 * 사용자가 「완료」를 눌러 목록으로 간다. 평문은 그 렌더 한 번에만 존재한다.
 *
 * ── 실패해도 입력을 잃지 않는다 ──────────────────────────────────────────
 *
 * `<form action={서버액션}>` 은 제출이 끝나면 React 가 폼을 **리셋한다** —
 * `defaultValue` 가 있든 없든 비워진다(`nodes/node-form.tsx` 가 같은 이유로 방금
 * 제어형이 됐다). 검증 실패마다 이름과 체크박스를 다시 고르게 만들지 않으려면
 * 값을 여기서 들고 있어야 한다.
 */
export function TokenForm({
  scopes,
  action,
  basePath,
  kind,
  defaultPreset = "90d",
}: TokenFormProps) {
  const [state, formAction, pending] = useActionState(
    useApiAction(action),
    INITIAL,
  );
  const [name, setName] = useState("");
  const [checked, setChecked] = useState<readonly string[]>(
    kind === "server"
      ? scopes.filter((scope) =>
          ["node:pair", "config:read", "config:write", "nginx:reload"].includes(
            scope,
          ),
        )
      : [],
  );
  const [preset, setPreset] = useState<string>(
    kind === "server" && defaultPreset === "never" ? "90d" : defaultPreset,
  );
  const [expiresOn, setExpiresOn] = useState("");
  const [copyStatus, setCopyStatus] = useState<CopyStatus>(COPY_IDLE);

  // 발급된 평문이 있으면 폼 대신 결과만 그린다.
  if (state.token) {
    return (
      <Surface
        title={<span id="token-issued-title">토큰을 발급했습니다</span>}
        aria-labelledby="token-issued-title"
      >
        <div className={styles.secretContent}>
          <Banner
            tone="warning"
            role="alert"
            title="이 값은 다시 볼 수 없습니다. 지금 복사해 안전한 곳에 보관하세요."
          />
          {/*
           * ⚠ 평문이 화면에 존재하는 **유일한 자리**다. 목록·상세에는 가림 표시만
           * 있고, URL·쿠키·로그 어디에도 실리지 않는다. `value` 속성이 아니라
           * 본문 텍스트로 두는 것도 의도다 — 한 번만 나타난다.
           */}
          <p className={styles.secretValue}>{state.token}</p>
          <p className={styles.secretMeta}>
            이름: {state.name} · 창을 닫거나 새로고침하면 이 값은 사라집니다.
          </p>
          <div className={styles.secretActions}>
            <Button
              type="button"
              variant="secondary"
              /*
               * ⚠ **이 한 줄은 검사가 아니라 손으로 확인한 것이 지킨다.** 하네스가
               * 서버 렌더 전용이라 클릭을 보낼 수 없어, 이 배선을
               * `onClick={() => setCopyStatus("copied")}` 로 바꿔도 검사는 **전부
               * 통과한다**(실측). 양쪽 끝은 각각 붙들려 있다 — `copyTokenToClipboard`
               * 의 세 결과와, 상태별로 그려지는 문구. 이어 주는 이 줄만 조용하다.
               *
               * 확인은 **secure context 가 아닌 출처**(비-localhost 평문 HTTP)에서
               * 손으로 했다: 버튼을 누르면 아래 실패 문구가 뜬다. 이 줄이 끊기면
               * 거기서 「복사됨」이 뜬다.
               */
              onClick={() =>
                void copyTokenToClipboard(state.token ?? "").then(setCopyStatus)
              }
            >
              {copyStatus === "copied" ? "복사됨" : "복사"}
            </Button>
            <Link
              href={basePath}
              className={buttonClassName({ variant: "primary" })}
            >
              완료
            </Link>
            {state.id ? (
              <Link
                href={`${basePath}/${state.id}`}
                className={buttonClassName({ variant: "secondary" })}
              >
                상세 보기
              </Link>
            ) : null}
          </div>
          {copyStatus === "failed" ? (
            <Banner
              tone="error"
              role="alert"
              title="복사할 수 없습니다 — 위 값을 직접 선택해 복사하세요."
            >
              보안 연결(https)이 아닌 주소에서는 브라우저가 클립보드를 막습니다.
            </Banner>
          ) : null}
        </div>
      </Surface>
    );
  }

  function toggleScope(scope: string, on: boolean) {
    setChecked((prev) =>
      on ? [...prev, scope] : prev.filter((s) => s !== scope),
    );
  }

  return (
    <form
      action={formAction}
      ref={preserveTokenForm}
      className={resourceStyles.form}
    >
      <FormStack>
        <FormErrors
          errors={state.fieldErrors}
          targets={{ expiresAt: "token-expiry", scopes: "token-scopes" }}
        />
        <Card
          id="access-token"
          title={kind === "server" ? "새 서버 토큰" : "새 액세스 토큰"}
          errorCount={
            Object.values(state.fieldErrors ?? {}).filter(Boolean).length
          }
        >
          <div className={formFieldsClassName({ columns: 2 })}>
            {state.error ? (
              <div data-wide>
                <Banner
                  tone="error"
                  role="alert"
                  title="토큰을 발급하지 못했습니다"
                >
                  {state.error}
                </Banner>
              </div>
            ) : null}

            {/* An error takes the hint's place, as before. */}
            <Field
              label="이름"
              htmlFor="token-name"
              hint={
                state.fieldErrors?.name
                  ? undefined
                  : "나중에 목록에서 이 토큰을 알아볼 이름입니다."
              }
              error={state.fieldErrors?.name}
            >
              <TextInput
                name="name"
                placeholder={NAME_PLACEHOLDER[kind]}
                maxLength={100}
                value={name}
                onChange={(e) => setName(e.target.value)}
                required
              />
            </Field>

            <Field
              label="만료"
              htmlFor="token-expiry"
              hint={
                state.fieldErrors?.expiresAt ? undefined : preset ===
                  "never" ? (
                  // 막지 않는다(스펙 §5) — 대신 무엇을 고른 것인지 말한다.
                  <span className={styles.hintWarning}>
                    무기한 토큰은 해지하기 전까지 계속 유효합니다. 유출되면
                    알아차릴 계기가 없습니다.
                  </span>
                ) : (
                  "기간이 지나면 이 토큰은 자동으로 거절됩니다."
                )
              }
              error={state.fieldErrors?.expiresAt}
            >
              {/* 드롭다운은 native `<select>` 금지 — 공통 `Select` 를 쓴다(프로젝트 규칙). */}
              <Select
                name="expiry"
                value={preset}
                onChange={setPreset}
                options={EXPIRY_PRESETS.filter(
                  (p) => kind !== "server" || p.value !== "never",
                ).map((p) => ({
                  value: p.value,
                  label: p.label,
                }))}
                aria-label="만료"
              />
            </Field>

            {preset === "custom" ? (
              /*
          문구와 파싱이 함께 움직인다 — `tokens.ts` 는 고른 날의 **끝**을 만료로 잡는다.
          ⚠ 「오늘부터 고를 수 있습니다」를 다시 붙이지 마라: UTC 서쪽 사용자에게는
          하루 7–8시간 동안 거짓이 된다(`tokens.ts` 의 그 자리 주석 참고).
        */
              <Field
                label="만료 날짜"
                htmlFor="token-expires-on"
                hint="고른 날짜가 끝나면 만료됩니다."
              >
                <TextInput
                  name="expiresOn"
                  type="date"
                  value={expiresOn}
                  onChange={(e) => setExpiresOn(e.target.value)}
                />
              </Field>
            ) : null}

            <FieldGroup
              id="token-scopes"
              legend="권한"
              hint={
                state.fieldErrors?.scopes
                  ? undefined
                  : "고른 권한만 이 토큰으로 할 수 있습니다. 나중에 바꿀 수 없습니다."
              }
              error={state.fieldErrors?.scopes}
              wide
            >
              <div className={styles.scopeGrid}>
                {scopes.map((scope) => (
                  <Checkbox
                    key={scope}
                    tile
                    name="scopes"
                    value={scope}
                    checked={checked.includes(scope)}
                    onChange={(e) => toggleScope(scope, e.target.checked)}
                    label={
                      <span className={styles.scopeItemName}>{scope}</span>
                    }
                    description={SCOPE_LABELS[scope]}
                  />
                ))}
              </div>
            </FieldGroup>
          </div>
        </Card>
        <FormBar>
          <Link
            href={basePath}
            className={buttonClassName({ variant: "secondary" })}
          >
            취소
          </Link>
          <Button type="submit" disabled={pending}>
            {pending ? "발급하는 중..." : "토큰 발급"}
          </Button>
        </FormBar>
      </FormStack>
    </form>
  );
}
