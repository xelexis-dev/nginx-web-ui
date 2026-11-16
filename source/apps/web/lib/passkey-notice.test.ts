import { describe, expect, it } from "vitest";
import { selectPasskeyNotice } from "./passkey-notice";
const urls = [
  null,
  "https://console.example.com",
  "http://console.example.com:8080",
  "http://localhost:8080",
  "http://192.0.2.10:8080",
  "http://[::1]:8080",
];
describe("passkey notice matrix", () => {
  for (const reason of [
    "insecure_origin",
    "ip_host",
    "auth_url_http",
    null,
    "future_reason",
  ]) {
    for (const role of ["admin", "editor", "viewer"]) {
      it.each(urls)(
        `${reason}/${role}/%s preserves server facts and limits advice by role`,
        (href) => {
          const browser = href ? new URL(href) : null;
          const server = selectPasskeyNotice(reason, null, role);
          const value = selectPasskeyNotice(reason, browser, role);
          expect(value.rows.slice(0, server.rows.length)).toEqual(server.rows);
          expect(value.cause.startsWith(server.cause)).toBe(true);
          expect(value.lead).toBe(server.lead);
          const text = JSON.stringify({ ...value, remedy: undefined });
          for (const code of [
            "insecure_origin",
            "ip_host",
            "auth_url_http",
            "future_reason",
          ])
            expect(text).not.toContain(code);
          if (browser?.protocol === "https:")
            expect(text).not.toContain("HTTPS로 접속하세요");
          if (role !== "admin") {
            expect(text).not.toContain("AUTH_URL");
            expect(value.footer).toBe(false);
          }
          const expected =
            reason === "auth_url_http"
              ? "G"
              : reason === "insecure_origin"
                ? !browser
                  ? null
                  : browser.protocol === "https:"
                    ? "D"
                    : browser.hostname === "localhost"
                      ? "UX"
                      : "E"
                : reason === "ip_host"
                  ? !browser
                    ? null
                    : browser.hostname === "192.0.2.10" ||
                        browser.hostname === "[::1]"
                      ? "F"
                      : "UX"
                  : "U0";
          expect(value.remedy).toBe(expected);
          expect(value.pending).toBe(expected === null);
          expect(value.remedyLabel).toBe(
            role === "admin"
              ? value.items.length > 1
                ? "해결 방법 · 둘 중 하나"
                : "해결 방법"
              : "할 수 있는 일",
          );
        },
      );
    }
  }
});
