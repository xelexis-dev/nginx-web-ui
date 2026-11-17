import { expect, it, vi } from "vitest";
import { renderToStaticMarkup as renderMarkup } from "react-dom/server";
import { ToastProvider } from "@/components/notifications/toast-provider";
import type { ReactNode } from "react";
const renderToStaticMarkup = (node: ReactNode) =>
  renderMarkup(<ToastProvider>{node}</ToastProvider>);
import type { NodeActionState } from "@/lib/node-action-state";

// Category cards retain one draft; submit always uses canonical per-route targets.
const mocks = vi.hoisted(() => ({
  state: { ok: false } as NodeActionState,
  pending: false,
}));
vi.mock("react", async (importOriginal) => ({
  ...(await importOriginal<typeof import("react")>()),
  useActionState: () => [mocks.state, () => undefined, mocks.pending],
}));
vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }),
}));

import { ServerForm } from "./server-form";

const render = () =>
  renderToStaticMarkup(
    <ServerForm
      action={{ id: "reverse-proxy.createServer", args: [] }}
      upstreams={[]}
      certificates={[]}
      cancelHref="/reverse-proxy"
    />,
  );

it("keeps all category cards mounted inside one per-route form", () => {
  mocks.state = { ok: false };
  const html = render();
  expect((html.match(/role="tab"/g) ?? []).length).toBe(0);
  expect((html.match(/role="tabpanel"/g) ?? []).length).toBe(0);
  for (const id of [
    "identity",
    "https",
    "access",
    "routes-card",
    "cache",
    "gzip",
    "common",
  ])
    expect(html).toContain(`id="server-form-${id}"`);
  expect(html).toContain('name="targetMode" value="locations"');
  expect(html).not.toContain('name="proxyPass"');
  expect(html).toContain('name="serverName"');
  expect(html).toContain('id="ip-mode"');
  expect(html).toContain("한눈에 보기");
});
it("announces all field failures in the summary and each owning view", () => {
  mocks.state = {
    ok: false,
    fieldErrors: {
      listen: "예약된 포트입니다.",
      serverName: "도메인을 입력하세요.",
      certificateId: "인증서가 없습니다.",
      realIp: "범위가 비었습니다.",
    },
  };
  const html = render();
  expect(html).toContain("입력 오류 4개");
  for (const message of Object.values(mocks.state.fieldErrors!))
    expect(html).toContain(message);
});
it("keeps an indexed location failure verbatim on the selected card and summary", () => {
  mocks.state = {
    ok: false,
    fieldErrors: {
      locations: 'location 1 (/): Invalid proxy_pass target: "broken"',
    },
  };
  const html = render();
  expect(
    html.match(/location 1 \(\/\): Invalid proxy_pass target:/g)?.length,
  ).toBeGreaterThanOrEqual(2);
});

import { toServerFormValues } from "./server-form-vm";
function posted(html: string, name: string) {
  const raw = new RegExp(`name="${name}"[^>]*value="([^"]*)"`).exec(html)?.[1];
  return raw
    ? JSON.parse(raw.replaceAll("&quot;", '"').replaceAll("&amp;", "&"))
    : undefined;
}
it.each([
  undefined,
  [],
  [
    { name: "proxy_read_timeout", args: ["300s"] },
    { name: "proxy_set_header", args: ["Host", "$host"] },
  ],
])(
  "carries the actual legacy directive shape through the UI: %j",
  (directives) => {
    mocks.state = { ok: false };
    const config =
      directives === undefined
        ? { proxyPass: "http://127.0.0.1:3000" }
        : {
            proxyPass: "http://127.0.0.1:3000",
            locationDirectives: directives,
          };
    const values = toServerFormValues(
      {
        id: "s",
        version: "v1",
        serverName: "app.example.com",
        listen: 80,
        enabled: true,
        upstreamId: null,
        certificateId: null,
        config,
      },
      [],
    );
    const html = renderToStaticMarkup(
      <ServerForm
        action={{ id: "reverse-proxy.updateServer", args: [] }}
        upstreams={[]}
        certificates={[]}
        defaultValues={values}
        cancelHref="/reverse-proxy/s"
      />,
    );
    expect(posted(html, "locations")[0].directives).toEqual(directives ?? null);
    expect(posted(html, "serverDirectives")).toBeNull();
    expect(html).toContain("저장하면 이 서버가 경로별 형식으로 바뀝니다");
    if (directives === undefined)
      expect(html).toContain("기존 기본값 (디렉티브 없음)");
  },
);
it("shows a dangling ID in both the route and summary without posting a stale ID", () => {
  mocks.state = { ok: false };
  const values = toServerFormValues(
    {
      id: "s",
      serverName: "app.example.com",
      listen: 80,
      enabled: true,
      upstreamId: null,
      certificateId: null,
      config: { locations: [{ path: "/", upstreamId: "gone" }] },
    },
    [],
  );
  const html = renderToStaticMarkup(
    <ServerForm
      action={{ id: "reverse-proxy.updateServer", args: [] }}
      upstreams={[]}
      certificates={[]}
      defaultValues={values}
      cancelHref="/reverse-proxy/s"
    />,
  );
  expect(
    html.match(/연결된 Upstream을 찾을 수 없습니다/g)?.length,
  ).toBeGreaterThanOrEqual(3);
  expect(posted(html, "locations")[0].upstreamId).toBe("");
  expect(html).not.toContain('value="gone"');
});

