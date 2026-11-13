import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

// The sign-in page's behaviour, independent of its look: where it sends people, what it asks for,
// which command each form sends, and which message shows for which result.
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

import LoginPage from "./page";

const signedOut = { firstRun: false, authenticated: false, providers: [] };
const render = async (params: { error?: string; pending?: string } = {}) =>
  renderToStaticMarkup(await LoginPage({ searchParams: Promise.resolve(params) }));
/** The opening tag of the element with this attribute, e.g. input(name="email"). */
const tag = (html: string, pattern: RegExp) => html.match(pattern)?.[0] ?? "";
const input = (html: string, name: string) => tag(html, new RegExp(`<input[^>]*name="${name}"[^>]*>`));

beforeEach(() => {
  mocks.actions.length = 0;
  vi.clearAllMocks();
  mocks.submit.mockResolvedValue({ result: { ok: false, error: "stop" } });
});

describe("where the page sends people", () => {
  it("first run goes to /setup before anything else", async () => {
    mocks.readAuthOptions.mockResolvedValue({ firstRun: true, authenticated: true, providers: [] });
    await expect(render({ error: "CredentialsSignin" })).rejects.toThrow("redirect:/setup");
    expect(mocks.redirect).toHaveBeenCalledExactlyOnceWith("/setup");
  });

  it("a valid session goes to /", async () => {
    mocks.readAuthOptions.mockResolvedValue({ firstRun: false, authenticated: true, providers: [] });
    await expect(render()).rejects.toThrow("redirect:/");
    expect(mocks.redirect).toHaveBeenCalledExactlyOnceWith("/");
  });

  it("a password already accepted for a 2FA account goes on to /login/2fa", async () => {
    mocks.readAuthOptions.mockResolvedValue({ ...signedOut, twoFactorPending: true });
    await expect(render()).rejects.toThrow("redirect:/login/2fa");
    expect(mocks.redirect).toHaveBeenCalledExactlyOnceWith("/login/2fa");
  });

  it.each([
    [{ firstRun: true, authenticated: false, providers: [], twoFactorPending: true }, "/setup"],
    [{ firstRun: false, authenticated: true, providers: [], twoFactorPending: true }, "/"],
  ])("first run and a valid session still come before a pending second factor: %o", async (options, path) => {
    mocks.readAuthOptions.mockResolvedValue(options);
    await expect(render()).rejects.toThrow(`redirect:${path}`);
    expect(mocks.redirect).toHaveBeenCalledExactlyOnceWith(path);
  });

  it("a stale or revoked session gets the form, not a redirect", async () => {
    mocks.readAuthOptions.mockResolvedValue(signedOut);
    const html = await render();
    expect(mocks.redirect).not.toHaveBeenCalled();
    expect(html).toContain("이메일과 비밀번호로 로그인하세요.");
    expect(input(html, "email")).not.toBe("");
  });
});

describe("the sign-in form", () => {
  it("asks for email and password only, with the fields password managers expect", async () => {
    mocks.readAuthOptions.mockResolvedValue(signedOut);
    const html = await render();
    const email = input(html, "email");
    const password = input(html, "password");
    expect(email).toMatch(/type="email"/);
    expect(email).toMatch(/autoComplete="email"|autocomplete="email"/);
    expect(email).toMatch(/required/);
    expect(email).toMatch(/id="email"/);
    expect(password).toMatch(/type="password"/);
    expect(password).toMatch(/autoComplete="current-password"|autocomplete="current-password"/);
    expect(password).toMatch(/required/);
    expect(password).toMatch(/id="password"/);
    for (const id of ["email", "password"]) expect(html).toMatch(new RegExp(`<label[^>]*for="${id}"`));
    // The second factor has its own screen after the password (/login/2fa): nothing here asks for it.
    expect(input(html, "totp")).toBe("");
    expect(html).not.toMatch(/one-time-code/);
  });

  it("sends the credentials as login.loginAction", async () => {
    mocks.readAuthOptions.mockResolvedValue(signedOut);
    await render();
    const form = new FormData();
    form.set("email", "ops@example.test");
    await mocks.actions[0]!({ ok: false }, form);
    expect(mocks.submit).toHaveBeenCalledExactlyOnceWith({ id: "login.loginAction", args: [] }, form);
  });
});

