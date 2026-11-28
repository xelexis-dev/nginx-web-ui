import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { BasicFormState } from "@repo/contracts";
import { buttonClassName } from "@repo/ui";
import type { PasskeyListView } from "@/lib/passkey-api";
const mocks = vi.hoisted(() => ({
  phase: "idle",
  supported: true,
  result: {} as Partial<BasicFormState>,
  state: { ok: false } as BasicFormState,
}));
vi.mock("@/lib/passkey-client", () => ({
  usePasskeyCeremony: () => ({ ...mocks, submit: vi.fn(), cancel: vi.fn() }),
}));
vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }),
}));
vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  useActionState: () => [mocks.state, () => {}, false],
}));
import { AccountPasskeys } from "./passkey-list";
import { RegisterPasskeyForm, RenamePasskeyForm } from "./passkey-forms";
import { RemovePasskeyDialog } from "./remove-passkey-dialog";
import { NewPasskeyView } from "@/app/(app)/account/passkeys/new-passkey";
import { PasskeyDetailView } from "@/app/(app)/account/passkeys/passkey-detail";
import { LinkedLogins } from "@/app/(app)/account/linked-logins";
import { AdminPasskeys } from "./admin-passkeys";
import { LoginScreen } from "../auth/login-screen";

const row = {
  id: "key-1",
  name: "개인 노트북",
  origin: "https://admin.example.test:8443",
  address: "admin.example.test:8443",
  rpId: "admin.example.test",
  createdAt: "등록 시각",
  createdOn: "등록일",
  lastUsedAt: "사용한 적 없음",
  backupEligible: true,
  backupState: true,
  unavailableHere: true,
};
const data = (): PasskeyListView => ({
  passkeys: [row],
  availability: { available: true, reason: null },
  currentOrigin: "https://admin.example.test",
  currentAddress: "admin.example.test",
  hasPassword: true,
  canRemove: true,
});
const html = (element: ReactNode) => renderToStaticMarkup(element);
beforeEach(() => {
  mocks.phase = "idle";
  mocks.supported = true;
  mocks.result = {};
  mocks.state = { ok: false };
});

