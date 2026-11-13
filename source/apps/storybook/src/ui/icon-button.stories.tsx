import { Icon, IconButton } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";

const meta = {
  title: "UI/IconButton",
  component: IconButton,
  args: { label: "알림", children: <Icon name="notifications" />, onClick: fn() },
  parameters: { docs: { description: { component: "공통 Button을 확장한 정사각형 버튼입니다. label은 필수 접근성 이름입니다. md는 32px, sm은 28px, lg는 40px, xs는 24px이며, 색상·hover·눌림·포커스·비활성은 Button과 같습니다. 기본 variant는 md에서 secondary, sm에서 ghost이며 type은 button입니다. 유틸리티 동작은 32px secondary, 행 동작은 28px ghost를 사용하며, 키보드 포커스 링은 유지합니다." } } },
  argTypes: {
    variant: { control: "inline-radio", options: ["primary", "secondary", "danger", "ghost", "dangerQuiet"] },
    size: { control: "inline-radio", options: ["md", "sm", "lg", "xs"] },
    children: { control: false },
  },
} satisfies Meta<typeof IconButton>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const Small: Story = { args: { size: "sm" } };
export const Disabled: Story = { args: { disabled: true } };
export const Ghost: Story = { args: { variant: "ghost", label: "메뉴 닫기", children: <Icon name="close" /> } };

export const Header: Story = { name: "헤더 아이콘", args: { size: "md", variant: "secondary" } };
export const HeaderOpen: Story = { name: "헤더 아이콘 · 열림", args: { size: "md", variant: "secondary", "aria-expanded": true, "aria-haspopup": "menu" }, parameters: { docs: { description: { story: "메뉴가 열린 트리거의 외형 예시입니다. 실제 메뉴 동작은 Product/Header에서 확인합니다." } } } };
export const Pressed: Story = { name: "토글 · 켜짐", args: { variant: "ghost", label: "자동 새로고침", children: <Icon name="refresh" />, "aria-pressed": true }, parameters: { docs: { description: { story: "켜진 토글은 aria-pressed로 알리고, 메뉴가 열린 트리거와 같은 누른 면을 유지합니다." } } } };

function IconStates() {
  return <main style={{ maxWidth: 1200, marginInline: "auto" }}>
    <h1>아이콘 버튼 상태</h1>
    {["default", "hover", "focus", "disabled"].map((state) => <section key={state} data-action-state={state}>
      <h2>{({ default: "기본", hover: "마우스 올림", focus: "키보드 포커스", disabled: "비활성" })[state]}</h2>
      <div style={{ display: "flex", gap: 24, padding: 8 }}>
        <IconButton label="유틸리티 설정 새로고침" ref={(el) => { if (el) el.dataset.refAttached = "true"; }} data-size="md" disabled={state === "disabled"}><Icon name="refresh" /></IconButton>
        <IconButton label="긴 이름의 리버스 프록시 서버 설정을 편집합니다" data-size="sm" size="sm" disabled={state === "disabled"}><Icon name="settings" /></IconButton>
      </div>
    </section>)}
  </main>;
}
export const StatesLight: Story = { render: () => <IconStates />, globals: { theme: "light" } };
export const StatesDark: Story = { render: () => <IconStates />, globals: { theme: "dark" } };

export const ExtraSmall: Story = { args: { size: "xs", label: "추가", children: <Icon name="plus" size={14} /> } };
