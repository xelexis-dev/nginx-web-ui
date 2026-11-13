import { Icon, iconNames } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";

const meta = {
  title: "UI/Icon",
  component: Icon,
  parameters: {
    docs: {
      subtitle: "메뉴와 버튼의 작은 선 아이콘. 의존성 없는 인라인 SVG",
      description: {
        component: [
          "`Icon`은 언제나 장식이다(`aria-hidden`). 아이콘을 쓰는 곳에는 보이는 글자나 `aria-label`이 함께 있어야 한다.",
          "",
          "- 색은 글자색(`currentColor`)을 따른다. 크기는 12(체크 전용) · 14 · 16(기본) · 20. 기존 18px 호출도 유지한다.",
          "- 메뉴 항목마다 하나: 개요, 서버, Upstreams, Stream, 인증서, 로그, 지표, 감사, 이력, 노드, 계정, 설정과 관리자 메뉴(사용자, 알림, OAuth, 보안, 토큰, 승인 대기, 비밀값).",
        ].join("\n"),
      },
    },
  },
  args: { name: "servers", size: 16 },
  argTypes: {
    name: { control: "select", options: iconNames },
    size: { control: "inline-radio", options: [12, 14, 16, 18, 20] },
  },
} satisfies Meta<typeof Icon>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Single: Story = {};

/** 모든 아이콘과 이름. */
export const All: Story = {
  render: () => (
    <ul
      style={{
        display: "grid",
        gridTemplateColumns: "repeat(auto-fill, minmax(140px, 1fr))",
        gap: 12,
        margin: 0,
        padding: 0,
        listStyle: "none",
      }}
    >
      {iconNames.map((name) => (
        <li
          key={name}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            color: "var(--text-1)",
            fontSize: "var(--fs-sm)",
          }}
        >
          <Icon name={name} size={20} />
          <code>{name}</code>
        </li>
      ))}
    </ul>
  ),
};

export const Rail18: Story = { args: { size: 18 } };
export const Rail18Dark: Story = { ...Rail18, globals: { theme: "dark" } };

export const Check12: Story = { args: { name: "check", size: 12 } };
export const Stroke14: Story = { args: { name: "circleAlert", size: 14 } };
