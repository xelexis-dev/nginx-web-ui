import { StoryPathContext } from "./adapters/next-navigation";
import { OperationsSectionNav } from "@/components/operations/operations-shell";
import { PROXY_TABS } from "@/app/(app)/nav-links";
import { ServerTemplates } from "@/components/reverse-proxy/server-templates";
import { SERVER_TEMPLATES } from "@/app/(app)/reverse-proxy/server-templates";
import { NodeErrorCard } from "@/components/node-error";
import { PageHeader } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
// Product: the real apps/web reverse-proxy list (components/reverse-proxy/server-list.tsx).
// A switch submits to a Storybook stand-in that saves nothing.
import type { Meta, StoryObj } from "@storybook/react-vite";
import { ServerList } from "@/components/reverse-proxy/server-list";
import {
  SERVER_LIST,
  SERVER_LIST_CERTS_UNKNOWN,
  SERVER_LIST_LOCATIONS,
} from "./fixtures";
import { ProductFrame, productViewports } from "./frame";

const WRITE = {
  newHref: "/reverse-proxy/new",
  templatesHref: "/reverse-proxy/templates",
  toggleAction: { id: "reverse-proxy.toggleServer", args: [] },
};

const meta = {
  id: "product-operations-server-list",
  title: "Product/Operations/리버스 프록시 목록",
  component: ServerList,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [
    (Story, context) => (
      <ProductFrame
        path={String(context.args.basePath)}
        label="리버스 프록시"
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
    navigation: <StoryPathContext.Provider value="/reverse-proxy"><OperationsSectionNav items={PROXY_TABS} label="프록시 구성" /></StoryPathContext.Provider>,
    basePath: "/reverse-proxy",
    data: SERVER_LIST,
    write: WRITE,
  },
} satisfies Meta<typeof ServerList>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Editor: Story = { name: "편집 권한" };
export const Viewer: Story = { name: "보기 전용", args: { write: null } };
export const CertificatesUnknown: Story = {
  name: "인증서를 읽지 못함",
  args: { data: SERVER_LIST_CERTS_UNKNOWN },
};
export const Empty: Story = {
  name: "서버 없음",
  args: { data: { ...SERVER_LIST, items: [], pendingCount: 0 } },
};
export const Mobile: Story = {
  name: "편집 권한 · 390",
  globals: { viewport: { value: "p390", isRotated: false } },
};
export const Locations: Story = {
  name: "location별 대상",
  args: { data: SERVER_LIST_LOCATIONS },
};
export const LocationsMobile: Story = {
  name: "location별 대상 · 390",
  args: { data: SERVER_LIST_LOCATIONS },
  globals: { viewport: { value: "p390", isRotated: false } },
};

export const Node: Story = {
  args: {
    basePath: "/nodes/node-a/reverse-proxy",
    navigation: undefined,
    write: {
      newHref: "/nodes/node-a/reverse-proxy/new",
      templatesHref: null,
      toggleAction: { id: "node.toggleNodeServer", args: ["node-a"] },
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
        retryHref={args.basePath}
        listHref={args.basePath}
        failure={{ kind: "unreachable", message: "노드에 연결할 수 없습니다." }}
      />
    </section>
  ),
};
export const Templates: Story = {
  args: { basePath: "/reverse-proxy/templates" },
  render: (args) => (
    <ServerTemplates
      canWrite={!!args.write}
      navigation={
        <StoryPathContext.Provider value="/reverse-proxy/templates"><OperationsSectionNav items={PROXY_TABS} label="프록시 구성" /></StoryPathContext.Provider>
      }
      items={SERVER_TEMPLATES.map((t) => ({
        id: t.id,
        name: t.name,
        tags: t.tags,
        description: t.description,
      }))}
    />
  ),
};
