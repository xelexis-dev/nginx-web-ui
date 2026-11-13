import Link from "next/link";
import { Icon, PageHeader, linkClassName } from "@repo/ui";
import { readPageApi } from "@/lib/api-page";
import { passkeyListView, type PasskeyList } from "@/lib/passkey-api";
import { RegisterPasskeyForm } from "@/components/passkeys/passkey-forms";
import layout from "@/components/page-layout.module.css";
export const runtime = "nodejs";
export default async function NewPasskeyPage() {
  const data = passkeyListView(
    await readPageApi<PasskeyList>("/api/ui/v1/account/passkeys"),
  );
  return (
    <section className={layout.page}>
      <PageHeader
        title="패스키 등록"
        eyebrow={
          <Link href="/account" className={linkClassName({ variant: "back" })}>
            <Icon name="arrowLeft" />
            계정으로
          </Link>
        }
      />
      <RegisterPasskeyForm data={data} />
    </section>
  );
}
