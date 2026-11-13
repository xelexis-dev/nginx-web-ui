import { Button, Icon, buttonClassName, type ButtonVariant } from "@repo/ui";
import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, within } from "storybook/test";

const variants: ButtonVariant[] = ["primary", "secondary", "danger", "ghost", "dangerQuiet"];

const meta = {
  title: "UI/Button",
  component: Button,
  parameters: {
    docs: {
      subtitle:
        "동작을 실행하는 버튼. 한 영역에 채운 버튼(primary)은 하나만 둔다",
      description: {
        component: [
          "`@repo/ui`의 `Button`은 네이티브 `<button>`이다. 네이티브 속성(`type`, `disabled`, `onClick`, `aria-*` 등)을 그대로 넘긴다.",
          "",
          "- **variant**: `primary`(기본, 강조색 면에 테마별 대비 글자) · `secondary`(차분한 중립 면과 옅은 테두리) · `danger`(되돌릴 수 없는 동작, 채운 오류색 면에 대비 글자) · `ghost`(테두리 없는, 표·도구 막대의 가벼운 동작) · `dangerQuiet`(화면의 조용한 삭제 동작).",
          "- **size**: `md`(폼·페이지, `--size-ctl` 32px, 터치도 동일) · `sm`(표·인라인, `--size-ctl-sm` 28px) · `lg`(인증 화면 40px) · `xs`(24px).",
          "- 일반 작은 버튼은 `sm`(28px)을 사용한다. 아이콘만 있는 동작은 같은 Button을 사용하는 **UI/IconButton**으로 만들며 `label`을 필수로 지정한다.",
          "- 마우스를 올리면 면이 바뀌고 누르면 면이 바뀐다. 키보드로 오면 2px 포커스 링이 2px 떨어져 보인다. 비활성은 전용 배경과 글자색을 쓴다.",
          '- ghost는 메뉴가 열린 트리거(`aria-expanded="true"`)와 켜진 토글(`aria-pressed="true"`)에서 누른 면을 유지한다.',
          "- 기본 규칙은 특이도가 0이라, 넘긴 `className`이 항상 이기고 `style`은 그 위에서 이긴다.",
          "- **폼 안에서는 `type`을 적는다.** 네이티브 버튼의 기본값은 `submit`이다.",
          "- 처리 중에는 `disabled`로 막고 문구로 진행 상태를 알린다(예: 「저장 중…」). 문구는 무엇이 일어나는지 동사로 쓴다.",
        ].join("\n"),
      },
    },
  },
  args: { children: "저장", onClick: fn() },
  argTypes: {
    children: {
      control: "text",
      description: "버튼 문구",
      table: { type: { summary: "ReactNode" } },
    },
    variant: {
      control: "inline-radio",
      options: variants,
      description: "시각적 역할",
      table: {
        type: { summary: '"primary" | "secondary" | "danger" | "ghost" | "dangerQuiet"' },
        defaultValue: { summary: '"primary"' },
      },
    },
    size: {
      control: "inline-radio",
      options: ["md", "sm", "lg", "xs"],
      description: "높이. md는 폼·페이지, sm은 표·인라인",
      table: {
        type: { summary: '"md" | "sm" | "lg" | "xs"' },
        defaultValue: { summary: '"md"' },
      },
    },
    type: {
      control: "inline-radio",
      options: ["button", "submit", "reset"],
      description: "네이티브 버튼 종류. 생략하면 `submit`",
      table: {
        type: { summary: '"button" | "submit" | "reset"' },
        defaultValue: { summary: '"submit"' },
      },
    },
    disabled: {
      control: "boolean",
      description: "눌리지 않게 막는다",
      table: { type: { summary: "boolean" } },
    },
    onClick: {
      description: "클릭 시 호출",
      table: { type: { summary: "(event) => void" } },
    },
    className: {
      control: false,
      description: "기본 모양보다 항상 우선한다",
      table: { type: { summary: "string" } },
    },
    style: {
      control: false,
      description: "인라인 스타일(className 위에서 이긴다)",
      table: { type: { summary: "CSSProperties" } },
    },
  },
} satisfies Meta<typeof Button>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 기본 모양. 한 영역의 주요 동작에 쓴다. */
export const Primary: Story = {};

