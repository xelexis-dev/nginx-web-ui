import { beforeEach, describe, expect, it, vi } from "vitest";
vi.mock("./api-client", () => ({ readView: vi.fn() }));
import { readView } from "./api-client";
import { readOperationsShellData } from "./operations-shell-data";

const diff = { servers: [], streams: [{ key: "st-1", label: "5432/tcp", status: "added", changes: [] }], upstreams: [], hasChanges: true };
const pending = { count: 3, diff };
const servers = [{ enabled: true }, { enabled: false }];
beforeEach(() => vi.mocked(readView).mockReset());

describe("Operations shell data ownership", () => {
  it("keeps the local view's permission and counts, including non-server changes", async () => {
    vi.mocked(readView).mockResolvedValue({ servers, pending, canApply: false });
    expect(await readOperationsShellData()).toMatchObject({ available: true, serverTotal: 2,
      serverActive: 1, pendingCount: 3, pendingDiff: diff, canApply: false, applyAction: { id: "apply.applyConfig", args: [] } });
    expect(readView).toHaveBeenCalledExactlyOnceWith("/reverse-proxy");
  });
  it("uses the remote role and pending data with a node-scoped action", async () => {
    vi.mocked(readView).mockResolvedValue({ ok: true, effectiveRole: "viewer", data: { servers: { servers }, pending } });
    expect(await readOperationsShellData("node-1")).toMatchObject({ available: true, canApply: false,
      pendingCount: 3, pendingDiff: diff, applyAction: { id: "node.applyNodeConfig", args: ["node-1", "/nodes/node-1/reverse-proxy"] } });
    expect(readView).toHaveBeenCalledExactlyOnceWith("/nodes/node-1/reverse-proxy");
  });
  it("does not replace a failed remote response with local state", async () => {
    vi.mocked(readView).mockResolvedValue({ ok: false });
    expect(await readOperationsShellData("offline")).toEqual({ available: false });
    expect(readView).toHaveBeenCalledTimes(1);
  });
  it("reports an unavailable optional read instead of zero/healthy status", async () => {
    vi.mocked(readView).mockRejectedValue(new Error("unavailable"));
    expect(await readOperationsShellData()).toEqual({ available: false });
    vi.mocked(readView).mockResolvedValue({ servers, pending: { count: -1, diff }, canApply: true });
    expect(await readOperationsShellData()).toEqual({ available: false });
  });
});


it.each([undefined, "node-1"])("preserves the baseline from the selected target %s", async (nodeId) => {
  for (const count of [0, 2]) for (const hasBaseline of [true, false, undefined]) {
    const pending = {count, hasBaseline, diff};
    vi.mocked(readView).mockResolvedValue(nodeId
      ? {ok: true, effectiveRole: "admin", data: {servers: {servers}, pending}}
      : {servers, pending, canApply: true});
    expect(await readOperationsShellData(nodeId)).toMatchObject({available: true, pendingCount: count, pendingHasBaseline: hasBaseline === true, canApply: true});
  }
});


it("carries the node version from the existing remote read without another request", async () => {
  vi.mocked(readView).mockResolvedValue({ok: true, node: {lastVersion: "2.1.1"}, effectiveRole: "viewer", data: {servers: {servers}, pending}});
  expect(await readOperationsShellData("node-1")).toMatchObject({available: true, nodeVersion: "2.1.1"});
  expect(readView).toHaveBeenCalledExactlyOnceWith("/nodes/node-1/reverse-proxy");
});
