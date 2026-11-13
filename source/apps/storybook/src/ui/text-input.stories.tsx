import { Field, TextArea, TextInput, type TextInputProps } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { useId, type ReactElement } from "react";
import { expect, userEvent, within } from "storybook/test";

/** A labelled control. Ids are per instance, so the Docs page can show every story. */
function Labelled({ label, hint, error, children }: { label: string; hint?: string; error?: string; children: ReactElement<Record<string, unknown>> }) {
  const id = useId();
  return (
    <Field label={label} htmlFor={id} hint={hint} error={error}>
      {children}
    </Field>
  );
}

const meta = {
  title: "UI/TextInput",
  component: TextInput,
  decorators: [(Story) => <main style={{ maxWidth: 1200, margin: "0 auto", width: "100%" }}><h1 style={{ fontSize: "var(--fs-2xl)", margin: "0 0 20px" }}>글자 입력</h1><Story /></main>],
  parameters: {
    docs: {
      subtitle: "글자를 입력하는 칸",
      description: {
        component: [
          "브라우저 기본 `<input>`·`<textarea>`에 앱의 입력 칸 모양(`--border-control`, `--r-2`, `--bg-input`, 높이 `--size-ctl`)을 입힌 것이다. `name`, `value`/`defaultValue`, `required`, `autoComplete`, `inputMode`, `placeholder`, `disabled`, `ref` 등은 그대로 전달되어 폼이 지금처럼 제출한다.",
          "",
          "- `TextInput`의 `type`은 글자 입력 종류(text, email, password, number, url, tel)와 `date`(브라우저의 날짜 선택, 같은 칸 모양)만 받는다. 검색은 `SearchField`, 파일은 `FileInput`, 켜기/끄기는 `Switch`. 체크박스·라디오·숨은 입력은 해당하지 않는다.",
          "- 브라우저가 그리는 부속(날짜 칸의 달력 표시, 스크롤 막대)은 `--color-scheme`으로 테마를 따른다.",
          "- `TextArea mono`: PEM, nginx 지시문처럼 원문을 다루는 칸(Pretendard, 13/20, wrap=off). 네 줄(96px) 이상이며 세로로만 크기를 바꾼다.",
          "- 라벨·도움말·오류는 `Field`로 준다. 오류가 있으면 테두리가 빨갛다(`aria-invalid`). 읽기 전용은 가라앉은 면이다.",
        ].join("\n"),
      },
    },
  },
  args: { placeholder: "app.example.test" },
  render: (args: TextInputProps) => <Labelled label="도메인"><TextInput {...args} /></Labelled>,
} satisfies Meta<typeof TextInput>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Empty: Story = { name: "빈 칸" };

export const Filled: Story = {
  name: "입력함",
  play: async ({ canvasElement }) => {
    const input = within(canvasElement).getByLabelText<HTMLInputElement>("도메인");
    await userEvent.type(input, "api.example.test");
    await expect(input).toHaveValue("api.example.test");
  },
};

export const Invalid: Story = {
  name: "오류",
  render: (args) => <Labelled label="도메인" error="도메인을 하나 이상 입력하세요."><TextInput {...args} /></Labelled>,
  play: async ({ canvasElement }) => {
    const input = within(canvasElement).getByLabelText("도메인");
    await expect(input).toHaveAttribute("aria-invalid", "true");
    await expect(input).toHaveAccessibleDescription("도메인을 하나 이상 입력하세요.");
  },
};

export const Disabled: Story = { name: "사용 불가", args: { disabled: true, defaultValue: "app.example.test" } };

export const ReadOnly: Story = { name: "읽기 전용", args: { readOnly: true, defaultValue: "nwui_tok_3f9a…d1f3" } };

export const Kinds: Story = {
  name: "종류",
  render: () => (
    <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(220px, 1fr))", gap: "var(--sp-4) var(--sp-6)" }}>
      <Labelled label="이메일"><TextInput type="email" autoComplete="email" placeholder="ops@example.test" /></Labelled>
      <Labelled label="비밀번호"><TextInput type="password" autoComplete="current-password" /></Labelled>
      <Labelled label="포트" hint="1~65535"><TextInput type="number" inputMode="numeric" defaultValue="443" /></Labelled>
      <Labelled label="웹훅 주소"><TextInput type="url" placeholder="https://hooks.example.test/…" /></Labelled>
      <Labelled label="만료일" hint="브라우저의 날짜 선택을 쓴다."><TextInput type="date" defaultValue="2026-12-31" /></Labelled>
    </div>
  ),
};

