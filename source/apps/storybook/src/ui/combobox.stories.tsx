import { Combobox, Field, type ComboboxOption } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { useArgs } from "storybook/preview-api";
import { expect, fn, userEvent, within } from "storybook/test";

// Common values are suggested; anything else can still be typed (an open list).
const headers: ComboboxOption[] = [
  { value: "X-Forwarded-For", hint: "클라이언트 주소" },
  { value: "X-Forwarded-Proto", hint: "http / https" },
  { value: "X-Real-IP" },
  { value: "Host", label: "Host", hint: "원래 호스트" },
];

const meta = {
  title: "UI/Combobox",
  component: Combobox,
  decorators: [
    (Story) => (
      <main
        style={{ maxWidth: 1200, margin: "0 auto", width: "100%", minWidth: 0 }}
      >
        <h1>자유 입력 목록</h1>
        <Story />
      </main>
    ),
  ],
  parameters: {
    docs: {
      subtitle: "32px 자유 입력과 제안 목록. 키보드·필터 동작 유지",
      description: {
        component: [
          "흔한 값은 목록에서 고르고, 목록에 없는 값도 그대로 입력·저장하는 **열린 목록**이다(nginx 헤더 이름처럼). 값이 반드시 목록 중 하나여야 하면 `Select`를 쓴다.",
          "",
          "- **항상 제어형:** `value` + `onChange`. 입력 문자열이 곧 값이다. `name`을 주면 입력 자체가 폼에 실린다.",
          "- **거르기:** 입력한 문자열을 이름과 힌트(`hint`)에서 찾는다. 값이 옵션과 정확히 같으면 전체 목록을 보여 다시 고를 수 있다.",
          "- **입력을 지킨다:** 목록이 열려도 미리 활성화된 항목이 없다. 방향키로 고른 뒤에만 Enter가 값을 바꾸고, Esc·포커스 이탈은 목록만 닫는다.",
          "- **키보드:** ↓로 열고 이동, ↑로 이동(첫 항목에서 한 번 더 누르면 입력으로), Enter로 확정, Esc로 닫기, Tab은 다음 필드로.",
        ].join("\n"),
      },
    },
  },
  args: {
    options: headers,
    value: "",
    onChange: fn(),
    placeholder: "헤더 이름",
    "aria-label": "헤더 이름",
  },
  argTypes: {
    options: {
      description:
        "제안 목록. `label`을 생략하면 `value`를 보여 주고, `hint`는 오른쪽에 흐리게 붙는다",
      table: {
        type: { summary: "{ value: string; label?: string; hint?: string }[]" },
      },
    },
    value: {
      control: "text",
      description: "입력된 문자열(항상 부모가 가진다)",
    },
    onChange: { description: "입력하거나 제안을 고르면 새 문자열로 호출" },
    placeholder: { description: "비어 있을 때 문구" },
    disabled: { description: "입력할 수 없게 막는다" },
    name: { description: "폼 제출용 이름. 입력 값이 그대로 실린다" },
    id: { control: false, description: "입력의 id(`<label htmlFor>` 연결용)" },
    "aria-label": { description: "화면에 라벨이 없을 때 읽어 줄 이름" },
    "aria-describedby": {
      control: false,
      description: "도움말·오류 문구의 id. `Field` 안에서는 자동으로 연결된다",
    },
    "aria-invalid": {
      description:
        "오류 상태. `Field`에 `error`가 있으면 자동으로 붙고, 테두리가 `--danger`로 바뀐다(hover·열림에서도 유지)",
    },
    "aria-required": {
      control: false,
      description: "필수 입력. `Field`의 `required`가 자동으로 붙인다",
    },
  },
  // Always controlled: the story args hold the value, so typing and the Controls panel agree.
  render: function Render(args) {
    const [, updateArgs] = useArgs<typeof args>();
    return (
      <div style={{ display: "grid", gap: 8 }}>
        <Combobox
          {...args}
          onChange={(next) => {
            updateArgs({ value: next });
            args.onChange(next);
          }}
        />
        <output>
          입력값: {args.value === "" ? "(비어 있음)" : args.value}
        </output>
      </div>
    );
  },
} satisfies Meta<typeof Combobox>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 비어 있는 입력. 포커스하면 제안 목록이 열린다. */
export const Empty: Story = {};

/** 목록에 없는 값도 그대로 값이 된다. */
export const CustomValue: Story = {
  args: { value: "X-Custom-Header" },
};

/** 비활성. 값은 보이지만 바꿀 수 없다. */
export const Disabled: Story = {
  args: { value: "Host", disabled: true },
};

/** 오류 상태: `Select`와 같이 `Field`가 입력에 `aria-invalid`·`aria-required`와 오류 문구 연결을 넣는다. 테두리는 hover·열림에서도 `--danger`를 유지한다. */
export const Invalid: Story = {
  args: { value: "X Forwarded For", "aria-label": undefined },
  render: function Render(args) {
    const [, updateArgs] = useArgs<typeof args>();
    return (
      <Field
        label="헤더 이름"
        htmlFor="header-invalid"
        error="헤더 이름에는 공백을 쓸 수 없습니다."
        required
      >
        <Combobox
          {...args}
          id="header-invalid"
          onChange={(next) => {
            updateArgs({ value: next });
            args.onChange(next);
          }}
        />
      </Field>
    );
  },
};

export const Hover: Story = {
  play: async ({ canvasElement }) => {
    await userEvent.hover(within(canvasElement).getByRole("combobox"));
  },
};
export const Focus: Story = {
  play: async () => {
    await userEvent.tab();
  },
};
export const Open: Story = {
  play: async ({ canvasElement }) => {
    await userEvent.click(within(canvasElement).getByRole("combobox"));
    await expect(within(canvasElement).getByRole("listbox")).toBeVisible();
  },
};
export const Selected: Story = { args: { value: "Host" }, play: Open.play };
export const Many: Story = {
  args: {
    options: Array.from({ length: 20 }, (_, i) => ({
      value: `X-Header-${i}`,
      hint: `운영 헤더 ${i}`,
    })),
  },
  play: Open.play,
};
export const NoOptions: Story = { args: { options: [] }, play: Open.play };
export const LongKorean: Story = {
  args: {
    value: "",
    options: [
      {
        value: "긴 이름의 운영 서비스에 전달할 사용자 정의 요청 헤더",
        hint: "관리자가 확인할 추가 설명",
      },
    ],
  },
  play: Open.play,
};
export const InvalidFocused: Story = {
  ...Invalid,
  play: async ({ canvasElement }) => {
    await userEvent.tab();
    await expect(within(canvasElement).getByRole("combobox")).toHaveFocus();
  },
};
export const KeyboardAndFreeText: Story = {
  play: async ({ canvasElement }) => {
    const c = within(canvasElement);
    const input = c.getByRole("combobox");
    await userEvent.tab();
    await expect(input).toHaveFocus();
    await userEvent.type(input, "X-Custom");
    await userEvent.keyboard("{Escape}");
    await expect(input).toHaveValue("X-Custom");
    await userEvent.clear(input);
    await userEvent.type(input, "클라이언트");
    await expect(c.getAllByRole("option")).toHaveLength(1);
    await userEvent.keyboard("{ArrowDown}{Enter}");
    await expect(input).toHaveValue("X-Forwarded-For");
    await expect(input).toHaveFocus();
    await userEvent.keyboard("{ArrowDown}{ArrowUp}");
    await expect(input).not.toHaveAttribute("aria-activedescendant");
    await userEvent.keyboard("{Escape}");
    await expect(input).toHaveValue("X-Forwarded-For");
  },
};
