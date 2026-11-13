import { Banner, FormStack } from "@repo/ui";
// Real admin views/forms; API loading stays in the production route adapters.
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Icon, Button, PageHeader, Select, buttonClassName } from "@repo/ui";
import Link from "next/link";
import { DEFAULT_EVENT_TYPES } from "@repo/presentation/webhook";
import { ApiForm } from "@/components/api-form";
import layout from "@/components/page-layout.module.css";
import { UserList } from "@/app/admin/users/user-list";
import { UserDetailView } from "@/app/admin/users/user-detail";
import { UserDetailActions } from "@/app/admin/users/[id]/detail-actions";
import { UserForm } from "@/app/admin/users/user-form";
import { EditUserForms } from "@/app/admin/users/[id]/edit/edit-user-forms";
import { PendingListView } from "@/app/admin/pending/pending-list";
import { SecurityForm } from "@/app/admin/security/security-form";
import { ConnectedGateways } from "@/app/admin/security/connected-gateways";
import { ProviderListView } from "@/app/admin/oauth/provider-list";
import {
  ProviderDetailView,
  ProviderDetailActions,
} from "@/app/admin/oauth/provider-detail";
import { ProviderForm } from "@/app/admin/oauth/provider-form";
import { ServerTokensView } from "@/app/(app)/account/tokens/token-list";
import { TokenDetail } from "@/app/(app)/account/tokens/token-detail";
import { TokenForm } from "@/app/(app)/account/tokens/token-form";
import {
  SecretsHealthView,
  type SecretRow,
} from "@/app/admin/secrets/secrets-health";
import { RecipientListView } from "@/app/admin/notifications/mail/recipient-list";
import { RecipientDetailView } from "@/app/admin/notifications/mail/recipient-detail";
import { MailRecipientDetailActions } from "@/app/admin/notifications/mail/[id]/detail-actions";
import { MailRecipientForm } from "@/app/admin/notifications/mail/recipient-form";
import { WebhookListView } from "@/app/admin/notifications/webhooks/webhook-list";
import { WebhookDetailView } from "@/app/admin/notifications/webhooks/webhook-detail";
import { WebhookDetailActions } from "@/app/admin/notifications/webhooks/[id]/detail-actions";
import { WebhookForm } from "@/app/admin/notifications/webhooks/webhook-form";
import { BaseUrlForm } from "@/app/admin/settings/general/base-url-form";
import { MetricsForm } from "@/app/admin/settings/general/metrics-form";
import { SmtpForm } from "@/app/admin/settings/smtp/smtp-form";
import { productFrame, productViewports } from "./frame";

