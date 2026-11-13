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
  StatusBadge: ({ children, tone }: { children: React.ReactNode; tone: string }) =>
    createElement("span", { "data-tone": tone }, children),
}));
vi.mock("./config-diff-view", () => ({ ConfigDiffView: () => null }));
import {
  PendingChangesBadge,
  PendingBanner,
  pendingIntro,
} from "./pending-changes-badge";

function diff(over: Partial<ConfigDiff>): ConfigDiff {
  return { servers: [], streams: [], upstreams: [], hasChanges: true, ...over };
}
const added = {
  key: "n",
  label: "fresh.test:80",
  status: "added" as const,
  changes: [],
};
const skipped = {
  key: "s",
  label: "broken.test:80",
  status: "skipped" as const,
  changes: [],
  reason: "포트 3001 는 …",
};

it("변경만 있으면 적용하면 반영된다고 안내한다", () => {
  expect(pendingIntro(diff({ servers: [added] }))).toBe(
    '"설정 적용"을 누르면 아래 내용이 nginx에 반영됩니다.',
  );
});

// 제외된 행은 적용을 눌러도 다시 제외된다 — 「반영됩니다」만 쓰면 사실과 다르다.
it("제외된 행이 섞여 있으면 그 행은 고쳐야 반영된다고 덧붙인다", () => {
  const intro = pendingIntro(diff({ servers: [added], streams: [skipped] }));
  expect(intro).toContain("반영됩니다");
  expect(intro).toContain("적용에서 제외됨");
  expect(intro).toContain("다시 적용해도");
});

it("제외된 행만 있으면 반영된다고 말하지 않는다", () => {
  const intro = pendingIntro(diff({ upstreams: [skipped] }));
  expect(intro).not.toContain("반영됩니다");
  expect(intro).toContain("이유");
});

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

it.each(["로컬 서버", "서울 운영 노드"])(
  "review intro names the actual %s confirmation",
  (target) => {
    expect(pendingIntro(diff({ servers: [added] }), target + "에 적용")).toBe(
      '"' + target + '에 적용"을 누르면 아래 내용이 nginx에 반영됩니다.',
    );
    expect(
      pendingIntro(diff({ servers: [added, skipped] }), target + "에 적용"),
    ).toContain('"' + target + '에 적용"을 누르면');
    expect(
      pendingIntro(diff({ servers: [added, skipped] }), target + "에 적용"),
    ).toContain("다시 적용해도 제외됩니다.");
    expect(pendingIntro(diff({ servers: [skipped] }), target + "에 적용")).toBe(
      "아래 항목은 적용에서 제외돼 있습니다. 표시된 이유를 고친 뒤 다시 적용하세요.",
    );
  },
);

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
  const html=renderToStaticMarkup(createElement(PendingChangesBadge,{count:0,hasBaseline:true,diff:diff({hasChanges:false})}));
  expect(html).toContain('data-tone="success"');
  expect(html).toContain("모두 적용됨");
});
