import type { Meta, StoryObj } from "@storybook/react-vite";
import { Banner, Button, Field, TextInput } from "@repo/ui";
import { AuthFrame } from "@/components/auth/auth-frame";
import auth from "@/components/auth/auth-screen.module.css";

function AuthTemplate({
  setup = false,
  failed = false,
}: {
  setup?: boolean;
  failed?: boolean;
}) {
  return (
    <AuthFrame
      title={setup ? "관리자 계정 만들기" : "로그인"}
      subtitle={
        setup
          ? "이 설치본을 관리할 첫 계정을 만드세요."
          : "등록된 계정으로 nginx 관리 콘솔에 로그인하세요."
      }
      firstRun={setup}
    >
      <form className={auth.form} onSubmit={(event) => event.preventDefault()}>
        {failed ? (
          <Banner tone="error" title="로그인하지 못했습니다." role="alert">
            이메일과 비밀번호를 확인하세요.
          </Banner>
        ) : null}
        <Field label="이메일" htmlFor="template-email" required>
          <TextInput
            id="template-email"
            type="email"
            autoComplete="username"
            placeholder="admin@example.test"
            required
          />
        </Field>
        <Field label="비밀번호" htmlFor="template-password" required>
          <TextInput
            id="template-password"
            type="password"
            autoComplete={setup ? "new-password" : "current-password"}
            required
          />
        </Field>
        {setup ? (
          <Field label="비밀번호 확인" htmlFor="template-confirm" required>
            <TextInput
              id="template-confirm"
              type="password"
              autoComplete="new-password"
              required
            />
          </Field>
        ) : null}
        <Button type="submit" className={auth.submit}>
          {setup ? "관리자 계정 만들기" : "로그인"}
        </Button>
      </form>
    </AuthFrame>
  );
}
const meta = {
  id: "templates-auth",
  title: "Templates/인증",
  component: AuthTemplate,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof AuthTemplate>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const Setup: Story = { name: "최초 설정", args: { setup: true } };
export const Failed: Story = { name: "로그인 실패", args: { failed: true } };
export const Dark: Story = { globals: { theme: "dark" } };
