"use client";

import {
  buttonClassName,
  linkClassName,
  SegmentedControl,
  TextArea,
  TextInput,
  Field,
  FieldGroup,
  Banner,
  Card,
  formFieldsClassName,
  FormBar,
  FormStack,
} from "@repo/ui";

import { useActionState, useState, type ReactNode } from "react";
import Link from "next/link";
import { Button } from "@repo/ui";
// 이 폼이 `NodeErrorCard` 를 들여오는 것은 **의도한 것이다** — 로컬·노드가 같은
// 컴포넌트를 쓰는 것이 이 계획의 방식이라, 노드 액션이 넘어올 자리를 로컬 화면이
// 그리는 폼도 그대로 갖고 있어야 한다(`components/upstreams/upstream-form.tsx`
// 헤더와 같은 판단). 그 카드는 클라이언트용이라 `node:tls` 사슬을 끌고 오지 않는다.
import { NodeErrorCard } from "@/components/node-error";
import type { NodeActionState, NodeFormAction } from "@/lib/node-action-state";
import { useApiAction } from "@/lib/use-api-action";
// 라우트 그룹의 시트를 그대로 들여온다 — 왜 복사하지 않는지는
// `certificate-list.tsx` 헤더.
import {
  targetLabel,
  type OperationsTarget,
} from "@repo/presentation/operations-target";
import styles from "@/app/(app)/certificates/certificates.module.css";

const INITIAL: NodeActionState = { ok: false };
const ERROR_FIELDS: Record<string, string> = {
  alias: "cert-alias",
  domains: "cert-domains",
  email: "cert-email",
  dnsToken: "cert-dns-token",
};

/** Rust certificate API alias limit, mirrored for the input length hint. */
const MAX_ALIAS_LENGTH = 100;

/**
 * 발급 요청 폼: 별칭(선택) + 도메인(줄당 1개) + ACME 계정 이메일 + 운영/스테이징 토글.
 * 제출하면 백그라운드 발급이 시작되고 상세 페이지로 이동한다(상세가 폴링).
 *
 * **로컬(`(app)/certificates/new`)과 노드(`(node)/nodes/[id]/certificates/new`)가
 * 같은 것을 렌더한다.** 다른 것은 넘기는 액션과 `defaultEmail` 의 출처뿐이다.
 */
