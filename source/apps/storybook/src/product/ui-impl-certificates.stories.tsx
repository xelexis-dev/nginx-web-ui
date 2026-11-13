// The twenty P3-3 route compositions use the real shared page bodies. API loaders are
// represented by isolated fixtures; no action sends a request from Storybook.
import type { Meta, StoryObj } from "@storybook/react-vite";
import { PageHeader, buttonClassName } from "@repo/ui";
import Link from "next/link";
import type { OperationsTarget } from "@repo/presentation/operations-target";
import { StreamList } from "@/components/streams/stream-list";
import { StreamDetail } from "@/components/streams/stream-detail";
import { StreamForm } from "@/components/streams/stream-form";
import { CertificateList } from "@/components/certificates/certificate-list";
import { CertificateDetail } from "@/components/certificates/certificate-detail";
import { CertificateDetailActions } from "@/components/certificates/certificate-detail-actions";
import { CertificateForm } from "@/components/certificates/certificate-form";
import { CertificateEditForm } from "@/components/certificates/certificate-edit-form";
import { ImportForm } from "@/components/certificates/import-form";
import { NodeErrorCard } from "@/components/node-error";
import type { CertificateDetailVM } from "@/components/certificates/certificate-vm";
import layout from "@/components/page-layout.module.css";
import { ProductFrame, productViewports } from "./frame";

const routes = {
  streamList: ["/stream", "Stream"], streamDetail: ["/stream/database", "listen 15432"],
  streamEdit: ["/stream/database/edit", "Stream 수정"], streamNew: ["/stream/new", "새 Stream"],
  certificateList: ["/certificates", "인증서"], certificateDetail: ["/certificates/cert-api", "API 서비스 인증서"],
  certificateEdit: ["/certificates/cert-api/edit", "인증서 수정"], certificateReplace: ["/certificates/cert-api/replace", "인증서 교체"],
  certificateImport: ["/certificates/import", "인증서 가져오기"], certificateNew: ["/certificates/new", "새 인증서"],
} as const;
type Route = keyof typeof routes;
const nodeName = "서울 운영 노드 — 긴 이름도 저장 대상을 가리지 않습니다";
const stream = { id: "database", listen: 15432, protocol: "tcp" as const, proxyPass: "database.example.test:5432", enabled: true,
  preview: { config: "server {\n    listen 15432;\n    proxy_pass database.example.test:5432;\n}\n", error: null } };
