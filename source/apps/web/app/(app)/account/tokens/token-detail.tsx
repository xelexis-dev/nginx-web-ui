"use client";

import {
  PageHeader,
  Icon,
  linkClassName,
  Banner,
  Surface,
  StatusBadge,
  type Tone,
  Facts,
  Button,
} from "@repo/ui";

import { useActionState, useState } from "react";
import Link from "next/link";
import type { RevokeTokenState } from "./form-types";
import { useApiAction, type FormAction } from "@/lib/use-api-action";
import {
  SCOPE_LABELS,
  STATUS_LABELS,
  type TokenRowView,
  type TokenStatus,
} from "./tokens";
import styles from "./tokens.module.css";
import layout from "@/components/page-layout.module.css";
import resourceStyles from "@/components/resource-layout.module.css";

const INITIAL: RevokeTokenState = { ok: false };

/** `token-list.tsx` 와 같은 표. CSS 모듈 값이라 `string | undefined` 다. */
const BADGE_TONE: Record<TokenStatus, Tone> = {
  active: "success",
  expired: "warning",
  revoked: "error",
};

export interface TokenDetailProps {
  /**
   * ⚠ **`Date` 가 아니라 이미 만들어진 문자열이다.** 이 컴포넌트는 `"use client"` 라
   * 브라우저에서 **다시 실행**되고, 여기서 시각을 포맷하면 서버(컨테이너 TZ=UTC)와
   * 브라우저 TZ 가 갈려 **목록과 상세가 같은 토큰에 다른 시각을 보인다**(실측: 9시간
   * 차이 + React #418 hydration mismatch). 그래서 포맷은 서버의 `tokenPresentation` 이
   * 하고, 그 사실을 규칙이 아니라 **이 타입이** 지킨다.
   */
  view: TokenRowView;
  /** 목록 경로 — 「목록으로」가 가는 곳. Task 6 은 `/admin/tokens` 를 넘긴다. */
  basePath: string;
  revoke: FormAction<RevokeTokenState>;
}

/**
 * 토큰 상세 + 해지.
 *
 * ⚠ **평문은 여기에 없다.** 다시 가져올 방법도 없다 — 저장되는 것은 해시와 뒤
 * 4자뿐이라(`access-token-service.ts`), 이 화면이 가진 것은 `masked` 가 전부다.
 * 「다시 보기」 같은 것을 나중에 붙이지 마라: 붙일 수 있는 값이 없다.
 */
export function TokenDetail({ view, basePath, revoke }: TokenDetailProps) {
  const [state, formAction, pending] = useActionState(
    useApiAction(revoke),
    INITIAL,
  );
  const [confirming, setConfirming] = useState(false);

  const server = basePath === "/admin/tokens";
  const actions = (
    <div className={resourceStyles.detailActions}>
      <div className={resourceStyles.actionRow}>
        {/*
         * 해지된 행에는 해지 버튼이 **없다** — 다시 누르면 처음 끊은 시각이 덮이는
         * 것이 아니라 아무 일도 일어나지 않는데(`revokeToken` 의 `isNull` 조건),
         * 사용자에게는 「눌렀는데 실패했다」로 보인다.
         *
         * 판정은 `revoked` **불리언**이다. `revokedAt` 문자열("해지되지 않음")로
         * 되물으면 그 문구를 고치는 날 버튼이 조용히 사라지거나 되살아난다.
         */}
        {view.revoked ? null : (
          <form action={formAction}>
            <input type="hidden" name="id" value={view.id} />
            {/*
             * ⚠ **`kind` 도 `ownerUserId` 도 폼에 없다.** 해지 범위는 세션이 만든다
             * (`token-actor.ts`) — 여기 hidden field 하나가 UPDATE 의 WHERE 를
             * 넓혀 남의 토큰이나 서버 토큰을 끊는 수단이 된다.
             */}
            {confirming ? (
              <span className={styles.inlineActions}>
                <Button type="submit" variant="dangerQuiet" disabled={pending}>
                  {pending ? "해지하는 중..." : "정말 해지"}
                </Button>
                <Button
                  type="button"
                  variant="secondary"
                  onClick={() => setConfirming(false)}
                  disabled={pending}
                >
                  취소
                </Button>
                <span className={styles.hintWarning}>
                  해지하면 되돌릴 수 없습니다.
                </span>
              </span>
            ) : (
              <Button
                type="button"
                variant="dangerQuiet"
                onClick={() => setConfirming(true)}
              >
                토큰 해지
              </Button>
            )}
          </form>
        )}
      </div>
      {state.error ? (
        <Banner tone="error" role="alert" title="해지하지 못했습니다">
          {state.error}
        </Banner>
      ) : null}
    </div>
  );
  return (
    <section className={layout.page}>
      <PageHeader
        title={view.name}
        actions={actions}
        eyebrow={
          <Link href={basePath} className={linkClassName({ variant: "back" })}>
            <Icon name="arrowLeft" size={14} />
            {server ? "서버 토큰 목록으로" : "토큰 목록으로"}
          </Link>
        }
        info={
          <>
            <p>
              {server
                ? "이 서버가 발급한 서버 토큰입니다."
                : "내 액세스 토큰입니다."}
            </p>
            <p>
              토큰 값은 발급할 때 한 번만 보여줍니다. 저장된 것은 해시와 뒤
              4자뿐이라 다시 볼 수 없습니다.
            </p>
          </>
        }
        description={
          server ? (
            <>
              <p className={layout.note}>
                해지하면 이 토큰으로는 더 이상 새로 연결할 수 없습니다. 이
                토큰과 연결 기록이 있는 게이트웨이의 접근도 함께 해지됩니다.
                업데이트 전에 연결된 게이트웨이는 이 토큰과의 연결 정보가 없어
                함께 해지되지 않으니, 「보안 (2FA)」의 「연결된 게이트웨이」에서
                해지하세요.
              </p>
              <Link
                href="/admin/security"
                className={linkClassName({ variant: "back" })}
              >
                연결된 게이트웨이
                <Icon name="arrowRight" size={14} />
              </Link>
            </>
          ) : undefined
        }
      />
      <Banner
        tone={BADGE_TONE[view.status]}
        title={STATUS_LABELS[view.status]}
      />
      <Surface as="div">
        <Facts
          columns={2}
          items={[
            { term: <>이름</>, value: <>{view.name}</> },
            { term: <>토큰</>, value: <>{view.masked}</> },
            {
              term: <>상태</>,
              value: (
                <>
                  <StatusBadge tone={BADGE_TONE[view.status]} size="sm">
                    {STATUS_LABELS[view.status]}
                  </StatusBadge>
                </>
              ),
            },
            {
              term: <>권한</>,
              value: (
                <>
                  <ul className={styles.scopeTags}>
                    {view.scopes.map((scope) => (
                      <li key={scope}>
                        <StatusBadge
                          tone="neutral"
                          size="sm"
                          variant="outline"
                          dot={false}
                        >
                          {scope}
                          {SCOPE_LABELS[scope as keyof typeof SCOPE_LABELS]
                            ? ` (${SCOPE_LABELS[scope as keyof typeof SCOPE_LABELS]})`
                            : ""}
                        </StatusBadge>
                      </li>
                    ))}
                  </ul>
                </>
              ),
            },
            { term: <>만든 날</>, value: <>{view.createdAt}</> },
            { term: <>마지막 사용</>, value: <>{view.lastUsedAt}</> },
            { term: <>만료</>, value: <>{view.expiresAt}</> },
            { term: <>해지</>, value: <>{view.revokedAt}</> },
          ]}
        />
      </Surface>
    </section>
  );
}
