// What the product CSS actually does, read at build time (read-only) from apps/web and @repo/ui.
// DESIGN.md states the intended scale; these counts show where the code stands today, so the pages
// can tell the two apart instead of presenting the design as if it were already the code.

import { light, narrow, resolveToken } from "./tokens";

const files = import.meta.glob(
  ["../../../web/app/**/*.css", "../../../web/components/**/*.css", "../../../../packages/ui/src/**/*.css"],
  { query: "?raw", import: "default", eager: true },
) as Record<string, string>;

const strip = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, "");
const sources = Object.entries(files).map(([path, css]) => [path, strip(css)] as const);

/** Number of CSS files read. Zero means the glob broke, and every table below would be empty. */
export const fileCount = sources.length;
if (fileCount === 0) throw new Error("No product CSS found for the foundation pages");

/** How often each value of a property is declared, most used first. */
export function usage(property: string): [value: string, count: number][] {
  const counts = new Map<string, number>();
  const pattern = new RegExp(`(?:^|[;{\\s])${property}\\s*:\\s*([^;}]+)`, "g");
  for (const [, css] of sources)
    for (const [, value] of css.matchAll(pattern)) {
      const v = value!.trim();
      counts.set(v, (counts.get(v) ?? 0) + 1);
    }
  return [...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
}

/** Product CSS files (path from the source root) whose text contains `needle`. */
export function filesWith(needle: string): string[] {
  return sources
    .filter(([, css]) => css.includes(needle))
    .map(([path]) => path.replace(/^(\.\.\/)+/, ""))
    .sort();
}

/** `@media` conditions and how many rules use each. */
export function mediaQueries(): [condition: string, count: number][] {
  const counts = new Map<string, number>();
  for (const [, css] of sources)
    for (const [, condition] of css.matchAll(/@media\s*([^{]+)\{/g)) {
      const c = condition!.trim();
      counts.set(c, (counts.get(c) ?? 0) + 1);
    }
  return [...counts].sort((a, b) => b[1] - a[1]);
}

function file(suffix: string): string {
  const found = sources.find(([path]) => path.endsWith(suffix));
  if (!found) throw new Error(`${suffix} not found`);
  return found[1];
}

/** A declaration inside the first `selector { … }`, optionally only inside a given `@media` block. */
function declaration(css: string, selector: string, property: string, fallback?: string): string {
  const start = css.indexOf(`${selector} {`);
  if (start < 0) {
    if (fallback !== undefined) return fallback;
    throw new Error(`${selector} not found`);
  }
  const body = css.slice(start, css.indexOf("}", start));
  const match = body.match(new RegExp(`(?:^|[;{\\s])${property}\\s*:\\s*([^;]+);`));
  if (!match) {
    if (fallback !== undefined) return fallback;
    throw new Error(`${selector} has no ${property}`);
  }
  return match[1]!.trim();
}

function mediaBlock(css: string, condition: string): string {
  const start = css.indexOf(`@media ${condition}`);
  if (start < 0) throw new Error(`@media ${condition} not found`);
  return css.slice(start);
}

// The shell is @repo/ui's AppShell (every screen: app, node, admin). Its module sizes itself with
// tokens, so the numbers come from tokens.css (with its narrow-screen block) and the rest from the module.
const shell = file("/packages/ui/src/app-shell.module.css");
const narrowQuery = "(max-width: 768px)";
const drawerQuery = "(max-width: 960px)";
const padding = (tokens: Record<string, string>) => `${tokens["--page-top"]} ${tokens["--page-x"]}`;

/** The app shell's layout values: AppShell's module and the tokens it reads. */
export const shellLayout = {
  contentMaxWidth: declaration(shell, ".content", "max-width"),
  contentPadding: padding(light),
  contentPaddingNarrow: padding({ ...light, ...narrow }),
  narrowQuery,
  drawerQuery,
  sidebarWidth: light["--size-nav"]!,
  topbarHeight: light["--size-topbar"]!,
  drawerWidth: declaration(mediaBlock(shell, drawerQuery), ".sidebar", "width", declaration(shell, ".sidebar", "width"))
    .replace(/var\(\s*(--[\w-]+)\s*\)/g, (_, name: string) => resolveToken(name, light)),
};

/**
 * The font-family the app declares for the page body (apps/web/app/globals.css), from whichever
 * rule's selector list names `body` (`html, body`, `:where(body)`, inside `@layer base`, …). null when none does, so a
 * restyled globals.css never breaks this page; the page then shows the token alone.
 */
export const bodyFontFamily: string | null = (() => {
  const css = file("/app/globals.css");
  // Innermost rules only (no braces inside), so a rule nested in `@layer`/`@media` is found too.
  for (const [, selector, body] of css.matchAll(/([^{};]+)\{([^{}]*)\}/g)) {
    if (!/(^|[\s,(])body([\s,):]|$)/.test(selector!.trim())) continue;
    const match = body!.match(/(?:^|[;\s])font-family\s*:\s*([^;]+);/);
    if (match) return match[1]!.trim();
  }
  return null;
})();
