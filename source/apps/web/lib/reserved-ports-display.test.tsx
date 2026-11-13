import { Banner, type BannerProps } from "@repo/ui";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import type { ReservedPorts } from "@repo/presentation/nginx";

/**
 * **예약 포트가 화면까지 도달하는가** — `reserved-ports-wiring.test.ts` 의 짝이다.
 *
 * 그 파일은 **쓰기** 진입점 여섯을 붙들었다. 여기는 **표시** 자리 셋이다:
 * 목록의 경고 배너(`(app)/stream/page.tsx`)와 폼의 안내 줄(`new`, `[id]/edit`).
 * 리뷰어가 세 곳에서 prop 을 `undefined` 로 바꿨을 때 437/437 이 그대로 통과했다 —
 * 부팅 가드가 격리한 사실을 운영자에게 알리는 유일한 창구가 **초록인 채로 사라질 수
 * 있었다.** 판정이 아니라 표시라 위험도는 낮지만 모양은 F2 와 같다.
 *
 * 화면을 렌더하지 않는다. 페이지는 async 함수이고 그 반환값이 React 엘리먼트
 * 트리이므로, **트리를 걸어 prop 을 직접 본다**(`lib/node-page.test.tsx` 의 관용).
 * DOM 도 렌더러도 필요 없다.
 */

const RESERVED: ReservedPorts = {
  process: [3000, 5432],
  http: [80, 443, 8080, 8443],
  streamTcp: [15432],
  owners: { 8443: "리버스 프록시 shop.example.com" },
};

const readPageViewMock = vi.fn(async () => ({
  reservedPorts: RESERVED, streams: streamRows, stream: streamRows[0],
  canWrite: true, canApply: true,
  pending: { count: 0, hasBaseline: true, diff: { servers: [], streams: [], upstreams: [], hasChanges: false } },
}));
vi.mock("@/lib/api-page", () => ({ readPageView: readPageViewMock }));

const streamRows = [
  { id: "s1", listen: 8443, protocol: "tcp" as const, proxyPass: "10.0.0.5:8443", enabled: true },
  { id: "s2", listen: 19000, protocol: "tcp" as const, proxyPass: "10.0.0.5:19000", enabled: true },
];

vi.mock("@repo/db", () => ({
  getDb: () => ({
    query: {
      nginxStreams: {
        findMany: async () => streamRows,
        findFirst: async () => streamRows[0],
      },
    },
  }),
}));

vi.mock("@repo/auth", () => ({ can: () => true }));
vi.mock("@/lib/guard", () => ({ requireRole: async () => ({ user: { id: "u1", role: "admin" } }) }));
vi.mock("@/lib/pending", () => ({ getPendingStatus: async () => ({ count: 0, diff: { added: [], removed: [], changed: [] } }) }));
vi.mock("@/lib/audit", () => ({ getClientIp: async () => null, writeAudit: async () => undefined }));
vi.mock("@/lib/apply", () => ({ applyConfigWithSnapshot: async () => ({ ok: true, output: "" }) }));
vi.mock("@/app/(app)/stream/service", () => ({
  createStreamRow: async () => ({ ok: true, value: { id: "s1" } }),
  updateStreamRow: async () => ({ ok: true, value: { id: "s1" } }),
  deleteStreamRow: async () => ({ ok: true, value: { id: "s1" } }),
  toggleStreamRow: async () => ({ ok: true, value: { id: "s1" } }),
}));
vi.mock("next/cache", () => ({ revalidatePath: () => undefined }));
vi.mock("next/navigation", () => ({
  redirect: () => {
    throw new Error("NEXT_REDIRECT");
  },
  notFound: () => {
    throw new Error("NEXT_NOT_FOUND");
  },
}));

const { StreamList } = await import("@/components/streams/stream-list");
const { StreamForm } = await import("@/components/streams/stream-form");
const StreamPage = (await import("@/app/(app)/stream/page")).default;
const NewStreamPage = (await import("@/app/(app)/stream/new/page")).default;
const EditStreamPage = (await import("@/app/(app)/stream/[id]/edit/page")).default;

