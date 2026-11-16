import type { Meta, StoryObj } from "@storybook/react-vite";
import { ErrorView } from "@/components/error-view";
const meta = {
  id: "templates-exceptional",
  title: "Templates/예외 상태",
  component: ErrorView,
  args: { digest: "example-request-id", onRetry: () => {} },
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof ErrorView>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Spec: Story = {};
export const States: Story = { args: { staleDeployment: true } };
export const Dark: Story = { globals: { theme: "dark" } };
