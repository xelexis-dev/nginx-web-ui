import { describe, expect, it, vi } from "vitest";
import { contrast, isColor, resolveToken } from "./tokens";

// The resolver tests supply their own palettes; no product CSS declaration is asserted.
vi.mock("@repo/ui/tokens.css?raw", () => ({ default: ":root {} :root[data-theme=\"dark\"] {} :root:not([data-theme=\"light\"]) {}" }));

describe("foundation token reader", () => {
  const light = { "--bg-canvas": "#ffffff", "--text-1": "#000000", "--control": "var(--bg-canvas)", "--nested": "var(--control)" };
  const dark = { ...light, "--bg-canvas": "#000000", "--text-1": "#ffffff" };

  it("resolves the same alias in its own light or dark palette", () => {
    expect(resolveToken("--nested", light)).toBe("#ffffff");
    expect(resolveToken("--nested", dark)).toBe("#000000");
  });

  it("enumerates resolved colours and measures their contrast", () => {
    expect(isColor(resolveToken("--control", light))).toBe(true);
    expect(contrast(resolveToken("--text-1", dark), resolveToken("--control", dark))).toBe(21);
    expect(isColor("12px")).toBe(false);
  });

  it("resolves repeated aliases and non-colour values", () => {
    expect(resolveToken("--pair", { "--pair": "var(--size) var(--size)", "--size": "3px" })).toBe("3px 3px");
    expect(resolveToken("--mono", { "--mono": "var(--sans)", "--sans": "Pretendard, sans-serif" })).toBe("Pretendard, sans-serif");
  });

  it("refuses missing names, dangling aliases and cycles", () => {
    expect(() => resolveToken("--missing", light)).toThrow("Token --missing not found");
    expect(() => resolveToken("--dangling", { "--dangling": "var(--missing)" })).toThrow("Token --missing not found");
    expect(() => resolveToken("--a", { "--a": "var(--b)", "--b": "var(--a)" })).toThrow("Circular token alias");
  });
});
