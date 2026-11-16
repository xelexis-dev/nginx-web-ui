import { expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { NodeView } from "@/lib/node-page";
import { NodeList } from "./node-list";
vi.mock("@/lib/use-api-action", () => ({
  useApiAction: () => async () => ({ ok: false }),
}));
const node: NodeView = {
  id: "n1",
  name: "Node",
  baseUrl: "https://node.example.test",
  tokenRole: "admin",
  allowInsecureTls: false,
  instanceId: "instance",
  gatewayName: null,
  createdByUserId: null,
  createdAt: "2026-10-01T00:00:00Z",
  lastSeenAt: null,
  lastStatus: null,
  lastVersion: null,
  lastError: null,
};
it("renders one comfortable table with the original node destination and last-observation distinction", () => {
  const html = renderToStaticMarkup(
    <NodeList
      nodes={[node, { ...node, id: "n2", lastStatus: "unreachable" }]}
      canWrite={false}
      unpairWarningText={null}
    />,
  );
  expect(html.match(/<table\b/g)).toHaveLength(1);
  expect(html).toContain('data-density="comfortable"');
  expect(html).toContain('data-narrow="stacked"');
  expect(html).toContain('data-label="상태"');
  expect(html).toContain('href="/nodes/n1"');
  expect(html).toContain("아직 확인되지 않았습니다 — 새로고침하세요.");
  expect(html).toContain("응답한 적 없음");
  expect(html).not.toContain('href="/nodes/new"');
  expect(html).not.toContain("상태 새로고침");
});
it("retains the empty copy and admin connect/refresh actions", () => {
  const html = renderToStaticMarkup(
    <NodeList nodes={[]} canWrite unpairWarningText="토큰 해지 실패" />,
  );
  expect(html).toMatch(
    /<td[^>]*colSpan="4"[^>]*><div[^>]*><p[^>]*>연결된 노드가 없습니다\./,
  );
  expect(html).toContain('href="/nodes/new"');
  expect(html).toContain("상태 새로고침");
  expect(html).toContain("토큰 해지 실패");
});
