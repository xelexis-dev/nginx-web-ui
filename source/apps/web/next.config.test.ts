import { expect, it } from "vitest";
import config from "./next.config";

it("keeps static hardening while runtime proxy owns the framing policy", async () => {
  const rules = await config.headers!();
  expect(config.poweredByHeader).toBe(false);
  expect(rules).toContainEqual({ source: "/:path*", headers: expect.arrayContaining([
    { key: "X-Content-Type-Options", value: "nosniff" },
    { key: "Referrer-Policy", value: "same-origin" },
  ]) });
  expect(rules.flatMap(rule => rule.headers).some(header => /^(content-security-policy|x-frame-options)$/i.test(header.key))).toBe(false);
});

it("disables the image optimizer without ineffective trace excludes", () => {
  expect(config.images?.unoptimized).toBe(true);
  expect(config.outputFileTracingExcludes).toBeUndefined();
});