describe("account list", () => {
  it("keeps address-unavailable rows and removal links, hiding only badges and registration", () => {
    const value = data();
    value.availability = { available: false, reason: "ip_host" };
    const output = html(<AccountPasskeys data={value} />);
    expect(output).toContain('href="/account/passkeys/key-1"');
    expect(output).toContain("admin.example.test:8443");
    expect(output).toContain("이 주소에서 사용 불가");
    expect(output).not.toContain('href="/account/passkeys/new"');
    expect(output).toContain('data-narrow="stacked"');
    expect(output).toContain('data-density="comfortable"');
  });
  it("uses the API's unavailableHere, not browser location, and keeps backup state on detail only", () => {
    const value = data();
    value.passkeys = [{ ...row, unavailableHere: false }];
    expect(html(<AccountPasskeys data={value} />)).not.toContain(
      "이 주소에서 사용 불가",
    );
    const output = html(<AccountPasskeys data={data()} />);
    expect(output).toContain("이 주소에서 사용 불가");
    expect(output).not.toContain("백업됨");
    expect(output).toContain("사용한 적 없음");
  });
  it("renders empty, OAuth-only, limit and removal-success states", () => {
    const empty = data();
    empty.passkeys = [];
    const emptyOutput = html(<AccountPasskeys data={empty} />);
    expect(emptyOutput).toMatch(/<h2[^>]*>패스키<\/h2><span[^>]*data-tag=""[^>]*>0\/10<\/span>/);
    expect(emptyOutput).toContain("등록한 패스키가 없습니다");
    const oauth = data();
    oauth.hasPassword = false;
    oauth.passkeys = [];
    expect(html(<AccountPasskeys data={oauth} />)).toContain(
      "외부 로그인만 쓰는 계정은 패스키를 등록할 수 없습니다.",
    );
    const limit = data();
    limit.passkeys = Array.from({ length: 10 }, (_, i) => ({
      ...row,
      id: `${i}`,
    }));
    const limitOutput = html(<AccountPasskeys data={limit} />);
    expect(limitOutput).toMatch(/<span[^>]*data-tag=""[^>]*>10\/10<\/span>/);
    expect(limitOutput).toContain("10개를 모두 등록했습니다.");
    expect(limitOutput).not.toContain(
      'href="/account/passkeys/new"',
    );
  });
});
describe("registration and removal", () => {
  it("keeps fields wide and shows unavailable browser and account reasons", () => {
    mocks.supported = false;
    const output = html(<RegisterPasskeyForm data={data()} />);
    expect(output).toContain("이 브라우저에서는 패스키를 등록할 수 없습니다.");
    expect(output).toMatch(/<button[^>]*disabled/);
  });
  it.each([
    ["waiting", "기기에서 확인해 주세요."],
    ["cancelled", "패스키 등록을 취소했습니다."],
    ["failed", "패스키를 등록하지 못했습니다."],
    ["duplicate", "이 기기에는 이미 등록된 패스키가 있습니다"],
  ])("renders %s", (phase, text) => {
    mocks.phase = phase;
    expect(html(<RegisterPasskeyForm data={data()} />)).toContain(text);
  });
  it("puts wrong-password details on the field and 429/409 errors in a banner", () => {
    mocks.result = {
      ok: false,
      fieldErrors: { currentPassword: "현재 비밀번호가 올바르지 않습니다." },
    };
    expect(html(<RegisterPasskeyForm data={data()} />)).toContain(
      "현재 비밀번호가 올바르지 않습니다.",
    );
    for (const error of ["너무 많이 시도했습니다.", "다시 시작하세요."]) {
      mocks.result = { ok: false, error };
      expect(html(<RegisterPasskeyForm data={data()} />)).toMatch(
        /role="alert"/,
      );
      expect(html(<RegisterPasskeyForm data={data()} />)).toContain(error);
    }
  });
  it.each([
    { userId: undefined, id: "key-1", all: false },
    { userId: "u1", id: "key-1", all: false },
    { userId: "u1", id: undefined, all: true },
  ])("preserves delete payload $userId/$id/$all and field errors", (props) => {
    mocks.state = {
      ok: false,
      fieldErrors: { currentPassword: "비밀번호 오류" },
    };
    const output = html(
      <RemovePasskeyDialog
        action={{
          id: props.userId
            ? "admin.users.passkeys.remove"
            : "account.passkeys.remove",
          args: [],
        }}
        {...props}
        name="개인 노트북"
        email="member@example.test"
        onClose={() => {}}
      />,
    );
    expect(output.includes('name="userId"')).toBe(Boolean(props.userId));
    expect(output.includes('name="id"')).toBe(Boolean(props.id));
    expect(output.includes('name="all"')).toBe(props.all);
    expect(output).toContain('name="currentPassword"');
    expect(output).toContain("비밀번호 오류");
    expect(output).toContain("삭제할까요?");
    expect(html(<RenamePasskeyForm row={row} />)).toMatch(/max[Ll]ength="64"/);
  });
  it.each([false, true])(
    "keeps the current session in self-target removal (all=%s)",
    (all) => {
      const output = html(
        <RemovePasskeyDialog
          action={{ id: "admin.users.passkeys.remove", args: [] }}
          userId="u1"
          email="admin@example.test"
          isSelf
          all={all}
          id={all ? undefined : "key-1"}
          onClose={() => {}}
        />,
      );
      expect(output).toContain(
        "이 계정의 다른 세션은 모두 로그아웃되고, 지금 세션은 유지됩니다.",
      );
      expect(output).not.toContain(
        "사용자의 모든 기기에서 로그인이 종료됩니다.",
      );
    },
  );
  it("keeps the admin passkey section free of the page-owned removal result", () => {
    const output = html(
      <AdminPasskeys
        data={data()}
        userId="u1"
        email="admin@example.test"
        isSelf
      />,
    );
    expect(output).not.toContain("패스키를 삭제했습니다.");
    expect(output).not.toContain(
      "패스키를 삭제하고 사용자의 로그인을 종료했습니다.",
    );
  });
  it("disables admin removal for passwordless actors and keeps its section notice", () => {
    const value = data();
    value.canRemove = false;
    const output = html(
      <AdminPasskeys data={value} userId="u1" email="member@example.test" />,
    );
    expect(output).toContain("외부 로그인 전용 관리자 계정");
    expect(output).not.toContain(
      "패스키를 삭제하고 사용자의 로그인을 종료했습니다.",
    );
    expect(output).toMatch(/<button[^>]*disabled/);
  });
});
describe("login", () => {
  it.each([
    ["idle", "이메일이나 비밀번호를 입력하지 않아도 됩니다."],
    ["waiting", "기기의 인증 창에서 로그인을 완료하세요."],
    ["cancelled", "로그인을 취소했습니다."],
    ["failed", "패스키를 확인하지 못했습니다."],
  ])("renders %s without marking password inputs", (phase, text) => {
    mocks.phase = phase;
    const output = html(
      <LoginScreen state="default" providers={[]} passkeyAvailable />,
    );
    expect(output).toContain(text);
    expect(output).not.toContain('aria-invalid="true"');
    expect(output).not.toMatch(/name="password"[^>]*disabled/);
  });
  it("uses one divider before passkey and providers and hides unavailable passkeys", () => {
    const providers = [{ id: "oidc", name: "회사" }];
    const output = html(
      <LoginScreen state="default" providers={providers} passkeyAvailable />,
    );
    expect(output.match(/>또는</g)).toHaveLength(1);
    expect(output.indexOf("패스키로 로그인")).toBeLessThan(
      output.indexOf("회사 로 로그인"),
    );
    expect(
      html(
        <LoginScreen state="default" providers={[]} passkeyAvailable={false} />,
      ),
    ).not.toContain("패스키로 로그인");
  });
});

