import { useState } from "react";
import { Button } from "@repo/ui";
import { ConfirmDialog } from "../../../../packages/ui/src/confirm-dialog";
import type { Meta, StoryObj } from "@storybook/react-vite";

const meta = {
  title: "UI/ConfirmDialog",
  component: ConfirmDialog,
  parameters: {
    docs: {
      description: {
        component:
          "480px 네이티브 dialog, 경고 아이콘과 대상 이름, sunken 푸터. 처리 중 잠금과 기존 닫기 동작 유지.",
      },
    },
  },
  args: {
    title: "서버 삭제",
    description: "선택한 항목을 삭제합니다. 이 작업은 되돌릴 수 없습니다.",
    target: "app.example.test",
    confirmLabel: "삭제",
    pending: false,
    open: false,
    onClose: () => {},
    onConfirm: () => {},
  },
  decorators: [
    (Story) => (
      <main
        style={{
          maxWidth: 1200,
          margin: "0 auto",
          width: "100%",
          boxSizing: "border-box",
        }}
      >
        <h1>삭제 확인</h1>
        <Story />
      </main>
    ),
  ],
  render: function Preview(args) {
    const [open, setOpen] = useState(args.open);
    const [confirmed, setConfirmed] = useState(false);
    return (
      <>
        <Button onClick={() => setOpen(true)}>삭제 확인 열기</Button>
        <p role="status">
          {confirmed
            ? "확인했습니다. 미리보기에서는 삭제하지 않습니다."
            : "미리보기에서는 데이터를 삭제하지 않습니다."}
        </p>
        <ConfirmDialog
          {...args}
          open={open}
          onClose={() => setOpen(false)}
          onConfirm={() => {
            setConfirmed(true);
            setOpen(false);
          }}
        />
      </>
    );
  },
} satisfies Meta<typeof ConfirmDialog>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const Open: Story = { args: { open: true }, tags: ["!autodocs"] };
export const Busy: Story = {
  args: { open: true, pending: true },
  tags: ["!autodocs"],
};
export const NoTarget: Story = { args: { target: undefined } };
export const LongKorean: Story = {
  args: {
    open: true,
    title: "운영 서버와 연결된 인증서를 삭제하시겠습니까?",
    target: "서울 운영 노드 · 아주 긴 한국어 이름의 서버와 연결된 인증서",
    description: "삭제할 대상을 확인하세요. ".repeat(15),
  },
  tags: ["!autodocs"],
};

export const Spec: Story = Default;
export const States: Story = Busy;
export const SpecDark: Story = { ...Spec, globals: { theme: "dark" } };
export const StatesDark: Story = { ...States, globals: { theme: "dark" } };
