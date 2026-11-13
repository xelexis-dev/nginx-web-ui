import { describe, expect, it } from "vitest";
import { directiveSummary, serverSummary } from "./server-summary";
import { directivesToState, emptyDirectiveState } from "./directive-state";
import { parseGzipConfig } from "./performance";

describe("directive summaries", () => {
  it("shows header names only, including headers carried after the advanced boundary", () => {
    const original = [
      { name: "return", args: ["200", "ok"] },
      { name: "proxy_set_header", args: ["Authorization", "Bearer private-token"] },
      { name: "add_header", args: ["X-Secret", "secret-value"] },
      { name: "client_max_body_size", args: ["10m"] },
    ];
    const summary = directiveSummary(directivesToState(original), original);
    expect(summary.requestHeaderNames).toEqual(["Authorization"]);
    expect(summary.responseHeaderNames).toEqual(["X-Secret"]);
    expect(summary.bodySize).toBe("10m");
    expect(JSON.stringify(summary)).not.toMatch(/private-token|secret-value|Bearer/);
    expect(directiveSummary(emptyDirectiveState(), null).implicitHost).toBe(true);
    expect(directiveSummary(emptyDirectiveState(), []).implicitHost).toBe(false);
  });

  it("reads a complete WebSocket shape as on, a partial one as manual, and no upgrade headers as off", () => {
    const state = emptyDirectiveState();
    expect(directiveSummary(state).webSocket).toBe("꺼짐");
    state.headers = [{ name: "Upgrade", value: "$http_upgrade" }];
    expect(directiveSummary(state).webSocket).toBe("직접 설정");
    state.headers.push({ name: "connection", value: "$connection_upgrade" });
    expect(directiveSummary(state).webSocket).toBe("직접 설정");
    state.httpVersion = "1.1";
    expect(directiveSummary(state).webSocket).toBe("켜짐");
    state.headers.push({ name: "Upgrade", value: "other" });
    expect(directiveSummary(state).webSocket).toBe("직접 설정");
  });
});

it("labels legacy gzip as manual without creating structured configuration", () => {
  const original = [{ name: "gzip", args: ["on"] }];
  const input = { serverName: "a.test b.test", listen: 80, locations: [], upstreams: [], serverDirectives: directivesToState(original), originalServerDirectives: original };
  expect(serverSummary(input)).toMatchObject({ names: ["a.test", "b.test"], listen: "80", gzip: "직접 설정" });
  expect(input).not.toHaveProperty("gzip");
  expect(serverSummary({ ...input, gzip: parseGzipConfig({ enabled: false }) }).gzip).toBe("꺼짐");
});

it("summarises targets by current ID and counts explicit false overrides", () => {
  const result = serverSummary({ serverName: "a.test", listen: "8081", serverDirectives: emptyDirectiveState(),
    upstreams: [{ id: "up", name: "renamed" }], locations: [
      { path: "/", mode: "upstream", upstreamId: "up", proxyPass: "", directives: emptyDirectiveState(), cacheEnabled: false },
      { path: "/missing", mode: "upstream", upstreamId: "", proxyPass: "", directives: emptyDirectiveState(), upstreamError: "missing", gzipEnabled: true },
    ] });
  expect(result.locations.map((location) => location.target)).toEqual(["renamed", "Upstream 없음"]);
  expect(result.locations[1]).toHaveProperty("error", "missing");
  expect(result).toMatchObject({ cacheOverrides: 1, gzipOverrides: 1, ipOverrides: 0 });
});

it("includes retry in the pure route summary without generating user directives", () => {
  const directives = emptyDirectiveState();
  const input = {
    serverName: "app.test",
    listen: 80,
    locations: [
      {
        path: "/",
        mode: "direct" as const,
        upstreamId: "",
        proxyPass: "http://service",
        directives,
        retryOtherInstances: true,
      },
    ],
    upstreams: [],
    serverDirectives: emptyDirectiveState(),
  };
  const before = JSON.stringify(directives);
  const summary = serverSummary(input);
  expect(summary.locations[0]).toHaveProperty("retryOtherInstances", true);
  expect(summary.locations[0]).toHaveProperty(
    "retryInfo.summary",
    "재시도: 최대 2회 · 연결 2초 · 3초 안에 재시도",
  );
  expect(JSON.stringify(directives)).toBe(before);
});
