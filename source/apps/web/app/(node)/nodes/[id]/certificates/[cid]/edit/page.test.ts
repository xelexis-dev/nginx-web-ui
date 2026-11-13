import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement } from "react";
import { CertificateEditForm, type CertificateEditFormProps } from "@/components/certificates/certificate-edit-form";
import type { LoadNodePageAllArgs } from "@/lib/node-page";

/**
 * 노드 인증서 **수정** 화면이 붙드는 것.
 *
 * 세 가지가 이 화면 고유다:
 * 1. **Cloudflare 토큰 원문은 오지 않는다** — 계약이 `hasDnsToken: boolean` 만
 *    준다. 폼은 「값을 안 보내면 유지」 표식으로 그 상태를 다룬다.
 * 2. **연락 이메일이 로컬과 다르다** — 노드의 `acme_accounts.email` 은 어떤
 *    `/api/v1` 응답에도 없다. 게이트웨이 사용자 것으로 채운다(의도된 차이).
 * 3. **수동 등록 인증서에는 이 화면이 없다** — 로컬과 같이 **그 노드의** 교체
 *    화면으로 보낸다.
 */
const loadNodePageAllMock = vi.fn();
vi.mock("@/lib/node-page", () => ({ loadNodePageAll: loadNodePageAllMock }));

const currentUserMock = vi.fn();
vi.mock("@/lib/api-page", () => ({ readPageApi: currentUserMock }));

// `@repo/ui` 사슬을 이 프로세스로 끌고 오지 않기 위한 목이다.
vi.mock("@/components/certificates/certificate-edit-form", () => ({ CertificateEditForm: () => null }));

const updateNodeCertificateMock = vi.fn();
vi.mock("../../../actions", () => ({ updateNodeCertificate: updateNodeCertificateMock }));

const redirectMock = vi.fn(() => {
  throw new Error("NEXT_REDIRECT_CALLED");
});
vi.mock("next/navigation", () => ({ redirect: redirectMock }));

const { default: EditNodeCertificatePage } = await import("./page");

type Args = LoadNodePageAllArgs<"certificate", unknown>;

const NODE_ID = "node-1";
const CERT_ID = "cert-9";
const BASE = `/nodes/${NODE_ID}/certificates`;

function certificateResponse(extra: Record<string, unknown> = {}) {
  return {
    certificate: {
      domains: ["a.example.com", "b.example.com"],
      status: "issued",
      source: "acme",
      staging: true,
      challengeType: "dns-01",
      dnsProvider: "cloudflare",
      hasDnsToken: true,
      acmeEmail: "node-owner@example.com",
      ...extra,
    },
  };
}

beforeEach(() => {
  loadNodePageAllMock.mockReset();
  loadNodePageAllMock.mockResolvedValue({ ok: false, render: null });
  currentUserMock.mockReset();
  currentUserMock.mockResolvedValue({ user: { id: "user-1", email: "ops@example.com" } });
  redirectMock.mockClear();
  updateNodeCertificateMock.mockReset();
});

async function captureArgs(cid = CERT_ID): Promise<Args> {
  await EditNodeCertificatePage({ params: Promise.resolve({ id: NODE_ID, cid }) });
  expect(loadNodePageAllMock).toHaveBeenCalledTimes(1);
  return loadNodePageAllMock.mock.calls[0]![0] as Args;
}

/** 응답 → 뷰모델. 이 화면은 경로가 하나라 raw 도 하나다. */
function viewModel(args: Args, certificate: unknown = certificateResponse()) {
  return args.toViewModel({ certificate } as Record<"certificate", unknown>) as {
    defaults: Omit<CertificateEditFormProps["defaults"], "email">;
    isIssued: boolean;
    hasStoredToken: boolean;
    source: string;
  };
}

/** 폼 엘리먼트의 props. 공유 머리말과 구분해 실제 폼을 찾는다. */
async function formProps(certificate: unknown = certificateResponse()): Promise<Record<string, unknown>> {
  loadNodePageAllMock.mockReset();
  loadNodePageAllMock.mockImplementation((args: Args) => {
    const data = args.toViewModel({ certificate } as Record<"certificate", unknown>);
    return Promise.resolve({ ok: true, node: { id: NODE_ID }, effectiveRole: "editor", data });
  });

  const tree = await EditNodeCertificatePage({ params: Promise.resolve({ id: NODE_ID, cid: CERT_ID }) });
  const found: Record<string, unknown>[] = [];
  const walk = (node: unknown): void => {
    if (Array.isArray(node)) return node.forEach(walk);
    if (!isValidElement(node)) return;
    const el = node as { type: unknown; props: Record<string, unknown> };
    if (el.type === CertificateEditForm) found.push(el.props);
    walk(el.props.children);
  };
  walk(tree);
  if (found.length !== 1) throw new Error(`폼을 찾지 못했습니다(${found.length}개)`);
  return found[0]!;
}

