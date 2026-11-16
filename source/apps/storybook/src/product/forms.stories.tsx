// Real local/node form components. Actions use Storybook's existing API stand-in;
// no request, credential, certificate issuance or nginx change is made.
import type { Meta, StoryObj } from "@storybook/react-vite";
import { PageHeader } from "@repo/ui";
import { UpstreamForm } from "@/components/upstreams/upstream-form";
import { StreamForm } from "@/components/streams/stream-form";
import { CertificateEditForm } from "@/components/certificates/certificate-edit-form";
import { CertificateForm } from "@/components/certificates/certificate-form";
import layout from "@/components/page-layout.module.css";
import { productFrame, productViewports } from "./frame";

const pages = {
  upstream: { path: "/reverse-proxy/upstreams/new", title: "새 Upstream" },
  stream: { path: "/stream/new", title: "새 Stream" },
  certificateEdit: { path: "/certificates/preview/edit", title: "인증서 수정" },
  certificate: { path: "/certificates/new", title: "새 인증서" },
} as const;
function ResourceForm({ kind }: { kind: keyof typeof pages }) {
  return <section className={layout.page}>
    <PageHeader title={pages[kind].title} />
    {kind === "upstream" ? <UpstreamForm runtimeDns={{ available: true, reason: null }} action={{ id: "upstreams.createUpstream", args: [] }} cancelHref="/reverse-proxy/upstreams" /> : null}
    {kind === "stream" ? <StreamForm runtimeDns={{ available: true, reason: null }} action={{ id: "stream.createStream", args: [] }} cancelHref="/stream" reservedListenPorts={[80, 3000, 3001]} /> : null}
    {kind === "certificateEdit" ? <CertificateEditForm action={{ id: "certificates.updateCertificate", args: [] }}
      id="preview" cancelHref="/certificates/preview" isIssued hasStoredToken={false}
      defaults={{ domains: ["app.example.test"], alias: "서비스 인증서", email: "operator@example.test",
        staging: false, challengeType: "dns-01", dnsProvider: "" }} /> : null}
    {kind === "certificate" ? <CertificateForm action={{ id: "certificates.createCertificate", args: [] }} defaultEmail="operator@example.test" cancelHref="/certificates" /> : null}
  </section>;
}
const meta = {
  id: "product-resource-forms", title: "Product/리소스 폼", component: ResourceForm, tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [productFrame((args) => { const page = pages[args.kind as keyof typeof pages] ?? pages.upstream; return { path: page.path, label: page.title, role: "editor" }; })],
  args: { kind: "upstream" },
} satisfies Meta<typeof ResourceForm>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Upstream: Story = { name: "업스트림" };
export const Stream: Story = { name: "Stream", args: { kind: "stream" } };
export const Certificate: Story = { name: "인증서 발급", args: { kind: "certificate" } };
export const Narrow: Story = { name: "인증서 발급 · 390", args: { kind: "certificate" }, globals: { viewport: { value: "p390", isRotated: false } } };

export const CertificateEdit: Story = { name: "인증서 수정", args: { kind: "certificateEdit" } };
