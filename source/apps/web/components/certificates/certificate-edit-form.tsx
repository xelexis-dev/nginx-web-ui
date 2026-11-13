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
// 로컬·노드가 같은 폼을 쓰므로 §9 카드가 이 폼 안에 있어야 한다 —
// `components/upstreams/upstream-form.tsx` 헤더와 같은 판단.
import { NodeErrorCard } from "@/components/node-error";
import type { NodeActionState, NodeFormAction } from "@/lib/node-action-state";
import { useApiAction } from "@/lib/use-api-action";
// 라우트 그룹의 시트를 그대로 들여온다 — 이유는 `certificate-list.tsx` 헤더.
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

/** Rust certificate API marker for preserving the stored DNS provider token. */
const KEEP_DNS_TOKEN = "__keep__";

/** Rust certificate API alias limit, mirrored for the input length hint. */
const MAX_ALIAS_LENGTH = 100;

export interface CertificateEditFormProps {
  info?: ReactNode;
  /**
   * 로컬 액션도 노드 액션도 그대로 들어간다 — **`ActionState` 로 되돌리지
   * 마라**(`NodeFormAction` 문서). 되돌리면 노드가 죽어도 §9 카드가 안 나온다.
   */
  action: NodeFormAction;
  id: string;
  /** 「취소」가 갈 곳 — 이 인증서의 상세(로컬/노드). */
  cancelHref: string;
  target?: OperationsTarget;
  defaults: {
    domains: string[];
    /**
     * 저장된 별칭. 없으면 `""`. **이 값은 그대로 다시 제출된다** — 빈 값을 넣으면
     * 저장하는 순간 「제거」로 실려 별칭이 지워지므로, 저장된 것을 그대로 넣어야
     * 무변경 저장이 무변경이다(`email` 과 같은 이유).
     */
    alias: string;
    /**
     * **저장돼 있던 ACME 연락 주소를 넣어라.** 이 값은 그대로 제출되고,
     * `getOrCreateAccount` 는 이메일이 다르면 그 대상의 계정 연락처를
     * **갱신한다** — 다른 주소를 기본값으로 넣으면 **아무것도 안 바꾸고 저장만
     * 해도** 그 대상의 모든 인증서 안내가 그 주소로 넘어간다(화면에 표시 없음).
     * 저장된 주소가 없을 때만 다른 것으로 물러난다(로컬·노드 두 화면 다 그렇다).
     */
    email: string;
    staging: boolean;
    challengeType: "http-01" | "dns-01";
    /** "" = 사람이 TXT 를 넣는 기존 흐름. */
    dnsProvider: "" | "cloudflare";
  };
  /** issued 인증서를 고치는 중인지 — 살아 있는 인증서라 안내 문구가 달라진다. */
  isIssued: boolean;
  /** 이 인증서에 Cloudflare 토큰이 저장돼 있는지 — 원문은 내려보내지 않는다. */
  hasStoredToken: boolean;
}

/**
 * 인증서 설정 수정 폼. 로컬(`(app)/certificates/[id]/edit`)과 노드
 * (`(node)/nodes/[id]/certificates/[cid]/edit`)가 같은 것을 렌더한다 — 다른
 * 것은 넘기는 액션과 `defaults.email` 의 **출처**뿐이다(순서는 같다: 저장된
 * 주소 → 없으면 지금 로그인한 사람. `defaults.email` 문서 참고).
 *
 * 저장은 설정만 바꾸고 발급을 시작하지 않는다. 살아 있는 인증서의 status를 건드리면
 * 다음 apply에서 연결된 서버의 TLS가 내려가기 때문이다(actions.ts updateCertificate
 * 주석 참조). 그래서 "무엇이 바뀌면 재발급이 필요한지"를 저장 전에 여기서 알린다.
 */
