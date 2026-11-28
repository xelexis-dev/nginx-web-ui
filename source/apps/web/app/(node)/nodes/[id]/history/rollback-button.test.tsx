import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { NodeResult } from "@/lib/node-result";

const mock = vi.hoisted(() => ({
  result: null as NodeResult<{ output: string }> | null,
  pending: false,
  action: vi.fn(),
}));
vi.mock("react", async original => ({
  ...(await original<typeof import("react")>()),
  useActionState: () => [mock.result, () => undefined, mock.pending],
}));
vi.mock("@/lib/use-api-action", () => ({ useApiAction: mock.action }));
import { NodeRollbackButton } from "./rollback-button";

const props = { nodeId: "n1", nodeName: "노드 A", snapshotId: "s2", label: "이전 설정",
  iso: "2026-09-28T04:20:00Z", retryHref: "/nodes/n1/history/s2", listHref: "/nodes/n1/history" };
const render = () => renderToStaticMarkup(<NodeRollbackButton {...props} />);
beforeEach(() => { mock.result = null; mock.pending = false; mock.action.mockClear(); });

it("binds rollback to the selected node and snapshot", () => {
  const html = render();
  expect(mock.action).toHaveBeenCalledWith({ id: "node.rollbackNodeSnapshot", args: ["n1", "s2"] });
  expect(html).toContain('type="submit"');
  expect(html).toContain("이 스냅샷으로 롤백");
  expect(html).not.toContain("노드에서 열기");
});
it("disables the rollback submission while pending", () => {
  mock.pending = true;
  expect(render()).toMatch(/<button[^>]*disabled=""[^>]*>[\s\S]*?롤백 중/);
});
it("keeps successful output below the controls", () => {
  mock.result = { ok: true, value: { output: "syntax is ok" }, nodeVersion: "1.19.6" };
  const html = render();
  expect(html).toContain('role="status"');
  expect(html).toContain("syntax is ok");
  expect(html.indexOf('role="status"')).toBeGreaterThan(html.indexOf("</form>"));
});
it("keeps the node error and retry destination", () => {
  mock.result = { ok: false, kind: "node_error", message: "노드 응답 실패", nodeVersion: null };
  const html = render();
  expect(html).toContain("노드에서 오류가 발생했습니다");
  expect(html).toContain('href="/nodes/n1/history/s2"');
});

it.each(["요청 시간이 초과되었습니다", "API 서버에 요청을 전달하지 못했습니다"])("renders a kind-less transport failure: %s", error => {
  mock.result = {ok:false,error} as unknown as NodeResult<{output:string}>;
  const html=render();
  expect(html).toContain("노드에서 오류가 발생했습니다");
  expect(html).toContain('href="/nodes/n1/history/s2"');
});

it("sends a deleted snapshot back to the list", () => {
  mock.result = { ok: false, kind: "not_found", message: "snapshot missing", nodeVersion: null };
  const html = render();
  expect(html).toContain("그 스냅샷을 노드에서 찾을 수 없습니다");
  expect(html).toContain('href="/nodes/n1/history"');
});
