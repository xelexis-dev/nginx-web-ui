import { Button, EmptyState, Surface } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";

const meta = {
  title: "UI/Surface",
  component: Surface,
  parameters: {
    docs: {
      subtitle: "제목이 있는 구역. 위쪽 구분선과 여백",
      description: {
        component: [
          "`Surface`는 상단 구분선과 24px 여백으로 나눈 구역이다. 생성·수정 폼에는 Card를 쓴다.",
          "",
          "- `title`·`description`·`actions`는 머리에, 내용은 머리 아래 16px 간격으로.",
          "- `flush`: 표·목록처럼 가장자리까지 닿는 내용. 본문은 양쪽 페이지 정렬을 유지한다.",
          "- 제목 단계는 `headingLevel`(기본 2).",
        ].join("\n"),
      },
    },
  },
  args: { title: "설정 요약", children: <p style={{ margin: 0 }}>면 안의 내용.</p> },
  argTypes: { children: { control: false }, actions: { control: false } },
} satisfies Meta<typeof Surface>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

/** 본문과 설명 속 평범한 링크(클래스 없는 a, 웹의 next/link): 주변 글자색 + 밑줄. */
export const WithTextLink: Story = {
  args: {
    title: "트래픽",
    description: <>서버별 트래픽은 <a href="#metrics">접속 지표</a>에서 봅니다.</>,
    children: <p style={{ margin: 0 }}>저장된 설정 기준입니다. 자세한 수치는 <a href="#metrics">접속 지표</a>에 있습니다.</p>,
  },
};

/** 머리에 설명과 동작. */
export const WithActions: Story = {
  args: { title: "미적용 변경 2건", description: "저장했지만 nginx에는 아직 반영되지 않았다", actions: <Button size="sm" onClick={fn().mockName("적용")}>적용</Button> },
};

/** 가장자리까지 닿는 내용(표 등). 빈 상태를 함께 보인다. */
export const FlushWithEmptyState: Story = {
  args: {
    title: "서버 상태",
    flush: true,
    children: <EmptyState title="조건에 맞는 서버가 없습니다" description="검색어나 상태 필터를 바꿔 보세요." action={<Button size="sm" variant="secondary" onClick={fn().mockName("필터 초기화")}>필터 초기화</Button>} />,
  },
};

export const WithInfo: Story = {
  args: { info: "이 영역과 설정을 설명합니다. 현재 상태와 이 페이지 동작의 결과는 화면에 그대로 둡니다." },
};