export function CertificateEditForm({
  info,
  action,
  id,
  cancelHref,
  target,
  defaults,
  isIssued,
  hasStoredToken,
}: CertificateEditFormProps) {
  // Stored credentials are never returned; clear each submitted replacement after the request.
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
  const [alias, setAlias] = useState(defaults.alias);
  const [email, setEmail] = useState(defaults.email);
  const [domains, setDomains] = useState(defaults.domains.join("\n"));
  const [staging, setStaging] = useState(defaults.staging);
  const [challengeType, setChallengeType] = useState<"http-01" | "dns-01">(
    defaults.challengeType,
  );
  const [dnsProvider, setDnsProvider] = useState<"" | "cloudflare">(
    defaults.dnsProvider,
  );

  const domainsChanged =
    domains
      .split(/\r?\n/)
      .map((s) => s.trim().toLowerCase())
      .filter(Boolean)
      .join("\n") !== defaults.domains.join("\n");
  const stagingChanged = staging !== defaults.staging;
  const challengeChanged = challengeType !== defaults.challengeType;
  // 도메인·발급 대상이 바뀌면 지금 들고 있는 인증서는 새 설정과 맞지 않는다.
  // 검증 방식만 바꾸는 건 "다음 갱신을 어떻게 할지"의 변경이라 재발급이 필요 없다.
  const needsReissue = domainsChanged || stagingChanged;

  return (
    <form action={formAction} className={styles.form}>
      {/* 노드 액션이 낸 「닿지 못함」·「버전 불일치」 등은 한 줄이 아니라 §9
            카드다. 로컬 액션은 `nodeError` 를 채우지 않으므로 동작이 그대로다. */}
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
                {
                  <>
                    <p>
                      {challengeType === "dns-01"
                        ? "갱신할 때마다 TXT 레코드를 다시 넣어야 합니다 — 자동 갱신되지 않으며, 갱신 시점이 되면 아래 연락 이메일로 안내 메일이 갑니다."
                        : "인증기관이 이 서버의 80 포트로 직접 확인합니다. 만료 30일 전부터 자동으로 갱신됩니다."}
                    </p>
                    {challengeChanged &&
                    defaults.challengeType === "dns-01" &&
                    challengeType === "http-01" ? (
                      <p>
                        DNS-01 → HTTP-01 로 바꾸면 다음 갱신부터 자동으로
                        처리됩니다. 도메인 DNS가 이 서버의 공인 IP를 가리키고 80
                        포트가 열려 있어야 합니다.
                      </p>
                    ) : null}
                  </>
                }
              </div>
            </fieldset>
            {challengeType === "dns-01" ? (
              <FieldGroup
                id="cert-dns-provider"
                legend="TXT 자동 등록"
                hint={
                  dnsProvider === "cloudflare"
                    ? "갱신할 때마다 _acme-challenge TXT 를 Cloudflare API 로 직접 등록·삭제합니다. 사람이 손댈 일이 없어 만료 30일 전 자동 갱신되며, 갱신 안내 메일도 보내지 않습니다."
                    : "도메인이 Cloudflare 네임서버를 쓴다면 'Cloudflare (자동)'을 고르세요 — DNS-01 도 자동 갱신됩니다."
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
                    필요한 권한: <strong>Zone:DNS:Edit</strong> +{" "}
                    <strong>Zone:Zone:Read</strong>. 이 인증서의 도메인 zone
                    에만 범위를 좁힌 토큰을 권장합니다. 저장할 때{" "}
                    <strong>
                      이 인증서의 도메인 zone 을 실제로 볼 수 있는지
                    </strong>
                    확인하며, 통과하지 못하면 저장하지 않습니다.
                  </>
                }
                wide
              >
                <TextInput
                  type="password"
                  value={dnsToken}
                  onChange={(e) => setDnsToken(e.target.value)}
                  autoComplete="new-password"
                  placeholder={
                    hasStoredToken
                      ? "저장됨 — 바꿀 때만 입력"
                      : "토큰을 입력하세요"
                  }
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
                ![
                  "challengeType",
                  "dnsProvider",
                  "dnsToken",
                  "staging",
                ].includes(name),
            ).length
          }
        >
          <div className={formFieldsClassName({ columns: 2 })}>
            <input type="hidden" name="id" value={id} />
            <input type="hidden" name="staging" value={String(staging)} />
            <input type="hidden" name="challengeType" value={challengeType} />
            {/* HTTP-01 로 바꾸면 자동화 값은 의미가 없다 — 서버도 무시하지만 폼에서도 비운다. */}
            <input
              type="hidden"
              name="dnsProvider"
              value={challengeType === "dns-01" ? dnsProvider : ""}
            />
            <input
              type="hidden"
              name="dnsToken"
              value={dnsToken || (hasStoredToken ? KEEP_DNS_TOKEN : "")}
            />
            <Field
              label="별칭 (선택)"
              htmlFor="cert-alias"
              hint="인증서 선택 목록에 별칭과 도메인을 함께 표시합니다. 발급되는 인증서 내용은 바뀌지 않습니다. 비우면 별칭을 지웁니다."
              error={state.fieldErrors?.alias}
            >
              <TextInput
                name="alias"
                type="text"
                value={alias}
                onChange={(event) => setAlias(event.target.value)}
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
                onChange={(e) => setDomains(e.target.value)}
              />
            </Field>
            <Field
              label="연락 이메일 (ACME 계정)"
              htmlFor="cert-email"
              hint={
                <>
                  이 주소는 인증서가 아니라 ACME 계정(운영/스테이징 각각)에
                  붙습니다 — 같은 대상을 쓰는 다른 인증서의 갱신 안내도 이
                  주소로 갑니다. 인증기관에 등록된 연락처가 바뀌는 것은
                  아닙니다.
                </>
              }
              error={state.fieldErrors?.email}
            >
              <TextInput
                name="email"
                type="email"
                value={email}
                onChange={(event) => setEmail(event.target.value)}
              />
            </Field>
            {needsReissue ? (
              <p data-wide className={styles.hint}>
                {isIssued
                  ? "도메인 또는 발급 대상이 바뀌었습니다. 저장해도 지금 서빙 중인 인증서는 그대로 유지되며, 새 설정은 상세 화면에서 갱신을 실행할 때 적용됩니다."
                  : "도메인 또는 발급 대상이 바뀌었습니다. 저장한 뒤 상세 화면에서 발급을 다시 시작하세요."}
              </p>
            ) : null}
          </div>
        </Card>
        <Card
          id="certificate-target"
          title="발급 대상"
          errorCount={
            Object.entries(state.fieldErrors ?? {}).filter(
              ([name, message]) => message && name === "staging",
            ).length
          }
        >
          <fieldset
            id="cert-staging"
            className={styles.verificationChoice}
            aria-labelledby="certificate-target-heading"
          >
            <SegmentedControl
              label="발급 대상"
              value={staging}
              onChange={setStaging}
              options={[
                { value: false, label: "운영 (Let's Encrypt)" },
                { value: true, label: "스테이징 (테스트)" },
              ]}
            />
          </fieldset>
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
          {pending ? "저장 중..." : "저장"}
        </Button>
      </FormBar>
    </form>
  );
}
