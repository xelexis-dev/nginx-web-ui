import { PageHeader } from "@repo/ui";
import QRCode from "qrcode";
import type { AccountView, TwoFactorView } from "@repo/contracts";
import { readPageApi } from "@/lib/api-page";
import { TwoFactorEnroll, TwoFactorVerify } from "./two-factor-forms";
import { TwoFactorManagement } from "./two-factor-management";
import layout from "@/components/page-layout.module.css";

// Enrollment and authorization belong to the API; SSR only renders its QR.
export const runtime = "nodejs";

export default async function TwoFactorPage() {
  const view = await readPageApi<TwoFactorView>("/api/ui/v1/account/2fa");
  if (view.enabled) {
    if (view.verificationRequired) {
      return (
        <section className={layout.page}>
          <PageHeader title={<>2단계 인증 확인</>} />
          <TwoFactorVerify />
        </section>
      );
    }
    const account = await readPageApi<AccountView>("/api/ui/v1/account");
    return <TwoFactorManagement hasPassword={account.hasPassword} />;
  }

  const { secret, otpauthUri } = view.pending;
  const qrDataUrl = await QRCode.toDataURL(otpauthUri, {
    margin: 1,
    width: 220,
  });

  return (
    <section className={layout.page}>
      <PageHeader title={<>2단계 인증 (2FA) 설정</>} />
      <TwoFactorEnroll secret={secret} qrDataUrl={qrDataUrl} />
    </section>
  );
}
