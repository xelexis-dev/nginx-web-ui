import { beforeEach, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import { Button, ConfirmDialog } from "@repo/ui";
import { ServerDetailActions } from "./reverse-proxy/detail-actions";
import { UpstreamDetailActions } from "./upstreams/detail-actions";
import { StreamDetailActions } from "./streams/detail-actions";
import { UserDetailActions } from "@/app/admin/users/[id]/detail-actions";
import { MailRecipientDetailActions } from "@/app/admin/notifications/mail/[id]/detail-actions";
import { WebhookDetailActions } from "@/app/admin/notifications/webhooks/[id]/detail-actions";
import { ProviderDetailActions } from "@/app/admin/oauth/provider-detail-actions";
const h = vi.hoisted(() => ({ state: null as string | null, refs: [] as { current: unknown }[], cursor: 0, pending: false, submit: vi.fn() }));
vi.mock("react", async (original) => ({
  ...await original<typeof import("react")>(),
  useActionState: () => [{ ok: false }, h.submit, h.pending],
  useState: () => [h.state, (value: string | null) => { h.state = value; }],
  useRef: (value: unknown) => h.refs[h.cursor++] ?? (h.refs[h.cursor - 1] = { current: value }),
}));
vi.mock("@/lib/use-api-action", () => ({ useApiAction: () => vi.fn() }));
const action = { id: "delete", args: [] };
const cases = [
  ["server", (id: string) => ServerDetailActions({ id, editHref: "/edit", serverName: "api.example.test", enabled: true, toggleAction: action, deleteAction: action })],
  ["upstream", (id: string) => UpstreamDetailActions({ id, targetName: "api-pool", editHref: "/edit", action })],
  ["stream", (id: string) => StreamDetailActions({ id, targetName: "listen 5432 · TCP", editHref: "/edit", enabled: true, toggleAction: action, deleteAction: action })],
  ["user", (id: string) => UserDetailActions({ id, targetName: "member@example.test", isSelf: false, enabled: true })],
  ["mail", (id: string) => MailRecipientDetailActions({ id, address: "ops@example.test" })],
  ["webhook", (id: string) => WebhookDetailActions({ id, targetName: "Operations", format: "generic", bodyTemplate: null })],
  ["oauth", (id: string) => ProviderDetailActions({ id, targetName: "SSO" })],
] as const;
function elements(node: ReactNode): ReactElement<Record<string, unknown>>[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<Record<string, unknown>>(node)) return [];
  return [node, ...elements(node.props.children as ReactNode)];
}
function render(build: (id: string) => ReactNode, id = "item-1") {
  h.cursor = 0; const tree = elements(build(id));
  return { tree, form: tree.find((node) => node.type === "form" && node.props.ref)!, dialog: tree.find((node) => node.type === ConfirmDialog)! };
}
function submit(form: ReactElement<Record<string, unknown>>) {
  const event = { preventDefault: vi.fn() };
  (form.props.onSubmit as (event: unknown) => void)(event);
  if (!event.preventDefault.mock.calls.length) (form.props.action as () => void)();
  return event.preventDefault;
}
beforeEach(() => { h.state = null; h.refs = []; h.cursor = 0; h.pending = false; vi.clearAllMocks(); });
it.each(cases)("%s confirms exactly one delete and cancel submits nothing", (_, build) => {
  let view = render(build);
  expect(submit(view.form)).toHaveBeenCalledOnce(); view = render(build);
  expect(view.dialog.props.open).toBe(true);
  (view.dialog.props.onClose as () => void)(); expect(render(build).dialog.props.open).toBe(false);
  expect(h.submit).not.toHaveBeenCalled();
  submit(render(build).form); view = render(build);
  const requestSubmit = vi.fn(() => submit(render(build).form));
  (view.form.props.ref as { current: unknown }).current = { requestSubmit };
  (view.dialog.props.onConfirm as () => void)();
  expect(requestSubmit).toHaveBeenCalledOnce(); expect(h.submit).toHaveBeenCalledOnce();
  expect(submit(render(build).form)).toHaveBeenCalledOnce(); expect(h.submit).toHaveBeenCalledOnce();
  expect(elements(view.form.props.children as ReactNode).find((node) => node.type === "input")?.props).toMatchObject({ name: "id", value: "item-1" });
  expect(elements(view.form.props.children as ReactNode).find((node) => node.type === Button)?.props.variant).toBe("dangerQuiet");
});
it.each(cases)("%s refuses confirmation after the id changes or while pending", (_, build) => {
  submit(render(build).form);
  let view = render(build, "different-id"); const requestSubmit = vi.fn();
  (view.form.props.ref as { current: unknown }).current = { requestSubmit };
  (view.dialog.props.onConfirm as () => void)(); expect(requestSubmit).not.toHaveBeenCalled();
  h.pending = true; view = render(build);
  (view.dialog.props.onConfirm as () => void)(); expect(requestSubmit).not.toHaveBeenCalled();
  expect(submit(view.form)).toHaveBeenCalledOnce(); expect(h.submit).not.toHaveBeenCalled();
});
it("keeps self-delete disabled", () => {
  h.cursor = 0; const tree = elements(UserDetailActions({ id: "self", isSelf: true, enabled: true }));
  expect(tree.find((node) => node.type === Button && node.props.variant === "dangerQuiet")?.props.disabled).toBe(true);
  expect(tree.filter((node) => node.type === "form" && node.props.ref)).toHaveLength(0);
});

import { renderToStaticMarkup } from "react-dom/server";
import { EmptyState } from "@repo/ui";
import { PendingListView } from "@/app/admin/pending/pending-list";
import { HistoryList } from "./history/history-list";
import { AdminTwoFactor } from "@/app/admin/users/admin-two-factor";
import type { AdminUserView, PendingUserView } from "@/lib/admin-api";
it("keeps the pending count outside the h1", () => {
  const html = renderToStaticMarkup(<PendingListView pending={[{ id: "pending", email: "pending@example.test", createdAt: "2026-11-18T00:00:00Z" } as PendingUserView]} approvalForm={() => null} rejectionForm={() => null} />);
  expect(html.match(/<h1[^>]*>(.*?)<\/h1>/)?.[1]).toBe("승인 대기");
  expect(html).toContain('data-tag=""'); expect(html).toContain("1명");
});
it("moves the node history notice to the page description without a banner", () => {
  const notice = "스냅샷 내용은 노드에서만 볼 수 있습니다.";
  const html = renderToStaticMarkup(<HistoryList data={{ items: [], showDetails: false }} info="노드 이력" notice={notice} />);
  expect(html).toContain(notice);
  expect(html).not.toContain('role="status"');
  expect(html.indexOf(notice)).toBeLessThan(html.indexOf("<table"));
});
it("uses the shared empty sentence for an unconfigured user's 2FA", () => {
  const tree = elements(AdminTwoFactor({ user: { totpEnabled: false } as AdminUserView, isSelf: false, canReset: true, passkeyCount: 0 }));
  expect(tree.find((node) => node.type === EmptyState)?.props.description).toBe("설정된 2FA가 없어 초기화할 항목이 없습니다.");
});
