import layout from "@/components/page-layout.module.css";
import {
  Banner,
  PageHeader,
  SettingRow,
  SettingRows,
  StatusBadge,
  Surface,
  buttonClassName,
} from "@repo/ui";
import Link from "next/link";
import type { AccountView } from "@repo/contracts";
import type { PasskeyListView } from "@/lib/passkey-api";
import { AccountPasskeys } from "@/components/passkeys/passkey-list";
import { RoleBadge } from "@/components/role-badge";
import { LinkedLogins } from "./linked-logins";
import styles from "./account-pages.module.css";

export type AccountNotice =
  | "removed"
  | "linked"
  | "unlinked"
  | "last-method"
  | "link-failed"
  | "already-linked";
export const accountMessages: Record<AccountNotice, string> = {
  removed:
    "패스키를 삭제했습니다. 이 계정의 다른 세션은 모두 로그아웃했습니다.",
  linked:
    "외부 로그인을 연결했습니다. 이제 로그인 화면의 그 버튼으로도 로그인할 수 있습니다.",
  unlinked: "외부 로그인 연결을 해제했습니다.",
  "last-method":
    "마지막 남은 로그인 방법은 해제할 수 없습니다. 먼저 다른 방법을 추가하세요.",
  "link-failed": "외부 로그인을 연결하지 못했습니다. 다시 시도하세요.",
  "already-linked": "이 외부 계정은 이미 다른 계정에 연결되어 있습니다.",
};
export function AccountOverview({
  account,
  notice,
  passkeys,
  role = "viewer",
}: {
  account: AccountView;
  passkeys?: PasskeyListView;
  role?: "admin" | "editor" | "viewer";
  notice?: { kind: AccountNotice };
}) {
  return (
    <div className={layout.page}>
      <PageHeader
        title="계정"
        descriptionVariant="meta"
        description={
          <span className={styles.accountMeta}>
            <strong>{account.email}</strong>
            <RoleBadge role={role} />
            {!account.hasPassword ? (
              <StatusBadge tone="neutral" size="sm" dot={false}>
                외부 로그인 전용
              </StatusBadge>
            ) : null}
          </span>
        }
      />
      {notice ? (
        <Banner
          tone={
            ["removed", "linked", "unlinked"].includes(notice.kind)
              ? "success"
              : "error"
          }
          title={accountMessages[notice.kind]}
        />
      ) : null}
      <Surface title="비밀번호와 2단계 인증">
        <SettingRows>
          <SettingRow
            label="비밀번호"
            status={
              <StatusBadge tone={account.hasPassword ? "success" : "neutral"}>
                {account.hasPassword ? "설정됨" : "없음"}
              </StatusBadge>
            }
            detail={
              !account.hasPassword
                ? "외부 로그인으로만 로그인하는 계정이라 비밀번호가 없습니다."
                : undefined
            }
            action={
              account.hasPassword ? (
                <Link
                  href="/account/password"
                  className={buttonClassName({
                    variant: "secondary",
                    size: "sm",
                  })}
                >
                  비밀번호 변경
                </Link>
              ) : undefined
            }
          />
          <SettingRow
            label="2단계 인증"
            status={
              <StatusBadge tone={account.totpEnabled ? "success" : "neutral"}>
                {account.totpEnabled ? "켜짐" : "설정 안 됨"}
              </StatusBadge>
            }
            detail={
              account.totpEnabled
                ? "로그인할 때 인증 앱의 코드를 한 번 더 확인합니다."
                : "설정하면 로그인할 때 인증 앱의 코드를 한 번 더 확인합니다."
            }
            action={
              <Link
                href="/account/2fa"
                aria-label={
                  account.totpEnabled ? "2단계 인증 관리" : "2단계 인증 설정"
                }
                className={buttonClassName({
                  variant: "secondary",
                  size: "sm",
                })}
              >
                {account.totpEnabled ? "관리" : "설정"}
              </Link>
            }
          />
        </SettingRows>
      </Surface>
      {passkeys ? <AccountPasskeys data={passkeys} role={role} /> : null}
      <LinkedLogins
        rows={account.logins.map((row) => ({
          ...row,
          linkedAt: row.linkedAt ? new Date(row.linkedAt) : null,
        }))}
        hasPassword={account.hasPassword}
      />
      <Surface title="API 접근">
        <SettingRows>
          <SettingRow
            label="액세스 토큰"
            wideDetail
            detail="API나 자동화 도구가 이 계정으로 작업할 때 씁니다. 계정의 현재 역할과 토큰에 고른 권한이 모두 허락하는 작업만 할 수 있습니다. 비밀번호·2단계 인증·패스키·토큰 관리 같은 계정 보안 작업은 토큰으로 할 수 없습니다. SMTP·웹훅 시험은 웹에서만 할 수 있습니다. 토큰을 쓰려면 2단계 인증을 등록해야 합니다."
            action={
              <Link
                href="/account/tokens"
                className={buttonClassName({
                  variant: "secondary",
                  size: "sm",
                })}
              >
                토큰 관리
              </Link>
            }
          />
        </SettingRows>
      </Surface>
    </div>
  );
}
