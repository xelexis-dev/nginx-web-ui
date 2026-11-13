import { expect, it } from "vitest";
import { apiUnavailableHtml, apiUnavailableResponse } from "./api-unavailable";

// What a person sees while the API is restarting: the product's own screen, not a bare sentence.
it("renders a styled screen with the Korean message and a retry control", () => {
  const html = apiUnavailableHtml();
  expect(html).toContain("<!DOCTYPE html>");
  expect(html).toContain('lang="ko"');
  expect(html).toContain("API 서버에 연결하지 못했습니다");
  expect(html).toMatch(/다시 시도/);
  expect(html).toContain("nginx-web-ui");
});

it("works on a phone and in dark mode", () => {
  const html = apiUnavailableHtml();
  expect(html).toContain('name="viewport"');
  expect(html).toContain("prefers-color-scheme: dark");
  // A fixed width would overflow a 390px viewport; a max-width does not.
  const fixed = [...html.matchAll(/(^|[^-\w])width:\s*(\d+)px/g)].map((m) => Number(m[2]));
  expect(fixed.every((w) => w <= 390)).toBe(true);
});

it("comes back on its own without the user clicking", () => {
  const html = apiUnavailableHtml();
  expect(html).toMatch(/location\.reload|http-equiv="refresh"/);
});

it("answers 503 as HTML that browsers render, and asks to be retried", () => {
  const response = apiUnavailableResponse();
  expect(response.status).toBe(503);
  expect(response.headers.get("content-type")).toMatch(/^text\/html/);
  expect(response.headers.get("cache-control")).toBe("no-store");
  expect(Number(response.headers.get("retry-after"))).toBeGreaterThan(0);
});
