import { expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { RoleBadge } from "@/components/role-badge";
import { UserList } from "./users/user-list";
import { ProviderListView } from "./oauth/provider-list";
import { AdminUserResultNotice } from "./users/admin-two-factor";
import { UserDetailView } from "./users/user-detail";
import { SecurityForm } from "./security/security-form";
import { RecipientListView } from "./notifications/mail/recipient-list";
import { WebhookListView } from "./notifications/webhooks/webhook-list";
import { PendingListView } from "./pending/pending-list";
import { SecretsHealthView } from "./secrets/secrets-health";

vi.mock("@/lib/use-api-action", () => ({
  useApiAction: () => async () => ({ ok: false }),
}));

it("opts entity tables into comfortable stacked rows without adding user actions", () => {
  const html = renderToStaticMarkup(
    <UserList
      selfUserId="self"
      items={[
        {
          id: "self",
          email: "owner@example.test",
          displayName: null,
          role: "admin",
          enabled: true,
          approved: true,
          methods: "비밀번호",
          createdAt: "2026-11-16",
        },
      ]}
    />,
  );
  expect(html).toContain('data-density="comfortable"');
  expect(html).toContain('data-narrow="stacked"');
  expect(html).toContain('data-label="상태"');
  expect(html).toContain('href="/admin/users/self"');
  expect(html.split("<tbody>")[1]).not.toContain("<button");
  const providers = renderToStaticMarkup(
    <ProviderListView providers={[]} toggleForm={() => null} />,
  );
  expect(providers).toContain('data-density="comfortable"');
  expect(providers).toContain('data-narrow="stacked"');
});

it("keeps every empty admin list inside a spanning table row", () => {
  for (const view of [
    <UserList items={[]} selfUserId="self" />,
    <ProviderListView providers={[]} toggleForm={() => null} />,
    <RecipientListView recipients={[]} mailer={{ ok: true }} />,
    <WebhookListView targets={[]} />,
    <PendingListView
      pending={[]}
      approvalForm={() => null}
      rejectionForm={() => null}
    />,
    <SecretsHealthView rows={[]} broken={[]} plaintext={[]} />,
  ]) {
    const html = renderToStaticMarkup(view);
    expect(html).toMatch(
      /<tbody><tr[^>]*><td[^>]*colSpan="[45]"[^>]*><div[^>]*><p/,
    );
    expect(html).not.toContain("emptyCell");
  }
});

it("renders status outside the user heading and preserves sensitive-data boundaries", () => {
  const html = renderToStaticMarkup(
    <UserDetailView
      user={{
        id: "u",
        email: "owner@example.test",
        role: "admin",
        enabled: true,
        approved: true,
        hasPassword: true,
        totpEnabled: false,
        createdAt: "2026-11-16T00:00:00Z",
      }}
      isSelf={false}
      detailActions={null}
    />,
  );
  expect(html).toMatch(
    /<h1[^>]*>owner@example.test<\/h1><span[^>]*data-tone="success"/,
  );
});

it("renders policy rows semantically with unchanged names and independent secondary submit", () => {
  const html = renderToStaticMarkup(
    <SecurityForm requireAdmin requireUser={false} requireOauth={false} />,
  );
  expect(html.match(/<li\b/g)).toHaveLength(3);
  for (const [name, value] of [
    ["require2faAdmin", "true"],
    ["require2faUser", "false"],
    ["require2faOauth", "false"],
  ]) {
    expect(html).toContain(`name="${name}" value="${value}"`);
  }
  expect(html.match(/type="submit"/g)).toHaveLength(1);
  expect(html).not.toContain("nwui-btn-primary");
});

it("labels every stacked provider value and control without reordering columns", () => {
  const html = renderToStaticMarkup(
    <ProviderListView
      providers={[
        {
          id: "oidc",
          name: "회사 로그인",
          issuer: "https://identity.example.test",
          clientId: "nwui",
          scope: "openid",
          enabled: true,
          sortOrder: 0,
          linkedUsers: 2,
        },
      ]}
      toggleForm={() => <button type="button">사용 안 함</button>}
    />,
  );
  const cells = html.split("<tbody>")[1]!.match(/<td\b[^>]*>/g)!;
  expect(cells).toHaveLength(5);
  expect(cells[0]).toContain('data-cell="primary"');
  expect(
    cells.slice(1).map((cell) => cell.match(/data-label="([^"]+)"/)?.[1]),
  ).toEqual(["로그인 서버 주소", "연동된 사용자", "상태", "작업"]);
});

it("places one redirect notice between the header and detail sections", () => {
  const html = renderToStaticMarkup(
    <UserDetailView
      user={{
        id: "u",
        email: "owner@example.test",
        role: "admin",
        enabled: true,
        approved: true,
        hasPassword: true,
        totpEnabled: false,
        createdAt: "2026-11-16T00:00:00Z",
      }}
      isSelf={false}
      detailActions={null}
      result={<AdminUserResultNotice message="redirect result" />}
    />,
  );
  expect(html.match(/data-tone="success"[^>]*role="status"/g)).toHaveLength(1);
  expect(html.indexOf("redirect result")).toBeGreaterThan(
    html.indexOf("</header>"),
  );
  expect(html.indexOf("redirect result")).toBeLessThan(html.indexOf("<dl"));
});

it.each(["admin", "editor", "viewer"] as const)("uses the shared neutral tag for %s", role => {
  const html = renderToStaticMarkup(<RoleBadge role={role} />);
  expect(html).toContain('data-tone="neutral"');
  expect(html).toContain('data-tag=""');
  expect(html).not.toMatch(new RegExp(`class="[^"]*${role}`));
  expect(html).toContain(`>${role}</span>`);
});
