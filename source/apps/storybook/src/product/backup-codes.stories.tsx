// Product: the real apps/web backup codes card (app/(app)/account/2fa/backup-codes-card.tsx) inside
// the /account/2fa page frame, as the page shows it once new codes exist: after enabling 2FA
// ("enrollment": the finish form ends the setup) or after issuing new codes from 2FA management
// ("regenerated": a link back to it). The codes are samples of the real shape. 백업 코드 TXT 저장 makes
// a real .txt download in this browser (nothing is sent); the finish form goes to the Storybook
// stand-in of use-api-action, which sends nothing and says so.
import type { Meta, StoryObj } from "@storybook/react-vite";
import { PageHeader } from "@repo/ui";
import { BackupCodesCard } from "@/app/(app)/account/2fa/backup-codes-card";
import styles from "@/app/(app)/account/2fa/two-factor.module.css";
import { productFrame, productViewports } from "./frame";

const CODES = ["aaaaa-00001", "bbbbb-00002", "ccccc-00003", "ddddd-00004", "eeeee-00005", "fffff-00006", "0a1b2-c3d4e", "9f8e7-d6c5b"];

const meta = {
  id: "product-account-2fa-backup-codes",
  title: "Product/Account/2FA Backup Codes",
  component: BackupCodesCard,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [productFrame(() => ({ path: "/account/2fa", label: "2단계 인증", role: "editor" }))],
  // Composed as app/(app)/account/2fa/page.tsx composes it.
  render: (args) => (
    <section className={styles.wrap}>
      <div>
        {args.context === "regenerated" ? (
          <PageHeader title="백업 코드 재발급" info="인증 앱 설정은 유지하고 백업 코드만 새로 발급합니다." />
        ) : (
          <PageHeader
            title="2단계 인증 (2FA) 설정"
            description="인증 앱(Google Authenticator, 1Password 등)으로 QR을 스캔한 뒤, 앱에 표시된 6자리 코드를 입력해 활성화합니다."
          />
        )}
      </div>
      <BackupCodesCard {...args} />
    </section>
  ),
  args: { codes: CODES },
} satisfies Meta<typeof BackupCodesCard>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 2FA를 켠 직후: 코드 8개를 한 번만 보여 주고, TXT로 저장할 수 있다. 「저장했습니다 — 계정으로」가 설정을 마친다. */
export const Enrollment: Story = { name: "2FA 설정 후" };
/** 2FA 관리에서 백업 코드를 새로 발급한 뒤: 이전 코드는 더 이상 쓸 수 없다. 2FA 관리로 돌아가는 링크. */
export const Regenerated: Story = { name: "새로 발급 후", args: { context: "regenerated" } };
export const Mobile: Story = { name: "2FA 설정 후 · 390", globals: { viewport: { value: "p390", isRotated: false } } };
