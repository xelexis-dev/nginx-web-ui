import type { Meta, StoryObj } from "@storybook/react-vite";
import { TwoFactorManagement } from "@/app/(app)/account/2fa/two-factor-management";
import { productFrame, productViewports } from "./frame";

const meta = {
  id: "product-account-2fa-management",
  title: "Product/Account/2FA Management",
  component: TwoFactorManagement,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [productFrame(() => ({ path: "/account/2fa", label: "2FA 관리", role: "editor" }))],
  args: { hasPassword: true },
} satisfies Meta<typeof TwoFactorManagement>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = { name: "2FA 관리" };
export const OAuthOnly: Story = { name: "OAuth 전용 계정", args: { hasPassword: false } };
export const Mobile: Story = { name: "2FA 관리 · 390", globals: { viewport: { value: "p390", isRotated: false } } };
