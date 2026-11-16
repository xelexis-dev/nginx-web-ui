import { redirect } from "next/navigation";
import { PageHeader } from "@repo/ui";
import type { AccountView, TwoFactorView } from "@repo/contracts";
import { readPageApi } from "@/lib/api-page";
import { RegenerateBackupCodesForm } from "../../regenerate-backup-codes-form";
import layout from "@/components/page-layout.module.css";

export const runtime = "nodejs";

export default async function RegenerateBackupCodesPage() {
  const view = await readPageApi<TwoFactorView>("/api/ui/v1/account/2fa");
  if (!view.enabled || view.verificationRequired) redirect("/account/2fa");
  const account = await readPageApi<AccountView>("/api/ui/v1/account");
  return (
    <section className={layout.page}>
      <PageHeader title="백업 코드 재발급" />
      <RegenerateBackupCodesForm hasPassword={account.hasPassword} />
    </section>
  );
}
