import type { Meta, StoryObj } from "@storybook/react-vite";
import { SetupScreen } from "@/components/auth/setup-screen";
import { productViewports } from "./frame";

const meta = {
  id: "product-setup",
  title: "Product/Setup",
  component: SetupScreen,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
} satisfies Meta<typeof SetupScreen>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = { name: "최초 관리자 설정" };
export const Dark: Story = { name: "최초 관리자 설정 · 다크", globals: { theme: "dark" } };
export const Mobile: Story = { name: "최초 관리자 설정 · 390", globals: { viewport: { value: "p390", isRotated: false } } };
