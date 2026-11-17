import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

const read = vi.hoisted(() => vi.fn());
vi.mock("@/lib/api-page", () => ({ readPageApi: read }));
vi.mock("./two-factor-forms", () => ({
  TwoFactorDisable: () => <form data-disable="" />,
  TwoFactorEnroll: () => <form data-enroll="" />,
  TwoFactorVerify: () => <form data-verify="" />,
}));
import Page from "./page";

beforeEach(() => { read.mockReset(); });

it("shows real management choices, not the disable form, for a verified enrolled account", async () => {
  read.mockResolvedValueOnce({ enabled: true, verificationRequired: false }).mockResolvedValueOnce({ hasPassword: true });
  const html = renderToStaticMarkup(await Page());
  expect(html).toMatch(/<h1[^>]*>2FA 관리<\/h1>/);
  expect(html).toContain('href="/account/2fa/backup-codes/new"');
  expect(html).toContain('href="/account/2fa/disable"');
  expect(html).not.toContain("data-disable");
  expect(read.mock.calls).toEqual([["/api/ui/v1/account/2fa"], ["/api/ui/v1/account"]]);
});

it("keeps the verification gate ahead of management and never reads the protected account view", async () => {
  read.mockResolvedValueOnce({ enabled: true, verificationRequired: true });
  const html = renderToStaticMarkup(await Page());
  expect(html).toContain("data-verify");
  expect(html).not.toContain("backup-codes/new");
  expect(read).toHaveBeenCalledTimes(1);
});

it("lets OAuth-only accounts regenerate using an authenticator without offering a password-only disable form", async () => {
  read.mockResolvedValueOnce({ enabled: true, verificationRequired: false }).mockResolvedValueOnce({ hasPassword: false });
  const html = renderToStaticMarkup(await Page());
  expect(html).toContain('href="/account/2fa/backup-codes/new"');
  expect(html).not.toContain('href="/account/2fa/disable"');
  expect(html).toContain("관리자");
});

it("tells password accounts, not OAuth-only ones, that disabling removes every passkey", async () => {
  for (const hasPassword of [true, false]) {
    read.mockResolvedValueOnce({ enabled: true, verificationRequired: false }).mockResolvedValueOnce({ hasPassword });
    const html = renderToStaticMarkup(await Page());
    expect(html.includes("현재 비밀번호로 본인을 확인한 뒤 해제합니다. 해제하면 등록한 패스키도 모두 삭제되고, 이 계정의 API 토큰도 모두 폐기됩니다. 2FA가 필수인 계정은 해제 후 다시 설정해야 합니다.")).toBe(hasPassword);
    expect(html.includes('href="/account/2fa/disable"')).toBe(hasPassword);
  }
});
