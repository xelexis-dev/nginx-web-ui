import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { ConfigDiffView } from "./config-diff-view";
import type { ConfigDiff } from "@/lib/config-diff";
const diff: ConfigDiff = {
  hasChanges: true,
  servers: [
    { key: "a", label: "added.test", status: "added", changes: [] },
    { key: "r", label: "removed.test", status: "removed", changes: [] },
    {
      key: "c",
      label: "changed.test",
      status: "changed",
      changes: [{ field: "주소", before: "<old>", after: "<new>" }],
    },
    {
      key: "s",
      label: "skipped.test",
      status: "skipped",
      changes: [],
      reason: "예약된 포트입니다.",
    },
  ],
  streams: [],
  upstreams: [],
};
it.each(["inline", "panel"] as const)(
  "%s keeps markers, escaped values and skipped reasons",
  (variant) => {
    const html = renderToStaticMarkup(
      <ConfigDiffView diff={diff} variant={variant} />,
    );
    expect(html).toContain('data-variant="' + variant + '"');
    for (const marker of ["+", "−", "~", "!"]) expect(html).toContain(marker);
    expect(html).toContain("&lt;old&gt;");
    expect(html).toContain("&lt;new&gt;");
    expect(html).not.toContain("<old>");
    expect(html).toContain("적용에서 제외됨");
    expect(html).toContain("예약된 포트입니다.");
  },
);
it("defaults to the inline review presentation", () => {
  expect(renderToStaticMarkup(<ConfigDiffView diff={diff} />)).toContain(
    'data-variant="inline"',
  );
});
