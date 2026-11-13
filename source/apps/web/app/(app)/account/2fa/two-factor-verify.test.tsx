import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { AccountState } from "../form-types";

const mocks = vi.hoisted(() => ({ actions: [] as Array<(previous: AccountState, form: FormData) => Promise<AccountState>>, state: { ok: false } as AccountState, pending: false, submit: vi.fn(), push: vi.fn(), refresh: vi.fn(), assign: vi.fn() }));
vi.mock("react", async original => ({
  ...await original<typeof import("react")>(),
  useActionState: (action: (previous: AccountState, form: FormData) => Promise<AccountState>) => {
    mocks.actions.push(action);
    return [mocks.state, () => undefined, mocks.pending];
  },
}));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: mocks.push, refresh: mocks.refresh }) }));
vi.mock("@/lib/api-transport", () => ({ submitApiForm: mocks.submit }));
import { TwoFactorVerify } from "./two-factor-forms";

beforeEach(() => { vi.clearAllMocks(); mocks.actions.length = 0; mocks.state = { ok: false }; mocks.pending = false; vi.stubGlobal("window", { location: { assign: mocks.assign } }); });
afterEach(() => vi.unstubAllGlobals());

it.each(["123456", "abcd-efgh-1234"])("sends TOTP or backup code %s unchanged through the verification API", async code => {
  const html = renderToStaticMarkup(<TwoFactorVerify />);
  expect(html).toContain('name="code"');
  expect(html).toContain('autoComplete="one-time-code"');
  expect(html).not.toContain('inputMode="numeric"');
  const form = new FormData(); form.set("code", code);
  mocks.submit.mockResolvedValue({ result: { ok: true }, redirectTo: "/", refresh: true });
  await mocks.actions[0]!({ ok: false }, form);
  expect(mocks.submit).toHaveBeenCalledExactlyOnceWith({ id: "account.verifyTwoFactor", args: [] }, form);
  expect(form.get("code")).toBe(code);
  expect(mocks.assign).toHaveBeenCalledExactlyOnceWith("/");
  expect(mocks.push).not.toHaveBeenCalled();
  expect(mocks.refresh).not.toHaveBeenCalled();
});

it("shows verification failures accessibly and keeps the action pending until completion", () => {
  mocks.state = { ok: false, error: "코드를 확인하세요." };
  mocks.pending = true;
  const html = renderToStaticMarkup(<TwoFactorVerify />);
  expect(html).toContain('role="alert"');
  expect(html).toContain("코드를 확인하세요.");
  expect(html).toContain('aria-invalid="true"');
  expect(html).toContain('disabled=""');
  expect(html).toContain("확인 중...");
});

import { TwoFactorManagement } from "./two-factor-management";
it("directs an external-only account to the administrator reset", () => {
  const html = renderToStaticMarkup(<TwoFactorManagement hasPassword={false} />);
  expect(html).toContain("관리자에게 2FA 초기화를 요청하세요.");
});
