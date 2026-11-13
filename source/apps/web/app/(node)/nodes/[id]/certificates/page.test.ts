import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement } from "react";
import type { CertificateListProps } from "@/components/certificates/certificate-list";
import type { CertificateListVM } from "@/components/certificates/certificate-vm";
import type { LoadNodePageAllArgs } from "@/lib/node-page";

/**
 * 노드 인증서 **목록**이 붙드는 것.
 *
 * 1. **「연결 서버」 수가 노드 값이어야 한다.** 게이트웨이 자기 서버로 세면
 *    화면은 멀쩡히 뜨고 타입도 통과하는데, 노드에 하나도 없는 인증서 옆에
 *    「연결 서버 3」이 붙는다 — 그 숫자를 보고 삭제를 포기하게 된다.
 * 2. **응답이 원시 행이 아니라 허용목록이다**(`api/v1/certificates/view.ts`).
 *    날짜가 `Date` 가 아니라 ISO 문자열이라, 되돌리지 않으면 만료 열이
 *    렌더 도중에 던진다.
 * 3. **쓰기 경로가 아직 없다.** `/nodes/<id>/certificates/new` 는 Task 7 이
 *    만든다 — 지금 버튼을 그리면 404 로 간다.
 *
 * `CertificateList` 를 목으로 두는 것은 `@repo/ui`·CSS 사슬을 이 프로세스로
 * 끌고 오지 않기 위해서다(검사 대상이 아니다 — 그쪽은
 * `components/certificates/certificate-list.test.tsx` 가 본다).
 */
const loadNodePageAllMock = vi.fn();
vi.mock("@/lib/node-page", () => ({
  loadNodePageAll: loadNodePageAllMock,
}));

vi.mock("@/components/certificates/certificate-list", () => ({ CertificateList: () => null }));

const { default: NodeCertificatesPage } = await import("./page");

type Key = "certificates" | "servers";
type Args = LoadNodePageAllArgs<Key, CertificateListVM>;

const NODE_ID = "node-1";
const BASE = `/nodes/${NODE_ID}/certificates`;

/** `certificateView` 가 내려주는 모양 — **날짜가 ISO 문자열**이고 PEM 이 없다. */
const NODE_CERTIFICATES = {
  certificates: [
    {
      id: "cert-1",
      domains: ["a.example.com", "www.a.example.com"],
      status: "issued",
      source: "acme",
      staging: false,
      expiresAt: "2026-11-01T00:00:00.000Z",
    },
    {
      id: "cert-2",
      domains: ["b.example.com"],
      status: "issuing",
      source: "acme",
      staging: true,
      expiresAt: null,
    },
  ],
};

/** 노드의 서버 목록 — **cert-1 을 둘이 쓰고 cert-2 는 아무도 안 쓴다.** */
const NODE_SERVERS = {
  servers: [
    { certificateId: "cert-1" },
    { certificateId: "cert-1" },
    { certificateId: null },
    { certificateId: "gone" },
  ],
};

beforeEach(() => {
  loadNodePageAllMock.mockReset();
  loadNodePageAllMock.mockResolvedValue({ ok: false, render: null });
});

async function captureArgs(): Promise<Args> {
  await NodeCertificatesPage({ params: Promise.resolve({ id: NODE_ID }) });
  expect(loadNodePageAllMock).toHaveBeenCalledTimes(1);
  return loadNodePageAllMock.mock.calls[0]![0] as Args;
}

function viewModel(args: Args, raw: Partial<Record<Key, unknown>> = {}): CertificateListVM {
  return args.toViewModel({
    certificates: NODE_CERTIFICATES,
    servers: NODE_SERVERS,
    ...raw,
  } as Record<Key, unknown>);
}

describe("노드 인증서 목록 — 무엇을 어디서 읽는가", () => {
  it("인증서 목록과 서버 목록을 한 번에 읽는다", async () => {
    const args = await captureArgs();

    expect(args.paths.certificates).toBe("/api/v1/certificates");
    // **서버 목록이 없으면 「연결 서버」 열을 채울 수 없다.** 빼고 게이트웨이
    // 자기 DB 를 읽으면 남의 서버 수가 노드 인증서 옆에 붙는다.
    expect(args.paths.servers).toBe("/api/v1/servers");
    expect(args.nodeId).toBe(NODE_ID);
    expect(args.action).toBe("config:read");
    expect(loadNodePageAllMock).toHaveBeenCalledTimes(1);
  });

  it("경로마다 서로 다른 이름을 준다 — 카드 하나가 조회 둘을 대표한다", async () => {
    const args = await captureArgs();

    expect(Object.keys(args.subjects).sort()).toEqual(["certificates", "servers"]);
    expect(new Set(Object.values(args.subjects)).size).toBe(2);
  });

  it("목록 화면이라 「다시 시도」와 「목록으로」가 같은 곳이다 — 명시적으로 같은 값", async () => {
    const args = await captureArgs();

    expect(args.retryHref).toBe(BASE);
    expect(args.listHref).toBe(BASE);
  });
});

