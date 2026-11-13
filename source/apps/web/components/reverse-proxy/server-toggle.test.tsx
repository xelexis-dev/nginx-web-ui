import { expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { NodeActionState } from "@/lib/node-action-state";

/**
 * The row switch's failure line. A node failure can come with `nodeError` only (no `error` line):
 * the row must still say what happened — the failure's own message — and offer the full card
 * (「자세히」), not collapse it into a generic sentence.
 */

const mocks = vi.hoisted(() => ({ state: { ok: false } as NodeActionState }));
vi.mock("react", async (importOriginal) => ({
  ...(await importOriginal<typeof import("react")>()),
  useActionState: () => [mocks.state, () => undefined, false],
}));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }) }));

import { ServerToggle } from "./server-toggle";

const render = () => renderToStaticMarkup(<ServerToggle id="s1" serverName="api.example.test" enabled action={{ id: "node.toggleNodeServer", args: ["n1"] }} />);

it("no failure: just the switch", () => {
  mocks.state = { ok: false };
  const html = render();
  expect(html).toContain('role="switch"');
  expect(html).not.toContain('role="alert"');
});

it("a node failure without an error line shows the failure's own message and offers the card", () => {
  mocks.state = {
    ok: false,
    nodeError: { failure: { kind: "version_mismatch", message: "노드 버전이 맞지 않습니다 (1.17.2)", nodeVersion: "1.17.2" }, nodeId: "n1", retryHref: "/nodes/n1/reverse-proxy", listHref: "/nodes/n1/reverse-proxy" },
  };
  const html = render();
  expect(html).toMatch(/role="alert"[^>]*><span>노드 버전이 맞지 않습니다 \(1\.17\.2\)<\/span>/);
  expect(html).toContain("자세히");
  expect(html).not.toContain("반영하지 못했습니다.");
});

it("a local failure line is shown as is, with no card to open", () => {
  mocks.state = { ok: false, error: "API 서버에 연결하지 못했습니다." };
  const html = render();
  expect(html).toContain("API 서버에 연결하지 못했습니다.");
  expect(html).not.toContain("자세히");
});
