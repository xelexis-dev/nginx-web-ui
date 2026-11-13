import { Icon, PageHeader, TextLink, linkClassName } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";

const meta = {
  title: "UI/TextLink",
  component: TextLink,
  parameters: {
    docs: {
      subtitle: "문장 안의 링크와 상위로 돌아가는 링크",
      description: {
        component: [
          "`TextLink`는 네이티브 `<a>`다. `href` 등 링크 속성을 그대로 넘긴다. 라우터 링크(웹은 `next/link`)에는 같은 모양의 `linkClassName()`을 입힌다.",
          "",
          "- **variant**: `inline`(기본, 항상 밑줄과 주변 크기·굵기를 유지하는 accent-text 링크) · `back`(제목 위의 돌아가기 링크: --fs-sm/500, hover 밑줄).",
          "- inline은 문단뿐 아니라 Banner·CodePane 같은 설명 안에서도 항상 밑줄과 주변 크기·굵기를 유지한다. back은 마우스를 올리면 밑줄이 생긴다. 키보드로 오면 2px 포커스 링이 2px 떨어져 보인다.",
          "- 다른 화면으로 가는 **버튼 모양**의 동작은 `buttonClassName`을 쓴다(UI/Button의 「링크」).",
        ].join("\n"),
      },
    },
  },
  args: { href: "#metrics", children: "접속 지표" },
  argTypes: {
    variant: {
      control: "inline-radio",
      options: ["inline", "back"],
      description: "모양. inline은 문장 안, back은 제목 위의 돌아가기",
      table: {
        type: { summary: '"inline" | "back"' },
        defaultValue: { summary: '"inline"' },
      },
    },
    children: {
      control: "text",
      description: "링크 문구",
      table: { type: { summary: "ReactNode" } },
    },
    href: { control: "text", description: "이동할 주소" },
    className: {
      control: false,
      description: "기본 모양 뒤에 붙는다",
      table: { type: { summary: "string" } },
    },
  },
} satisfies Meta<typeof TextLink>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 문장 안의 링크. */
export const Inline: Story = {
  name: "문장 안",
  render: (args) => (
    <p
      style={{
        margin: 0,
        color: "var(--text-2)",
        fontSize: "var(--fs-md)",
      }}
    >
      서버별 요청 수와 오류율은 <TextLink {...args} />
      에서 봅니다.
    </p>
  ),
  play: async ({ canvasElement }) => {
    const link = within(canvasElement).getByRole("link", { name: "접속 지표" });
    await userEvent.tab();
    await expect(link).toHaveFocus();
    await expect(link).toHaveAttribute("href", "#metrics");
  },
};

/** 제목 위의 돌아가기 링크. 아이콘은 장식이라 이름은 문구다. */
export const Back: Story = {
  name: "돌아가기",
  args: { variant: "back", href: "#list", children: "리버스 프록시" },
  render: (args) => (
    <PageHeader
      eyebrow={
        <TextLink {...args}>
          <Icon name="back" size={14} />
          {args.children}
        </TextLink>
      }
      title="api.example.test"
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const link = canvas.getByRole("link", { name: /리버스 프록시/ });
    await userEvent.tab();
    await expect(link).toHaveFocus();
    await expect(link.querySelector("svg")).toHaveAttribute(
      "aria-hidden",
      "true",
    );
    await expect(canvas.getByRole("heading", { level: 1 })).toHaveTextContent(
      "api.example.test",
    );
  },
};

/** 라우터 링크처럼 다른 컴포넌트에는 `linkClassName()`을 입힌다. 모습은 TextLink와 같다. */
export const WithClassName: Story = {
  name: "linkClassName",
  parameters: { controls: { disable: true } },
  render: () => (
    <div
      style={{ display: "grid", gap: "var(--sp-3)", justifyItems: "start" }}
    >
      <a href="#list" className={linkClassName({ variant: "back" })}>
        <Icon name="arrowLeft" size={14} />
        인증서
      </a>
      <p
        style={{
          margin: 0,
          color: "var(--text-2)",
          fontSize: "var(--fs-md)",
        }}
      >
        만료 30일 전부터{" "}
        <a href="#renew" className={linkClassName()}>
          자동 갱신
        </a>
        을 시도합니다.
      </p>
    </div>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    for (const name of [/인증서/, "자동 갱신"]) {
      await userEvent.tab();
      await expect(canvas.getByRole("link", { name })).toHaveFocus();
    }
  },
};

function LinkStates() {
  return <main style={{ maxWidth: 1200, marginInline: "auto" }}>
    <h1>텍스트 링크 상태</h1>
    {["default", "hover", "focus"].map((state) => <section key={state} data-action-state={state}>
      <h2>{({ default: "기본", hover: "마우스 올림", focus: "키보드 포커스" })[state]}</h2>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 24, padding: 8 }}>
        <TextLink href="#history">적용 이력</TextLink>
        <TextLink variant="back" href="#list"><Icon name="back" size={14} />리버스 프록시</TextLink>
        <a href="#metrics" className={linkClassName()}>접속 지표</a>
      </div>
    </section>)}
    <p data-running-text>긴 문장 안의 <TextLink href="#long">긴 이름의 리버스 프록시 서버에 연결된 인증서와 설정의 적용 이력을 확인합니다</TextLink></p>
    <p data-running-text>중첩된 강조 문장 안의 <strong><TextLink href="#nested">SMTP 설정</TextLink></strong>을 확인합니다.</p>
    <div data-running-text>문단 태그가 아닌 본문의 <TextLink href="#div-flow">사이트 설정</TextLink>도 구분합니다.</div>
    <ul><li data-running-text>목록 안의 <TextLink href="#list-inline">접속 지표</TextLink>를 확인합니다.</li></ul>
    <dl><dt>알림</dt><dd data-running-text>설명 안의 <TextLink href="#description">메일 설정</TextLink>을 확인합니다.</dd></dl>
  </main>;
}
export const StatesLight: Story = { render: () => <LinkStates />, globals: { theme: "light" } };
export const StatesDark: Story = { render: () => <LinkStates />, globals: { theme: "dark" } };
