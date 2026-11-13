import { afterEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { NodeActionState } from "@/lib/node-action-state";
import { HeaderActions } from "../operations/header-actions";

import { StreamDetail } from "../streams/stream-detail";
import { UpstreamDetail } from "../upstreams/upstream-detail";
import { StreamForm } from "../streams/stream-form";
import { UpstreamForm } from "../upstreams/upstream-form";
import { ServerTemplates } from "./server-templates";
const mocked = vi.hoisted(() => ({ state: { ok: false } as NodeActionState }));
vi.mock("react", async original => ({ ...await original<typeof import("react")>(), useActionState: () => [mocked.state, () => {}, false] }));
afterEach(() => { mocked.state = { ok: false }; });
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }) }));


it("marks an applied baseline with the success dot and an unknown baseline with warning", () => {
  for (const baseline of [true, false]) {
    const html = renderToStaticMarkup(<HeaderActions canApply={false} pendingCount={0}
      pendingHasBaseline={baseline} onApply={() => {}} />);
    expect(html).toContain(baseline ? 'data-tone="success"' : 'data-tone="warning"');
    expect(html).toContain(baseline ? "모두 적용됨" : "적용 상태 확인 필요");
  }
});

it.each(["upstream", "stream"])("keeps %s save/cancel outside the category card in one form", (kind) => {
  const action = { id: kind === "stream" ? "stream.createStream" : "upstreams.createUpstream", args: [] };
  const html = renderToStaticMarkup(kind === "stream"
    ? <StreamForm action={action} cancelHref="/stream" />
    : <UpstreamForm action={action} cancelHref="/reverse-proxy/upstreams" />);
  expect(html.match(/<form\b/g)).toHaveLength(1);
  expect(html.match(/<footer\b/g)).toHaveLength(1);
  expect(html.indexOf("</section>")).toBeLessThan(html.indexOf("<footer"));
  expect(html.indexOf("취소")).toBeLessThan(html.indexOf('type="submit"'));
  expect(html.match(/nwui-btn-primary/g)).toHaveLength(1);
});

it("keeps repeated template creation secondary and hides it from viewers", () => {
  const items = ["a", "b"].map(id => ({ id, name: id, description: id, tags: [] }));
  const editor = renderToStaticMarkup(<ServerTemplates items={items} canWrite />);
  expect(editor).not.toContain("nwui-btn-primary");
  expect(editor.match(/이 템플릿으로 생성/g)).toHaveLength(2);
  const viewer = renderToStaticMarkup(<ServerTemplates items={items} canWrite={false} />);
  expect(viewer).not.toContain("이 템플릿으로 생성");
  expect(viewer).not.toContain("빈 폼으로 생성");
});

it.each(["upstream", "stream"])("shows %s field errors in the card and linked summary", kind => {
  mocked.state = { ok: false, error: "입력값을 확인하세요", fieldErrors: kind === "stream"
    ? { listen: "포트 오류", resolveAtRuntime: "이름 확인 오류" }
    : { name: "이름 오류", resolveAtRuntime: "이름 확인 오류" } };
  const action = { id: kind === "stream" ? "stream.createStream" : "upstreams.createUpstream", args: [] };
  const html = renderToStaticMarkup(kind === "stream" ? <StreamForm action={action} cancelHref="/stream" /> : <UpstreamForm action={action} cancelHref="/reverse-proxy/upstreams" />);
  expect(html).toContain('data-error=""');
  expect(html).toContain("오류 2");
  expect(html).toContain(`href="#${kind}-runtime-dns"`);
  expect(html).toContain(`id="${kind}-runtime-dns"`);
  expect(html).toContain(`href="#${kind === "stream" ? "stream-listen" : "upstream-name"}"`);
  expect(html.indexOf('role="alert"')).toBeLessThan(html.indexOf('<section'));
});

it.each(["/reverse-proxy/upstreams", "/nodes/n1/reverse-proxy/upstreams"])("keeps upstream detail actions in the header and hides viewer writes: %s", basePath => {
  for (const canWrite of [true, false]) {
    const html = renderToStaticMarkup(<UpstreamDetail basePath={basePath} data={{ id: "u1", name: "pool", servers: ["origin:8080"] }} canWrite={canWrite} deleteAction={{ id: "upstreams.deleteUpstream", args: [] }} />);
    expect(html.includes(`${basePath}/u1/edit`)).toBe(canWrite);
    if (canWrite) expect(html.indexOf('<form')).toBeLessThan(html.indexOf('</header>'));
    else expect(html).not.toContain('<form');
  }
});

it.each(["/stream", "/nodes/n1/stream"])("keeps Stream toggle/delete in the header and hides viewer writes: %s", basePath => {
  for (const canWrite of [true, false]) {
    const html = renderToStaticMarkup(<StreamDetail basePath={basePath} data={{ id: "s1", listen: 15432, protocol: "tcp", proxyPass: "origin:5432", enabled: true }} canWrite={canWrite} toggleAction={{ id: "stream.toggleStream", args: [] }} deleteAction={{ id: "stream.deleteStream", args: [] }} />);
    expect(html.includes(`${basePath}/s1/edit`)).toBe(canWrite);
    if (canWrite) expect(html.indexOf('<form')).toBeLessThan(html.indexOf('</header>'));
    else expect(html).not.toContain('<form');
  }
});

vi.mock("@/lib/node-page", () => ({
  loadNodePage: async () => ({ ok: true, effectiveRole: "viewer", data: [] }),
  loadNodePageAll: async () => ({ ok: true, effectiveRole: "viewer", data: { servers: [], upstreamNameById: new Map(), pendingCount: 0, pendingHasBaseline: true, pendingDiff: {} } }),
}));
vi.mock("./certificate-read", () => ({ readNodeCertificateSummaries: async () => [] }));

it.each(["server", "upstream"])("renders only real proxy tabs on the node %s list", async kind => {
  const page = kind === "server" ? (await import("@/app/(node)/nodes/[id]/reverse-proxy/page")).default
    : (await import("@/app/(node)/nodes/[id]/reverse-proxy/upstreams/page")).default;
  const html = renderToStaticMarkup(await page({ params: Promise.resolve({ id: "n1" }) }));
  expect(html).toContain('aria-label="프록시 구성"');
  expect(html).toContain('href="/nodes/n1/reverse-proxy"');
  expect(html).toContain('href="/nodes/n1/reverse-proxy/upstreams"');
  expect(html).not.toContain('/nodes/n1/reverse-proxy/templates');
  expect(html).not.toContain('Upstreams 관리');
});

it.each(["upstream", "stream"])("places the unchanged unapplied info before the %s category card", kind => {
  const html = renderToStaticMarkup(kind === "stream"
    ? <StreamForm action={{ id: "stream.createStream", args: [] }} cancelHref="/stream" />
    : <UpstreamForm action={{ id: "upstreams.createUpstream", args: [] }} cancelHref="/reverse-proxy/upstreams" />);
  const info = "저장하면 ‘미적용’ 상태가 됩니다. nginx에는 상단 바의 ‘적용’을 눌러야 반영되고, nginx -t 검사에 실패하면 현재 설정이 그대로 유지됩니다.";
  expect(html).toContain(info);
  expect(html.indexOf(info)).toBeLessThan(html.indexOf(kind === "stream" ? 'id="stream-settings"' : 'id="upstream-form-fields"'));
  if (kind === "upstream") expect(html).toMatch(/id="upstream-form-fields-heading"[^>]*>Upstream<\/h2>/);
});
