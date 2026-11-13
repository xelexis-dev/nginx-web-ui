import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement } from "react";
import type { LoadNodePageAllArgs } from "@/lib/node-page";

// The existing node response is validated locally; apply now belongs to the node shell.
const loadNodePageAllMock = vi.fn();
vi.mock("@/lib/node-page", () => ({
  loadNodePageAll: loadNodePageAllMock,
}));

vi.mock("@/components/streams/stream-list", () => ({ StreamList: () => null }));
vi.mock("@/lib/permissions", () => ({ can: () => true }));

const { default: NodeStreamsPage } = await import("./page");

type Args = LoadNodePageAllArgs<"streams" | "pending", unknown>;

/** 노드가 `GET /api/v1/pending` 으로 주는 것(`lib/pending.ts` 의 `PendingStatus`). */
const NODE_PENDING = {
  count: 3,
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

const NODE_STREAMS = {
  streams: [{ id: "st-1", listen: 5432, protocol: "tcp", proxyPass: "127.0.0.1:5432", enabled: true }],
};

beforeEach(() => {
  loadNodePageAllMock.mockReset();
  // 화면이 조기 반환하도록 실패를 돌려준다 — 우리가 보려는 것은 인자다.
  loadNodePageAllMock.mockResolvedValue({ ok: false, render: null });
});

async function captureArgs(): Promise<Args> {
  await NodeStreamsPage({ params: Promise.resolve({ id: "node-1" }) });
  expect(loadNodePageAllMock).toHaveBeenCalledTimes(1);
  return loadNodePageAllMock.mock.calls[0]![0] as Args;
}

describe("노드 stream 목록 — 응답 검증과 헤더 적용 분리", () => {
  it("노드의 /api/v1/pending 을 함께 부른다", async () => {
    const args = await captureArgs();

    // 목록만 부르고 배지를 게이트웨이에서 채우면 이 단언이 죽는다.
    expect(args.paths).toEqual({ streams: "/api/v1/streams", pending: "/api/v1/pending" });
    // 실패가 하나로 모이도록 **한 번**만 부른다(`loadNodePageAll` 문서: 여러 번
    // 부르면 카드가 여럿 생겨 어느 호출이 실패했는지 화면이 말하지 못한다).
    expect(loadNodePageAllMock).toHaveBeenCalledTimes(1);
  });

  it("기존 pending 메타데이터를 노드 응답에서 검증한다", async () => {
    const args = await captureArgs();

    const vm = args.toViewModel({ streams: NODE_STREAMS, pending: NODE_PENDING }) as {
      pendingCount: number;
      pendingDiff: unknown;
      items: unknown[];
    };

    expect(vm.pendingCount).toBe(3);
    // 숫자와 모달 내용이 **같은 응답**에서 와야 둘이 어긋나지 않는다.
    // (동일 객체가 아니라 **같은 내용**이다 — `readConfigDiff` 가 화면 깊이로
    //  읽으면서 읽은 값을 돌려주므로, 그리는 것과 읽은 것이 같은 물건이다.)
    expect(vm.pendingDiff).toStrictEqual(NODE_PENDING.diff);
    expect(vm.items).toHaveLength(1);
  });

  /**
   * 리버스 프록시 목록의 같은 이름 검사와 짝이다 — `pendingDiff` 는 이 화면이
   * **읽지 않고 그대로 넘기는** 유일한 값이라, 관문 안에서 던지지 않으면 어긋난
   * 모양이 렌더 도중에 터져 §9 카드가 아니라 빈 화면이 된다
   * (`LoadNodePageAllArgs.toViewModel` 문서).
   */
  it("모양이 어긋난 diff 를 그대로 넘기지 않는다 — 관문 안에서 던져야 카드가 된다", async () => {
    const args = await captureArgs();
    const raw = (pending: unknown) => ({ streams: NODE_STREAMS, pending }) as Record<"streams" | "pending", unknown>;

    expect(() => args.toViewModel(raw({ count: 1 }))).toThrow();
    expect(() => args.toViewModel(raw({ count: 1, diff: { servers: [], streams: [] } }))).toThrow();
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
    const raw = (pending: unknown) => ({ streams: NODE_STREAMS, pending }) as Record<"streams" | "pending", unknown>;
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

    expect(args.retryHref).toBe("/nodes/node-1/stream");
    expect(args.listHref).toBe("/nodes/node-1/stream");
  });
});


it("passes only list data and node links, leaving apply to the header", async () => {
  loadNodePageAllMock.mockResolvedValue({ ok: true, effectiveRole: "editor", data: {
    items: NODE_STREAMS.streams, pendingCount: 3, pendingDiff: NODE_PENDING.diff,
  } });
  const page = await NodeStreamsPage({ params: Promise.resolve({ id: "node-1" }) });
  if (!isValidElement(page)) throw new Error("Expected StreamList");
  expect(page.props).toEqual({
    data: { items: NODE_STREAMS.streams, canWrite: true },
    basePath: "/nodes/node-1/stream",
  });
});
