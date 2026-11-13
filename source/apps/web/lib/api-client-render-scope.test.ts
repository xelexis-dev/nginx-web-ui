import { beforeEach, expect, it, vi } from "vitest";
const scope = vi.hoisted(() => ({ map: new Map<string, unknown>(), cookie: "authjs.session-token=first" }));
// React owns this map on the Flight request; resetting it represents the next render.
vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  cache: (fn: (path: string) => unknown) => (path: string) => {
    if (!scope.map.has(path)) scope.map.set(path, fn(path));
    return scope.map.get(path);
  },
}));
vi.mock("next/headers", () => ({ headers: async () => new Headers({ cookie: scope.cookie }) }));
import { apiRead } from "./api-client";
beforeEach(() => { scope.map.clear(); scope.cookie = "authjs.session-token=first"; vi.unstubAllGlobals(); });
it("reads each path once per render, then reads again with the next request's cookie", async () => {
  const fetch = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => Response.json({ ok: true }));
  vi.stubGlobal("fetch", fetch);
  const rp = "/api/ui/v1/views?path=%2Freverse-proxy";
  await Promise.all([apiRead(rp), apiRead(rp)]);
  await apiRead("/api/ui/v1/views?path=%2Fcertificates");
  expect(fetch).toHaveBeenCalledTimes(2);
  scope.map.clear();
  scope.cookie = "authjs.session-token=second";
  await apiRead(rp);
  expect(fetch).toHaveBeenCalledTimes(3);
  expect(new Headers(fetch.mock.calls[2]![1]?.headers).get("cookie")).toBe(scope.cookie);
});
it("shares one rejected read without changing the API error", async () => {
  const fetch = vi.fn(async () => { throw new DOMException("timed out", "TimeoutError"); });
  vi.stubGlobal("fetch", fetch);
  const both = await Promise.allSettled([apiRead("/api/ui/v1/session"), apiRead("/api/ui/v1/session")]);
  expect(both.map(r => r.status === "rejected" && (r.reason as { code: string }).code)).toEqual(["api_timeout", "api_timeout"]);
  expect(both[0].status === "rejected" && both[0].reason).toBe(both[1].status === "rejected" && both[1].reason);
  expect(fetch).toHaveBeenCalledTimes(1);
});
