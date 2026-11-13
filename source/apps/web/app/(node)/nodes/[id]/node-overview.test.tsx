import {expect, it} from "vitest";
import {renderToStaticMarkup} from "react-dom/server";
import {NodeOverview, type NodeOverviewView} from "./node-overview";

it.each([0,2])("unconfirmed node status uses a recovery-safe explanation with %s changes", (count) => {
  const result: NodeOverviewView = {ok: true, node: {id: "n1", name: "Test node", baseUrl: "https://node.example", tokenRole: "admin", allowInsecureTls: false, instanceId: "node1", gatewayName: null, createdByUserId: null, createdAt: "2026-10-25T00:00:00Z", lastSeenAt: null, lastStatus: null, lastVersion: null, lastError: null}, canManage: false,
    data: {semver: "2.0.0", counts: {servers: 0, streams: 0, upstreams: 0}, certificates: {total: 0, expiringSoon: 0}, pending: {count, hasBaseline: false}, nginx: {configOk: true, output: "ok"}}};
  const html = renderToStaticMarkup(<NodeOverview id="n1" result={result} />);
  expect(html).toContain("적용 상태 확인 필요");
  expect(html).toContain("현재 동작 중인 설정을 확인하지 못했습니다");
  expect(html).not.toContain("한 번도");
});
