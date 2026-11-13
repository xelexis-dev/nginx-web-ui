import { expect, it } from "vitest";
import cases from "./certificate-hostname.cases.json";
import { certificateCoversHostname } from "./certificate-hostname";

it.each(cases)("$pattern covers $hostname: $expected", ({ pattern, hostname, expected }) => {
  expect(certificateCoversHostname(pattern, hostname)).toBe(expected);
});

it("rejects overlong DNS labels and names", () => {
  const label = `${"a".repeat(64)}.example.test`;
  expect(certificateCoversHostname(label, label)).toBe(false);
  const name = Array(5).fill("a".repeat(63)).join(".");
  expect(certificateCoversHostname(name, name)).toBe(false);
});
