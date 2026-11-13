import { Checkbox, FieldGroup } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";

const meta = {
  title: "UI/Checkbox",
  component: Checkbox,
  decorators: [(Story) => <main style={{ maxWidth: 1200, margin: "0 auto", width: "100%" }}><h1 style={{ fontSize: "var(--fs-2xl)", margin: "0 0 20px" }}>체크박스와 라디오</h1><Story /></main>],
  parameters: {
    docs: {
      subtitle: "글로 설명하는 예/아니오 하나",
      description: {
        component: [
          '- 브라우저 기본 `<input type="checkbox">`를 이름표 안에 둔다. `name`·`value`·`checked`/`defaultChecked`·`onChange`·`required`·`disabled`·`form`이 그대로 전달되어 폼은 예전처럼 값을 보낸다. Space로 바꾸고, 글자를 눌러도 바뀐다.',
          "- `description`은 흐린 작은 글자로 아래에 붙고 입력의 설명(`aria-describedby`)이 된다.",
          "- `tile`은 공통 테두리·canvas 면·12px 안쪽 여백을 갖는 선택 타일이다. 생략하면 기존 줄 모양이다.",
          "- 글자 크기와 색은 놓인 자리를 따른다(폼 본문, 도구 막대의 작은 글자). `className`·`style`은 상자가 아니라 줄 전체에 붙는다(타일 테두리 등).",
          "- 상자는 16px이며 `--border-strong` 경계를 쓴다. 선택은 `--accent`와 고정 원본 체크 표시로 보이고, 일부 선택도 지원한다. 네이티브 입력과 키보드 포커스는 유지한다.",
          "- 글자 없이 켜고 끄는 설정은 `Switch`, 여러 개를 한 이름으로 묶으면 `FieldGroup`.",
        ].join("\n"),
      },
    },
  },
  args: { label: "저장된 비밀번호 지우기" },
} satisfies Meta<typeof Checkbox>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 기본: 글자를 눌러도 바뀐다. */
export const Default: Story = {
  play: async ({ canvasElement }) => {
    const box = within(canvasElement).getByRole("checkbox", {
      name: "저장된 비밀번호 지우기",
    });
    await expect(box).not.toBeChecked();
    await userEvent.click(
      within(canvasElement).getByText("저장된 비밀번호 지우기"),
    );
    await expect(box).toBeChecked();
    await userEvent.click(box);
    await expect(box).not.toBeChecked();
  },
};

/** 설명이 붙은 체크박스: 설명은 입력의 aria-describedby가 된다. */
export const WithDescription: Story = {
  args: {
    id: "ack",
    label: "기존 백업 코드가 모두 무효화되는 것을 확인했습니다.",
    description: "새 코드를 받으면 이전 코드로는 로그인할 수 없습니다.",
    required: true,
  },
  play: async ({ canvasElement }) => {
    const box = within(canvasElement).getByRole("checkbox");
    await expect(box).toHaveAttribute("aria-describedby", "ack-description");
    await expect(box).toBeRequired();
  },
};

/** 꺼진 상태와 켜진 채 꺼진 상태. */
export const Disabled: Story = {
  render: () => (
    <div style={{ display: "grid", gap: "var(--sp-3)" }}>
      <Checkbox label="꺼진 체크박스" disabled />
      <Checkbox label="켜진 채 꺼진 체크박스" defaultChecked disabled />
    </div>
  ),
};

/** 여러 개를 한 이름으로: FieldGroup 안의 체크박스 타일(알림 사건 고르기). 오류는 묶음 전체를 설명한다. */
export const InFieldGroup: Story = {
  name: "FieldGroup 안에서",
  render: () => (
    <FieldGroup
      id="events"
      legend="보낼 사건"
      error="사건을 하나 이상 고르세요."
    >
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fit, minmax(240px, 1fr))",
          gap: "var(--sp-2) var(--sp-6)",
        }}
      >
        {[
          [
            "certificate.expiring",
            "인증서 · 만료 예정",
            "만료 30일 전부터 하루 한 번",
          ],
          ["apply.failed", "적용 · 실패", "nginx -t 또는 reload 실패"],
          ["node.unreachable", "노드 · 연결 끊김", undefined],
        ].map(([value, label, description]) => (
          <Checkbox
            key={value}
            name="events"
            value={value}
            label={label}
            description={description}
            tile
          />
        ))}
      </div>
    </FieldGroup>
  ),
};

