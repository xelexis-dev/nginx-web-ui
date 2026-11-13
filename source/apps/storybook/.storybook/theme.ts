import { create } from "storybook/theming";

export type Theme = "light" | "dark";
export const normalizeTheme = (value: unknown): Theme =>
  value === "dark" ? "dark" : "light";

// Storybook's own chrome (sidebar, toolbar, Docs prose, prop tables, code blocks) in the app's
// font stack and Figma operator-console surfaces, light or dark with the `theme` global. The manager bundle does
// not load @repo/ui/tokens.css, so these mirror its values (canvas, surface, ink, muted, hairline);
// the Foundations page compares them with the production tokens.
const tokens = {
  light: {
    canvas: "#f3f6f6",
    surface: "#ffffff",
    ink: "#142b2f",
    muted: "#596e72",
    hairline: "#d8e2e1",
  },
  dark: {
    canvas: "#0c171a",
    surface: "#132326",
    ink: "#edf6f4",
    muted: "#a7bcbd",
    hairline: "#31474b",
  },
} as const;
// The manager cannot read tokens.css, so this repeats --font-sans; the Typography page warns when
// the copy drifts. Code blocks use the same face: every text is Pretendard (--font-mono is an alias).
const fontBase =
  '"Pretendard Variable", Pretendard, -apple-system, BlinkMacSystemFont, system-ui, Roboto, "Helvetica Neue", "Segoe UI", "Apple SD Gothic Neo", "Noto Sans KR", "Malgun Gothic", "Apple Color Emoji", "Segoe UI Emoji", "Segoe UI Symbol", sans-serif';
const fontCode = fontBase;
const theme = (base: Theme) => {
  const t = tokens[base];
  return create({
    base,
    brandTitle: "nginx-web-ui UI",
    fontBase,
    fontCode,
    appBg: t.surface,
    appContentBg: t.canvas,
    appPreviewBg: t.canvas,
    appBorderColor: t.hairline,
    appBorderRadius: 10,
    textColor: t.ink,
    textMutedColor: t.muted,
    barBg: t.canvas,
    inputBg: t.canvas,
    inputBorder: t.hairline,
    inputTextColor: t.ink,
    inputBorderRadius: 6,
  });
};
export const toolThemes = { light: theme("light"), dark: theme("dark") };
export const themeTokens = tokens;
export const themeFonts = { sans: fontBase, mono: fontCode };
