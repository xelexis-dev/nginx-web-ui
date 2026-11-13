import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

// The second sign-in step's behaviour: who reaches it, what it asks for, which command each form
// sends, and which message shows for a wrong code. The API holds the pending sign-in; this page only
// asks whether one exists (AuthOptions.twoFactorPending) and never sees the challenge itself.
type FormAction = (state: { ok: boolean }, form: FormData) => Promise<unknown>;
const mocks = vi.hoisted(() => ({
  readAuthOptions: vi.fn(),
  redirect: vi.fn((path: string) => {
    throw new Error(`redirect:${path}`);
  }),
  actions: [] as FormAction[],
  submit: vi.fn(),
}));
vi.mock("next/navigation", () => ({ redirect: mocks.redirect, useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }) }));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<typeof import("@/lib/api-client")>()),
  readAuthOptions: mocks.readAuthOptions,
}));
// Capture each form's browser callback so a test can submit it; the transport records the command.
vi.mock("react", async (importOriginal) => ({
  ...(await importOriginal<typeof import("react")>()),
  useActionState: (action: FormAction, initial: { ok: boolean }) => {
    mocks.actions.push(action);
    return [initial, () => undefined, false];
  },
}));
vi.mock("@/lib/api-transport", () => ({ submitApiForm: mocks.submit }));

import { ApiRequestError } from "@/lib/api-client";
import TwoFactorPage from "./page";

const pending = { firstRun: false, authenticated: false, providers: [], twoFactorPending: true };
const render = async (params: { error?: string } = {}) => renderToStaticMarkup(await TwoFactorPage({ searchParams: Promise.resolve(params) }));
const tag = (html: string, pattern: RegExp) => html.match(pattern)?.[0] ?? "";
const input = (html: string, name: string) => tag(html, new RegExp(`<input[^>]*name="${name}"[^>]*>`));

beforeEach(() => {
  mocks.actions.length = 0;
  vi.clearAllMocks();
  mocks.submit.mockResolvedValue({ result: { ok: false, error: "stop" } });
});

describe("who reaches the second step", () => {
  it("an API that cannot be reached says so instead of showing a form", async () => {
    mocks.readAuthOptions.mockRejectedValue(new ApiRequestError(503, "api_unavailable", "API 서버에 연결하지 못했습니다."));
    const html = await render();
    expect(mocks.redirect).not.toHaveBeenCalled();
    expect(html).not.toMatch(/name="totp"/);
  });

  it.each([
    [{ ...pending, firstRun: true }, "/setup"],
    [{ ...pending, authenticated: true }, "/"],
    [{ ...pending, twoFactorPending: false }, "/login?error=TwoFactorExpired"],
    [{ firstRun: false, authenticated: false, providers: [] }, "/login?error=TwoFactorExpired"],
  ])("%o goes to %s", async (options, path) => {
    mocks.readAuthOptions.mockResolvedValue(options);
    await expect(render({ error: "invalid" })).rejects.toThrow(`redirect:${path}`);
    expect(mocks.redirect).toHaveBeenCalledExactlyOnceWith(path);
  });

  it("a pending sign-in gets the code form", async () => {
    mocks.readAuthOptions.mockResolvedValue(pending);
    const html = await render();
    expect(mocks.redirect).not.toHaveBeenCalled();
    expect(html).toMatch(/<h1[^>]*>2단계 인증<\/h1>/);
  });
});

/** The browser's own reading of a `pattern` attribute: the whole value, v flag. */
const matchesPattern = (pattern: string, value: string) => new RegExp(`^(?:${pattern})$`, "v").test(value);
const attr = (tag: string, name: string) => new RegExp(`\\s${name}="([^"]*)"`, "i").exec(tag)?.[1];
const forms = (html: string) => html.match(/<form[\s\S]*?<\/form>/g) ?? [];