/** 도구 막대처럼 작은 글자 자리: 글자 크기·색은 줄에 붙인 className(여기서는 인라인 스타일)을 따른다. */
export const Toolbar: Story = {
  render: () => (
    <div style={{ fontSize: "var(--fs-sm)", color: "var(--text-2)" }}>
      <Checkbox label="정적·텔레메트리 숨김" defaultChecked />
    </div>
  ),
};

export const Checked: Story = { name: "선택됨", args: { defaultChecked: true } };

export const Focused: Story = {
  name: "키보드 포커스",
  play: async ({ canvasElement }) => {
    await userEvent.tab();
    await expect(within(canvasElement).getByRole("checkbox")).toHaveFocus();
  },
};

export const Tile: Story = {
  name: "선택 타일과 긴 한글",
  render: () => <form style={{ display: "grid", gap: 12 }}><Checkbox tile name="acknowledged" value="yes" label="백업이 완료된 것을 확인했으며 저장된 설정을 변경할 때 기존 연결에 영향을 줄 수 있다는 사실을 이해했습니다." description="상자나 설명을 눌러 선택하고 키보드 Space로 선택을 해제할 수 있습니다." /><Checkbox tile disabled label="변경할 권한이 없는 설정" description="이 설정은 관리자만 변경할 수 있습니다." /></form>,
  play: async ({ canvasElement }) => {
    const boxes = within(canvasElement).getAllByRole<HTMLInputElement>("checkbox");
    await userEvent.click(within(canvasElement).getByText("백업이 완료된 것을 확인했으며 저장된 설정을 변경할 때 기존 연결에 영향을 줄 수 있다는 사실을 이해했습니다."));
    await expect(boxes[0]!).toBeChecked();
    await expect(new FormData(boxes[0]!.form!).get("acknowledged")).toBe("yes");
    boxes[0]!.focus();
    await userEvent.keyboard(" ");
    await expect(boxes[0]!).not.toBeChecked();
    await expect(new FormData(boxes[0]!.form!).has("acknowledged")).toBe(false);
    await expect(boxes[1]!).toBeDisabled();
    await expect(boxes[0]!).toHaveAccessibleDescription("상자나 설명을 눌러 선택하고 키보드 Space로 선택을 해제할 수 있습니다.");
  },
};

export const RadioFoundation: Story = {
  name: "라디오 기본 상태",
  render: () => <fieldset style={{ margin: 0, minWidth: 0, border: 0, padding: 0, display: "grid", gap: 16 }}><legend>인증서 갱신 알림 방법</legend>
    <label><input type="radio" name="notification" value="email" defaultChecked /> 이메일로 알림 받기</label>
    <label><input type="radio" name="notification" value="webhook" /> 운영팀과 고객지원팀이 사용하는 알림 수신 주소로 전달하기</label>
    <label style={{ color: "var(--text-disabled)" }}><input type="radio" name="notification" value="disabled" disabled /> 사용할 수 없는 알림 방법</label>
  </fieldset>,
  play: async ({ canvasElement }) => {
    const radios = within(canvasElement).getAllByRole("radio");
    await expect(radios[0]!).toBeChecked();
    await userEvent.tab();
    await expect(radios[0]!).toHaveFocus();
    await userEvent.keyboard("{ArrowDown}");
    await expect(radios[1]!).toBeChecked();
    await expect(radios[1]!).toHaveFocus();
    await expect(radios[2]!).toBeDisabled();
  },
};

export const Indeterminate: Story = { name: "일부 선택", args: { label: "일부 항목 선택", ref: input => { if (input) input.indeterminate = true; } } };
