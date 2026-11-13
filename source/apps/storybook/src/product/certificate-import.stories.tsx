import layout from "@/components/page-layout.module.css";
// Product: the real apps/web certificate import form (components/certificates/import-form.tsx), composed
// as app/(app)/certificates/import/page.tsx composes it: header, then the form. The file choosers are
// @repo/ui FileInput: choosing a file fills the text box in this browser only, and 가져오기 goes to the
// Storybook stand-in of use-api-action, which sends nothing. No certificate or key is included here.
import type { Meta, StoryObj } from "@storybook/react-vite";
import Link from "next/link";
import { buttonClassName, PageHeader } from "@repo/ui";
import { ImportForm } from "@/components/certificates/import-form";
import { productFrame, productViewports } from "./frame";

const meta = {
  id: "product-certificates-import",
  title: "Product/Certificates/Import",
  component: ImportForm,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [
    productFrame(() => ({ path: "/certificates/import", label: "인증서 가져오기", role: "editor" })),
  ],
  // Wrapped in <section className={layout.page}> exactly as the route is: the shell's <main> is a grid with a gap, so bare
  // siblings would add that gap on top of the form card's own top margin.
  render: (args) => (
    <section className={layout.page}>
      <PageHeader
        title="인증서 가져오기"
        actions={<Link href={args.cancelHref} className={buttonClassName({ variant: "secondary" })}>취소</Link>}
        info="보유한 PEM 인증서와 개인키를 붙여넣거나 파일로 업로드해 등록합니다. 도메인·만료일은 인증서에서 자동 추출됩니다."
      />
      <ImportForm {...args} />
    </section>
  ),
  // The local import route's own action and cancel path.
  args: { action: { id: "certificates.importCertificate", args: [] }, cancelHref: "/certificates" },
} satisfies Meta<typeof ImportForm>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = { name: "인증서 가져오기" };
export const Mobile: Story = { name: "인증서 가져오기 · 390", globals: { viewport: { value: "p390", isRotated: false } } };
