import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

const mocks = vi.hoisted(() => ({
  readAuthOptions: vi.fn(),
  redirect: vi.fn((path: string) => {
    throw new Error(`redirect:${path}`);
  }),
}));
vi.mock("next/navigation", () => ({ redirect: mocks.redirect, useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }) }));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<typeof import("@/lib/api-client")>()),
  readAuthOptions: mocks.readAuthOptions,
}));

import { ApiRequestError } from "@/lib/api-client";
import LoginPage from "./page";
import SetupPage from "../setup/page";

beforeEach(() => vi.clearAllMocks());

// Sign-in is where a person goes when something looks broken; it must not be a bare 500.
it.each([
  ["login", () => LoginPage({ searchParams: Promise.resolve({}) })],
  ["setup", () => SetupPage()],
])("%s shows the API-unavailable screen instead of throwing", async (_name, render) => {
  mocks.readAuthOptions.mockRejectedValue(new ApiRequestError(503, "api_unavailable", "API 서버에 연결하지 못했습니다."));
  const html = renderToStaticMarkup(await render());
  expect(html).toContain("API 서버에 연결하지 못했습니다");
  expect(html).toMatch(/다시 시도/);
  expect(mocks.redirect).not.toHaveBeenCalled();
});

// The middleware's document really is a 503. A page cannot set that status, so the page version
// must not print a status line the response does not carry — curl and uptime checks read 200 here.
it("does not claim an HTTP status the page response does not have", async () => {
  mocks.readAuthOptions.mockRejectedValue(new ApiRequestError(503, "api_unavailable", "API 서버에 연결하지 못했습니다."));
  const html = renderToStaticMarkup(await LoginPage({ searchParams: Promise.resolve({}) }));
  // Token scale names such as zinc-500 are CSS, not the status text a person sees.
  const markup = html.replace(/<style\b[^>]*>[\s\S]*?<\/style>/gi, "");
  expect(markup).not.toMatch(/\b5\d\d\b/);
  expect(markup).not.toMatch(/Service Unavailable/i);
});

it("still renders the form when the API answers", async () => {
  mocks.readAuthOptions.mockResolvedValue({ firstRun: false, authenticated: false, providers: [] });
  const html = renderToStaticMarkup(await LoginPage({ searchParams: Promise.resolve({}) }));
  expect(html).toContain("이메일과 비밀번호로 로그인하세요.");
});
