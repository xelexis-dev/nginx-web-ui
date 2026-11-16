import { expect, it, vi } from "vitest";
import type { ConfigDiff } from "@/lib/config-diff";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
vi.mock("@repo/ui", () => ({
  Button: ({ children }: { children: React.ReactNode }) =>
    createElement("button", null, children),
  Modal: () => null,
  Icon: () => null,
  IconButton: ({ label, ...props }: { label: string }) =>
    createElement("button", { ...props, "aria-label": label }),
  StatusBadge: ({
    children,
    tone,
  }: {
    children: React.ReactNode;
    tone: string;
  }) => createElement("span", { "data-tone": tone }, children),
}));
vi.mock("./config-diff-view", () => ({ ConfigDiffView: () => null }));
import { PendingChangesBadge, PendingBanner } from "./pending-changes-badge";

vi.mock("./apply-dialog", () => ({ ApplyDialog: () => null }));

function diff(over: Partial<ConfigDiff>): ConfigDiff {
  return { servers: [], streams: [], upstreams: [], hasChanges: true, ...over };
}
it.each([0, 2])(
  "an unconfirmed baseline warns even with %s changes",
  (count) => {
    const html = renderToStaticMarkup(
      createElement(PendingChangesBadge, {
        count,
        hasBaseline: false,
        diff: diff({ hasChanges: count > 0 }),
      }),
    );
    expect(html).toContain("적용 상태 확인 필요");
    expect(html).not.toContain("적용됨");
  },
);
it("a confirmed empty configuration is applied", () => {
  expect(
    renderToStaticMarkup(
      createElement(PendingChangesBadge, {
        count: 0,
        hasBaseline: true,
        diff: diff({ hasChanges: false }),
      }),
    ),
  ).toContain("적용됨");
});

it("pending banner is presentational and exposes review/dismiss actions", () => {
  const onReview = vi.fn(),
    onDismiss = vi.fn();
  const tree = PendingBanner({ count: 3, onReview, onDismiss });
  const html = renderToStaticMarkup(tree);
  expect(html).toContain('role="status"');
  expect(html).toContain("3건");
  expect(html).toContain("변경 내용 보기");
  expect(html).toContain('aria-label="미적용 변경 알림 닫기"');
  expect(html).not.toContain("href=");
  expect(PendingBanner({ count: 0, onReview, onDismiss })).toBeNull();
});

it("confirmed applied state uses the success dot", () => {
  const html = renderToStaticMarkup(
    createElement(PendingChangesBadge, {
      count: 0,
      hasBaseline: true,
      diff: diff({ hasChanges: false }),
    }),
  );
  expect(html).toContain('data-tone="success"');
  expect(html).toContain("모두 적용됨");
});
