import { describe, expect, it } from "vitest";
import { seriesDash } from "./series-style";

describe("seriesDash", () => {
  it("draws slots 1–3 solid and 4–6 in three different patterns", () => {
    expect([0, 1, 2].map(seriesDash)).toEqual([undefined, undefined, undefined]);
    const patterns = [3, 4, 5].map(seriesDash);
    expect(patterns.every(Boolean)).toBe(true);
    expect(new Set(patterns).size).toBe(3);
  });

  it("separates the pairs that colour-vision simulation merges (2–6, 3–5, 4–5, 2–5)", () => {
    for (const [a, b] of [[2, 6], [3, 5], [4, 5], [2, 5]] as const) expect(seriesDash(a - 1)).not.toBe(seriesDash(b - 1));
  });
});