it("shows the node's save context in the identity strip and summary", () => {
  mocks.state = { ok: false };
  const html = renderToStaticMarkup(
    <ServerForm
      action={{ id: "node.createNodeServer", args: [] }}
      upstreams={[]}
      certificates={[]}
      nodeName="node-seoul"
      target={{ kind: "node", id: "n", name: "admin" }}
      cancelHref="/nodes/n/reverse-proxy"
    />,
  );
  expect(html).toContain("저장 위치: 노드");
  expect(html).toContain("저장 대상: admin");
  expect(html.match(/node-seoul/g)?.length).toBeGreaterThanOrEqual(2);
});

import { directivesToState } from "@repo/presentation/nginx/directive-state";
it("closed directive rows show current values and header names without header values", () => {
  mocks.state = { ok: false };
  const directives = directivesToState([
    {
      name: "proxy_set_header",
      args: ["Authorization", "request-token-secret"],
    },
    { name: "add_header", args: ["X-Auth", "response-token-secret"] },
    { name: "proxy_read_timeout", args: ["73s"] },
    { name: "rewrite", args: ["^/old/(.*)$", "/new/$1", "last"] },
    { name: "return", args: ["204"] },
  ]);
  const html = renderToStaticMarkup(
    <ServerForm
      action={{ id: "reverse-proxy.updateServer", args: [] }}
      upstreams={[]}
      certificates={[]}
      defaultValues={{
        serverName: "app.example.com",
        listen: 80,
        upstreamId: null,
        proxyPass: null,
        locations: [
          {
            path: "/",
            mode: "direct",
            upstreamId: "",
            proxyPass: "http://127.0.0.1:3000",
            directives,
          },
        ],
      }}
      cancelHref="/reverse-proxy/s"
    />,
  );
  const closed = [...html.matchAll(/<summary>(.*?)<\/summary>/gs)]
    .map((match) => match[1])
    .join(" ");
  for (const shown of [
    "Authorization",
    "X-Auth",
    "73s",
    "^/old/(.*)$",
    "1줄",
    "return",
  ])
    expect(closed).toContain(shown);
  expect(closed).not.toContain("request-token-secret");
  expect(closed).not.toContain("response-token-secret");
});

it.each(["local", "node"])(
  "keeps a general %s save failure visible in rail, bar and sheet",
  (kind) => {
    const message =
      "다른 사용자가 이 설정을 변경했습니다. 입력 내용을 복사한 뒤 다시 여세요.";
    mocks.state =
      kind === "local"
        ? { ok: false, error: message }
        : {
            ok: false,
            nodeError: {
              failure: { kind: "conflict", message },
              nodeId: "node-a",
              retryHref: "/nodes/node-a/reverse-proxy/s/edit",
              listHref: "/nodes/node-a/reverse-proxy",
            },
          };
    const html = render();
    expect(html).toContain(message);
    expect(html).toContain('aria-label="저장 오류"');
    expect(html.match(/저장하지 못했습니다/g)?.length).toBe(3);
    expect(html.match(/오류 확인/g)?.length).toBe(3);
  },
);

