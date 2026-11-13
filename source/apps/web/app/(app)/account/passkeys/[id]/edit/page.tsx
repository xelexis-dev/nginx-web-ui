import Link from "next/link";
import { Icon, PageHeader, linkClassName } from "@repo/ui";
import { readPageApi } from "@/lib/api-page";
import { passkeyRowView, type PasskeyDetail } from "@/lib/passkey-api";
import { RenamePasskeyForm } from "@/components/passkeys/passkey-forms";
import layout from "@/components/page-layout.module.css";
export const runtime = "nodejs";
export default async function EditPasskeyPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const data = await readPageApi<PasskeyDetail>(
    `/api/ui/v1/account/passkeys/${encodeURIComponent(id)}`,
  );
  return (
    <section className={layout.page}>
      <PageHeader
        title="패스키 이름 변경"
        eyebrow={
          <Link href="/account" className={linkClassName({ variant: "back" })}>
            <Icon name="arrowLeft" />
            계정으로
          </Link>
        }
      />
      <RenamePasskeyForm row={passkeyRowView(data.passkey)} />
    </section>
  );
}
