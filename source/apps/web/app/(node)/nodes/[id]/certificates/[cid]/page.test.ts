import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement } from "react";
import type { CertificateDetailProps } from "@/components/certificates/certificate-detail";
import type { CertificateDetailVM } from "@/components/certificates/certificate-vm";
import type { LoadNodePageAllArgs } from "@/lib/node-page";

/**
 * 노드 인증서 **상세**가 붙드는 것.
 *
 * 이 화면은 라우트 파라미터가 둘이다(`[id]` = 노드, `[cid]` = 인증서). 둘을
 * 헷갈리면 **조용히 엉뚱한 것을 가져온다** — 노드 id 로 인증서를 조회하면 404 가
 * 뜨고 사용자는 「인증서가 사라졌다」로 읽는다.
 *
 * 그리고 이 화면에는 **경로가 둘 있다**: 인증서 목록으로 가는 `basePath` 와
 * 연결된 서버로 가는 `serverBasePath`. 하나에서 다른 하나를 파생시키면 링크는
 * 멀쩡해 보이는데 열어 보면 게이트웨이 자신의 서버로 간다.
 */
const loadNodePageAllMock = vi.fn();
vi.mock("@/lib/node-page", () => ({
  loadNodePageAll: loadNodePageAllMock,
}));

// `@repo/ui`·CSS 모듈 사슬을 이 프로세스로 끌고 오지 않기 위한 목이다(검사 대상 아님 —
// 그쪽은 `components/certificates/certificate-detail.test.tsx` 가 본다).
vi.mock("@/components/certificates/certificate-detail", () => ({ CertificateDetail: () => null }));
// 슬롯에 들어가는 클라이언트 컴포넌트 둘도 같은 이유로 목이다. **여기서 보는
// 것은 「무엇을 넘겼나」이지 「어떻게 그리나」가 아니다** — 그 둘은 훅을 써서
// 렌더 환경 없이 부를 수 없다.
vi.mock("@/components/certificates/dns-challenge-panel", () => ({ DnsChallengePanel: () => null }));
vi.mock("@/components/certificates/certificate-detail-actions", () => ({ CertificateDetailActions: () => null }));
// 서버 액션 모듈은 `@repo/db` → postgres 사슬을 끌고 온다. 이 화면이 붙드는 것은
// **어느 액션을 어느 노드로 묶어 넘기는가**가 아니라 경로·슬롯이라 목으로 둔다
// (액션 자체는 `certificate-actions.test.ts` 가 본다).
vi.mock("../../actions", () => ({
  deleteNodeCertificate: vi.fn(),
  renewDnsNodeCertificate: vi.fn(),
  renewNodeCertificate: vi.fn(),
  retryNodeCertificate: vi.fn(),
  submitNodeDnsChallenge: vi.fn(),
}));

const { default: NodeCertificateDetailPage } = await import("./page");

type Key = "certificate" | "servers";
type Args = LoadNodePageAllArgs<Key, CertificateDetailVM>;

const NODE_ID = "node-1";
const CERT_ID = "cert-9";
const BASE = `/nodes/${NODE_ID}/certificates`;

function certificateResponse(extra: Record<string, unknown> = {}) {
  return {
    certificate: {
      id: CERT_ID,
      domains: ["a.example.com"],
      status: "issued",
      source: "acme",
      staging: false,
      challengeType: "http-01",
      dnsProvider: null,
      dnsRecords: null,
      dnsSubmittedAt: null,
      issuedAt: "2026-08-01T00:00:00.000Z",
      expiresAt: "2026-11-01T00:00:00.000Z",
      lastError: null,
      lastAttemptAt: "2026-08-01T00:00:00.000Z",
      failureCount: 0,
      issueLog: [{ at: "2026-08-01T00:00:01.000Z", level: "info", msg: "발급 시작" }],
      ...extra,
    },
  };
}

