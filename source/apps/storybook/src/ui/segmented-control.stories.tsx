import {
  Button,
  SearchField,
  Select,
  SegmentedControl,
  Switch,
} from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { expect, userEvent, within } from "storybook/test";

function Policy({ disabled = false }: { disabled?: boolean }) {
  const [value, setValue] = useState(false);
  return (
    <SegmentedControl
      label="관리자 계정 2FA 강제"
      options={[
        { value: false, label: "해제" },
        { value: true, label: "강제" },
      ]}
      value={value}
      onChange={setValue}
      disabled={disabled}
    />
  );
}

function Theme() {
  const [value, setValue] = useState<"system" | "light" | "dark">("system");
  return (
    <SegmentedControl
      label="테마"
      options={[
        { value: "system", label: "시스템" },
        { value: "light", label: "라이트" },
        { value: "dark", label: "다크" },
      ]}
      value={value}
      onChange={setValue}
    />
  );
}

const meta = {
  title: "UI/SegmentedControl",
  component: SegmentedControl,
  decorators: [
    (Story) => (
      <main
        style={{ maxWidth: 1200, margin: "0 auto", width: "100%", minWidth: 0 }}
      >
        <h1>선택 묶음</h1>
        <Story />
      </main>
    ),
  ],
  parameters: {
    docs: {
      subtitle: "몇 개 중 하나를 고르는 버튼 묶음",
      description: {
        component: [
          '이름 있는 묶음(`role="group"`) 안의 버튼들이고, 고른 것이 눌린 상태(`aria-pressed`)다. 폼 안의 설정값(「해제 / 강제」, 「시스템 / 라이트 / 다크」)에 쓴다.',
          "",
          "- 개수가 붙은 상태 거르기는 `FilterChips`, 따로 문구가 없는 켜기/끄기는 `Switch`.",
          "- 문구는 쓰는 쪽이 정한다. 제어형이라 부모가 `value`를 가진다(폼에 실으려면 숨은 입력으로).",
          "- 묶음 높이는 32px, `size=sm`은 28px이다. 묶음에는 `--bg-sunken` 배경과 4px 내부 여백을 둔다. 고른 항목은 `--bg-canvas` 면과 `--text-1` 글자, 옅은 `--line-strong` 경계로 구분한다. 선택된 버튼은 hover 중에도 그대로다. 비활성은 `--text-disabled`, 고른 것은 `--bg-disabled` 면으로 남는다.",
        ].join("\n"),
      },
    },
  },
  args: {
    label: "",
    options: [],
    value: false as never,
    onChange: () => undefined,
  },
  argTypes: {
    options: { control: false },
    value: { control: false },
    onChange: { control: false },
  },
} satisfies Meta<typeof SegmentedControl>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 켜기/끄기에 제 문구가 있는 설정(관리자 2FA 강제). 누르면 고른 것이 바뀐다. */
export const TwoOptions: Story = {
  name: "두 값",
  render: () => <Policy />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(
      canvas.getByRole("group", { name: "관리자 계정 2FA 강제" }),
    ).toBeInTheDocument();
    await expect(canvas.getByRole("button", { name: "해제" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await userEvent.click(canvas.getByRole("button", { name: "강제" }));
    await expect(canvas.getByRole("button", { name: "강제" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(canvas.getByRole("button", { name: "해제" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  },
};

export const ThreeOptions: Story = { name: "세 값", render: () => <Theme /> };

/** 막힌 상태: 눌러도 바뀌지 않는다. */
export const Disabled: Story = {
  name: "사용 불가",
  render: () => <Policy disabled />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const on = canvas.getByRole("button", { name: "강제" });
    await expect(on).toBeDisabled();
    await userEvent.click(on, { pointerEventsCheck: 0 });
    await expect(on).toHaveAttribute("aria-pressed", "false");
    await expect(canvas.getByRole("button", { name: "해제" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  },
};

function LongLabels() {
  const [value, setValue] = useState("all");
  return (
    <SegmentedControl
      label="알림 받을 사건"
      options={[
        { value: "all", label: "모든 적용 결과" },
        { value: "failed", label: "실패한 적용만" },
        { value: "cert", label: "인증서 만료 임박" },
        { value: "none", label: "받지 않음" },
      ]}
      value={value}
      onChange={setValue}
    />
  );
}

/** 좁은 화면(390)에서 문구가 길면 줄을 바꿔 가로로 넘치지 않는다. */
export const NarrowLongLabels: Story = {
  name: "좁은 화면 · 긴 문구",
  render: () => <LongLabels />,
  globals: { viewport: { value: "ui390", isRotated: false } },
  parameters: {
    viewport: {
      options: {
        ui390: {
          name: "Mobile 390",
          styles: { width: "390px", height: "844px" },
          type: "mobile",
        },
      },
    },
  },
};

/** 키보드: Tab으로 버튼마다 들어오고 Enter·Space로 고른다. 포커스 링이 보인다. */
export const Keyboard: Story = {
  name: "키보드",
  render: () => <Policy />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.tab();
    await userEvent.tab();
    const on = canvas.getByRole("button", { name: "강제" });
    await expect(on).toHaveFocus();
    await expect(on.matches(":focus-visible")).toBe(true);
    await userEvent.keyboard(" ");
    await expect(on).toHaveAttribute("aria-pressed", "true");
  },
};

function ControlSizes() {
  const [value, setValue] = useState("table");
  const [query, setQuery] = useState("");
  const [checked, setChecked] = useState(false);
  return (
    <div style={{ display: "grid", gap: "var(--sp-6)" }}>
      {(["sm", "md"] as const).map((size) => (
        <div
          key={size}
          data-control-size={size}
          style={{
            display: "flex",
            flexWrap: "wrap",
            alignItems: "center",
            gap: "var(--sp-3)",
          }}
        >
          <SegmentedControl
            size={size}
            label={`${size} 보기`}
            options={[
              { value: "table", label: "표" },
              { value: "raw", label: "Raw" },
            ]}
            value={value}
            onChange={setValue}
          />
          <SearchField
            size={size}
            label={`${size} 검색`}
            value={query}
            onChange={setQuery}
            placeholder="로그 검색"
          />
          <div style={{ width: 160 }}>
            <Select
              size={size}
              aria-label={`${size} 메서드`}
              options={[
                { value: "all", label: "전체 메서드" },
                { value: "get", label: "GET" },
              ]}
              defaultValue="all"
            />
          </div>
          <Button size={size} variant="secondary">
            Clear
          </Button>
          <Switch
            size={size}
            checked={checked}
            label={`${size} 활성`}
            onCheckedChange={setChecked}
          />
        </div>
      ))}
    </div>
  );
}
export const Sizes: Story = {
  name: "공통 컨트롤 · 작은 크기 / 기본",
  render: () => <ControlSizes />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(
      canvas
        .getByRole("group", { name: "sm 보기" })
        .querySelectorAll("button")[1]!,
    );
    await expect(
      within(canvas.getByRole("group", { name: "sm 보기" })).getByRole(
        "button",
        { name: "Raw" },
      ),
    ).toHaveAttribute("aria-pressed", "true");
    const selected = within(
      canvas.getByRole("group", { name: "sm 보기" }),
    ).getByRole("button", { name: "Raw" });
    await userEvent.hover(selected);
    await expect(selected).toHaveAttribute("aria-pressed", "true");
  },
};

export const Hover: Story = {
  render: () => <Policy />,
  play: async ({ canvasElement }) => {
    await userEvent.hover(
      within(canvasElement).getByRole("button", { name: "강제" }),
    );
  },
};
