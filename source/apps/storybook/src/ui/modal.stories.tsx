import { Button, Modal } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ComponentProps } from "react";
import { useArgs, useState } from "storybook/preview-api";
import { fn } from "storybook/test";

// `withFooter` and `longContent` are story switches: the footer buttons need this story's close
// handler, and the body text is the story's own example.
type Args = ComponentProps<typeof Modal> & {
  withFooter: boolean;
  longContent: boolean;
};

const meta = {
  title: "UI/Modal",
  component: Modal,
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
        <h1>대화상자</h1>
        <Story />
      </main>
    ),
  ],
  parameters: {
    docs: {
      subtitle: "위험한 동작을 확인하는 네이티브 대화상자",
      description: {
        component: [
          "네이티브 `<dialog>` + `showModal()`로 연다. 포커스 가두기, Esc 닫기, 배경 비활성(inert)은 브라우저가 맡는다. 헤더(제목·닫기) / 본문 / 푸터(동작 버튼) 세 칸이다.",
          "",
          "- **열고 닫기는 부모가 가진다:** `open`으로 열고, 어떤 경로(닫기 버튼, 배경 클릭, Esc)로 닫혀도 `onClose` 하나로 알려 준다. `onClose`에서 `open`을 `false`로 되돌린다.",
          "- **본문이 길면 본문만 스크롤된다.** 대화상자는 화면 위에서 64px 떨어지고, 헤더와 푸터는 제자리에 있다.",
          "- 생성·수정 같은 긴 입력은 모달 대신 별도 페이지로 만든다(이 앱의 규칙). 모달은 확인·짧은 선택에 쓴다.",
        ].join("\n"),
      },
    },
  },
  args: {
    open: false,
    dismissible: true,
    description: "변경 사항을 확인한 뒤 진행하세요.",
    title: "설정 적용",
    onClose: fn(),
    children: null,
    withFooter: true,
    longContent: false,
  },
  argTypes: {
    open: { control: "boolean", description: "열림 여부(부모가 가진다)" },
    title: { description: "헤더 제목. 대화상자의 이름으로도 읽힌다" },
    onClose: { description: "닫기 버튼·배경 클릭·Esc로 닫힐 때 호출" },
    // Rendered by the story itself, so not editable here.
    children: {
      control: false,
      description: "본문",
      table: { type: { summary: "ReactNode" } },
    },
    footer: {
      control: false,
      description: "푸터(동작 버튼 묶음). 생략하면 푸터 칸이 없다",
      table: { type: { summary: "ReactNode" } },
    },
    withFooter: {
      control: "boolean",
      description: "(예제용) 취소·적용 푸터를 붙인다",
      table: { category: "예제" },
    },
    longContent: {
      control: "boolean",
      description: "(예제용) 긴 본문으로 스크롤을 보여 준다",
      table: { category: "예제" },
    },
  },
  // Native <dialog> + showModal(): focus trap, Esc and the inert background come from the browser.
  // `open` lives in the story args, so the Controls toggle and the dialog's own close agree.
  render: function Render({ withFooter, longContent, ...args }) {
    const [, updateArgs] = useArgs<Args>();
    const close = () => {
      updateArgs({ open: false });
      args.onClose();
    };
    return (
      <>
        <Button onClick={() => updateArgs({ open: true })}>모달 열기</Button>
        <Modal
          {...args}
          onClose={close}
          footer={
            withFooter ? (
              <>
                <Button
                  type="button"
                  variant="secondary"
                  disabled={!args.dismissible}
                  onClick={close}
                >
                  취소
                </Button>
                <Button disabled={!args.dismissible} onClick={close}>
                  적용
                </Button>
              </>
            ) : undefined
          }
        >
          {longContent ? (
            Array.from({ length: 30 }, (_, i) => (
              <p key={i} style={{ margin: "0 0 12px" }}>
                {i + 1}. api-{String(i + 1).padStart(2, "0")}.example.test —
                설정 변경 1건
              </p>
            ))
          ) : (
            <p style={{ margin: 0 }}>
              변경된 설정 3개를 nginx 에 적용합니다. 적용 중에는 다른 설정을
              저장할 수 없습니다.
            </p>
          )}
        </Modal>
      </>
    );
  },
} satisfies Meta<Args>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 확인이 필요한 동작: 취소·적용 푸터가 있다. 「모달 열기」로 연다. */
export const WithFooter: Story = {};

/** 알림만 하는 대화상자: 푸터 없이 닫기(×)·Esc로 닫는다. */
export const WithoutFooter: Story = {
  args: { withFooter: false },
};

/** 본문이 길면 본문만 스크롤되고 헤더·푸터는 고정된다. */
export const LongContent: Story = {
  args: { longContent: true },
};

/**
 * 처음부터 열린 상태(Canvas 전용). showModal()은 페이지 전체를 막으므로 Docs 페이지에는 싣지
 * 않는다.
 */
export const OpenInitially: Story = {
  args: { open: true },
  tags: ["!autodocs"],
};

/** 설정 적용 안에서 변경 내역을 확인하는 실제 중첩 구성. 안쪽 창만 닫으면 바깥 창은 유지된다. */
export const Nested: Story = {
  name: "중첩된 변경 내역",
  tags: ["!autodocs"],
  argTypes: { withFooter: { control: false }, longContent: { control: false } },
  render: function NestedDialogs({ open, title, onClose }) {
    const [, updateArgs] = useArgs<Args>();
    const [detailOpen, setDetailOpen] = useState(false);
    return (
      <>
        <Button onClick={() => updateArgs({ open: true })}>
          설정 적용 창 열기
        </Button>
        <Modal
          open={open}
          title={title}
          onClose={() => {
            updateArgs({ open: false });
            onClose();
          }}
        >
          <Button onClick={() => setDetailOpen(true)}>변경 내역 보기</Button>
          <Modal
            open={detailOpen}
            title="미적용 변경 내역"
            onClose={() => setDetailOpen(false)}
          >
            <p>이 창을 Escape나 닫기로 닫아도 설정 적용 창은 유지됩니다.</p>
          </Modal>
        </Modal>
      </>
    );
  },
};

export const LongKorean: Story = {
  args: {
    open: true,
    title: "긴 한국어 제목이 여러 줄로 이어지는 확인 대화상자",
    description:
      "아주 긴 한국어 설명이 이어져도 닫기 버튼과 대화상자의 내용은 화면 너비 안에서 읽을 수 있어야 합니다. ".repeat(
        4,
      ),
    longContent: true,
  },
  tags: ["!autodocs"],
};

export const Warning: Story = {
  args: { open: true, icon: "warning" },
  tags: ["!autodocs"],
};
export const Danger: Story = {
  args: { open: true, icon: "danger" },
  tags: ["!autodocs"],
};

export const Review: Story = {
  args: { size: "review", open: true, longContent: true },
  tags: ["!autodocs"],
};
export const Locked: Story = {
  args: { size: "review", open: true, dismissible: false },
  tags: ["!autodocs"],
};
export const Spec: Story = WithFooter;
export const States: Story = Locked;
export const SpecDark: Story = { ...Spec, globals: { theme: "dark" } };
export const StatesDark: Story = { ...States, globals: { theme: "dark" } };
