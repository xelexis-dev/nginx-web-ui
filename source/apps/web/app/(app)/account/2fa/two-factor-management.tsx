import layout from "@/components/page-layout.module.css";
import Link from "next/link";
import {
  PageHeader,
  SettingRow,
  SettingRows,
  Surface,
  buttonClassName,
} from "@repo/ui";

/** The account's available operations; the API still authorizes every action. */
export function TwoFactorManagement({ hasPassword }: { hasPassword: boolean }) {
  return (
    <section className={layout.page}>
      <PageHeader
        title="2FA 관리"
        description="2단계 인증을 사용 중입니다."
        info="백업 코드를 새로 발급하거나 2FA를 해제할 수 있습니다."
      />
      <Surface>
        <SettingRows>
          <SettingRow
            label="백업 코드"
            wideDetail
            detail="인증 앱을 사용할 수 없을 때 백업 코드 하나로 로그인할 수 있습니다. 새로 발급하면 기존 백업 코드는 모두 사용할 수 없게 됩니다. 새 코드는 발급 직후 한 번만 표시되며 TXT 파일로 저장할 수 있습니다."
            action={
              <Link
                href="/account/2fa/backup-codes/new"
                className={buttonClassName({
                  variant: "secondary",
                  size: "sm",
                })}
              >
                백업 코드 재발급
              </Link>
            }
          />
          <SettingRow
            label="2FA 해제"
            wideDetail
            detail={
              <>
                해제하면 로그인할 때 인증 앱 코드나 백업 코드를 확인하지
                않습니다.{" "}
                {hasPassword
                  ? "현재 비밀번호로 본인을 확인한 뒤 해제합니다. 해제하면 등록한 패스키도 모두 삭제되고, 이 계정의 API 토큰도 모두 폐기됩니다. 2FA가 필수인 계정은 해제 후 다시 설정해야 합니다."
                  : "비밀번호가 없는 OAuth 계정은 관리자에게 2FA 초기화를 요청하세요. 백업 코드 재발급은 인증 앱으로 본인을 확인한 뒤 이용할 수 있습니다."}
              </>
            }
            action={
              hasPassword ? (
                <Link
                  href="/account/2fa/disable"
                  className={buttonClassName({
                    variant: "dangerQuiet",
                    size: "sm",
                  })}
                >
                  2FA 해제
                </Link>
              ) : undefined
            }
          />
        </SettingRows>
      </Surface>
    </section>
  );
}
