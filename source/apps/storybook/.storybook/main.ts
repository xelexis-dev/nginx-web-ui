import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import type { StorybookConfig } from "@storybook/react-vite";
import { mergeConfig } from "vite";

// Pretendard is @repo/ui's dependency (fonts.css). The preview gets it through that import; the
// manager (sidebar, toolbar) is a separate bundle, so it gets the same package files as static
// files and a stylesheet link in its <head>. Resolved from @repo/ui so this workspace needs no copy.
const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const webVersion: string = require("../../web/package.json").version;
const fromUi = createRequire(require.resolve("@repo/ui/tokens.css"));
const pretendard = join(dirname(fromUi.resolve("pretendard/package.json")), "dist/web/variable");

// Development-only workspace: not a dependency of `web`, so `turbo prune web` (the image
// build) leaves it and its packages out.
const config: StorybookConfig = {
  framework: "@storybook/react-vite",
  // MDX pages (Introduction, Foundations) and component stories; the sidebar order is set by
  // storySort in preview.tsx.
  stories: ["../src/**/*.mdx", "../src/**/*.stories.@(ts|tsx)"],
  addons: ["@storybook/addon-docs"],
  core: { disableTelemetry: true },
  staticDirs: [
    { from: pretendard, to: "/fonts/pretendard" },
    // The full SIL Open Font License text, the same file the app serves.
    { from: "../../web/public/licenses", to: "/licenses" },
  ],
  managerHead: (head = "") =>
    `${head}<link rel="stylesheet" href="./fonts/pretendard/pretendardvariable-dynamic-subset.css" />`,
  // @repo/ui's tsconfig keeps JSX as-is (`"jsx": "preserve"`, from the Next.js preset: Next
  // compiles it). Vite applies each file's nearest tsconfig, so without this its components
  // reach the bundler as raw JSX. Compile JSX for React here instead of editing the product
  // tsconfig.
  viteFinal: (vite) =>
    mergeConfig(vite, {
      oxc: { jsx: { runtime: "automatic" } },
      // The real web version module normally receives these constants from Next.
      // Dev Vite evaluates it before tree-shaking; define only its public build
      // metadata here, without adding a browser process shim or exposing env vars.
      define: {
        "process.env.APP_SEMVER": JSON.stringify(webVersion),
        "process.env.APP_GIT_SHA": JSON.stringify(""),
      },
      // Product stories render the real apps/web components (src/product). `@/` is the web app's own
      // path alias; Next's router and link, and the web's form action hook, get Storybook-only stand-ins
      // (src/product/adapters) — there is no Next server or API behind a story. The specific entries
      // come before `@/`. Storybook imports apps/web, never the reverse, so `turbo prune web` still
      // leaves this workspace out of the image.
      resolve: {
        alias: [
          { find: /^next\/link$/, replacement: join(here, "../src/product/adapters/next-link.tsx") },
          { find: /^next\/navigation$/, replacement: join(here, "../src/product/adapters/next-navigation.ts") },
          { find: /^@\/lib\/use-api-action$/, replacement: join(here, "../src/product/adapters/use-api-action.ts") },
          { find: /^@\//, replacement: `${join(here, "../../web")}/` },
        ],
      },
    }),
};

export default config;
