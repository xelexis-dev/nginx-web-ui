import { expect, it } from "vitest";
import { adminPasskeyRemovedMessage } from "./admin-passkey-removed-message";
it.each([
  [
    true,
    "패스키를 삭제했습니다. 다른 기기의 로그인은 종료되고 이 기기의 로그인은 유지됩니다.",
  ],
  [false, "패스키를 삭제하고 사용자의 로그인을 종료했습니다."],
] as const)("preserves admin passkey removal copy (self=%s)", (self, text) => {
  expect(adminPasskeyRemovedMessage(self)).toBe(text);
});
