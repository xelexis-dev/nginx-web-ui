import { beforeEach, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ read: vi.fn(), redirect: vi.fn((path: string) => { throw new Error(`redirect:${path}`); }), notFound: vi.fn(() => { throw new Error("not-found"); }) }));
vi.mock("next/navigation", () => ({ redirect: mocks.redirect, notFound: mocks.notFound }));
vi.mock("./api-client", async (original) => ({ ...await original<typeof import("./api-client")>(), readView: mocks.read }));
vi.mock("next/headers", () => ({ headers: async () => new Headers() }));
import { ApiRequestError } from "./api-client";
import { readPageView } from "./api-page";
beforeEach(() => vi.clearAllMocks());
it.each([
  [401, "unauthorized", "redirect:/login"],
  [403, "two_factor_required", "redirect:/account/2fa"],
  [403, "forbidden", "redirect:/login"],
  [404, "not_found", "not-found"],
])("maps API %s %s to presentation navigation", async (status, code, expected) => {
  mocks.read.mockRejectedValue(new ApiRequestError(status, code, "safe"));
  await expect(readPageView("/reverse-proxy")).rejects.toThrow(expected);
});
it("preserves API outages for the error boundary and successful view data", async () => {
  const error = new ApiRequestError(503, "unavailable", "safe");
  mocks.read.mockRejectedValueOnce(error);
  await expect(readPageView("/stream")).rejects.toBe(error);
  const view = { streams: [], canWrite: false };
  mocks.read.mockResolvedValueOnce(view);
  expect(await readPageView("/stream", { page: "2" })).toBe(view);
  expect(mocks.read).toHaveBeenLastCalledWith("/stream", { page: "2" });
});
