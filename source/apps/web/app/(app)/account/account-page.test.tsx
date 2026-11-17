import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
vi.mock("@/lib/api-client", () => ({
  readSession: async () => ({ user: { role: "admin" } }),
}));

const read = vi.fn();
vi.mock("@/lib/api-page", () => ({ readPageApi: read }));

vi.mock("./change-password-form", () => ({ ChangePasswordForm: () => null }));
vi.mock("./linked-logins", () => ({ LinkedLogins: () => null }));

const { default: AccountPage } = await import("./page");

async function render(): Promise<string> {
  return renderToStaticMarkup(
    await AccountPage({ searchParams: Promise.resolve({}) }),
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  read.mockImplementation((path: string) =>
    path.endsWith("/passkeys")
      ? {
          passkeys: [],
          availability: { available: true, reason: null },
          currentOrigin: "https://account.example.test",
          hasPassword: true,
          canRemove: true,
        }
      : {
          email: "me@example.com",
          hasPassword: true,
          totpEnabled: false,
          logins: [],
        },
  );
});

describe("account composition", () => {
  it("renders hairline sections and the API access row", async () => {
    const html = await render();
    expect(html).toContain("비밀번호와 2단계 인증");
    expect(html).toContain(
      "API나 자동화 도구가 이 계정으로 작업할 때 씁니다. 계정의 현재 역할과 토큰에 고른 권한이 모두 허락하는 작업만 할 수 있습니다. 비밀번호·2단계 인증·패스키·토큰 관리 같은 계정 보안 작업은 토큰으로 할 수 없습니다. SMTP·웹훅 시험은 웹에서만 할 수 있습니다. 토큰을 쓰려면 2단계 인증을 등록해야 합니다.",
    );
    expect(html).toContain('href="/account/tokens"');
    expect(html).toContain('aria-label="2단계 인증 설정"');
    expect(html).toContain(">admin</span>");
  });
  it.each([
    [
      { removed: "1", linked: "1" },
      "패스키를 삭제했습니다. 이 계정의 다른 세션은 모두 로그아웃했습니다.",
    ],
    [
      { linked: "1", unlinked: "1" },
      "외부 로그인을 연결했습니다. 이제 로그인 화면의 그 버튼으로도 로그인할 수 있습니다.",
    ],
    [
      { unlinked: "1", error: "last-method" },
      "외부 로그인 연결을 해제했습니다.",
    ],
    [
      { error: "last-method" },
      "마지막 남은 로그인 방법은 해제할 수 없습니다. 먼저 다른 방법을 추가하세요.",
    ],
    [
      { error: "link-failed" },
      "외부 로그인을 연결하지 못했습니다. 다시 시도하세요.",
    ],
    [
      { error: "already-linked" },
      "이 외부 계정은 이미 다른 계정에 연결되어 있습니다.",
    ],
  ])("renders one redirect result with priority %o", async (query, text) => {
    const html = renderToStaticMarkup(
      await AccountPage({ searchParams: Promise.resolve(query) }),
    );
    expect(html).toContain(text);
    expect(html.split(text)).toHaveLength(2);
  });
});

vi.mock("next/navigation", () => ({
  redirect: (path: string) => {
    throw new Error(`redirect:${path}`);
  },
}));
const { default: PasswordPage } = await import("./password/page");
it("gives password-backed users a separate password page and account link", async () => {
  const html = await render();
  expect(html).toContain('href="/account/password"');
  expect(renderToStaticMarkup(await PasswordPage())).toContain("비밀번호 변경");
  expect(read).toHaveBeenLastCalledWith("/api/ui/v1/account");
});
it("keeps OAuth-only users out of the password route", async () => {
  read.mockResolvedValue({
    email: "oauth@example.test",
    hasPassword: false,
    totpEnabled: false,
    logins: [],
  });
  await expect(PasswordPage()).rejects.toThrow("redirect:/account");
});
