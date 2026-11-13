import { ImportForm } from "@/components/certificates/import-form";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement } from "react";
import type { LoadNodePageAllArgs } from "@/lib/node-page";

/**
 * 노드 인증서 **교체**(수동 PEM 덮어쓰기) 화면이 붙드는 것.
 *
 * 세 가지가 이 화면 고유다:
 * 1. **읽는 것이 `source` 하나다** — 로컬 교체 화면이 DB 행을 읽는 이유와 같다
 *    (수동 인증서인가). 그런데 그 값이 **화면을 그릴지 404 를 낼지**를 고르므로
 *    관문에서 읽어야 한다 — 통과만 시키면 이상한 값이 조용히 404 로 접힌다.
 * 2. **`nginx:reload` 로 연다** — 교체는 연결된 서버가 있으면 곧바로 재적용된다.
 * 3. **현재 PEM 을 보여주지 않는다 — 로컬도 그렇다.** 계약이 `certPem`/`keyPem`
 *    을 빼지만 로컬 교체 화면도 빈 textarea 로 시작하므로, 노드 쪽에만 비어 있는
 *    자리가 생기지 않는다. 두 화면이 **보이는 것이 같다**는 것을 아래에서
 *    「폼에 초기값을 넘기지 않는다」로 고정한다.
 */
const loadNodePageAllMock = vi.fn();
vi.mock("@/lib/node-page", () => ({ loadNodePageAll: loadNodePageAllMock }));

// `@repo/ui`·CSS 모듈 사슬을 이 프로세스로 끌고 오지 않기 위한 목이다.
vi.mock("@/components/certificates/import-form", () => ({ ImportForm: () => null }));

const replaceNodeCertificateMock = vi.fn();
vi.mock("../../../actions", () => ({ replaceNodeCertificate: replaceNodeCertificateMock }));

const notFoundMock = vi.fn(() => {
  throw new Error("NEXT_NOT_FOUND_CALLED");
});
vi.mock("next/navigation", () => ({ notFound: notFoundMock }));

const { default: ReplaceNodeCertificatePage } = await import("./page");

type Args = LoadNodePageAllArgs<"certificate", unknown>;

const NODE_ID = "node-1";
const CERT_ID = "cert-9";
const BASE = `/nodes/${NODE_ID}/certificates`;

function certificateResponse(extra: Record<string, unknown> = {}) {
  return { certificate: { source: "manual", ...extra } };
}

beforeEach(() => {
  loadNodePageAllMock.mockReset();
  loadNodePageAllMock.mockResolvedValue({ ok: false, render: null });
  notFoundMock.mockClear();
  replaceNodeCertificateMock.mockReset();
});

async function captureArgs(cid = CERT_ID): Promise<Args> {
  await ReplaceNodeCertificatePage({ params: Promise.resolve({ id: NODE_ID, cid }) });
  expect(loadNodePageAllMock).toHaveBeenCalledTimes(1);
  return loadNodePageAllMock.mock.calls[0]![0] as Args;
}

/** 응답 → 뷰모델. 이 화면은 경로가 하나라 raw 도 하나다. */
function viewModel(args: Args, certificate: unknown = certificateResponse()) {
  return args.toViewModel({ certificate } as Record<"certificate", unknown>) as { source: string };
}

/** 폼 엘리먼트의 props. 공유 머리말과 구분해 실제 폼을 찾는다. */
async function formProps(certificate: unknown = certificateResponse()): Promise<Record<string, unknown>> {
  loadNodePageAllMock.mockReset();
  loadNodePageAllMock.mockImplementation((args: Args) => {
    const data = args.toViewModel({ certificate } as Record<"certificate", unknown>);
    return Promise.resolve({ ok: true, node: { id: NODE_ID }, effectiveRole: "editor", data });
  });

  const tree = await ReplaceNodeCertificatePage({ params: Promise.resolve({ id: NODE_ID, cid: CERT_ID }) });
  const found: Record<string, unknown>[] = [];
  const walk = (node: unknown): void => {
    if (Array.isArray(node)) return node.forEach(walk);
    if (!isValidElement(node)) return;
    const el = node as { type: unknown; props: Record<string, unknown> };
    if (el.type === ImportForm) found.push(el.props);
    walk(el.props.children);
  };
  walk(tree);
  if (found.length !== 1) throw new Error(`폼을 찾지 못했습니다(${found.length}개)`);
  return found[0]!;
}

