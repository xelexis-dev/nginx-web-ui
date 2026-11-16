import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
vi.mock("./api-client", () => ({
  readSession: async () => ({ user: { role: "admin" } }),
}));

const read = vi.hoisted(() => vi.fn());
const qr = vi.hoisted(() =>
  vi.fn().mockResolvedValue("data:image/png;base64,test"),
);
vi.mock("./api-page", () => ({ readPageApi: read }));
vi.mock("qrcode", () => ({ default: { toDataURL: qr } }));
vi.mock("@repo/db", () => {
  throw new Error("SSR must not access the database");
});
vi.mock("@repo/auth", () => {
  throw new Error("SSR must not own authentication");
});
vi.mock("../app/(app)/account/change-password-form", () => ({
  ChangePasswordForm: () => <div>Password form</div>,
}));
vi.mock("../app/(app)/account/linked-logins", () => ({
  LinkedLogins: () => null,
}));
vi.mock("../app/(app)/account/2fa/two-factor-forms", () => ({
  TwoFactorDisable: () => <div>Disable form</div>,
  TwoFactorEnroll: ({ secret }: { secret: string }) => (
    <div>Enroll {secret}</div>
  ),
  TwoFactorVerify: () => <div>Verify form</div>,
}));
vi.mock("../app/admin/settings/general/base-url-form", () => ({
  BaseUrlForm: ({ defaultValue }: { defaultValue: string }) => (
    <div>{defaultValue}</div>
  ),
}));
vi.mock("../app/admin/settings/general/metrics-form", () => ({
  MetricsForm: ({ defaultRetentionDays }: { defaultRetentionDays: string }) => (
    <div>Retention {defaultRetentionDays}</div>
  ),
}));

beforeEach(() => {
  read.mockReset();
  qr.mockClear();
});
describe("account and settings SSR API boundary", () => {
  it("renders the authenticated API account without reading secrets or DB", async () => {
    read.mockImplementation((path: string) =>
      path.endsWith("/passkeys")
        ? {
            passkeys: [],
            availability: { available: false, reason: "insecure_origin" },
            currentOrigin: null,
            hasPassword: false,
            canRemove: false,
          }
        : {
            email: "member@example.test",
            hasPassword: false,
            totpEnabled: true,
            logins: [],
          },
    );
    const { default: Page } = await import("../app/(app)/account/page");
    const html = renderToStaticMarkup(
      await Page({ searchParams: Promise.resolve({}) }),
    );
    expect(read).toHaveBeenCalledWith("/api/ui/v1/account");
    expect(html).toContain("member@example.test");
    expect(html).not.toContain("Password form");
    expect(html).toContain("켜짐");
  });
  it("renders enrollment QR from API-generated enrollment material", async () => {
    read.mockResolvedValue({
      enabled: false,
      verificationRequired: false,
      pending: {
        secret: "APISECRET",
        otpauthUri: "otpauth://totp/test?secret=APISECRET",
      },
    });
    const { default: Page } = await import("../app/(app)/account/2fa/page");
    expect(renderToStaticMarkup(await Page())).toContain("Enroll APISECRET");
    expect(qr).toHaveBeenCalledWith("otpauth://totp/test?secret=APISECRET", {
      margin: 1,
      width: 220,
    });
  });
  it("does not generate enrollment material for enabled accounts", async () => {
    read
      .mockResolvedValueOnce({
        enabled: true,
        pending: null,
        verificationRequired: false,
      })
      .mockResolvedValueOnce({ hasPassword: true });
    const { default: Page } = await import("../app/(app)/account/2fa/page");
    const html = renderToStaticMarkup(await Page());
    expect(html).toContain("2FA 관리");
    expect(html).toContain('href="/account/2fa/disable"');
    expect(html).not.toContain("Disable form");
    expect(qr).not.toHaveBeenCalled();
  });
  it("renders verification instead of management for an enrolled unverified session", async () => {
    read.mockResolvedValue({
      enabled: true,
      pending: null,
      verificationRequired: true,
    });
    const { default: Page } = await import("../app/(app)/account/2fa/page");
    const html = renderToStaticMarkup(await Page());
    expect(html).toContain("Verify form");
    expect(html).not.toContain("Disable form");
    expect(qr).not.toHaveBeenCalled();
  });
  it("renders general settings returned by the admin API", async () => {
    read.mockResolvedValue({
      publicBaseUrl: "https://app.example.test",
      metricsRetentionDays: 30,
      metricsExcludedHosts: "",
    });
    const { default: Page } =
      await import("../app/admin/settings/general/page");
    const html = renderToStaticMarkup(await Page());
    expect(read).toHaveBeenCalledWith("/api/ui/v1/admin/settings/general");
    expect(html).toContain("https://app.example.test");
    expect(html).toContain("Retention 30");
  });
});
