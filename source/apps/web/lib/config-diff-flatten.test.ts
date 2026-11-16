import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { FIELD_LABELS, flattenChanges } from "./config-diff-flatten";
import type { ItemDiff } from "./config-diff";
const item = (before: unknown, after: unknown, field = "config"): ItemDiff => ({
  key: "k",
  label: "a:80",
  status: "changed",
  changes: [{ field, before, after }],
});
it("labels scalars, missing values and resource states", () => {
  expect(
    flattenChanges("servers", item(false, true, "enabled"))[0],
  ).toMatchObject({ path: ["상태"], before: "비활성", after: "활성" });
  expect(
    flattenChanges("streams", item(null, "", "proxyPass"))[0],
  ).toMatchObject({ before: null, after: null });
  expect(
    flattenChanges(
      "servers",
      item(
        { redirectHttp: false, tlsPort: 443 },
        { redirectHttp: true, tlsPort: 443 },
      ),
    ),
  ).toHaveLength(1);
});
it("matches locations by path and expands one-sided leaves only in full mode", () => {
  const a = { path: "/", proxyPass: "http://a" },
    b = { path: "/api", proxyPass: "http://b", retryOtherInstances: true };
  expect(
    flattenChanges(
      "servers",
      item({ locations: [a, b] }, { locations: [b, a] }),
    ),
  ).toEqual([]);
  expect(
    flattenChanges("servers", item({ locations: [a] }, { locations: [a, b] })),
  ).toHaveLength(1);
  const full = flattenChanges(
    "servers",
    item({ locations: [a, b] }, { locations: [a] }),
    {},
    true,
  );
  expect(
    full.some(
      (row) =>
        row.path.includes("재시작 중 다른 인스턴스로 재시도") &&
        row.after === null,
    ),
  ).toBe(true);
});
it("retains repeated directive identities, raw argument boundaries and order changes", () => {
  const d = (name: string, ...args: string[]) => ({ name, args });
  const a = [
    d("add_header", "X-Foo", "one two", "always"),
    d("add_header", "X-Foo", ""),
  ];
  const b = [
    d("add_header", "X-Foo", "one", "two always"),
    d("add_header", "X-Foo", "new"),
  ];
  const rows = flattenChanges(
    "servers",
    item({ serverDirectives: a }, { serverDirectives: b }),
  );
  expect(rows).toHaveLength(2);
  expect(rows[0]!.before).toBe('"one two" always');
  expect(rows[1]!.before).toContain("빈 값");
  expect(rows[1]!.path.at(-1)).toBe("X-Foo#2");
  const before = [d("rewrite", "a", "b"), d("return", "301")];
  expect(
    flattenChanges(
      "servers",
      item(
        { locationDirectives: before },
        { locationDirectives: [...before].reverse() },
      ),
    )[0]!.path.at(-1),
  ).toBe("순서 변경");
});
it("keeps duplicate upstream members for insert, remove and reweight", () => {
  const a = ["a:80 weight=1", "a:80 weight=2"];
  expect(
    flattenChanges("upstreams", item(a, [...a, "a:80 weight=3"], "servers")),
  ).toHaveLength(1);
  expect(
    flattenChanges("upstreams", item(a, a.slice(0, 1), "servers")),
  ).toHaveLength(1);
  expect(
    flattenChanges("upstreams", item(a, [a[0], "a:80 weight=4"], "servers"))[0]!
      .path,
  ).toContain("a:80#2");
});
it("uses set differences and index fallback without raw dumps", () => {
  expect(
    flattenChanges(
      "servers",
      item(
        { ipAccess: { entries: ["a", "b"] } },
        { ipAccess: { entries: ["b", "c"] } },
      ),
    ),
  ).toHaveLength(2);
  const rows = flattenChanges(
    "servers",
    item({ extra: [{ x: 1 }] }, { extra: [{ x: 2 }] }),
  );
  expect(rows[0]).toMatchObject({
    path: ["extra 1", "x"],
    before: "1",
    after: "2",
  });
  expect(
    flattenChanges("servers", item("[blue]", "[red]", "unknown"))[0]!.before,
  ).toBe("[blue]");
});
it("resolves own name then lookup then titled ID and counts unchanged full leaves", () => {
  const rows = flattenChanges(
    "servers",
    item(
      {
        locations: [
          { path: "/", upstreamId: "123456789", upstreamName: "old" },
        ],
      },
      {
        locations: [
          { path: "/", upstreamId: "987654321", upstreamName: "own" },
        ],
      },
    ),
    { "987654321": "lookup" },
  );
  expect(
    rows.find((row) => row.path.at(-1) === "Upstream" && row.afterTitle)?.after,
  ).toBe("own");
  expect(
    flattenChanges("servers", item("old", "123456789", "upstreamId"))[0],
  ).toMatchObject({ after: "ID 12345678", afterTitle: "123456789" });
  expect(
    flattenChanges(
      "servers",
      item({ tlsPort: 443 }, { tlsPort: 443 }),
      {},
      true,
    ).filter((row) => !row.changed),
  ).toHaveLength(1);
});
it("covers all properties of contract configuration schemas", () => {
  const schemas = JSON.parse(
    readFileSync(
      new URL("../../../packages/contracts/openapi.json", import.meta.url),
      "utf8",
    ),
  ).components.schemas;
  for (const name of [
    "NodeServerConfig",
    "NodeLocation",
    "NodeTlsOptions",
    "NodeHsts",
    "NodeIpAccess",
  ])
    for (const key of Object.keys(schemas[name].properties))
      expect(FIELD_LABELS[key], `${name}.${key}`).toBeTruthy();
});
it("renders a three-location TLS change as child rows", () => {
  const rows = flattenChanges(
    "servers",
    item(
      { locations: [{ path: "/", proxyPass: "a" }] },
      {
        locations: [
          { path: "/", proxyPass: "b" },
          { path: "/api", proxyPass: "c" },
          { path: "/assets", proxyPass: "d" },
        ],
        tlsPort: 443,
        tlsOptions: { hsts: { enabled: true } },
      },
    ),
  );
  expect(rows.some((row) => row.path.includes("location /api"))).toBe(true);
  expect(rows.some((row) => row.path.includes("HTTPS 포트"))).toBe(true);
  expect(rows.every((row) => !row.path.includes("config"))).toBe(true);
});

