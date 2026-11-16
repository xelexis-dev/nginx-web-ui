import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup as renderMarkup } from "react-dom/server";
import { ToastProvider } from "@/components/notifications/toast-provider";
const renderToStaticMarkup = (node: ReactNode) =>
  renderMarkup(<ToastProvider>{node}</ToastProvider>);
import type { ReactNode } from "react";
import type { NodeActionState } from "@/lib/node-action-state";

const mocks = vi.hoisted(() => ({
  actions: [] as Array<
    (state: NodeActionState, form: FormData) => Promise<NodeActionState>
  >,
  submit: vi.fn(),
  push: vi.fn(),
  refresh: vi.fn(),
}));
vi.mock("react", async (importOriginal) => ({
  ...(await importOriginal<typeof import("react")>()),
  useActionState: (
    action: (
      state: NodeActionState,
      form: FormData,
    ) => Promise<NodeActionState>,
    initial: NodeActionState,
  ) => {
    if (typeof action !== "function")
      throw new Error("useActionState requires a browser callback");
    mocks.actions.push(action);
    return [initial, () => undefined, false];
  },
}));
vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: mocks.push, refresh: mocks.refresh }),
}));
vi.mock("@/lib/api-transport", () => ({ submitApiForm: mocks.submit }));

import { UpstreamForm } from "@/components/upstreams/upstream-form";
import { StreamForm } from "@/components/streams/stream-form";
import { ServerForm } from "./server-form";
import { UpstreamDetailActions } from "@/components/upstreams/detail-actions";
import { StreamDetailActions } from "@/components/streams/detail-actions";
import { ServerDetailActions } from "./detail-actions";
import { ApplyResult } from "@/components/apply-bar";
import { ApplyDialog } from "@/components/apply-dialog";

beforeEach(() => {
  mocks.actions.length = 0;
  vi.clearAllMocks();
});

it.each([
  [
    "upstreams.createUpstream",
    (id: string) => (
      <UpstreamForm
        action={{ id, args: [] }}
        cancelHref="/reverse-proxy/upstreams"
      />
    ),
  ],
  [
    "stream.createStream",
    (id: string) => (
      <StreamForm action={{ id, args: [] }} cancelHref="/stream" />
    ),
  ],
  [
    "reverse-proxy.createServer",
    (id: string) => (
      <ServerForm
        action={{ id, args: [] }}
        upstreams={[]}
        certificates={[]}
        cancelHref="/reverse-proxy"
      />
    ),
  ],
] as const)(
  "%s wraps API commands and preserves field failures",
  async (id, component) => {
    renderToStaticMarkup(component(id));
    const result = { ok: false, fieldErrors: { listen: "예약된 포트입니다." } };
    mocks.submit.mockResolvedValue({ result });
    const form = new FormData();
    form.set("listen", "3001");
    expect(await mocks.actions[0]!({ ok: false }, form)).toBe(result);
    expect(mocks.submit).toHaveBeenCalledExactlyOnceWith(
      { id, args: [] },
      form,
    );
    expect(mocks.push).not.toHaveBeenCalled();
  },
);

it.each([
  [
    <UpstreamDetailActions
      editHref="/reverse-proxy/upstreams/u1/edit"
      id="u1"
      action={{ id: "upstreams.deleteUpstream", args: [] }}
    />,
    ["upstreams.deleteUpstream"],
  ],
  [
    <StreamDetailActions
      editHref="/stream/l1/edit"
      id="l1"
      enabled
      toggleAction={{ id: "stream.toggleStream", args: [] }}
      deleteAction={{ id: "stream.deleteStream", args: [] }}
    />,
    ["stream.toggleStream", "stream.deleteStream"],
  ],
  [
    <ServerDetailActions
      editHref="/reverse-proxy/s1/edit"
      id="s1"
      serverName="s1.example.test"
      enabled
      toggleAction={{ id: "reverse-proxy.toggleServer", args: [] }}
      deleteAction={{ id: "reverse-proxy.deleteServer", args: [] }}
    />,
    ["reverse-proxy.toggleServer", "reverse-proxy.deleteServer"],
  ],
] as Array<[ReactNode, string[]]>)(
  "detail controls send command references and refresh after mutations",
  async (component, ids) => {
    renderToStaticMarkup(component);
    mocks.submit.mockResolvedValue({ result: { ok: true }, refresh: true });
    const form = new FormData();
    form.set("id", "resource-id");
    for (const action of mocks.actions) await action({ ok: false }, form);
    expect(mocks.submit.mock.calls.map((c) => c[0].id)).toEqual(ids);
    expect(mocks.refresh).toHaveBeenCalledTimes(ids.length);
  },
);

it("the shared form still accepts a remote node callback", async () => {
  const legacy = vi.fn(async () => ({ ok: false, error: "원격 연결 실패" }));
  renderToStaticMarkup(
    <UpstreamForm
      action={legacy}
      cancelHref="/nodes/n1/reverse-proxy/upstreams"
    />,
  );
  const form = new FormData();
  expect(await mocks.actions[0]!({ ok: false }, form)).toEqual({
    ok: false,
    error: "원격 연결 실패",
  });
  expect(legacy).toHaveBeenCalledExactlyOnceWith({ ok: false }, form);
  expect(mocks.submit).not.toHaveBeenCalled();
});

it("the apply bar submits an API reference and displays transport failure without claiming rollback", async () => {
  renderToStaticMarkup(
    <ApplyDialog
      open
      onClose={() => {}}
      target={{ kind: "local" }}
      targetLabel="로컬 서버"
      canApply
      pendingHasBaseline={true}
      pendingCount={1}
      pendingDiff={{
        servers: [],
        streams: [],
        upstreams: [],
        hasChanges: false,
      }}
      applyAction={{ id: "apply.applyConfig", args: [] }}
    />,
  );
  mocks.submit.mockResolvedValue({
    result: { ok: false, error: "API 서버에 연결하지 못했습니다." },
  });
  const result = await mocks.actions[0]!({ ok: false }, new FormData());
  const html = renderToStaticMarkup(<ApplyResult result={result} />);
  expect(html).toContain("API 서버에 연결하지 못했습니다.");
  expect(html).not.toContain("변경 사항은 롤백됨");
});

it.each([
  ["401", { ok: false, error: "로그인이 필요합니다." }],
  ["403", { ok: false, error: "권한이 없습니다." }],
  ["5xx", { ok: false, error: "서버에서 저장하지 못했습니다." }],
  ["transport", { ok: false, error: "API 서버에 연결하지 못했습니다." }],
  ["409", { ok: false, error: "설정이 변경되었습니다. 다시 확인하세요." }],
] as const)(
  "server form preserves the original %s failure and comparison version",
  async (_kind, result) => {
    const html = renderToStaticMarkup(
      <ServerForm
        action={{ id: "reverse-proxy.updateServer", args: [] }}
        upstreams={[]}
        certificates={[]}
        defaultValues={{
          id: "s",
          version: "captured-v1",
          serverName: "app.example.com",
          listen: 80,
          upstreamId: null,
          proxyPass: "http://127.0.0.1:3000",
        }}
        cancelHref="/reverse-proxy/s"
      />,
    );
    expect(html).toContain('name="version" value="captured-v1"');
    expect(html).toContain('name="targetMode" value="locations"');
    mocks.submit.mockResolvedValue({ result });
    expect(await mocks.actions[0]!({ ok: false }, new FormData())).toBe(result);
    expect(mocks.push).not.toHaveBeenCalled();
  },
);