/** 노드의 서버 목록 — **둘만 이 인증서를 쓴다.** */
const NODE_SERVERS = {
  servers: [
    { id: "srv-1", serverName: "a.example.com", listen: 80, certificateId: CERT_ID },
    { id: "srv-2", serverName: "z.example.com", listen: 8080, certificateId: "other" },
    { id: "srv-3", serverName: "b.example.com", listen: 80, certificateId: CERT_ID },
  ],
};

beforeEach(() => {
  loadNodePageAllMock.mockReset();
  loadNodePageAllMock.mockResolvedValue({ ok: false, render: null });
});

/**
 * 교체 성공 뒤 붙는 커버리지 경고 쿼리. 대부분의 검사는 빈 값이면 되므로 여기
 * 기본값을 둔다 — 「무엇이 배너로 나가는가」는 전용 검사가 따로 본다.
 */
function search(uncovered?: string): Promise<{ uncovered?: string }> {
  return Promise.resolve(uncovered === undefined ? {} : { uncovered });
}

async function captureArgs(): Promise<Args> {
  await NodeCertificateDetailPage({
    params: Promise.resolve({ id: NODE_ID, cid: CERT_ID }),
    searchParams: search(),
  });
  expect(loadNodePageAllMock).toHaveBeenCalledTimes(1);
  return loadNodePageAllMock.mock.calls[0]![0] as Args;
}

function viewModel(args: Args, raw: Partial<Record<Key, unknown>> = {}): CertificateDetailVM {
  return args.toViewModel({
    certificate: certificateResponse(),
    servers: NODE_SERVERS,
    ...raw,
  } as Record<Key, unknown>);
}

describe("노드 인증서 상세 — 무엇을 어디서 읽는가", () => {
  it("인증서 한 건 + 서버 목록을 한 번에 읽는다", async () => {
    const args = await captureArgs();

    // **노드 id 가 아니라 인증서 id 로 조회한다.**
    expect(args.paths.certificate).toBe(`/api/v1/certificates/${CERT_ID}`);
    // 노드에는 「이 인증서를 쓰는 서버」 종점이 없다 — 목록을 받아 걸러 쓴다.
    expect(args.paths.servers).toBe("/api/v1/servers");
    expect(args.nodeId).toBe(NODE_ID);
    expect(args.action).toBe("config:read");
  });

  it("경로의 인증서 id 를 이스케이프한다 — 주소창 값이 노드 API 경로를 벗어나지 못한다", async () => {
    await NodeCertificateDetailPage({
      params: Promise.resolve({ id: NODE_ID, cid: "../../apply" }),
      searchParams: search(),
    });
    const args = loadNodePageAllMock.mock.calls[0]![0] as Args;

    expect(args.paths.certificate).toBe("/api/v1/certificates/..%2F..%2Fapply");
  });

  it("경로마다 서로 다른 이름을 준다", async () => {
    const args = await captureArgs();

    expect(Object.keys(args.subjects).sort()).toEqual(["certificate", "servers"]);
    expect(new Set(Object.values(args.subjects)).size).toBe(2);
  });

  it("상세라서 「다시 시도」와 「목록으로」가 다르다 — 같으면 막다른 길이다", async () => {
    const args = await captureArgs();

    expect(args.retryHref).toBe(`${BASE}/${CERT_ID}`);
    expect(args.listHref).toBe(BASE);
    expect(args.retryHref).not.toBe(args.listHref);
  });
});