it("keeps directive order changes visible alongside insertion and repeated names", () => {
  const rewrite = { name: "rewrite", args: ["^/a", "/b", "last"] };
  const redirect = { name: "return", args: ["301", "/new"] };
  const added = { name: "add_header", args: ["X-Test", "yes"] };
  const mixed = flattenChanges(
    "servers",
    item(
      { locationDirectives: [rewrite, redirect] },
      { locationDirectives: [redirect, added, rewrite] },
    ),
  );
  expect(mixed.some((row) => row.path.at(-1) === "순서 변경")).toBe(true);
  const second = { name: "rewrite", args: ["^/c", "/d", "last"] };
  const repeated = flattenChanges(
    "servers",
    item(
      { locationDirectives: [rewrite, second] },
      { locationDirectives: [second, rewrite] },
    ),
  );
  expect(repeated[0]).toMatchObject({
    path: ["응답 헤더", "순서 변경"],
    before: "rewrite ^/a /b last › rewrite ^/c /d last",
    after: "rewrite ^/c /d last › rewrite ^/a /b last",
  });
});

it("preserves already quoted arguments and titles one-sided location IDs", () => {
  const quoted = flattenChanges(
    "servers",
    item(
      { serverDirectives: [] },
      {
        serverDirectives: [
          {
            name: "add_header",
            args: ["X-Label", '"typed words"', "always"],
          },
        ],
      },
    ),
  );
  expect(quoted[0]!.after).toBe('"typed words" always');
  const location = flattenChanges(
    "servers",
    item(
      { locations: [] },
      { locations: [{ path: "/api", upstreamId: "123456789" }] },
    ),
  );
  expect(location[0]).toMatchObject({
    after: "Upstream ID 12345678",
    afterTitle: "123456789",
  });
});
