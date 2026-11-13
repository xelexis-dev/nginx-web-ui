import { Field, FileInput, type FileInputProps } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { useId } from "react";
import { expect, fn, mocked, userEvent, within } from "storybook/test";

const LABEL = "인증서 파일";
const ACCEPT = ".pem,.crt,.cer,.txt";
const LONG_NAME = "운영팀과_고객지원팀이_함께_사용하는_서비스의_갱신된_전체_인증서_fullchain.pem";
// A real File. Its text is a placeholder: nothing here reads it.
const sampleFile = (name: string) => new File(["-----BEGIN CERTIFICATE-----\n(예시)\n-----END CERTIFICATE-----\n"], name, { type: "application/x-pem-file" });

/**
 * Chooses a real File the way the browser's picker does: a real FileList through the input's own
 * `files`, then `input` and `change`. The browser then draws the file's name itself. (userEvent.upload
 * only overrides `files` in script, so the browser went on drawing 「선택된 파일 없음」.)
 */
function chooseFile(input: HTMLInputElement, file: File) {
  const transfer = new DataTransfer();
  transfer.items.add(file);
  input.files = transfer.files;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

/** The control in a Field. Ids are per instance, so the Docs page can show several stories. */
function Labelled({ error, ...args }: FileInputProps & { error?: string }) {
  const id = useId();
  return (
    <Field label={LABEL} htmlFor={id} error={error}>
      <FileInput {...args} />
    </Field>
  );
}

const meta = {
  title: "UI/FileInput",
  component: FileInput,
  decorators: [(Story) => <main style={{ maxWidth: 1200, margin: "0 auto", width: "100%" }}><h1 style={{ fontSize: "var(--fs-2xl)", margin: "0 0 20px" }}>파일 선택</h1><Story /></main>],
  parameters: {
    docs: {
      subtitle: "로컬 파일 하나를 고르는 칸",
      description: {
        component: [
          "브라우저 기본 `<input type=\"file\">`에 32px 입력 칸 모양을 입힌 것이다. 키보드, 운영체제의 파일 선택 창, `accept`, 화면 읽기 프로그램은 브라우저 그대로 동작한다.",
          "",
          "- **고르기만 한다.** `name`이 없어 파일은 폼 제출에 실리지 않는다(API가 파일 부분을 거절한다). 쓰는 쪽이 `onChange`에서 `event.currentTarget.files?.[0]`을 읽는다.",
          "- **문구와 파일 이름은 브라우저가 그린다.** 「파일 선택」과 「선택된 파일 없음」은 브라우저 언어를 따르고, 파일 이름은 고른 파일의 실제 이름이며 길면 브라우저가 줄인다.",
          "- **라벨·도움말·오류**는 `Field`로 준다(화면에 라벨이 없으면 `aria-label`). 오류가 있으면 테두리가 빨갛다(`aria-invalid`).",
          "- **키보드:** Tab으로 들어와 Enter나 Space로 파일 선택 창을 연다.",
          "- 한 번에 한 파일이다. 끌어 놓기, 지우기, 업로드 진행 표시는 없다.",
          "",
          "파일을 고른 예시(「선택됨」, 「좁은 화면」)는 각 이야기 화면에서 play가 실제 파일을 넣어 보여 준다.",
        ].join("\n"),
      },
    },
  },
  args: { accept: ACCEPT, onChange: fn() },
  argTypes: {
    accept: { description: "고를 수 있는 파일 형식(확장자나 MIME)" },
    onChange: { description: "파일을 고르면 호출. 파일은 `event.currentTarget.files?.[0]`" },
    disabled: { description: "고를 수 없게 막는다" },
    id: { control: false, description: "입력의 id(`Field`의 `htmlFor`가 채운다)" },
    "aria-label": { description: "화면에 라벨이 없을 때 읽어 줄 이름" },
  },
  render: (args) => <Labelled {...args} />,
} satisfies Meta<typeof FileInput>;
export default meta;
type Story = StoryObj<typeof meta>;

/** 아직 고르지 않은 상태. 오른쪽 문구는 브라우저가 쓴다. */
export const Empty: Story = { name: "선택 전" };

/** 실제 파일을 고른 상태(play가 넣는다). 이름은 브라우저가 그리고, 길면 줄인다. */
export const Selected: Story = {
  name: "선택됨 · 긴 이름",
  play: async ({ canvasElement, args }) => {
    const onChange = mocked(args.onChange!);
    onChange.mockClear();
    const input = within(canvasElement).getByLabelText<HTMLInputElement>(LABEL);
    chooseFile(input, sampleFile(LONG_NAME));
    await expect(input.files?.[0]?.name).toBe(LONG_NAME);
    await expect(input.value.endsWith(LONG_NAME)).toBe(true);
    await expect(onChange).toHaveBeenCalledOnce();
  },
};

/** `Field`의 오류: 테두리가 빨갛고, 문구가 입력의 설명으로 이어진다. */
export const Invalid: Story = {
  name: "오류",
  render: (args) => <Labelled {...args} error="PEM 형식의 인증서 파일을 고르세요." />,
  play: async ({ canvasElement }) => {
    const input = within(canvasElement).getByLabelText<HTMLInputElement>(LABEL);
    await expect(input).toHaveAttribute("aria-invalid", "true");
    await expect(input).toHaveAccessibleDescription("PEM 형식의 인증서 파일을 고르세요.");
  },
};

export const Disabled: Story = {
  name: "사용 불가",
  args: { disabled: true },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByLabelText(LABEL)).toBeDisabled();
  },
};

/** 좁은 화면(390): 칸은 폭을 채우고, 긴 이름은 브라우저가 줄여 가로로 넘치지 않는다. */
export const Narrow: Story = {
  name: "좁은 화면 · 긴 이름",
  globals: { viewport: { value: "ui390", isRotated: false } },
  parameters: { viewport: { options: { ui390: { name: "Mobile 390", styles: { width: "390px", height: "844px" }, type: "mobile" } } } },
  play: Selected.play,
};

/** 키보드: Tab 한 번에 이 칸으로 들어오고 포커스 테두리가 보인다(Enter·Space로 파일 선택 창). */
export const Keyboard: Story = {
  name: "키보드 포커스",
  play: async ({ canvasElement }) => {
    const input = within(canvasElement).getByLabelText<HTMLInputElement>(LABEL);
    await userEvent.tab();
    await expect(input).toHaveFocus();
    await expect(input.matches(":focus-visible")).toBe(true);
  },
};

export const InvalidFocused: Story = {
  ...Invalid,
  name: "오류와 키보드 포커스",
  play: async (context) => {
    await Invalid.play!(context);
    await userEvent.tab();
    await expect(within(context.canvasElement).getByLabelText(LABEL)).toHaveFocus();
  },
};
