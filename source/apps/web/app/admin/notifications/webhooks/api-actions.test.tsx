import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { WebhookFormState } from "./form-types";

const mocks = vi.hoisted(() => ({ actions: [] as Array<(state: WebhookFormState, form: FormData) => Promise<WebhookFormState>>, submit: vi.fn(), push: vi.fn(), refresh: vi.fn(), states: [] as WebhookFormState[] }));
vi.mock("react", async original => ({
  ...await original<typeof import("react")>(),
  useActionState: (action: (state: WebhookFormState, form: FormData) => Promise<WebhookFormState>, initial: WebhookFormState) => {
    if (typeof action !== "function") throw new Error("Browser callback required");
    mocks.actions.push(action);
    return [mocks.states.shift() ?? initial, () => undefined, false];
  },
}));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: mocks.push, refresh: mocks.refresh }) }));
vi.mock("@/lib/api-transport", () => ({ submitApiForm: mocks.submit }));
vi.mock("./actions", () => { throw new Error("Server Actions must not enter browser forms"); });

import { WebhookForm } from "./webhook-form";
import { WebhookDetailActions } from "./[id]/detail-actions";
import { MailRecipientForm } from "../mail/recipient-form";
import { MailRecipientDetailActions } from "../mail/[id]/detail-actions";
import { SmtpForm } from "../../settings/smtp/smtp-form";

const defaults = { id: "w1", name: "Ops", enabled: true, format: "generic" as const, events: ["cert.issued"], bodyTemplate: "" };
beforeEach(() => { vi.clearAllMocks(); mocks.actions.length = 0; mocks.states.length = 0; });

it.each(["create", "edit"] as const)("webhook %s routes save and test through the API without losing validation errors", async mode => {
  const html = renderToStaticMarkup(<WebhookForm mode={mode} defaults={defaults} maskedUrl="https://hooks.example/••••" storedHeaderNames={["Authorization"]} />);
  expect(html).toContain(`name="url" value="${mode === "edit" ? "__keep__" : ""}"`);
  const result = { ok: false, fieldErrors: { url: "올바른 주소를 입력하세요." } };
  mocks.submit.mockResolvedValue({ result });
  const form = new FormData(); form.set("id", "w1"); form.set("url", "__keep__");
  for (const action of mocks.actions) expect(await action({ ok: false }, form)).toBe(result);
  expect(mocks.submit.mock.calls.map(call => call[0])).toEqual([
    { id: mode === "create" ? "admin.webhooks.create" : "admin.webhooks.update", args: [] },
    { id: "admin.webhooks.test", args: [] },
  ]);
  expect(mocks.push).not.toHaveBeenCalled();
});

it("detail delete follows the API redirect and test keeps the stored URL", async () => {
  const html = renderToStaticMarkup(<WebhookDetailActions id="w1" format="generic" bodyTemplate={null} />);
  expect(html).toContain('name="url" value="__keep__"');
  mocks.submit.mockResolvedValueOnce({ result: { ok: true, message: "전송했습니다." } }).mockResolvedValueOnce({ redirectTo: "/admin/notifications/webhooks", refresh: true });
  const form = new FormData(); form.set("id", "w1");
  for (const action of mocks.actions) await action({ ok: false }, form);
  expect(mocks.submit.mock.calls.map(call => call[0].id)).toEqual(["admin.webhooks.test", "admin.webhooks.delete"]);
  expect(mocks.push).toHaveBeenCalledWith("/admin/notifications/webhooks");
  expect(mocks.refresh).toHaveBeenCalledOnce();
});

it("save errors remain visible alongside a successful test", () => {
  mocks.states.push({ ok: false, error: "API 서버에 연결하지 못했습니다." }, { ok: true, message: "테스트 전송 완료" });
  const html = renderToStaticMarkup(<WebhookForm mode="edit" defaults={defaults} maskedUrl="https://hooks.example/••••" />);
  expect(html).toContain("API 서버에 연결하지 못했습니다.");
  expect(html).toContain("테스트 전송 완료");
});

it.each(["create", "edit"] as const)("mail recipient %s submits separate save and test commands", async mode => {
  renderToStaticMarkup(<MailRecipientForm mode={mode} defaults={{ id: "m1", label: "Ops", address: "ops@example.test", enabled: true, events: ["cert.issued"] }} smtpReady />);
  const result = { ok: false, fieldErrors: { address: "주소를 확인하세요." } };
  mocks.submit.mockResolvedValue({ result });
  const form = new FormData(); form.set("address", "invalid");
  for (const action of mocks.actions) expect(await action({ ok: false }, form)).toBe(result);
  expect(mocks.submit.mock.calls.map(call => call[0])).toEqual([
    { id: mode === "create" ? "admin.mailRecipients.create" : "admin.mailRecipients.update", args: [] },
    { id: "admin.mailRecipients.test", args: [] },
  ]);
});

it("mail detail uses the API for test and delete", async () => {
  const html = renderToStaticMarkup(<MailRecipientDetailActions id="m1" address="ops@example.test" />);
  expect(html).toContain('name="address" value="ops@example.test"');
  mocks.submit.mockResolvedValue({ result: { ok: true }, refresh: true });
  for (const action of mocks.actions) await action({ ok: false }, new FormData());
  expect(mocks.submit.mock.calls.map(call => call[0].id)).toEqual(["admin.mailRecipients.test", "admin.mailRecipients.delete"]);
});

it("SMTP retains stored passwords as an opaque marker and separates connection test from saving", async () => {
  const html = renderToStaticMarkup(<SmtpForm defaults={{ enabled: true, host: "smtp.example.test", port: 587, secure: false, username: "ops", fromEmail: "nginx@example.test", fromName: "Nginx", rejectUnauthorized: true }} hasStoredPassword />);
  expect(html).toContain('name="password" value="__keep__"');
  mocks.submit.mockResolvedValue({ result: { ok: false, error: "SMTP 연결 실패" } });
  const form = new FormData(); form.set("password", "__keep__");
  for (const action of mocks.actions) expect(await action({ ok: false }, form)).toEqual({ ok: false, error: "SMTP 연결 실패" });
  expect(mocks.submit.mock.calls.map(call => call[0])).toEqual([{ id: "admin.smtp.update", args: [] }, { id: "admin.smtp.test", args: [] }]);
});
