import { expect, it } from "vitest";
import {
  DIFF_GROUPS,
  diffCounts,
  pendingIntro,
  serverLabel,
  sortedItems,
} from "./config-diff-summary";
import type { ConfigDiff, ItemDiff } from "./config-diff";
const items: ItemDiff[] = ["added", "removed", "changed", "skipped"].map(
  (status, i) => ({
    key: String(i),
    label: String(i),
    status: status as ItemDiff["status"],
    changes: [],
  }),
);
const diff: ConfigDiff = {
  hasChanges: true,
  servers: items,
  streams: [],
  upstreams: [],
};
it("sorts groups and kinds, and counts kept-previous as excluded", () => {
  expect(DIFF_GROUPS).toEqual(["servers", "upstreams", "streams"]);
  expect(sortedItems(items).map((i) => i.status)).toEqual([
    "skipped",
    "changed",
    "removed",
    "added",
  ]);
  expect(
    diffCounts({ ...diff, servers: [{ ...items[2]!, reason: "kept" }] }),
  ).toEqual({ added: 0, changed: 0, removed: 0, skipped: 1 });
});
it("splits multi-name server labels at the last colon", () =>
  expect(serverLabel("a b:80")).toEqual({ name: "a b", port: "80" }));
it("covers normal, mixed, excluded, viewer and unknown-baseline introductions", () => {
  expect(pendingIntro({ ...diff, servers: [items[0]!] })).toContain("nginx -t");
  expect(pendingIntro(diff)).toContain("다시 적용해도 제외됩니다");
  expect(pendingIntro({ ...diff, servers: [items[3]!] })).toBe(
    "아래 항목은 적용에서 제외돼 있습니다. 표시된 이유를 고친 뒤 다시 적용하세요.",
  );
  expect(pendingIntro(diff, false)).toContain("적용 권한이 있는 사용자");
  expect(pendingIntro(diff, true, false)).toContain(
    "마지막으로 적용된 설정을 확인하지 못해",
  );
});
