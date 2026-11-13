// Product: the real apps/web second sign-in step (components/auth/two-factor-screen.tsx), as
// /login/2fa renders it. The page (app/login/2fa/page.tsx) asks the API whether a pending sign-in
// exists and picks the state from ?error=invalid; here the state is a story arg. Submitting goes to the
// Storybook stand-in of use-api-action, which sends nothing and says so. No app shell.
import type { Meta, StoryObj } from "@storybook/react-vite";
import { TwoFactorScreen } from "@/components/auth/two-factor-screen";
import { authFrame, productViewports } from "./frame";

const meta = {
  id: "product-login-2fa",
  title: "Product/Login 2FA",
  component: TwoFactorScreen,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  // 「백업 코드로 인증」 opens Product/Login Backup, as the link does in the app.
  decorators: [authFrame()],
  args: { state: "default" },
} satisfies Meta<typeof TwoFactorScreen>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 비밀번호를 확인한 뒤 2FA를 켠 계정: 인증 앱의 숫자 6자리만 받는다(다른 글자는 입력되지 않는다). 백업 코드는 링크로 다음 화면에서. */
export const Default: Story = { name: "2단계 인증" };
/** 코드가 틀렸을 때: 이 단계에 머물며 칸에 이어진 오류를 보인다(비밀번호 오류로 말하지 않는다). */
export const Invalid: Story = { name: "코드 틀림", args: { state: "error" } };
export const Mobile: Story = { name: "2단계 인증 · 390", globals: { viewport: { value: "p390", isRotated: false } } };
