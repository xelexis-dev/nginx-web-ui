import cards from "@/components/account-cards.module.css";
import layout from "@/components/page-layout.module.css";
import {
  Facts,
  PageHeader,
  Card,
  FormStack,
  StatusBadge,
  buttonClassName,
} from "@repo/ui";
import Link from "next/link";
import type { AccountView } from "@repo/contracts";
import type { PasskeyListView } from "@/lib/passkey-api";
import { AccountPasskeys } from "@/components/passkeys/passkey-list";
import { LinkedLogins, type LinkedLoginRow } from "./linked-logins";

/** Product composition shared with Storybook; account data is read by the server page. */
export function AccountOverview({
  account,
  notice,
  passkeys,
  passkeyRemoved,
}: {
  account: AccountView;
  passkeys?: PasskeyListView;
  passkeyRemoved?: boolean;
  notice?: { kind: "linked" | "unlinked" | "last-method" };
}) {
  const twoFactorOn = account.totpEnabled;
  const hasPassword = account.hasPassword;
  const linkRows: LinkedLoginRow[] = account.logins.map((row) => ({
    ...row,
    linkedAt: row.linkedAt ? new Date(row.linkedAt) : null,
  }));
  return (
    <div className={`${layout.page} ${cards.page}`}>
      <PageHeader
        title={<>계정</>}
        description={<>{account.email}</>}
        info={<>비밀번호, 2단계 인증, 액세스 토큰을 관리합니다.</>}
      />
      <FormStack>
        <Card
          id="account-password"
          title="비밀번호"
          actions={
            hasPassword ? (
              <Link
                href="/account/password"
                className={buttonClassName({ variant: "secondary" })}
              >
                비밀번호 변경
              </Link>
            ) : undefined
          }
        >
          {!hasPassword ? (
            <p
              style={{
                margin: 0,
                color: "var(--text-2)",
                fontSize: "var(--fs-sm)",
                lineHeight: "var(--lh-sm)",
              }}
            >
              이 계정은 OAuth 로그인만 사용합니다 — 비밀번호가 없어 변경할
              항목이 없습니다.
            </p>
          ) : undefined}
        </Card>
        <Card
          id="account-2fa"
          title="2단계 인증 (2FA)"
          info="인증 앱으로 생성하는 6자리 코드로 로그인을 한 단계 더 보호합니다."
          actions={
            <Link
              href="/account/2fa"
              className={buttonClassName({ variant: "secondary" })}
            >
              {twoFactorOn ? "2FA 관리" : "2FA 설정"}
            </Link>
          }
        >
          <div className={cards.content}>
            <Facts
              items={[
                {
                  term: "상태",
                  value: (
                    <StatusBadge tone={twoFactorOn ? "success" : "neutral"}>
                      {twoFactorOn ? "사용 중" : "미설정"}
                    </StatusBadge>
                  ),
                },
              ]}
            />
          </div>
        </Card>
        <Card
          id="account-tokens"
          title="액세스 토큰"
          info="외부 서비스에서 이용 가능한 엑세스 토큰을 관리합니다"
          actions={
            <Link
              href="/account/tokens"
              className={buttonClassName({ variant: "secondary" })}
            >
              토큰 관리
            </Link>
          }
        />
        {passkeys ? (
          <AccountPasskeys data={passkeys} removed={passkeyRemoved} />
        ) : null}
        <LinkedLogins
          rows={linkRows}
          hasPassword={hasPassword}
          notice={notice}
        />
      </FormStack>
    </div>
  );
}
