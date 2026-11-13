import { Button, Field, Select, type SelectOption } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { useArgs } from "storybook/preview-api";
import { expect, fn, userEvent, within } from "storybook/test";

const protocols: SelectOption[] = [
  { value: "tcp", label: "TCP" },
  { value: "udp", label: "UDP" },
];
// More than five options: the search box appears automatically.
const roles: SelectOption[] = [
  "admin",
  "editor",
  "viewer",
  "auditor",
  "operator",
  "guest",
  "service",
].map((value) => ({ value, label: value }));

const meta = {
  title: "UI/Select",
  component: Select,
  decorators: [
    (Story) => (
      <main
        style={{ maxWidth: 1200, margin: "0 auto", width: "100%", minWidth: 0 }}
      >
        <h1>선택 목록</h1>
        <Story />
      </main>
    ),
  ],
  parameters: {
    docs: {
      subtitle: "정해진 목록에서 하나를 고르는 드롭다운",
      description: {
        component: [
          "앱의 **유일한 드롭다운**이다. 네이티브 `<select>` 대신 항상 이것을 쓴다. 값은 반드시 `options` 중 하나다(목록에 없는 값을 받아야 하면 `Combobox`).",
          "",
          "- **폼 제출:** `name`을 주면 숨은 입력이 값을 싣는다(비제어형, `defaultValue`로 초기값).",
          "- **제어형:** `value` + `onChange`. 부모가 값을 가진다.",
          "- **검색:** 옵션이 5개보다 많으면 목록 위에 검색창이 자동으로 나온다(`searchable`로 강제). 이름으로 거르고, 없으면 「결과 없음」.",
          "- **키보드:** 닫힌 상태에서 Enter·Space·↓로 열고, ↑/↓로 이동, Enter로 선택, Esc로 닫는다. 바깥을 누르면 닫힌다.",
          "- 라벨이 화면에 없으면 `aria-label`로 이름을 준다.",
        ].join("\n"),
      },
    },
  },
  args: { options: protocols, "aria-label": "프로토콜", onChange: fn() },
  argTypes: {
    options: {
      description: "고를 수 있는 값과 표시 이름",
      table: { type: { summary: "{ value: string; label: string }[]" } },
    },
    // `value` makes Select controlled; only the Controlled story wires it (below).
    value: {
      control: false,
      description: "제어형 값. 주면 부모가 값을 가진다(`onChange` 필요)",
    },
    // Read once on mount: editing it afterwards would change nothing on screen.
    defaultValue: {
      control: false,
      description: "비제어형 초기값. 처음 그릴 때 한 번만 읽는다",
    },
    onChange: { description: "선택이 바뀌면 새 값으로 호출" },
    name: { description: "주면 같은 이름의 숨은 입력이 값을 폼에 싣는다" },
    placeholder: {
      description: "아무것도 고르지 않았을 때 문구",
      table: { defaultValue: { summary: '"선택하세요"' } },
    },
    searchable: {
      description:
        "검색창 표시를 강제한다. 생략하면 옵션이 5개보다 많을 때만 보인다",
    },
    disabled: { description: "열 수 없게 막는다" },
    id: {
      control: false,
      description: "트리거 버튼의 id(`<label htmlFor>` 연결용)",
    },
    "aria-label": { description: "화면에 라벨이 없을 때 읽어 줄 이름" },
    "aria-invalid": {
      description:
        "오류 상태. `Field`에 `error`가 있으면 자동으로 붙고, 테두리가 `--danger`로 바뀐다(hover·열림에서도 유지)",
    },
  },
} satisfies Meta<typeof Select>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 비제어형: `name`으로 숨은 입력이 값을 폼에 싣는다. 초기값은 `defaultValue`. */
export const Uncontrolled: Story = {
  args: { name: "protocol", defaultValue: "tcp" },
};

/** 아직 고르지 않은 상태. `placeholder` 문구가 보인다. */
export const Placeholder: Story = {};

/** 옵션이 5개보다 많으면 검색창이 나온다. 없는 이름을 입력하면 「결과 없음」. */
export const SearchableWhenLong: Story = {
  args: { options: roles, "aria-label": "역할" },
};

/** 비활성. 값은 보이지만 열리지 않는다. */
export const Disabled: Story = {
  args: { defaultValue: "udp", disabled: true },
};

/** 제어형: 스토리 인자가 값을 가지므로 Controls와 목록이 함께 바뀐다. */
export const Controlled: Story = {
  args: { value: "udp" },
  argTypes: {
    value: { control: "inline-radio", options: protocols.map((o) => o.value) },
  },
  render: function Render(args) {
    const [, updateArgs] = useArgs<typeof args>();
    return (
      <div style={{ display: "grid", gap: 8 }}>
        <Select
          {...args}
          onChange={(next) => {
            updateArgs({ value: next });
            args.onChange?.(next);
          }}
        />
        <output>선택: {args.value}</output>
      </div>
    );
  },
};

