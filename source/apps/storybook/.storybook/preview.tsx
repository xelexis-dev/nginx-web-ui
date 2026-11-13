import "@repo/ui/tokens.css";
import "@repo/ui/fonts.css";
// The app's own base layer (body type, form-control fonts, focus ring, mono, reduced motion), so every
// story renders on the same canvas as the app. `@/` is apps/web (main.ts).
import "@/app/globals.css";
import "./preview.css";
import "../src/docs/docs.css";
import { applyTheme } from "@repo/ui";
import type { Preview } from "@storybook/react-vite";
import { ThemedDocsContainer } from "./docs-container";
import { normalizeTheme } from "./theme";

if (typeof document !== "undefined") document.documentElement.lang = "ko";

// Light/Dark through the app's own switch: tokens.css reads `data-theme` on <html>, and
// applyTheme is what the app calls. The toolbar value is a global, so it applies to every story;
// Docs pages follow it through ThemedDocsContainer (MDX pages run no decorators).
const preview: Preview = {
  globalTypes: {
    theme: {
      description: "색상 테마 (<html>의 data-theme)",
      toolbar: {
        title: "테마",
        icon: "mirror",
        items: [
          { value: "light", title: "라이트", icon: "sun" },
          { value: "dark", title: "다크", icon: "moon" },
        ],
        dynamicTitle: true,
      },
    },
  },
  initialGlobals: { theme: "light" },
  decorators: [
    (Story, context) => {
      applyTheme(normalizeTheme(context.globals.theme));
      return <Story />;
    },
  ],
  // Every component gets a Docs page (usage, props, examples) above its stories.
  tags: ["autodocs"],
  parameters: {
    // The canvas color is the theme's --canvas token, not a Storybook background.
    backgrounds: { disable: true },
    layout: "padded",
    controls: { sort: "requiredFirst" },
    docs: {
      container: ThemedDocsContainer,
      toc: { headingSelector: "h2, h3", ignoreSelector: ".docs-story *, .sbdocs-subtitle", title: "목차" },
      // Code snippets show the component usage, not story plumbing.
      source: { excludeDecorators: true },
    },
    options: {
      storySort: {
        order: [
          // The product as it ships comes first: real apps/web pages on the shared components.
          "Product",
          ["Login", "Login 2FA", "Login Backup", "Operations", ["개요", "리버스 프록시 목록", "서버 상세", "서버 수정"], "Certificates", ["Import"], "Account", ["2FA Backup Codes"]],
          // Stable Figma review links render the same production screens and shared components.
          // The page contracts every screen follows, built only from @repo/ui.
          "Templates",
          ["소개", "목록", "상세", "폼", "설정"],
          "UI",
          [
            "AppShell",
            "PageHeader",
            "Surface",
            "Table",
            "StatusBadge",
            "KpiRow",
            "Facts",
            "Banner",
            "CodePane",
            "FilterChips · SearchField · Switch",
            "FormSection · Field",
            "TextInput",
            "FileInput",
            "SegmentedControl",
            "Button",
            "Select",
            "Combobox",
            "Modal",
            "Icon",
          ],
          "Foundations",
          ["Colors", "Typography", "Layout", "Spacing", "Accessibility"],
          "Introduction",
        ],
      },
    },
  },
};

export default preview;
