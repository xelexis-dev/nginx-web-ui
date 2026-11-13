import type { Meta, StoryObj } from "@storybook/react-vite";
import { UpstreamDetail } from "@/components/upstreams/upstream-detail";
import { StreamDetail } from "@/components/streams/stream-detail";
import { CertificateDetail } from "@/components/certificates/certificate-detail";
import { CertificateDetailActions } from "@/components/certificates/certificate-detail-actions";
import { AuditDetail } from "@/components/audit/audit-detail";
import { productFrame, productViewports } from "./frame";

const pages = {
  upstream: { path: "/reverse-proxy/upstreams/api-pool", title: "api_pool" },
  stream: { path: "/stream/database", title: "listen 15432" },
  certificate: { path: "/certificates/cert-api", title: "API 서비스 인증서" },
  audit: { path: "/audit/audit-1", title: "server.update" },
} as const;
function ResourceDetail({ kind, canWrite, unresolvedPreview }: { kind: keyof typeof pages; canWrite: boolean; unresolvedPreview?: boolean }) {
  if (kind === "upstream") return <UpstreamDetail runtimeDns={{ available: true, reason: null }} basePath="/reverse-proxy/upstreams" canWrite={canWrite} deleteAction={{ id: "upstreams.deleteUpstream", args: [] }}
    data={{ id: "api-pool", name: "api_pool", servers: ["api-1.example.test:8080", "api-2.example.test:8080"] }} />;
  if (kind === "stream") return <StreamDetail runtimeDns={{ available: true, reason: null }} basePath="/stream" canWrite={canWrite}
    toggleAction={{ id: "stream.toggleStream", args: [] }} deleteAction={{ id: "stream.deleteStream", args: [] }}
    data={{ id: "database", listen: 15432, protocol: "tcp", proxyPass: "database.example.test:5432", enabled: true,
      preview: { config: "server {\n    listen 15432;\n    proxy_pass database.example.test:5432;\n}\n", error: null, ...(unresolvedPreview ? { unresolvedPaths: ["stream"] } : {}) } }} />;
  if (kind === "audit") return <AuditDetail basePath="/audit" data={{ action: "server.update", time: "2026. 10. 2. 오후 2:18:00", actorEmail: "operator@example.test", actorLabel: null, target: "api.example.test", ip: "192.0.2.10", beforeJson: JSON.stringify({ serverName: "api.example.test", proxyPass: "http://api-1.example.test:8080" }, null, 2), afterJson: JSON.stringify({ serverName: "api.example.test", upstream: "api_pool" }, null, 2) }} />;
  return <CertificateDetail basePath="/certificates" serverBasePath="/reverse-proxy" uncovered={[]} dnsPanel={null}
    data={{ id: "cert-api", alias: "API 서비스 인증서", domains: { primary: "api.example.test", all: "api.example.test, www.api.example.test", extra: 1 }, status: "issued", source: "acme", staging: false, challengeType: "http-01", dnsProvider: null,
      issuedAt: new Date("2026-10-01T04:00:00Z"), expiresAt: new Date("2026-12-30T04:00:00Z"), lastAttemptAt: new Date("2026-10-01T04:00:00Z"), failureCount: 0, lastError: null, dnsOrder: null,
      issueLog: [{ at: "2026-10-01 04:00:00", level: "info", msg: "인증서 발급을 완료했습니다." }], linkedServers: [{ id: "srv-api", serverName: "api.example.test", listen: 443 }] }}
    actions={canWrite ? <CertificateDetailActions id="cert-api" cancelPath="/storybook/certificates/cert-api/cancel" editHref="/certificates/cert-api/edit" replaceHref="/certificates/cert-api/replace" status="issued" linkedCount={1} source="acme" challengeType="http-01" dnsAutomated={false} dnsOrderInFlight={false} dnsVerifying={false}
      retryAction={{ id: "certificates.retryCertificate", args: [] }} renewAction={{ id: "certificates.renewCertificate", args: [] }} renewDnsAction={{ id: "certificates.renewDnsCertificate", args: [] }} deleteAction={{ id: "certificates.deleteCertificate", args: [] }} /> : null} />;
}
const meta = {
  id: "product-resource-details", title: "Product/리소스 상세", component: ResourceDetail, tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [productFrame((args) => { const page = pages[args.kind as keyof typeof pages] ?? pages.upstream; return { path: page.path, label: page.title, role: args.canWrite ? "editor" : "viewer" }; })],
  args: { kind: "upstream", canWrite: true },
} satisfies Meta<typeof ResourceDetail>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Upstream: Story = { name: "업스트림 상세" };
export const Stream: Story = { name: "Stream 상세", args: { kind: "stream" } };
export const Certificate: Story = { name: "인증서 상세", args: { kind: "certificate" } };
export const CertificateViewer: Story = { name: "인증서 상세 · 보기 전용", args: { kind: "certificate", canWrite: false } };
export const Audit: Story = { name: "감사 상세", args: { kind: "audit" } };
export const NarrowStream: Story = { name: "Stream 상세 · 390", args: { kind: "stream" }, globals: { viewport: { value: "p390", isRotated: false } } };

export const StreamUnresolvedPreview: Story = { name: "Stream · 이름 확인 없는 미리보기", args: { kind: "stream", unresolvedPreview: true } };
