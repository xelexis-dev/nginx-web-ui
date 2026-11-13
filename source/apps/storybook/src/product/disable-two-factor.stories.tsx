import type { Meta, StoryObj } from "@storybook/react-vite";
import Link from "next/link";
import { PageHeader, linkClassName } from "@repo/ui";
import { TwoFactorDisable } from "@/app/(app)/account/2fa/two-factor-forms";
import styles from "@/app/(app)/account/2fa/two-factor-management.module.css";
import { productFrame, productViewports } from "./frame";
const meta = {
  id: "product-account-2fa-disable",
  title: "Product/Account/2FA Disable",
  component: TwoFactorDisable,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [productFrame(() => ({ path: "/account/2fa/disable", label: "2FA 해제", role: "editor" }))],
  render: () => <section className={styles.page}>
    <PageHeader eyebrow={<Link href="/account/2fa" className={linkClassName({ variant: "back" })}>2FA 관리로</Link>} title="2FA 해제" description="해제하면 기존 인증 앱과 백업 코드로 인증할 수 없고, 등록한 패스키도 모두 삭제됩니다. 현재 비밀번호를 확인합니다." />
    <TwoFactorDisable />
  </section>,
} satisfies Meta<typeof TwoFactorDisable>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = { name: "2FA 해제" };
export const Mobile: Story = { name: "2FA 해제 · 390", globals: { viewport: { value: "p390", isRotated: false } } };