/** 취소, 보조 동작. 차분한 중립 면과 옅은 테두리로 칩·패널과 한 계열로 보인다. */
export const Secondary: Story = {
  args: { variant: "secondary", children: "취소" },
};

/** 삭제처럼 되돌릴 수 없는 동작. 색만으로 뜻을 전하지 않게 문구에 동작을 쓴다. */
export const Danger: Story = {
  args: { variant: "danger", children: "서버 삭제" },
};

/** 표의 행 동작, 도구 막대처럼 내용과 경쟁하지 않아야 하는 동작. */
export const Ghost: Story = {
  args: { variant: "ghost", children: "편집" },
};

/** 표·인라인에 쓰는 작은 크기. */
export const Small: Story = {
  args: { variant: "secondary", size: "sm", children: "복사" },
};

/** 모든 variant의 활성·비활성 모습. 테마 도구 막대로 라이트와 다크를 비교한다. */
export const AllVariants: Story = {
  parameters: { controls: { disable: true } },
  render: (args) => (
    <div style={{ display: "grid", gap: "var(--sp-3)" }}>
      {[false, true].map((disabled) => (
        <div
          key={String(disabled)}
          style={{
            display: "flex",
            flexWrap: "wrap",
            alignItems: "center",
            gap: "var(--sp-3)",
          }}
        >
          {variants.map((variant) => (
            <Button
              key={variant}
              {...args}
              variant={variant}
              disabled={disabled}
            >
              {variant}
            </Button>
          ))}
          {variants.map((variant) => (
            <Button
              key={`${variant}-sm`}
              {...args}
              variant={variant}
              size="sm"
              disabled={disabled}
            >
              {variant} sm
            </Button>
          ))}
        </div>
      ))}
    </div>
  ),
};

/** ghost의 상태: 기본, 켜진 토글(`aria-pressed`), 메뉴가 열린 트리거(`aria-expanded`). 뒤의 둘은 누른 면을 유지한다. */
export const GhostStates: Story = {
  name: "Ghost 상태",
  parameters: { controls: { disable: true } },
  render: (args) => (
    <div
      style={{
        display: "flex",
        flexWrap: "wrap",
        alignItems: "center",
        gap: "var(--sp-3)",
      }}
    >
      <Button {...args} type="button" variant="ghost">
        기본
      </Button>
      <Button {...args} type="button" variant="ghost" aria-pressed>
        켜짐
      </Button>
      <Button
        {...args}
        type="button"
        variant="ghost"
        aria-haspopup="menu"
        aria-expanded
      >
        메뉴 열림
      </Button>
    </div>
  ),
};

/** 지금은 실행할 수 없는 동작. 이유는 버튼 주변 문구로 알린다. */
export const Disabled: Story = {
  args: { disabled: true },
};

/** 긴 문구는 좁은 화면에서 여러 줄로 감긴다. 버튼 문구는 짧게 쓴다. */
export const LongLabel: Story = {
  args: { children: "변경 사항을 저장하고 nginx 설정을 다시 적용" },
};

/** 요청을 처리하는 동안: 비활성화하고 진행 문구를 보여 중복 제출을 막는다. */
export const Busy: Story = {
  args: { children: <><Icon name="loaderCircle" />저장 중…</>, "aria-busy": true, disabled: true },
};

/**
 * 다른 화면으로 이동하는 동작은 `<a>`(웹은 `next/link`)에 `buttonClassName`을 입힌다. 모습은 같은 variant의 버튼과 같고
 * 역할은 링크로 남는다. Tab으로 순서대로 들어온다.
 */
export const AsLink: Story = {
  name: "링크 (buttonClassName)",
  parameters: { controls: { disable: true } },
  render: () => (
    <div
      style={{
        display: "flex",
        flexWrap: "wrap",
        alignItems: "center",
        gap: "var(--sp-3)",
      }}
    >
      <a href="#new" className={buttonClassName()}>
        새 서버
      </a>
      <a href="#edit" className={buttonClassName({ variant: "secondary" })}>
        수정
      </a>
      <a href="#new-token" className={buttonClassName({ size: "sm" })}>
        새 토큰
      </a>
      <a
        href="#list"
        className={buttonClassName({ variant: "secondary", size: "sm" })}
      >
        목록으로
      </a>
    </div>
  ),
  play: async ({ canvasElement }) => {
    const links = within(canvasElement).getAllByRole("link");
    await expect(links).toHaveLength(4);
    for (const link of links) {
      await userEvent.tab();
      await expect(link).toHaveFocus();
      await expect(link).toHaveAccessibleName();
    }
  },
};

