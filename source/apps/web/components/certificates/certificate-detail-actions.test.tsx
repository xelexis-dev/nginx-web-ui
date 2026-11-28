import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import { Button, ConfirmDialog } from "@repo/ui";
import { CertificateDetailActions, type CertificateDetailActionsProps } from "./certificate-detail-actions";

const hooks = vi.hoisted(() => ({ state: null as string | null, refs: [] as { current: unknown }[], cursor: 0, pending: false }));
vi.mock("react", async (original) => ({
  ...await original<typeof import("react")>(),
  useActionState: () => [{ ok: false }, vi.fn(), hooks.pending],
  useState: () => [hooks.state, (value: string | null) => { hooks.state = value; }],
  useRef: (value: unknown) => hooks.refs[hooks.cursor++] ?? (hooks.refs[hooks.cursor - 1] = { current: value }),
}));
vi.mock("@/lib/use-api-action", () => ({ useApiAction: () => vi.fn() }));
vi.mock("next/navigation", () => ({ useRouter: () => ({ refresh: vi.fn(), push: vi.fn() }) }));
const props: CertificateDetailActionsProps = {
  id: "cert-1", targetName: "api.example.test", cancelPath: "/cancel", editHref: "/edit", replaceHref: "/replace",
  status: "issued", linkedCount: 0, source: "manual", challengeType: "http-01", dnsAutomated: false, dnsOrderInFlight: false, dnsVerifying: false,
  retryAction: { id: "certificates.retryCertificate", args: [] }, renewAction: { id: "certificates.renewCertificate", args: [] },
  renewDnsAction: { id: "certificates.renewDnsCertificate", args: [] }, deleteAction: { id: "certificates.deleteCertificate", args: [] },
};
function elements(node: ReactNode): ReactElement<Record<string, unknown>>[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement(node)) return [];
  const element = node as ReactElement<Record<string, unknown>>;
  return [element, ...elements(element.props.children as ReactNode)];
}
function render(over: Partial<CertificateDetailActionsProps> = {}) {
  hooks.cursor = 0;
  const tree = elements(CertificateDetailActions({ ...props, ...over }));
  return { form: tree.find((element) => element.type === "form")!, dialog: tree.find((element) => element.type === ConfirmDialog)! };
}
const submit = (form: ReactElement<Record<string, unknown>>) => {
  const event = { preventDefault: vi.fn() };
  (form.props.onSubmit as (event: unknown) => void)(event);
  return event.preventDefault;
};
beforeEach(() => { hooks.state = null; hooks.refs = []; hooks.pending = false; });
describe("certificate delete confirmation", () => {
  it("cancel never submits; confirmation resubmits the same native form exactly once", () => {
    let view = render();
    expect(submit(view.form)).toHaveBeenCalledOnce();
    view = render();
    expect(view.dialog.props.open).toBe(true);
    (view.dialog.props.onClose as () => void)();
    expect(render().dialog.props.open).toBe(false);
    submit(view.form);
    view = render();
    const requestSubmit = vi.fn();
    hooks.refs[0]!.current = { requestSubmit };
    (view.dialog.props.onConfirm as () => void)();
    expect(requestSubmit).toHaveBeenCalledOnce();
    expect(submit(render().form)).not.toHaveBeenCalled();
    expect(submit(render().form)).toHaveBeenCalledOnce();
  });
  it.each([{ linkedCount: 1 }, { id: "cert-2" }, { pending: true }])("refuses changed authority or pending: %j", (over) => {
    submit(render().form);
    hooks.pending = "pending" in over;
    const view = render(over);
    const requestSubmit = vi.fn();
    hooks.refs[0]!.current = { requestSubmit };
    (view.dialog.props.onConfirm as () => void)();
    expect(requestSubmit).not.toHaveBeenCalled();
    expect(submit(view.form)).toHaveBeenCalledOnce();
  });
});

it("keeps delete outlined on the page and danger inside its existing confirmation", () => {
  const { form, dialog } = render();
  const button = elements(form.props.children as ReactNode).find(e => e.type === Button)!;
  expect(button.props.variant).toBe("dangerOutline");
  expect(dialog.props.confirmLabel).toBe("삭제");
});

it("keeps the linked certificate's delete outlined and disabled", () => {
  const { form } = render({ linkedCount: 1 });
  const button = elements(form.props.children as ReactNode).find(e => e.type === Button);
  expect(button?.props).toMatchObject({
    variant: "dangerOutline",
    disabled: true,
    children: "삭제 (연결 해제 필요)",
  });
});

it("orders ACME actions before delete and keeps edit last", () => {
  hooks.cursor = 0;
  const tree = elements(CertificateDetailActions({ ...props, source: "acme", status: "issued" }));
  const row = tree.find(element => typeof element.props.className === "string" && element.props.className.includes("actionsRow"))!;
  const controls = elements(row.props.children as ReactNode).filter(element => element.type === Button || element.props.href === props.editHref);
  expect(controls.map(element => element.props.children)).toEqual(["지금 갱신", "삭제", "설정 수정"]);
});
