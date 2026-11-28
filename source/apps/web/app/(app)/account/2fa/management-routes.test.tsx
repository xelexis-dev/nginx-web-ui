import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { buttonClassName } from "@repo/ui";
const read = vi.hoisted(() => vi.fn());
vi.mock("@/lib/api-page", () => ({ readPageApi: read }));
vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }),
  redirect: (url: string) => {
    throw new Error(`redirect:${url}`);
  },
}));
vi.mock("./two-factor-forms", () => ({
  TwoFactorDisable: () => <form data-disable="" />,
}));
vi.mock("./regenerate-backup-codes-form", () => ({
  RegenerateBackupCodesForm: ({ hasPassword }: { hasPassword: boolean }) => (
    <form data-method={hasPassword ? "password" : "totp"} />
  ),
}));
import RegeneratePage from "./backup-codes/new/page";
import DisablePage from "./disable/page";
beforeEach(() => read.mockReset());

it.each([RegeneratePage, DisablePage])(
  "gates management forms when verification is incomplete",
  async (page) => {
    read.mockResolvedValueOnce({ enabled: true, verificationRequired: true });
    await expect(page()).rejects.toThrow("redirect:/account/2fa");
    expect(read).toHaveBeenCalledTimes(1);
  },
);
it.each([RegeneratePage, DisablePage])(
  "requires enrollment first",
  async (page) => {
    read.mockResolvedValueOnce({ enabled: false });
    await expect(page()).rejects.toThrow("redirect:/account/2fa");
  },
);
it.each([true, false])(
  "passes the real password capability to regeneration: %s",
  async (hasPassword) => {
    read
      .mockResolvedValueOnce({ enabled: true, verificationRequired: false })
      .mockResolvedValueOnce({ hasPassword });
    const html = renderToStaticMarkup(await RegeneratePage());
    expect(html).toContain(
      `data-method="${hasPassword ? "password" : "totp"}"`,
    );
    expect(html.match(/<header[\s\S]*?<\/header>/)?.[0]).not.toContain(
      'href="/account/2fa"',
    );
  },
);
it("keeps the existing disable form only on the explicitly named disable page", async () => {
  read
    .mockResolvedValueOnce({ enabled: true, verificationRequired: false })
    .mockResolvedValueOnce({ hasPassword: true });
  const html = renderToStaticMarkup(await DisablePage());
  expect(html).toMatch(/<h1[^>]*>2FA 해제<\/h1>/);
  expect(html).toContain("data-disable");
});
it("does not show an unusable password-only disable form to OAuth-only users", async () => {
  read
    .mockResolvedValueOnce({ enabled: true, verificationRequired: false })
    .mockResolvedValueOnce({ hasPassword: false });
  await expect(DisablePage()).rejects.toThrow("redirect:/account/2fa");
});

it("warns on the disable page that every registered passkey is removed", async () => {
  read
    .mockResolvedValueOnce({ enabled: true, verificationRequired: false })
    .mockResolvedValueOnce({ hasPassword: true });
  const html = renderToStaticMarkup(await DisablePage());
  const { TwoFactorDisable } =
    await vi.importActual<typeof import("./two-factor-forms")>(
      "./two-factor-forms",
    );
  expect(renderToStaticMarkup(<TwoFactorDisable />)).toContain(
    "해제하면 기존 인증 앱과 백업 코드로 인증할 수 없고, 등록한 패스키도 모두 삭제됩니다. 이 계정의 API 토큰도 모두 폐기됩니다. 현재 비밀번호를 확인합니다.",
  );
  expect(html).toContain("data-disable");
});

it("styles the 2FA disable link as a small outlined danger action", async () => {
  const { TwoFactorManagement } = await import("./two-factor-management");
  const html = renderToStaticMarkup(<TwoFactorManagement hasPassword />);
  const link = html.match(/<a[^>]*>2FA 해제<\/a>/)?.[0];
  expect(link).toContain('href="/account/2fa/disable"');
  expect(link).toContain(`class="${buttonClassName({ variant: "dangerOutline", size: "sm" })}"`);
});

it("uses shared setting rows for management and preserves passwordless restrictions", async () => {
  const { TwoFactorManagement } = await import("./two-factor-management");
  const html = renderToStaticMarkup(
    <TwoFactorManagement hasPassword={false} />,
  );
  expect(html.match(/data-wide=""/g)).toHaveLength(2);
  expect(html).toContain('href="/account/2fa/backup-codes/new"');
  expect(html).not.toContain('href="/account/2fa/disable"');
  expect(html).toContain("관리자에게 2FA 초기화를 요청하세요.");
});
