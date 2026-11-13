import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import type { CertificateNewView } from "@repo/contracts";
import { CertificateForm } from "@/components/certificates/certificate-form";
import { readPageView } from "@/lib/api-page";
export const runtime = "nodejs";
export default async function NewCertificatePage() {
  const { defaultEmail } =
    await readPageView<CertificateNewView>("/certificates/new");
  return (
    <section className={layout.page}>
      <PageHeader title={<>새 인증서</>} />
      <div>
        <CertificateForm
          info={
            <>
              Let&apos;s Encrypt에서 인증서를 발급받습니다. 검증 방식에 따라
              필요한 준비가 다릅니다.
            </>
          }
          action={{ id: "certificates.createCertificate", args: [] }}
          defaultEmail={defaultEmail}
          cancelHref="/certificates"
        />
      </div>
    </section>
  );
}
