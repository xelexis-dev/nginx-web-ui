import {
  StatusBadge,
  Icon,
  Button,
  PageHeader,
  buttonClassName,
} from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, within } from "storybook/test";

const meta = {
  title: "UI/PageHeader",
  component: PageHeader,
  parameters: {
    docs: {
      subtitle: "페이지 제목과 그 페이지의 주요 동작",
      description: {
        component: [
          "`PageHeader`는 페이지마다 하나의 `<h1>`을 그린다. 제목은 `--fs-2xl`(넓은 화면 24px, 좁은 화면 20px).",
          "",
          "- `eyebrow`: 제목 위의 돌아가기 링크나 위치. 링크는 호출하는 쪽이 만든다(웹은 `next/link`).",
          "- `description`: 한 줄 요약(개수, 기준 시각 등).",
          "- `actions`: 버튼이나 버튼 모양 링크(`buttonClassName`). 좁은 화면에서는 제목 아래로 내려간다.",
        ].join("\n"),
      },
    },
  },
  args: {
    title: "리버스 프록시",
    description: "서버 13개 · 활성 11 · 미적용 2",
    actions: (
      <a href="#new" className={buttonClassName({ size: "sm" })}>
        새 서버
      </a>
    ),
  },
  argTypes: {
    eyebrow: { control: false },
    actions: { control: false },
    title: { control: "text" },
    description: { control: "text" },
  },
} satisfies Meta<typeof PageHeader>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 목록 페이지. 「새 서버」는 이동이라 버튼 모양의 링크다. */
export const List: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("heading", { level: 1 })).toHaveTextContent(
      "리버스 프록시",
    );
    await expect(canvas.getByRole("link", { name: "새 서버" })).toBeVisible();
  },
};

/** 제목만: 위치 링크·요약·동작이 없는 페이지. */
export const TitleOnly: Story = {
  name: "제목만",
  args: { description: undefined, actions: undefined },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getAllByRole("heading")).toHaveLength(1);
    await expect(canvas.getByRole("heading", { level: 1 })).toHaveTextContent(
      "리버스 프록시",
    );
    await expect(canvas.queryByRole("link")).toBeNull();
  },
};

/** 요약 없이 동작만: 제목과 동작이 한 줄에 놓인다. */
export const WithActionsOnly: Story = {
  name: "제목과 동작",
  args: {
    description: undefined,
    actions: (
      <>
        <Button
          size="sm"
          variant="secondary"
          type="button"
          onClick={fn().mockName("새로고침")}
        >
          새로고침
        </Button>
        <a href="#new" className={buttonClassName({ size: "sm" })}>
          새 서버
        </a>
      </>
    ),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("heading", { level: 1 })).toHaveTextContent(
      "리버스 프록시",
    );
    await expect(
      canvas.getByRole("button", { name: "새로고침" }),
    ).toBeVisible();
    await expect(canvas.getByRole("link", { name: "새 서버" })).toBeVisible();
  },
};

/** 상세 페이지: 돌아가기 링크, 상태, 여러 동작. */
export const Detail: Story = {
  args: {
    eyebrow: (
      <a href="#list">
        <Icon name="arrowLeft" size={14} /> 리버스 프록시
      </a>
    ),
    title: "api.example.test",
    description: "주의 · 5xx 2.8%",
    actions: (
      <>
        <Button size="sm" variant="dangerOutline" onClick={fn().mockName("삭제")}>
          삭제
        </Button>
        <a href="#edit" className={buttonClassName({ size: "sm" })}>
          수정
        </a>
      </>
    ),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getAllByRole("heading")).toHaveLength(1);
    await expect(canvas.getByRole("heading", { level: 1 })).toHaveTextContent(
      "api.example.test",
    );
    await expect(
      canvas.getByRole("link", { name: "리버스 프록시" }),
    ).toBeVisible();
  },
};

export const WithInfo: Story = {
  args: {
    info: "이 영역과 설정을 설명합니다. 현재 상태와 이 페이지 동작의 결과는 화면에 그대로 둡니다.",
  },
};

export const Status: Story = {
  args: {
    title: "운영 서버",
    descriptionVariant: "meta",
    status: <StatusBadge tone="success">사용 중</StatusBadge>,
    description: (
      <>
        서울 <span>관리자</span>
      </>
    ),
  },
};

export const Spec: Story = List;
export const States: Story = Status;
export const SpecDark: Story = { ...Spec, globals: { theme: "dark" } };
export const StatesDark: Story = { ...States, globals: { theme: "dark" } };
