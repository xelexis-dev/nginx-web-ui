import { renderToStaticMarkup } from "react-dom/server";
import { NodeOverview } from "./node-overview";
import { PageHeader, type PageHeaderProps } from "@repo/ui";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { NodeErrorCard, NODE_FORBIDDEN_MESSAGE } from "@/components/node-error";
const mock = vi.hoisted(() => ({ read: vi.fn() }));
vi.mock("@/lib/api-page", () => ({ readPageView: mock.read }));
const LogViewerStub = vi.fn(() => null);
const MetricsViewStub = vi.fn(() => null);
const EditStub = vi.fn(() => null);
vi.mock("@/components/logs/log-viewer", () => ({ LogViewer: LogViewerStub }));
vi.mock("@/components/metrics/metrics-view", () => ({
  MetricsView: MetricsViewStub,
}));
vi.mock("./edit/node-edit-form", () => ({ NodeEditForm: EditStub }));
vi.mock("./edit/unpair-button", () => ({ UnpairButton: () => null }));
const { NodeEditView } = await import("./edit/node-edit-view");
const { default: Summary } = await import("./page");
const { default: Logs } = await import("./logs/page");
const { default: Metrics } = await import("./metrics/page");
const { default: Edit } = await import("./edit/page");
const node = {
  id: "node-1",
  name: "office-node-1",
  baseUrl: "https://node.example.com",
  tokenRole: "viewer",
  allowInsecureTls: false,
  gatewayName: "home",
};
const params = Promise.resolve({ id: node.id });
type El = ReactElement<{
  children?: unknown;
  href?: string;
  streamPath?: string;
  dataPath?: string;
}>;
function elements(tree: unknown, type: unknown): El[] {
  const result: El[] = [];
  function visit(value: unknown) {
    if (Array.isArray(value)) {
      value.forEach(visit);
      return;
    }
    if (!isValidElement(value)) return;
    const el = value as El;
    if (el.type === NodeOverview) {
      visit(
        NodeOverview(el.props as unknown as Parameters<typeof NodeOverview>[0]),
      );
      return;
    }
    if (el.type === NodeEditView) {
      visit(NodeEditView(el.props as unknown as Parameters<typeof NodeEditView>[0]));
      return;
    }
    if (el.type === PageHeader) {
      visit(PageHeader(el.props as unknown as PageHeaderProps));
      return;
    }
    if (el.type === type) result.push(el);
    visit(el.props.children);
  }
  visit(tree);
  return result;
}
function text(tree: unknown): string {
  return renderToStaticMarkup(tree as ReactNode).replace(/<[^>]+>/g, "");
}

beforeEach(() => vi.clearAllMocks());
const cases = [
  {
    name: "summary",
    path: "/nodes/node-1",
    render: () => Summary({ params }),
    viewer: null,
  },
  {
    name: "logs",
    path: "/nodes/node-1/logs",
    render: () => Logs({ params }),
    viewer: LogViewerStub,
  },
  {
    name: "metrics",
    path: "/nodes/node-1/metrics",
    render: () => Metrics({ params, searchParams: Promise.resolve({}) }),
    viewer: MetricsViewStub,
  },
];
for (const c of cases) {
  it(
    c.name +
      " renders authorization failure with a real error card and no normal content",
    async () => {
      const nodeError = {
        failure: { kind: "forbidden", message: NODE_FORBIDDEN_MESSAGE },
        nodeId: node.id,
        retryHref: c.path,
        listHref: c.name === "summary" ? "/nodes" : c.path,
      };
      mock.read.mockResolvedValue({
        ok: false,
        node,
        canManage: true,
        nodeError,
      });
      const result = await c.render();
      const cards = elements(result, NodeErrorCard);
      expect(cards).toHaveLength(1);
      expect(cards[0]!.props).toEqual(nodeError);
      expect(
        text(NodeErrorCard(nodeError as Parameters<typeof NodeErrorCard>[0])),
      ).toContain(NODE_FORBIDDEN_MESSAGE);
      if (c.viewer) expect(elements(result, c.viewer)).toHaveLength(0);
      else expect(text(result)).not.toContain("미적용 변경");
      expect(mock.read).toHaveBeenCalledOnce();
    },
  );
  it(c.name + " preserves a local not-found navigation exception", async () => {
    const error = new Error("NEXT_NOT_FOUND");
    mock.read.mockRejectedValue(error);
    await expect(c.render()).rejects.toBe(error);
  });
}
it("summary preserves a failed remote response and management link", async () => {
  const failure = {
    kind: "unreachable",
    message: "connection refused",
    nodeVersion: null,
  };
  const nodeError = {
    failure,
    nodeId: node.id,
    retryHref: "/nodes/node-1",
    listHref: "/nodes",
  };
  mock.read.mockResolvedValue({ ok: false, node, canManage: true, nodeError });
  const result = await Summary({ params });
  expect(elements(result, NodeErrorCard)[0]!.props).toEqual(nodeError);
  expect(text(result)).toContain("수정");
  expect(text(result)).not.toContain("미적용 변경");
});
it("summary uses API status and gateway management permission independently of node token role", async () => {
  mock.read.mockResolvedValue({
    ok: true,
    node,
    canManage: true,
    data: {
      semver: "1.18.0",
      counts: { servers: 2, streams: 1, upstreams: 3 },
      pending: { count: 0, hasBaseline: true },
      certificates: { total: 4, expiringSoon: 1 },
      nginx: { configOk: true, output: "" },
    },
  });
  const result = await Summary({ params });
  expect(elements(result, NodeErrorCard)).toHaveLength(0);
  expect(text(result)).toContain("v1.18.0");
  expect(text(result)).toContain("수정");
});
it("logs uses the node SSE relay after API authorization", async () => {
  mock.read.mockResolvedValue({
    ok: true,
    node,
    effectiveRole: "viewer",
    data: null,
  });
  const result = await Logs({ params });
  expect(elements(result, LogViewerStub)[0]!.props.streamPath).toBe(
    "/api/nodes/node-1/logs/stream",
  );
  expect(elements(result, NodeErrorCard)).toHaveLength(0);
});
it("metrics uses the node metrics relay after API authorization", async () => {
  mock.read.mockResolvedValue({
    ok: true,
    node,
    effectiveRole: "viewer",
    data: null,
  });
  const result = await Metrics({
    params,
    searchParams: Promise.resolve({ range: "24h" }),
  });
  expect(elements(result, MetricsViewStub)[0]!.props.dataPath).toBe(
    "/api/nodes/node-1/metrics",
  );
  expect(elements(result, NodeErrorCard)).toHaveLength(0);
});
it("edit reads the local registry view even when the remote token is viewer", async () => {
  mock.read.mockResolvedValue({ node });
  const result = await Edit({ params });
  expect(elements(result, EditStub)).toHaveLength(1);
  expect(mock.read).toHaveBeenCalledExactlyOnceWith("/nodes/node-1/edit");
});
it("edit preserves the API not-found result", async () => {
  const error = new Error("NEXT_NOT_FOUND");
  mock.read.mockRejectedValue(error);
  await expect(Edit({ params })).rejects.toBe(error);
});
