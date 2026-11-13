import { CodePane } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";

const sample = `server {
    listen 80;
    server_name api.example.test;
    return 301 https://$host$request_uri;
}

server {
    listen 443 ssl;
    http2 on;
    server_name api.example.test;

    location /v2/ {
        proxy_pass http://api_v2;
        proxy_set_header Host $host;
    }
}`;

const meta = {
  title: "UI/CodePane",
  component: CodePane,
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
        <h1>코드 미리보기</h1>
        <Story />
      </main>
    ),
  ],
  parameters: {
    docs: {
      subtitle: "생성되는 설정 같은 원문과 복사 버튼",
      description: {
        component: [
          "`CodePane`은 제목 줄과 두 테마에서 sunken 면을 쓰는 코드 영역(`--bg-sunken`)이다. 긴 원문은 줄을 바꾸지 않고 코드 영역 안에서 가로로 스크롤하며 숫자는 같은 폭이다.",
          "",
          "- `error`: 원문 대신 오류를 보인다(미리보기를 만들 수 없을 때).",
          "- `copyLabel={false}`: 복사 버튼 없음. 클립보드를 쓸 수 없으면 「복사할 수 없음」이라고 알린다.",
          "- `sticky`: 1024px 이상에서 옆 내용과 함께 스크롤해도 화면에 남는다.",
        ].join("\n"),
      },
    },
  },
  args: {
    title: "nginx 설정",
    description: "적용 시 기록되는 내용",
    code: sample,
  },
} satisfies Meta<typeof CodePane>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const PreviewError: Story = {
  args: {
    code: undefined,
    error: "미리보기를 만들 수 없습니다: upstream api_v2 가 없습니다.",
  },
};

export const Long: Story = {
  args: {
    description: "긴 설정 줄은 코드 영역 안에서 가로로 스크롤합니다.",
    code: sample + "\n# " + "아주긴한국어설정이름".repeat(35),
  },
};
export const WithoutCopy: Story = { args: { copyLabel: false } };
