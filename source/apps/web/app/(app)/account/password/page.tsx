import Link from "next/link";
import { redirect } from "next/navigation";
import { PageHeader, linkClassName } from "@repo/ui";
import type { AccountView } from "@repo/contracts";
import { readPageApi } from "@/lib/api-page";
import { ChangePasswordForm } from "../change-password-form";
import layout from "@/components/page-layout.module.css";
export const runtime = "nodejs";
export default async function PasswordPage() {
  const account = await readPageApi<AccountView>("/api/ui/v1/account");
  if (!account.hasPassword) redirect("/account");
  return (
    <section className={layout.page}>
      <PageHeader
        title="비밀번호 변경"
        eyebrow={
          <Link href="/account" className={linkClassName({ variant: "back" })}>
            계정으로
          </Link>
        }
      />
      <ChangePasswordForm />
    </section>
  );
}
