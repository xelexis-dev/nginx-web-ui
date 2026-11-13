import { expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { ReactNode } from "react";
import type { ActionRef } from "@repo/contracts";

vi.mock("react", async (original) => {
  const react = await original<typeof import("react")>();
  return {
    ...react,
    useState: (initial: unknown) =>
      initial === false ? [true, () => undefined] : react.useState(initial),
  };
});
vi.mock("next/navigation", () => ({ usePathname: () => "/" }));
vi.mock("./api-form", () => ({
  ApiForm: ({
    action,
    children,
  }: {
    action: ActionRef;
    children: ReactNode;
  }) => <form data-action={action.id}>{children}</form>,
}));
import { ProfileMenu } from "./profile-menu";
import { HeaderActions } from "./operations/header-actions";

const FULL_SHA = "4cb27512d0d6bebb4d5e6c123e8a3222f530e4cf";
const build = { version: "2.0.0", sha: "4cb2751", fullSha: FULL_SHA };

// Behaviour (toggle, Escape, backdrop, drawer) is checked in the browser on the Storybook export; this
// node renderer only sees the open markup.
it.each(["admin", "editor", "viewer"] as const)(
  "shows the account, build and sign-out without an admin link for %s",
  (role) => {
    const html = renderToStaticMarkup(
      <ProfileMenu email="member@example.test" role={role} {...build} />,
    );
    expect(html).not.toContain('href="/admin"');
    expect(html).not.toContain("관리자 패널");
    expect(html).not.toMatch(/role="menu(item)?"/);
    expect(html).toMatch(
      /<form data-action="session\.signOut"><button type="submit"[^>]*>(?:(?!<\/button>)[\s\S])*<span>모든 기기에서 로그아웃<\/span><\/button><\/form>/,
    );
    expect(html).toContain('title="member@example.test"');
    expect(html).toContain("member@<wbr/>example.<wbr/>test");
    expect(html.match(new RegExp(`>${role}<`, "g"))).toHaveLength(1);
    expect(html).toContain("<dd>nginx-web-ui v2.0.0</dd>");
    expect(html).toContain('href="/account"');
    expect(html).toContain('href="/account/tokens"');
    expect(html).toContain('aria-label="계정 메뉴: member@example.test"');
    expect(html).toContain(`<span title="${FULL_SHA}">4cb2751</span>`);
  },
);

it("names the open flyout as a dialog the row controls", () => {
  const html = renderToStaticMarkup(
    <ProfileMenu email="member@example.test" role="admin" {...build} />,
  );
  const trigger =
    /<button[^>]*aria-haspopup="dialog"[^>]*>/.exec(html)?.[0] ?? "";
  expect(trigger).toContain('aria-expanded="true"');
  const controls = /aria-controls="([^"]+)"/.exec(trigger)?.[1];
  const panel =
    new RegExp(`<section id="${controls}"[^>]*>`).exec(html)?.[0] ?? "";
  expect(panel).toContain('role="dialog"');
  expect(panel).not.toContain("hidden");
  const title = /aria-labelledby="([^"]+)"/.exec(panel)?.[1];
  expect(html).toMatch(new RegExp(`id="${title}">로그인 계정<`));
});

it("shows dev without a full-SHA tooltip when the build has no commit", () => {
  const html = renderToStaticMarkup(
    <ProfileMenu
      email="member@example.test"
      role="viewer"
      version="2.0.0"
      sha="dev"
      fullSha=""
    />,
  );
  expect(html).toContain("<dd><span>dev</span></dd>");
});

it("offers a single named icon action for switching to the other theme", () => {
  const html = renderToStaticMarkup(
    <HeaderActions canApply={false} pendingCount={0} onApply={() => {}} />,
  );
  expect(html).toMatch(/<button[^>]*aria-label="다크 테마로 전환"[^>]*>/);
  expect(html).not.toContain('role="group" aria-label="테마"');
  const profile = renderToStaticMarkup(
    <ProfileMenu email="member@example.test" role="viewer" {...build} />,
  );
  expect(profile).toContain('aria-label="다크 테마로 전환"');
});

it("keeps the account dialog heading out of the page banner landmarks", () => {
  const html = renderToStaticMarkup(
    <ProfileMenu email="member@example.test" role="editor" {...build} />,
  );
  expect(html).toContain('role="dialog"');
  expect(html).toContain("로그인 계정");
  expect(html).not.toMatch(/<header(?:\s|>)|role="banner"/);
});
