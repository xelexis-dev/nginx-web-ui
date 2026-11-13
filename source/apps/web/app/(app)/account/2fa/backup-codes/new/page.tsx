import { FormStack, Banner } from "@repo/ui";
import Link from "next/link";
import { redirect } from "next/navigation";
import { PageHeader, linkClassName } from "@repo/ui";
import type { AccountView, TwoFactorView } from "@repo/contracts";
import { readPageApi } from "@/lib/api-page";
import { RegenerateBackupCodesForm } from "../../regenerate-backup-codes-form";
import styles from "../../two-factor-management.module.css";

export const runtime = "nodejs";

export default async function RegenerateBackupCodesPage() {
  const view = await readPageApi<TwoFactorView>("/api/ui/v1/account/2fa");
  if (!view.enabled || view.verificationRequired) redirect("/account/2fa");
  const account = await readPageApi<AccountView>("/api/ui/v1/account");
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
        title="백업 코드 재발급"
      />
      <FormStack>
        <Banner
          tone="info"
          title={"인증 앱 설정은 유지하고 백업 코드만 새로 발급합니다."}
        />
        <RegenerateBackupCodesForm hasPassword={account.hasPassword} />
      </FormStack>
    </section>
  );
}
