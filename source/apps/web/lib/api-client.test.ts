import { afterEach, expect, it, vi } from "vitest";
import { apiRead, localApiUrl } from "./api-client";

vi.mock("next/headers", () => ({ headers: async () => new Headers({ cookie: "authjs.session-token=test-only", host: "ui.example.test" }) }));
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); });

it("refuses destinations that would send browser credentials outside the local API", () => {
  expect(localApiUrl("/api/ui/v1/session").href).toBe("http://127.0.0.1:3001/api/ui/v1/session");
  for (const base of ["https://outside.example.test", "http://127.0.0.1@outside.test", "http://127.0.0.1/api"]) {
    expect(() => localApiUrl("/api/ui/v1/session", base)).toThrow();
  }
  for (const path of ["//outside.test", "https://outside.test", "/api/../../outside", "/api/\\outside"]) {
    expect(() => localApiUrl(path)).toThrow();
  }
});

it("preserves session context but never follows an API redirect", async () => {
  let sent: RequestInit | undefined;
  vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
    sent = init;
    return Response.json({ error: { code: "moved", message: "이동 실패" } }, { status: 302, headers: { location: "https://outside.test" } });
  });
  await expect(apiRead("/api/ui/v1/session")).rejects.toMatchObject({ status: 302, code: "moved" });
  expect(new Headers(sent?.headers).get("cookie")).toBe("authjs.session-token=test-only");
  expect(sent?.redirect).toBe("manual");
  expect(sent?.cache).toBe("no-store");
});

it("waits longer than the node read timeout so unreachable results can arrive",async()=>{
  const timeout=vi.spyOn(AbortSignal,"timeout");
  vi.stubGlobal("fetch",async()=>Response.json({ok:false,nodeError:{failure:{kind:"unreachable",message:"연결할 수 없습니다"}}}));
  const result=await apiRead<{ok:boolean}>("/api/ui/v1/views?path=/nodes/n1");
  expect(result.ok).toBe(false);
  expect(timeout.mock.calls.at(-1)?.[0]).toBeGreaterThan(10_000);
  timeout.mockRestore();
});
it("distinguishes an SSR response timeout from an unreachable API",async()=>{
  vi.stubGlobal("fetch",async()=>{throw new DOMException("timed out","TimeoutError")});
  await expect(apiRead("/api/ui/v1/session")).rejects.toMatchObject({status:503,code:"api_timeout"});
});