describe("노드 인증서 수정 — 무엇을 어디서 읽는가", () => {
  it("인증서 한 건만 읽고, 노드 id 가 아니라 인증서 id 로 조회한다", async () => {
    const args = await captureArgs();

    expect(args.paths).toEqual({ certificate: `/api/v1/certificates/${CERT_ID}` });
    expect(args.nodeId).toBe(NODE_ID);
  });

  /** 쓰기 화면이라 `config:write` 로 연다 — 읽기 등급으로는 폼을 띄울 이유가 없다. */
  it("config:write 로 연다", async () => {
    expect((await captureArgs()).action).toBe("config:write");
  });

  it("경로의 인증서 id 를 이스케이프한다", async () => {
    expect((await captureArgs("../../apply")).paths.certificate).toBe("/api/v1/certificates/..%2F..%2Fapply");
  });

  it("「다시 시도」는 이 수정 화면, 「목록으로」는 목록이다", async () => {
    const args = await captureArgs();

    expect(args.retryHref).toBe(`${BASE}/${CERT_ID}/edit`);
    expect(args.listHref).toBe(BASE);
    expect(args.retryHref).not.toBe(args.listHref);
  });
});

describe("노드 인증서 수정 — 응답 → 폼 초기값", () => {
  it("저장된 설정을 그대로 폼 초기값으로 옮긴다", async () => {
    const vm = viewModel(await captureArgs());

    expect(vm.defaults).toEqual({
      domains: ["a.example.com", "b.example.com"],
      alias: "",
      staging: true,
      challengeType: "dns-01",
      dnsProvider: "cloudflare",
    });
    expect(vm.isIssued).toBe(true);
  });

  /**
   * 별칭은 입력의 `defaultValue` 가 되어 **그대로 다시 제출된다.** 여기서 빠뜨리면
   * 화면이 빈 입력란으로 열리고, 저장하는 순간 빈 값이 「제거」로 실려 노드의
   * 별칭이 지워진다 — 무변경 저장이 무변경이 아니게 된다.
   */
  it("별칭을 폼 초기값으로 옮긴다", async () => {
    const vm = viewModel(await captureArgs(), certificateResponse({ alias: "운영 API" }));

    expect(vm.defaults.alias).toBe("운영 API");
  });

  it("별칭이 null 이거나(없음) 키 자체가 없으면(구 노드) 빈 입력란으로 연다 — 던지지 않는다", async () => {
    const args = await captureArgs();

    expect(viewModel(args, certificateResponse({ alias: null })).defaults.alias).toBe("");
    expect(viewModel(args, certificateResponse()).defaults.alias).toBe("");
  });

  it("별칭이 문자열이 아니면 던진다", async () => {
    const args = await captureArgs();

    expect(() => viewModel(args, certificateResponse({ alias: { name: "x" } }))).toThrow();
  });

  /**
   * **토큰이 있는지만 온다.** 원문을 되받으면 이 화면을 여는 것만으로 노드의
   * Cloudflare 자격증명이 게이트웨이와 브라우저로 흘러나간다.
   */
  it("토큰은 있는지 여부만 싣는다", async () => {
    const args = await captureArgs();

    expect(viewModel(args, certificateResponse({ hasDnsToken: true })).hasStoredToken).toBe(true);
    expect(viewModel(args, certificateResponse({ hasDnsToken: false })).hasStoredToken).toBe(false);
    // **응답에 토큰 원문이 섞여 오더라도** 폼까지 가지 않는다 — 허용목록이
    // 뚫리는 날(또는 구버전 노드가 원시 행을 주는 날) 이 화면이 두 번째 문이다.
    const leaked = viewModel(args, certificateResponse({ dnsProviderTokenEnc: "cf-token-plaintext" }));
    expect(JSON.stringify(leaked)).not.toContain("cf-token-plaintext");
  });

  /**
   * **원소 깊이로 읽는다.** 폼이 `defaults.domains.join("\n")` 으로 textarea
   * 초기값을 만들고 그 문자열이 그대로 화면에 들어간다 — `join` 은 배열만 보고
   * 원소가 객체여도 `"[object Object]"` 를 만들어 **조용히 틀린 도메인**을 채운다.
   * 사용자가 저장을 누르면 그 값이 굳는다.
   */
  it("도메인 원소가 문자열이 아니면 던진다", async () => {
    const args = await captureArgs();

    expect(() => viewModel(args, certificateResponse({ domains: [{ host: "a.example.com" }] }))).toThrow();
    expect(() => viewModel(args, certificateResponse({ domains: "a.example.com" }))).toThrow();
    expect(() => viewModel(args)).not.toThrow();
  });

  /**
   * `acmeEmail` 도 화면에 나가는 값이다 — 이메일 입력의 `defaultValue` 가 되고
   * **그대로 다시 제출된다.** 문자열이 아닌 것이 통과하면 폼에
   * 「[object Object]」가 들어앉고, 저장하는 순간 노드의 연락처가 그걸로 바뀐다.
   */
  it("연락 주소가 문자열이 아니면 던진다", async () => {
    const args = await captureArgs();

    expect(() => viewModel(args, certificateResponse({ acmeEmail: { at: "a@example.com" } }))).toThrow();
    // `null` 은 계약이 정한 값이라 통과한다(계정이 아직 없다).
    expect(() => viewModel(args, certificateResponse({ acmeEmail: null }))).not.toThrow();
  });

  /** 모르는 값은 안전한 쪽으로 눕힌다 — 로컬 수정 화면과 같은 좁히기다. */
  it("모르는 검증 방식·공급자는 기본 갈래로 눕힌다", async () => {
    const args = await captureArgs();
    const vm = viewModel(args, certificateResponse({ challengeType: "email-01", dnsProvider: "route53" }));

    expect(vm.defaults.challengeType).toBe("http-01");
    expect(vm.defaults.dnsProvider).toBe("");
  });
});

