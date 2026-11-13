import { afterEach, beforeEach, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ submit: vi.fn(), push: vi.fn(), refresh: vi.fn(), assign: vi.fn() }));
vi.mock("react", () => ({ useCallback: (callback: unknown) => callback }));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: mocks.push, refresh: mocks.refresh }) }));
vi.mock("./api-transport", async (original) => ({ ...await original<typeof import("./api-transport")>(), submitApiForm: mocks.submit }));
import { useApiAction } from "./use-api-action";

beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("window", { location: { assign: mocks.assign } });
});
afterEach(() => vi.unstubAllGlobals());

it.each([
  ["login.loginAction", "/"],
  ["login.loginAction", "/login/2fa"],
  ["login.verifyTwoFactor", "/"],
  ["login.verifyTwoFactor", "/login/2fa?error=invalid"],
  ["login.verifyTwoFactor", "/login?error=TwoFactorExpired"],
  ["login.cancelTwoFactor", "/login"],
  ["account.verifyTwoFactor", "/"],
  ["account.finishTwoFactorSetup", "/account"],
  ["account.changePassword", "/login"],
  ["account.disableTwoFactor", "/account/2fa"],
  ["session.signOut", "/login"],
  ["login.passkeyFinish", "/"],
  ["account.passkeys.finish", "/account/passkeys/key?added=1"],
  ["account.passkeys.remove", "/account?removed=1"],
  ["admin.users.passkeys.remove", "/admin/users/u1"],
])("reloads the document after %s so navigation reads the changed session cookie", async (id, redirectTo) => {
  mocks.submit.mockResolvedValue({ redirectTo, refresh: true });
  const state = { ok: false };
  const action = useApiAction<typeof state>({ id, args: [] });
  expect(await action(state, new FormData())).toBe(state);
  expect(mocks.assign).toHaveBeenCalledExactlyOnceWith(redirectTo);
  expect(mocks.push).not.toHaveBeenCalled();
  expect(mocks.refresh).not.toHaveBeenCalled();
});

it("keeps a rejected verification on the current form without navigating", async () => {
  const result = { ok: false, error: "코드가 올바르지 않습니다." };
  mocks.submit.mockResolvedValue({ result });
  const action = useApiAction({ id: "account.verifyTwoFactor", args: [] });
  expect(await action({ ok: false }, new FormData())).toEqual(result);
  expect(mocks.assign).not.toHaveBeenCalled();
  expect(mocks.push).not.toHaveBeenCalled();
  expect(mocks.refresh).not.toHaveBeenCalled();
});

it("preserves enrollment backup codes on the current page until setup is finished", async () => {
  const result = { ok: true, backupCodes: ["synthetic-backup"] };
  mocks.submit.mockResolvedValue({ result });
  const action = useApiAction({ id: "account.enableTwoFactor", args: [] });
  expect(await action({ ok: false }, new FormData())).toEqual(result);
  expect(mocks.assign).not.toHaveBeenCalled();
  expect(mocks.push).not.toHaveBeenCalled();
  expect(mocks.refresh).not.toHaveBeenCalled();
});

it("carries pairing attempt state while replacing an injected counter", async () => {
  mocks.submit.mockResolvedValue({result:{ok:false,attempt:3}});
  const form=new FormData();form.set("_attempt","999");
  const action=useApiAction({id:"nodes.pairNode",args:[]});
  await action({ok:false,attempt:2},form);
  expect((mocks.submit.mock.calls[0]![1] as FormData).get("_attempt")).toBe("2");
});

it("starts OIDC with full document navigation so the provider receives no RSC request", async () => {
  const assign = vi.fn();
  vi.stubGlobal("window", { location: { assign } });
  mocks.submit.mockResolvedValue({ redirectTo: "https://identity.example.test/authorize?state=random" });
  const action = useApiAction({ id: "login.oauthSignIn", args: [] });
  await action({ ok: false }, new FormData());
  expect(assign).toHaveBeenCalledWith("https://identity.example.test/authorize?state=random");
  expect(mocks.push).not.toHaveBeenCalled();
  expect(mocks.refresh).not.toHaveBeenCalled();
  vi.unstubAllGlobals();
});

it("preserves an explicit null state instead of redisplaying an old success", async () => {
  mocks.submit.mockResolvedValue({ result: null });
  const action = useApiAction<{ ok: boolean } | null>({ id: "config.apply", args: [] });
  expect(await action({ ok: true }, new FormData())).toBeNull();
});

it("refreshes server-rendered data after a mutation and keeps field errors", async () => {
  const result = { ok: false, fieldErrors: { name: "필수 항목입니다." } };
  mocks.submit.mockResolvedValue({ result, refresh: true });
  const action = useApiAction({ id: "nodes.update", args: ["n1"] });
  expect(await action({ ok: true }, new FormData())).toEqual(result);
  expect(mocks.refresh).toHaveBeenCalledOnce();
  expect(mocks.push).not.toHaveBeenCalled();
});

it("navigates on successful commands without requiring a returned form state", async () => {
  mocks.submit.mockResolvedValue({ redirectTo: "/certificates/c1" });
  const state = { ok: false };
  const action = useApiAction<typeof state>({ id: "certificates.create", args: [] });
  expect(await action(state, new FormData())).toBe(state);
  expect(mocks.push).toHaveBeenCalledWith("/certificates/c1");
  expect(mocks.refresh).toHaveBeenCalledOnce();
  expect(mocks.assign).not.toHaveBeenCalled();
});

// Defence in depth: the transport already refuses an absolute URL from any other command, but the
// hook must not send the browser off-origin just because a response looked like a URL.
it("leaves the origin only for the OIDC commands", async () => {
  mocks.submit.mockResolvedValue({ redirectTo: "https://idp.example.test/authorize" });
  await useApiAction({ id: "admin.users.create", args: [] })({} as never, new FormData());
  expect(mocks.assign).not.toHaveBeenCalled();
  expect(mocks.push).not.toHaveBeenCalledWith("https://idp.example.test/authorize");

  await useApiAction({ id: "login.oauthSignIn", args: [] })({} as never, new FormData());
  expect(mocks.assign).toHaveBeenCalledWith("https://idp.example.test/authorize");
});