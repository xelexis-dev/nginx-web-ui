import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement } from "react";
import type { ServerListProps } from "@/components/reverse-proxy/server-list";
import type { LoadNodePageAllArgs } from "@/lib/node-page";

/**
 * 노드 리버스 프록시 **목록**이 붙드는 것 셋. 전부 「화면은 멀쩡히 뜨고 타입도
 * 통과하는데 값만 남의 것」인 계열이라 구조로 잡는다.
 *
 * 1. **미적용 배지와 upstream 이름이 노드 값이어야 한다.** 게이트웨이 자신의
 *    `getPendingStatus()`/upstream 목록을 쓰면 「적용할 게 없는데 배지가 떠
 *    있다」(반대도), 「대상 열에 남의 upstream 이름」이 된다.
 * 2. **쓰기 명령이 이 노드로 묶여야 한다.** 행 스위치가 다른 노드 id 나 로컬 명령으로 가면 남의
 *    서버를 바꾼다. (「설정 적용」은 이제 셸 머리줄에 있어서 목록은 적용 명령을 들고 있지 않다 —
 *    노드 적용이 어느 경로로 묶이는지는 `lib/operations-shell-data.ts` 가 정한다.)
 *
 * 목은 `loadNodePageAll` 과 액션 모듈이다. 나머지 목(`ServerList`)은 검사
 * 대상이 아니라 `@repo/ui` 사슬을 이 테스트 프로세스로 끌고 오지 않기 위한 것이다
 * (`stream/page.test.ts` 와 같은 관용). **`toServerRow` 는 목으로 두지 않는다** —
 * 대상 라벨이 노드 upstream 이름으로 만들어지는지가 검사 대상이다.
 */
const loadNodePageAllMock = vi.fn();
vi.mock("@/lib/node-page", () => ({
  loadNodePageAll: loadNodePageAllMock,
}));

vi.mock("@/components/reverse-proxy/server-list", () => ({ ServerList: () => null }));

/** TLS 칸의 인증서는 노드 인증서 뷰를 보조로 읽어 온다 — 여기서 그 결과를 정한다. */
const readCertsMock = vi.fn();
vi.mock("@/components/reverse-proxy/certificate-read", () => ({ readNodeCertificateSummaries: readCertsMock }));

const applyNodeConfigMock = vi.fn();
vi.mock("../actions", () => ({ applyNodeConfig: applyNodeConfigMock }));

/**
 * `can` 은 **동작별로** 답한다 — 고정 `true` 로 두면 「적용 바 판정을 쓰기 UI 에
 * 그대로 쓴다」 같은 변이가 통과한다. 아래 테스트들은 `nginx:reload` 만 통과하는
 * 등급을 만들어 두 판정이 실제로 갈리는지 본다.
 */
const canMock = vi.fn((_role: string, _action: string) => true);
vi.mock("@/lib/permissions", () => ({ can: (role: string, action: string) => canMock(role, action) }));

const { default: NodeReverseProxyPage } = await import("./page");

type Key = "upstreams" | "servers" | "pending";
type Args = LoadNodePageAllArgs<Key, unknown>;

const BASE = "/nodes/node-1/reverse-proxy";

/** 노드가 `GET /api/v1/pending` 으로 주는 것(`lib/pending.ts` 의 `PendingStatus`). */
const NODE_PENDING = {
  count: 4,
  hasBaseline: true,
  // **원소가 하나는 있어야 한다** — 빈 배열만 주면 `readConfigDiff` 의 원소
  // 읽기가 정상 경로에서 한 번도 안 돌고, 「원소를 안 읽는다」 변이가 살아남는다.
  diff: {
    servers: [{ key: "srv-1", label: "a.example.com:443", status: "changed", changes: [{ field: "listen", before: 80, after: 443 }] }],
    streams: [],
    upstreams: [{ key: "up-1", label: "node-pool", status: "added", changes: [] }],
    hasChanges: true,
  },
};

/** 노드가 `GET /api/v1/upstreams` 로 주는 것 — 게이트웨이 것과 **다른 이름**을 쓴다. */
const NODE_UPSTREAMS = { upstreams: [{ id: "up-1", name: "node-pool", servers: ["127.0.0.1:3000"] }] };

const NODE_SERVERS = {
  servers: [
    { id: "srv-1", serverName: "a.example.com", listen: 80, enabled: true, upstreamId: "up-1", config: {} },
    {
      id: "srv-2",
      serverName: "b.example.com",
      listen: 80,
      enabled: false,
      upstreamId: null,
      config: { proxyPass: "http://127.0.0.1:9000" },
    },
  ],
};

