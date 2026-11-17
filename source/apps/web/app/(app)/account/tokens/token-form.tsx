"use client";
import { Card, FormStack, FormBar, formFieldsClassName } from "@repo/ui";
import resourceStyles from "@/components/resource-layout.module.css";
import { FormErrors } from "@/components/form-errors";
import { useActionState, useState } from "react";
import Link from "next/link";
import {
  CodePane,
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
  twoFactorEnabled?: boolean;
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
  twoFactorEnabled = true,
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
          <CodePane title="토큰" variant="secret" code={state.token} />
          <p className={styles.secretMeta}>
            이름: {state.name} · 창을 닫거나 새로고침하면 이 값은 사라집니다.
          </p>
          {kind === "user" ? <p className={styles.secretMeta}>요청 머리글: <code>Authorization: Bearer &lt;토큰&gt;</code> · 대상: <code>&lt;관리 주소&gt;/api/ui/v1/…</code>(웹과 같은 API, HTTPS로만 쓰세요)</p> : null}
          <div className={styles.secretActions}>
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
        </div>
      </Surface>
    );
  }

  if (kind === "user" && !twoFactorEnabled) {
    return <Surface title="2단계 인증 등록 필요">
      <Banner tone="info" title="사용자 API 토큰을 발급하려면 먼저 2단계 인증을 등록하세요." />
      <div className={styles.secretActions}>
        <Link href={basePath} className={buttonClassName({ variant: "secondary" })}>취소</Link>
        <Link href="/account/2fa" className={buttonClassName()}>2단계 인증 설정</Link>
      </div>
    </Surface>;
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
      <FormErrors
        errors={state.fieldErrors}
        targets={{ expiresAt: "token-expiry", scopes: "token-scopes" }}
      />
      <FormStack>
        <Banner
          tone="info"
          title={
            kind === "user"
              ? "고른 권한만 이 토큰으로 할 수 있습니다."
              : "다른 게이트웨이가 이 서버를 노드로 연결할 때 제시하는 토큰입니다. 이름에 어느 게이트웨이에 줄 것인지 적어 두세요."
          }
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
                  : kind === "user" ? "고른 권한만 이 토큰으로 할 수 있습니다. 나중에 바꿀 수 없습니다. config:write는 설정 적용을 포함하지 않습니다. 롤백에는 config:write와 nginx:reload가 모두 필요합니다. user:manage는 자격증명 관리를 포함하지 않습니다. 인증서 교체에는 config:write와 nginx:reload가 모두 필요합니다. 계정에 역할을 부여하거나 계정을 활성화할 때는 그 역할의 모든 권한이 토큰에 있어야 합니다. admin 계정 변경과 admin 역할 부여는 할 수 없습니다. SMTP·웹훅 시험은 웹에서만 할 수 있습니다." : "고른 권한만 이 토큰으로 할 수 있습니다. 나중에 바꿀 수 없습니다."
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
