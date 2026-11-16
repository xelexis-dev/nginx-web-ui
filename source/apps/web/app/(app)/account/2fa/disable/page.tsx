import { redirect } from "next/navigation";
import { PageHeader } from "@repo/ui";
import type { AccountView, TwoFactorView } from "@repo/contracts";
import { readPageApi } from "@/lib/api-page";
import { TwoFactorDisable } from "../two-factor-forms";
import layout from "@/components/page-layout.module.css";

export const runtime = "nodejs";

export default async function DisableTwoFactorPage() {
  const view = await readPageApi<TwoFactorView>("/api/ui/v1/account/2fa");
  if (!view.enabled || view.verificationRequired) redirect("/account/2fa");
  const account = await readPageApi<AccountView>("/api/ui/v1/account");
  if (!account.hasPassword) redirect("/account/2fa");
  return (
    <section className={layout.page}>
      <PageHeader title="2FA 해제" />
      <TwoFactorDisable />
    </section>
  );
}
