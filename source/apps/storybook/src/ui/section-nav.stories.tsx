import { PageHeader, SectionNav } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { PROXY_TABS, ADMIN_CATEGORIES } from "@/app/(app)/nav-links";
import { nodeNavCategories } from "@/app/(node)/nodes/[id]/node-nav";
const meta = {
  title: "UI/SectionNav",
  component: SectionNav,
  render: (args) => (
    <div style={{ maxWidth: 1200, marginInline: "auto" }}>
      <PageHeader title="섹션 내비게이션" />
      <SectionNav {...args} />
    </div>
  ),
  args: {
    items: PROXY_TABS,
    currentPath: "/reverse-proxy",
    label: "프록시 구성",
  },
} satisfies Meta<typeof SectionNav>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Tabs: Story = {};
export const TabsDark: Story = { globals: { theme: "dark" } };
export const GroupedTabs: Story = {
  args: {
    groups: ADMIN_CATEGORIES,
    items: undefined,
    currentPath: "/admin/security",
    label: "관리자 설정",
  },
};
export const NodeTabs: Story = {
  args: {
    groups: nodeNavCategories("example"),
    items: undefined,
    currentPath: "/nodes/example/audit",
    label: "노드 메뉴",
  },
};
export const Long: Story = {
  args: {
    items: [
      {
        href: "/long",
        label: "productiongatewaywithoutwordbreaks 긴 이름의 구성 메뉴",
      },
    ],
    currentPath: "/long",
  },
};
