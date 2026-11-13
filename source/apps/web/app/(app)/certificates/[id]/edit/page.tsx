import layout from "@/components/page-layout.module.css";
import { Icon, linkClassName, PageHeader } from "@repo/ui";
import type { CertificateEditView } from "@repo/contracts";
import { redirect } from "next/navigation";
import { readPageView } from "@/lib/api-page";
import Link from "next/link";
import { CertificateEditForm } from "@/components/certificates/certificate-edit-form";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * 인증서 설정 수정. 인증서 ID를 유지하므로 연결된 reverse-proxy 서버가 끊기지
 * 않는다 — 예전처럼 삭제 → 재발급 → 재연결 할 필요가 없다.
 */
export default async function EditCertificatePage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const view = await readPageView<CertificateEditView>(
    `/certificates/${id}/edit`,
  );
  if ("redirectTo" in view) redirect(view.redirectTo);
  return (
    <section className={layout.page}>
      <PageHeader
        title={<>인증서 수정</>}
        eyebrow={
          <>
            {" "}
            <Link
              href={`/certificates/${id}`}
              className={linkClassName({ variant: "back" })}
            >
              <Icon name="arrowLeft" size={14} />
              상세로
            </Link>{" "}
          </>
        }
        info="발급 설정을 바꿉니다."
      />
      <CertificateEditForm
        info={
          <>
            인증서 ID는 그대로라 <strong>연결된 서버는 그대로 유지</strong>
            됩니다. 저장은 설정만 바꾸며, 실제 재발급은 상세 화면에서
            실행합니다.
          </>
        }
        action={{ id: "certificates.updateCertificate", args: [] }}
        id={id}
        cancelHref={`/certificates/${id}`}
        isIssued={view.isIssued}
        hasStoredToken={view.hasStoredToken}
        defaults={view.defaults}
      />
    </section>
  );
}