it.each([false, true, undefined])(
  "public unavailable note only for false (%s)",
  (available) => {
    const output = html(
      <LoginScreen
        state="default"
        providers={[]}
        passkeyAvailable={available}
      />,
    );
    expect(
      output.includes("이 접속에서는 패스키 로그인을 쓸 수 없습니다."),
    ).toBe(available === false);
    expect(output).not.toContain("AUTH_URL");
  },
);

it("prioritizes passwordless, count, then connection blocks without a disabled form", () => {
  const value: PasskeyListView = {
    ...data(),
    hasPassword: false,
    passkeys: Array.from({ length: 10 }, (_, i) => ({ ...row, id: String(i) })),
    availability: { available: false, reason: "insecure_origin" },
  };
  let output = html(<NewPasskeyView data={value} role="admin" />);
  expect(output).toContain(
    "외부 로그인만 쓰는 계정은 패스키를 등록할 수 없습니다.",
  );
  expect(output).not.toContain("<form");
  value.hasPassword = true;
  output = html(<NewPasskeyView data={value} role="admin" />);
  expect(output).toContain("최대 10개까지");
  expect(output).not.toContain("<form");
  value.passkeys = [];
  output = html(<NewPasskeyView data={value} role="admin" />);
  expect(output).toContain("패스키 접속 조건 확인 실패");
  expect(output).not.toContain("<form");
  value.availability = { available: true, reason: null };
  expect(html(<NewPasskeyView data={value} />)).toContain("패스키 만들기");
});
it("describes a scheme-only mismatch and the historical backup state truthfully", () => {
  const output = html(
    <PasskeyDetailView
      data={{
        passkey: {
          ...row,
          origin: "http://localhost:8080",
          createdAt: "2026-11-01T00:00:00Z",
          lastUsedAt: null,
        },
        currentOrigin: "https://localhost:8080",
        availability: { available: true, reason: null },
        hasPassword: true,
        canRemove: true,
      }}
    />,
  );
  expect(output).toContain("프로토콜 또는 주소·포트");
  expect(output).toContain("등록 시 또는 마지막 패스키 로그인에서 확인한 상태");
  expect(output).toContain('data-notes=""');
  expect(output).not.toContain("기기를 잃어도");
  const opener = output.match(/<button[^>]*>삭제<\/button>/)?.[0];
  expect(opener).toContain(`class="${buttonClassName({ variant: "dangerOutline" })}"`);
});
it("keeps an admin table name plain and retains the action column", () => {
  const output = html(
    <AdminPasskeys data={data()} userId="u1" email="member@example.test" />,
  );
  expect(output).toContain("작업");
  expect(output).toContain('data-label="작업"');
  expect(output).not.toContain('href="/account/passkeys/key-1"');
});
it("keeps a linked-provider refusal in the row message", () => {
  mocks.state = { ok: false, error: "마지막 로그인 방법입니다." };
  const output = html(
    <LinkedLogins
      hasPassword
      rows={[
        {
          providerId: "company",
          name: "회사",
          linkedAt: new Date("2026-11-01T00:00:00Z"),
        },
      ]}
    />,
  );
  expect(output).toMatch(
    /<li[^>]*>[\s\S]*<form[\s\S]*<\/form>[\s\S]*마지막 로그인 방법입니다\.[\s\S]*<\/li>/,
  );
});

it("server-renders stable connection facts and skeletons before browser diagnostics", () => {
  const value = data();
  value.availability = { available: false, reason: "insecure_origin" };
  const output = html(<AccountPasskeys data={value} role="admin" />);
  expect(output).toContain('aria-busy="true"');
  expect(output).toContain("서버가 이 접속을 HTTPS로 확인하지 못했습니다.");
  expect(output).toContain("확인하는 중");
  expect(output).not.toContain("AUTH_URL");
  expect(output).not.toContain("CDN");
});

it("keeps the browser-address label while IP diagnostics are pending", () => {
  const value = data();
  value.availability = { available: false, reason: "ip_host" };
  const output = html(<AccountPasskeys data={value} role="admin" />);
  expect(output).toContain("브라우저 주소");
  expect(output).not.toContain("브라우저 연결");
  expect(output).toMatch(/<span[^>]*>확인하는 중<\/span>/);
});
