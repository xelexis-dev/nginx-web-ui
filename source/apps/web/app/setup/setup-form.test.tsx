import { afterEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { BasicFormState } from "@repo/contracts";
import { submitApiForm } from "@/lib/api-transport";

const mocks = vi.hoisted(() => ({ state: { ok: false } as BasicFormState, action: vi.fn() }));
vi.mock("react", async original => ({ ...await original<typeof import("react")>(), useActionState: () => [mocks.state, undefined, false] }));
vi.mock("@/lib/use-api-action", () => ({ useApiAction: mocks.action }));
import { SetupForm } from "./setup-form";

afterEach(() => vi.unstubAllGlobals());

it("shows the setup Host policy's 403 message as an alert without redirecting", async () => {
  const message = "최초 관리자 설정은 localhost 또는 IP 주소로 접속하거나 AUTH_URL에 관리 화면 주소를 지정한 뒤 진행하세요.";
  vi.stubGlobal("fetch", vi.fn(async () => Response.json({ error: { code: "setup_host_forbidden", message } }, { status: 403 })));
  const response = await submitApiForm<BasicFormState>({ id: "setup.createFirstAdmin", args: [] }, new FormData());
  expect(response.redirectTo).toBeUndefined();
  expect(response.result).toEqual({ ok: false, error: message });
  mocks.state = response.result!;
  const html = renderToStaticMarkup(<SetupForm />);
  expect(html).toContain('role="alert"');
  expect(html).toContain(message);
  expect(mocks.action).toHaveBeenCalledWith({ id: "setup.createFirstAdmin", args: [] });
});
