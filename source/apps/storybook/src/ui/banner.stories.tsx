import { Banner, Button, type Tone } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { fn } from "storybook/test";

const meta = {
  title: "UI/Banner",
  component: Banner,
  decorators: [
    (Story) => (
      <main
        style={{
          maxWidth: 1200,
          margin: "0 auto",
          width: "100%",
          boxSizing: "border-box",
        }}
      >
        <h1>알림</h1>
        <Story />
      </main>
    ),
  ],
  parameters: {
    docs: {
      subtitle: "페이지 안의 알림. 저장·적용 결과, 경고, 오류",
      description: {
        component: [
          "`Banner`는 상태 틴트 면에 전체 테두리와 톤 아이콘를 둔다. 제목은 기본 글자색, 본문은 보조 글자색. 아이콘과 면만 상태색을 쓴다.",
          "",
          "- `output`: 도구 출력(nginx -t 등). 고정폭, 줄바꿈 유지, 길면 스크롤(키보드로도 스크롤할 수 있다).",
          "- `role`: 오류는 기본 `alert`, 그 외는 `status`. 명시한 role은 그대로 쓴다.",
          "- `onDismiss`가 있으면 닫기 버튼이 생긴다.",
        ].join("\n"),
      },
    },
  },
  args: {
    tone: "success",
    title: "설정을 적용했습니다.",
    children: "nginx -t 검사를 통과했고 nginx를 다시 불러왔습니다.",
  },
  argTypes: {
    tone: {
      control: "inline-radio",
      options: ["success", "warning", "error", "info", "neutral"] as Tone[],
    },
    action: { control: false },
    children: { control: "text" },
  },
} satisfies Meta<typeof Banner>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Success: Story = {};

/** 적용 실패: 출력이 함께 보인다. */
export const ApplyFailed: Story = {
  args: {
    tone: "error",
    role: "alert",
    title: "설정을 적용하지 못했습니다.",
    children:
      "nginx -t 검사에서 오류가 나 nginx는 이전 설정으로 계속 동작합니다.",
    output:
      'nginx: [emerg] host not found in upstream "legacy.internal.example.test:8000" in /etc/nginx/conf.d/srv-legacy.conf:9\nnginx: configuration file /etc/nginx/nginx.conf test failed',
  },
};

/** 동작과 닫기 버튼. */
export const WithActionAndDismiss: Story = {
  args: {
    tone: "info",
    title: "미적용 변경 2건",
    children: "설정 적용을 눌러야 nginx에 반영됩니다.",
    action: (
      <Button size="sm" onClick={fn().mockName("적용")}>
        적용
      </Button>
    ),
    onDismiss: fn().mockName("닫기"),
  },
};

export const Warning: Story = {
  args: {
    tone: "warning",
    title: "설정을 확인하세요.",
    children: "적용하기 전에 연결 상태를 확인하세요.",
  },
};
export const Neutral: Story = {
  args: { tone: "neutral", title: "변경 사항이 없습니다." },
};
export const LongKorean: Story = {
  args: {
    tone: "info",
    title:
      "아주 긴 한국어 제목이 여러 줄로 이어져도 알림의 내용과 동작을 확인할 수 있습니다.",
    children:
      "설정 변경과 관련된 안내가 길어져도 화면 너비 안에서 자연스럽게 줄을 바꿉니다. ".repeat(
        5,
      ),
  },
};
export const CustomIcon: Story = {
  args: { tone: "info", icon: "servers", title: "서버 상태를 확인했습니다." },
};
export const NoIcon: Story = {
  args: { tone: "info", icon: false, title: "아이콘 없이 표시하는 안내" },
};
export const Dismissible: Story = {
  render: function Dismissible(args) {
    const [visible, setVisible] = useState(true);
    return visible ? (
      <Banner {...args} onDismiss={() => setVisible(false)} />
    ) : (
      <Button onClick={() => setVisible(true)}>알림 다시 보기</Button>
    );
  },
};

export const SingleLineInfo: Story = {
  args: {
    tone: "info",
    title: "검증을 마친 설정만 적용할 수 있습니다.",
    children: undefined,
  },
};
