import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";
const state = vi.hoisted(() => ({
  error: "마지막 로그인 방법입니다." as string | undefined,
}));
vi.mock("@/lib/use-api-action", () => ({
  useApiAction: () => async () => ({ ok: false }),
}));
vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  useActionState: () => [state, () => {}, false],
}));
import { LinkedLogins } from "./linked-logins";
it("keeps a 409 refusal under its own row and preserves unlink payload", () => {
  const html = renderToStaticMarkup(
    <LinkedLogins
      hasPassword
      rows={[
        {
          providerId: "oidc",
          name: "회사",
          linkedAt: new Date("2026-11-01T00:00:00Z"),
        },
      ]}
    />,
  );
  expect(html).toContain('name="providerId" value="oidc"');
  expect(html).toMatch(
    /<li[^>]*>[\s\S]*<form[\s\S]*<\/form>[\s\S]*마지막 로그인 방법입니다\.[\s\S]*<\/li>/,
  );
});
it("shows no providers and a last-method explanation without an unlink form", () => {
  expect(
    renderToStaticMarkup(<LinkedLogins hasPassword rows={[]} />),
  ).toContain("관리자가 설정한 외부 로그인이 없습니다.");
  state.error = undefined;
  const html = renderToStaticMarkup(
    <LinkedLogins
      hasPassword={false}
      rows={[
        {
          providerId: "oidc",
          name: "회사",
          linkedAt: new Date("2026-11-01T00:00:00Z"),
        },
      ]}
    />,
  );
  expect(html).toContain("마지막 남은 로그인 방법이라 해제할 수 없습니다.");
  expect(html).not.toContain("<form");
});
