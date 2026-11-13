import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { NextRequest } from "next/server";
import { readdirSync } from "node:fs";
import { unstable_doesMiddlewareMatch } from "next/experimental/testing/server";
const fetchMock = vi.fn();
vi.mock("next-auth", () => { throw new Error("SSR proxy must not decrypt sessions"); });
beforeEach(() => { vi.stubGlobal("fetch", fetchMock); fetchMock.mockReset(); vi.resetModules(); vi.stubEnv("NWUI_FRAME_ANCESTORS", ""); });
afterEach(() => { vi.unstubAllEnvs(); vi.restoreAllMocks(); });
const request = (path = "/account") => new NextRequest(`https://app.example.test${path}`, { headers: { cookie: "session=opaque", "x-real-ip": "192.0.2.2", "x-forwarded-proto": "https", "x-forwarded-host": "app.example.test" } });
it("uses the API-validated session and forwards only request context", async () => {
  fetchMock.mockResolvedValue(Response.json({ user: { requires2fa: false, totpEnabled: false, verificationRequired: false } }));
  const { proxy } = await import("./proxy");
  const result = await proxy(request());
  expect(result.headers.get("x-middleware-next")).toBe("1");
  const [url, options] = fetchMock.mock.calls[0]!;
  expect(String(url)).toBe("http://127.0.0.1:3001/api/ui/v1/session");
  expect(options.headers.get("cookie")).toBe("session=opaque");
  expect(options.headers.get("x-forwarded-proto")).toBe("https");
  expect(options.cache).toBe("no-store");
});
it("redirects expired sessions to login", async () => {
  fetchMock.mockResolvedValue(Response.json({ error: { code: "session_required" } }, { status: 401 }));
  const { proxy } = await import("./proxy");
  expect((await proxy(request())).headers.get("location")).toBe("https://app.example.test/login");
});
it("enforces MFA navigation while allowing enrollment itself", async () => {
  fetchMock.mockImplementation(() => Promise.resolve(Response.json({ user: { requires2fa: true, totpEnabled: false, verificationRequired: false } })));
  const { proxy } = await import("./proxy");
  expect((await proxy(request())).headers.get("location")).toBe("https://app.example.test/account/2fa");
  expect((await proxy(request("/account/2fa"))).headers.get("x-middleware-next")).toBe("1");
  expect((await proxy(request("/account/2fa-bypass"))).headers.get("location")).toBe("https://app.example.test/account/2fa");
});
it("returns unavailable instead of login loops or granting access when API fails", async () => {
  const { proxy } = await import("./proxy");
  for (const response of [new Response("offline", { status: 503 }), Response.json({ unexpected: true })]) {
    fetchMock.mockResolvedValueOnce(response);
    const result = await proxy(request());
    expect(result.status).toBe(503);
    expect(result.headers.get("location")).toBeNull();
    expect(result.headers.get("cache-control")).toContain("no-store");
  }
});

it("requires verification for enrolled sessions and permits only the verification page", async () => {
  fetchMock.mockImplementation(() => Promise.resolve(Response.json({ user: { requires2fa: false, totpEnabled: true, verificationRequired: true } })));
  const { proxy } = await import("./proxy");
  expect((await proxy(request("/"))).headers.get("location")).toBe("https://app.example.test/account/2fa");
  expect((await proxy(request("/account/2fa"))).headers.get("x-middleware-next")).toBe("1");
  expect((await proxy(request("/account/2fa-bypass"))).headers.get("location")).toBe("https://app.example.test/account/2fa");
});

it("lets an enrolled and verified session navigate normally", async () => {
  fetchMock.mockResolvedValue(Response.json({ user: { requires2fa: true, totpEnabled: true, verificationRequired: false } }));
  const { proxy } = await import("./proxy");
  expect((await proxy(request("/reverse-proxy"))).headers.get("x-middleware-next")).toBe("1");
});

it.each([undefined, "false", null])("rejects a malformed verification flag (%s) without granting navigation", async flag => {
  fetchMock.mockResolvedValue(Response.json({ user: { requires2fa: true, totpEnabled: true, verificationRequired: flag } }));
  const { proxy } = await import("./proxy");
  expect((await proxy(request())).status).toBe(503);
});


