import { beforeEach, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ read: vi.fn(), redirect: vi.fn((path: string) => { throw new Error(path); }) }));
vi.mock("./api-page", () => ({ readPageApi: mocks.read }));
vi.mock("next/navigation", () => ({ redirect: mocks.redirect }));
import { readAdminSession } from "./admin-api";
beforeEach(() => vi.clearAllMocks());

it("an enrolled admin must complete session verification before admin navigation", async () => {
  mocks.read.mockResolvedValue({ user: { role: "admin", requires2fa: true, totpEnabled: true, verificationRequired: true } });
  await expect(readAdminSession()).rejects.toThrow("/account/2fa");
});

it("an enrolled verified admin keeps access to admin navigation", async () => {
  const session = { user: { role: "admin", requires2fa: true, totpEnabled: true, verificationRequired: false } };
  mocks.read.mockResolvedValue(session);
  expect(await readAdminSession()).toBe(session);
  expect(mocks.redirect).not.toHaveBeenCalled();
});
