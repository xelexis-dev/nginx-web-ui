import { describe, expect, it } from "vitest";
import { afterInsert, keepsCode } from "./six-digit-code-input";

// The rule the /login/2fa code field applies to every edit, paste and drop (the browser behaviour is
// checked in Storybook; this is the decision itself).
describe("which edits the code field keeps", () => {
  it.each(["", "0", "00", "001234", "123456", "000000"])("keeps %j", (value) => {
    expect(keepsCode(value)).toBe(true);
  });

  it.each(["1234567", "12a456", "a", "123 45", " 123456", "abcde-12345", "１２３４５６", "12.345", "-12345"])("refuses %j", (value) => {
    expect(keepsCode(value)).toBe(false);
  });
});

describe("what a paste would leave", () => {
  it("replaces the selection", () => {
    expect(afterInsert("123456", 2, 4, "9")).toBe("12956");
    expect(afterInsert("", 0, 0, "012345")).toBe("012345");
    expect(afterInsert("12", 2, 2, "3456")).toBe("123456");
  });

  it.each([
    ["", 0, 0, "1234567", false],
    ["", 0, 0, "12a456", false],
    ["", 0, 0, "abcde-12345", false],
    ["123", 3, 3, "4567", false],
    ["", 0, 0, "001234", true],
    ["123456", 0, 6, "654321", true],
  ])("pasting into %j at [%i,%i) %j is kept: %s", (current, start, end, text, kept) => {
    expect(keepsCode(afterInsert(current, start, end, text))).toBe(kept);
  });
});
