// Product: the real apps/web backup code step (components/auth/backup-code-screen.tsx), as
// /login/2fa/backup renders it. The page (app/login/2fa/backup/page.tsx) asks the API whether a pending
// sign-in exists and picks the state from ?error=invalid; here the state is a story arg. Submitting goes
// to the Storybook stand-in of use-api-action, which sends nothing and says so. 「인증 앱 코드로 인증」
// opens Product/Login 2FA, as the link does in the app. No app shell.
import type { Meta, StoryObj } from "@storybook/react-vite";
import { BackupCodeScreen } from "@/components/auth/backup-code-screen";
import { authFrame, productViewports } from "./frame";

const meta = {
  id: "product-login-backup",
  title: "Product/Login Backup",
  component: BackupCodeScreen,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [authFrame()],
  args: { state: "default" },
} satisfies Meta<typeof BackupCodeScreen>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 인증 앱을 쓸 수 없을 때: 백업 코드(xxxxx-xxxxx) 하나를 받는다. */
export const Default: Story = { name: "백업 코드" };
/** 백업 코드가 틀렸을 때: 이 단계에 머물며 칸에 이어진 오류를 보인다. */
export const Invalid: Story = { name: "백업 코드 틀림", args: { state: "error" } };
export const Mobile: Story = { name: "백업 코드 · 390", globals: { viewport: { value: "p390", isRotated: false } } };
