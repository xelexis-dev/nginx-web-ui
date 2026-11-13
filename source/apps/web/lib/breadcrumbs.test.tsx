import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";

const state = vi.hoisted(() => ({
  reads: [] as { kind: string; path: string; query?: unknown }[],
  probes: [] as { kind: string; path: string; query?: unknown }[],
  value: undefined as unknown,
  failure: false,
}));
const session = vi.hoisted(() => ({
  user: {
    id: "u1",
    role: "admin",
    email: "admin@example.test",
    requires2fa: false,
    totpEnabled: true,
    verificationRequired: false,
  },
}));
vi.mock("./api-client", async (original) => ({
  ...(await original<typeof import("./api-client")>()),
  readSession: async () => session,
  readView: async (route: string, query?: unknown) => {
    state.reads.push({ kind: "view", path: route, query });
    if (state.failure) throw new Error("failed read");
    return route.endsWith("/layout")
      ? { node: { name: "서울 운영 노드" } }
      : state.value;
  },
  apiRead: async (route: string) => {
    state.reads.push({ kind: "api", path: route });
    if (state.failure) throw new Error("failed read");
    return state.value;
  },
}));
vi.mock("./api-page", () => ({
  readPageView: async (route: string, query?: unknown) => {
    state.probes.push({ kind: "view", path: route, query });
    throw new Error("page-read-probe");
  },
  readPageApi: async (route: string) => {
    state.probes.push({ kind: "api", path: route });
    throw new Error("page-read-probe");
  },
}));
vi.mock("./admin-api", async (original) => ({
  ...(await original<typeof import("./admin-api")>()),
  readAdminSession: async () => session,
}));
import { breadcrumbRoutes, renderBreadcrumbs } from "./breadcrumbs";

beforeEach(() => {
  state.reads = [];
  state.probes = [];
  state.value = undefined;
  state.failure = false;
});
const roots = [
  { dir: "(app)", prefix: "" },
  { dir: "admin", prefix: "/admin" },
  { dir: "(node)/nodes/[id]", prefix: "/nodes/[id]" },
];
async function pages(dir: string): Promise<string[]> {
  const found: string[] = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    if (entry.name.startsWith("@")) continue;
    const target = path.join(dir, entry.name);
    if (entry.isDirectory()) found.push(...(await pages(target)));
    else if (entry.name === "page.tsx") found.push(target);
  }
  return found;
}
it("mirrors every real page in every shell, without slot catch-alls or extra pages", async () => {
  const patterns: string[] = [];
  for (const root of roots) {
    const dir = path.resolve(import.meta.dirname, "../app", root.dir);
    const real = (await pages(dir))
      .map((file) => path.relative(dir, file))
      .sort();
    const slot = path.join(dir, "@crumbs");
    const parallel = (await pages(slot))
      .map((file) => path.relative(slot, file))
      .sort();
    expect(parallel).toEqual(real);
    expect(parallel.some((file) => file.includes("..."))).toBe(false);
    expect(await readFile(path.join(dir, "default.tsx"), "utf8")).toContain(
      "notFound()",
    );
    expect(await readFile(path.join(slot, "default.tsx"), "utf8")).toContain(
      "return null",
    );
    for (const file of real) {
      const suffix =
        path.dirname(file) === "."
          ? ""
          : "/" + path.dirname(file).split(path.sep).join("/");
      patterns.push(root.prefix + suffix || "/");
    }
  }
  expect(Object.keys(breadcrumbRoutes).sort()).toEqual(patterns.sort());
  expect(patterns).toHaveLength(90);
});

const modules = import.meta.glob("../app/**/page.tsx");
const params = {
  id: "item-1",
  sid: "server-1",
  cid: "cert-1",
  upstreamId: "upstream-1",
  streamId: "stream-1",
  logId: "log-1",
};
const query = {
  uncovered: "a.test",
  added: "1",
  tracking: "keep-only-if-page-reads-it",
};
const named = Object.entries(breadcrumbRoutes).filter(
  ([, route]) => "source" in route,
);
it.each(named)(
  "uses an existing page read for %s (no endpoint added for its crumb)",
  async (pattern, route) => {
    const root = pattern.startsWith("/nodes/[id]")
      ? "(node)"
      : pattern.startsWith("/admin")
        ? ""
        : "(app)";
    const pageFile = `../app/${root ? root + "/" : ""}${pattern === "/" ? "" : pattern.slice(1) + "/"}page.tsx`;
    const load = modules[pageFile];
    expect(load, pageFile).toBeDefined();
    const module = (await load!()) as {
      default: (props: unknown) => Promise<unknown>;
    };
    await module
      .default({
        params: Promise.resolve(params),
        searchParams: Promise.resolve(query),
      })
      .catch(() => undefined);
    const expected = ("source" in route ? route.source : "").replace(
      /\[([^\]]+)\]/g,
      (_, key: string) =>
        encodeURIComponent(params[key as keyof typeof params]),
    );
    const read = state.probes.find((probe) => probe.path === expected);
    expect(read, JSON.stringify(state.probes)).toBeDefined();
    expect(read?.kind).toBe(expected.startsWith("/api/") ? "api" : "view");
    expect(read?.query ?? {}).toEqual(
      "query" in route && route.query ? query : {},
    );
  },
);

