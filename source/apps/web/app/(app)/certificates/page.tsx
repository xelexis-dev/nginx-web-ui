import type { CertificateListView } from "@repo/contracts";
import { CertificateList } from "@/components/certificates/certificate-list";
import { readPageView } from "@/lib/api-page";
import { certificateRowPresentation } from "@/lib/certificate-api";
export const runtime="nodejs";
export default async function CertificatesPage(){
  const view=await readPageView<CertificateListView>("/certificates");
  return (
    <CertificateList
      data={{items:view.data.items.map(certificateRowPresentation)}}
      basePath="/certificates"
      write={view.canWrite ? { newHref: "/certificates/new", importHref: "/certificates/import" } : null}
    />
  );
}
