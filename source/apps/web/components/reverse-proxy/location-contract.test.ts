import { expect, expectTypeOf, it } from "vitest";
import type { NodeLocation, NodeLocationInput } from "@repo/contracts";
import type {
  LocationConfig,
  ResolvedLocationConfig,
} from "@repo/presentation/nginx";
import { toServerFormValues, toLocationSubmission } from "./server-form-vm";
import { locationTarget } from "./server-vm";

it("keeps canonical location fields aligned with the generated A2 read contract", () => {
  expectTypeOf<LocationConfig["path"]>().toEqualTypeOf<NodeLocation["path"]>();
  expectTypeOf<LocationConfig["upstreamId"]>().toEqualTypeOf<
    NodeLocation["upstreamId"]
  >();
  expectTypeOf<LocationConfig["upstreamName"]>().toEqualTypeOf<
    NodeLocation["upstreamName"]
  >();
  expectTypeOf<LocationConfig["directives"]>().toEqualTypeOf<
    NodeLocation["directives"]
  >();
  expectTypeOf<LocationConfig["retryOtherInstances"]>().toEqualTypeOf<
    NodeLocation["retryOtherInstances"]
  >();
  expectTypeOf<ResolvedLocationConfig["proxyPass"]>().toEqualTypeOf<string>();
  const absent: NodeLocationInput = {
    path: "/",
    mode: "direct",
    proxyPass: "http://service",
    directives: null,
  };
  expect(absent.directives).toBeNull();
});
it("normalizes nullable wire overrides while retaining ID targets and absent directives", () => {
  const stored: NodeLocation = {
    path: "/",
    upstreamId: "u",
    upstreamName: "node-pool",
    ipAccess: null,
    cacheEnabled: null,
    gzipEnabled: null,
  };
  const values = toServerFormValues(
    {
      id: "s",
      serverName: "app.test",
      listen: 80,
      enabled: true,
      upstreamId: null,
      certificateId: null,
      config: { locations: [stored] },
    },
    [{ id: "u", name: "node-pool" }],
  );
  expect(values.locations![0]!.ipAccess).toBeUndefined();
  expect(values.locations![0]!.cacheEnabled).toBeUndefined();
  expect(values.locations![0]!.gzipEnabled).toBeUndefined();
  const posted = toLocationSubmission(values.locations![0]!);
  expect(posted).toEqual({
    path: "/",
    mode: "upstream",
    upstreamId: "u",
    directives: null,
  });
  expect(posted).not.toHaveProperty("upstreamName");
  expect(posted).not.toHaveProperty("proxyPass");
  expect(locationTarget(stored, new Set())).toEqual({
    label: "upstream: node-pool",
  });
});
it("never replaces a dangling projected ID with a stale URL or a different scope's name", () => {
  const stored: NodeLocation = {
    path: "/",
    upstreamId: "missing",
    proxyPass: "http://stale",
  };
  expect(locationTarget(stored, new Set(["stale"]))).toEqual({
    label: "Upstream 없음",
    missingUpstream: true,
  });
  const values = toServerFormValues(
    {
      id: "s",
      serverName: "app.test",
      listen: 80,
      enabled: true,
      upstreamId: null,
      certificateId: null,
      config: { locations: [stored] },
    },
    [{ id: "another", name: "stale" }],
  );
  expect(values.locations![0]!.upstreamId).toBe("");
  expect(values.locations![0]!.mode).toBe("upstream");
  expect(toLocationSubmission(values.locations![0]!)).not.toHaveProperty(
    "proxyPass",
  );
});

it.each([undefined, false, true])(
  "restores and serializes the strict optional retry flag: %s",
  (retryOtherInstances) => {
    const original = [
      { name: "proxy_read_timeout", args: ["300s"] },
      { name: "proxy_set_header", args: ["Host", "$host"] },
    ];
    const stored: NodeLocation = {
      path: "/",
      proxyPass: "http://service",
      directives: original,
      ...(retryOtherInstances !== undefined ? { retryOtherInstances } : {}),
    };
    const values = toServerFormValues(
      {
        id: "s",
        serverName: "app.test",
        listen: 80,
        enabled: true,
        upstreamId: null,
        certificateId: null,
        config: { locations: [stored] },
      },
      [],
    );
    const route = values.locations![0]!;
    expect(route.retryOtherInstances).toBe(
      retryOtherInstances === true ? true : undefined,
    );
    const posted = toLocationSubmission(route);
    if (retryOtherInstances === true)
      expect(posted).toHaveProperty("retryOtherInstances", true);
    else expect(posted).not.toHaveProperty("retryOtherInstances");
    expect(posted.directives).toEqual(original);
  },
);
it("clears an existing true flag by replacement, then can turn it on again without editing directives", () => {
  const values = toServerFormValues(
    {
      id: "s",
      serverName: "app.test",
      listen: 80,
      enabled: true,
      upstreamId: null,
      certificateId: null,
      config: {
        locations: [
          { path: "/", proxyPass: "http://service", retryOtherInstances: true },
        ],
      },
    },
    [],
  );
  const route = values.locations![0]!;
  const before = JSON.stringify(route.directives);
  expect(toLocationSubmission(route)).toHaveProperty(
    "retryOtherInstances",
    true,
  );
  expect(
    toLocationSubmission({
      ...route,
      retryOtherInstances: false,
    }),
  ).not.toHaveProperty("retryOtherInstances");
  expect(
    toLocationSubmission({
      ...route,
      retryOtherInstances: undefined,
    }),
  ).not.toHaveProperty("retryOtherInstances");
  expect(
    toLocationSubmission({
      ...route,
      retryOtherInstances: true,
    }),
  ).toHaveProperty("retryOtherInstances", true);
  expect(JSON.stringify(route.directives)).toBe(before);
  expect(toLocationSubmission(route).directives).toBeNull();
});
it.each([null, "true", 0, 1])(
  "refuses malformed retry state from a node: %s",
  (retryOtherInstances) => {
    expect(() =>
      toServerFormValues(
        {
          id: "s",
          serverName: "app.test",
          listen: 80,
          enabled: true,
          upstreamId: null,
          certificateId: null,
          config: {
            locations: [
              { path: "/", proxyPass: "http://service", retryOtherInstances },
            ],
          },
        },
        [],
      ),
    ).toThrow("재시도 설정은 참 또는 거짓이어야 합니다.");
  },
);