it("keeps validation failures routed to fields without a competing general-error jump", () => {
  mocks.state = {
    ok: false,
    error: "입력값이 올바르지 않습니다.",
    fieldErrors: { locations: "location 1 (/): 대상이 없습니다." },
  };
  const html = render();
  expect(html).toContain("입력 오류 1개");
  expect(html).not.toContain("저장하지 못했습니다");
  expect(html).not.toContain('aria-label="저장 오류"');
  expect(html).not.toContain("입력값이 올바르지 않습니다.");
});

it("locks editable controls and custom menus during submission", () => {
  mocks.state = { ok: false };
  mocks.pending = true;
  try {
    const html = render();
    const fieldset = html.match(/<fieldset[^>]*>/)?.[0];
    expect(fieldset).toContain('disabled=""');
    expect(fieldset).toContain('inert=""');
    expect(html.match(/<form[^>]*>/)?.[0]).toContain('aria-busy="true"');
  } finally {
    mocks.pending = false;
  }
});

it.each([
  [0, 0],
  [3600, 3600],
  [Number.NaN, 63072000],
])(
  "serializes HSTS max-age %s as %s without changing the TLS payload shape",
  (maxAge, expected) => {
    mocks.state = { ok: false };
    const html = renderToStaticMarkup(
      <ServerForm
        action={{ id: "reverse-proxy.createServer", args: [] }}
        upstreams={[]}
        certificates={[]}
        cancelHref="/reverse-proxy"
        defaultValues={{
          serverName: "hsts.example.test",
          listen: 80,
          upstreamId: null,
          proxyPass: "http://192.0.2.1",
          certificateId: "cert-a",
          tlsOptions: {
            hsts: {
              enabled: true,
              maxAge,
              includeSubDomains: false,
              preload: false,
            },
          },
        }}
      />,
    );
    const value = html.match(/name="tlsOptions" value="([^"]*)"/)?.[1];
    expect(value).toBeDefined();
    expect(JSON.parse(value!.replaceAll("&quot;", '"'))).toEqual({
      http2: false,
      hsts: {
        enabled: true,
        maxAge: expected,
        includeSubDomains: false,
        preload: false,
      },
      preferServerCiphers: false,
    });
  },
);

it("keeps every location draft mounted while blocks may collapse independently", () => {
  mocks.state = { ok: false };
  const html = renderToStaticMarkup(
    <ServerForm
      action={{ id: "reverse-proxy.updateServer", args: [] }}
      upstreams={[]}
      certificates={[]}
      cancelHref="/reverse-proxy/s"
      defaultValues={{
        serverName: "blocks.example.test",
        listen: 80,
        upstreamId: null,
        proxyPass: null,
        locations: ["/", "/assets/"].map((path) => ({
          path,
          mode: "direct",
          upstreamId: "",
          proxyPass: "http://app.test:9000",
          directives: directivesToState([]),
        })),
      }}
    />,
  );
  for (const index of [0, 1])
    expect(html).toContain(`id="route-proxy-${index}"`);
  expect(html.match(/<div id="route-block-0-body"[^>]*>/)?.[0]).not.toContain(
    "hidden",
  );
  expect(html.match(/<div id="route-block-1-body"[^>]*>/)?.[0]).toContain(
    'hidden=""',
  );
  expect(html).toContain('aria-label="경로 1 아래로 이동"');
  expect(html).toContain('aria-label="경로 2 위로 이동"');
  expect(
    posted(html, "locations").map((route: { path: string }) => route.path),
  ).toEqual(["/", "/assets/"]);
});

it("uses visible field labels in the rail and puts sheet errors before summary facts", () => {
  mocks.state = { ok: false, fieldErrors: { listen: "예약된 포트입니다.", certificateId: "인증서를 확인하세요." } };
  const html = render();
  expect(html).toContain("listen (포트) — 예약된 포트입니다.");
  expect(html).toContain("인증서 — 인증서를 확인하세요.");
  expect(html).not.toContain("certificateId —");
  const sheet = html.slice(html.indexOf("<dialog"));
  expect(sheet.indexOf("입력 오류 2개")).toBeLessThan(sheet.indexOf("저장할 설정입니다."));
});
