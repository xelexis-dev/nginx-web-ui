import type { Meta, StoryObj } from "@storybook/react-vite";
import { PageHeader, Surface, Banner, FormStack } from "@repo/ui";
import { ProductFrame, productViewports } from "./frame";
import layout from "@/components/page-layout.module.css";
import { AccountOverview } from "@/app/(app)/account/account-overview";
import { ChangePasswordForm } from "@/app/(app)/account/change-password-form";
import {
  TwoFactorEnroll,
  TwoFactorVerify,
  TwoFactorDisable,
} from "@/app/(app)/account/2fa/two-factor-forms";
import { TwoFactorManagement } from "@/app/(app)/account/2fa/two-factor-management";
import { RegenerateBackupCodesForm } from "@/app/(app)/account/2fa/regenerate-backup-codes-form";
import { BackupCodesCard } from "@/app/(app)/account/2fa/backup-codes-card";
import {
  RegisterPasskeyForm,
  RenamePasskeyForm,
  RemovePasskeyForm,
} from "@/components/passkeys/passkey-forms";
import { AccountPasskeys } from "@/components/passkeys/passkey-list";
import type { PasskeyListView } from "@/lib/passkey-api";
const row = {
  id: "fixture-key",
  rpId: "account.example.test",
  origin: "https://account.example.test",
  name: "개인 노트북",
  address: "https://account.example.test",
  createdAt: "2026. 11. 1. 오전 9:00 UTC",
  lastUsedAt: "사용 기록 없음",
  backupEligible: true,
  backupState: true,
  unavailableHere: false,
};
const data: PasskeyListView = {
  passkeys: [row],
  availability: { available: true, reason: null },
  currentAddress: "https://account.example.test",
  hasPassword: true,
  canRemove: true,
  currentOrigin: "https://account.example.test",
};
const qr =
  "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='220' height='220'%3E%3Crect width='220' height='220' fill='white'/%3E%3Ctext x='20' y='110'%3EQR preview%3C/text%3E%3C/svg%3E";
