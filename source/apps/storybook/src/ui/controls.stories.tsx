import { FilterChips, SearchField, Switch } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { expect, userEvent, within } from "storybook/test";

function ListToolbar() {
  const [status, setStatus] = useState("all");
  const [query, setQuery] = useState("");
  return (
    <div
      style={{
        display: "flex",
        flexWrap: "wrap",
        alignItems: "center",
        gap: 12,
      }}
    >
      <FilterChips
        label="상태 필터"
        value={status}
        onChange={setStatus}
        options={[
          { value: "all", label: "전체", count: 13 },
          { value: "active", label: "활성", count: 11 },
          { value: "disabled", label: "비활성", count: 2 },
          { value: "attention", label: "확인 필요", count: 4 },
        ]}
      />
      <SearchField
        label="서버 검색"
        value={query}
        onChange={setQuery}
        placeholder="도메인 · 포트 · 대상"
      />
    </div>
  );
}

function Switches() {
  const [on, setOn] = useState(true);
  return (
    <div
      style={{
        display: "flex",
        flexWrap: "wrap",
        gap: 24,
        alignItems: "center",
      }}
    >
      <Switch
        label="app.example.test 활성"
        checked={on}
        onCheckedChange={setOn}
        showState
      />
      <Switch
        label="legacy.example.test 활성"
        checked={false}
        onCheckedChange={() => undefined}
      />
      <Switch label="읽기 전용" checked disabled />
    </div>
  );
}

const meta = {
  title: "UI/FilterChips · SearchField · Switch",
  component: FilterChips,
  decorators: [
    (Story) => (
      <main
        style={{ maxWidth: 1200, margin: "0 auto", width: "100%", minWidth: 0 }}
      >
        <h1>필터 · 검색 · 스위치</h1>
        <Story />
      </main>
    ),
  ],
  parameters: {
    docs: {
      subtitle: "목록의 도구 막대와 행의 켜기/끄기",
      description: {
        component: [
          "- `FilterChips`: 개수가 붙은 상태 필터. 누른 것 하나가 `aria-pressed`. 묶음 높이는 32px, 선택한 항목은 control 면과 옅은 경계로 구분한다. `disabled`는 `SegmentedControl`처럼 묶음 전체를 막는다(눌린 칩은 `--bg-disabled`에 남는다).",
          "- `SearchField`: 앞에 돋보기가 있는 검색 입력. 이름표는 화면 읽기 프로그램용이다.",
          '- `Switch`: 실제 `<button role="switch" aria-checked>`. `onCheckedChange`로 제어하거나, 폼 안에서 `type="submit"`과 `name`/`value`로 서버에 보낸다. 이름(`label`)에는 무엇을 켜는지 적는다.',
        ].join("\n"),
      },
    },
  },
  args: {
    label: "상태 필터",
    value: "all",
    onChange: () => undefined,
    options: [],
  },
} satisfies Meta<typeof FilterChips>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Toolbar: Story = { render: () => <ListToolbar /> };
export const SwitchStates: Story = { render: () => <Switches /> };

/** 비활성 필터: 목록을 불러오는 동안처럼 묶음 전체를 막는다. 눌린 칩은 `--bg-disabled` 면으로 자리를 지키고, hover는 반응하지 않는다. */
export const FilterChipsDisabled: Story = {
  render: () => (
    <FilterChips
      label="상태 필터"
      value="active"
      onChange={() => undefined}
      disabled
      options={[
        { value: "all", label: "전체", count: 13 },
        { value: "active", label: "활성", count: 11 },
        { value: "disabled", label: "비활성", count: 2 },
      ]}
    />
  ),
};

export const SwitchFocus: Story = {
  render: () => <Switches />,
  play: async ({ canvasElement }) => {
    const c = within(canvasElement);
    await userEvent.tab();
    const control = c.getByRole("switch", { name: "app.example.test 활성" });
    await expect(control).toHaveFocus();
    await userEvent.keyboard(" ");
    await expect(control).toHaveAttribute("aria-checked", "false");
    await userEvent.keyboard("{Enter}");
    await expect(control).toHaveAttribute("aria-checked", "true");
    await expect(c.getByRole("switch", { name: "읽기 전용" })).toBeDisabled();
  },
};
export const SwitchSmall: Story = {
  render: () => <Switch size="sm" label="작은 스위치" checked={true} />,
};
export const SearchFocus: Story = {
  render: () => <ListToolbar />,
  play: async ({ canvasElement }) => {
    const input = within(canvasElement).getByRole("searchbox", {
      name: "서버 검색",
    });
    await userEvent.click(input);
    await userEvent.keyboard("app.example.test");
    await expect(input).toHaveValue("app.example.test");
  },
};
export const FilterFocus: Story = {
  render: () => <ListToolbar />,
  play: async ({ canvasElement }) => {
    const c = within(canvasElement);
    await userEvent.tab();
    await userEvent.tab();
    const active = c.getByRole("button", { name: /^활성/ });
    await expect(active).toHaveFocus();
    await userEvent.keyboard(" ");
    await expect(active).toHaveAttribute("aria-pressed", "true");
  },
};
export const FilterHover: Story = {
  render: () => <ListToolbar />,
  play: async ({ canvasElement }) => {
    await userEvent.hover(
      within(canvasElement).getByRole("button", { name: /^활성/ }),
    );
  },
};
export const LongKorean: Story = {
  render: () => (
    <FilterChips
      label="긴 상태 필터"
      value="all"
      onChange={() => undefined}
      options={[
        { value: "all", label: "모든 운영 서비스의 적용 상태", count: 1234 },
        {
          value: "attention",
          label: "운영 담당자의 확인이 필요한 상태",
          count: 12,
        },
      ]}
    />
  ),
};

export const Spec: Story = Toolbar;
export const States: Story = SwitchStates;
export const SpecDark: Story = { ...Spec, globals: { theme: "dark" } };
export const StatesDark: Story = { ...States, globals: { theme: "dark" } };
