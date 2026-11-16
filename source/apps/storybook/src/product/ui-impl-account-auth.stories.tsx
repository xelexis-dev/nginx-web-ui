import type { Meta, StoryObj } from "@storybook/react-vite";
import { PageHeader } from "@repo/ui";
import { AccountOverview } from "@/app/(app)/account/account-overview";
import { AccountTokensView } from "@/app/(app)/account/tokens/token-list";
import { TokenForm } from "@/app/(app)/account/tokens/token-form";
import { TokenDetail } from "@/app/(app)/account/tokens/token-detail";
import {
  TwoFactorEnroll,
  TwoFactorVerify,
} from "@/app/(app)/account/2fa/two-factor-forms";
import RootNotFound from "@/app/not-found";
import AppNotFound from "@/app/(app)/not-found";
import AdminNotFound from "@/app/admin/not-found";
import GlobalError from "@/app/global-error";
import { ApiUnavailableScreen } from "@/components/api-unavailable-screen";
import { RoleBadge } from "@/components/role-badge";
import { productFrame, productViewports } from "./frame";
import accountStyles from "@/app/(app)/account/account-pages.module.css";
import layout from "@/components/page-layout.module.css";
import type { TokenRowView } from "@/app/(app)/account/tokens/tokens";

// Fixture values exist only in Storybook; the product still reads its authenticated API.
const token: TokenRowView = {
  id: "preview-token",
  name: "운영팀 자동화",
  masked: "nwui_…demo",
  scopes: ["config:read"],
  status: "active",
  createdAt: "2026. 10. 1.",
  lastUsedAt: "2026. 10. 2.",
  expiresAt: "2026. 12. 30.",
  revokedAt: "—",
  revoked: false,
};
type Kind =
  | "account"
  | "tokens"
  | "token-new"
  | "token-detail"
  | "enroll"
  | "verify"
  | "root404"
  | "app404"
  | "admin404"
  | "global"
  | "api"
  | "roles";
function AccountPreview({ kind }: { kind: Kind }) {
  if (kind === "account")
    return (
      <AccountOverview
        role="admin"
        passkeys={{
          passkeys: [],
          availability: { available: true, reason: null },
          currentOrigin: "https://account.example.test",
          currentAddress: "account.example.test",
          hasPassword: true,
          canRemove: true,
        }}
        account={{
          email: "operator@example.test",
          hasPassword: true,
          totpEnabled: true,
          logins: [
            {
              providerId: "company",
              name: "회사 로그인",
              linkedAt: "2026-10-01T05:00:00Z",
            },
          ],
        }}
      />
    );
  if (kind === "token-detail")
    return (
      <TokenDetail
        view={token}
        basePath="/account/tokens"
        revoke={{ id: "account.tokens.revoke", args: [] }}
      />
    );
  if (kind === "tokens") return <AccountTokensView rows={[token]} />;
  if (kind === "root404") return <RootNotFound />;
  if (kind === "app404") return <AppNotFound />;
  if (kind === "admin404") return <AdminNotFound />;
  if (kind === "global") {
    // Render the actual fallback body without nesting a second html/body in Storybook's document.
    const document = GlobalError({
      error: Object.assign(new Error("preview"), { digest: "preview-id" }),
      retry: () => {},
    });
    return <>{document.props.children.props.children}</>;
  }
  if (kind === "api") return <ApiUnavailableScreen />;
  if (kind === "roles")
    return (
      <section className={layout.page}>
        <PageHeader title="계정 등급" />
        <div className={accountStyles.accountMeta}>
          {(["admin", "editor", "viewer"] as const).map((role) => (
            <RoleBadge key={role} role={role} />
          ))}
        </div>
      </section>
    );
  const title =
    kind === "token-new"
      ? "새 액세스 토큰"
      : kind === "enroll"
        ? "2단계 인증 (2FA) 설정"
        : "2단계 인증 확인";
  return (
    <section className={layout.page}>
      <PageHeader title={title} />
      {kind === "token-new" ? (
        <TokenForm
          kind="user"
          scopes={["config:read", "config:write", "nginx:reload"]}
          action={{ id: "account.tokens.issue", args: [] }}
          basePath="/account/tokens"
        />
      ) : kind === "enroll" ? (
        <TwoFactorEnroll
          secret="PREVIEW-KEY-NOT-A-REAL-SECRET"
          qrDataUrl="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='220' height='220'%3E%3Crect width='220' height='220' fill='white'/%3E%3Ctext x='20' y='110'%3EQR preview%3C/text%3E%3C/svg%3E"
        />
      ) : (
        <TwoFactorVerify />
      )}
    </section>
  );
}
const meta = {
  id: "product-ui-impl-account-auth",
  title: "Product/계정과 인증 적용",
  component: AccountPreview,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  args: { kind: "account" },
  decorators: [
    (Story, context) =>
      ["root404", "global", "api"].includes(context.args.kind) ? (
        <Story />
      ) : (
        productFrame(() => ({
          path: context.args.kind.startsWith("token")
            ? "/account/tokens"
            : context.args.kind === "admin404"
              ? "/admin/users"
              : "/account",
          label: "계정",
          role: "admin",
        }))(Story, context)
      ),
  ],
} satisfies Meta<typeof AccountPreview>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Account: Story = {};
export const Tokens: Story = { args: { kind: "tokens" } };
export const NewToken: Story = { args: { kind: "token-new" } };
export const Token: Story = { args: { kind: "token-detail" } };
export const Enroll: Story = { args: { kind: "enroll" } };
export const VerifySession: Story = { args: { kind: "verify" } };
export const Root404: Story = { args: { kind: "root404" } };
export const App404: Story = { args: { kind: "app404" } };
export const Admin404: Story = { args: { kind: "admin404" } };
export const GlobalFailure: Story = { args: { kind: "global" } };
export const ApiUnavailable: Story = { args: { kind: "api" } };
export const Roles: Story = { args: { kind: "roles" } };