/** 아이콘을 앞에 둔 버튼. 아이콘은 장식(`aria-hidden`)이라 버튼 이름은 문구 그대로다. */
export const WithIcon: Story = {
  name: "아이콘과 문구",
  parameters: { controls: { disable: true } },
  render: (args) => (
    <div
      style={{
        display: "flex",
        flexWrap: "wrap",
        alignItems: "center",
        gap: "var(--sp-3)",
      }}
    >
      <Button {...args} type="button" variant="secondary">
        <Icon name="refresh" />
        상태 새로고침
      </Button>
      <Button {...args} type="button" size="sm">
        <Icon name="plus" size={14} />
        헤더 추가
      </Button>
    </div>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    for (const name of ["상태 새로고침", "헤더 추가"]) {
      const button = canvas.getByRole("button", { name });
      const icon = button.querySelector("svg");
      await expect(icon).not.toBeNull();
      await expect(icon).toHaveAttribute("aria-hidden", "true");
    }
  },
};

/** Browser qualification drives real hover/active/focus on each named state row. */
function ActionStates() {
  return <main style={{ maxWidth: 1200, marginInline: "auto" }}>
    <h1>버튼 상태</h1>
    {["default", "hover", "active", "focus", "disabled", "busy"].map((state) => <section key={state} data-action-state={state}>
      <h2>{({ default: "기본", hover: "마우스 올림", active: "누름", focus: "키보드 포커스", disabled: "비활성", busy: "처리 중" })[state]}</h2>
      <div style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: 16, padding: 8 }}>
        {variants.map((variant) => <Button key={variant} data-variant={variant} type="button" variant={variant} aria-busy={state === "busy" || undefined} disabled={state === "disabled" || state === "busy"}>{state === "busy" ? "저장 중…" : variant === "danger" ? "서버 삭제" : variant === "primary" ? "설정 적용" : variant === "secondary" ? "미리보기" : "편집"}</Button>)}
      </div>
    </section>)}
    <h2>작은 크기·링크·긴 문구</h2>
    <div style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: 16, padding: 8 }}>
      <Button type="button" size="sm" ref={(el) => { if (el) el.dataset.refAttached = "true"; }}>설정 복사</Button>
      <a href="#new" className={buttonClassName({ variant: "secondary", size: "sm" })}>새 서버</a>
      <Button type="button">변경 사항을 저장하고 리버스 프록시의 nginx 설정을 다시 적용합니다</Button>
    </div>
  </main>;
}
export const StatesLight: Story = { render: () => <ActionStates />, globals: { theme: "light" } };
export const StatesDark: Story = { render: () => <ActionStates />, globals: { theme: "dark" } };

export const Large: Story = { args: { size: "lg", children: "로그인" } };
export const ExtraSmall: Story = { args: { size: "xs", variant: "secondary", children: "복사" } };
export const DangerQuiet: Story = { args: { variant: "dangerQuiet", children: "서버 삭제" } };

function LoadingWidth() {
  const [busy, setBusy] = useState(false);
  return <main style={{ display: "flex", flexWrap: "wrap", gap: "var(--gap-actions)", fontFamily: "var(--font)" }}>
    <Button type="button" aria-busy={busy || undefined} disabled={busy} onClick={() => setBusy(true)}>
      <Icon name={busy ? "loaderCircle" : "check"} />
      <span style={{ display: "grid" }}>
        <span aria-hidden="true" style={{ gridArea: "1 / 1", visibility: "hidden" }}>변경 사항 저장</span>
        <span style={{ gridArea: "1 / 1" }}>{busy ? "저장 중…" : "변경 사항 저장"}</span>
      </span>
    </Button>
    <Button type="button" variant="secondary" onClick={() => setBusy(false)}>작업 완료</Button>
  </main>;
}
export const LoadingStableWidth: Story = { render: () => <LoadingWidth /> };
