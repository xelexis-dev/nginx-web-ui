import type { Meta, StoryObj } from "@storybook/react-vite";
import { HistoryList, NODE_HISTORY_LIST_INFO, type HistoryListVM } from "@/components/history/history-list";
import { productFrame, productViewports } from "./frame";

const data: HistoryListVM = {
  showDetails: false,
  items: [
    { id: "snapshot-2", iso: "2026-09-28T04:20:00Z", label: "API upstream 변경" },
    { id: "snapshot-1", iso: "2026-09-27T10:00:00Z", label: null },
  ],
};

function NodeHistoryPreview() {
  return <HistoryList data={data} basePath="/nodes/preview-node/history" info={NODE_HISTORY_LIST_INFO} />;
}

const meta = {
  id: "product-node-history",
  title: "Product/Nodes/History",
  component: NodeHistoryPreview,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [productFrame(() => ({ path: "/nodes/preview-node/history", label: "노드 적용 이력", role: "editor" }))],
} satisfies Meta<typeof NodeHistoryPreview>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = { name: "노드 적용 이력" };
export const Mobile: Story = { name: "노드 적용 이력 · 390", globals: { viewport: { value: "p390", isRotated: false } } };
