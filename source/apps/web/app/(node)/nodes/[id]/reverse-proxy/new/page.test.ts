import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement } from "react";
import type { LoadNodePageAllArgs } from "@/lib/node-page";

/**
 * 노드 리버스 프록시 **생성** 폼이 붙드는 것.
 *
 * 이 화면의 핵심은 **후보 목록 둘이 노드 것인가**다. 게이트웨이 자기 목록을
 * 실으면 화면은 멀쩡히 뜨고 저장도 되는데, 노드에서 그 upstream 참조가 깨진다
 * (또는 우연히 같은 id 가 있으면 엉뚱한 upstream 으로 이어진다). 값만 남의
 * 것이라 타입도 렌더도 아무것도 못 잡는 계열이다.
 *
 * 목은 `loadNodePageAll` 과 액션 모듈이다. `ServerForm` 목은 검사 대상이 아니라
 * `@repo/ui`·CSS 모듈 사슬을 이 프로세스로 끌고 오지 않기 위한 것이다.
 * **`toCertificateOptions` 는 목으로 두지 않는다** — 발급 전 인증서가 후보에서
 * 빠지는지가 검사 대상이다.
 */
const loadNodePageAllMock = vi.fn();
vi.mock("@/lib/node-page", () => ({
  loadNodePageAll: loadNodePageAllMock,
}));

vi.mock("@/components/reverse-proxy/server-form", () => ({ ServerForm: () => null }));

const createNodeServerMock = vi.fn();
vi.mock("../../actions", () => ({
  createNodeServer: (...args: unknown[]) => createNodeServerMock(...args),
}));

const { default: NewNodeServerPage } = await import("./page");

type Key = "upstreams" | "certificates";
type FormData_ = { upstreams: { id: string; name: string }[]; certificates: { id: string; label: string }[] };
type Args = LoadNodePageAllArgs<Key, FormData_>;

const NODE_ID = "node-1";
const BASE = `/nodes/${NODE_ID}/reverse-proxy`;

/** 노드가 `GET /api/v1/upstreams` 로 주는 것 — 게이트웨이 것과 **다른 이름**을 쓴다. */
const NODE_UPSTREAMS = {
  upstreams: [
    { id: "up-1", name: "node-pool", servers: ["127.0.0.1:3000"] },
    { id: "up-2", name: "node-api", servers: [] },
  ],
};

/** `certificateView` 가 내려주는 모양 — **`certPem`/`keyPem` 이 없다**(허용목록). */
const NODE_CERTIFICATES = {
  certificates: [
    { id: "cert-1", domains: ["a.example.com"], staging: false, status: "issued" },
    { id: "cert-2", domains: ["b.example.com"], staging: true, status: "pending" },
  ],
};

beforeEach(() => {
  loadNodePageAllMock.mockReset();
  createNodeServerMock.mockReset();
  // 화면이 조기 반환하도록 실패를 돌려준다 — 우리가 보려는 것은 인자다.
  loadNodePageAllMock.mockResolvedValue({ ok: false, render: null });
});

async function captureArgs(): Promise<Args> {
  await NewNodeServerPage({ params: Promise.resolve({ id: NODE_ID }) });
  expect(loadNodePageAllMock).toHaveBeenCalledTimes(1);
  return loadNodePageAllMock.mock.calls[0]![0] as Args;
}