beforeEach(() => {
  loadNodePageAllMock.mockReset();
  applyNodeConfigMock.mockReset();
  readCertsMock.mockReset();
  readCertsMock.mockResolvedValue(null);
  canMock.mockReset();
  canMock.mockReturnValue(true);
  // 화면이 조기 반환하도록 실패를 돌려준다 — 우리가 보려는 것은 인자다.
  loadNodePageAllMock.mockResolvedValue({ ok: false, render: null });
});

async function captureArgs(): Promise<Args> {
  await NodeReverseProxyPage({ params: Promise.resolve({ id: "node-1" }) });
  expect(loadNodePageAllMock).toHaveBeenCalledTimes(1);
  return loadNodePageAllMock.mock.calls[0]![0] as Args;
}

describe("노드 리버스 프록시 목록 — 무엇을 어디서 읽는가", () => {
  it("upstream·서버·미적용 변경을 노드에서 한 번에 읽는다", async () => {
    const args = await captureArgs();

    // `pending` 을 빼고 게이트웨이 값을 쓰거나, upstream 을 빼고 이름을 못 붙이면
    // 이 단언이 죽는다.
    expect(args.paths).toEqual({
      upstreams: "/api/v1/upstreams",
      servers: "/api/v1/servers",
      pending: "/api/v1/pending",
    });
    // 실패가 하나로 모이도록 **한 번**만 부른다(`loadNodePageAll` 문서).
    expect(loadNodePageAllMock).toHaveBeenCalledTimes(1);
  });

  it("경로마다 이름을 준다 — 실패 카드가 무엇을 못 읽었는지 말하도록", async () => {
    const args = await captureArgs();

    // 키가 `paths` 와 어긋나면 컴파일이 깨지고, 여기서는 **셋 다 서로 다른**
    // 이름인지를 본다(하나를 복사해 넣으면 카드가 엉뚱한 것을 말한다).
    expect(Object.keys(args.subjects).sort()).toEqual(["pending", "servers", "upstreams"]);
    expect(new Set(Object.values(args.subjects)).size).toBe(3);
  });

  it("배지 숫자·모달 내용과 대상 라벨을 전부 노드 응답에서 꺼낸다", async () => {
    const args = await captureArgs();
    // 화면을 실제로 돌린다: `loadNodePageAll` 이 노드 응답을 이 화면의 `toViewModel` 로 읽은 결과를
    // 돌려주고, 행은 인증서 보조 읽기가 끝난 뒤 화면이 만든다.
    const data = args.toViewModel({ upstreams: NODE_UPSTREAMS, servers: NODE_SERVERS, pending: NODE_PENDING });
    loadNodePageAllMock.mockResolvedValue({ ok: true, node: { id: "node-1" }, effectiveRole: "editor", data });
    const rendered = await NodeReverseProxyPage({ params: Promise.resolve({ id: "node-1" }) });
    if (!isValidElement<ServerListProps>(rendered)) throw new Error("엘리먼트가 아닙니다");
    const vm = rendered.props.data;

    expect(vm.pendingCount).toBe(4);
    // 숫자와 모달 내용이 **같은 응답**에서 와야 둘이 어긋나지 않는다.
    // (동일 객체가 아니라 **같은 내용**이다 — `readConfigDiff` 가 읽은 값을
    //  돌려주므로 그리는 것과 읽은 것이 같은 물건이다.)
    expect(vm.pendingDiff).toStrictEqual(NODE_PENDING.diff);
    // **노드의** upstream 이름이다 — 게이트웨이 목록으로 붙이면 id 가 그대로
    // 나오거나 남의 이름이 뜬다.
    expect(vm.items[0]!.targetLabel).toBe("upstream: node-pool");
    expect(vm.items[1]!.targetLabel).toBe("proxy_pass: http://127.0.0.1:9000");
    // 미적용 표시도 **노드의** diff 에서 온다(key = 서버 id).
    expect(vm.items.map((i) => i.pending)).toEqual([true, false]);
  });

  it("TLS 칸은 노드 인증서 보조 읽기에서 온다 — 못 읽으면 추측하지 않는다", async () => {
    const args = await captureArgs();
    const servers = { servers: [{ ...NODE_SERVERS.servers[0], certificateId: "c-1" }] };
    const data = args.toViewModel({ upstreams: NODE_UPSTREAMS, servers, pending: NODE_PENDING });
    loadNodePageAllMock.mockResolvedValue({ ok: true, node: { id: "node-1" }, effectiveRole: "editor", data });
    const render = async () => {
      const rendered = await NodeReverseProxyPage({ params: Promise.resolve({ id: "node-1" }) });
      if (!isValidElement<ServerListProps>(rendered)) throw new Error("엘리먼트가 아닙니다");
      return rendered.props.data;
    };

    readCertsMock.mockResolvedValue(null);
    const unknown = await render();
    expect(unknown.certificatesUnknown).toBe(true);
    expect(unknown.items[0]!.tls.label).toBe("확인 불가");
    expect(readCertsMock).toHaveBeenCalledWith("node-1");

    readCertsMock.mockResolvedValue([]);
    const deleted = await render();
    expect(deleted.certificatesUnknown).toBe(false);
    expect(deleted.items[0]!.tls.label).toBe("인증서 삭제됨");
  });

  /**
   * **`pendingDiff` 는 이 화면이 읽지 않고 그대로 넘기는 유일한 값이다.**
   * 그냥 통과시키면 어긋난 모양이 `loadNodePageAll` 의 try/catch **밖**,
   * `ConfigDiffView` 의 렌더 도중에 던진다 — 하드 로드에서는 §9 카드가 아니라
   * 빈 화면이다(React 오류 경계는 SSR 에서 안 돈다 — `app/error.tsx:9-18`).
   *
   * `toViewModel` 이 **던지는 것**이 곧 카드다: `lib/node-page.test.tsx` 의
   * 「`toViewModel` 이 던지면 node_error 카드」가 그 뒷단을 이미 붙들고 있으므로,
   * 여기서는 **던지는지**만 보면 사슬이 이어진다.
   */
  it("모양이 어긋난 diff 를 그대로 넘기지 않는다 — 관문 안에서 던져야 카드가 된다", async () => {
    const args = await captureArgs();
    const raw = (pending: unknown) =>
      ({ upstreams: NODE_UPSTREAMS, servers: NODE_SERVERS, pending }) as Record<Key, unknown>;

    // diff 자체가 없다.
    expect(() => args.toViewModel(raw({ count: 1 }))).toThrow();
    // 세 배열 중 하나가 없다 — 렌더는 셋 다 `.map` 한다.
    expect(() => args.toViewModel(raw({ count: 1, diff: { servers: [], streams: [] } }))).toThrow();
    expect(() => args.toViewModel(raw({ count: 1, diff: { streams: [], upstreams: [] } }))).toThrow();
    // 온전한 모양은 통과한다 — 읽기가 정상 응답을 막으면 안 된다.
    expect(() => args.toViewModel(raw(NODE_PENDING))).not.toThrow();
  });

  /**
   * **컨테이너 깊이에서 멈추면 안 닫힌다** — 이 자리는 이 계획에서 세 번
   * 보고됐다. `.length` 셋을 더하는 것으로 「읽었다」고 봤는데 **문자열에도
   * `.length` 가 있어** 통과했고, 화면의 `.map` 에서 던졌다.
   *
   * `ConfigDiffView` 가 실제로 만지는 넷을 각각 붙든다: 배열 여부,
   * `MARKER[item.status]`(모르는 상태면 `undefined.char` 로 던진다),
   * React 자식이 되는 `item.label`, 그리고 `item.changes[].field`.
   */
  it("diff 원소가 이상하면 던진다 — 컨테이너만 읽으면 여기가 통과한다", async () => {
    const args = await captureArgs();
    const raw = (pending: unknown) =>
      ({ upstreams: NODE_UPSTREAMS, servers: NODE_SERVERS, pending }) as Record<Key, unknown>;
    const withDiff = (diff: unknown) => raw({ count: 1, diff });
    const item = (over: Record<string, unknown>) => ({
      servers: [{ key: "k", label: "a.example.com:443", status: "added", changes: [], ...over }],
      streams: [],
      upstreams: [],
      hasChanges: true,
    });

    // **문자열이 `.length` 를 통과하던 바로 그 모양.**
    expect(() =>
      args.toViewModel(withDiff({ servers: "3", streams: [], upstreams: [], hasChanges: true })),
    ).toThrow();
    // 모르는 상태 — 렌더가 `MARKER[status].char` 로 던진다.
    expect(() => args.toViewModel(withDiff(item({ status: "renamed" })))).toThrow();
    // 라벨이 React 자식이 된다.
    expect(() => args.toViewModel(withDiff(item({ label: { host: "a.example.com" } })))).toThrow();
    // 변경 목록도 `.map` 되고, 그 안의 `field` 도 React 자식이다.
    expect(() => args.toViewModel(withDiff(item({ changes: "없음" })))).toThrow();
    expect(() =>
      args.toViewModel(withDiff(item({ status: "changed", changes: [{ field: { name: "listen" }, before: 1, after: 2 }] }))),
    ).toThrow();
    // 온전한 모양은 통과한다.
    expect(() => args.toViewModel(withDiff(item({})))).not.toThrow();
  });

  it("목록 화면이라 「다시 시도」와 「목록으로」가 같은 곳이다 — 명시적으로 같은 값", async () => {
    const args = await captureArgs();

    expect(args.retryHref).toBe(BASE);
    expect(args.listHref).toBe(BASE);
  });
});

