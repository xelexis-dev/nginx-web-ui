// WP-3H: production admin/account-token components, safe DTO fixtures, no network or stored secrets.
import { expect, userEvent, within } from "storybook/test";
import { AdminTwoFactor } from "@/app/admin/users/admin-two-factor";
import { AdminPasskeys } from "@/components/passkeys/admin-passkeys";
import { passkeyListView } from "@/lib/passkey-api";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Banner, FormStack, Button, PageHeader, Select } from "@repo/ui";
import { ApiForm } from "@/components/api-form";
import { UserForm } from "@/app/admin/users/user-form";
import { EditUserForms } from "@/app/admin/users/[id]/edit/edit-user-forms";
import { UserList } from "@/app/admin/users/user-list";
import { UserDetailView } from "@/app/admin/users/user-detail";
import { UserDetailActions } from "@/app/admin/users/[id]/detail-actions";
import { PendingListView } from "@/app/admin/pending/pending-list";
import { ProviderForm } from "@/app/admin/oauth/provider-form";
import { ProviderListView } from "@/app/admin/oauth/provider-list";
import {
  ProviderDetailView,
  ProviderDetailActions,
} from "@/app/admin/oauth/provider-detail";
import { BaseUrlForm } from "@/app/admin/settings/general/base-url-form";
import { MetricsForm } from "@/app/admin/settings/general/metrics-form";
import { SmtpForm } from "@/app/admin/settings/smtp/smtp-form";
import { SecurityForm } from "@/app/admin/security/security-form";
import { ConnectedGateways } from "@/app/admin/security/connected-gateways";
import { SecretsHealthView } from "@/app/admin/secrets/secrets-health";
import { MailRecipientForm } from "@/app/admin/notifications/mail/recipient-form";
import { RecipientListView } from "@/app/admin/notifications/mail/recipient-list";
import { RecipientDetailView } from "@/app/admin/notifications/mail/recipient-detail";
import { MailRecipientDetailActions } from "@/app/admin/notifications/mail/[id]/detail-actions";
import { WebhookForm } from "@/app/admin/notifications/webhooks/webhook-form";
import { WebhookListView } from "@/app/admin/notifications/webhooks/webhook-list";
import { WebhookDetailView } from "@/app/admin/notifications/webhooks/webhook-detail";
import { WebhookDetailActions } from "@/app/admin/notifications/webhooks/[id]/detail-actions";
import { TokenForm } from "@/app/(app)/account/tokens/token-form";
import {
  AccountTokensView,
  ServerTokensView,
} from "@/app/(app)/account/tokens/token-list";
import { TokenDetail } from "@/app/(app)/account/tokens/token-detail";
import type { TokenScope } from "@repo/contracts";
import layout from "@/components/page-layout.module.css";
import { ProductFrame, productViewports } from "./frame";

