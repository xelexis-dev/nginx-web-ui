import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { FALLBACK_THEME_CSS } from "./fallback-theme";

// A root-layout/API failure must retain the design without fetching its CSS bundle.
// Compare declarations, including both palettes, so a later token change cannot leave stale colours.
describe("standalone error theme", () => {
  it("mirrors the shared tokens for both themes", () => {
    const source = readFileSync(resolve(import.meta.dirname, "../../../packages/ui/src/tokens.css"), "utf8");
    const normalize = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\s+/g, " ").trim();
    expect(normalize(FALLBACK_THEME_CSS)).toBe(normalize(source));
  });
});

// Aliases must stay self-contained when neither the app CSS nor its root layout arrives.
it("defines every variable its standalone palette references", () => {
  const defined = new Set([...FALLBACK_THEME_CSS.matchAll(/(--[\w-]+)\s*:/g)].map(([, name]) => name));
  for (const [, reference] of FALLBACK_THEME_CSS.matchAll(/var\(\s*(--[\w-]+)/g)) {
    expect(defined.has(reference), reference).toBe(true);
  }
});
