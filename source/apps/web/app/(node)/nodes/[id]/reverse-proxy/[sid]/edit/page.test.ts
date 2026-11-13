import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement } from "react";
import type { ServerFormValues } from "@/components/reverse-proxy/server-form-vm";
import type { LoadNodePageAllArgs } from "@/lib/node-page";

/**
 * 노드 리버스 프록시 **수정** 폼이 붙드는 것. 이 계획에서 엔드포인트를 가장 많이
 * 부르는 화면이다(서버 한 행 + upstream 전부 + 인증서 전부).
 *
 * 라우트 파라미터가 둘이라(`[id]` = 노드, `[sid]` = 서버) 상세 화면과 같은
 * 함정이 있다 — 둘을 헷갈리면 조용히 엉뚱한 것을 가져온다. 그래서 여기서도
 * 노드 id 와 서버 id 를 일부러 다른 값으로 둔다.
 *
 * 그리고 **후보 목록이 노드 것인가**가 생성 화면보다 더 중요하다: 저장된
 * location 의 `http://<name>` 을 upstream 모드로 되돌리는 데 그 목록이 쓰이므로,
 * 남의 목록을 주면 **upstream 으로 저장된 location 이 직접 주소로 열리고** 저장
 * 하는 순간 그 값이 굳는다.
 */
const loadNodePageAllMock = vi.fn();
vi.mock("@/lib/node-page", () => ({
  loadNodePageAll: loadNodePageAllMock,
}));

vi.mock("@/components/reverse-proxy/server-form", () => ({ ServerForm: () => null }));

const updateNodeServerMock = vi.fn();
vi.mock("../../../actions", () => ({
  updateNodeServer: (...args: unknown[]) => updateNodeServerMock(...args),
}));

const { default: EditNodeServerPage } = await import("./page");

type Key = "server" | "upstreams" | "certificates";
interface EditData {
  values: ServerFormValues;
  upstreams: { id: string; name: string }[];
  certificates: { id: string; label: string }[];
}
type Args = LoadNodePageAllArgs<Key, EditData>;

const NODE_ID = "node-1";
const SERVER_ID = "srv-9";
const BASE = `/nodes/${NODE_ID}/reverse-proxy`;

/** 노드가 `GET /api/v1/upstreams` 로 주는 것 — 게이트웨이 것과 **다른 이름**을 쓴다. */
const NODE_UPSTREAMS = {
  upstreams: [{ id: "up-1", name: "node-pool", servers: ["127.0.0.1:3000"] }],
};

const NODE_CERTIFICATES = {
  certificates: [
    { id: "cert-1", domains: ["a.example.com"], staging: false, status: "issued" },
    { id: "cert-2", domains: ["b.example.com"], staging: true, status: "pending" },
  ],
};

function serverResponse(extra: Record<string, unknown> = {}) {
  return {
    server: {
      id: SERVER_ID,
      serverName: "a.example.com",
      listen: 80,
      enabled: true,
      upstreamId: null,
      certificateId: null,
      config: {},
      ...extra,
    },
  };
}

beforeEach(() => {
  loadNodePageAllMock.mockReset();
  updateNodeServerMock.mockReset();
  loadNodePageAllMock.mockResolvedValue({ ok: false, render: null });
});

async function captureArgs(): Promise<Args> {
  await EditNodeServerPage({ params: Promise.resolve({ id: NODE_ID, sid: SERVER_ID }) });
  expect(loadNodePageAllMock).toHaveBeenCalledTimes(1);
  return loadNodePageAllMock.mock.calls[0]![0] as Args;
}

function viewModel(args: Args, raw: Partial<Record<Key, unknown>> = {}): EditData {
  return args.toViewModel({
    server: serverResponse(),
    upstreams: NODE_UPSTREAMS,
    certificates: NODE_CERTIFICATES,
    ...raw,
  } as Record<Key, unknown>);
}

describe("노드 서버 수정 폼 — 무엇을 어디서 읽는가", () => {
  it("서버 한 행 + upstream 전부 + 인증서를 한 번에 읽는다", async () => {
    const args = await captureArgs();

    // **노드 id 가 아니라 서버 id 로 조회한다.**
    expect(args.paths).toEqual({
      server: `/api/v1/servers/${SERVER_ID}`,
      upstreams: "/api/v1/upstreams",
      certificates: "/api/v1/certificates",
    });
    expect(args.nodeId).toBe(NODE_ID);
    expect(loadNodePageAllMock).toHaveBeenCalledTimes(1);
  });

  it("경로의 서버 id 를 이스케이프한다 — 주소창 값이 노드 API 경로를 벗어나지 못한다", async () => {
    await EditNodeServerPage({ params: Promise.resolve({ id: NODE_ID, sid: "../../apply" }) });
    const args = loadNodePageAllMock.mock.calls[0]![0] as Args;

    expect(args.paths.server).toBe("/api/v1/servers/..%2F..%2Fapply");
  });

  it("쓰기 등급으로 연다 — 읽기 등급으로 열면 viewer 에게 헛폼이 그려진다", async () => {
    expect((await captureArgs()).action).toBe("config:write");
  });

  it("경로마다 서로 다른 이름을 준다", async () => {
    const args = await captureArgs();

    expect(Object.keys(args.subjects).sort()).toEqual(["certificates", "server", "upstreams"]);
    expect(new Set(Object.values(args.subjects)).size).toBe(3);
  });

  /**
   * `not_found` 는 「그 서버가 이미 없다」다 — 「목록으로」가 이 수정 화면을
   * 가리키면 같은 카드가 다시 뜨는 막다른 길이 된다.
   */
  it("「다시 시도」는 이 수정 화면, 「목록으로」는 목록이다", async () => {
    const args = await captureArgs();

    expect(args.retryHref).toBe(`${BASE}/${SERVER_ID}/edit`);
    expect(args.listHref).toBe(BASE);
    expect(args.retryHref).not.toBe(args.listHref);
  });
});