it.each([
  [
    "/reverse-proxy/[id]",
    { server: { serverName: "api.example.test other.test" } },
    "api.example.test",
  ],
  [
    "/reverse-proxy/upstreams/[id]/edit",
    { upstream: { name: "backend" } },
    "backend",
  ],
  ["/stream/[id]", { stream: { listen: 5432 } }, "listen 5432"],
  [
    "/certificates/[id]",
    { data: { alias: "운영 인증서", domains: { primary: "api.test" } } },
    "운영 인증서",
  ],
  [
    "/certificates/[id]/edit",
    { certificate: { alias: "", domains: ["api.test"] } },
    "api.test",
  ],
  ["/history/[id]", { snapshot: { label: "배포 전" } }, "배포 전"],
  ["/audit/[id]", { log: { action: "설정 적용" } }, "설정 적용"],
  ["/admin/oauth/[id]", { name: "회사 OIDC" }, "회사 OIDC"],
  [
    "/admin/notifications/mail/[id]",
    { label: "", address: "ops@example.test" },
    "ops@example.test",
  ],
  [
    "/admin/notifications/webhooks/[id]/edit",
    { name: "운영 알림" },
    "운영 알림",
  ],
  [
    "/admin/users/[id]",
    { email: "member@example.test" },
    "member@example.test",
  ],
  ["/account/tokens/[id]", { name: "자동화" }, "자동화"],
  ["/account/passkeys/[id]/edit", { passkey: { name: "노트북" } }, "노트북"],
] as const)(
  "renders the page's visible resource name for %s",
  async (pattern, value, name) => {
    state.value = value;
    const html = renderToStaticMarkup(
      await renderBreadcrumbs(pattern, { params: Promise.resolve(params) }),
    );
    expect(html).toContain('aria-label="경로"');
    expect(html).toContain(name);
    expect(html).toContain(`title="${name}"`);
    expect(html.match(/aria-current="page"/g)).toHaveLength(1);
    expect(html).toContain(pattern.endsWith("/edit") ? "수정" : "상세");
    expect(state.reads).toHaveLength(1);
  },
);
it("reuses node layout metadata and the node page's nested response without a second name endpoint", async () => {
  state.value = {
    ok: true,
    data: { server: { server: { serverName: "node.test extra.test" } } },
  };
  const html = renderToStaticMarkup(
    await renderBreadcrumbs("/nodes/[id]/reverse-proxy/[sid]/edit", {
      params: Promise.resolve(params),
    }),
  );
  expect(html).toContain("서울 운영 노드");
  expect(html).toContain("node.test");
  expect(state.reads.map((r) => r.path)).toEqual([
    "/nodes/item-1/layout",
    "/nodes/item-1/reverse-proxy/server-1/edit",
  ]);
});
it.each([
  "/certificates/[id]/replace",
  "/nodes/[id]/certificates/[cid]/replace",
] as const)("keeps certificate replacement nameless at %s", async (pattern) => {
  const html = renderToStaticMarkup(
    await renderBreadcrumbs(pattern, { params: Promise.resolve(params) }),
  );
  expect(html).toContain("교체");
  expect(html).not.toContain("상세");
  expect(state.reads.every((r) => r.path.endsWith("/layout"))).toBe(true);
});
it("reads certificate query parameters only where the page itself sends them", async () => {
  state.value = { data: { alias: "인증서" } };
  await renderBreadcrumbs("/certificates/[id]", {
    params: Promise.resolve(params),
    searchParams: Promise.resolve(query),
  });
  expect(state.reads[0]?.query).toEqual(query);
  state.reads = [];
  state.value = { server: { serverName: "api.test" } };
  await renderBreadcrumbs("/reverse-proxy/[id]", {
    params: Promise.resolve(params),
    searchParams: Promise.resolve(query),
  });
  expect(state.reads[0]?.query).toBeUndefined();
});
it("failed reads leave only group/item crumbs and never redirect or expose an unknown object", async () => {
  state.failure = true;
  const html = renderToStaticMarkup(
    await renderBreadcrumbs("/reverse-proxy/[id]/edit", {
      params: Promise.resolve(params),
    }),
  );
  expect(html).toContain("구성");
  expect(html).toContain("리버스 프록시");
  expect(html).not.toContain("수정");
  state.failure = false;
  state.value = { name: { secret: "must-never-print" } };
  expect(
    renderToStaticMarkup(
      await renderBreadcrumbs("/admin/tokens/[id]", {
        params: Promise.resolve(params),
      }),
    ),
  ).not.toContain("must-never-print");
});

it("renders the separate password page breadcrumb without any data read", async () => {
  const html = renderToStaticMarkup(
    await renderBreadcrumbs("/account/password"),
  );
  expect(html).toContain('href="/account"');
  expect(html).toContain('aria-current="page"');
  expect(html).toContain("비밀번호 변경");
  expect(state.reads).toEqual([]);
});
