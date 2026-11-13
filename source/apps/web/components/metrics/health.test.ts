import { describe, expect, it } from "vitest";
import { STALE_AFTER_MS, collectionHealth } from "./health";

const at = (t: number) => `T${t}`;

describe("collectionHealth", () => {
  it("says there is no sample yet", () => {
    expect(collectionHealth(null, 1_000_000, at)).toEqual({ state: "none", text: "아직 수집된 표본이 없습니다" });
  });

  it("is collecting just under five minutes", () => {
    const last = 1_000_000;
    expect(collectionHealth(last, last + STALE_AFTER_MS - 1, at)).toEqual({ state: "ok", text: `수집 중 · 마지막 표본 T${last}`, lastSampleAt: last });
  });

  it("says the collection is late at exactly five minutes, in words and not only by colour", () => {
    const last = 1_000_000;
    expect(STALE_AFTER_MS).toBe(300_000);
    expect(collectionHealth(last, last + STALE_AFTER_MS, at)).toEqual({ state: "stale", text: `수집 지연 · 마지막 표본 T${last}`, lastSampleAt: last });
  });
});
