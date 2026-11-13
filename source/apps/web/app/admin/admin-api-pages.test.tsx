import { beforeEach, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";
const mocks = vi.hoisted(() => ({ read: vi.fn(), session: vi.fn() }));
vi.mock("@/lib/api-page", () => ({ readPageApi: mocks.read }));
vi.mock("@/lib/admin-api", () => ({ readAdminSession: mocks.session }));
import UsersPage from "./users/page";
import UserDetail from "./users/[id]/page";
import UserEdit from "./users/[id]/edit/page";
import PendingPage from "./pending/page";
import SecurityPage from "./security/page";

function propsOf(
  node: ReactNode,
  component: string,
): Record<string, unknown>[] {
  if (Array.isArray(node)) return node.flatMap((n) => propsOf(n, component));
  if (!isValidElement<{ children?: ReactNode; actions?: ReactNode }>(node)) return [];
  const name = typeof node.type === "function" ? node.type.name : node.type;
  // Resolve only the moved server presentation; action/client components remain unexecuted.
  if (
    typeof node.type === "function" &&
    ["UserDetailView", "PendingListView"].includes(String(name))
  ) {
    return propsOf(
      (node.type as (props: unknown) => ReactNode)(node.props),
      component,
    );
  }
  return [
    ...(name === component ? [node.props as Record<string, unknown>] : []),
    ...propsOf(node.props.children, component),
    ...propsOf(node.props.actions, component),
  ];
}
const user = {
  id: "u1",
  email: "user@example.test",
  role: "editor",
  enabled: true,
  approved: true,
  hasPassword: false,
  totpEnabled: false,
  createdAt: "2026-09-22T01:00:00Z",
};
beforeEach(() => {
  vi.clearAllMocks();
  mocks.session.mockResolvedValue({ user: { id: "admin1", role: "admin" } });
});
it("renders safe user metadata and passes the API hasPassword flag to edit", async () => {
  mocks.read.mockResolvedValueOnce([
    { ...user, displayName: "사용자", providers: ["OIDC"] },
  ]);
  expect(await UsersPage()).toBeTruthy();
  expect(mocks.read).toHaveBeenLastCalledWith("/api/ui/v1/admin/users");
  mocks.read.mockResolvedValueOnce(user).mockResolvedValueOnce({ passkeys: [], availability: { available: true, reason: null }, currentOrigin: "https://admin.example.test", hasPassword: false, canRemove: true });
  expect(
    propsOf(
      await UserDetail({ params: Promise.resolve({ id: "u1" }) }),
      "UserDetailActions",
    )[0],
  ).toMatchObject({ id: "u1", isSelf: false, enabled: true });
  mocks.read.mockResolvedValueOnce(user);
  expect(
    propsOf(
      await UserEdit({ params: Promise.resolve({ id: "u1" }) }),
      "EditUserForms",
    )[0],
  ).toMatchObject({ hasPassword: false, currentRole: "editor" });
});
it.each([false, true])("passes actor identity to admin passkeys (isSelf=%s)", async (isSelf) => {
  mocks.session.mockResolvedValue({ user: { id: isSelf ? "u1" : "admin1", role: "admin" } });
  mocks.read.mockResolvedValueOnce(user).mockResolvedValueOnce({ passkeys: [], availability: { available: true, reason: null }, currentOrigin: "https://admin.example.test", hasPassword: false, canRemove: true });
  const page = await UserDetail({ params: Promise.resolve({ id: "u1" }), searchParams: Promise.resolve({ passkeysRemoved: "1" }) });
  expect(propsOf(page, "AdminPasskeys")[0]).toMatchObject({ isSelf, removed: true, userId: "u1" });
});
it("keeps approval and rejection separate browser API commands", async () => {
  mocks.read.mockResolvedValue([
    { ...user, displayName: null, provider: "OIDC" },
  ]);
  const page = await PendingPage();
  const forms = propsOf(page, "ApiForm");
  expect(forms.map((form) => form.action)).toEqual([
    { id: "admin.pending.approve", args: [] },
    { id: "admin.pending.reject", args: [] },
  ]);
  expect(forms[0]?.id).toBe("approve-u1");
  expect(propsOf(page, "Select")[0]).toMatchObject({
    name: "role",
    defaultValue: "viewer",
  });
  expect(propsOf(page, "Button")[0]).toMatchObject({
    type: "submit",
    form: "approve-u1",
  });
});
it("renders policy and connected gateways from one authorized API response", async () => {
  const gateways = [
    {
      id: "g1",
      name: "gateway",
      role: "viewer",
      createdAt: user.createdAt,
      lastUsedAt: null,
      lastUsedIp: null,
      revokedAt: null,
    },
  ];
  mocks.read.mockResolvedValue({
    policy: {
      require2faAdmin: true,
      require2faUser: false,
      require2faOauth: true,
    },
    gateways,
  });
  const page = await SecurityPage();
  expect(propsOf(page, "SecurityForm")[0]).toMatchObject({
    requireAdmin: true,
    requireUser: false,
    requireOauth: true,
  });
  expect(propsOf(page, "ConnectedGateways")[0]?.rows).toBe(gateways);
  expect(mocks.read).toHaveBeenCalledTimes(1);
});
it("passes the target factor state, actor proof capability and reset result to the user detail", async () => {
  mocks.read.mockResolvedValueOnce({ ...user, totpEnabled: true }).mockResolvedValueOnce({ passkeys: [], availability: { available: true, reason: null }, currentOrigin: "https://admin.example.test", hasPassword: false, canRemove: false });
  const page = await UserDetail({ params: Promise.resolve({ id: "u1" }), searchParams: Promise.resolve({ twoFactorReset: "1" }) });
  expect(propsOf(page, "AdminTwoFactor")[0]).toMatchObject({ user: { totpEnabled: true }, isSelf: false, canReset: false, passkeyCount: 0, reset: true });
});
