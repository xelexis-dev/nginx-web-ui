import {
  StatusBadge,
  StatusDot,
  type StatusBadgeProps,
  type StatusDotProps,
} from "../../../../packages/ui/src/data";
import type { Tone } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";

const tones: Tone[] = ["success", "warning", "error", "info", "neutral"];
const sizes: NonNullable<StatusBadgeProps["size"]>[] = ["md", "sm"];
const badgeVariants: NonNullable<StatusBadgeProps["variant"]>[] = [
  "soft",
  "outline",
];
const labels: Record<Tone, string> = {
  success: "정상",
  warning: "주의",
  error: "확인 필요",
  info: "적용 중",
  neutral: "비활성",
};

/** Every badge in the canvas shows its text, and a dot (when present) is decorative. */
async function expectBadges(canvasElement: HTMLElement, count: number) {
  const badges = [
    ...canvasElement.querySelectorAll<HTMLElement>("[data-tone]"),
  ];
  await expect(badges).toHaveLength(count);
  for (const badge of badges) {
    await expect(badge.textContent?.trim()).toBe(
      labels[badge.dataset.tone as Tone],
    );
    for (const dot of badge.querySelectorAll("span"))
      await expect(dot).toHaveAttribute("aria-hidden", "true");
  }
}

const meta = {
  title: "UI/StatusBadge",
  component: StatusBadge,
  decorators: [
    (Story) => (
      <main style={{ maxWidth: 1200, width: "100%", margin: "0 auto" }}>
        <h1>상태 표시</h1>
        <Story />
      </main>
    ),
  ],
  parameters: {
    docs: {
      subtitle: "상태 표시. 뜻은 글자가 전하고 색은 거든다",
      description: {
        component:
          "8px 도형과 글자로 상태를 표시한다: 정상 ●, 주의 ◆, 오류 ■, 중립 ○. dot=false는 20px 분류 태그다. 기존 size·variant·pulse props는 유지한다.",
      },
    },
  },
  args: { tone: "success", children: "정상" },
  argTypes: {
    tone: { control: "inline-radio", options: tones },
    children: { control: "text" },
    size: {
      control: "inline-radio",
      options: sizes,
      table: { defaultValue: { summary: '"md"' } },
    },
    variant: {
      control: "inline-radio",
      options: badgeVariants,
      table: { defaultValue: { summary: '"soft"' } },
    },
    dot: { control: "boolean", table: { defaultValue: { summary: "true" } } },
  },
} satisfies Meta<typeof StatusBadge>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

/** 다섯 가지 상태. */
export const AllTones: Story = {
  render: () => (
    <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
      <StatusBadge tone="success">정상</StatusBadge>
      <StatusBadge tone="warning">주의</StatusBadge>
      <StatusBadge tone="error">확인 필요</StatusBadge>
      <StatusBadge tone="info">적용 중</StatusBadge>
      <StatusBadge tone="neutral">비활성</StatusBadge>
    </div>
  ),
  play: async ({ canvasElement }) => expectBadges(canvasElement, 5),
};

/** 점 없이: 표의 좁은 칸처럼 글자만으로 충분한 곳. */
export const WithoutDot: Story = {
  name: "분류 태그",
  args: { dot: false },
  play: async ({ canvasElement }) => {
    const badge = within(canvasElement).getByText("정상");
    await expect(badge.querySelector("span")).toBeNull();
  },
};

/** 크기(md·sm) × 모양(soft·outline) × 다섯 상태. 테마 도구 막대로 라이트와 다크를 비교한다. */
export const SizesAndVariants: Story = {
  name: "크기와 모양",
  parameters: { controls: { disable: true } },
  render: () => (
    <div style={{ display: "grid", gap: "var(--sp-3)" }}>
      {sizes.flatMap((size) =>
        badgeVariants.map((variant) => (
          <div
            key={`${size}-${variant}`}
            style={{
              display: "flex",
              flexWrap: "wrap",
              alignItems: "center",
              gap: 8,
            }}
          >
            {tones.map((tone) => (
              <StatusBadge key={tone} tone={tone} size={size} variant={variant}>
                {labels[tone]}
              </StatusBadge>
            ))}
          </div>
        )),
      )}
    </div>
  ),
  play: async ({ canvasElement }) =>
    expectBadges(
      canvasElement,
      sizes.length * badgeVariants.length * tones.length,
    ),
};

const dotSizes: NonNullable<StatusDotProps["size"]>[] = [7, 8, 6];

/**
 * `StatusDot`: 틴트 면 없이 점만 쓰는 상태 표시(상단 상태 줄, 접속 지표의 상태, 알림 목록). 도형은 장식(`aria-hidden`)이라
 * 이름이 없으므로 **보이는 상태 글자를 반드시 옆에 둔다**. 크기는 8(기본)·7·6, 색은 상태색(neutral은 `--text-2`).
 */
export const Dot: Story = {
  name: "점 (StatusDot)",
  parameters: { controls: { disable: true } },
  render: () => (
    <div style={{ display: "grid", gap: "var(--sp-3)" }}>
      {dotSizes.map((size) => (
        <div
          key={size}
          style={{
            display: "flex",
            flexWrap: "wrap",
            alignItems: "center",
            gap: "var(--sp-4)",
          }}
        >
          {tones.map((tone) => (
            <span
              key={tone}
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: "var(--sp-2)",
                color: "var(--text-1)",
                fontSize: "var(--fs-sm)",
              }}
            >
              <StatusDot tone={tone} size={size} />
              {labels[tone]}
            </span>
          ))}
        </div>
      ))}
    </div>
  ),
  play: async ({ canvasElement }) => {
    const dots = [
      ...canvasElement.querySelectorAll<HTMLElement>("[data-tone]"),
    ];
    await expect(dots).toHaveLength(dotSizes.length * tones.length);
    for (const dot of dots) {
      await expect(dot).toHaveAttribute("aria-hidden", "true");
      await expect(dot.parentElement?.textContent?.trim()).toBe(
        labels[dot.dataset.tone as Tone],
      );
    }
  },
};

/** 연결된 실시간 로그만 pulse를 사용한다. 기존 상태 표시는 정적이다. */
export const ConnectedLiveLog: Story = {
  name: "연결된 실시간 로그",
  render: () => (
    <div style={{ display: "flex", flexWrap: "wrap", gap: 16 }}>
      <StatusBadge tone="success" pulse>
        실시간 로그 연결됨
      </StatusBadge>
      <span>
        <StatusDot tone="success" pulse /> 실시간 로그 연결됨
      </span>
      <StatusBadge tone="neutral">연결 대기</StatusBadge>
    </div>
  ),
  play: async ({ canvasElement }) => {
    await expect(canvasElement.querySelectorAll("[data-pulse]")).toHaveLength(
      2,
    );
    await expect(
      within(canvasElement).getByText("연결 대기"),
    ).not.toHaveAttribute("data-pulse");
  },
};
export const LongKorean: Story = {
  name: "긴 한국어 상태",
  args: {
    tone: "warning",
    children:
      "저장된 설정을 서버에 적용하고 응답을 확인하고 있습니다. 잠시 기다려 주세요.",
  },
};
