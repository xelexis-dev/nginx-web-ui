import { describe, expect, it } from "vitest";
import { toServerRow, type ServerSourceRow } from "./server-vm";

/**
 * The 대상 column follows the renderer (apps/api/src/control/render.rs): a non-empty `locations`
 * list renders one `location` per entry with its own proxy_pass and ignores the server's
 * upstream/proxy_pass; otherwise the single `location /` uses the upstream, then proxy_pass.
 */

const upstreams = new Map([
  ["up-api", "api_pool"],
  ["up-web", "web_pool"],
]);
const src = (over: Partial<ServerSourceRow> = {}): ServerSourceRow => ({
  id: "s1",
  serverName: "a.example.test",
  listen: 80,
  enabled: true,
  upstreamId: null,
  config: {},
  ...over,
});

describe("toServerRow targets", () => {
  it("reads projected names for canonical IDs without an upstream-list fetch and ignores stale URLs", () => {
    const config = { locations: [
      { path: "/id", upstreamId: "up-api", upstreamName: "renamed_pool", proxyPass: "http://stale.test" },
      { path: "/literal", proxyPass: "http://origin.test/a/" },
    ] };
    const before = structuredClone(config);
    const vm = toServerRow(src({ config }), new Map());
    expect(vm.targets).toEqual([{ path: "/id", label: "upstream: renamed_pool" }, { path: "/literal", label: "proxy_pass: http://origin.test/a/" }]);
    expect(vm.targetLabel).not.toContain("stale");
    expect(config).toEqual(before);
  });

  it("marks an ID without a projected name as missing even if a separate list has that ID", () => {
    const vm = toServerRow(src({ config: { locations: [{ path: "/gone", upstreamId: "up-api", proxyPass: "http://stale.test" }] } }), upstreams);
    expect(vm.targets).toEqual([{ path: "/gone", label: "Upstream 없음", missingUpstream: true }]);
  });

  it("keeps the single-target label: upstream first, then proxy_pass, then —", () => {
    for (const [row, label] of [
      [src({ upstreamId: "up-api", config: { proxyPass: "http://ignored.example.test" } }), "upstream: api_pool"],
      [src({ upstreamId: "up-gone" }), "upstream: up-gone"],
      [src({ config: { proxyPass: "http://10.0.0.1:3000" } }), "proxy_pass: http://10.0.0.1:3000"],
      [src(), "—"],
      [src({ config: "not an object" }), "—"],
      [src({ config: { locations: [], proxyPass: "http://10.0.0.1:3000" } }), "proxy_pass: http://10.0.0.1:3000"],
    ] as const) {
      const vm = toServerRow(row, upstreams);
      expect(vm.targetLabel).toBe(label);
      expect(vm.targets).toEqual([{ path: null, label }]);
    }
  });

  it("lists every location in order with its own target, ignoring the server-level target", () => {
    const vm = toServerRow(
      src({
        upstreamId: "up-web",
        config: {
          proxyPass: "http://ignored.example.test",
          locations: [
            { path: "/api", proxyPass: "http://api_pool" },
            { path: "/static", proxyPass: "http://10.0.5.10:8080" },
            { path: "/ws", proxyPass: "http://unknown_pool" },
          ],
        },
      }),
      upstreams,
    );
    expect(vm.targets).toEqual([
      { path: "/api", label: "upstream: api_pool" },
      { path: "/static", label: "proxy_pass: http://10.0.5.10:8080" },
      { path: "/ws", label: "proxy_pass: http://unknown_pool" },
    ]);
    // One searchable, titled summary: every path and target, none of the ignored ones.
    expect(vm.targetLabel).toBe("/api → upstream: api_pool · /static → proxy_pass: http://10.0.5.10:8080 · /ws → proxy_pass: http://unknown_pool");
    expect(vm.targetLabel).not.toMatch(/web_pool|ignored|—/);
  });

  it("shows a single location as a location, since that is what the renderer writes", () => {
    const vm = toServerRow(src({ config: { locations: [{ path: "/", proxyPass: "http://api_pool" }] } }), upstreams);
    expect(vm.targets).toEqual([{ path: "/", label: "upstream: api_pool" }]);
    expect(vm.targetLabel).toBe("/ → upstream: api_pool");
  });

  it("resolves a location upstream only by exact name from the same-origin list", () => {
    const location = (proxyPass: string) => toServerRow(src({ config: { locations: [{ path: "/a", proxyPass }] } }), upstreams).targets[0]?.label;
    expect(location("http://api_pool")).toBe("upstream: api_pool");
    expect(location("http://api_pool/")).toBe("proxy_pass: http://api_pool/");
    expect(location("https://api_pool")).toBe("proxy_pass: https://api_pool");
    expect(toServerRow(src({ config: { locations: [{ path: "/a", proxyPass: "http://api_pool" }] } }), new Map()).targets[0]?.label)
      .toBe("proxy_pass: http://api_pool");
  });

  it("marks a location without a target as — instead of hiding it or throwing", () => {
    const vm = toServerRow(src({ config: { locations: [{ path: "/a", proxyPass: "" }, { path: "/b" }, null] } }), upstreams);
    expect(vm.targets).toEqual([
      { path: "/a", label: "—" },
      { path: "/b", label: "—" },
      { path: "", label: "—" },
    ]);
  });
});
