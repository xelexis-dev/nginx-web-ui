import { PageHeader } from "@repo/ui";
import { NodeErrorCard } from "@/components/node-error";
import layout from "@/components/page-layout.module.css";
// Product: the real apps/web server detail (components/reverse-proxy/server-detail.tsx).
import type { Meta, StoryObj } from "@storybook/react-vite";
import { ServerDetail } from "@/components/reverse-proxy/server-detail";
import { SERVER_DETAIL, PREVIEW_UNRESOLVED_PATHS } from "./fixtures";
import { ProductFrame, productViewports } from "./frame";

const WRITE = {
  editHref: "/reverse-proxy/srv-api/edit",
  toggleAction: { id: "reverse-proxy.toggleServer", args: [] },
  deleteAction: { id: "reverse-proxy.deleteServer", args: [] },
};

const meta = {
  id: "product-operations-server-detail",
  title: "Product/Operations/서버 상세",
  component: ServerDetail,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [
    (Story, context) => (
      <ProductFrame
        path={`${String(context.args.basePath)}/srv-api`}
        label="api.example.test"
        role={context.args.write ? "editor" : "viewer"}
        storyId={context.id}
        viewport={undefined}
        available={!context.parameters.offline}
        nodeName="서울 운영 노드"
      >
        <Story />
      </ProductFrame>
    ),
  ],
  args: {
    basePath: "/reverse-proxy",
    data: SERVER_DETAIL,
    write: WRITE,
    configOrigin: "this-instance",
  },
} satisfies Meta<typeof ServerDetail>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = { name: "서버 상세" };
export const Viewer: Story = { name: "보기 전용", args: { write: null } };
export const CertificateDeleted: Story = {
  name: "인증서가 삭제됨",
  args: { data: { ...SERVER_DETAIL, cert: null } },
};
export const NodePreviewError: Story = {
  name: "노드 · 미리보기 없음",
  args: {
    basePath: "/nodes/node-a/reverse-proxy",
    configOrigin: "remote-node",
    data: { ...SERVER_DETAIL, preview: undefined },
  },
};
export const Mobile: Story = {
  name: "서버 상세 · 390",
  globals: { viewport: { value: "p390", isRotated: false } },
};

export const UnresolvedPreview: Story = {
  name: "이름 확인 없는 미리보기",
  args: {
    data: {
      ...SERVER_DETAIL,
      preview: {
        ...SERVER_DETAIL.preview!,
        unresolvedPaths: PREVIEW_UNRESOLVED_PATHS,
      },
    },
  },
};

export const Node: Story = {
  args: {
    basePath: "/nodes/node-a/reverse-proxy",
    configOrigin: "remote-node",
    write: {
      editHref: "/nodes/node-a/reverse-proxy/srv-api/edit",
      toggleAction: { id: "node.toggleNodeServer", args: ["node-a"] },
      deleteAction: { id: "node.deleteNodeServer", args: ["node-a"] },
    },
  },
};
export const NodeOffline: Story = {
  ...Node,
  parameters: { offline: true },
  args: { ...Node.args, write: null },
  render: (args) => (
    <section className={layout.page}>
      <PageHeader title="리버스 프록시" />
      <NodeErrorCard
        nodeId="node-a"
        retryHref={`${args.basePath}/srv-api`}
        listHref={args.basePath}
        failure={{ kind: "unreachable", message: "노드에 연결할 수 없습니다." }}
      />
    </section>
  ),
};
