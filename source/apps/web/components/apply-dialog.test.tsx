import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, expect, it, vi } from "vitest";
import { ApplyDialog, type ApplyDialogProps } from "./apply-dialog";
const hooks = vi.hoisted(() => ({ pending: false }));
vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  useId: () => "apply-form",
  useActionState: () => [null, () => {}, hooks.pending],
}));
vi.mock("@/lib/use-api-action", () => ({
  useApiAction: (action: unknown) => action,
}));
vi.mock("./notifications/toast-provider", () => ({
  useToast: () => ({ toast: () => {} }),
}));
const props: ApplyDialogProps = {
  open: true,
  onClose: () => {},
  canApply: true,
  target: { kind: "local" },
  targetLabel: "로컬 서버",
  pendingCount: 1,
  pendingHasBaseline: true,
  pendingDiff: {
    hasChanges: true,
    servers: [{ key: "s", label: "test:80", status: "added", changes: [] }],
    streams: [],
    upstreams: [],
  },
  applyAction: async () => ({ ok: true }),
};
beforeEach(() => {
  hooks.pending = false;
});
it("puts external submit and cancel in the fixed Modal footer", () => {
  const tree = ApplyDialog(props);
  const foot = renderToStaticMarkup(tree.props.footer),
    body = renderToStaticMarkup(tree.props.children);
  expect(foot).toContain('type="submit"');
  expect(foot).toContain('form="apply-form"');
  expect(foot).toContain("취소");
  expect(body).not.toContain('type="submit"');
  expect(body).toContain('<form id="apply-form"');
  expect(tree.props.size).toBe("review");
});
it("locks every close seam and both buttons while pending", () => {
  hooks.pending = true;
  const tree = ApplyDialog(props);
  expect(tree.props.dismissible).toBe(false);
  const html = renderToStaticMarkup(tree.props.footer);
  expect(html.match(/disabled=""/g)).toHaveLength(2);
  expect(html).toContain('aria-busy="true"');
  expect(html).toContain('role="progressbar"');
  expect(renderToStaticMarkup(tree.props.children)).toContain(
    'aria-busy="true"',
  );
});
it("gives viewer and known-empty only a close button", () => {
  for (const p of [
    { ...props, canApply: false },
    { ...props, pendingCount: 0 },
  ]) {
    const html = renderToStaticMarkup(<ApplyDialog {...p} />);
    expect(html).not.toContain('type="submit"');
    expect(html).toContain("닫기");
  }
  expect(ApplyDialog({ ...props, pendingCount: 0 }).props.size).toBe("default");
});
it("keeps apply for unknown baseline with zero items and never claims all applied", () => {
  const html = renderToStaticMarkup(
    <ApplyDialog {...props} pendingHasBaseline={false} pendingCount={0} />,
  );
  expect(html).toContain('type="submit"');
  expect(html).toContain("적용 상태 확인 필요");
  expect(html).not.toContain("모두 적용됨");
});