describe("messages after a sign-in attempt", () => {
  it.each([[{ pending: "1" }], [{ error: "pending" }], [{ pending: "1", error: "CredentialsSignin" }], [{ pending: "1", error: "disabled" }]])(
    "pending approval wins over any error: %o",
    async (params) => {
      mocks.readAuthOptions.mockResolvedValue(signedOut);
      const html = await render(params);
      expect(html).toContain("계정이 만들어졌습니다.");
      expect(html).toContain("관리자가 승인한 뒤에");
      expect(html).not.toContain("이메일 또는 비밀번호가 올바르지 않습니다.");
      expect(html).not.toContain("비활성화된 계정입니다.");
    },
  );

  it.each(["CredentialsSignin", "AccessDenied", "OAuthCallbackError"])("%s shows the one generic credentials message, tied to the fields", async (error) => {
    mocks.readAuthOptions.mockResolvedValue(signedOut);
    const html = await render({ error });
    const alert = tag(html, /<[a-z]+[^>]*role="alert"[^>]*>[^<]*이메일 또는 비밀번호가 올바르지 않습니다\./);
    expect(alert).not.toBe("");
    const alertId = alert.match(/id="([^"]+)"/)?.[1];
    expect(alertId).toBeTruthy();
    for (const name of ["email", "password"]) {
      expect(input(html, name)).toMatch(/aria-invalid="true"/);
      expect(input(html, name)).toMatch(new RegExp(`aria-describedby="[^"]*${alertId}`));
    }
  });

  it("a disabled account says so, without marking the fields invalid", async () => {
    mocks.readAuthOptions.mockResolvedValue(signedOut);
    const html = await render({ error: "disabled" });
    expect(html).toMatch(/role="alert"[^>]*>비활성화된 계정입니다\. 관리자에게 문의하세요\./);
    expect(html).not.toContain("이메일 또는 비밀번호가 올바르지 않습니다.");
    for (const name of ["email", "password"]) expect(input(html, name)).not.toMatch(/aria-invalid/);
  });

  it("an expired or cancelled second factor says to start again, without blaming the password", async () => {
    mocks.readAuthOptions.mockResolvedValue(signedOut);
    const html = await render({ error: "TwoFactorExpired" });
    expect(html).toContain("로그인을 처음부터 다시 하세요.");
    expect(html).not.toContain("이메일 또는 비밀번호가 올바르지 않습니다.");
    expect(html).not.toMatch(/role="alert"/);
    for (const name of ["email", "password"]) expect(input(html, name)).not.toMatch(/aria-invalid/);
  });

  it("pending approval still wins over an expired second factor", async () => {
    mocks.readAuthOptions.mockResolvedValue(signedOut);
    const html = await render({ pending: "1", error: "TwoFactorExpired" });
    expect(html).toContain("계정이 만들어졌습니다.");
    expect(html).not.toContain("로그인을 처음부터 다시 하세요.");
  });

  it("no attempt, no message", async () => {
    mocks.readAuthOptions.mockResolvedValue(signedOut);
    const html = await render();
    expect(html).not.toMatch(/role="alert"/);
    expect(html).not.toContain("계정이 만들어졌습니다.");
  });
});

describe("sign-in providers (OIDC)", () => {
  it("none registered: no divider and no provider buttons", async () => {
    mocks.readAuthOptions.mockResolvedValue(signedOut);
    const html = await render();
    expect(html).not.toMatch(/>또는</);
    expect(html).not.toMatch(/name="providerId"/);
    expect(mocks.actions).toHaveLength(1);
  });

  it("each provider gets its own form sending login.oauthSignIn with its providerId, after the credentials form", async () => {
    mocks.readAuthOptions.mockResolvedValue({ ...signedOut, providers: [{ id: "kc", name: "Keycloak" }, { id: "sso", name: "사내 SSO" }] });
    const html = await render();
    expect(html).toMatch(/>또는</);
    expect(html).toMatch(/<input[^>]*type="hidden"[^>]*name="providerId"[^>]*value="kc"|<input[^>]*name="providerId"[^>]*type="hidden"[^>]*value="kc"|<input[^>]*type="hidden"[^>]*value="kc"[^>]*name="providerId"/);
    expect(html).toContain("Keycloak 로 로그인");
    expect(html).toContain("사내 SSO 로 로그인");
    // Reading and Tab order start with the credentials submit.
    expect(html.indexOf(">로그인<")).toBeGreaterThan(-1);
    expect(html.indexOf(">로그인<")).toBeLessThan(html.indexOf("Keycloak 로 로그인"));
    expect(mocks.actions).toHaveLength(3);
    const form = new FormData();
    form.set("providerId", "kc");
    await mocks.actions[1]!({ ok: false }, form);
    expect(mocks.submit).toHaveBeenCalledExactlyOnceWith({ id: "login.oauthSignIn", args: [] }, form);
  });
});

it("passkey errors leave password fields unmarked and show only the passkey failure line", async () => {
  mocks.readAuthOptions.mockResolvedValue({ ...signedOut, passkey: { available: true, reason: null } });
  const html = await render({ error: "passkey" });
  expect(html).toContain("패스키를 확인하지 못했습니다. 다시 시도해 주세요.");
  expect(html).not.toContain("이메일 또는 비밀번호가 올바르지 않습니다.");
  for (const name of ["email", "password"]) expect(input(html, name)).not.toMatch(/aria-invalid/);
});
