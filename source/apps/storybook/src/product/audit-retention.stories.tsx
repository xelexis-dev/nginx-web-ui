import type { Meta, StoryObj } from "@storybook/react-vite";
import { useEffect, useState } from "react";
import { PageHeader } from "@repo/ui";
import { CertificateForm } from "@/components/certificates/certificate-form";
import { UserForm } from "@/app/admin/users/user-form";
import { SetupScreen } from "@/components/auth/setup-screen";
import { LoginScreen } from "@/components/auth/login-screen";
import { NodeForm } from "@/app/(app)/nodes/node-form";
import { TokenForm } from "@/app/(app)/account/tokens/token-form";
import { productFrame } from "./frame";
import layout from "@/components/page-layout.module.css";

async function reject() {
  return { ok: false, error: "입력값을 확인하세요. 저장하지 않았습니다." };
}

function RetentionReview({ kind }: {
  kind: "certificate-local" | "certificate-node" | "user" | "setup" | "login" | "pair" | "token";
}) {
  const [navigated, setNavigated] = useState(false);
  useEffect(() => {
    const navigate = () => setNavigated(true);
    window.addEventListener("nwui-preview-navigation", navigate);
    return () => window.removeEventListener("nwui-preview-navigation", navigate);
  }, []);
  if (navigated) return <p role="status">미리보기 이동 완료</p>;
  if (kind === "login") return <LoginScreen state="default" providers={[]} />;
  if (kind === "setup") return <SetupScreen />;
  return (
    <section className={layout.page}>
      <PageHeader title="오류 후 입력 확인" />
      {kind.startsWith("certificate-") ? (
        <CertificateForm
          action={reject}
          defaultEmail="ops@example.test"
          cancelHref={kind === "certificate-node" ? "/nodes/n1/certificates" : "/certificates"}
        />
      ) : null}
      {kind === "user" ? <UserForm /> : null}
      {kind === "pair" ? <NodeForm defaultGatewayName="게이트웨이" /> : null}
      {kind === "token" ? (
        <TokenForm scopes={["config:read", "config:write"]} action={reject} basePath="/account/tokens" kind="user" />
      ) : null}
    </section>
  );
}

const frame = productFrame(() => ({ path: "/account/tokens/new", label: "입력 확인", role: "admin" }));
const meta = {
  id: "product-retention-review",
  title: "Product/오류 후 입력 유지 검토",
  component: RetentionReview,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen" },
  decorators: [(Story, context) => context.args.kind === "setup" || context.args.kind === "login" ? <Story /> : frame(Story, context)],
  args: { kind: "certificate-local" },
} satisfies Meta<typeof RetentionReview>;
export default meta;
type Story = StoryObj<typeof meta>;
export const CertificateLocal: Story = { args: { kind: "certificate-local" } };
export const CertificateNode: Story = { args: { kind: "certificate-node" } };
export const User: Story = { args: { kind: "user" } };
export const Setup: Story = { args: { kind: "setup" } };
export const Login: Story = { args: { kind: "login" } };
export const Pair: Story = { args: { kind: "pair" } };
export const Token: Story = { args: { kind: "token" } };
