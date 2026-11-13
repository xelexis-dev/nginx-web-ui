import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement } from "react";
import type { ServerDetailProps, ServerDetailVM } from "@/components/reverse-proxy/server-detail";
import type { LoadNodePageAllArgs } from "@/lib/node-page";

/**
 * 노드 리버스 프록시 **상세**가 붙드는 것.
 *
 * 이 화면은 라우트 파라미터가 둘이다(`[id]` = 노드, `[sid]` = 서버). 둘을
 * 헷갈리면 **조용히 엉뚱한 것을 가져온다** — 노드 id 로 서버를 조회하면 404 가
 * 뜨고 사용자는 「서버가 사라졌다」로 읽는다. 그래서 노드 id 와 서버 id 를
 * 일부러 다른 값으로 두고 어느 쪽이 어디에 쓰이는지 본다.
 *
 * 그리고 상세는 **「다시 시도」와 「목록으로」가 서로 달라야 하는** 화면이다 —
 * 항목이 없어서 카드가 뜬 것이므로 같은 주소로 되돌리면 같은 카드가 다시 뜬다.
 */
const loadNodePageAllMock = vi.fn();
vi.mock("@/lib/node-page", () => ({
  loadNodePageAll: loadNodePageAllMock,
}));

// `@repo/ui`·CSS 모듈 사슬을 이 프로세스로 끌고 오지 않기 위한 목이다(검사 대상 아님).
vi.mock("@/components/reverse-proxy/server-detail", () => ({ ServerDetail: () => null }));

// 액션 둘은 **`nodeId` 로 묶여서** 내려간다 — `.bind` 된 인자는 밖에서 들여다볼
// 수 없으므로 불러서 확인한다(목록 화면의 `applyAction` 과 같은 관용).
const toggleNodeServerMock = vi.fn();
const deleteNodeServerMock = vi.fn();
vi.mock("../../actions", () => ({
  toggleNodeServer: (...args: unknown[]) => toggleNodeServerMock(...args),
  deleteNodeServer: (...args: unknown[]) => deleteNodeServerMock(...args),
}));

/**
 * `can` 은 **동작별로** 답한다 — 고정 `true` 로 두면 등급 판정을 지운 변이가
 * 통과한다(목록 화면의 같은 자리와 같은 이유).
 */
const canMock = vi.fn((_role: string, _action: string) => true);
vi.mock("@/lib/permissions", () => ({ can: (role: string, action: string) => canMock(role, action) }));

const { default: NodeServerDetailPage } = await import("./page");

type Key = "server" | "upstreams" | "certificates";
type Args = LoadNodePageAllArgs<Key, ServerDetailVM>;

const NODE_ID = "node-1";
const SERVER_ID = "srv-9";
const BASE = `/nodes/${NODE_ID}/reverse-proxy`;

const NODE_UPSTREAMS = { upstreams: [{ id: "up-1", name: "node-pool", servers: ["127.0.0.1:3000"] }] };

/** `certificateView` 가 내려주는 모양 — **`certPem`/`keyPem` 이 없다**(허용목록). */
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
      upstreamId: "up-1",
      certificateId: null,
      config: {},
      ...extra,
    },
  };
}

beforeEach(() => {
  loadNodePageAllMock.mockReset();
  loadNodePageAllMock.mockResolvedValue({ ok: false, render: null });
  toggleNodeServerMock.mockReset();
  deleteNodeServerMock.mockReset();
  canMock.mockReset();
  canMock.mockReturnValue(true);
});

async function captureArgs(): Promise<Args> {
  await NodeServerDetailPage({ params: Promise.resolve({ id: NODE_ID, sid: SERVER_ID }) });
  expect(loadNodePageAllMock).toHaveBeenCalledTimes(1);
  return loadNodePageAllMock.mock.calls[0]![0] as Args;
}

function viewModel(args: Args, raw: Partial<Record<Key, unknown>> = {}): ServerDetailVM {
  return args.toViewModel({
    server: serverResponse(),
    upstreams: NODE_UPSTREAMS,
    certificates: NODE_CERTIFICATES,
    ...raw,
  } as Record<Key, unknown>);
}