describe("노드 리버스 프록시 목록 — 쓰기와 적용", () => {
  it("적용은 셸에 있다 — 목록은 적용 명령을 들고 있지 않고, 화면 경로는 이 노드의 목록이다", async () => {
    loadNodePageAllMock.mockResolvedValue({
      ok: true,
      node: { id: "node-1" },
      effectiveRole: "editor",
      data: { servers: [], upstreamNameById: new Map(), pendingCount: 0, pendingDiff: null },
    });

    const rendered = await NodeReverseProxyPage({ params: Promise.resolve({ id: "node-1" }) });
    if (!isValidElement<ServerListProps>(rendered)) throw new Error("엘리먼트가 아닙니다");

    expect("applyAction" in rendered.props).toBe(false);
    expect(rendered.props.basePath).toBe(BASE);
  });

  /** 쓰기 UI 를 그릴 때 화면을 실제로 렌더한다(`loadNodePageAll` 이 성공한 상태). */
  async function renderList(effectiveRole: string): Promise<ServerListProps> {
    loadNodePageAllMock.mockResolvedValue({
      ok: true,
      node: { id: "node-1" },
      effectiveRole,
      data: { servers: [], upstreamNameById: new Map(), pendingCount: 0, pendingDiff: null },
    });

    const rendered = await NodeReverseProxyPage({ params: Promise.resolve({ id: "node-1" }) });
    if (!isValidElement<ServerListProps>(rendered)) throw new Error("엘리먼트가 아닙니다");
    return rendered.props;
  }

  /**
   * 이제 「+ 새 서버」가 갈 곳이 있다(Task 5). 목적지를 **값으로** 확인하는 것이
   * 요점이다 — 컴포넌트가 `basePath` 에서 파생하지 않고 이 페이지가 주소를 적기
   * 때문에(`ServerListWrite` 문서), 여기가 그 주소를 붙드는 유일한 자리다.
   */
  it("쓰기 등급이면 「+ 새 서버」를 노드 경로로 그린다", async () => {
    const props = await renderList("editor");

    // 행 스위치는 상세의 토글과 같은 명령을 **이 노드**로 묶는다 — 다른 노드 id 나 로컬 명령이면 남의 서버를 바꾼다.
    expect(props.write).toEqual({ newHref: `${BASE}/new`, templatesHref: null, toggleAction: { id: "node.toggleNodeServer", args: ["node-1"] } });
  });

  /**
   * **템플릿 갤러리는 게이트웨이 로컬 라우트다.** 노드 경로 아래에는 같은
   * 라우트가 없으므로 `${BASE}/templates` 는 404 이고, 로컬 주소
   * (`/reverse-proxy/templates`)를 넣으면 노드를 보다가 **게이트웨이 자신의**
   * 화면으로 새어 나간다. 둘 다 화면은 멀쩡히 뜨고 타입도 통과한다.
   */
  it("템플릿 버튼은 노드에서 사라진다 — 로컬 주소로도, 노드 주소로도 걸지 않는다", async () => {
    const props = await renderList("admin");

    expect(props.write?.templatesHref).toBeNull();
  });

  it("쓰기 등급이 없으면 write 는 null 이고, 적용 바 판정은 그와 별개다", async () => {
    // `nginx:reload` 는 되고 `config:write` 는 안 되는 등급을 만든다 — 두 판정을
    // 한 조건으로 합쳐 두면(둘 중 하나를 다른 하나로 쓰면) 이 검사가 죽는다.
    canMock.mockImplementation((_role, action) => action === "nginx:reload");

    const props = await renderList("viewer");

    expect(props.write).toBeNull();
    expect(props.data.canApply).toBe(true);
  });
});

it.each([0, 2])("remote list retains an uncertain baseline with %s changes", async (count) => {
  const args = await captureArgs();
  expect(args.toViewModel({upstreams: NODE_UPSTREAMS, servers: NODE_SERVERS, pending: {...NODE_PENDING, count, hasBaseline: false}})).toMatchObject({pendingCount: count, pendingHasBaseline: false});
});
