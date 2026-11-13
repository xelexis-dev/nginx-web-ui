import { readInstant, type CertificateDetailVM, type CertificateRowVM } from "@/components/certificates/certificate-vm";
export type { CertificateListItemView as CertificateRowResponse, CertificateDetailDataView as CertificateDetailResponse } from "@repo/contracts";
import type { CertificateListItemView as CertificateRowResponse, CertificateDetailDataView as CertificateDetailResponse } from "@repo/contracts";
export function certificateRowPresentation(row:CertificateRowResponse):CertificateRowVM {
  return {...row,expiresAt:readInstant(row.expiresAt)};
}
export function certificateDetailPresentation(row:CertificateDetailResponse):CertificateDetailVM {
  const dnsOrder = row.dnsOrder?.phase === "awaiting-txt"
    ? { phase: row.dnsOrder.phase, records: row.dnsOrder.records.map((record) => {
      if (record.domain === null || record.recordName === null || record.recordValue === null) {
        throw new TypeError("인증서 DNS 레코드가 올바르지 않습니다.");
      }
      return { domain: record.domain, recordName: record.recordName, recordValue: record.recordValue };
    }) }
    : row.dnsOrder;
  return {...row,dnsOrder,issuedAt:readInstant(row.issuedAt),expiresAt:readInstant(row.expiresAt),lastAttemptAt:readInstant(row.lastAttemptAt)};
}
