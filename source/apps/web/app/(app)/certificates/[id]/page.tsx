import type { CertificateDetailView } from "@repo/contracts";
import { CertificateDetail } from "@/components/certificates/certificate-detail";
import { CertificateDetailActions } from "@/components/certificates/certificate-detail-actions";
import { DnsChallengePanel } from "@/components/certificates/dns-challenge-panel";
import {readPageView} from "@/lib/api-page";
import {certificateDetailPresentation} from "@/lib/certificate-api";
export const runtime="nodejs";
export default async function CertificateDetailPage({
  params,
  searchParams,
}: {
  params: Promise<{ id: string }>;
  searchParams: Promise<{ uncovered?: string }>;
}) {
  const {id}=await params;
  const view=await readPageView<CertificateDetailView>(`/certificates/${id}`,await searchParams);
  const {canWrite,uncovered}=view;
  const data=certificateDetailPresentation(view.data);
  return (
    <CertificateDetail
      data={data}
      basePath="/certificates"
      serverBasePath="/reverse-proxy"
      uncovered={uncovered}
      dnsPanel={
        data.dnsOrder?.phase === "awaiting-txt" ? (
          <DnsChallengePanel
            id={data.id}
            submitAction={{id:"certificates.submitDnsChallengeAction",args:[]}}
            cancelPath={`/api/certificates/${data.id}/cancel`}
            records={data.dnsOrder.records}
            mode={data.status === "awaiting-dns" ? "initial" : "renew"}
            canWrite={canWrite}
          />
        ) : null
      }
      actions={
        canWrite ? (
          <CertificateDetailActions
            targetName={data.alias ?? data.domains.primary ?? "인증서"}
            id={data.id}
            cancelPath={`/api/certificates/${data.id}/cancel`}
            editHref={`/certificates/${data.id}/edit`}
            replaceHref={`/certificates/${data.id}/replace`}
            status={data.status}
            linkedCount={data.linkedServers.length}
            source={data.source}
            challengeType={data.challengeType}
            dnsAutomated={data.dnsProvider === "cloudflare"}
            dnsOrderInFlight={!!data.dnsOrder}
            dnsVerifying={data.dnsOrder?.phase === "verifying"}
            retryAction={{id:"certificates.retryCertificate",args:[]}}
            renewAction={{id:"certificates.renewCertificate",args:[]}}
            renewDnsAction={{id:"certificates.renewDnsCertificate",args:[]}}
            deleteAction={{id:"certificates.deleteCertificate",args:[]}}
          />
        ) : null
      }
    />
  );
}
