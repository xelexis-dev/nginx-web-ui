import Link from "next/link";
import { SectionNav } from "@repo/ui";
import { PROXY_TABS } from "@/app/(app)/nav-links";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { UpstreamList } from "@/components/upstreams/upstream-list";
import { CertificateList } from "@/components/certificates/certificate-list";
import { AuditList } from "@/components/audit/audit-list";
import { AccountTokensView } from "@/app/(app)/account/tokens/token-list";
import { productFrame, productViewports } from "./frame";

const pages = {
  upstream: { path: "/reverse-proxy/upstreams", title: "Upstreams" },
  certificate: { path: "/certificates", title: "인증서" },
  audit: { path: "/audit", title: "감사 로그" },
  token: { path: "/account/tokens", title: "액세스 토큰" },
} as const;
function ResourceList({ kind }: { kind: keyof typeof pages }) {
  if (kind === "upstream") return <UpstreamList navigation={<SectionNav items={PROXY_TABS} currentPath="/reverse-proxy/upstreams" label="프록시 구성" renderLink={link => <Link {...link} />} />} basePath="/reverse-proxy/upstreams" data={{ canWrite: true, items: [
    { id: "api-pool", name: "api_pool", servers: ["api-1.example.test:8080", "api-2.example.test:8080"] },
    { id: "files", name: "files", servers: ["files.example.test:9100"] },
  ] }} />;
  if (kind === "certificate") return <CertificateList basePath="/certificates" write={{ newHref: "/certificates/new", importHref: "/certificates/import" }} data={{ items: [
    { id: "cert-api", alias: "API 서비스", domains: { primary: "api.example.test", all: "api.example.test", extra: 0 }, status: "issued", source: "acme", staging: false, expiresAt: new Date("2030-01-01T00:00:00Z"), linkedCount: 2 },
    { id: "cert-files", alias: null, domains: { primary: "files.example.test", all: "files.example.test", extra: 0 }, status: "failed", source: "acme", staging: false, expiresAt: null, linkedCount: 1 },
  ] }} />;
  if (kind === "audit") return <AuditList basePath="/audit" data={{ total: 3, page: 1, totalPages: 1, actions: ["nginx.reload", "server.update", "auth.login"], actionFilter: "", items: [
    { id: "audit-1", time: "2026. 10. 2. 오후 2:20:00", actorEmail: "operator@example.test", actorLabel: null, action: "nginx.reload", target: "local" },
    { id: "audit-2", time: "2026. 10. 2. 오후 2:18:00", actorEmail: "operator@example.test", actorLabel: null, action: "server.update", target: "api.example.test" },
    { id: "audit-3", time: "2026. 10. 2. 오후 2:10:00", actorEmail: "operator@example.test", actorLabel: null, action: "auth.login", target: null },
  ] }} />;
  return <AccountTokensView rows={[
    { id: "preview-token", name: "지표 수집", masked: "nwui_…demo", scopes: ["metrics:read"], status: "active", createdAt: "2026. 10. 1.", lastUsedAt: "2026. 10. 2.", expiresAt: "2026. 12. 30.", revokedAt: "—", revoked: false },
  ]} />;
}
const meta = {
  id: "product-resource-lists", title: "Product/리소스 목록", component: ResourceList, tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [productFrame((args) => { const page = pages[args.kind as keyof typeof pages] ?? pages.upstream; return { path: page.path, label: page.title, role: "editor" }; })],
  args: { kind: "upstream" },
} satisfies Meta<typeof ResourceList>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Upstream: Story = { name: "업스트림" };
export const Certificate: Story = { name: "인증서", args: { kind: "certificate" } };
export const Audit: Story = { name: "감사 로그", args: { kind: "audit" } };
export const Token: Story = { name: "계정 · 액세스 토큰", args: { kind: "token" } };
export const NarrowAudit: Story = { name: "감사 로그 · 390", args: { kind: "audit" }, globals: { viewport: { value: "p390", isRotated: false } } };
