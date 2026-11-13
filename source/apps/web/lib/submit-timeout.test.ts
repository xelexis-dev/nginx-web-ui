import { expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { LONG_SUBMIT_MS, SUBMIT_TIMEOUT_MS, isProviderRedirectAction, submitApiForm, submitTimeoutMs } from "./api-transport";

// A save must not sit for five minutes with no word. Only the commands that really take minutes
// keep the long budget.
it("gives ordinary commands a short budget and long jobs the long one", () => {
  expect(submitTimeoutMs("reverse-proxy.createServer")).toBe(SUBMIT_TIMEOUT_MS);
  expect(submitTimeoutMs("admin.users.create")).toBe(SUBMIT_TIMEOUT_MS);
  expect(SUBMIT_TIMEOUT_MS).toBeLessThanOrEqual(60_000);
  for (const long of ["apply.applyConfig", "certificates.createCertificate", "certificates.renewCertificate", "nodes.pairNode"]) {
    expect(submitTimeoutMs(long)).toBe(LONG_SUBMIT_MS);
  }
  expect(LONG_SUBMIT_MS).toBeGreaterThanOrEqual(300_000);
});

// Only OIDC initiation may send the browser off this origin.
it("names the actions allowed to return an absolute URL", () => {
  expect(isProviderRedirectAction("login.oauthSignIn")).toBe(true);
  expect(isProviderRedirectAction("account.startLink")).toBe(true);
  for (const other of ["login.loginAction", "apply.applyConfig", "admin.users.create"]) {
    expect(isProviderRedirectAction(other)).toBe(false);
  }
});

// `AbortSignal.timeout` fires on a platform timer, so the mapping is what this pins: an aborted
// request must read as "no answer in time", not as "could not send".
it("tells the user the request timed out instead of a generic failure", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = vi.fn(() => Promise.reject(new DOMException("timed out", "TimeoutError"))) as unknown as typeof fetch;
  try {
    const result = await submitApiForm({ id: "admin.users.create", args: [] }, new FormData());
    const error = String((result.result as { error?: string } | undefined)?.error);
    expect(error).toMatch(/응답이 없어/);
    expect(error).not.toMatch(/전달하지 못했습니다/);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

it("still reports a transport failure plainly when the request never left", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = vi.fn(() => Promise.reject(new TypeError("Failed to fetch"))) as unknown as typeof fetch;
  try {
    const result = await submitApiForm({ id: "admin.users.create", args: [] }, new FormData());
    expect(String((result.result as { error?: string } | undefined)?.error)).toMatch(/전달하지 못했습니다/);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

it.each(["node.applyNodeConfig", "node.rollbackNodeSnapshot", "node.renewNodeCertificate", "node.replaceNodeCertificate", "history.rollback"])("keeps %s within its server budget", (id) => {
  expect(submitTimeoutMs(id)).toBe(LONG_SUBMIT_MS);
});

it.each(["node.createNodeServer", "node.updateNodeCertificate", "node.deleteNodeCertificate", "node.applyNodeConfig.extra"])("keeps ordinary or unknown remote command %s short", (id) => {
  expect(submitTimeoutMs(id)).toBe(SUBMIT_TIMEOUT_MS);
});

it("gives the real webhook test more than its 34 second retry worst case",()=>{
  expect(submitTimeoutMs("admin.webhooks.test")).toBeGreaterThanOrEqual(40_000);
});
it("uses registered API IDs for the timeout examples",()=>{
  const spec=JSON.parse(readFileSync(new URL("../../../packages/contracts/openapi.json",import.meta.url),"utf8"));
  for(const id of ["reverse-proxy.createServer","certificates.createCertificate","certificates.renewCertificate"]){
    expect(spec.paths[`/api/ui/v1/forms/${id}`]).toBeDefined();
  }
});
