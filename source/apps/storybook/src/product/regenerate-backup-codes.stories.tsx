import type { Meta, StoryObj } from "@storybook/react-vite";
import Link from "next/link";
import { PageHeader, linkClassName } from "@repo/ui";
import { RegenerateBackupCodesForm } from "@/app/(app)/account/2fa/regenerate-backup-codes-form";
import styles from "@/app/(app)/account/2fa/two-factor-management.module.css";
import { productFrame, productViewports } from "./frame";

const meta = {
  id: "product-account-2fa-regenerate",
  title: "Product/Account/2FA Regenerate",
  component: RegenerateBackupCodesForm,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [productFrame(() => ({ path: "/account/2fa/backup-codes/new", label: "백업 코드 재발급", role: "editor" }))],
  render: args => <section className={styles.page}>
    <PageHeader eyebrow={<Link href="/account/2fa" className={linkClassName({ variant: "back" })}>2FA 관리로</Link>} title="백업 코드 재발급" info="인증 앱 설정은 유지하고 백업 코드만 새로 발급합니다." />
    <RegenerateBackupCodesForm {...args} />
  </section>,
  args: { hasPassword: true },
} satisfies Meta<typeof RegenerateBackupCodesForm>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = { name: "비밀번호로 본인 확인" };
export const OAuthOnly: Story = { name: "OAuth 계정 · 인증 앱 확인", args: { hasPassword: false } };
export const Mobile: Story = { name: "백업 코드 재발급 · 390", globals: { viewport: { value: "p390", isRotated: false } } };
