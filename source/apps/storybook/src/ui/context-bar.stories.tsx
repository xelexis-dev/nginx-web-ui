import { PageHeader, ContextBar, StatusBadge } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
const meta = {
  title: "UI/ContextBar",
  component: ContextBar,
  render: (args) => (
    <div style={{ maxWidth: 1200, marginInline: "auto" }}>
      <PageHeader title="노드 맥락" />
      <ContextBar {...args} />
    </div>
  ),
  args: {
    name: "서울 운영 노드",
    facts: "nginx 1.28.0 · Linux",
    status: <StatusBadge tone="success">연결됨</StatusBadge>,
  },
} satisfies Meta<typeof ContextBar>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Online: Story = {};
export const OnlineDark: Story = { globals: { theme: "dark" } };
export const Offline: Story = {
  args: {
    offline: true,
    status: <StatusBadge tone="error">연결할 수 없음</StatusBadge>,
  },
};
export const OfflineDark: Story = { ...Offline, globals: { theme: "dark" } };
export const Long: Story = {
  args: {
    name: "productiongatewaywithoutwordbreaks-서울-데이터센터-장애대응-원격노드",
  },
};
export const LongDark: Story = { ...Long, globals: { theme: "dark" } };
