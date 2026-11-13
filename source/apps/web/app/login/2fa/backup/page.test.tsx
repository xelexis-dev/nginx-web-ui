import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

// The second sign-in step with a backup code (/login/2fa/backup): the same guard as /login/2fa, one
// backup code field, the command that says it is a backup code, and the way back to the authenticator
// code. The API holds the pending sign-in; this page only asks whether one exists.
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
import BackupCodePage from "./page";

const pending = { firstRun: false, authenticated: false, providers: [], twoFactorPending: true };
const render = async (params: { error?: string } = {}) => renderToStaticMarkup(await BackupCodePage({ searchParams: Promise.resolve(params) }));
const tag = (html: string, pattern: RegExp) => html.match(pattern)?.[0] ?? "";
const input = (html: string, name: string) => tag(html, new RegExp(`<input[^>]*name="${name}"[^>]*>`));

beforeEach(() => {
  mocks.actions.length = 0;
  vi.clearAllMocks();
  mocks.submit.mockResolvedValue({ result: { ok: false, error: "stop" } });
});

const matchesPattern = (pattern: string, value: string) => new RegExp(`^(?:${pattern})$`, "v").test(value);
const attr = (tag: string, name: string) => new RegExp(`\\s${name}="([^"]*)"`, "i").exec(tag)?.[1];
const forms = (html: string) => html.match(/<form[\s\S]*?<\/form>/g) ?? [];

describe("who reaches the backup code step", () => {
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

  it("a pending sign-in gets the backup code form", async () => {
    mocks.readAuthOptions.mockResolvedValue(pending);
    const html = await render();
    expect(mocks.redirect).not.toHaveBeenCalled();
    expect(html).toMatch(/<h1[^>]*>백업 코드로 인증<\/h1>/);
  });
});

describe("the backup code form", () => {
  it("takes one backup code as text, not a one-time code", async () => {
    mocks.readAuthOptions.mockResolvedValue(pending);
    const html = await render();
    const code = input(html, "totp");
    expect(attr(code, "id")).toBe("backup-code");
    expect(attr(code, "type")).toBe("text");
    expect(attr(code, "inputMode")).toBe("text");
    expect(attr(code, "autoComplete")).toBe("off");
    expect(code).toMatch(/\srequired/);
    expect(attr(code, "maxLength")).toBeUndefined();
    expect(html).toMatch(/<label[^>]*for="backup-code"[^>]*>백업 코드/);
    expect(html).not.toMatch(/name="(email|password)"/);
    expect(html).toMatch(/>인증 확인</);
  });

  it.each([
    ["abcde-12345", true],
    ["abcde12345", true],
    ["ABCDE-12345", true],
    [" abcde-12345 ", true],
    ["012345", false],
    ["abcde-1234", false],
    ["ghijk-12345", false],
    ["abcde--12345", false],
  ])("the browser accepts %j: %s", async (value, ok) => {
    mocks.readAuthOptions.mockResolvedValue(pending);
    const pattern = attr(input(await render(), "totp"), "pattern");
    expect(pattern).toBeTruthy();
    expect(matchesPattern(pattern!, value)).toBe(ok);
  });

  it("sends the code as login.verifyTwoFactor, saying it is a backup code", async () => {
    mocks.readAuthOptions.mockResolvedValue(pending);
    const html = await render();
    const verify = forms(html)[0]!;
    expect(verify).toMatch(/<input[^>]*type="hidden"[^>]*name="method"[^>]*value="backup"|<input[^>]*name="method"[^>]*type="hidden"[^>]*value="backup"/);
    const form = new FormData();
    form.set("totp", "abcde-12345");
    form.set("method", "backup");
    await mocks.actions[0]!({ ok: false }, form);
    expect(mocks.submit).toHaveBeenCalledExactlyOnceWith({ id: "login.verifyTwoFactor", args: [] }, form);
  });

  it("goes back to the authenticator code by a link, not a form", async () => {
    mocks.readAuthOptions.mockResolvedValue(pending);
    const html = await render();
    expect(html).toMatch(/<a[^>]*href="\/login\/2fa"[^>]*>인증 앱 코드로 인증<\/a>/);
    for (const form of forms(html)) expect(form).not.toContain("인증 앱 코드로 인증");
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

describe("after a wrong backup code", () => {
  it("says the backup code was wrong, tied to the field, and stays on this step", async () => {
    mocks.readAuthOptions.mockResolvedValue(pending);
    const html = await render({ error: "invalid" });
    const alert = tag(html, /<[a-z]+[^>]*role="alert"[^>]*>[^<]*백업 코드가 올바르지 않습니다\./);
    expect(alert).not.toBe("");
    const alertId = alert.match(/id="([^"]+)"/)?.[1];
    expect(alertId).toBeTruthy();
    expect(input(html, "totp")).toMatch(/aria-invalid="true"/);
    expect(input(html, "totp")).toMatch(new RegExp(`aria-describedby="[^"]*${alertId}`));
    expect(html).not.toContain("이메일 또는 비밀번호가 올바르지 않습니다.");
    expect(html).not.toContain("인증 앱의 6자리 코드를 다시 입력하세요.");
    expect(html).toContain("백업 코드도 없다면 관리자에게 2FA 초기화를 요청하세요.");
  });

  it.each([{}, { error: "something-else" }])("no wrong code, no message: %o", async (params) => {
    mocks.readAuthOptions.mockResolvedValue(pending);
    const html = await render(params);
    expect(html).not.toMatch(/role="alert"/);
    expect(input(html, "totp")).not.toMatch(/aria-invalid/);
  });
});
