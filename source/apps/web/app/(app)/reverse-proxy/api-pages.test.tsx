import { beforeEach, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import type { ServerListProps } from "@/components/reverse-proxy/server-list";
import type { ServerDetailProps } from "@/components/reverse-proxy/server-detail";

const mocks = vi.hoisted(() => ({ read: vi.fn() }));
vi.mock("@/lib/api-page", () => ({ readPageView: mocks.read }));
vi.mock("@repo/db", () => ({ getDb: () => { throw new Error("SSR must read the API view"); } }));
vi.mock("@/lib/guard", () => ({ requireRole: () => { throw new Error("API owns authorization"); } }));
vi.mock("@/components/reverse-proxy/server-list", () => ({ ServerList: () => null }));
vi.mock("@/components/reverse-proxy/server-detail", () => ({ ServerDetail: () => null }));
vi.mock("@/components/reverse-proxy/server-form", () => ({ ServerForm: () => null }));
vi.mock("@/components/upstreams/upstream-list", () => ({ UpstreamList: () => null }));
vi.mock("@/components/upstreams/upstream-detail", () => ({ UpstreamDetail: () => null }));
vi.mock("@/components/upstreams/upstream-form", () => ({ UpstreamForm: () => null }));
vi.mock("@/components/streams/stream-list", () => ({ StreamList: () => null }));
vi.mock("@/components/streams/stream-detail", () => ({ StreamDetail: () => null }));
vi.mock("@/components/streams/stream-form", () => ({ StreamForm: () => null }));

const upstream = { id: "u1", name: "api-pool", servers: ["backend:8080"] };
const server = { id: "s1", serverName: "a.test", listen: 80, upstreamId: "u1", certificateId: "c1", config: {}, enabled: false };
const stream = { id: "l1", listen: 15432, protocol: "tcp", proxyPass: "database:5432", enabled: false };
const certificate = { id: "c1", domains: ["a.test"], status: "issued", staging: true, alias: "API certificate", tlsReady: false };
const pending = { count: 2, hasBaseline: true, diff: { servers: [], streams: [], upstreams: [{ key: "u1", label: "api-pool", status: "added", changes: [] }], hasChanges: true } };
const reservedPorts = { process: [3000, 3001, 5432], http: [80, 443, 8080, 8443], streamTcp: [15432], owners: { 8443: "API-owned listener" } };
const view = { canWrite: true, canApply: true, upstreams: [upstream], servers: [server], streams: [stream], upstream, server, stream, certificates: [certificate], selfProxyNotice: true, selfProxyUncheckedPaths: ["/api"], pending, reservedPorts };

function element<T>(node: ReactNode, type: unknown): ReactElement<T> {
  const visit = (candidate: ReactNode): ReactElement<T> | undefined => {
    if (Array.isArray(candidate)) return candidate.map(visit).find(Boolean);
    if (!isValidElement(candidate)) return undefined;
    if (candidate.type === type) return candidate as ReactElement<T>;
    return visit((candidate.props as { children?: ReactNode }).children);
  };
  const found = visit(node);
  expect(found).toBeDefined();
  return found!;
}
beforeEach(() => { mocks.read.mockReset(); mocks.read.mockResolvedValue(view); });

it("server list uses API-owned upstream labels, pending metadata and independent permissions", async () => {
  const Page = (await import("./page")).default;
  const { ServerList } = await import("@/components/reverse-proxy/server-list");
  mocks.read.mockResolvedValue({ ...view, canWrite: false, canApply: true });
  const props = element<ServerListProps>(await Page(), ServerList).props;
  expect(mocks.read).toHaveBeenCalledWith("/reverse-proxy");
  expect(props.data.items[0]?.targetLabel).toContain("api-pool");
  expect(props.data.pendingCount).toBe(2);
  expect(props.data.pendingDiff).toBe(pending.diff);
  expect(props.data.canApply).toBe(true);
  expect(props.write).toBeNull();
  // 적용은 셸 머리줄에 있다 — 목록은 적용 명령을 들고 있지 않다.
  expect("applyAction" in props).toBe(false);
});

it("server list binds the row switch to the local toggle command only with write permission", async () => {
  const Page = (await import("./page")).default;
  const { ServerList } = await import("@/components/reverse-proxy/server-list");
  mocks.read.mockResolvedValue({ ...view, canWrite: true, canApply: false });
  const props = element<ServerListProps>(await Page(), ServerList).props;
  expect(props.write).toEqual({ newHref: "/reverse-proxy/new", templatesHref: "/reverse-proxy/templates", toggleAction: { id: "reverse-proxy.toggleServer", args: [] } });
});

it("server detail uses safe certificate readiness and the API self-proxy decision", async () => {
  const Page = (await import("./[id]/page")).default;
  const { ServerDetail } = await import("@/components/reverse-proxy/server-detail");
  const props = element<ServerDetailProps>(await Page({ params: Promise.resolve({ id: "s1" }) }), ServerDetail).props;
  expect(mocks.read).toHaveBeenCalledWith("/reverse-proxy/s1");
  expect(props.data.cert).toMatchObject({ id: "c1", issued: true, tlsReady: false, staging: true });
  expect(props.data.selfProxyNotice).toBe(true);
  expect(props.data.selfProxyUncheckedPaths).toEqual(["/api"]);
  expect(props.data.enabled).toBe(false);
  expect(props.write?.toggleAction).toEqual({ id: "reverse-proxy.toggleServer", args: [] });
  expect(props.write?.deleteAction).toEqual({ id: "reverse-proxy.deleteServer", args: [] });
});

it("server edit keeps stored values and filters certificate options", async () => {
  const Page = (await import("./[id]/edit/page")).default;
  const { ServerForm } = await import("@/components/reverse-proxy/server-form");
  mocks.read.mockResolvedValue({ ...view, certificates: [certificate, { ...certificate, id: "pending", status: "issuing" }] });
  const props = element<{ action: unknown; certificates: { id: string; label: string }[]; defaultValues: { id: string; upstreamId: string } }>(await Page({ params: Promise.resolve({ id: "s1" }) }), ServerForm).props;
  expect(mocks.read).toHaveBeenCalledWith("/reverse-proxy/s1/edit");
  expect(props.action).toEqual({ id: "reverse-proxy.updateServer", args: [] });
  expect(props.certificates.map(c => c.id)).toEqual(["c1"]);
  expect(props.certificates[0]?.label).toBe("API certificate (a.test)");
  expect(props.defaultValues).toMatchObject({ id: "s1", upstreamId: "u1" });
});

it("server creation retains template defaults without reading the database", async () => {
  const Page = (await import("./new/page")).default;
  const { ServerForm } = await import("@/components/reverse-proxy/server-form");
  const { SERVER_TEMPLATES } = await import("./server-templates");
  const template = SERVER_TEMPLATES[0]!;
  const props = element<{ action: unknown; defaultValues: { listen: number }; upstreams: { id: string }[] }>(await Page({ searchParams: Promise.resolve({ template: template.id }) }), ServerForm).props;
  expect(mocks.read).toHaveBeenCalledWith("/reverse-proxy/new");
  expect(props.action).toEqual({ id: "reverse-proxy.createServer", args: [] });
  expect(props.defaultValues.listen).toBe(template.defaults.listen);
  expect(props.upstreams).toEqual([{ id: "u1", name: "api-pool" }]);
});

it("upstream list/detail preserve read-only permissions and API data", async () => {
  const List = (await import("./upstreams/page")).default;
  const Detail = (await import("./upstreams/[id]/page")).default;
  const { UpstreamList } = await import("@/components/upstreams/upstream-list");
  const { UpstreamDetail } = await import("@/components/upstreams/upstream-detail");
  mocks.read.mockResolvedValue({ ...view, canWrite: false });
  const list = element<{ data: { items: unknown[]; canWrite: boolean } }>(await List(), UpstreamList).props;
  expect(list.data).toEqual({ items: [upstream], canWrite: false });
  const detail = element<{ data: unknown; canWrite: boolean; deleteAction: unknown }>(await Detail({ params: Promise.resolve({ id: "u1" }) }), UpstreamDetail).props;
  expect(detail.data).toEqual(upstream); expect(detail.canWrite).toBe(false);
  expect(detail.deleteAction).toEqual({ id: "upstreams.deleteUpstream", args: [] });
  expect(mocks.read.mock.calls.map(c => c[0])).toEqual(["/reverse-proxy/upstreams", "/reverse-proxy/upstreams/u1"]);
});

it("upstream new/edit supply serializable commands and stored edit values", async () => {
  const New = (await import("./upstreams/new/page")).default;
  const Edit = (await import("./upstreams/[id]/edit/page")).default;
  const { UpstreamForm } = await import("@/components/upstreams/upstream-form");
  const created = element<{ action: unknown }>(await New(), UpstreamForm).props;
  const edited = element<{ action: unknown; defaultValues: unknown }>(await Edit({ params: Promise.resolve({ id: "u1" }) }), UpstreamForm).props;
  expect(created.action).toEqual({ id: "upstreams.createUpstream", args: [] });
  expect(edited.action).toEqual({ id: "upstreams.updateUpstream", args: [] });expect(edited.defaultValues).toEqual(upstream);
  expect(mocks.read.mock.calls.map(c => c[0])).toEqual(["/reverse-proxy/upstreams/new", "/reverse-proxy/upstreams/u1/edit"]);
});

it("stream list and forms display API reserved listeners including API_PORT", async () => {
  const List = (await import("../stream/page")).default;
  const New = (await import("../stream/new/page")).default;
  const Edit = (await import("../stream/[id]/edit/page")).default;
  const { StreamList } = await import("@/components/streams/stream-list");
  const { StreamForm } = await import("@/components/streams/stream-form");
  const list = element<{ reservedPorts: unknown; data: { items: unknown[]; canWrite: boolean } }>(await List(), StreamList).props;
  expect(list.reservedPorts).toBe(reservedPorts);
  expect(list.data.items).toHaveLength(1);
  expect(list).not.toHaveProperty("applyAction");
  const created = element<{ action: unknown; reservedListenPorts: number[] }>(await New(), StreamForm).props;
  const edited = element<{ action: unknown; reservedListenPorts: number[]; defaultValues: unknown }>(await Edit({ params: Promise.resolve({ id: "l1" }) }), StreamForm).props;
  expect(created.action).toEqual({ id: "stream.createStream", args: [] });expect(edited.action).toEqual({ id: "stream.updateStream", args: [] });
  expect(created.reservedListenPorts).toEqual([...reservedPorts.process, ...reservedPorts.http]);expect(edited.reservedListenPorts).toEqual(created.reservedListenPorts);
  expect(edited.defaultValues).toMatchObject({ id: "l1", protocol: "tcp", proxyPass: "database:5432" });
  expect(mocks.read.mock.calls.map(c => c[0])).toEqual(["/stream", "/stream/new", "/stream/l1/edit"]);
});

it("stream detail binds API toggle/delete references and preserves disabled state", async () => {
  const Page = (await import("../stream/[id]/page")).default;
  const { StreamDetail } = await import("@/components/streams/stream-detail");
  const props = element<{ data: unknown; toggleAction: unknown; deleteAction: unknown }>(await Page({ params: Promise.resolve({ id: "l1" }) }), StreamDetail).props;
  expect(props.data).toEqual(stream);expect(props.toggleAction).toEqual({ id: "stream.toggleStream", args: [] });expect(props.deleteAction).toEqual({ id: "stream.deleteStream", args: [] });
});

it.each([0, 2])("local list retains an uncertain baseline with %s changes", async (count) => {
  const Page = (await import("./page")).default;
  const { ServerList } = await import("@/components/reverse-proxy/server-list");
  mocks.read.mockResolvedValue({...view, pending: {...pending, count, hasBaseline: false}});
  expect(element<ServerListProps>(await Page(), ServerList).props.data).toMatchObject({pendingCount: count, pendingHasBaseline: false});
});
