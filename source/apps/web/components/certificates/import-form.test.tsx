import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { NodeActionState, NodeFormAction } from "@/lib/node-action-state";

/**
 * The import form carries a private key. What it submits is the text in the hidden `certPem` and
 * `keyPem` fields and nothing else: the file choosers have no name, so a chosen file never leaves
 * the browser (the header of import-form.tsx). These checks hold for every route that renders it.
 */

const mocks = vi.hoisted(() => ({ state: { ok: false } as NodeActionState }));
vi.mock("react", async (importOriginal) => ({
  ...(await importOriginal<typeof import("react")>()),
  useActionState: () => [mocks.state, () => undefined, false],
}));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }) }));

import { ImportForm } from "./import-form";

// The four routes, with the actions and paths they pass (app/(app)/certificates and
// app/(node)/nodes/[id]/certificates).
const ROUTES: { name: string; action: NodeFormAction; certId?: string; cancelHref: string; submit: string }[] = [
  { name: "local import", action: { id: "certificates.importCertificate", args: [] }, cancelHref: "/certificates", submit: "가져오기" },
  { name: "local replace", action: { id: "certificates.replaceCertificate", args: [] }, certId: "c1", cancelHref: "/certificates", submit: "교체" },
  { name: "node import", action: { id: "node.importNodeCertificate", args: ["n1"] }, cancelHref: "/nodes/n1/certificates", submit: "가져오기" },
  { name: "node replace", action: { id: "node.replaceNodeCertificate", args: ["n1"] }, certId: "c9", cancelHref: "/nodes/n1/certificates", submit: "교체" },
];

const tags = (html: string, tag: string) => html.match(new RegExp(`<${tag}\\b[^>]*>`, "g")) ?? [];
const attr = (tag: string, name: string) => new RegExp(`\\s${name}="([^"]*)"`).exec(tag)?.[1];

describe.each(ROUTES)("ImportForm on $name", ({ action, certId, cancelHref, submit }) => {
  mocks.state = { ok: false };
  const html = renderToStaticMarkup(<ImportForm action={action} certId={certId} cancelHref={cancelHref} />);
  const inputs = tags(html, "input");
  const files = inputs.filter((t) => attr(t, "type") === "file");

  it("keeps one category card and one native submit/cancel footer", () => {
    expect(tags(html, "footer")).toHaveLength(1);
    expect(html).toContain('aria-labelledby="certificate-pem-heading"');
    expect(tags(html, "button").filter(t => attr(t, "type") === "submit")).toHaveLength(1);
    expect(html).toContain('href="' + cancelHref + '"');
  });

  it("has one chooser per PEM, and neither is submitted", () => {
    expect(files).toHaveLength(2);
    for (const f of files) expect(attr(f, "name")).toBeUndefined();
    expect(files.map((f) => attr(f, "accept"))).toEqual([".pem,.crt,.cer,.txt", ".pem,.key,.txt"]);
  });

  it("names each chooser for what it loads", () => {
    expect(files.map((f) => attr(f, "aria-label"))).toEqual(["인증서 파일 선택", "개인키 파일 선택"]);
  });

  it("submits the PEM text only through the hidden fields", () => {
    const named = inputs.map((t) => attr(t, "name")).filter(Boolean);
    expect(named.sort()).toEqual((certId ? ["certPem", "id", "keyPem"] : ["certPem", "keyPem"]).sort());
    for (const t of tags(html, "textarea")) expect(attr(t, "name")).toBeUndefined();
  });

  it(certId ? "replaces that certificate" : "registers a new certificate", () => {
    const id = inputs.find((t) => attr(t, "name") === "id");
    expect(id ? attr(id, "value") : undefined).toBe(certId);
    expect(html).toMatch(new RegExp(`<button[^>]*type="submit"[^>]*>${submit}</button>`));
    expect(html).toContain(`href="${cancelHref}"`);
  });
});

describe("ImportForm failures", () => {
  const render = () => renderToStaticMarkup(<ImportForm action={{ id: "node.importNodeCertificate", args: ["n1"] }} cancelHref="/nodes/n1/certificates" />);

  it("a PEM refusal is one line under the fields", () => {
    mocks.state = { ok: false, error: "개인키 PEM을 해석할 수 없습니다", fieldErrors: { keyPem: "개인키가 비어 있습니다" } };
    const html = render();
    expect(html).toContain("개인키 PEM을 해석할 수 없습니다");
    expect(html).toContain("개인키가 비어 있습니다");
  });

  it("a node failure shows its card instead of repeating the line", () => {
    mocks.state = {
      ok: false,
      error: "노드에 연결하지 못했습니다",
      nodeError: { failure: { kind: "unreachable", message: "노드에 연결하지 못했습니다" }, nodeId: "n1", retryHref: "/nodes/n1/certificates/import", listHref: "/nodes/n1/certificates" },
    };
    const html = render();
    expect(html.split("노드에 연결하지 못했습니다").length - 1).toBe(1);
  });
});
