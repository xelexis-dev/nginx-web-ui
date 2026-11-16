// The production forms, rendered through the existing no-network API action adapter.
import type { Meta, StoryObj } from "@storybook/react-vite";
import { PageHeader } from "@repo/ui";
import { UserForm } from "@/app/admin/users/user-form";
import { SecurityForm } from "@/app/admin/security/security-form";
import { SmtpForm } from "@/app/admin/settings/smtp/smtp-form";
import { BaseUrlForm } from "@/app/admin/settings/general/base-url-form";
import { ProviderForm } from "@/app/admin/oauth/provider-form";
import { MailRecipientForm } from "@/app/admin/notifications/mail/recipient-form";
import { WebhookForm } from "@/app/admin/notifications/webhooks/webhook-form";
import { NodeNewView } from "@/app/(app)/nodes/node-new-view";
import { APP_SEMVER } from "@/lib/version";
import { MetricsForm } from "@/app/admin/settings/general/metrics-form";
import { NodeEditView } from "@/app/(node)/nodes/[id]/edit/node-edit-view";
import { ChangePasswordForm } from "@/app/(app)/account/change-password-form";
import layout from "@/components/page-layout.module.css";
import { productFrame, productViewports } from "./frame";

const pages = {
  user: { path: "/admin/users/new", title: "새 사용자" },
  security: { path: "/admin/security", title: "보안" },
  smtp: { path: "/admin/settings/smtp", title: "SMTP 설정" },
  general: { path: "/admin/settings/general", title: "일반" },
  oauth: { path: "/admin/oauth/new", title: "로그인 공급자 추가" },
  oauthEdit: { path: "/admin/oauth/example/edit", title: "로그인 공급자 수정" },
  mail: { path: "/admin/notifications/mail/new", title: "메일 수신자 추가" },
  webhook: { path: "/admin/notifications/webhooks/new", title: "웹후크 추가" },
  node: { path: "/nodes/new", title: "노드 연결" },
  nodeEdit: { path: "/nodes/example/edit", title: "노드 수정" },
  password: { path: "/account", title: "비밀번호 변경" },
} as const;

function SettingsForm({ kind }: { kind: keyof typeof pages }) {
  if (kind === "node") return <NodeNewView defaultGatewayName="nginx.example.test" />;
  if (kind === "nodeEdit") return <NodeEditView node={{ id:"example", name:"edge.example.test", baseUrl:"https://edge.example.test", allowInsecureTls:false, tokenRole:"editor", instanceId:"preview", gatewayName:"nginx.example.test", createdByUserId:"operator", createdAt:"2026-10-01T05:00:00Z", lastSeenAt:"2026-10-02T05:00:00Z", lastStatus:"ok", lastVersion:APP_SEMVER, lastError:null }} />;
  return (
    <section className={layout.page}>
      <PageHeader title={pages[kind].title} />
      {kind === "user" ? <UserForm /> : null}
      {kind === "security" ? (
        <SecurityForm requireAdmin requireUser={false} requireOauth={false} />
      ) : null}
      {kind === "smtp" ? (
        <SmtpForm
          hasStoredPassword
          defaults={{
            enabled: true,
            host: "smtp.example.test",
            port: 587,
            secure: false,
            username: "notifications@example.test",
            fromEmail: "notifications@example.test",
            fromName: "nginx-web-ui",
            rejectUnauthorized: true,
          }}
        />
      ) : null}
      {kind === "general" ? (
        <><BaseUrlForm defaultValue="https://nginx.example.test" /><MetricsForm defaultRetentionDays="30" defaultExcludedHosts="" /></>
      ) : null}
      {kind === "oauth" ? <ProviderForm mode="create" /> : null}
      {kind === "oauthEdit" ? (
        <ProviderForm
          mode="edit"
          callbackUrl="https://nginx.example.test/api/auth/callback/example"
          defaults={{
            id: "example",
            name: "회사 로그인",
            issuer: "https://identity.example.test",
            clientId: "nwui-preview",
            scope: "openid email profile",
            enabled: true,
            sortOrder: 0,
          }}
        />
      ) : null}
      {kind === "mail" ? (
        <MailRecipientForm
          mode="create"
          smtpReady
          defaults={{
            label: "운영팀",
            address: "operators@example.test",
            enabled: true,
            events: ["certificate.failed"],
          }}
        />
      ) : null}
      {kind === "webhook" ? (
        <WebhookForm
          mode="create"
          defaults={{
            name: "운영 알림",
            enabled: true,
            format: "generic",
            events: [],
            bodyTemplate: "",
          }}
        />
      ) : null}
      {kind === "password" ? <ChangePasswordForm /> : null}
    </section>
  );
}

const meta = {
  id: "product-settings-forms",
  title: "Product/관리·계정·노드 폼",
  component: SettingsForm,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [
    productFrame((args) => {
      const page = pages[args.kind as keyof typeof pages] ?? pages.user;
      return {
        path: page.path,
        label: page.title,
        role: page.path.startsWith("/admin") ? "admin" : "editor",
      };
    }),
  ],
  args: { kind: "user" },
} satisfies Meta<typeof SettingsForm>;
export default meta;
type Story = StoryObj<typeof meta>;
export const User: Story = { name: "관리자 · 사용자 생성" };
export const Security: Story = {
  name: "관리자 · 보안 정책",
  args: { kind: "security" },
};
export const Smtp: Story = { name: "관리자 · SMTP", args: { kind: "smtp" } };
export const General: Story = {
  name: "관리자 · 기본 주소",
  args: { kind: "general" },
};
export const Oauth: Story = {
  name: "관리자 · 로그인 공급자",
  args: { kind: "oauth" },
};
export const OauthEdit: Story = {
  name: "관리자 · 로그인 공급자 수정",
  args: { kind: "oauthEdit" },
};
export const Mail: Story = {
  name: "관리자 · 메일 수신자",
  args: { kind: "mail" },
};
export const Webhook: Story = {
  name: "관리자 · 웹후크",
  args: { kind: "webhook" },
};
export const Node: Story = { name: "노드 · 연결", args: { kind: "node" } };
export const NodeEdit: Story = {
  name: "노드 · 수정",
  args: { kind: "nodeEdit" },
};
export const Password: Story = {
  name: "계정 · 비밀번호 변경",
  args: { kind: "password" },
};
export const NarrowNode: Story = {
  name: "노드 연결 · 390",
  args: { kind: "node" },
  globals: { viewport: { value: "p390", isRotated: false } },
};
export const NarrowWebhook: Story = {
  name: "웹후크 · 390",
  args: { kind: "webhook" },
  globals: { viewport: { value: "p390", isRotated: false } },
};
