import { BrandLogo } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";

const meta = {
  title: "UI/BrandLogo",
  component: BrandLogo,
  args: { size: "sidebar", decorative: false },
  parameters: {
    docs: {
      description: {
        component:
          "NWUI 심볼은 SVG 경로입니다. 원본의 화살표 형태와 그라데이션을 유지하며 작은 크기에서 뭉치지 않도록 사선 틈과 글자 사이를 넓혔습니다. 제품명은 Pretendard로 렌더링하며 테마의 본문색을 따릅니다. 로그인·앱·관리자·노드에서 함께 사용합니다.",
      },
    },
  },
  argTypes: {
    size: { control: "inline-radio", options: ["sidebar", "signIn"] },
    decorative: { control: "boolean" },
  },
} satisfies Meta<typeof BrandLogo>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Sidebar: Story = {};
export const SignIn: Story = { args: { size: "signIn" } };

export const OnRail: Story = {
  render: (args) => (
    <div
      style={{
        background: "var(--bg-chrome)",
        color: "var(--text-1)",
        padding: 20,
      }}
    >
      <BrandLogo {...args} />
    </div>
  ),
};
export const OnRailDark: Story = { ...OnRail, globals: { theme: "dark" } };
