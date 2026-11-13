// Product: the real apps/web overview component (components/operations/overview.tsx) with sample
// values of its production view model. The app reads these values from the API (lib/overview-read).
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { OperationsOverview } from "@/components/operations/overview";
import { OVERVIEW, OVERVIEW_DEGRADED, OVERVIEW_VIEWER } from "./fixtures";
import { productFrame, productViewports } from "./frame";

const meta = {
  id: "product-operations-overview",
  title: "Product/Operations/개요",
  component: OperationsOverview,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [
    productFrame((args) => {
      const { servers } = (args as { vm: typeof OVERVIEW }).vm;
      return { path: "/", label: "개요", role: servers.ok && servers.data.canWrite ? "editor" : "viewer" };
    }),
  ],
} satisfies Meta<typeof OperationsOverview>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = { name: "개요", args: { vm: OVERVIEW }, play: async ({ canvasElement }) => {
  const canvas = within(canvasElement);
  await expect(canvasElement.querySelectorAll('dl[data-columns="4"] > div')).toHaveLength(4);
  if (!OVERVIEW.nodes.ok) throw new Error("node fixture missing");
  await expect(canvas.getByText(`연결된 노드 ${OVERVIEW.nodes.data.length}대`)).toBeVisible();
  await expect(canvas.getByRole("link", { name: "노드 관리" })).toHaveAttribute("href", "/nodes");
  for (const node of OVERVIEW.nodes.data) await expect(canvas.queryByText(node.name)).toBeNull();
  await expect(canvasElement.querySelectorAll('a[href^="/nodes/"]')).toHaveLength(0);
} };
/** nginx 수치 없음, 인증서·노드·이벤트 읽기 실패(그 칸에 이유). 나머지는 그대로 보인다. */
/** 보기 전용(viewer): 쓰기·적용 없이 같은 요약. */
export const Viewer: Story = { name: "보기 전용", args: { vm: OVERVIEW_VIEWER } };
export const Degraded: Story = { name: "보조 읽기 실패", args: { vm: OVERVIEW_DEGRADED } };
export const Mobile: Story = { play: Default.play, name: "개요 · 390", args: { vm: OVERVIEW }, globals: { viewport: { value: "p390", isRotated: false } } };

export const NodeEvent: Story = {
  name: "노드 이벤트 · 대상 이름 비노출",
  args: { vm: { ...OVERVIEW, events: { ok:true, data:[
    { id:"node-event", time:"2026. 10. 31. 오후 10:00:00", action:"node:pair", target:"노드", actor:"operator@example.test" },
  ] } } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const event = canvas.getByText("node:pair").parentElement!;
    await expect(event).toHaveTextContent("노드");
    await expect(event).toHaveTextContent("operator@example.test");
    await expect(canvas.queryByText("edge-02")).toBeNull();
  },
};
