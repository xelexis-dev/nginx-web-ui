import type { Meta, StoryObj } from "@storybook/react-vite";
import { LoginScreen } from "@/components/auth/login-screen";
import { TwoFactorScreen } from "@/components/auth/two-factor-screen";
import { BackupCodeScreen } from "@/components/auth/backup-code-screen";
import { SetupScreen } from "@/components/auth/setup-screen";
import { ErrorView } from "@/components/error-view";
import RootNotFound from "@/app/not-found";
import AppNotFound from "@/app/(app)/not-found";
import AdminNotFound from "@/app/admin/not-found";
import { RegenerateBackupCodesForm } from "@/app/(app)/account/2fa/regenerate-backup-codes-form";
import { productFrame, productViewports } from "./frame";

function AuthStatus({ screen }: { screen: "login" | "two-factor" | "two-factor-error" | "backup" | "backup-error" | "setup" | "error" | "stale" | "not-found" | "app-not-found" | "admin-not-found" | "regenerate" }) {
  switch (screen) {
    case "regenerate": return <RegenerateBackupCodesForm hasPassword={false} />;
    case "login": return <LoginScreen state="default" providers={[]} passkeyAvailable />;
    case "two-factor": return <TwoFactorScreen state="default" />;
    case "two-factor-error": return <TwoFactorScreen state="error" />;
    case "backup": return <BackupCodeScreen state="default" />;
    case "backup-error": return <BackupCodeScreen state="error" />;
    case "setup": return <SetupScreen />;
    case "error": return <ErrorView standalone digest="preview-error-id" onRetry={() => {}} homeHref="/" />;
    case "stale": return <ErrorView standalone staleDeployment homeHref="/" />;
    case "not-found": return <RootNotFound />;
    case "app-not-found": return <AppNotFound />;
    case "admin-not-found": return <AdminNotFound />;
  }
}
const meta = {
  id: "product-auth-status",
  title: "Product/Auth and status",
  component: AuthStatus,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  args: { screen: "login" },
} satisfies Meta<typeof AuthStatus>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Login: Story = {};
export const TwoFactor: Story = { args: { screen: "two-factor" } };
export const TwoFactorError: Story = { args: { screen: "two-factor-error" } };
export const Backup: Story = { args: { screen: "backup" } };
export const BackupError: Story = { args: { screen: "backup-error" } };
export const Setup: Story = { args: { screen: "setup" } };
export const Error: Story = { args: { screen: "error" } };
export const Stale: Story = { args: { screen: "stale" } };
export const NotFound: Story = { args: { screen: "not-found" } };
export const AppMissing: Story = { args: { screen: "app-not-found" }, decorators: [productFrame(() => ({ path: "/certificates/missing", label: "찾는 항목이 없습니다", role: "editor" }))] };
export const AdminMissing: Story = { args: { screen: "admin-not-found" }, decorators: [productFrame(() => ({ path: "/admin/users/missing", label: "찾는 항목이 없습니다", role: "admin" }))] };

export const Regenerate: Story = { args: { screen: "regenerate" } };