export const TextAreas: Story = {
  name: "여러 줄",
  render: () => (
    <div style={{ display: "grid", gap: "var(--sp-4)" }}>
      <Labelled label="설명" hint="목록에 함께 보입니다."><TextArea defaultValue="운영팀 공용 서버" /></Labelled>
      <Labelled label="인증서 (fullchain PEM)"><TextArea mono placeholder={"-----BEGIN CERTIFICATE-----\n..."} /></Labelled>
      <Labelled label="nginx 지시문" error="세미콜론으로 끝나야 합니다."><TextArea mono defaultValue={"proxy_read_timeout 60s"} /></Labelled>
    </div>
  ),
};

/** 좁은 화면(390): 칸은 폭을 채운다. */
export const Narrow: Story = {
  name: "좁은 화면",
  render: () => (
    <div style={{ display: "grid", gap: "var(--sp-4)" }}>
      <Labelled label="도메인"><TextInput defaultValue="very-long-subdomain-for-layout-testing.customer-portal.internal.example.test" /></Labelled>
      <Labelled label="개인키 (PEM)"><TextArea mono placeholder={"-----BEGIN PRIVATE KEY-----\n..."} /></Labelled>
    </div>
  ),
  globals: { viewport: { value: "ui390", isRotated: false } },
  parameters: { viewport: { options: { ui390: { name: "Mobile 390", styles: { width: "390px", height: "844px" }, type: "mobile" } } } },
};

export const Focused: Story = {
  name: "키보드 포커스",
  play: async ({ canvasElement }) => {
    await userEvent.tab();
    await expect(within(canvasElement).getByLabelText("도메인")).toHaveFocus();
  },
};

export const InvalidFocused: Story = {
  ...Invalid,
  name: "오류와 키보드 포커스",
  play: async (context) => {
    await Invalid.play!(context);
    await userEvent.tab();
    await expect(within(context.canvasElement).getByLabelText("도메인")).toHaveFocus();
  },
};

export const TextAreaInvalidFocused: Story = {
  name: "여러 줄 오류와 키보드 포커스",
  render: () => <Labelled label="추가 지시문" error="지시문을 확인하세요."><TextArea mono defaultValue="proxy_read_timeout 60s" /></Labelled>,
  play: async ({ canvasElement }) => {
    const input = within(canvasElement).getByLabelText("추가 지시문");
    await expect(input).toHaveAttribute("aria-invalid", "true");
    await userEvent.tab();
    await expect(input).toHaveFocus();
  },
};

export const TextAreaFocused: Story = {
  name: "여러 줄 키보드 포커스",
  render: () => <Labelled label="설명"><TextArea placeholder="서비스를 알아볼 수 있는 설명을 입력하세요." /></Labelled>,
  play: async ({ canvasElement }) => {
    await userEvent.tab();
    await expect(within(canvasElement).getByLabelText("설명")).toHaveFocus();
  },
};

export const TextAreaStates: Story = {
  name: "여러 줄 상태와 긴 한글",
  render: () => <div style={{ display: "grid", gap: "var(--sp-4)" }}>
    <Labelled label="설명"><TextArea placeholder="사용자가 알아볼 수 있는 설명을 입력하세요." /></Labelled>
    <Labelled label="읽기 전용 설명"><TextArea readOnly defaultValue="운영팀과 고객지원팀이 함께 사용하는 서비스의 요청 처리 시간을 확인합니다." /></Labelled>
    <Labelled label="사용할 수 없는 설명"><TextArea disabled defaultValue="현재 계정에는 이 설정을 수정할 권한이 없습니다." /></Labelled>
    <Labelled label="서비스를 구분할 수 있는 자세하고 긴 한글 이름과 설명" hint="긴 설명은 좁은 화면에서도 칸의 너비 안에서 줄바꿈합니다."><TextInput defaultValue="운영팀과 고객지원팀이 함께 사용하는 내부 서비스" /></Labelled>
  </div>,
};