const user = {
  id: "operator",
  email: "operator@example.test",
  role: "admin" as const,
  enabled: true,
  approved: true,
  hasPassword: true, totpEnabled: false,
  createdAt: "2026-10-01T05:00:00Z",
};
const provider = {
  id: "company",
  name: "회사 로그인",
  issuer: "https://identity.example.test",
  clientId: "nginx-web-ui",
  scope: "openid email profile",
  enabled: true,
  sortOrder: 0,
  linkedUsers: 3,
  callbackUrl: "https://nginx.example.test/api/auth/callback/company",
};
const recipient = {
  id: "operations",
  label: "운영팀",
  address: "operators@example.test",
  enabled: true,
  events: ["certificate.failed"],
  lastAttemptAt: "2026-10-01T05:00:00Z",
  lastSuccessAt: "2026-10-01T05:00:00Z",
  lastError: null,
};
const webhook = {
  id: "operations",
  name: "운영 알림",
  enabled: true,
  format: "generic" as const,
  events: ["certificate.failed"],
  lastAttemptAt: null,
  lastSuccessAt: null,
  lastStatus: null,
  lastError: null,
  bodyTemplate: null,
  maskedUrl: "https://hooks.example.test/******",
  headerNames: ["Authorization"],
  openError: null,
};
const token = {
  id: "gateway",
  name: "운영 게이트웨이",
  masked: "nwui_••••••••a12b",
  scopes: ["node:pair"],
  status: "active" as const,
  createdAt: "2026. 10. 1. 오후 2:00",
  lastUsedAt: "사용한 적 없음",
  expiresAt: "무기한",
  revokedAt: "해지되지 않음",
  revoked: false,
};
const secrets: SecretRow[] = [
  {
    group: "OAuth",
    label: provider.name,
    state: "sealed-ok",
    impact: "공급자 로그인",
    recovery: "비밀키를 다시 입력하세요.",
    href: "/admin/oauth/company",
  },
  {
    group: "SMTP",
    label: "SMTP 비밀번호",
    state: "plaintext",
    impact: "메일 발송",
    recovery: "비밀번호를 다시 입력하세요.",
    href: "/admin/settings/smtp",
  },
];
const pages = {
  users: ["/admin/users", "사용자"],
  user: ["/admin/users/operator", "사용자 상세"],
  userEdit: ["/admin/users/operator/edit", "사용자 수정"],
  userNew: ["/admin/users/new", "새 사용자"],
  pending: ["/admin/pending", "승인 대기"],
  security: ["/admin/security", "보안 — 2FA 강제"],
  oauth: ["/admin/oauth", "OAuth 로그인"],
  provider: ["/admin/oauth/company", "회사 로그인"],
  providerEdit: ["/admin/oauth/company/edit", "공급자 수정"],
  providerNew: ["/admin/oauth/new", "공급자 추가"],
  tokens: ["/admin/tokens", "서버 액세스 토큰"],
  token: ["/admin/tokens/gateway", "서버 토큰 상세"],
  tokenNew: ["/admin/tokens/new", "새 서버 토큰"],
  secrets: ["/admin/secrets", "저장된 비밀값"],
  mail: ["/admin/notifications/mail", "메일"],
  recipient: ["/admin/notifications/mail/operations", "운영팀"],
  recipientEdit: ["/admin/notifications/mail/operations/edit", "수신처 수정"],
  recipientNew: ["/admin/notifications/mail/new", "새 수신처"],
  webhooks: ["/admin/notifications/webhooks", "Webhook"],
  webhook: ["/admin/notifications/webhooks/operations", "운영 알림"],
  webhookEdit: [
    "/admin/notifications/webhooks/operations/edit",
    "webhook 수정",
  ],
  webhookNew: ["/admin/notifications/webhooks/new", "새 webhook"],
  general: ["/admin/settings/general", "일반"],
  smtp: ["/admin/settings/smtp", "SMTP 설정"],
} as const;
// The two remaining admin routes are redirects, not rendered UI.
const ADMIN_REDIRECTS = {
  "/admin": "/admin/users",
  "/admin/smtp": "/admin/settings/smtp",
} as const;
function AdminPage({ kind }: { kind: keyof typeof pages }) {
  switch (kind) {
    case "users":
      return (
        <UserList
          selfUserId={user.id}
          items={[
            {
              ...user,
              displayName: "운영 관리자",
              methods: "비밀번호 · 회사 로그인",
            },
            {
              ...user,
              id: "editor",
              email: "editor@example.test",
              displayName: null,
              role: "editor",
              methods: "비밀번호",
            },
          ]}
        />
      );
    case "user":
      return (
        <UserDetailView
          user={user}
          isSelf
          detailActions={<UserDetailActions id={user.id} isSelf enabled />}
        />
      );
    case "pending":
      return (
        <PendingListView
          pending={[
            {
              id: "pending",
              email: "new@example.test",
              displayName: "신규 사용자",
              provider: provider.name,
              createdAt: user.createdAt,
            },
          ]}
          approvalForm={(u) => (
            <ApiForm
              action={{ id: "admin.pending.approve", args: [] }}
              id={`approve-${u.id}`}
            >
              <input type="hidden" name="id" value={u.id} />
              <Select
                size="sm"
                name="role"
                defaultValue="viewer"
                options={[
                  { value: "viewer", label: "viewer" },
                  { value: "editor", label: "editor" },
                  { value: "admin", label: "admin" },
                ]}
                aria-label="승인할 등급"
              />
            </ApiForm>
          )}
          rejectionForm={(u) => (
            <ApiForm action={{ id: "admin.pending.reject", args: [] }}>
              <input type="hidden" name="id" value={u.id} />
              <Button size="sm" type="submit" variant="secondary">
                거절
              </Button>
            </ApiForm>
          )}
        />
      );
    case "oauth":
      return (
        <ProviderListView
          providers={[provider]}
          toggleForm={(p) => (
            <ApiForm action={{ id: "admin.oauth.toggle", args: [] }}>
              <input type="hidden" name="id" value={p.id} />
              <Button size="sm" type="submit" variant="secondary">
                사용 안 함
              </Button>
            </ApiForm>
          )}
        />
      );
    case "provider":
      return (
        <ProviderDetailView
          provider={provider}
          deleteForm={<ProviderDetailActions id={provider.id} />}
        />
      );
    case "secrets":
      return (
        <SecretsHealthView
          rows={secrets}
          broken={[]}
          plaintext={secrets.filter((row) => row.state === "plaintext")}
        />
      );
    case "mail":
      return (
        <RecipientListView
          recipients={[recipient]}
          mailer={{ ok: true, message: "준비됨" }}
        />
      );
    case "recipient":
      return (
        <RecipientDetailView
          recipient={recipient}
          events={recipient.events}
          detailActions={
            <MailRecipientDetailActions
              id={recipient.id}
              address={recipient.address}
            />
          }
        />
      );
    case "webhooks":
      return <WebhookListView targets={[webhook]} />;
    case "webhook":
      return (
        <WebhookDetailView
          target={webhook}
          maskedUrl={webhook.maskedUrl}
          headerNames={webhook.headerNames}
          events={webhook.events}
          detailActions={
            <WebhookDetailActions
              id={webhook.id}
              format={webhook.format}
              bodyTemplate={webhook.bodyTemplate}
            />
          }
        />
      );
  }
  const explanation =
    kind === "userNew"
      ? "이메일, 비밀번호, 역할을 지정해 새 계정을 생성합니다."
      : kind === "security"
        ? "역할별로 2단계 인증(2FA)을 강제할지 설정합니다."
        : kind === "providerNew"
          ? "공급자 쪽에서 애플리케이션을 먼저 만들고, 거기서 받은 클라이언트 ID 와 비밀키를 넣으세요. 저장하면 등록해야 할 콜백 주소를 알려 드립니다."
          : kind === "recipientNew"
            ? "인증서 알림을 받을 주소를 등록합니다."
            : kind === "webhookNew"
              ? "알림을 받을 HTTP 수신처를 추가합니다."
              : kind === "general"
                ? "사이트 전체에 적용되는 설정입니다."
                : kind === "smtp"
                  ? "앱이 메일을 보낼 때 쓰는 서버입니다."
                  : undefined;
  const isForm = kind !== "tokens" && kind !== "token";
  const content = (
    <>
      {kind === "userNew" && <UserForm />}
      {kind === "userEdit" && (
        <EditUserForms
          id={user.id}
          email={user.email}
          currentRole={user.role}
          hasPassword
        />
      )}
      {kind === "security" && (
        <>
          <SecurityForm requireAdmin requireUser={false} requireOauth={false} />
          <ConnectedGateways
            rows={[
              {
                id: "gateway",
                name: "운영 게이트웨이",
                role: "editor",
                createdAt: user.createdAt,
                lastUsedAt: user.createdAt,
                lastUsedIp: "192.0.2.10",
                revokedAt: null,
              },
            ]}
          />
        </>
      )}
      {kind === "providerNew" && <ProviderForm mode="create" />}
      {kind === "providerEdit" && (
        <ProviderForm
          mode="edit"
          callbackUrl={provider.callbackUrl}
          defaults={provider}
        />
      )}
      {kind === "tokens" && <ServerTokensView rows={[token]} />}
      {kind === "token" && (
        <TokenDetail
          view={token}
          basePath="/admin/tokens"
          revoke={{ id: "admin.tokens.revoke", args: [] }}
        />
      )}
      {kind === "tokenNew" && (
        <TokenForm
          kind="server"
          scopes={["node:pair"]}
          action={{ id: "admin.tokens.issue", args: [] }}
          basePath="/admin/tokens"
        />
      )}
      {kind === "recipientNew" && (
        <MailRecipientForm
          mode="create"
          smtpReady
          defaults={{
            label: "",
            address: "",
            enabled: true,
            events: [...DEFAULT_EVENT_TYPES],
          }}
        />
      )}
      {kind === "recipientEdit" && (
        <MailRecipientForm mode="edit" smtpReady defaults={recipient} />
      )}
      {kind === "webhookNew" && (
        <WebhookForm
          mode="create"
          defaults={{
            name: "",
            enabled: true,
            format: "generic",
            events: [...DEFAULT_EVENT_TYPES],
            bodyTemplate: "",
          }}
        />
      )}
      {kind === "webhookEdit" && (
        <WebhookForm mode="edit" defaults={{ ...webhook, bodyTemplate: "" }} />
      )}
      {kind === "general" && (
        <>
          <BaseUrlForm defaultValue="https://nginx.example.test" />
          <MetricsForm
            defaultRetentionDays="30"
            defaultExcludedHosts="localhost"
          />
        </>
      )}
      {kind === "smtp" && (
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
      )}
    </>
  );
  if (kind === "tokens" || kind === "token") return content;
  return (
    <section className={layout.page}>
      <PageHeader
        title={pages[kind][1]}
        description={
          kind === "security" ? (
            <>
              기본값은 모두 <strong>해제</strong>이며, 강제로 바꾸면 해당 역할
              사용자는 다음 로그인 시 2FA 등록(계정 → 2FA)을 마쳐야 합니다.
            </>
          ) : kind === "providerEdit" ? (
            provider.name
          ) : kind === "recipientEdit" ? (
            recipient.address
          ) : kind === "webhookEdit" ? (
            webhook.name
          ) : kind === "smtp" ? (
            <>
              받을 사람은{" "}
              <Link href="/admin/notifications/mail">알림 → 메일</Link>에서
              정합니다.
            </>
          ) : undefined
        }

        actions={
          kind === "providerNew" || kind === "providerEdit" ? (
            <Link
              href={
                kind === "providerNew"
                  ? "/admin/oauth"
                  : `/admin/oauth/${provider.id}`
              }
              className={buttonClassName({ variant: "secondary" })}
            >
              {kind === "providerNew" ? "목록으로" : "상세로"}
            </Link>
          ) : undefined
        }
      />
      {isForm ? (
        <FormStack>
          {explanation ? <Banner tone="info" title={explanation} /> : null}
          {content}
        </FormStack>
      ) : (
        <div className={layout.section}>{content}</div>
      )}
    </section>
  );
}
const meta = {
  id: "product-ui-impl-admin",
  title: "Product/UI Impl/Administration",
  component: AdminPage,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [
    productFrame((args) => {
      const page = pages[args.kind as keyof typeof pages] ?? pages.users;
      return { path: page[0], label: page[1], role: "admin" };
    }),
  ],
  args: { kind: "users" },
} satisfies Meta<typeof AdminPage>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Users: Story = { args: { kind: "users" } };
export const User: Story = { args: { kind: "user" } };
export const UserEdit: Story = { args: { kind: "userEdit" } };
export const UserNew: Story = { args: { kind: "userNew" } };
export const Pending: Story = { args: { kind: "pending" } };
export const Security: Story = { args: { kind: "security" } };
export const Oauth: Story = { args: { kind: "oauth" } };
export const Provider: Story = { args: { kind: "provider" } };
export const ProviderEdit: Story = { args: { kind: "providerEdit" } };
export const ProviderNew: Story = { args: { kind: "providerNew" } };
export const Tokens: Story = { args: { kind: "tokens" } };
export const Token: Story = { args: { kind: "token" } };
export const TokenNew: Story = { args: { kind: "tokenNew" } };
export const Secrets: Story = { args: { kind: "secrets" } };
export const Mail: Story = { args: { kind: "mail" } };
export const Recipient: Story = { args: { kind: "recipient" } };
export const RecipientEdit: Story = { args: { kind: "recipientEdit" } };
export const RecipientNew: Story = { args: { kind: "recipientNew" } };
export const Webhooks: Story = { args: { kind: "webhooks" } };
export const Webhook: Story = { args: { kind: "webhook" } };
export const WebhookEdit: Story = { args: { kind: "webhookEdit" } };
export const WebhookNew: Story = { args: { kind: "webhookNew" } };
export const General: Story = { args: { kind: "general" } };
export const Smtp: Story = { args: { kind: "smtp" } };
