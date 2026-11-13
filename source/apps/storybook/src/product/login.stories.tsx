// Product: the real apps/web login screen (components/auth/login-screen.tsx), as /login renders it for
// each sign-in state. The page (app/login/page.tsx) reads the API and picks the state; here the state
// is a story arg. Submitting goes to the Storybook stand-in of use-api-action, which sends nothing and
// says so. The login has no app shell, so there is no ProductFrame.
import type { Meta, StoryObj } from "@storybook/react-vite";
import { LoginScreen } from "@/components/auth/login-screen";
import { productViewports } from "./frame";

const PROVIDERS = [
  { id: "company-sso", name: "사내 SSO" },
  { id: "example-oidc", name: "Example OIDC" },
];

const meta = {
  id: "product-login",
  title: "Product/Login",
  component: LoginScreen,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  args: { state: "default", providers: [] },
} satisfies Meta<typeof LoginScreen>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 공급자를 등록하지 않은 설치본: 구분선과 공급자 버튼이 없다. */
export const Default: Story = { name: "로그인" };
/** 이메일·비밀번호가 틀렸거나, 시도가 잦거나, OIDC 로그인이 실패했을 때(같은 문구). 2FA 코드는 다음 단계(Product/Login 2FA)에서 받는다. */
export const Failed: Story = { name: "로그인 실패", args: { state: "error" } };
/** 2단계 인증을 마치지 못하고 돌아왔을 때(5분 지남, 코드를 여러 번 틀림, 취소): 비밀번호 오류가 아니라 처음부터 다시 하라는 안내. */
export const Expired: Story = { name: "2단계 인증 만료", args: { state: "expired" } };
export const Pending: Story = { name: "승인 대기", args: { state: "pending" } };
export const Disabled: Story = { name: "비활성 계정", args: { state: "disabled" } };
export const WithProviders: Story = { name: "OIDC 공급자 있음", args: { providers: PROVIDERS } };
export const Mobile: Story = { name: "로그인 · 390", args: { providers: PROVIDERS }, globals: { viewport: { value: "p390", isRotated: false } } };

export const Dark: Story = { name: "로그인 · 다크", globals: { theme: "dark" } };

/** 짧은 노트북 화면: 기본 로그인에 불필요한 세로 스크롤이 없어야 한다. */
export const Laptop: Story = {
  name: "로그인 · 1366×768",
  parameters: { viewport: { options: { ...productViewports, laptop: { name: "Laptop 1366×768", styles: { width: "1366px", height: "768px" } } } } },
  globals: { viewport: { value: "laptop", isRotated: false } },
};
