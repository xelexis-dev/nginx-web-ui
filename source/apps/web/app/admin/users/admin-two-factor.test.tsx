import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { BasicFormState } from "@repo/contracts";
const mocks = vi.hoisted(() => ({
  state: { ok: false } as BasicFormState,
  open: false,
  pending: false,
  submit: vi.fn(),
  push: vi.fn(),
  refresh: vi.fn(),
  assign: vi.fn(),
  actions: [] as Array<
    (previous: BasicFormState, form: FormData) => Promise<BasicFormState>
  >,
}));
vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  useState: () => [
    mocks.open,
    (value: boolean) => {
      mocks.open = value;
    },
  ],
  useActionState: (
    action: (
      previous: BasicFormState,
      form: FormData,
    ) => Promise<BasicFormState>,
  ) => {
    mocks.actions.push(action);
    return [mocks.state, vi.fn(), mocks.pending];
  },
}));
vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: mocks.push, refresh: mocks.refresh }),
}));
vi.mock("@/lib/api-transport", async (original) => ({
  ...(await original<typeof import("@/lib/api-transport")>()),
  submitApiForm: mocks.submit,
}));
import {
  AdminTwoFactor,
  AdminUserResultNotice,
  ResetDialog,
} from "./admin-two-factor";
const user = {
  id: "u1",
  email: "owner@example.test",
  role: "editor" as const,
  enabled: true,
  approved: true,
  hasPassword: true,
  totpEnabled: true,
  createdAt: "2026-11-13T00:00:00Z",
};
beforeEach(() => {
  vi.clearAllMocks();
  mocks.actions.length = 0;
  mocks.state = { ok: false };
  mocks.open = false;
  mocks.pending = false;
});
it("offers reset only for another enrolled account and preserves all non-action states", () => {
  const props = { user, isSelf: false, canReset: true, passkeyCount: 2 };
  expect(renderToStaticMarkup(<AdminTwoFactor {...props} />)).toContain(
    "2FA 초기화</button>",
  );
  const off = renderToStaticMarkup(
    <AdminTwoFactor {...props} user={{ ...user, totpEnabled: false }} />,
  );
  expect(off).toContain("미설정");
  expect(off).not.toContain("2FA 초기화</button>");
  const self = renderToStaticMarkup(<AdminTwoFactor {...props} isSelf />);
  expect(self).toContain('href="/account/2fa"');
  expect(self).toContain("본인 계정의 2FA는 계정 화면에서 관리합니다.");
  expect(self).not.toContain("2FA 초기화</button>");
  const oidc = renderToStaticMarkup(
    <AdminTwoFactor {...props} canReset={false} />,
  );
  expect(oidc).toContain(
    "외부 로그인 전용 관리자 계정은 2FA를 초기화할 수 없습니다.",
  );
  expect(oidc).not.toContain("2FA 초기화</button>");
  expect(
    renderToStaticMarkup(
      <AdminUserResultNotice message="2FA를 초기화했습니다. 인증 앱 등록, 백업 코드와 패스키를 삭제하고 사용자의 로그인을 종료했습니다." />,
    ),
  ).toContain(
    "2FA를 초기화했습니다. 인증 앱 등록, 백업 코드와 패스키를 삭제하고 사용자의 로그인을 종료했습니다.",
  );
});
it.each([0, 2])(
  "confirms the exact consequences and native form fields (passkeys=%i)",
  (count) => {
    const html = renderToStaticMarkup(
      <ResetDialog user={user} passkeyCount={count} onClose={vi.fn()} />,
    );
    expect(html).toContain("owner@example.test의 2FA를 초기화하시겠습니까?");
    expect(html).toContain('name="id" value="u1"');
    expect(html).toContain('name="currentPassword"');
    expect(html).toContain("관리자 본인의 비밀번호입니다.");
    expect(html.includes(`패스키 ${count}개`)).toBe(count > 0);
    expect(html).toContain(
      "2FA가 필수이면 설정을 마쳐야 다른 화면을 쓸 수 있습니다.",
    );
  },
);
it.each([
  { fieldErrors: { currentPassword: "현재 비밀번호가 올바르지 않습니다." } },
  { error: "2단계 인증이 설정되어 있지 않습니다." },
  { error: "실패가 잦아 잠시 차단되었습니다. 15분 뒤에 다시 시도하세요." },
])("keeps server failures in the dialog: %o", (error) => {
  mocks.state = { ok: false, ...error };
  const html = renderToStaticMarkup(
    <ResetDialog user={user} passkeyCount={2} onClose={vi.fn()} />,
  );
  expect(html).toContain(
    "fieldErrors" in error ? error.fieldErrors?.currentPassword : error.error,
  );
  expect(html).toContain(
    "fieldErrors" in error ? 'aria-invalid="true"' : 'role="alert"',
  );
});
it("submits the administrator proof and target without replacing the session document", async () => {
  renderToStaticMarkup(
    <ResetDialog user={user} passkeyCount={2} onClose={vi.fn()} />,
  );
  mocks.submit.mockResolvedValue({
    redirectTo: "/admin/users/u1?twoFactorReset=1",
  });
  const form = new FormData();
  form.set("id", "u1");
  form.set("currentPassword", "synthetic-password");
  await mocks.actions[0]!({ ok: false }, form);
  expect(mocks.submit).toHaveBeenCalledExactlyOnceWith(
    { id: "admin.users.resetTwoFactor", args: [] },
    form,
  );
  expect(mocks.push).toHaveBeenCalledExactlyOnceWith(
    "/admin/users/u1?twoFactorReset=1",
  );
  expect(mocks.refresh).toHaveBeenCalledOnce();
  expect(mocks.assign).not.toHaveBeenCalled();
});

it("locks close, cancel and submit while resetting", () => {
  mocks.pending = true;
  const html = renderToStaticMarkup(
    <ResetDialog user={user} passkeyCount={2} onClose={vi.fn()} />,
  );
  expect(html.match(/<button\b[^>]*disabled=""/g)).toHaveLength(3);
  expect(html).toContain('aria-busy="true"');
});