describe("노드 인증서 교체 — 무엇을 어디서 읽는가", () => {
  it("인증서 한 건만 읽고, 노드 id 가 아니라 인증서 id 로 조회한다", async () => {
    const args = await captureArgs();

    expect(args.paths).toEqual({ certificate: `/api/v1/certificates/${CERT_ID}` });
    expect(args.nodeId).toBe(NODE_ID);
  });

  /**
   * **`nginx:reload` 로 연다.** 교체는 연결된 서버가 있으면 노드가 곧바로
   * 재적용하므로 노드 라우트도 그 등급을 요구한다(`[id]/replace/route.ts`).
   * 더 낮은 등급으로 열면 PEM 을 다 붙여넣고 제출한 뒤에야 거부당한다.
   */
  it("nginx:reload 로 연다 — config:read 도 config:write 도 아니다", async () => {
    expect((await captureArgs()).action).toBe("nginx:reload");
  });

  it("「다시 시도」는 이 교체 화면, 「목록으로」는 목록이다", async () => {
    const args = await captureArgs();

    expect(args.retryHref).toBe(`${BASE}/${CERT_ID}/replace`);
    expect(args.listHref).toBe(BASE);
    expect(args.retryHref).not.toBe(args.listHref);
  });

  it("경로의 인증서 id 를 이스케이프한다 — 주소창 값이 노드 API 경로를 벗어나지 못한다", async () => {
    const args = await captureArgs("../../apply");

    expect(args.paths.certificate).toBe("/api/v1/certificates/..%2F..%2Fapply");
  });

  /**
   * **그려지지 않는데도 관문에서 읽는다.** `source` 는 화면에 안 나가지만 **이
   * 화면을 그릴지 404 를 낼지**를 고른다 — 문자열이 아닌 무엇이 오면
   * `!== "manual"` 이 참이라 조용히 「그런 화면이 없다」로 접히고, 사용자는 자기
   * 인증서가 사라졌다고 읽는다. 관문에서 던지면 §9 카드가 된다.
   */
  it("source 가 문자열이 아니면 뷰모델에서 던진다 — 404 로 접히지 않는다", async () => {
    const args = await captureArgs();

    expect(() => viewModel(args, certificateResponse({ source: {} }))).toThrow();
    expect(() => viewModel(args, certificateResponse({ source: undefined }))).toThrow();
    expect(viewModel(args).source).toBe("manual");
  });
});

describe("노드 인증서 교체 — 폼에 무엇을 넘기나", () => {
  /**
   * **ACME 인증서에는 교체라는 게 없다** — 갱신·재발급이 그 자리다. 로컬 교체
   * 화면의 `cert.source !== "manual"` → `notFound()` 와 같은 판정이다.
   */
  it("ACME 인증서면 화면이 없다", async () => {
    await expect(formProps(certificateResponse({ source: "acme" }))).rejects.toThrow("NEXT_NOT_FOUND_CALLED");
    expect(notFoundMock).toHaveBeenCalledTimes(1);
  });

  it("수동 인증서는 그대로 폼을 그린다", async () => {
    await formProps();

    expect(notFoundMock).not.toHaveBeenCalled();
  });

  /** 어느 인증서를 덮는지 — 이 값이 hidden `id` 가 되어 액션의 경로를 만든다. */
  it("교체 대상 인증서 id 를 폼에 넘긴다", async () => {
    expect((await formProps()).certId).toBe(CERT_ID);
  });

  it("취소는 노드 인증서 목록으로 간다 — 로컬 목록이 아니다", async () => {
    const props = await formProps();

    expect(props.cancelHref).toBe(BASE);
    expect(props.cancelHref).not.toBe("/certificates");
  });

  /**
   * **현재 PEM 을 프리필하지 않는다 — 로컬과 같다.** 계약이 `certPem`/`keyPem`
   * 을 빼서 못 하는 것이기도 하지만, 로컬 교체 화면도 빈 칸으로 시작하므로
   * **두 화면이 보이는 것이 같다.** 누군가 「노드 쪽만 비어 있다」고 오해해
   * 계약에 PEM 을 더하려 들면 이 검사가 먼저 깨져야 한다 — 그건 개인키를
   * 게이트웨이로 끌어올리는 변경이다.
   */
  it("폼에 PEM 초기값을 넘기지 않는다", async () => {
    expect(Object.keys(await formProps()).sort()).toEqual(["action", "cancelHref", "certId", "info", "target"]);
  });

  /**
   * 액션은 `nodeId` 로 묶여야 한다 — 안 묶으면 폼의 `(prev, formData)` 가 그대로
   * 첫 두 인자로 들어가 **노드 id 자리에 이전 상태 객체**가 온다.
   */
  it("교체 액션이 이 노드로 묶여 있다", async () => {
    expect((await formProps()).action).toEqual({id:"node.replaceNodeCertificate",args:[NODE_ID]});
  });
});
