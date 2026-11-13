import { FormStack, Banner } from "@repo/ui";
import Link from "next/link";
import { redirect } from "next/navigation";
import { PageHeader, linkClassName } from "@repo/ui";
import type { AccountView, TwoFactorView } from "@repo/contracts";
import { readPageApi } from "@/lib/api-page";
import { TwoFactorDisable } from "../two-factor-forms";
import styles from "../two-factor-management.module.css";

export const runtime = "nodejs";

export default async function DisableTwoFactorPage() {
  const view = await readPageApi<TwoFactorView>("/api/ui/v1/account/2fa");
  if (!view.enabled || view.verificationRequired) redirect("/account/2fa");
  const account = await readPageApi<AccountView>("/api/ui/v1/account");
  if (!account.hasPassword) redirect("/account/2fa");
  return (
    <section className={styles.page}>
      <PageHeader
        eyebrow={
          <Link
            href="/account/2fa"
            className={linkClassName({ variant: "back" })}
          >
            2FA 관리로
          </Link>
        }
        title="2FA 해제"
      />
      <FormStack>
        <Banner
          tone="info"
          title={
            "해제하면 기존 인증 앱과 백업 코드로 인증할 수 없고, 등록한 패스키도 모두 삭제됩니다. 현재 비밀번호를 확인합니다."
          }
        />
        <TwoFactorDisable />
      </FormStack>
    </section>
  );
}
