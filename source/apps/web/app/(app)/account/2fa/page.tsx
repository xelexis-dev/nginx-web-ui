import { FormStack, Banner } from "@repo/ui";
import { PageHeader } from "@repo/ui";
import QRCode from "qrcode";
import type { AccountView, TwoFactorView } from "@repo/contracts";
import { readPageApi } from "@/lib/api-page";
import { TwoFactorEnroll, TwoFactorVerify } from "./two-factor-forms";
import { TwoFactorManagement } from "./two-factor-management";
import styles from "./two-factor.module.css";

// Enrollment and authorization belong to the API; SSR only renders its QR.
export const runtime = "nodejs";

export default async function TwoFactorPage() {
  const view = await readPageApi<TwoFactorView>("/api/ui/v1/account/2fa");
  if (view.enabled) {
    if (view.verificationRequired) {
      return (
        <section className={styles.wrap}>
          <div>
            <PageHeader title={<>2단계 인증 확인</>} />
          </div>
          <FormStack>
            <Banner
              tone="info"
              title={<>계속하려면 이 로그인 세션의 인증을 완료하세요.</>}
            />
            <TwoFactorVerify />
          </FormStack>
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
    <section className={styles.wrap}>
      <div>
        <PageHeader title={<>2단계 인증 (2FA) 설정</>} />
      </div>
      <FormStack>
        <Banner
          tone="info"
          title={
            <>
              인증 앱(Google Authenticator, 1Password 등)으로 QR을 스캔한 뒤,
              앱에 표시된 6자리 코드를 입력해 활성화합니다.
            </>
          }
        />
        <TwoFactorEnroll secret={secret} qrDataUrl={qrDataUrl} />
      </FormStack>
    </section>
  );
}