describe("the authenticator code form", () => {
  it("takes exactly six digits, as text so a leading zero stays", async () => {
    mocks.readAuthOptions.mockResolvedValue(pending);
    const html = await render();
    const code = input(html, "totp");
    expect(attr(code, "id")).toBe("totp");
    expect(attr(code, "type")).toBe("text");
    expect(attr(code, "inputMode")).toBe("numeric");
    expect(attr(code, "autoComplete")).toBe("one-time-code");
    expect(attr(code, "minLength")).toBe("6");
    expect(attr(code, "maxLength")).toBe("6");
    expect(code).toMatch(/\srequired/);
    expect(html).toMatch(/<label[^>]*for="totp"[^>]*>인증 앱의 6자리 코드/);
    // Nothing from the first step travels with it: no email or password on this page.
    expect(html).not.toMatch(/name="(email|password)"/);
    expect(html).toMatch(/>인증 확인</);
  });

  it.each([
    ["012345", true],
    ["000000", true],
    ["987654", true],
    ["12345", false],
    ["1234567", false],
    ["12a456", false],
    ["123 45", false],
    ["abcde-12345", false],
    ["１２３４５６", false],
  ])("the browser accepts %j: %s", async (value, ok) => {
    mocks.readAuthOptions.mockResolvedValue(pending);
    const pattern = attr(input(await render(), "totp"), "pattern");
    expect(pattern).toBeTruthy();
    expect(matchesPattern(pattern!, value)).toBe(ok);
  });

  it("sends the code as login.verifyTwoFactor, saying it is an authenticator code", async () => {
    mocks.readAuthOptions.mockResolvedValue(pending);
    const html = await render();
    const verify = forms(html)[0]!;
    expect(verify).toMatch(/<input[^>]*type="hidden"[^>]*name="method"[^>]*value="totp"|<input[^>]*name="method"[^>]*type="hidden"[^>]*value="totp"/);
    const form = new FormData();
    form.set("totp", "012345");
    form.set("method", "totp");
    await mocks.actions[0]!({ ok: false }, form);
    expect(mocks.submit).toHaveBeenCalledExactlyOnceWith({ id: "login.verifyTwoFactor", args: [] }, form);
  });

  it("a backup code is a link to its own screen, not a form", async () => {
    mocks.readAuthOptions.mockResolvedValue(pending);
    const html = await render();
    expect(html).toMatch(/<a[^>]*href="\/login\/2fa\/backup"[^>]*>백업 코드로 인증<\/a>/);
    for (const form of forms(html)) expect(form).not.toContain("백업 코드로 인증");
  });

  it("starting again is its own form, sending login.cancelTwoFactor without the code", async () => {
    mocks.readAuthOptions.mockResolvedValue(pending);
    const html = await render();
    expect(mocks.actions).toHaveLength(2);
    expect(forms(html)).toHaveLength(2);
    expect(forms(html)[1]).toMatch(/>처음부터 다시 로그인</);
    expect(forms(html)[1]).not.toMatch(/name="(totp|method)"/);
    const form = new FormData();
    await mocks.actions[1]!({ ok: false }, form);
    expect(mocks.submit).toHaveBeenCalledExactlyOnceWith({ id: "login.cancelTwoFactor", args: [] }, form);
  });
});

describe("after a wrong code", () => {
  it("says the code was wrong, tied to the field, and stays on this step", async () => {
    mocks.readAuthOptions.mockResolvedValue(pending);
    const html = await render({ error: "invalid" });
    const alert = tag(html, /<[a-z]+[^>]*role="alert"[^>]*>[^<]*코드가 올바르지 않습니다\. 인증 앱의 6자리 코드를 다시 입력하세요\./);
    expect(alert).not.toBe("");
    const alertId = alert.match(/id="([^"]+)"/)?.[1];
    expect(alertId).toBeTruthy();
    expect(input(html, "totp")).toMatch(/aria-invalid="true"/);
    expect(input(html, "totp")).toMatch(new RegExp(`aria-describedby="[^"]*${alertId}`));
    // A wrong code is not reported as a wrong password.
    expect(html).not.toContain("이메일 또는 비밀번호가 올바르지 않습니다.");
  });

  it.each([{}, { error: "something-else" }])("no wrong code, no message: %o", async (params) => {
    mocks.readAuthOptions.mockResolvedValue(pending);
    const html = await render(params);
    expect(html).not.toMatch(/role="alert"/);
    expect(input(html, "totp")).not.toMatch(/aria-invalid/);
  });
});