describe("노드 서버 생성 폼 — 무엇을 어디서 읽는가", () => {
  it("upstream 과 인증서를 노드에서 한 번에 읽는다", async () => {
    const args = await captureArgs();

    expect(args.paths).toEqual({ upstreams: "/api/v1/upstreams", certificates: "/api/v1/certificates" });
    expect(args.nodeId).toBe(NODE_ID);
    // 실패가 하나로 모이도록 **한 번**만 부른다(`loadNodePageAll` 문서).
    expect(loadNodePageAllMock).toHaveBeenCalledTimes(1);
  });

  /**
   * **읽기지만 `config:write` 로 연다.** `config:read` 로 열면 viewer 에게 폼이
   * 그려지고, 저장을 눌러야 비로소 거절당한다 — 로컬 `new/page.tsx` 의
   * `requireRole("config:write")` 가 막는 것이 바로 그것이다.
   */
  it("쓰기 등급으로 연다 — 읽기 등급으로 열면 viewer 에게 헛폼이 그려진다", async () => {
    expect((await captureArgs()).action).toBe("config:write");
  });

  it("경로마다 서로 다른 이름을 준다 — 실패 카드가 무엇을 못 읽었는지 말하도록", async () => {
    const args = await captureArgs();

    expect(Object.keys(args.subjects).sort()).toEqual(["certificates", "upstreams"]);
    expect(new Set(Object.values(args.subjects)).size).toBe(2);
  });

  /**
   * 생성 화면이라 「다시 시도」는 이 폼이고 「목록으로」는 목록이다. 되돌려
   * 보내면(둘을 같은 값으로 두면) 사용자는 목록으로만 나갈 수 있거나, 반대로
   * 「목록으로」가 다시 빈 폼을 연다.
   */
  it("「다시 시도」는 이 폼, 「목록으로」는 목록이다", async () => {
    const args = await captureArgs();

    expect(args.retryHref).toBe(`${BASE}/new`);
    expect(args.listHref).toBe(BASE);
    expect(args.retryHref).not.toBe(args.listHref);
  });
});

describe("노드 서버 생성 폼 — 응답 → 후보 목록", () => {
  function viewModel(args: Args): FormData_ {
    return args.toViewModel({ upstreams: NODE_UPSTREAMS, certificates: NODE_CERTIFICATES } as Record<Key, unknown>);
  }

  it("upstream 후보를 **노드의** 목록에서 만든다", async () => {
    expect(viewModel(await captureArgs()).upstreams).toEqual([
      { id: "up-1", name: "node-pool" },
      { id: "up-2", name: "node-api" },
    ]);
  });

  /**
   * 로컬은 SQL 에서 `status = "issued"` 를 걸지만 노드는 목록 전체가 온다.
   * 안 거르면 발급 전 인증서가 후보로 뜨고, 고르면 저장은 되는데 노드의
   * `applyAll` 이 TLS 블록을 안 써서 **HTTPS 가 안 되는데 「연결됨」**이 된다.
   */
  it("발급 완료가 아닌 인증서는 후보에 없다", async () => {
    expect(viewModel(await captureArgs()).certificates).toEqual([
      { id: "cert-1", label: "a.example.com", staging: false, domains: ["a.example.com"] },
    ]);
  });
});

describe("노드 서버 생성 폼 — 액션 배선", () => {
  it("생성 액션을 이 노드 id 로 묶고, 취소는 목록으로 간다", async () => {
    loadNodePageAllMock.mockResolvedValue({
      ok: true,
      node: { id: NODE_ID },
      effectiveRole: "editor",
      data: { upstreams: [{ id: "up-1", name: "node-pool" }], certificates: [] },
    });

    const rendered = await NewNodeServerPage({ params: Promise.resolve({ id: NODE_ID }) });
    if (!isValidElement<{ children?: unknown }>(rendered)) throw new Error("엘리먼트가 아닙니다");

    // `<section>` 안의 마지막 자식이 폼이다 — 제목·설명 문단 다음.
    const children = (rendered.props as { children: unknown[] }).children;
    const formEl = children.at(-1);
    if (!isValidElement<Record<string, unknown>>(formEl)) throw new Error("폼 엘리먼트가 아닙니다");

    // `.bind` 된 인자는 밖에서 들여다볼 수 없다 — **불러서** 확인한다.
    expect(formEl.props.action).toEqual({id:"node.createNodeServer",args:[NODE_ID]});

    expect(formEl.props.cancelHref).toBe(BASE);
    // 후보 목록은 노드에서 읽은 그것이 그대로 내려간다.
    expect(formEl.props.upstreams).toEqual([{ id: "up-1", name: "node-pool" }]);
    // 생성이라 초기값이 없다 — 있으면 폼이 수정 모드처럼 hidden id 를 싣는다.
    expect(formEl.props.defaultValues).toBeUndefined();
  });
});