describe("노드 인증서 상세 — 응답 → 뷰모델", () => {
  it("연결된 서버만 남기고 순서를 그대로 둔다", async () => {
    const vm = viewModel(await captureArgs());

    expect(vm.linkedServers).toEqual([
      { id: "srv-1", serverName: "a.example.com", listen: 80 },
      { id: "srv-3", serverName: "b.example.com", listen: 80 },
    ]);
  });

  it("ISO 시각 셋을 Date 로 되돌린다 — 화면이 toISOString 을 부른다", async () => {
    const vm = viewModel(await captureArgs());

    expect(vm.issuedAt?.toISOString()).toBe("2026-08-01T00:00:00.000Z");
    expect(vm.expiresAt?.toISOString()).toBe("2026-11-01T00:00:00.000Z");
    expect(vm.lastAttemptAt?.toISOString()).toBe("2026-08-01T00:00:00.000Z");
  });

  it("발급 로그 시각을 화면 표기로 바꿔 싣는다", async () => {
    const vm = viewModel(await captureArgs());

    expect(vm.issueLog).toEqual([{ at: "2026-08-01 00:00:01", level: "info", msg: "발급 시작" }]);
  });

  it("진행 중인 DNS 주문이 없으면 dnsOrder 는 null 이다", async () => {
    const vm = viewModel(await captureArgs());

    expect(vm.dnsOrder).toBeNull();
  });

  /**
   * **단계는 `dnsSubmittedAt` 이 가른다**(1.16.0 이 더한 응답 필드). `status` 로
   * 짐작하면 **갱신에서 틀린다** — 갱신 주문은 TXT 대기와 검증 중 **양쪽 다**
   * `issued` 를 유지한다. 그래서 이 검사는 `status` 를 `issued` 로 고정한 채
   * 두 단계가 갈리는지를 본다: `status` 를 다시 보게 되면 여기서 죽는다.
   *
   * 틀렸을 때의 비용이 양쪽 다 조용하다 — `awaiting-txt` 로 잘못 그리면 이미
   * 검증 중인 주문에 「TXT 를 넣으세요」라고 말하고(노드가 「이미 검증이 진행
   * 중입니다」로 거절한다), `verifying` 으로 잘못 그리면 **사용자가 넣어야 할
   * TXT 를 감춘다.**
   */
  it("DNS 주문 단계를 dnsSubmittedAt 으로 가른다 — 갱신(status=issued)에서도 갈린다", async () => {
    const args = await captureArgs();
    const records = [{ domain: "a.example.com", recordName: "_acme-challenge.a.example.com", recordValue: "v" }];
    const phaseOf = (status: string, dnsSubmittedAt: string | null) =>
      viewModel(args, { certificate: certificateResponse({ status, dnsRecords: records, dnsSubmittedAt }) }).dnsOrder
        ?.phase;

    // 갱신 — 두 단계가 같은 `status` 를 쓴다.
    expect(phaseOf("issued", null)).toBe("awaiting-txt");
    expect(phaseOf("issued", "2026-08-29T00:00:00.000Z")).toBe("verifying");
    // 최초 발급도 같은 값으로 갈린다(상태는 거들 뿐이다).
    expect(phaseOf("awaiting-dns", null)).toBe("awaiting-txt");
    expect(phaseOf("issuing", "2026-08-29T00:00:00.000Z")).toBe("verifying");
  });

  it("모양이 어긋난 응답을 그대로 넘기지 않는다", async () => {
    const args = await captureArgs();
    const raw = (certificate: unknown) => ({ certificate, servers: NODE_SERVERS }) as Record<Key, unknown>;

    // 발급 로그가 배열이 아니다 — 패널이 `.map` 한다.
    expect(() => args.toViewModel(raw(certificateResponse({ issueLog: "없음" })))).toThrow();
    // TXT 안내가 배열이 아니다 — 패널이 `.map` 한다(Task 7 이 그린다).
    expect(() => args.toViewModel(raw(certificateResponse({ dnsRecords: {} })))).toThrow();
    // 도메인이 배열이 아니다 — 제목과 「도메인」 행이 파고든다.
    expect(() => args.toViewModel(raw(certificateResponse({ domains: "a.example.com" })))).toThrow();
    // 만료일이 날짜가 아니다.
    expect(() => args.toViewModel(raw(certificateResponse({ expiresAt: "언젠가" })))).toThrow();
    // 온전한 모양은 통과한다.
    expect(() => viewModel(args)).not.toThrow();
  });

  /**
   * **원소 깊이의 자리들.** 넷 다 화면이 **그대로 React 자식으로** 그린다 —
   * 상태 문구(`statusLabel` 의 「모르는 값은 그대로」 갈래), 실패 횟수, 오류
   * 배너의 `<pre>`, 연결 서버 표의 두 셀. 옮겨 담기만 하면 여기가 통과하고
   * 렌더에서 터진다.
   */
  it("원소가 이상하면 던진다 — 컨테이너만 읽으면 여기가 통과한다", async () => {
    const args = await captureArgs();
    const raw = (certificate: unknown, servers: unknown = NODE_SERVERS) =>
      ({ certificate, servers }) as Record<Key, unknown>;

    expect(() => args.toViewModel(raw(certificateResponse({ status: { v: "issued" } })))).toThrow();
    // **`dnsSubmittedAt` 은 그려지지 않는데도 읽는다** — 이 값은 화면의 어느
    // 절반을 그릴지를 고른다. 문자열이 아닌 것은 truthy 라 그냥 넘기면 조용히
    // 「검증 중」으로 접히고, 사용자가 넣어야 할 TXT 가 화면에서 사라진다.
    expect(() =>
      args.toViewModel(raw(certificateResponse({ dnsRecords: [], dnsSubmittedAt: { at: 1 } }))),
    ).toThrow();
    expect(() => args.toViewModel(raw(certificateResponse({ failureCount: "셋" })))).toThrow();
    expect(() => args.toViewModel(raw(certificateResponse({ lastError: { msg: "터짐" } })))).toThrow();
    expect(() => args.toViewModel(raw(certificateResponse({ issueLog: [{ at: "2026-08-29T00:00:00.000Z", level: "info", msg: {} }] })))).toThrow();
    // 연결된 서버 표의 두 셀.
    expect(() =>
      args.toViewModel(
        raw(certificateResponse(), { servers: [{ id: "srv-1", serverName: { v: 1 }, listen: 80, certificateId: CERT_ID }] }),
      ),
    ).toThrow();
    expect(() =>
      args.toViewModel(
        raw(certificateResponse(), { servers: [{ id: "srv-1", serverName: "a.example.com", listen: "80", certificateId: CERT_ID }] }),
      ),
    ).toThrow();
  });

  /**
   * **읽지 않는 것도 계약이다.** 검증 중에는 TXT 패널이 안 뜨므로 안내를 읽지
   * 않는다 — 읽으면 아무도 안 그릴 응답 조각 때문에 멀쩡한 화면이 카드가 된다.
   */
  it("검증 중에는 어긋난 TXT 안내로도 던지지 않는다", async () => {
    const args = await captureArgs();
    const raw = (certificate: unknown) => ({ certificate, servers: NODE_SERVERS }) as Record<Key, unknown>;

    expect(() =>
      args.toViewModel(
        raw(certificateResponse({ dnsRecords: "망가짐", dnsSubmittedAt: "2026-08-29T00:00:00.000Z" })),
      ),
    ).not.toThrow();
    expect(() => args.toViewModel(raw(certificateResponse({ dnsRecords: "망가짐", dnsSubmittedAt: null })))).toThrow();
  });
});