describe("노드 인증서 수정 — 폼에 무엇을 넘기나", () => {
  /**
   * **이 검사가 이 화면에서 제일 비싼 실수를 붙든다.**
   *
   * `getOrCreateAccount`(`lib/acme-service.ts`)는 넘어온 이메일이 저장된 것과
   * 다르면 그 대상의 계정 연락처를 **의도적으로 갱신한다.** 그러니 폼 기본값이
   * 게이트웨이 사용자 주소면, 챌린지 방식만 바꿔 저장하는 것만으로 그 노드의
   * **같은 대상을 쓰는 모든 인증서**의 만료·갱신 안내가 그 사람에게 넘어간다 —
   * 화면에는 아무 표시도 없고, 로컬 화면에서는 일어나지 않는 일이다.
   *
   * 그래서 순서를 **양방향으로** 고정한다: 저장된 주소가 있으면 그것,
   * 없을 때만 지금 로그인한 사람. 한쪽만 보면 반대 방향이 조용히 통과한다.
   */
  it("연락 이메일은 노드에 저장된 주소가 우선이다", async () => {
    const defaults = (await formProps()).defaults as CertificateEditFormProps["defaults"];

    expect(defaults.email).toBe("node-owner@example.com");
    // 게이트웨이 사용자 주소가 아니다 — 그게 들어가면 무변경 저장이 연락처를 덮는다.
    expect(defaults.email).not.toBe("ops@example.com");
    // 나머지는 노드가 준 값 그대로다.
    expect(defaults.domains).toEqual(["a.example.com", "b.example.com"]);
  });

  /** 그 대상에 계정이 아직 없을 때만 로그인한 사람의 주소로 물러난다(로컬과 같은 순서). */
  it("저장된 주소가 없으면 지금 로그인한 사람의 주소로 물러난다", async () => {
    const props = await formProps(certificateResponse({ acmeEmail: null }));
    const defaults = props.defaults as CertificateEditFormProps["defaults"];

    expect(defaults.email).toBe("ops@example.com");
  });

  /** 둘 다 없으면 빈 값 — 폼이 `parseCertificateInput` 의 필드 오류로 막는다. */
  it("둘 다 없으면 빈 문자열이다", async () => {
    currentUserMock.mockResolvedValue({ user: { id: "user-1", email: null } });
    const props = await formProps(certificateResponse({ acmeEmail: null }));
    const defaults = props.defaults as CertificateEditFormProps["defaults"];

    expect(defaults.email).toBe("");
  });

  it("id 와 취소 경로가 노드의 것이다", async () => {
    const props = await formProps();

    expect(props.id).toBe(CERT_ID);
    expect(props.cancelHref).toBe(`${BASE}/${CERT_ID}`);
    expect(props.cancelHref).not.toBe(`/certificates/${CERT_ID}`);
  });

  it("저장 액션이 이 노드로 묶여 있다", async () => {
    expect((await formProps()).action).toEqual({id:"node.updateNodeCertificate",args:[NODE_ID]});
  });

  /**
   * **수동 등록 인증서에는 발급 설정이 없다** — 그 대신 있는 것이 교체 화면이고,
   * 로컬 수정 화면과 **같이** 그리로 보낸다. 폼을 그리면 저장할 때마다 노드가
   * 409 로 거절하는 화면이 되고, 404 로 막으면 사용자가 원하던 화면(교체)이
   * 바로 옆에 있는데도 길이 끊긴다.
   *
   * **보내는 곳이 노드 경로여야 한다** — 로컬 교체 화면으로 보내면 노드의 수동
   * 인증서를 고치려던 사람이 게이트웨이 자신의 인증서를 덮게 된다.
   */
  it("수동 등록 인증서면 그 노드의 교체 화면으로 보낸다", async () => {
    await expect(formProps(certificateResponse({ source: "manual" }))).rejects.toThrow("NEXT_REDIRECT_CALLED");
    expect(redirectMock).toHaveBeenCalledExactlyOnceWith(`${BASE}/${CERT_ID}/replace`);
  });

  it("ACME 인증서는 그대로 폼을 그린다", async () => {
    await formProps();

    expect(redirectMock).not.toHaveBeenCalled();
  });
});
