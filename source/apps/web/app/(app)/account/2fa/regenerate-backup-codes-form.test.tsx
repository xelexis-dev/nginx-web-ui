import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
const mocks = vi.hoisted(() => ({ state: {} as Record<string, unknown>, pending: false, action: vi.fn() }));
vi.mock("react", async original => ({ ...await original<typeof import("react")>(), useActionState: () => [mocks.state, undefined, mocks.pending] }));
vi.mock("@/lib/use-api-action", () => ({ useApiAction: mocks.action }));
vi.mock("./backup-codes-card", () => ({ BackupCodesCard: ({ codes, context }: { codes: string[]; context: string }) => <div data-context={context}>{codes.join("|")}</div> }));
import { RegenerateBackupCodesForm } from "./regenerate-backup-codes-form";
beforeEach(() => { mocks.state = { ok: false }; mocks.pending = false; mocks.action.mockClear(); });
it("requires current password and acknowledgement of invalidating old codes", () => {
  const html = renderToStaticMarkup(<RegenerateBackupCodesForm hasPassword />);
  expect(html).toContain('name="currentPassword"');
  expect(html).toContain('type="password"');
  expect(html).toContain('autoComplete="current-password"');
  expect(html).toMatch(/type="checkbox"[^>]*required=""/);
  expect(html).not.toContain('name="code"');
  expect(mocks.action).toHaveBeenCalledWith({ id: "account.regenerateBackupCodes", args: [] });
});
it("uses exactly-six-digit authenticator proof for passwordless accounts", () => {
  const html = renderToStaticMarkup(<RegenerateBackupCodesForm hasPassword={false} />);
  expect(html).toContain('name="code"');
  expect(html).toContain('pattern="[0-9]{6}"');
  expect(html).toContain('inputMode="numeric"');
  expect(html).not.toContain('name="currentPassword"');
});
it("replaces all proof inputs with the newly returned code card after success", () => {
  mocks.state = { ok: true, backupCodes: ["abcde-12345", "12345-abcde"] };
  const html = renderToStaticMarkup(<RegenerateBackupCodesForm hasPassword />);
  expect(html).toContain('data-context="regenerated"');
  expect(html).toContain("abcde-12345|12345-abcde");
  expect(html).not.toContain("<form");
  expect(html).not.toContain('type="password"');
});
it.each([true, false])("links server field errors and blocks duplicate requests while pending: %s", hasPassword => {
  const field = hasPassword ? "currentPassword" : "code";
  mocks.state = { ok: false, error: "입력값을 확인하세요.", fieldErrors: { [field]: "본인 확인 실패" } };
  mocks.pending = true;
  const html = renderToStaticMarkup(<RegenerateBackupCodesForm hasPassword={hasPassword} />);
  expect(html).toContain('aria-invalid="true"');
  expect(html).toContain("본인 확인 실패");
  expect(html).toContain('disabled=""');
  expect(html).toContain("재발급 중...");
});