describe("노드 리버스 프록시 상세 — 무엇을 어디서 읽는가", () => {
  it("서버 한 행 + upstream 전부 + 인증서를 한 번에 읽는다", async () => {
    const args = await captureArgs();

    // **노드 id 가 아니라 서버 id 로 조회한다** — 둘을 바꿔 넣으면 404 가 뜨고
    // 사용자는 「서버가 사라졌다」로 읽는다.
    expect(args.paths.server).toBe(`/api/v1/servers/${SERVER_ID}`);
    expect(args.paths.upstreams).toBe("/api/v1/upstreams");
    expect(args.paths.certificates).toBe("/api/v1/certificates");
    expect(args.nodeId).toBe(NODE_ID);
    expect(loadNodePageAllMock).toHaveBeenCalledTimes(1);
  });

  it("경로의 서버 id 를 이스케이프한다 — 주소창 값이 노드 API 경로를 벗어나지 못한다", async () => {
    await NodeServerDetailPage({ params: Promise.resolve({ id: NODE_ID, sid: "../../apply" }) });
    const args = loadNodePageAllMock.mock.calls[0]![0] as Args;

    expect(args.paths.server).toBe("/api/v1/servers/..%2F..%2Fapply");
  });

  /**
   * 카드 하나가 조회 셋을 대표한다. 이름이 없으면 인증서 조회가 404 인 것을
   * 사용자는 「이 서버가 사라졌다」로 읽고, 그 밑의 「목록으로 돌아가기」가 그
   * 오해를 확인해 준다(`NodeErrorCardProps.subject` 문서).
   */
  it("경로마다 서로 다른 이름을 준다", async () => {
    const args = await captureArgs();

    expect(Object.keys(args.subjects).sort()).toEqual(["certificates", "server", "upstreams"]);
    expect(new Set(Object.values(args.subjects)).size).toBe(3);
  });

  it("상세라서 「다시 시도」와 「목록으로」가 다르다 — 같으면 막다른 길이다", async () => {
    const args = await captureArgs();

    expect(args.retryHref).toBe(`${BASE}/${SERVER_ID}`);
    expect(args.listHref).toBe(BASE);
    expect(args.retryHref).not.toBe(args.listHref);
  });
});

