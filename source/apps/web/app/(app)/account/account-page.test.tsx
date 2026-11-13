import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

/**
 * `/account` 의 「액세스 토큰」 절 — **그 절이 하는 사실 주장**만 본다.
 *
 * 앞선 문구는 「API 와 CLI 가 내 계정으로 접속할 때 쓰는 토큰」이었는데, 이
 * 버전에는 사용자 토큰을 **받는 곳이 하나도 없다**: `access_tokens` 를 인증에 쓰는
 * 자리는 `app/api/v1/pair/route.ts` 하나이고 거기는
 * `checkToken(row, { kind: "server", … })` 로 서버 토큰만 받는다(스펙 §3·§9-2).
 * 그 문구대로 믿은 사용자는 토큰을 발급받아 보관하고 어디에도 쓰지 못하며,
 * 거절 메시지조차 받지 못한다 — 받아 주는 엔드포인트 자체가 없으므로.
 *
 * **이웃 컴포넌트는 그리지 않는다.** 이 검사가 보는 것은 문단 하나이고, 진짜를
 * 그리면 `@repo/ui` 와 서버 액션 그래프를 통째로 끌고 온다. 페이지 자신은
 * **진짜**를 부른다 — 문구가 페이지에 있기 때문이다.
 */

const read = vi.fn();
vi.mock("@/lib/api-page", () => ({ readPageApi: read }));

vi.mock("./change-password-form", () => ({ ChangePasswordForm: () => null }));
vi.mock("./linked-logins", () => ({ LinkedLogins: () => null }));

const { default: AccountPage } = await import("./page");

/**
 * `<h2>{heading}</h2>` **바로 뒤**의 안내 문단 하나.
 *
 * **문단 단위로 잘라내는 것이 요점이다.** 화면 전체를 `toContain` 으로 훑으면,
 * 이 기능에는 글자까지 같은 문장이 다른 화면(`/account/tokens` 의 안내)에도 있어
 * 「이 문단이 통째로 없어졌다」를 못 본다. 잘라낸 문단을 **통째로** 비교하면 한
 * 문장이 빠지거나 뒤바뀌어도 그 자리에서 죽는다.
 */

/**
 * `<h2>{heading}</h2>` 옆 정보 아이콘의 설명(`role="tooltip"`) 하나. 절의 설명문은
 * 2026-11-02 부터 보이는 문단이 아니라 제목 옆 정보 아이콘에 있다(DESIGN.md 「페이지 설명」).
 */
function sectionTip(html: string, heading: string): string {
  const after = html.split(`${heading}</h2>`)[1] ?? "";
  return (
    /<div[^>]*role="tooltip"[^>]*>([\s\S]*?)<\/div>/.exec(after)?.[1] ?? ""
  )
    .replace(/<[^>]*>/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

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

describe("/account 의 액세스 토큰 절", () => {
  it("요청한 액세스 토큰 설명을 표시한다", async () => {
    const html = await render();

    expect(sectionTip(html, "액세스 토큰")).toBe(
      "외부 서비스에서 이용 가능한 엑세스 토큰을 관리합니다",
    );
  });

  /**
   * **음성 대조군.** 위 검사가 실패했을 때 「문구가 틀렸다」와 「잘라내기가 아무것도
   * 못 찾았다」를 구별한다 — 후자면 절을 통째로 지워도 빈 문자열끼리 맞아떨어지는
   * 검사를 짤 수 있고, 그런 검사는 아무것도 붙들지 않는다.
   */
  it("문단 잘라내기가 실제로 그 절의 문단을 집는다", async () => {
    const html = await render();

    // 상태는 보이는 문단에, 설명은 정보 아이콘에 있다 — 둘 다 그 절의 것을 집는다.
    expect(
      html.split("2단계 인증 (2FA)</h2>")[1]?.split("</section>")[0],
    ).toContain("상태");
    expect(sectionTip(html, "2단계 인증 (2FA)")).toContain("6자리 코드");
  });

  it("토큰 관리 화면으로 가는 길이 그 절에 있다", async () => {
    const html = await render();

    expect([...html.matchAll(/href="([^"]*)"/g)].map((m) => m[1])).toContain(
      "/account/tokens",
    );
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
