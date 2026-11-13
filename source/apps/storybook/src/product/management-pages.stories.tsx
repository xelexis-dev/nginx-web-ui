import type { Meta, StoryObj } from "@storybook/react-vite";
import { APP_SEMVER } from "@/lib/version";
import type { NodeView } from "@repo/contracts";
import { AccountOverview } from "@/app/(app)/account/account-overview";
import { NodeList } from "@/app/(app)/nodes/node-list";
import { NodeOverview } from "@/app/(node)/nodes/[id]/node-overview";
import { UserList } from "@/app/admin/users/user-list";
import { productFrame, productViewports } from "./frame";

const node: NodeView = {
  id: "preview-node", name: "운영 노드", baseUrl: "https://node.example.test", tokenRole: "editor", allowInsecureTls: false,
  instanceId: "preview-instance", gatewayName: "메인 게이트웨이", createdByUserId: "operator", createdAt: "2026-10-01T05:00:00Z",
  lastSeenAt: "2026-10-02T05:00:00Z", lastStatus: "ok", lastVersion: APP_SEMVER, lastError: null,
};
const pages = {
  nodes: { path: "/nodes", title: "노드" },
  node: { path: "/nodes/preview-node", title: "운영 노드" },
  users: { path: "/admin/users", title: "사용자" },
  account: { path: "/account", title: "계정" },
} as const;
function ManagementPage({ kind, canWrite = true }: { kind: keyof typeof pages; canWrite?: boolean }) {
  if (kind === "nodes") return <NodeList canWrite={canWrite} unpairWarningText={null} nodes={[node, { ...node, id: "new-node", name: "신규 노드", baseUrl: "https://new-node.example.test", lastSeenAt: null, lastStatus: null, lastVersion: null }]} />;
  if (kind === "node") return <NodeOverview id={node.id} result={{ node, canManage: canWrite, ok: true,
    data: { semver: APP_SEMVER, counts: { servers: 5, streams: 2, upstreams: 3 }, pending: { count: 2, hasBaseline: true }, certificates: { total: 4, expiringSoon: 1 }, nginx: { configOk: true, output: "nginx: the configuration file syntax is ok\nnginx: configuration file test is successful" } } }} />;
  if (kind === "users") return <UserList selfUserId="operator" items={[
    { id: "operator", email: "operator@example.test", displayName: "운영 관리자", role: "admin", enabled: true, approved: true, methods: "비밀번호", createdAt: "2026. 10. 1. 오후 2:00" },
    { id: "editor", email: "editor@example.test", displayName: null, role: "editor", enabled: true, approved: true, methods: "비밀번호 · 회사 로그인", createdAt: "2026. 10. 1. 오후 2:10" },
    { id: "pending", email: "viewer@example.test", displayName: null, role: "viewer", enabled: true, approved: false, methods: "회사 로그인", createdAt: "2026. 10. 2. 오후 2:00" },
  ]} />;
  return <AccountOverview account={{ email: "operator@example.test", hasPassword: true, totpEnabled: true, logins: [
    { providerId: "company", name: "회사 로그인", linkedAt: "2026-10-01T05:00:00Z" },
  ] }} />;
}
const meta = {
  id: "product-management-pages", title: "Product/관리·계정·노드 화면", component: ManagementPage, tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [productFrame((args) => { const page = pages[args.kind as keyof typeof pages] ?? pages.nodes; return { path: page.path, label: page.title, role: args.kind === "users" ? "admin" : args.canWrite ? "editor" : "viewer" }; })],
  args: { kind: "nodes", canWrite: true },
} satisfies Meta<typeof ManagementPage>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Nodes: Story = { name: "노드 목록" };
export const Node: Story = { name: "노드 개요", args: { kind: "node" } };
export const NodeViewer: Story = { name: "노드 개요 · 보기 전용", args: { kind: "node", canWrite: false } };
export const Users: Story = { name: "관리자 · 사용자 목록", args: { kind: "users" } };
export const Account: Story = { name: "계정 개요", args: { kind: "account" } };
export const NarrowAccount: Story = { name: "계정 개요 · 390", args: { kind: "account" }, globals: { viewport: { value: "p390", isRotated: false } } };
