import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { StreamForm } from "./stream-form";
vi.mock("react", async (original) => ({ ...await original<typeof import("react")>(), useActionState: () => [{ ok: false }, () => undefined, false] }));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }) }));
const action = { id: "stream.createStream", args: [] } as const;
describe("Stream form composition", () => {
 it("keeps the local port empty, with no remote target label", () => {
  const html = renderToStaticMarkup(<StreamForm action={{ ...action, args: [] }} cancelHref="/stream" />);
  expect(html).toMatch(/name="listen"[^>]*value=""/);
  expect(html).not.toContain("저장 대상:");
  expect(html).toContain('id="stream-settings-heading"');
  expect(html).toContain("<footer");
  expect(html.indexOf("</section>")).toBeLessThan(html.indexOf("<footer"));
 });
 it("shows the typed node name beside save without changing the hidden resource/version", () => {
  const html = renderToStaticMarkup(<StreamForm target={{ kind: "node", id: "n1", name: "서울 노드" }} action={{ id: "node.updateNodeStream", args: ["n1"] }} cancelHref="/nodes/n1/stream/s1" defaultValues={{ id: "s1", version: "v1", listen: 15432, protocol: "tcp", proxyPass: "database.example.test:5432" }} />);
  expect(html).toContain("저장 대상: 서울 노드");
  expect(html).toContain('name="id" value="s1"');
  expect(html).toContain('name="version" value="v1"');
 });
});