/** 화면이 실제로 받는 뷰모델. 슬롯 검사가 이 위에서 갈래를 바꿔 본다. */
function detailVM(over: Partial<CertificateDetailVM> = {}): CertificateDetailVM {
  return {
    id: CERT_ID,
    domains: { primary: "a.example.com", all: "a.example.com", extra: 0 },
    alias: null,
    status: "issued",
    source: "acme",
    staging: false,
    challengeType: "http-01",
    dnsProvider: null,
    issuedAt: null,
    expiresAt: null,
    lastAttemptAt: null,
    failureCount: 0,
    lastError: null,
    issueLog: [],
    dnsOrder: null,
    linkedServers: [],
    ...over,
  };
}

describe("노드 인증서 상세 — 링크와 쓰기 슬롯", () => {
  async function renderDetail(
    over: Partial<CertificateDetailVM> = {},
    effectiveRole = "admin",
    uncovered?: string,
  ): Promise<CertificateDetailProps> {
    loadNodePageAllMock.mockResolvedValue({
      ok: true,
      node: { id: NODE_ID },
      effectiveRole,
      data: detailVM(over),
    });

    const rendered = await NodeCertificateDetailPage({
      params: Promise.resolve({ id: NODE_ID, cid: CERT_ID }),
      searchParams: search(uncovered),
    });
    if (!isValidElement<CertificateDetailProps>(rendered)) throw new Error("엘리먼트가 아닙니다");
    return rendered.props;
  }

  /** 슬롯에 들어간 엘리먼트의 props — 클라이언트 컴포넌트라 렌더하지 않고 본다. */
  function slotProps(node: unknown): Record<string, unknown> {
    if (!isValidElement(node)) throw new Error("슬롯이 비어 있습니다");
    return node.props as Record<string, unknown>;
  }

  const RECORDS = [{ domain: "a.example.com", recordName: "_acme-challenge.a.example.com", recordValue: "v" }];

  /**
   * **연결된 서버 행은 리버스 프록시 상세로 가야 한다.** 인증서 경로에서
   * 파생시키면(`${basePath}/${serverId}`) 존재하지 않는 인증서를 찾게 되고,
   * 로컬 `/reverse-proxy` 를 그대로 쓰면 노드를 보다가 **게이트웨이 자신의**
   * 서버로 새어 나간다. 둘 다 화면은 멀쩡히 뜨고 타입도 통과한다.
   */
  it("인증서 경로와 서버 경로가 각각 노드의 것이다", async () => {
    const props = await renderDetail();

    expect(props.basePath).toBe(BASE);
    expect(props.serverBasePath).toBe(`/nodes/${NODE_ID}/reverse-proxy`);
  });

  /**
   * **이 태스크 전체가 이 한 갈래에 걸려 있다.** `awaiting-txt` 면 TXT 표를
   * 그려야 하고, `verifying` 이면 그리면 안 된다 — 반대로 하면 이미 검증 중인
   * 주문에 「TXT 를 넣으세요」라고 말하거나(노드가 「이미 검증이 진행 중입니다」로
   * 거절한다), **사용자가 넣어야 할 TXT 를 감춘다.** 둘 다 화면은 멀쩡히 뜬다.
   */
  it("TXT 대기에서만 DNS 패널을 그린다", async () => {
    const awaiting = await renderDetail({ dnsOrder: { phase: "awaiting-txt", records: RECORDS } });
    const verifying = await renderDetail({ dnsOrder: { phase: "verifying" } });

    expect(slotProps(awaiting.dnsPanel).records).toEqual(RECORDS);
    expect(verifying.dnsPanel).toBeNull();
  });

  /**
   * 최초 발급(`awaiting-dns`)과 **갱신**은 안내 문구가 다르다 — 갱신은 서빙 중인
   * 인증서가 그대로 살아 있다는 말을 덧붙인다.
   */
  it("최초 발급과 갱신을 mode 로 가른다", async () => {
    const initial = await renderDetail({
      status: "awaiting-dns",
      dnsOrder: { phase: "awaiting-txt", records: RECORDS },
    });
    const renew = await renderDetail({ status: "issued", dnsOrder: { phase: "awaiting-txt", records: RECORDS } });

    expect(slotProps(initial.dnsPanel).mode).toBe("initial");
    expect(slotProps(renew.dnsPanel).mode).toBe("renew");
  });

  /**
   * **중단은 중계 라우트다.** 로컬 경로(`/api/certificates/<id>/cancel`)를 넘기면
   * 노드 화면의 「중단」이 **게이트웨이 자신의 발급**을 끊으러 간다 — 버튼은
   * 멀쩡히 눌리고 아무 오류도 안 난다. 패널과 액션 행이 **같은 경로**를 받아야
   * 하는 것도 여기서 붙든다(한쪽만 틀리면 화면의 절반에서만 안 듣는다).
   */
  it("중단 경로가 둘 다 노드 중계 라우트다", async () => {
    const props = await renderDetail({ dnsOrder: { phase: "awaiting-txt", records: RECORDS } });
    const expected = `/api/nodes/${NODE_ID}/certificates/${CERT_ID}/cancel`;

    expect(slotProps(props.dnsPanel).cancelPath).toBe(expected);
    expect(slotProps(props.actions).cancelPath).toBe(expected);
    expect(slotProps(props.actions).cancelPath).not.toBe(`/api/certificates/${CERT_ID}/cancel`);
  });

  it("중단 경로의 id 둘을 이스케이프한다 — 주소창 값이 중계 경로를 벗어나지 못한다", async () => {
    loadNodePageAllMock.mockResolvedValue({
      ok: true,
      node: { id: NODE_ID },
      effectiveRole: "admin",
      data: detailVM(),
    });

    const rendered = await NodeCertificateDetailPage({
      params: Promise.resolve({ id: NODE_ID, cid: "../../apply" }),
      searchParams: search(),
    });
    if (!isValidElement<CertificateDetailProps>(rendered)) throw new Error("엘리먼트가 아닙니다");

    expect(slotProps(rendered.props.actions).cancelPath).toBe(
      `/api/nodes/${NODE_ID}/certificates/..%2F..%2Fapply/cancel`,
    );
  });

  /**
   * 「설정 수정」과 「인증서 교체」가 **둘 다 노드 경로**여야 한다. 교체 쪽이
   * 로컬 경로면 수동 인증서에서 게이트웨이 **자신의** 교체 화면이 열리고,
   * 거기서 붙여넣은 PEM 이 엉뚱한 서버의 인증서를 덮는다.
   */
  it("수정·교체 링크가 둘 다 노드 경로다", async () => {
    const props = await renderDetail();

    expect(slotProps(props.actions).editHref).toBe(`${BASE}/${CERT_ID}/edit`);
    expect(slotProps(props.actions).replaceHref).toBe(`${BASE}/${CERT_ID}/replace`);
  });

  /**
   * 교체 성공 뒤 붙는 커버리지 경고. **상세가 이 쿼리를 실제로 읽어야 한다** —
   * 액션이 붙여 보내도 화면이 `uncovered={[]}` 로 고정돼 있으면 배너가 영영 안
   * 뜨고, 사용자는 새 인증서가 커버하지 않는 이름이 있다는 것을 모른 채
   * 브라우저 경고를 만난다. 그 값이 곧 배너의 본문이라 **쪼갠 결과**까지 본다.
   */
  it("?uncovered= 를 읽어 커버리지 경고로 넘긴다", async () => {
    const props = await renderDetail({}, "admin", "a.example.com,b.example.com");

    expect(props.uncovered).toEqual(["a.example.com", "b.example.com"]);
  });

  /** 쿼리가 없으면 배너도 없다 — 빈 문자열이 원소로 새면 「, 」만 그려진다. */
  it("uncovered 쿼리가 없거나 비면 빈 배열이다", async () => {
    expect((await renderDetail()).uncovered).toEqual([]);
    expect((await renderDetail({}, "admin", "")).uncovered).toEqual([]);
  });

  /**
   * **판정은 `effectiveRole`(= min(사용자, 노드 토큰))이다.** 그리고 **패널은
   * 그대로 뜬다** — TXT 값은 `config:read` 가 봐도 되는 공개값이고, 무엇이 대기
   * 중인지 아는 것이 유용하다(패널이 `canWrite` 로 버튼만 감춘다).
   */
  it("쓰기 등급이 없으면 액션 행이 없고, 패널은 읽기 전용으로 남는다", async () => {
    const props = await renderDetail({ dnsOrder: { phase: "awaiting-txt", records: RECORDS } }, "viewer");

    expect(props.actions).toBeNull();
    expect(slotProps(props.dnsPanel).canWrite).toBe(false);
  });

  /** 교체 화면이 로컬 전용이라 이 쿼리를 붙여 보내는 곳이 노드 경로에 없다. */
  it("커버리지 경고 배너는 노드에서 나오지 않는다", async () => {
    const props = await renderDetail();

    expect(props.uncovered).toEqual([]);
  });
});