describe("노드 리버스 프록시 상세 — 응답 → 뷰모델", () => {
  it("대상 이름과 참조 블록에 **노드의** upstream 을 쓴다", async () => {
    const vm = viewModel(await captureArgs());

    expect(vm.upstreamId).toBe("up-1");
    expect(vm.upstreams).toEqual([{ id: "up-1", name: "node-pool", servers: ["127.0.0.1:3000"] }]);
  });

  it("연결된 인증서가 issued 면 TLS 로 렌더한다", async () => {
    const args = await captureArgs();
    const vm = viewModel(args, { server: serverResponse({ certificateId: "cert-1" }) });

    expect(vm.cert).toEqual({
      id: "cert-1",
      domains: ["a.example.com"],
      staging: false,
      issued: true,
      tlsReady: true,
    });
  });

  it("발급 전 인증서는 TLS 로 렌더하지 않는다 — 라벨은 「연결됨」이다", async () => {
    const args = await captureArgs();
    const vm = viewModel(args, { server: serverResponse({ certificateId: "cert-2" }) });

    expect(vm.cert?.issued).toBe(false);
    expect(vm.cert?.tlsReady).toBe(false);
    // 연결은 돼 있다 — 「없음 (HTTP)」과 「발급 대기/실패」는 다른 상태다.
    expect(vm.certificateId).toBe("cert-2");
  });

  /**
   * 연결된 인증서가 목록에 없으면 **삭제된 것**이다. `certificateId` 만 보고
   * `cert` 를 지어내면 공유 컴포넌트가 「연결된 인증서가 삭제됨」 대신 엉뚱한
   * 도메인을 보여준다.
   */
  it("연결된 인증서가 목록에 없으면 cert 는 null 이다 — certificateId 는 남는다", async () => {
    const args = await captureArgs();
    const vm = viewModel(args, { server: serverResponse({ certificateId: "gone" }) });

    expect(vm.cert).toBeNull();
    expect(vm.certificateId).toBe("gone");
  });

  /**
   * 자기 자신을 가리키는지 안내는 **게이트웨이 화면만** 판정할 수 있다 —
   * 필요한 「집계 제외 도메인」이 `/api/v1` 에 없고, 안내가 가리키는 설정 화면은
   * 게이트웨이 자신의 것이다. 노드에서 띄우면 엉뚱한 서버를 고치라고 말한다.
   */
  it("자기 프록시 안내는 노드 화면에서 절대 뜨지 않는다", async () => {
    const vm = viewModel(await captureArgs());

    expect(vm.selfProxyNotice).toBe(false);
  });

  /** 쓰기 UI 를 볼 때 화면을 실제로 렌더한다(`loadNodePageAll` 이 성공한 상태). */
  async function renderDetail(effectiveRole: string): Promise<ServerDetailProps> {
    loadNodePageAllMock.mockResolvedValue({
      ok: true,
      node: { id: NODE_ID },
      effectiveRole,
      data: { id: SERVER_ID },
    });

    const rendered = await NodeServerDetailPage({ params: Promise.resolve({ id: NODE_ID, sid: SERVER_ID }) });
    if (!isValidElement<ServerDetailProps>(rendered)) throw new Error("엘리먼트가 아닙니다");
    return rendered.props;
  }

  /**
   * 「수정」의 목적지는 **노드 id 가 아니라 서버 id** 로 만들어진다. 둘 다
   * 문자열이라 바꿔 넣어도 타입이 안 잡히고, 화면은 멀쩡히 뜬 채 링크만 엉뚱한
   * 곳(자기 자신의 노드를 서버로 찾는 수정 화면)을 가리킨다.
   */
  it("쓰기 등급이면 「수정」이 이 서버의 수정 화면으로 간다", async () => {
    const props = await renderDetail("editor");

    expect(props.write?.editHref).toBe(`${BASE}/${SERVER_ID}/edit`);
    expect(props.basePath).toBe(BASE);
  });

  /**
   * 토글·삭제는 `nodeId` 로 묶여 내려간다. **묶는 값이 서버 id 면** 그 액션은
   * 존재하지 않는 노드를 고치려 들고, 사용자는 「노드를 찾을 수 없습니다」를 본다.
   * `.bind` 된 인자는 밖에서 못 보므로 **불러서** 확인한다.
   */
  it("토글·삭제를 노드 id 로 묶는다 — 서버 id 가 아니다", async () => {
    const props = await renderDetail("admin");

    expect(props.write!.toggleAction).toEqual({id:"node.toggleNodeServer",args:[NODE_ID]});
    expect(props.write!.deleteAction).toEqual({id:"node.deleteNodeServer",args:[NODE_ID]});
  });

  it("쓰기 등급이 없으면 write 는 통째로 null 이다", async () => {
    canMock.mockImplementation((_role, action) => action !== "config:write");

    const props = await renderDetail("viewer");

    expect(props.write).toBeNull();
  });

  /**
   * 「생성되는 nginx 설정」 블록은 **게이트웨이 프로세스**가 렌더한다. 그 안의
   * `certFilePaths` 는 순수 함수가 아니라 `process.env.CERTS_DIR` 을 읽으므로
   * (`packages/nginx/src/apply.ts` 의 `certsDir`), 인증서 경로가 **게이트웨이의**
   * 설정에서 온다. `"this-instance"` 로 두면 화면이 「설정 적용 시 nginx에
   * **그대로** 기록됩니다」라고 말하는데, 그 문장은 노드에서 거짓이 될 수 있다.
   */
  it("설정 블록을 「노드가 적용할 것을 게이트웨이가 렌더한 것」으로 표시한다", async () => {
    loadNodePageAllMock.mockResolvedValue({
      ok: true,
      node: { id: NODE_ID },
      effectiveRole: "viewer",
      data: { id: SERVER_ID },
    });

    const rendered = await NodeServerDetailPage({ params: Promise.resolve({ id: NODE_ID, sid: SERVER_ID }) });
    if (!isValidElement<ServerDetailProps>(rendered)) throw new Error("엘리먼트가 아닙니다");

    expect(rendered.props.configOrigin).toBe("remote-node");
  });
});

it("preserves the remote API preview without gateway rendering", async () => {
  const preview={config:"ssl_certificate /remote/only/cert.pem;",error:null};
  const vm=viewModel(await captureArgs(),{server:{...serverResponse(),preview}});
  expect(vm.preview).toEqual(preview);
});

it("passes the target node preview unresolved paths to the shared detail", async () => {
  const args = await captureArgs();
  const preview = { config: "# node preview", error: null, unresolvedPaths: ["/node-only"] };
  const vm = viewModel(args, { server: { ...serverResponse(), preview } });
  expect(vm.preview).toEqual(preview);
});
