import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ButtonProps, CodePaneProps } from "@repo/ui";
import { renderToStaticMarkup } from "react-dom/server";

let copied = "idle";
let copy: (() => void) | undefined;
let effects: (() => unknown)[] = [];
vi.mock("react", async (original) => {
  const actual = await original<typeof import("react")>();
  return {
    ...actual,
    useState: (initial: unknown) =>
      initial === "idle"
        ? [
            copied,
            (next: string) => {
              copied = next;
            },
          ]
        : actual.useState(initial as never),
    useEffect: (effect: () => unknown) => {
      effects.push(effect);
    },
  };
});
vi.mock("../../../packages/ui/src/button", async (original) => {
  const actual =
    await original<typeof import("../../../packages/ui/src/button")>();
  return {
    ...actual,
    Button: (props: ButtonProps) => {
      copy = () => props.onClick?.({} as never);
      return actual.Button(props);
    },
  };
});
const { CodePane } = await import("@repo/ui");
const instruction = "복사할 수 없습니다 — 위 값을 직접 선택해 복사하세요.";
function render(variant: CodePaneProps["variant"]) {
  effects = [];
  const html = renderToStaticMarkup(
    <CodePane title="토큰" variant={variant} code="example-secret" />,
  );
  for (const effect of effects) effect();
  return html;
}
beforeEach(() => {
  copied = "idle";
  vi.useFakeTimers();
  vi.stubGlobal("window", { setTimeout, clearTimeout });
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

it.each(["absent", "rejected"])(
  "announces %s clipboard failure only for secrets and retains it until retry",
  async (failure) => {
    for (const variant of ["secret", "code"] as const) {
      copied = "idle";
      vi.stubGlobal(
        "navigator",
        failure === "absent"
          ? {}
          : {
              clipboard: {
                writeText: () => Promise.reject(new Error("NotAllowedError")),
              },
            },
      );
      expect(render(variant)).not.toContain(instruction);
      copy!();
      await Promise.resolve();
      let html = render(variant);
      expect(html).toContain("복사할 수 없음");
      expect(html.includes(instruction)).toBe(variant === "secret");
      expect(html.includes('role="alert"')).toBe(variant === "secret");
      vi.advanceTimersByTime(2500);
      html = render(variant);
      expect(html.includes(instruction)).toBe(variant === "secret");
      if (variant === "secret") {
        let finish!: () => void;
        vi.stubGlobal("navigator", {
          clipboard: {
            writeText: () =>
              new Promise<void>((resolve) => {
                finish = resolve;
              }),
          },
        });
        copy!();
        expect(render(variant)).not.toContain(instruction);
        finish();
        await Promise.resolve();
        expect(render(variant)).toContain("복사됨");
        vi.advanceTimersByTime(2500);
        expect(render(variant)).not.toContain("복사됨");
      }
    }
  },
);