describe("노드 서버 수정 폼 — 응답 → 초기값", () => {
  it("초기값을 **노드의** 서버 행에서 만든다", async () => {
    const data = viewModel(await captureArgs(), {
      server: serverResponse({ serverName: "b.example.com", listen: 8080, certificateId: "cert-1" }),
    });

    expect(data.values).toMatchObject({
      id: SERVER_ID,
      serverName: "b.example.com",
      listen: 8080,
      certificateId: "cert-1",
    });
  });

  /**
   * **후보 목록이 노드 것이어야 하는 이유가 수정 화면에서 하나 더 있다.**
   * 저장된 location 은 `http://<upstream 이름>` 이고, 그것을 upstream 모드로
   * 되돌리려면 **그 노드의** 이름 목록이 필요하다. 게이트웨이 목록을 주면 이름이
   * 안 맞아 직접 proxy_pass 로 열리고, 사용자가 저장하는 순간 그 값이 굳는다.
   */
  it("저장된 http://<name> 을 노드의 upstream 목록으로 되돌린다", async () => {
    const data = viewModel(await captureArgs(), {
      server: serverResponse({
        config: { locations: [{ path: "/", proxyPass: "http://node-pool", directives: [] }] },
      }),
    });

    expect(data.values.locations?.[0]).toMatchObject({ mode: "upstream", upstreamId: "up-1" });
    // 그리고 Select 후보도 같은 목록이다 — 되돌린 id 가 후보에 없으면 폼이 빈
    // 선택으로 열린다.
    expect(data.upstreams).toEqual([{ id: "up-1", name: "node-pool" }]);
  });

  it("발급 완료가 아닌 인증서는 TLS 후보에 없다", async () => {
    expect(viewModel(await captureArgs()).certificates).toEqual([
      { id: "cert-1", label: "a.example.com", staging: false, domains: ["a.example.com"] },
    ]);
  });
});

describe("노드 서버 수정 폼 — 액션 배선", () => {
  it("수정 액션을 이 노드 id 로 묶고, 취소는 이 서버의 상세로 간다", async () => {
    const values = { id: SERVER_ID, serverName: "a.example.com", listen: 80, upstreamId: null, proxyPass: null };
    loadNodePageAllMock.mockResolvedValue({
      ok: true,
      node: { id: NODE_ID },
      effectiveRole: "editor",
      data: { values, upstreams: [{ id: "up-1", name: "node-pool" }], certificates: [] },
    });

    const rendered = await EditNodeServerPage({ params: Promise.resolve({ id: NODE_ID, sid: SERVER_ID }) });
    if (!isValidElement<{ children?: unknown }>(rendered)) throw new Error("엘리먼트가 아닙니다");

    const children = (rendered.props as { children: unknown[] }).children;
    const formEl = children.at(-1);
    if (!isValidElement<Record<string, unknown>>(formEl)) throw new Error("폼 엘리먼트가 아닙니다");

    expect(formEl.props.action).toEqual({id:"node.updateNodeServer",args:[NODE_ID]});

    // **취소는 목록이 아니라 이 서버의 상세다** — 수정을 그만두면 보고 있던
    // 항목으로 돌아가는 것이 맞다(로컬 수정 화면과 같다).
    expect(formEl.props.cancelHref).toBe(`${BASE}/${SERVER_ID}`);
    expect(formEl.props.defaultValues).toBe(values);
  });
});

it.each([
  ["1.19.11", { mode: "upstream", upstreamId: "up-1", proxyPass: "" }, false],
  ["1.19.11", { mode: "direct", upstreamId: "", proxyPass: "http://app.test:9000", retryOtherInstances: true }, true],
  ["1.19.11", { mode: "direct", upstreamId: "", proxyPass: "http://app.test:9000", resolveAtRuntime: true }, true],
  ["1.19.11", { mode: "direct", upstreamId: "", proxyPass: "" }, true],
  ["2.0.0", { mode: "direct", upstreamId: "", proxyPass: "http://app.test:9000", retryOtherInstances: true }, false],
])("gates only unrepresentable saved routes on %s", async (lastVersion, route, readOnly) => {
  loadNodePageAllMock.mockResolvedValue({ok:true,node:{id:NODE_ID,name:"Node",lastVersion},effectiveRole:"editor",data:{values:{serverName:"app.test",listen:18440,upstreamId:null,proxyPass:null,locations:[{path:"/",...route}]},upstreams:[],certificates:[]}});
  const rendered=await EditNodeServerPage({params:Promise.resolve({id:NODE_ID,sid:SERVER_ID})});
  if (!isValidElement<{children:unknown[]}>(rendered)) throw new Error("missing page");
  const content=rendered.props.children.at(-1);
  if (!isValidElement<Record<string,unknown>>(content)) throw new Error("missing page content");
  if (readOnly) {
    expect(content.props.title).toBe("이 노드는 1.19라 이 구성을 편집할 수 없습니다 — 노드를 2.0으로 올리세요");
    expect(content.props.action).toBeUndefined();
  } else {
    expect(content.props.action).toEqual({id:"node.updateNodeServer",args:[NODE_ID]});
    expect(content.props.nodeVersionFamily).toBe(lastVersion.startsWith("1.19.") ? "1.19" : "2.0");
  }
});