const certificate: CertificateDetailVM = {
  id: "cert-api", alias: "API 서비스 인증서", domains: { primary: "api.example.test", all: "api.example.test, www.api.example.test", extra: 1 },
  source: "acme", status: "issued", staging: false, challengeType: "http-01", dnsProvider: null,
  issuedAt: new Date("2026-10-01T04:00:00Z"), expiresAt: new Date("2026-12-30T04:00:00Z"), lastAttemptAt: new Date("2026-10-01T04:00:00Z"),
  failureCount: 0, lastError: null, dnsOrder: null,
  issueLog: [{ at: "2026-10-01 04:00:00", level: "info", msg: "인증서 발급을 완료했습니다." }], linkedServers: [],
};
function RoutePage({ route, node, offline, viewer, linkedCount }: { route: Route; node: boolean; offline: boolean; viewer: boolean; linkedCount: number }) {
  const prefix = node ? "/nodes/preview-node" : "";
  const path = prefix + routes[route][0];
  const target: OperationsTarget = node ? { kind: "node", id: "preview-node", name: nodeName } : { kind: "local" };
  const streamBase = prefix + "/stream", certBase = prefix + "/certificates";
  if (offline) return <section className={layout.page}><PageHeader title={routes[route][1]} /><NodeErrorCard nodeId="preview-node" retryHref={path} listHref="/nodes" failure={{ kind: "unreachable", message: "노드에 연결할 수 없습니다." }} /></section>;
  if (route === "streamList") return <StreamList basePath={streamBase} data={{ items: [stream, { id: "resolver", listen: 15353, protocol: "udp", proxyPass: "resolver.example.test:53", enabled: false }], canWrite: !viewer }} />;
  if (route === "streamDetail") return <StreamDetail basePath={streamBase} data={stream} canWrite={!viewer} toggleAction={node ? { id: "node.toggleNodeStream", args: ["preview-node"] } : { id: "stream.toggleStream", args: [] }} deleteAction={node ? { id: "node.deleteNodeStream", args: ["preview-node"] } : { id: "stream.deleteStream", args: [] }} />;
  if (route === "certificateList") return <CertificateList basePath={certBase} data={{ items: [{ ...certificate, linkedCount }] }} write={viewer ? null : { newHref: certBase + "/new", importHref: certBase + "/import" }} />;
  if (route === "certificateDetail") return <CertificateDetail basePath={certBase} serverBasePath={prefix + "/reverse-proxy"} data={{ ...certificate, linkedServers: linkedCount ? [{ id: "srv-api", serverName: "api.example.test", listen: 443 }] : [] }} uncovered={[]} dnsPanel={null}
    actions={viewer ? null : <CertificateDetailActions id={certificate.id} targetName={certificate.alias ?? ""} cancelPath={certBase + "/cert-api/cancel"} editHref={certBase + "/cert-api/edit"} replaceHref={certBase + "/cert-api/replace"} status="issued" linkedCount={linkedCount} source="acme" challengeType="http-01" dnsAutomated={false} dnsOrderInFlight={false} dnsVerifying={false}
      retryAction={node ? { id: "node.retryNodeCertificate", args: ["preview-node"] } : { id: "certificates.retryCertificate", args: [] }} renewAction={node ? { id: "node.renewNodeCertificate", args: ["preview-node"] } : { id: "certificates.renewCertificate", args: [] }} renewDnsAction={node ? { id: "node.renewDnsNodeCertificate", args: ["preview-node"] } : { id: "certificates.renewDnsCertificate", args: [] }} deleteAction={node ? { id: "node.deleteNodeCertificate", args: ["preview-node"] } : { id: "certificates.deleteCertificate", args: [] }} />} />;
  const cancelHref = route.startsWith("stream") ? streamBase + (route === "streamEdit" ? "/database" : "") : certBase + (route === "certificateEdit" ? "/cert-api" : "");
  return <section className={layout.page}>
    <PageHeader title={routes[route][1]} actions={route.startsWith("certificate") ? undefined : <Link href={cancelHref} className={buttonClassName({ variant: "secondary" })}>취소</Link>} />
    {route === "streamNew" || route === "streamEdit" ? <StreamForm target={target} action={node ? { id: route === "streamEdit" ? "node.updateNodeStream" : "node.createNodeStream", args: ["preview-node"] } : { id: route === "streamEdit" ? "stream.updateStream" : "stream.createStream", args: [] }} cancelHref={cancelHref} defaultValues={route === "streamEdit" ? { ...stream, version: "2026-10-31T12:00:00Z" } : undefined} reservedListenPorts={node ? undefined : [80, 3000, 3001]} /> : null}
    {route === "certificateNew" ? <CertificateForm target={target} action={node ? { id: "node.createNodeCertificate", args: ["preview-node"] } : { id: "certificates.createCertificate", args: [] }} defaultEmail="operator@example.test" cancelHref={cancelHref} /> : null}
    {route === "certificateEdit" ? <CertificateEditForm target={target} action={node ? { id: "node.updateNodeCertificate", args: ["preview-node"] } : { id: "certificates.updateCertificate", args: [] }} id="cert-api" cancelHref={cancelHref} isIssued hasStoredToken defaults={{ alias: certificate.alias ?? "", domains: ["api.example.test"], email: "operator@example.test", staging: false, challengeType: "dns-01", dnsProvider: "cloudflare" }} /> : null}
    {route === "certificateReplace" || route === "certificateImport" ? <ImportForm target={target} action={node ? { id: route === "certificateReplace" ? "node.replaceNodeCertificate" : "node.importNodeCertificate", args: ["preview-node"] } : { id: route === "certificateReplace" ? "certificates.replaceCertificate" : "certificates.importCertificate", args: [] }} certId={route === "certificateReplace" ? "cert-api" : undefined} cancelHref={cancelHref} /> : null}
  </section>;
}
const meta = {
  id: "product-ui-impl-certificates", title: "Product/P3 Stream and certificates", component: RoutePage, tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [(Story, context) => <ProductFrame path={(context.args.node ? "/nodes/preview-node" : "") + routes[context.args.route as Route][0]} label={routes[context.args.route as Route][1]} role={context.args.viewer ? "viewer" : "editor"} storyId={context.id} viewport={undefined} nodeName={nodeName} available={!context.args.offline}><Story /></ProductFrame>],
  args: { route: "streamList", node: false, offline: false, viewer: false, linkedCount: 0 },
} satisfies Meta<typeof RoutePage>;
export default meta;
type Story = StoryObj<typeof meta>;
export const LocalStreamList: Story = { args: { route: "streamList", node: false } };
export const LocalStreamDetail: Story = { args: { route: "streamDetail", node: false } };
export const LocalStreamEdit: Story = { args: { route: "streamEdit", node: false } };
export const LocalStreamNew: Story = { args: { route: "streamNew", node: false } };
export const LocalCertificateList: Story = { args: { route: "certificateList", node: false } };
export const LocalCertificateDetail: Story = { args: { route: "certificateDetail", node: false } };
export const LocalCertificateEdit: Story = { args: { route: "certificateEdit", node: false } };
export const LocalCertificateReplace: Story = { args: { route: "certificateReplace", node: false } };
export const LocalCertificateImport: Story = { args: { route: "certificateImport", node: false } };
export const LocalCertificateNew: Story = { args: { route: "certificateNew", node: false } };
export const NodeStreamList: Story = { args: { route: "streamList", node: true } };
export const NodeStreamDetail: Story = { args: { route: "streamDetail", node: true } };
export const NodeStreamEdit: Story = { args: { route: "streamEdit", node: true } };
export const NodeStreamNew: Story = { args: { route: "streamNew", node: true } };
export const NodeCertificateList: Story = { args: { route: "certificateList", node: true } };
export const NodeCertificateDetail: Story = { args: { route: "certificateDetail", node: true } };
export const NodeCertificateEdit: Story = { args: { route: "certificateEdit", node: true } };
export const NodeCertificateReplace: Story = { args: { route: "certificateReplace", node: true } };
export const NodeCertificateImport: Story = { args: { route: "certificateImport", node: true } };
export const NodeCertificateNew: Story = { args: { route: "certificateNew", node: true } };
export const OfflineStreamList: Story = { args: { route: "streamList", node: true, offline: true } };
export const OfflineStreamDetail: Story = { args: { route: "streamDetail", node: true, offline: true } };
export const OfflineStreamEdit: Story = { args: { route: "streamEdit", node: true, offline: true } };
export const OfflineStreamNew: Story = { args: { route: "streamNew", node: true, offline: true } };
export const OfflineCertificateList: Story = { args: { route: "certificateList", node: true, offline: true } };
export const OfflineCertificateDetail: Story = { args: { route: "certificateDetail", node: true, offline: true } };
export const OfflineCertificateEdit: Story = { args: { route: "certificateEdit", node: true, offline: true } };
export const OfflineCertificateReplace: Story = { args: { route: "certificateReplace", node: true, offline: true } };
export const OfflineCertificateImport: Story = { args: { route: "certificateImport", node: true, offline: true } };
export const OfflineCertificateNew: Story = { args: { route: "certificateNew", node: true, offline: true } };
export const ViewerStream: Story = { args: { route: "streamList", viewer: true } };
export const ViewerCertificate: Story = { args: { route: "certificateDetail", viewer: true } };
export const LinkedCertificate: Story = { args: { route: "certificateDetail", linkedCount: 1 } };