/** 폼 안에서: 제출하면 숨은 입력의 값이 `FormData`에 실린다. */
export const InForm: Story = {
  args: { name: "protocol", defaultValue: "tcp" },
  render: function Render(args) {
    const [sent, setSent] = useState<string | null>(null);
    return (
      <form
        style={{ display: "grid", gap: 12 }}
        onSubmit={(event) => {
          event.preventDefault();
          setSent(String(new FormData(event.currentTarget).get("protocol")));
        }}
      >
        <Select {...args} />
        <div>
          <Button type="submit">저장</Button>
        </div>
        <output>
          {sent === null ? "아직 제출하지 않음" : `제출된 값: ${sent}`}
        </output>
      </form>
    );
  },
  play: async ({ canvasElement }) => {
    const c = within(canvasElement);
    await userEvent.click(c.getByRole("button", { name: "프로토콜" }));
    await userEvent.click(c.getByRole("button", { name: "UDP" }));
    await userEvent.click(c.getByRole("button", { name: "저장" }));
    await expect(c.getByText("제출된 값: udp")).toBeVisible();
  },
};

/** 오류 상태: `Field`의 `error`가 트리거에 `aria-invalid`와 오류 문구 연결(`aria-describedby`)을 넣는다. 테두리는 hover·열림에서도 `--danger`를 유지하고, 포커스 링은 그대로다. */
export const Invalid: Story = {
  args: { "aria-label": undefined },
  render: (args) => (
    <Field
      label="프로토콜"
      htmlFor="protocol-invalid"
      error="프로토콜을 고르세요."
      required
    >
      <Select {...args} id="protocol-invalid" />
    </Field>
  ),
};

export const Hover: Story = {
  play: async ({ canvasElement }) => {
    await userEvent.hover(
      within(canvasElement).getByRole("button", { name: "프로토콜" }),
    );
  },
};
export const Focus: Story = {
  play: async () => {
    await userEvent.tab();
  },
};
export const Open: Story = {
  play: async ({ canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByRole("button", { name: "프로토콜" }),
    );
    await expect(within(canvasElement).getByRole("listbox")).toBeVisible();
  },
};
export const Selected: Story = {
  args: { defaultValue: "tcp" },
  play: Open.play,
};
export const Many: Story = {
  ...SearchableWhenLong,
  play: async ({ canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByRole("button", { name: "역할" }),
    );
    await expect(
      within(canvasElement).getByRole("textbox", { name: "검색" }),
    ).toBeVisible();
  },
};
export const NoOptions: Story = { args: { options: [] }, play: Open.play };
export const LongKorean: Story = {
  args: {
    defaultValue: "long",
    options: [
      {
        value: "long",
        label: "서비스 운영을 담당하는 관리자와 긴 이름의 유지보수 담당자",
      },
      {
        value: "other",
        label: "읽기 전용으로 모든 운영 상태를 확인하는 담당자",
      },
    ],
  },
  play: Open.play,
};
export const InvalidFocused: Story = {
  ...Invalid,
  play: async ({ canvasElement }) => {
    await userEvent.tab();
    await expect(
      within(canvasElement).getByRole("button", { name: /프로토콜/ }),
    ).toHaveFocus();
  },
};
export const KeyboardAndSearch: Story = {
  ...SearchableWhenLong,
  play: async ({ canvasElement }) => {
    const c = within(canvasElement);
    const trigger = c.getByRole("button", { name: "역할" });
    await userEvent.tab();
    await expect(trigger).toHaveFocus();
    await userEvent.keyboard("{ArrowDown}");
    const search = c.getByRole("textbox", { name: "검색" });
    await userEvent.type(search, "editor");
    await expect(c.getAllByRole("option")).toHaveLength(1);
    await userEvent.keyboard("{Enter}");
    await expect(trigger).toHaveTextContent("editor");
    await expect(trigger).toHaveFocus();
    await userEvent.keyboard(" ");
    await userEvent.type(c.getByRole("textbox", { name: "검색" }), "없는역할");
    await expect(c.getByText("결과 없음")).toBeVisible();
    await userEvent.keyboard("{Escape}");
    await expect(trigger).toHaveAttribute("aria-expanded", "false");
    await expect(trigger).toHaveFocus();
  },
};

/** Exact 5/6 boundary and overrides remain the existing contract. */
export const FiveOptions: Story = {
  args: { options: roles.slice(0, 5) },
  play: Open.play,
};
export const SixOptions: Story = {
  args: { options: roles.slice(0, 6) },
  play: Open.play,
};
export const SearchForcedOn: Story = {
  args: { options: protocols, searchable: true },
  play: Open.play,
};
export const SearchForcedOff: Story = {
  args: { options: roles, searchable: false },
  play: Open.play,
};