const scenes = {
  account: ["/account", "계정"],
  oauth: ["/account", "계정"],
  password: ["/account/password", "비밀번호 변경"],
  enroll: ["/account/2fa", "2단계 인증 (2FA) 설정"],
  verify: ["/account/2fa", "2단계 인증 확인"],
  management: ["/account/2fa", "2FA 관리"],
  "management-oauth": ["/account/2fa", "2FA 관리"],
  regenerate: ["/account/2fa/backup-codes/new", "백업 코드 재발급"],
  "regenerate-oauth": ["/account/2fa/backup-codes/new", "백업 코드 재발급"],
  disable: ["/account/2fa/disable", "2FA 해제"],
  "passkey-new": ["/account/passkeys/new", "패스키 등록"],
  "passkey-rename": ["/account/passkeys/fixture-key/edit", "패스키 이름 변경"],
  "passkey-remove": ["/account/passkeys/fixture-key", "개인 노트북"],
  "passkey-unavailable": ["/account", "계정"],
  "passkey-limit": ["/account", "계정"],
  "passkey-oauth": ["/account", "계정"],
  "backup-codes": ["/account/2fa", "2단계 인증 (2FA) 설정"],
} as const;
type Kind = keyof typeof scenes;
const explanations: Partial<Record<Kind, string>> = {
  enroll:
    "인증 앱(Google Authenticator, 1Password 등)으로 QR을 스캔한 뒤, 앱에 표시된 6자리 코드를 입력해 활성화합니다.",
  verify: "계속하려면 이 로그인 세션의 인증을 완료하세요.",
  regenerate: "인증 앱 설정은 유지하고 백업 코드만 새로 발급합니다.",
  "regenerate-oauth": "인증 앱 설정은 유지하고 백업 코드만 새로 발급합니다.",
  disable:
    "해제하면 기존 인증 앱과 백업 코드로 인증할 수 없고, 등록한 패스키도 모두 삭제됩니다. 현재 비밀번호를 확인합니다.",
};
function AccountPage({ kind }: { kind: Kind }) {
  if (kind === "account" || kind === "oauth")
    return (
      <AccountOverview
        account={{
          email: "operator@example.test",
          hasPassword: kind !== "oauth",
          totpEnabled: true,
          logins: [
            {
              providerId: "company",
              name: "회사 로그인",
              linkedAt: "2026-10-01T05:00:00Z",
            },
            { providerId: "extra", name: "보조 로그인", linkedAt: null },
          ],
        }}
        passkeys={{ ...data, hasPassword: kind !== "oauth" }}
      />
    );
  if (kind === "management" || kind === "management-oauth")
    return <TwoFactorManagement hasPassword={kind === "management"} />;
  const content =
    kind === "password" ? (
      <ChangePasswordForm />
    ) : kind === "enroll" ? (
      <TwoFactorEnroll secret="FIXTURE-NOT-A-REAL-SECRET" qrDataUrl={qr} />
    ) : kind === "verify" ? (
      <TwoFactorVerify />
    ) : kind === "disable" ? (
      <TwoFactorDisable />
    ) : kind.startsWith("regenerate") ? (
      <RegenerateBackupCodesForm hasPassword={kind === "regenerate"} />
    ) : kind === "passkey-new" ? (
      <RegisterPasskeyForm data={data} />
    ) : kind === "passkey-rename" ? (
      <RenamePasskeyForm row={row} />
    ) : kind === "passkey-remove" ? (
      <Surface title="패스키 삭제">
        <RemovePasskeyForm
          action={{ id: "account.passkeys.remove", args: [] }}
          id={row.id}
          canRemove
        />
      </Surface>
    ) : kind === "backup-codes" ? (
      <BackupCodesCard
        codes={["abcd1234", "efgh5678", "ijkl9012", "mnop3456"]}
      />
    ) : (
      <>
        <AccountPasskeys
          data={{
            ...data,
            hasPassword: kind !== "passkey-oauth",
            availability: {
              available: kind !== "passkey-unavailable",
              reason: kind === "passkey-unavailable" ? "insecure_origin" : null,
            },
            passkeys:
              kind === "passkey-limit"
                ? Array.from({ length: 10 }, (_, i) => ({
                    ...row,
                    id: `key-${i}`,
                    name: `기기 ${i}`,
                  }))
                : [],
          }}
        />
      </>
    );
  return (
    <section className={layout.page}>
      <PageHeader title={scenes[kind][1]} />
      {explanations[kind] ? (
        <FormStack>
          <Banner tone="info" title={explanations[kind]} />
          {content}
        </FormStack>
      ) : (
        content
      )}
    </section>
  );
}
const meta = {
  id: "product-account-pages",
  title: "Product/계정 설정 적용",
  component: AccountPage,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  args: { kind: "account" },
  decorators: [
    (Story, context) => (
      <ProductFrame
        path={scenes[context.args.kind][0]}
        label={scenes[context.args.kind][1]}
        role="admin"
        storyId={context.id}
        viewport={undefined}
      >
        <Story />
      </ProductFrame>
    ),
  ],
} satisfies Meta<typeof AccountPage>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Account: Story = {};
export const OAuth: Story = { args: { kind: "oauth" } };
export const Password: Story = { args: { kind: "password" } };
export const Enroll: Story = { args: { kind: "enroll" } };
export const Verify: Story = { args: { kind: "verify" } };
export const Management: Story = { args: { kind: "management" } };
export const ManagementOAuth: Story = { args: { kind: "management-oauth" } };
export const Regenerate: Story = { args: { kind: "regenerate" } };
export const RegenerateOAuth: Story = { args: { kind: "regenerate-oauth" } };
export const Disable: Story = { args: { kind: "disable" } };
export const PasskeyNew: Story = { args: { kind: "passkey-new" } };
export const PasskeyRename: Story = { args: { kind: "passkey-rename" } };
export const PasskeyRemove: Story = { args: { kind: "passkey-remove" } };
export const PasskeyUnavailableState: Story = {
  args: { kind: "passkey-unavailable" },
};
export const PasskeyLimit: Story = { args: { kind: "passkey-limit" } };
export const PasskeyOAuth: Story = { args: { kind: "passkey-oauth" } };
export const BackupCodes: Story = { args: { kind: "backup-codes" } };