describe("노드 인증서 목록 — 응답 → 뷰모델", () => {
  it("연결 서버 수를 **노드의** 서버 목록에서 센다", async () => {
    const vm = viewModel(await captureArgs());

    expect(vm.items.map((c) => [c.id, c.linkedCount])).toEqual([
      ["cert-1", 2],
      ["cert-2", 0],
    ]);
  });

  it("도메인 요약과 만료일을 화면이 쓰는 모양으로 만든다", async () => {
    const vm = viewModel(await captureArgs());

    expect(vm.items[0]!.domains).toEqual({
      primary: "a.example.com",
      all: "a.example.com, www.a.example.com",
      extra: 1,
    });
    // ISO 문자열이 `Date` 로 되돌아온다 — 목록이 `getTime()` 을 부른다.
    expect(vm.items[0]!.expiresAt?.toISOString()).toBe("2026-11-01T00:00:00.000Z");
    expect(vm.items[1]!.expiresAt).toBeNull();
  });

  it("상태·출처·staging 을 그대로 싣는다 — 배지 셋이 여기서 갈린다", async () => {
    const vm = viewModel(await captureArgs());

    expect(vm.items[1]!.status).toBe("issuing");
    expect(vm.items[1]!.source).toBe("acme");
    expect(vm.items[1]!.staging).toBe(true);
  });

  /**
   * **관문 안에서 던져야 카드가 된다.** 통과시키면 그 실패가 렌더 도중에
   * 터지고, 그러면 §9 카드가 아니라 노드 레이아웃 경계의 **일반 500 화면**이
   * 뜬다(`LoadNodePageAllArgs.toViewModel` 문서의 실측). 여기서는 **던지는지**만 보면
   * 된다: 뒷단(던짐 → 카드)은 `lib/node-page.test.tsx` 가 붙들고 있다.
   */
  it("모양이 어긋난 응답을 그대로 넘기지 않는다", async () => {
    const args = await captureArgs();
    const raw = (certificates: unknown) => ({ certificates, servers: NODE_SERVERS }) as Record<Key, unknown>;

    // domains 가 배열이 아니다 — 목록이 `[0]`/`.length` 로 파고든다.
    expect(() =>
      args.toViewModel(raw({ certificates: [{ ...NODE_CERTIFICATES.certificates[0], domains: "a.example.com" }] })),
    ).toThrow();
    // expiresAt 이 날짜가 아니다 — 목록이 `getTime()` 을 부른다.
    expect(() =>
      args.toViewModel(raw({ certificates: [{ ...NODE_CERTIFICATES.certificates[0], expiresAt: "언젠가" }] })),
    ).toThrow();
    // 목록 자체가 없다.
    expect(() => args.toViewModel(raw({}))).toThrow();
    // 온전한 모양은 통과한다 — 읽기가 정상 응답을 막으면 안 된다.
    expect(() => viewModel(args)).not.toThrow();
  });

  /**
   * **원소 깊이의 자리들.** 컨테이너만 읽으면 여기서 다 통과하고, 화면이
   * 그 값을 React 자식으로 넘기는 순간 렌더에서 터진다 — 관문 밖이다.
   * `status` 는 `statusLabel` 의 「모르는 값은 그대로」 갈래로 새고,
   * `domains` 원소는 `join` 이 `"[object Object]"` 로 삼켜 버린다.
   */
  it("원소가 이상하면 던진다 — 컨테이너만 읽으면 여기가 통과한다", async () => {
    const args = await captureArgs();
    const one = (over: Record<string, unknown>) =>
      ({
        certificates: { certificates: [{ ...NODE_CERTIFICATES.certificates[0], ...over }] },
        servers: NODE_SERVERS,
      }) as Record<Key, unknown>;

    expect(() => args.toViewModel(one({ status: { v: "issued" } }))).toThrow();
    expect(() => args.toViewModel(one({ domains: [{ host: "a.example.com" }] }))).toThrow();
  });
});

describe("노드 인증서 목록 — 쓰기 UI", () => {
  async function renderList(effectiveRole: string) {
    loadNodePageAllMock.mockResolvedValue({
      ok: true,
      node: { id: NODE_ID },
      effectiveRole,
      data: { items: [] },
    });

    const rendered = await NodeCertificatesPage({ params: Promise.resolve({ id: NODE_ID }) });
    if (!isValidElement<CertificateListProps>(rendered)) throw new Error("엘리먼트가 아닙니다");
    return rendered.props;
  }

  /**
   * **두 버튼 다 노드 경로여야 한다.** 로컬 경로(`/certificates/import`)를
   * 넘기면 「이 노드에 가져오기」를 누른 사람이 **게이트웨이 자신에게** PEM 을
   * 등록하게 된다 — 화면은 멀쩡히 뜨고, 등록도 성공하고, 잘못된 서버에 남는다.
   */
  it("발급·가져오기 버튼이 둘 다 노드 경로로 간다", async () => {
    const props = await renderList("admin");

    expect(props.write).toEqual({ newHref: `${BASE}/new`, importHref: `${BASE}/import` });
    // 행 링크가 노드 경로로 간다 — 로컬 `/certificates/<id>` 로 새면 게이트웨이
    // 자신의 인증서를 보게 된다.
    expect(props.basePath).toBe(BASE);
  });

  /**
   * **판정은 `effectiveRole`(= min(사용자, 노드 토큰))이다.** 세션 등급으로 다시
   * 만들면 노드 토큰이 viewer 인데 버튼이 그려지고, 눌러야만 거부당한다.
   */
  it("쓰기 등급이 없으면 쓰기 UI 가 통째로 없다", async () => {
    expect((await renderList("viewer")).write).toBeNull();
  });
});