const CANARY = "WP3H_STORED_SECRET_MUST_NOT_RENDER";
const user = {
  id: "user-1",
  email: "operator@example.test",
  displayName: "운영자",
  role: "editor" as const,
  enabled: true,
  approved: true,
  hasPassword: true,
  totpEnabled: false,
  methods: "비밀번호 · OIDC",
  providers: ["OIDC"],
  createdAt: "2026-10-01T05:00:00Z",
  passwordHash: CANARY,
};
const provider = {
  id: "oidc-1",
  name: "회사 로그인",
  issuer: "https://identity.example.test",
  clientId: "nwui-preview",
  scope: "openid email profile",
  enabled: true,
  sortOrder: 0,
  linkedUsers: 2,
  callbackUrl: "https://nginx.example.test/api/auth/callback/oidc-1",
  clientSecret: CANARY,
};
const recipient = {
  id: "mail-1",
  label: "운영팀",
  address: "operators@example.test",
  enabled: true,
  events: ["certificate.failed"],
  lastAttemptAt: null,
  lastSuccessAt: null,
  lastError: null,
};
const webhook = {
  id: "hook-1",
  name: "운영 알림",
  enabled: true,
  format: "generic" as const,
  events: ["certificate.failed"],
  lastAttemptAt: null,
  lastSuccessAt: null,
  lastStatus: null,
  lastError: null,
  bodyTemplate: "",
  maskedUrl: "https://hooks.example.test/…",
  headerNames: ["Authorization"],
  openError: null,
  url: CANARY,
  headers: CANARY,
};
const smtp = {
  enabled: true,
  host: "smtp.example.test",
  port: 587,
  secure: false,
  username: "notifications@example.test",
  fromEmail: "notifications@example.test",
  fromName: "nginx-web-ui",
  rejectUnauthorized: true,
  password: CANARY,
};
const token = {
  id: "token-1",
  name: "운영 도구",
  masked: "nwui_••••abcd",
  scopes: ["config:read"],
  status: "active" as const,
  createdAt: "2026. 10. 1. 오전 5:00 UTC",
  lastUsedAt: "사용 기록 없음",
  expiresAt: "2027. 1. 1. 오전 5:00 UTC",
  revokedAt: "해지되지 않음",
  revoked: false,
  token: CANARY,
};
const serverScopes: TokenScope[] = [
  "node:pair",
  "config:read",
  "config:write",
  "nginx:reload",
];
const userScopes: TokenScope[] = [
  "config:read",
  "config:write",
  "nginx:reload",
];
const scenes = {
  "user-2fa-on": ["/admin/users/user-1", "사용자"],
  "user-2fa-confirm": ["/admin/users/user-1", "사용자"],
  "user-2fa-wrong": ["/admin/users/user-1", "사용자"],
  "user-2fa-conflict": ["/admin/users/user-1", "사용자"],
  "user-2fa-rate": ["/admin/users/user-1", "사용자"],
  "user-2fa-success": ["/admin/users/user-1", "사용자"],
  "user-2fa-off": ["/admin/users/user-1", "사용자"],
  "user-2fa-self": ["/admin/users/user-1", "사용자"],
  "user-2fa-oidc": ["/admin/users/user-1", "사용자"],
  "user-2fa-zero": ["/admin/users/user-1", "사용자"],
  "user-new": ["/admin/users/new", "새 사용자"],
  "user-edit": ["/admin/users/user-1/edit", "사용자 수정"],
  "user-oauth-edit": ["/admin/users/user-1/edit", "사용자 수정"],
  users: ["/admin/users", "사용자"],
  "user-detail": ["/admin/users/user-1", "사용자"],
  "user-self": ["/admin/users/user-1", "사용자"],
  pending: ["/admin/pending", "승인 대기"],
  "oauth-new": ["/admin/oauth/new", "공급자 추가"],
  "oauth-edit": ["/admin/oauth/oidc-1/edit", "공급자 수정"],
  "oauth-list": ["/admin/oauth", "OAuth 로그인"],
  "oauth-detail": ["/admin/oauth/oidc-1", "회사 로그인"],
  general: ["/admin/settings/general", "일반"],
  smtp: ["/admin/settings/smtp", "SMTP 설정"],
  security: ["/admin/security", "보안 — 2FA 강제"],
  secrets: ["/admin/secrets", "저장된 비밀값"],
  "mail-new": ["/admin/notifications/mail/new", "새 수신처"],
  "mail-edit": ["/admin/notifications/mail/mail-1/edit", "수신처 수정"],
  "mail-list": ["/admin/notifications/mail", "메일"],
  "mail-detail": ["/admin/notifications/mail/mail-1", "운영팀"],
  "webhook-new": ["/admin/notifications/webhooks/new", "새 webhook"],
  "webhook-edit": ["/admin/notifications/webhooks/hook-1/edit", "webhook 수정"],
  "webhook-list": ["/admin/notifications/webhooks", "Webhook"],
  "webhook-detail": ["/admin/notifications/webhooks/hook-1", "운영 알림"],
  "server-token-new": ["/admin/tokens/new", "새 서버 토큰"],
  "server-token-list": ["/admin/tokens", "서버 액세스 토큰"],
  "server-token-detail": ["/admin/tokens/token-1", "운영 도구"],
  "user-token-new": ["/account/tokens/new", "새 액세스 토큰"],
  "viewer-token-new": ["/account/tokens/new", "새 액세스 토큰"],
  "user-token-list": ["/account/tokens", "액세스 토큰"],
  "user-token-detail": ["/account/tokens/token-1", "운영 도구"],
} as const;
type Kind = keyof typeof scenes;
const explanations: Partial<Record<Kind, React.ReactNode>> = {
  "user-new": <>이메일, 비밀번호, 역할을 지정해 새 계정을 생성합니다.</>,
  "oauth-new": (
    <>
      공급자 쪽에서 애플리케이션을 먼저 만들고, 거기서 받은 클라이언트 ID 와
      비밀키를 넣으세요. 저장하면 등록해야 할 콜백 주소를 알려 드립니다.
    </>
  ),
  general: <>사이트 전체에 적용되는 설정입니다.</>,
  smtp: <>앱이 메일을 보낼 때 쓰는 서버입니다.</>,
  security: <>역할별로 2단계 인증(2FA)을 강제할지 설정합니다.</>,
  "mail-new": <>인증서 알림을 받을 주소를 등록합니다.</>,
  "webhook-new": <>알림을 받을 HTTP 수신처를 추가합니다.</>,
  "server-token-new": (
    <>
      다른 게이트웨이가 이 서버를 노드로 연결할 때 제시하는 토큰입니다. 이름에
      어느 게이트웨이에 줄 것인지 적어 두세요.
    </>
  ),
  "user-token-new": <>고른 권한만 이 토큰으로 할 수 있습니다.</>,
  "viewer-token-new": <>고른 권한만 이 토큰으로 할 수 있습니다.</>,
};
function AdminPage({ kind }: { kind: Kind }) {
  const header = (children: React.ReactNode) => (
    <section className={layout.page}>
      <PageHeader title={scenes[kind][1]} />
      {explanations[kind] ? (
        <FormStack>
          <Banner tone="info" title={explanations[kind]} />
          {children}
        </FormStack>
      ) : (
        children
      )}
    </section>
  );
  if (kind === "user-new") return header(<UserForm />);
  if (kind === "user-edit" || kind === "user-oauth-edit")
    return header(
      <EditUserForms
        id={user.id}
        email={user.email}
        currentRole={user.role}
        hasPassword={kind !== "user-oauth-edit"}
      />,
    );
  if (kind === "users")
    return (
      <UserList
        items={[
          user,
          {
            ...user,
            id: "viewer-1",
            email: "viewer@example.test",
            role: "viewer",
          },
        ]}
        selfUserId="actor"
      />
    );
  if (kind.startsWith("user-2fa-")) {
    const isSelf = kind === "user-2fa-self";
    const canRemove = kind !== "user-2fa-oidc";
    const target = {
      ...user,
      totpEnabled: !["user-2fa-off", "user-2fa-success"].includes(kind),
    };
    const keys = passkeyListView({
      availability: { available: true, reason: null },
      currentOrigin: "https://admin.example.test",
      hasPassword: true,
      canRemove,
      passkeys: ["user-2fa-zero", "user-2fa-success"].includes(kind)
        ? []
        : [
            {
              id: "key-1",
              name: "개인 노트북",
              origin: "https://admin.example.test",
              rpId: "admin.example.test",
              createdAt: user.createdAt,
              lastUsedAt: null,
              backupEligible: true,
              backupState: false,
              unavailableHere: false,
            },
          ],
    });
    return (
      <UserDetailView
        user={target}
        isSelf={isSelf}
        detailActions={
          <UserDetailActions id={user.id} isSelf={isSelf} enabled />
        }
        twoFactor={
          <AdminTwoFactor
            user={target}
            isSelf={isSelf}
            canReset={canRemove}
            passkeyCount={keys.passkeys.length}
            reset={kind === "user-2fa-success"}
          />
        }
        passkeys={
          <AdminPasskeys
            data={keys}
            isSelf={isSelf}
            userId={user.id}
            email={user.email}
          />
        }
      />
    );
  }
  if (kind === "user-detail" || kind === "user-self")
    return (
      <UserDetailView
        user={user}
        isSelf={kind === "user-self"}
        detailActions={
          <UserDetailActions
            id={user.id}
            isSelf={kind === "user-self"}
            enabled
          />
        }
      />
    );
  if (kind === "pending")
    return (
      <PendingListView
        pending={[
          {
            id: user.id,
            email: user.email,
            displayName: user.displayName,
            provider: "OIDC",
            createdAt: user.createdAt,
          },
        ]}
        approvalForm={(u) => (
          <ApiForm
            id={`approve-${u.id}`}
            action={{ id: "admin.pending.approve", args: [] }}
          >
            <input type="hidden" name="id" value={u.id} />
            <Select
              size="sm"
              name="role"
              defaultValue="viewer"
              options={["viewer", "editor", "admin"].map((value) => ({
                value,
                label: value,
              }))}
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
  if (kind === "oauth-new" || kind === "oauth-edit")
    return header(
      <ProviderForm
        mode={kind === "oauth-new" ? "create" : "edit"}
        defaults={kind === "oauth-edit" ? provider : undefined}
        callbackUrl={kind === "oauth-edit" ? provider.callbackUrl : undefined}
      />,
    );
  if (kind === "oauth-list")
    return (
      <ProviderListView
        providers={[provider]}
        toggleForm={(p) => (
          <ApiForm action={{ id: "admin.oauth.toggle", args: [] }}>
            <input type="hidden" name="id" value={p.id} />
            <Button size="sm" type="submit" variant="secondary">
              {p.enabled ? "끄기" : "켜기"}
            </Button>
          </ApiForm>
        )}
      />
    );
  if (kind === "oauth-detail")
    return (
      <ProviderDetailView
        provider={provider}
        deleteForm={<ProviderDetailActions id={provider.id} />}
      />
    );
  if (kind === "general")
    return header(
      <>
        <BaseUrlForm defaultValue="https://nginx.example.test" />
        <MetricsForm
          defaultRetentionDays="30"
          defaultExcludedHosts="admin.example.test"
        />
      </>,
    );
  if (kind === "smtp")
    return header(<SmtpForm defaults={smtp} hasStoredPassword />);
  if (kind === "security")
    return header(
      <>
        <SecurityForm requireAdmin requireUser={false} requireOauth={false} />
        <ConnectedGateways
          rows={[
            {
              id: "gateway-1",
              name: "운영 게이트웨이",
              role: "editor",
              createdAt: user.createdAt,
              lastUsedAt: null,
              lastUsedIp: null,
              revokedAt: null,
            },
          ]}
        />
      </>,
    );
  if (kind === "secrets")
    return (
      <SecretsHealthView
        rows={[
          {
            group: "SMTP",
            label: "비밀번호",
            state: "sealed-ok",
            impact: "메일",
            recovery: "다시 입력",
          },
        ]}
        broken={[]}
        plaintext={[]}
      />
    );
  if (kind === "mail-new" || kind === "mail-edit")
    return header(
      <MailRecipientForm
        mode={kind === "mail-new" ? "create" : "edit"}
        smtpReady
        defaults={{
          ...recipient,
          id: kind === "mail-edit" ? recipient.id : undefined,
          label: recipient.label ?? "",
        }}
      />,
    );
  if (kind === "mail-list")
    return <RecipientListView recipients={[recipient]} mailer={{ ok: true }} />;
  if (kind === "mail-detail")
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
  if (kind === "webhook-new" || kind === "webhook-edit")
    return header(
      <WebhookForm
        mode={kind === "webhook-new" ? "create" : "edit"}
        defaults={{
          ...webhook,
          id: kind === "webhook-edit" ? webhook.id : undefined,
        }}
        maskedUrl={kind === "webhook-edit" ? webhook.maskedUrl : undefined}
        storedHeaderNames={kind === "webhook-edit" ? webhook.headerNames : []}
      />,
    );
  if (kind === "webhook-list") return <WebhookListView targets={[webhook]} />;
  if (kind === "webhook-detail")
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
  const server = kind.startsWith("server-");
  const basePath = server ? "/admin/tokens" : "/account/tokens";
  if (kind.endsWith("token-new"))
    return header(
      <TokenForm
        kind={server ? "server" : "user"}
        scopes={
          server
            ? serverScopes
            : kind === "viewer-token-new"
              ? ["config:read"]
              : userScopes
        }
        basePath={basePath}
        action={{
          id: server ? "admin.tokens.issue" : "account.tokens.issue",
          args: [],
        }}
      />,
    );
  if (kind.endsWith("token-list"))
    return server ? (
      <ServerTokensView rows={[token]} />
    ) : (
      <AccountTokensView rows={[token]} />
    );
  return (
    <TokenDetail
      view={token}
      basePath={basePath}
      revoke={{
        id: server ? "admin.tokens.revoke" : "account.tokens.revoke",
        args: [],
      }}
    />
  );
}
const meta = {
  id: "product-admin-pages",
  title: "Product/Admin package",
  component: AdminPage,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  args: { kind: "user-new" },
  decorators: [
    (Story, context) => (
      <ProductFrame
        path={scenes[context.args.kind!][0]}
        label={scenes[context.args.kind!][1]}
        role={context.args.kind === "viewer-token-new" ? "viewer" : "admin"}
        storyId={context.id}
        viewport={undefined}
      >
        <Story />
      </ProductFrame>
    ),
  ],
} satisfies Meta<typeof AdminPage>;
export default meta;
type Story = StoryObj<typeof meta>;
export const UserNew: Story = { args: { kind: "user-new" } };
export const UserEdit: Story = { args: { kind: "user-edit" } };
export const UserOauthEdit: Story = { args: { kind: "user-oauth-edit" } };
export const Users: Story = { args: { kind: "users" } };
export const UserDetail: Story = { args: { kind: "user-detail" } };
export const UserSelf: Story = { args: { kind: "user-self" } };
export const Pending: Story = { args: { kind: "pending" } };
export const OauthNew: Story = { args: { kind: "oauth-new" } };
export const OauthEdit: Story = { args: { kind: "oauth-edit" } };
export const OauthList: Story = { args: { kind: "oauth-list" } };
export const OauthDetail: Story = { args: { kind: "oauth-detail" } };
export const General: Story = { args: { kind: "general" } };
export const Smtp: Story = { args: { kind: "smtp" } };
export const Security: Story = { args: { kind: "security" } };
export const Secrets: Story = { args: { kind: "secrets" } };
export const MailNew: Story = { args: { kind: "mail-new" } };
export const MailEdit: Story = { args: { kind: "mail-edit" } };
export const MailList: Story = { args: { kind: "mail-list" } };
export const MailDetail: Story = { args: { kind: "mail-detail" } };
export const WebhookNew: Story = { args: { kind: "webhook-new" } };
export const WebhookEdit: Story = { args: { kind: "webhook-edit" } };
export const WebhookList: Story = { args: { kind: "webhook-list" } };
export const WebhookDetail: Story = { args: { kind: "webhook-detail" } };
export const ServerTokenNew: Story = { args: { kind: "server-token-new" } };
export const ServerTokenList: Story = { args: { kind: "server-token-list" } };
export const ServerTokenDetail: Story = {
  args: { kind: "server-token-detail" },
};
export const UserTokenNew: Story = { args: { kind: "user-token-new" } };
export const ViewerTokenNew: Story = { args: { kind: "viewer-token-new" } };
export const UserTokenList: Story = { args: { kind: "user-token-list" } };
export const UserTokenDetail: Story = { args: { kind: "user-token-detail" } };

export const UserTwoFactorOn: Story = { args: { kind: "user-2fa-on" } };
export const UserTwoFactorOff: Story = { args: { kind: "user-2fa-off" } };
export const UserTwoFactorSelf: Story = { args: { kind: "user-2fa-self" } };
export const UserTwoFactorOidc: Story = { args: { kind: "user-2fa-oidc" } };
export const UserTwoFactorSuccess: Story = {
  args: { kind: "user-2fa-success" },
};
const openReset: NonNullable<Story["play"]> = async ({ canvasElement }) => {
  const canvas = within(canvasElement);
  await userEvent.click(
    canvas.getByRole("button", { name: "2FA 초기화" }),
  );
  await expect(canvas.getByRole("dialog")).toBeVisible();
  await expect(canvas.getByLabelText("현재 비밀번호")).toBeVisible();
};
export const UserTwoFactorConfirm: Story = {
  args: { kind: "user-2fa-confirm" },
  play: openReset,
};
export const UserTwoFactorZero: Story = {
  args: { kind: "user-2fa-zero" },
  play: openReset,
};
const errorReset = (
  kind: "user-2fa-wrong" | "user-2fa-conflict" | "user-2fa-rate",
): Story => ({
  args: { kind },
  play: async (context) => {
    await openReset(context);
    const dialog = within(context.canvasElement).getByRole("dialog");
    const message =
      kind === "user-2fa-wrong"
        ? "현재 비밀번호가 올바르지 않습니다."
        : kind === "user-2fa-conflict"
          ? "2단계 인증이 설정되어 있지 않습니다."
          : "실패가 잦아 잠시 차단되었습니다. 15분 뒤에 다시 시도하세요.";
    window.__NWUI_PREVIEW_ACTION__ = async () =>
      kind === "user-2fa-wrong"
        ? { ok: false, fieldErrors: { currentPassword: message } }
        : { ok: false, error: message };
    try {
      await userEvent.type(
        within(dialog).getByLabelText("현재 비밀번호"),
        "preview-password",
      );
      await userEvent.click(
        within(dialog).getByRole("button", { name: "2FA 초기화" }),
      );
      await expect(within(dialog).getByText(message)).toBeVisible();
    } finally {
      delete window.__NWUI_PREVIEW_ACTION__;
    }
  },
});
export const UserTwoFactorWrongPassword: Story = errorReset("user-2fa-wrong");
export const UserTwoFactorConflict: Story = errorReset("user-2fa-conflict");
export const UserTwoFactorRateLimited: Story = errorReset("user-2fa-rate");