export function CertificateForm({
  info,
  action,
  defaultEmail,
  cancelHref,
  target,
}: {
  info?: ReactNode;
  /**
   * 로컬 액션도 노드 액션도 그대로 들어간다 — `NodeFormAction` 이 기존
   * `ActionState` 액션의 상위 계약이다(그 타입의 문서). **`ActionState` 로
   * 되돌리지 마라**: 컴파일은 통과하는데 아래 `state.nodeError` 가 타입에서
   * 사라져 §9 카드가 영영 안 그려진다.
   */
  action: NodeFormAction;
  /**
   * 연락 이메일 기본값. **로컬과 노드가 다른 값을 넘긴다** — 그 이유는 노드
   * 발급 화면(`(node)/nodes/[id]/certificates/new/page.tsx`) 헤더에 있다.
   *
   * **이 값은 그 대상의 ACME 계정 연락처가 된다** — `getOrCreateAccount` 가
   * 이메일이 다르면 갱신하기 때문이다. 그래서 「어느 주소를 1순위로 넣을지」는
   * 화면이 정한다: 발급은 「지금 지시하는 사람」, **수정은 「이미 저장돼 있던
   * 주소」**(`CertificateEditFormProps.defaults.email`).
   */
  defaultEmail: string;
  /** 「취소」가 갈 곳 — 로컬/노드의 인증서 목록. */
  cancelHref: string;
  target?: OperationsTarget;
}) {
  const [dnsToken, setDnsToken] = useState("");
  const submit = useApiAction<NodeActionState>(action);
  const [state, formAction, pending] = useActionState(
    async (previous: NodeActionState, form: FormData) => {
      try {
        return await submit(previous, form);
      } finally {
        setDnsToken("");
      }
    },
    INITIAL,
  );
  const [alias, setAlias] = useState("");
  const [domains, setDomains] = useState("");
  const [email, setEmail] = useState(defaultEmail);
  const [staging, setStaging] = useState(false);
  const [challengeType, setChallengeType] = useState<"http-01" | "dns-01">(
    "http-01",
  );
  const [dnsProvider, setDnsProvider] = useState<"" | "cloudflare">("");

  return (
    <form action={formAction} className={styles.form}>
      {/* 노드 액션이 낸 「닿지 못함」·「버전 불일치」 등은 한 줄 빨간 글씨가 아니라
            §9 카드다. 로컬 액션은 `nodeError` 를 채우는 일이 없어 아래 한 줄로
            그대로 떨어진다(동작 무변화) — `upstream-form.tsx` 와 같은 갈래. */}
      {state.nodeError ? (
        <NodeErrorCard {...state.nodeError} />
      ) : state.error || Object.keys(state.fieldErrors ?? {}).length > 0 ? (
        <Banner tone="error" role="alert" title={state.error}>
          {Object.entries(state.fieldErrors ?? {}).map(([name, message]) => (
            <div key={name}>
              <a
                className={linkClassName()}
                href={`#${ERROR_FIELDS[name] ?? "certificate-settings-heading"}`}
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
          id="certificate-verification"
          title="도메인 소유권 검증 방식"
          errorCount={
            Object.entries(state.fieldErrors ?? {}).filter(
              ([name, message]) =>
                message &&
                ["challengeType", "dnsProvider", "dnsToken"].includes(name),
            ).length
          }
        >
          <div className={formFieldsClassName({ columns: 2 })}>
            <fieldset
              id="cert-challenge"
              className={styles.verificationChoice}
              data-wide
              aria-labelledby="certificate-verification-heading"
              aria-describedby="cert-challenge-hint"
            >
              <SegmentedControl
                label="검증 방식"
                value={challengeType}
                onChange={setChallengeType}
                options={[
                  { value: "http-01", label: "HTTP-01 (자동)" },
                  { value: "dns-01", label: "DNS-01 (수동 TXT)" },
                ]}
              />
              <div id="cert-challenge-hint" className={styles.verificationHint}>
                {challengeType === "dns-01"
                  ? "인증기관이 이 서버로 접속하지 않으므로 80 포트를 열 수 없는 환경에서도 발급됩니다. TXT 레코드를 직접 넣으면 갱신 때마다 다시 넣어야 하고, Cloudflare 자동 등록을 쓰면 갱신까지 자동입니다."
                  : "인증기관이 이 서버의 80 포트로 직접 접속해 확인합니다. 도메인 DNS가 이 서버의 공인 IP를 가리키고, 80 포트가 인터넷에 열려 있어야 합니다."}
              </div>
            </fieldset>
            {challengeType === "dns-01" ? (
              <FieldGroup
                id="cert-dns-provider"
                legend="TXT 자동 등록"
                hint={
                  dnsProvider === "cloudflare"
                    ? "_acme-challenge TXT 를 Cloudflare API 로 등록하고 검증 후 삭제합니다. 갱신도 만료 30일 전에 자동으로 처리됩니다."
                    : "도메인이 Cloudflare 네임서버를 쓴다면 'Cloudflare (자동)'을 고르세요."
                }
                wide
              >
                <SegmentedControl
                  label="TXT 자동 등록"
                  value={dnsProvider}
                  onChange={setDnsProvider}
                  options={[
                    { value: "", label: "직접 입력" },
                    { value: "cloudflare", label: "Cloudflare (자동)" },
                  ]}
                />
              </FieldGroup>
            ) : null}
            {challengeType === "dns-01" && dnsProvider === "cloudflare" ? (
              <Field
                label="Cloudflare API 토큰"
                htmlFor="cert-dns-token"
                hint={
                  <>
                    토큰은 <strong>이 인증서에만</strong> 저장됩니다 —
                    도메인마다 Cloudflare 계정이 다를 수 있기 때문입니다. 발급을
                    시작하기 전에 위 도메인의 zone 을 실제로 볼 수 있는지
                    확인합니다.
                  </>
                }
                wide
              >
                <TextInput
                  type="password"
                  value={dnsToken}
                  onChange={(e) => setDnsToken(e.target.value)}
                  autoComplete="new-password"
                  placeholder="Zone:DNS:Edit + Zone:Zone:Read 권한 토큰"
                />
              </Field>
            ) : null}
          </div>
        </Card>
        <Card
          id="certificate-settings"
          title="인증서"
          errorCount={
            Object.entries(state.fieldErrors ?? {}).filter(
              ([name, message]) =>
                message &&
                !["challengeType", "dnsProvider", "dnsToken"].includes(name),
            ).length
          }
        >
          <div className={formFieldsClassName({ columns: 2 })}>
            <input type="hidden" name="staging" value={String(staging)} />
            <input type="hidden" name="challengeType" value={challengeType} />
            <input
              type="hidden"
              name="dnsProvider"
              value={challengeType === "dns-01" ? dnsProvider : ""}
            />
            <input type="hidden" name="dnsToken" value={dnsToken} />
            <Field
              label="별칭 (선택)"
              htmlFor="cert-alias"
              hint="인증서 선택 목록에 별칭과 도메인을 함께 표시합니다. 발급되는 인증서 내용은 바뀌지 않습니다."
              error={state.fieldErrors?.alias}
            >
              <TextInput
                name="alias"
                value={alias}
                onChange={(event) => setAlias(event.target.value)}
                type="text"
                maxLength={MAX_ALIAS_LENGTH}
                autoComplete="off"
                placeholder="예: 운영 API"
              />
            </Field>
            <Field
              label="도메인 (줄당 1개)"
              htmlFor="cert-domains"
              error={state.fieldErrors?.domains}
              wide
            >
              <TextArea
                mono
                name="domains"
                value={domains}
                onChange={(event) => setDomains(event.target.value)}
                placeholder={"example.com\nwww.example.com"}
                required
              />
            </Field>
            <Field
              label="연락 이메일 (ACME 계정)"
              htmlFor="cert-email"
              hint={
                <>
                  기본값은 지금 로그인한 계정의 주소입니다. 이 주소는 인증서가
                  아니라 ACME 계정(운영/스테이징 각각)에 붙어서, 같은 대상을
                  쓰는 다른 인증서의 갱신 안내도 이 주소로 가게 됩니다.
                  인증기관에 등록된 연락처가 바뀌는 것은 아닙니다.
                </>
              }
              error={state.fieldErrors?.email}
            >
              <TextInput
                name="email"
                type="email"
                value={email}
                onChange={(event) => setEmail(event.target.value)}
                required
              />
            </Field>
            <FieldGroup id="cert-staging" legend="발급 대상">
              <SegmentedControl
                label="발급 대상"
                value={staging}
                onChange={setStaging}
                options={[
                  { value: false, label: "운영 (Let's Encrypt)" },
                  { value: true, label: "스테이징 (테스트)" },
                ]}
              />
            </FieldGroup>
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
          {pending ? "발급 시작 중..." : "발급 시작"}
        </Button>
      </FormBar>
    </form>
  );
}
