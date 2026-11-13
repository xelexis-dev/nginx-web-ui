import type { Meta, StoryObj } from "@storybook/react-vite";
import { HistoryList, type HistoryListVM } from "@/components/history/history-list";
import { NodeHistoryActions } from "@/app/(node)/nodes/[id]/history/rollback-button";
import { productFrame, productViewports } from "./frame";

const data: HistoryListVM = {
  showDetails: false,
  items: [
    { id: "snapshot-2", iso: "2026-09-28T04:20:00Z", label: "API upstream 변경" },
    { id: "snapshot-1", iso: "2026-09-27T10:00:00Z", label: null },
  ],
};

function NodeHistoryPreview({ canRollback }: { canRollback: boolean }) {
  return <HistoryList data={data}
    info="이 노드에 적용된 설정 스냅샷 이력입니다. 게이트웨이는 스냅샷 내용을 읽을 수 없어 개수·작성자 열은 표시하지 않습니다."
    notice="스냅샷 내용은 노드에서만 볼 수 있습니다."
    renderRowAction={row => <NodeHistoryActions nodeId="preview-node" nodeName="미리보기 노드"
      nodeUrl="https://node.example.test" canRollback={canRollback}
      snapshotId={row.id} label={row.label} iso={row.iso} retryHref="/nodes/preview-node/history" />} />;
}

const meta = {
  id: "product-node-history",
  title: "Product/Nodes/History",
  component: NodeHistoryPreview,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [productFrame(args => ({ path: "/nodes/preview-node/history", label: "노드 적용 이력", role: args.canRollback ? "editor" : "viewer" }))],
  args: { canRollback: true },
} satisfies Meta<typeof NodeHistoryPreview>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = { name: "노드 적용 이력" };
export const Viewer: Story = { name: "보기 전용", args: { canRollback: false } };
export const Mobile: Story = { name: "노드 적용 이력 · 390", globals: { viewport: { value: "p390", isRotated: false } } };