it.each([undefined, "", "   "])("keeps same-origin framing by default (%s) on normal and failure responses", async value => {
  vi.stubEnv("NWUI_FRAME_ANCESTORS", value);
  const { proxy } = await import("./proxy");
  for (const response of [Response.json({ user: { requires2fa: false, totpEnabled: false, verificationRequired: false } }), new Response(null, { status: 401 }), new Response(null, { status: 503 })]) {
    fetchMock.mockResolvedValueOnce(response);
    const result = await proxy(request());
    expect(result.headers.get("content-security-policy")).toBe("frame-ancestors 'self'");
    expect(result.headers.get("x-frame-options")).toBe("SAMEORIGIN");
  }
});
it("reads valid additional origins per request and removes X-Frame-Options", async () => {
  const { proxy } = await import("./proxy");
  fetchMock.mockImplementation(() => Promise.resolve(Response.json({ user: { requires2fa: false, totpEnabled: false, verificationRequired: false } })));
  vi.stubEnv("NWUI_FRAME_ANCESTORS", "https://portal.example.test http://localhost:8790 https://portal.example.test");
  const result = await proxy(request());
  expect(result.headers.get("content-security-policy")).toBe("frame-ancestors 'self' https://portal.example.test http://localhost:8790");
  expect(result.headers.has("x-frame-options")).toBe(false);
  vi.stubEnv("NWUI_FRAME_ANCESTORS", "");
  expect((await proxy(request())).headers.get("x-frame-options")).toBe("SAMEORIGIN");
});
it.each([
  "*", "'none'", "https://*.example.test", "https://ok.test/path", "https://ok.test/",
  "https://user:pass@ok.test", "https://ok.test?q=1", "https://ok.test#x",
  "javascript:alert(1)", "ftp://ok.test", "https://ok.test;report-uri", "https://ok.test https://bad.test/path",
  "http://[::1]:8790", "https://portal.example.test http://[::1]:8790",
  "https://@ok.test", "https://..", "https://-bad.test", "https://bad-.test", "https://a..test",
  "https://ok.test:99999", "https://ok.test\\evil",
])("defaults safely and warns once for invalid origin configuration %s", async value => {
  vi.stubEnv("NWUI_FRAME_ANCESTORS", value);
  const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
  const { proxy } = await import("./proxy");
  fetchMock.mockResolvedValue(new Response(null, { status: 401 }));
  for (let i = 0; i < 2; i++) {
    const result = await proxy(request());
    expect(result.headers.get("content-security-policy")).toBe("frame-ancestors 'self'");
    expect(result.headers.get("x-frame-options")).toBe("SAMEORIGIN");
  }
  expect(warn).toHaveBeenCalledOnce();
  expect(warn.mock.calls[0]?.join(" ")).not.toContain(value);
});
it.each(["/login", "/login/2fa", "/login/2fa/backup", "/setup", "/login-unrelated", "/setup-unrelated"])("frames public HTML %s without an authenticated session lookup", async path => {
  const { proxy } = await import("./proxy");
  const result = await proxy(request(path));
  expect(result.headers.get("x-middleware-next")).toBe("1");
  expect(result.headers.get("content-security-policy")).toBe("frame-ancestors 'self'");
  expect(fetchMock).not.toHaveBeenCalled();
});
it("matches every HTML page and unknown-page document while excluding API/static assets", async () => {
  const { config } = await import("./proxy");
  const pages = readdirSync(new URL("./app", import.meta.url), { recursive: true })
    .filter(file => String(file).endsWith("page.tsx"))
    .map(file => "/" + String(file).split("/").filter(part => !part.startsWith("(") && part !== "page.tsx").map(part => part.startsWith("[") ? "fixture" : part).join("/"));
  expect(pages.length).toBeGreaterThan(40);
  for (const path of [...pages, "/missing-page", "/login-unrelated"]) {
    expect(unstable_doesMiddlewareMatch({ config, url: `https://app.test${path}` }), path).toBe(true);
  }
  // Next normalizes page-data URLs to the protected document route, not a static asset.
  expect(unstable_doesMiddlewareMatch({ config, url: "https://app.test/_next/data/build/page.json" })).toBe(true);
  for (const path of ["/api/ui/v1/session", "/api/auth/callback/proof", "/_next/static/chunk.js", "/_next/image", "/favicon.ico", "/icon.svg"]) {
    expect(unstable_doesMiddlewareMatch({ config, url: `https://app.test${path}` }), path).toBe(false);
  }
});

it("accepts and normalizes explicit ports, IPv4 and hostname case", async () => {
  vi.stubEnv("NWUI_FRAME_ANCESTORS", "HTTPS://Portal.Example.test:443 http://127.0.0.1:8790");
  const { proxy } = await import("./proxy");
  fetchMock.mockResolvedValue(new Response(null, { status: 401 }));
  const result = await proxy(request());
  expect(result.headers.get("content-security-policy")).toBe("frame-ancestors 'self' https://portal.example.test http://127.0.0.1:8790");
  expect(result.headers.has("x-frame-options")).toBe(false);
});
