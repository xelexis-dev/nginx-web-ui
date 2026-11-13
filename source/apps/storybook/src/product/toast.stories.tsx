import type { Meta, StoryObj } from "@storybook/react-vite";
import { Button, PageHeader } from "@repo/ui";
import {
  ToastProvider,
  ToastViewport,
  useToast,
} from "@/components/notifications/toast-provider";
import { productFrame, productViewports } from "./frame";
import layout from "@/components/page-layout.module.css";

function ToastTrigger() {
  const { toast } = useToast();
  return (
    <section className={layout.page}>
      <PageHeader
        title="알림"
        description="실제 토스트 컴포넌트입니다. 6초 뒤 닫히며, 닫기 버튼으로도 닫을 수 있습니다."
      />
      <Button
        variant="secondary"
        onClick={() =>
          toast({
            severity: "success",
            title: "설정을 적용했습니다",
            body: "이 알림은 미리보기입니다. 실제 설정은 변경하지 않았습니다.",
          })
        }
      >
        알림 보기
      </Button>
      <div style={{ marginTop: "70vh" }}>
        <Button variant="secondary" style={{ width: "100%" }}>포커스 유지 검사</Button>
      </div>
      <ToastViewport />
    </section>
  );
}
function ToastPreview() {
  return (
    <ToastProvider>
      <ToastTrigger />
    </ToastProvider>
  );
}
const meta = {
  id: "product-toast",
  title: "Product/알림",
  component: ToastPreview,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [
    productFrame(() => ({ path: "/", label: "알림", role: "editor" })),
  ],
} satisfies Meta<typeof ToastPreview>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = { name: "토스트" };

export const Dark: Story = { globals: { theme: "dark" } };