/** 엘리먼트 트리를 걸어 특정 컴포넌트의 엘리먼트를 찾는다. */
function findElement(node: ReactNode, component: unknown): ReactElement | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const hit = findElement(child as ReactNode, component);
      if (hit) return hit;
    }
    return null;
  }
  if (!isValidElement(node)) return null;
  if (node.type === component) return node;
  const { children } = node.props as { children?: ReactNode };
  return children === undefined ? null : findElement(children, component);
}

/**
 * 트리 안의 모든 문자열을 한 덩어리로 모은다(렌더 없이 문구를 확인하려고).
 * 형제는 **공백 없이** 잇는다 — React 도 그렇게 붙이므로, `{n}건의` 같은
 * 보간이 `1 건의` 로 벌어지지 않는다.
 */
function collectText(node: ReactNode): string {
  if (node === null || node === undefined || typeof node === "boolean") return "";
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map((c) => collectText(c as ReactNode)).join("");
  if (!isValidElement(node)) return "";
  if (node.type === Banner) return collectText(Banner(node.props as BannerProps));
  const { children } = node.props as { children?: ReactNode };
  return collectText(children);
}

beforeEach(() => {
  readPageViewMock.mockClear();
});

describe("표시 자리 1: /stream 목록이 예약 집합을 StreamList 로 넘긴다", () => {
  it("reservedPorts prop 이 실제로 전달된다", async () => {
    const tree = await StreamPage();
    const list = findElement(tree, StreamList);
    expect(list).not.toBeNull();
    // `undefined` 로 바뀌면 여기서 죽는다 — 그게 리뷰어가 만든 변이다.
    expect((list!.props as { reservedPorts?: ReservedPorts }).reservedPorts).toBe(RESERVED);
    expect(readPageViewMock).toHaveBeenCalledWith("/stream");
  });
});

describe("표시 자리 2·3: stream 폼이 예약 포트 숫자를 받는다", () => {
  it("새 stream 화면", async () => {
    const tree = await NewStreamPage();
    const formEl = findElement(tree, StreamForm);
    expect(formEl).not.toBeNull();
    const ports = (formEl!.props as { reservedListenPorts?: readonly number[] }).reservedListenPorts;
    expect(ports).toBeDefined();
    // DB 에서 온 8443 이 들어 있어야 한다 — 상수만 넘기는 회귀도 함께 잡는다.
    expect(ports).toEqual(expect.arrayContaining([3000, 5432, 80, 443, 8080, 8443]));
  });

  it("stream 수정 화면", async () => {
    const tree = await EditStreamPage({ params: Promise.resolve({ id: "s1" }) });
    const formEl = findElement(tree, StreamForm);
    expect(formEl).not.toBeNull();
    const ports = (formEl!.props as { reservedListenPorts?: readonly number[] }).reservedListenPorts;
    expect(ports).toBeDefined();
    expect(ports).toEqual(expect.arrayContaining([3000, 5432, 80, 443, 8080, 8443]));
  });
});

describe("StreamList 가 받은 집합을 실제로 쓴다", () => {
  const props = {
    data: {
      items: streamRows,
      canWrite: true,
      canApply: true,
      pendingCount: 0, pendingHasBaseline: true,
      pendingDiff: { added: [], removed: [], changed: [] } as never,
    },
    basePath: "/stream",
    applyAction: (async () => ({ ok: true, output: "" })) as never,
  };

  it("걸린 행이 있으면 배너와 행 경고를 그린다", async () => {
    // prop 을 넘기는 것만으로는 부족하다 — 컴포넌트가 그 값을 쓰지 않으면 배너 JSX 를
    // 지워도 위 세 테스트는 그대로 통과한다. 여기가 그 나머지 절반이다.
    const text = collectText(StreamList({ ...props, reservedPorts: RESERVED }));
    expect(text).toContain("1건의 stream");
    expect(text).toContain("리버스 프록시 shop.example.com");
    // 19000 은 stream 자기 테이블에만 있으므로 경고 대상이 아니다.
    expect(text).not.toContain("19000 은");
  });

  it("집합을 안 넘기면(노드 화면) 아무 경고도 그리지 않는다", () => {
    const text = collectText(StreamList(props));
    expect(text).not.toContain("건의 stream");
    expect(text).not.toContain("차지하고 있습니다");
  });
});
