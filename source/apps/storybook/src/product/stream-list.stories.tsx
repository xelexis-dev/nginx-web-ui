import type { Meta, StoryObj } from "@storybook/react-vite";
import { StreamList, type StreamListVM } from "@/components/streams/stream-list";
import { productFrame, productViewports } from "./frame";

const data: StreamListVM = {
  canWrite: true,
  items: [
    { id: "st-db", listen: 5432, protocol: "tcp", proxyPass: "database:5432", enabled: true },
    { id: "st-dns", listen: 5353, protocol: "udp", proxyPass: "resolver:53", enabled: false },
  ],
};

// The real local/node list has no apply control. Its shell owns the single apply entry point.
const meta = {
  id: "product-stream-list",
  title: "Product/Streams/List",
  component: StreamList,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [productFrame(args => ({ path: "/stream", label: "Stream", role: (args.data as StreamListVM).canWrite ? "editor" : "viewer" }))],
  args: { data, basePath: "/stream" },
} satisfies Meta<typeof StreamList>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = { name: "Stream 목록" };
export const Empty: Story = { name: "빈 목록", args: { data: { ...data, items: [] } } };
export const Viewer: Story = { name: "보기 전용", args: { data: { ...data, canWrite: false } } };
export const Mobile: Story = { name: "Stream 목록 · 390", globals: { viewport: { value: "p390", isRotated: false } } };
