import { KpiRow } from "../../../../packages/ui/src/data";
import { Icon } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect } from "storybook/test";

const meta = {
  title: "UI/KpiRow",
  component: KpiRow,
  decorators: [
    (Story) => (
      <main style={{ maxWidth: 1200, width: "100%", margin: "0 auto" }}>
        <h1>주요 지표</h1>
        <Story />
      </main>
    ),
  ],
  parameters: {
    docs: {
      subtitle: "가로 구분선으로 연결한 핵심 지표",
      description: {
        component: [
          "위아래 가로 구분선과 셀 사이 세로 구분선으로 연결된 지표 행. 첫 셀은 왼쪽에 맞춘다.",
          "값은 24/32, 500, tabular-nums, 설명은 12/16. 긴 값은 셀 폭에 맞춰 줄어든다.",
          "769–1023px에서는 3열, 768px 이하에서는 1열. 기존 columns·icon·tone·note props를 유지한다.",
        ].join("\n"),
      },
    },
  },
  args: {
    label: "nginx 실시간",
    items: [
      { label: "활성 연결", value: "1,284" },
      { label: "읽기", value: "3" },
      { label: "쓰기", value: "41" },
      { label: "대기 (keep-alive)", value: "1,240" },
      { label: "누적 요청", value: "18,420,771" },
      { label: "드롭된 연결", value: "0" },
    ],
  },
} satisfies Meta<typeof KpiRow>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

/** 서버 상세의 트래픽: 기준을 넘은 5xx를 오류색으로. */
export const WithTone: Story = {
  args: {
    label: "트래픽 · 최근 1시간",
    items: [
      { label: "요청", value: "758,400" },
      { label: "5xx", value: "2.8%", tone: "error" },
    ],
  },
};

/** 다섯 상태색과 기본(tone 없음). 값의 색만 바뀌고 카드·라벨은 같다(neutral은 기본과 같은 `--text-1`). 테마 도구 막대로 라이트와 다크를 비교한다. */
export const AllTones: Story = {
  name: "모든 상태색",
  args: {
    label: "상태색 비교",
    items: [
      { label: "기본", value: "1,284" },
      { label: "정상", value: "99.9%", tone: "success" },
      { label: "주의", value: "82%", tone: "warning" },
      { label: "오류", value: "2.8%", tone: "error" },
      { label: "안내", value: "3", tone: "info" },
      { label: "중립", value: "0", tone: "neutral" },
    ],
  },
};

/** 개요의 nginx 실시간: 값 아래에 설명(`note`)을 단다. */
export const WithNote: Story = {
  name: "설명 붙임",
  args: {
    columns: 4,
    items: [
      { label: "활성 연결", value: "1,284", note: "현재 연결된 클라이언트" },
      {
        label: "누적 요청",
        value: "18,420,771",
        note: "nginx 시작 이후 처리한 요청",
      },
      { label: "대기(keep-alive)", value: "1,240", note: "읽기 3 · 쓰기 41" },
      {
        label: "드롭된 연결",
        value: "0",
        note: "수락한 연결 중 처리되지 않은 연결",
      },
    ],
  },
};

const sixItems = [
  { label: "활성 연결", value: "1,284" },
  { label: "읽기", value: "3" },
  { label: "쓰기", value: "41" },
  { label: "대기 (keep-alive)", value: "1,240" },
  { label: "누적 요청", value: "18,420,771" },
  { label: "드롭된 연결", value: "0" },
];

/** 2열 고정. 768px 이하에서 1열. */
export const Columns2: Story = {
  name: "2열",
  args: { columns: 2, items: sixItems },
};

/** 3열 고정. 768px 이하에서 1열. */
export const Columns3: Story = {
  name: "3열",
  args: { columns: 3, items: sixItems },
};

/** 4열 고정. 1180px 이하에서 2열, 768px 이하에서 1열. */
export const Columns4: Story = {
  name: "4열",
  args: { columns: 4, items: sixItems.slice(0, 4) },
};

/** 320px 폭의 긴 값: 줄을 바꾸지 않고 글자가 작아져 카드 밖으로 넘치지 않는다. */
export const LongValueNarrow: Story = {
  name: "좁은 폭 · 긴 값",
  args: {
    label: "누적 처리량",
    items: [
      { label: "누적 요청", value: "18,420,771,204" },
      { label: "전송량", value: "1,284,630.5 GB", tone: "info" },
    ],
  },
  render: (args) => (
    <div style={{ width: "100%" }}>
      <KpiRow {...args} />
    </div>
  ),
  play: async ({ canvasElement }) => {
    const row = canvasElement.querySelector("dl");
    await expect(row).not.toBeNull();
    await expect(row!.scrollWidth).toBeLessThanOrEqual(row!.clientWidth);
    const values = [...row!.querySelectorAll("dd")];
    await expect(values).toHaveLength(2);
    for (const value of values)
      await expect(value.scrollWidth).toBeLessThanOrEqual(value.clientWidth);
  },
};

export const WithIcons: Story = {
  name: "아이콘과 알 수 없는 값",
  args: {
    columns: 4,
    label: "운영 지표",
    items: [
      {
        label: "연결",
        value: "1,284",
        icon: <Icon name="servers" />,
        note: "현재 연결된 클라이언트",
      },
      {
        label: "응답 오류",
        value: "2.8%",
        tone: "error",
        icon: <Icon name="alert" />,
      },
      {
        label: "처리량",
        value: "알 수 없음",
        icon: <Icon name="metrics" />,
        note: "서버 응답을 기다리고 있습니다",
      },
      { label: "마지막 확인", value: "—", icon: <Icon name="pending" /> },
    ],
  },
  play: async ({ canvasElement }) => {
    await expect(
      canvasElement.querySelectorAll("dt svg[aria-hidden=true]"),
    ).toHaveLength(4);
    await expect(canvasElement.querySelectorAll("dt")).toHaveLength(4);
    await expect(canvasElement.querySelectorAll("dd")).toHaveLength(4);
  },
};
export const LongKorean: Story = {
  name: "긴 한국어 지표와 설명",
  args: {
    label: "긴 운영 지표",
    items: [
      {
        label: "클라이언트 응답을 기다리는 연결 수와 현재 처리 현황",
        value: "18,420,771,204",
        note: "선택한 서버에서 마지막으로 확인한 값입니다. 연결이 복구되면 다시 확인합니다.",
        icon: <Icon name="metrics" />,
      },
      {
        label: "최근 요청을 정상적으로 처리했는지 확인할 수 없는 상태",
        value: "알 수 없음",
        note: "측정된 값이 없으며 임의의 수치로 대신하지 않습니다.",
      },
    ],
  },
};
