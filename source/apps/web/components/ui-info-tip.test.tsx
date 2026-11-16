import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  Card,
  CodePane,
  Field,
  InfoTip,
  PageHeader,
  Surface,
} from "@repo/ui";
import { infoTipReducer } from "../../../packages/ui/src/info-tip";

it("keeps the closed explanation in the DOM and links it to the named trigger", () => {
  const html = renderToStaticMarkup(
    <InfoTip label="페이지 설명">기존 설명</InfoTip>,
  );
  const button = html.match(/<button[^>]*>/)![0];
  const id = button.match(/aria-describedby="([^"]+)"/)![1];
  expect(button).toContain('type="button"');
  expect(button).toContain('aria-label="페이지 설명"');
  expect(button).toContain('aria-expanded="false"');
  expect(html).toContain(`id="${id}" role="tooltip" hidden=""`);
  expect(html).toContain("기존 설명");
});

it.each([false, true])(
  "merges Field info with existing hint, error and caller descriptions (%s)",
  (callback) => {
    const html = renderToStaticMarkup(
      <Field
        label="기간"
        htmlFor="days"
        info="배경"
        hint="숫자"
        error="확인"
        describedBy="extra"
        required
      >
        {callback ? (
          (props) => <input {...props} />
        ) : (
          <input aria-describedby="own" />
        )}
      </Field>,
    );
    const input = html.match(/<input[^>]*>/)![0];
    expect(input).toContain('id="days"');
    expect(input).toContain(
      `aria-describedby="${callback ? "" : "own "}extra days-hint days-error days-info"`,
    );
    expect(input).toContain('aria-required="true"');
    expect(input).toContain('aria-invalid="true"');
    expect(html).toContain('id="days-info" role="tooltip" hidden=""');
    expect(html).toContain('for="days"');
  },
);

it("keeps tips beside headings and labels, and preserves visible descriptions", () => {
  const html = renderToStaticMarkup(
    <>
      <PageHeader title="페이지" info="배경" description="현재 상태" />
      <Surface title="카드" info="배경">
        본문
      </Surface>
      <Card id="section" title="구역" info="배경">
        본문
      </Card>
      <CodePane title={<span>설정</span>} info="배경" code="safe" />
    </>,
  );
  expect(html.match(/role="tooltip"/g)).toHaveLength(4);
  expect(html).toContain('aria-label="카드 설명"');
  expect(html).toContain('aria-label="구역 설명"');
  expect(html).toContain('aria-label="설명"');
  expect(html).toContain("현재 상태");
  for (const heading of html.matchAll(/<h[123][^>]*>(.*?)<\/h[123]>/g))
    expect(heading[1]).not.toContain("<button");
});

describe("independent open and pinned state (R3)", () => {
  const closed = { open: false, pinned: false };
  it("opens on focus/hover, pins on first click, closes on second click without pointerleave", () => {
    let state = infoTipReducer(closed, "open");
    expect(state).toEqual({ open: true, pinned: false });
    state = infoTipReducer(state, "toggle");
    expect(state).toEqual({ open: true, pinned: true });
    expect(infoTipReducer(state, "leave")).toEqual(state);
    expect(infoTipReducer(state, "toggle")).toEqual(closed);
  });
  it("leaves pointer focus closed until click and toggles the pinned state", () => {
    const afterTap = infoTipReducer(closed, "toggle");
    expect(afterTap).toEqual({ open: true, pinned: true });
    expect(infoTipReducer(afterTap, "toggle")).toEqual(closed);
  });
  it("blur/leave closes only unpinned tips, while Esc/outside always close", () => {
    expect(infoTipReducer({ open: true, pinned: false }, "leave")).toEqual(
      closed,
    );
    expect(infoTipReducer({ open: true, pinned: true }, "close")).toEqual(
      closed,
    );
  });
});
