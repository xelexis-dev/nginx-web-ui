import { Facts } from "../../../../packages/ui/src/data";
import { expect, within } from "storybook/test";
import type { Meta, StoryObj } from "@storybook/react-vite";

const meta = {
  title: "UI/Facts",
  component: Facts,
  decorators: [
    (Story) => (
      <main style={{ maxWidth: 1200, width: "100%", margin: "0 auto" }}>
        <h1>정보 요약</h1>
        <Story />
      </main>
    ),
  ],
  parameters: {
    docs: {
      subtitle: "이름과 값의 목록(상세 페이지의 설정 요약)",
      description: {
        component:
          "가로 구분선이 있는 키/값 행. 라벨 200px, 좁은 화면에서는 한 열로 쌓인다. 기존 columns·strip·wide·mono props 유지.",
      },
    },
  },
  args: {
    items: [
      { term: "도메인", value: "api.example.test", mono: true },
      { term: "포트", value: "80 → 443 (HTTPS, 리디렉트)" },
      { term: "대상", value: "경로 3개로 나눔" },
      { term: "TLS", value: "*.example.test · 61일 후 만료" },
      { term: "접근 제어", value: "모두 허용" },
      { term: "실제 IP", value: "Cloudflare (CF-Connecting-IP)" },
    ],
  },
} satisfies Meta<typeof Facts>;
export default meta;
type Story = StoryObj<typeof meta>;

export const OneColumn: Story = {};
export const TwoColumns: Story = { args: { columns: 2 } };

export const Strip: Story = {
  name: "목록 요약 띠",
  args: {
    layout: "strip",
    items: [
      { term: "서버", value: "3개 활성" },
      { term: "인증서", value: "2개 사용" },
      { term: "마지막 적용", value: "확인 대기" },
    ],
  },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.querySelector("dl")).toHaveAttribute(
      "data-layout",
      "strip",
    );
    await expect(canvasElement.querySelectorAll("dt")).toHaveLength(3);
    await expect(within(canvasElement).getByText("3개 활성")).toBeVisible();
  },
};
export const LongAndMissing: Story = {
  name: "긴 값과 미설정",
  args: {
    columns: 2,
    items: [
      {
        term: "실제 클라이언트 요청을 처리할 서버 주소",
        value:
          "https://very-long-upstream-name.example.test:8443/service/requests/without/shortening",
      },
      { term: "전달할 추가 요청 헤더", value: "미설정" },
      {
        term: "확인 결과",
        value:
          "서버가 아직 응답하지 않아 적용 상태를 확인할 수 없습니다. 잠시 후 다시 확인하세요.",
        wide: true,
      },
    ],
  },
};
export const LongStrip: Story = {
  ...LongAndMissing,
  name: "긴 요약 띠",
  args: { ...LongAndMissing.args, layout: "strip" },
};
