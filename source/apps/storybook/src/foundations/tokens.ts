import css from "@repo/ui/tokens.css?raw";

// The foundation pages read the tokens from the stylesheet the product ships, so a value shown here
// is the value the app renders. A missing token throws: a renamed token breaks the page instead of
// showing an empty swatch.

const withoutComments = css.replace(/\/\*[\s\S]*?\*\//g, "");

function block(selector: string): Record<string, string> {
  const start = withoutComments.indexOf(`${selector} {`);
  if (start < 0) throw new Error(`Token block ${selector} not found in @repo/ui/tokens.css`);
  const open = withoutComments.indexOf("{", start);
  const body = withoutComments.slice(open + 1, withoutComments.indexOf("}", open));
  const tokens: Record<string, string> = {};
  for (const [, name, value] of body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) tokens[name!] = value!.trim();
  return tokens;
}

export const light = block(":root");
/** The explicit dark palette (`:root[data-theme="dark"]`), which the toolbar and the app switch to. */
export const dark = block(':root[data-theme="dark"]');
/** The copy for pages whose theme script never ran (404/500), keyed on `prefers-color-scheme`. */
export const darkBySystem = block(':root:not([data-theme="light"])');


/** Overrides for narrow screens (`@media (max-width: 768px)`), e.g. the smaller page padding. */
export const narrow: Record<string, string> = (() => {
  const at = withoutComments.indexOf("(max-width: 768px)");
  if (at < 0) return {};
  const start = withoutComments.indexOf(":root {", at);
  const body = withoutComments.slice(start + ":root {".length, withoutComments.indexOf("}", start));
  return Object.fromEntries([...body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)].map(([, n, v]) => [n!, v!.trim()]));
})();

/** Tokens whose two dark declarations differ; tokens.css says they must always be equal. */
export const darkMismatches = [...new Set([...Object.keys(dark), ...Object.keys(darkBySystem)])].filter(
  (name) => dark[name] !== darkBySystem[name],
);

/** Every token in stylesheet order. */
export const tokenNames = Object.keys(light);

/** Resolve aliases against one palette, never the outer document's selected theme. */
export function resolveToken(name: string, values: Record<string, string>, trail: string[] = []): string {
  if (trail.includes(name)) throw new Error(`Circular token alias: ${[...trail, name].join(" → ")}`);
  const value = values[name];
  if (!value) throw new Error(`Token ${name} not found`);
  return value.replace(/var\(\s*(--[\w-]+)\s*\)/g, (_, alias: string) => resolveToken(alias, values, [...trail, name]));
}

const darkPalette = { ...light, ...dark };

export function token(name: string, theme: "light" | "dark" = "light"): string {
  return resolveToken(name, theme === "dark" ? darkPalette : light);
}

type Rgba = [number, number, number, number];

function parseColor(value: string): Rgba {
  const hex = value.match(/^#([0-9a-f]{6})$/i);
  if (hex) {
    const n = parseInt(hex[1]!, 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255, 1];
  }
  const rgba = value.match(/^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+))?\s*\)$/i);
  if (rgba) return [Number(rgba[1]), Number(rgba[2]), Number(rgba[3]), rgba[4] === undefined ? 1 : Number(rgba[4])];
  throw new Error(`Not a colour this page can measure: ${value}`);
}

// Swatches can paint color-mix; contrast() measures only hex/rgb values with a known background.
export function isColor(value: string): boolean {
  return /^#[0-9a-f]{6}$/i.test(value) || /^rgba?\(/i.test(value) || /^color-mix\(/i.test(value);
}

/** WCAG 2 contrast ratio. A translucent foreground is composited over the background first. */
export function contrast(foreground: string, background: string): number {
  const [br, bg, bb] = parseColor(background);
  const [fr, fg, fb, fa] = parseColor(foreground);
  const mix = (f: number, b: number) => f * fa + b * (1 - fa);
  const luminance = (r: number, g: number, b: number) =>
    [r, g, b]
      .map((c) => c / 255)
      .map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4))
      .reduce((sum, c, i) => sum + c * [0.2126, 0.7152, 0.0722][i]!, 0);
  const a = luminance(mix(fr, br), mix(fg, bg), mix(fb, bb));
  const b = luminance(br, bg, bb);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}
