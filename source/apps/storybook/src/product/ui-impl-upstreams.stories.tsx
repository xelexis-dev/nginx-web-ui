import { StoryPathContext } from "./adapters/next-navigation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { PageHeader } from "@repo/ui";
import { OperationsSectionNav } from "@/components/operations/operations-shell";
import { PROXY_TABS } from "@/app/(app)/nav-links";
import { UpstreamList } from "@/components/upstreams/upstream-list";
import { UpstreamDetail } from "@/components/upstreams/upstream-detail";
import { UpstreamForm } from "@/components/upstreams/upstream-form";
import { NodeErrorCard } from "@/components/node-error";
import { ProductFrame } from "./frame";
import { SERVER_DETAIL } from "./fixtures";
import layout from "@/components/page-layout.module.css";

type Mode = "list" | "detail" | "new" | "edit";
interface PreviewProps {
  mode: Mode;
  node: boolean;
  offline: boolean;
}
const target = { kind: "node", id: "node-a", name: "서울 운영 노드" } as const;
const upstream = SERVER_DETAIL.upstreams[0]!;
function route({ mode, node }: PreviewProps) {
  const base = `${node ? "/nodes/node-a" : ""}/reverse-proxy/upstreams`;
  return mode === "list"
    ? base
    : mode === "new"
      ? `${base}/new`
      : mode === "edit"
        ? `${base}/${upstream.id}/edit`
        : `${base}/${upstream.id}`;
}
function UpstreamRoute({ mode, node, offline }: PreviewProps) {
  const base = `${node ? "/nodes/node-a" : ""}/reverse-proxy/upstreams`;
  const cancel = mode === "new" ? base : `${base}/${upstream.id}`;
  const title =
    mode === "new"
      ? "새 Upstream"
      : mode === "edit"
        ? "Upstream 수정"
        : "Upstreams";
  if (offline)
    return (
      <section className={layout.page}>
        <PageHeader title={title} />
        <NodeErrorCard
          nodeId={target.id}
          retryHref={route({ mode, node, offline })}
          listHref={base}
          failure={{
            kind: "unreachable",
            message: "노드에 연결할 수 없습니다.",
          }}
        />
      </section>
    );
  if (mode === "list")
    return (
      <UpstreamList
        data={{ items: SERVER_DETAIL.upstreams, canWrite: true }}
        basePath={base}
        navigation={
          !node ? (
            <StoryPathContext.Provider value={base}><OperationsSectionNav items={PROXY_TABS} label="프록시 구성" /></StoryPathContext.Provider>
          ) : undefined
        }
      />
    );
  if (mode === "detail")
    return (
      <UpstreamDetail
        data={upstream}
        runtimeDns={{ available: true, reason: null }}
        canWrite
        basePath={base}
        deleteAction={{
          id: node ? "node.deleteNodeUpstream" : "upstreams.deleteUpstream",
          args: node ? [target.id] : [],
        }}
      />
    );
  return (
    <section className={layout.page}>
      <PageHeader
        title={title}
        info={
          mode === "new" ? "새 upstream 그룹을 등록합니다." : undefined
        }
      />
      <UpstreamForm
        target={node ? target : undefined}
        runtimeDns={{ available: true, reason: null }}
        defaultValues={mode === "edit" ? upstream : undefined}
        cancelHref={cancel}
        action={{
          id: node
            ? mode === "new"
              ? "node.createNodeUpstream"
              : "node.updateNodeUpstream"
            : mode === "new"
              ? "upstreams.createUpstream"
              : "upstreams.updateUpstream",
          args: node ? [target.id] : [],
        }}
      />
    </section>
  );
}
const meta = {
  id: "product-ui-impl-upstreams",
  title: "Product/Operations/Upstreams",
  component: UpstreamRoute,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen" },
  decorators: [
    (Story, context) => (
      <ProductFrame
        path={route(context.args)}
        label="Upstreams"
        role={context.args.offline ? "viewer" : "editor"}
        storyId={context.id}
        viewport={undefined}
        available={!context.args.offline}
        nodeName={target.name}
      >
        <Story />
      </ProductFrame>
    ),
  ],
  args: { mode: "list", node: false, offline: false },
} satisfies Meta<typeof UpstreamRoute>;
export default meta;
type Story = StoryObj<typeof meta>;
export const List: Story = {};
export const Detail: Story = { args: { mode: "detail" } };
export const New: Story = { args: { mode: "new" } };
export const Edit: Story = { args: { mode: "edit" } };
export const NodeList: Story = { args: { mode: "list", node: true } };
export const NodeDetail: Story = { args: { mode: "detail", node: true } };
export const NodeNew: Story = { args: { mode: "new", node: true } };
export const NodeEdit: Story = { args: { mode: "edit", node: true } };
export const NodeListOffline: Story = {
  args: { mode: "list", node: true, offline: true },
};
export const NodeDetailOffline: Story = {
  args: { mode: "detail", node: true, offline: true },
};
export const NodeNewOffline: Story = {
  args: { mode: "new", node: true, offline: true },
};
export const NodeEditOffline: Story = {
  args: { mode: "edit", node: true, offline: true },
};
