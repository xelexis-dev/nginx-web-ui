import { expect, it } from "vitest";
import type { CertificateDetailResponse, CertificateRowResponse } from "./certificate-api";
import { certificateDetailPresentation, certificateRowPresentation } from "./certificate-api";

const row: CertificateRowResponse = {
  id: "c1", domains: { primary: "example.test", all: "example.test", extra: 0 },
  alias: null, status: "issued", source: "acme", staging: false,
  expiresAt: "2026-12-01T00:00:00.000Z", linkedCount: 1,
};
const detail: CertificateDetailResponse = {
  id: "c1", domains: row.domains, alias: null, status: "issued", source: "acme", staging: false,
  challengeType: "dns-01", dnsProvider: null, lastError: null,
  issuedAt: "2026-09-01T00:00:00.000Z", expiresAt: row.expiresAt, lastAttemptAt: null,
  renewalNoticeSentAt: null, createdAt: "2026-09-01T00:00:00.000Z", failureCount: 0,
  hasDnsToken: false, acmeEmail: "ops@example.test", dnsRecords: null, dnsSubmittedAt: null,
  issueLog: [{ at: "2026-09-01 00:00:00", level: "info", msg: "발급 완료" }],
  linkedServers: [{ id: "s1", serverName: "example.test", listen: 443 }],
  dnsOrder: { phase: "awaiting-txt", records: [{ domain: "example.test", recordName: "_acme-challenge.example.test", recordValue: "proof" }] },
};

it("API certificate dates become instants while display domains and log times remain unchanged", () => {
  const result = certificateRowPresentation(row);
  expect(result.expiresAt?.toISOString()).toBe(row.expiresAt);
  expect(result.domains).toEqual(row.domains);
  const value = certificateDetailPresentation(detail);
  expect(value.issuedAt?.toISOString()).toBe(detail.issuedAt);
  expect(value.lastAttemptAt).toBeNull();
  expect(value.issueLog).toEqual(detail.issueLog);
  expect(value.dnsOrder).toEqual(detail.dnsOrder);
});

it.each(["domain", "recordName", "recordValue"] as const)("incomplete DNS %s is rejected before the record reaches the TXT table", key => {
  expect(() => certificateDetailPresentation({ ...detail, dnsOrder: {
    phase: "awaiting-txt", records: [{ domain: "example.test", recordName: "_acme-challenge.example.test", recordValue: "proof", [key]: null }],
  } })).toThrow();
});
