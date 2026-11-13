import { CertificateForm } from "@/components/certificates/certificate-form";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement } from "react";

/**
 * 노드 인증서 **발급** 화면이 붙드는 것.
 *
 * 이 화면은 노드에서 읽어오는 것이 없다 — 폼에 채울 후보 목록이 없기 때문이다.
 * 그래서 `loadNodePage*` 가 아니라 `authorizeNodePage` 로 접근만 연다.
 *
 * **연락 이메일이 로컬과 다르다**(게이트웨이 사용자 것). 노드의
 * `acme_accounts.email` 은 어떤 `/api/v1` 응답에도 없어서 보여줄 수가 없고,
 * 노드에 계정이 이미 있으면 그 값은 쓰이지도 않는다(`getOrCreateAccount` 는
 * 기존 계정의 이메일을 건드리지 않는다). **의도된 차이라 검사로 고정한다** —
 * 안 그러면 다음 사람이 버그로 보고 계약을 바꾸려 든다.
 */
const authorizeNodePageMock = vi.fn();
vi.mock("@/lib/node-page", () => ({ authorizeNodePage: authorizeNodePageMock }));

const currentUserMock = vi.fn();
vi.mock("@/lib/api-page", () => ({ readPageApi: currentUserMock }));

// `@repo/ui`·CSS 모듈 사슬을 이 프로세스로 끌고 오지 않기 위한 목이다.
vi.mock("@/components/certificates/certificate-form", () => ({ CertificateForm: () => null }));

const createNodeCertificateMock = vi.fn();
vi.mock("../../actions", () => ({ createNodeCertificate: createNodeCertificateMock }));

const { default: NewNodeCertificatePage } = await import("./page");

const NODE_ID = "node-1";
const BASE = `/nodes/${NODE_ID}/certificates`;

beforeEach(() => {
  authorizeNodePageMock.mockReset();
  currentUserMock.mockReset();
  currentUserMock.mockResolvedValue({ user: { id: "user-1", email: "ops@example.com" } });
});

function render() {
  return NewNodeCertificatePage({ params: Promise.resolve({ id: NODE_ID }) });
}

/** 폼 엘리먼트를 트리에서 찾는다 — 이 화면은 `<section>` 하나에 폼이 들어 있다. */
async function formProps(): Promise<Record<string, unknown>> {
  authorizeNodePageMock.mockResolvedValue({ ok: true, node: { id: NODE_ID }, effectiveRole: "editor" });
  const tree = await render();
  const found: Record<string, unknown>[] = [];
  const walk = (node: unknown): void => {
    if (Array.isArray(node)) return node.forEach(walk);
    if (!isValidElement(node)) return;
    const el = node as { type: unknown; props: Record<string, unknown> };
    if (el.type === CertificateForm) found.push(el.props);
    walk(el.props.children);
  };
  walk(tree);
  if (found.length !== 1) throw new Error(`폼을 찾지 못했습니다(${found.length}개)`);
  return found[0]!;
}

describe("노드 인증서 발급 화면 — 접근", () => {
  /**
   * **읽기가 아니라 쓰기 등급으로 연다.** `config:read` 로 열면 viewer 에게 폼이
   * 그려지고, 제출해야만 거부당한다 — 로컬 `new/page.tsx` 의
   * `requireRole("config:write")` 에 대응한다.
   */
  it("config:write 로 열고, 「다시 시도」와 「목록으로」가 다르다", async () => {
    authorizeNodePageMock.mockResolvedValue({ ok: false, render: null });

    await render();

    expect(authorizeNodePageMock).toHaveBeenCalledExactlyOnceWith({
      nodeId: NODE_ID,
      action: "config:write",
      retryHref: `${BASE}/new`,
      listHref: BASE,
    });
  });

  it("접근이 막히면 그 카드를 그대로 그린다", async () => {
    const card = { card: true };
    authorizeNodePageMock.mockResolvedValue({ ok: false, render: card });

    expect(await render()).toBe(card);
  });
});

describe("노드 인증서 발급 화면 — 폼에 무엇을 넘기나", () => {
  it("취소는 노드 인증서 목록으로 간다 — 로컬 목록이 아니다", async () => {
    const props = await formProps();

    expect(props.cancelHref).toBe(BASE);
    expect(props.cancelHref).not.toBe("/certificates");
  });

  /**
   * **연락 이메일은 게이트웨이 사용자 것이다** — 위 파일 헤더. 세션이 아니라
   * `currentUser()` 의 DB 행에서 읽는다(로컬 발급 화면과 같은 출처).
   */
  it("연락 이메일 기본값이 지금 로그인한 사람의 주소다", async () => {
    expect((await formProps()).defaultEmail).toBe("ops@example.com");
  });

  /** 이메일이 없는 계정(OAuth 전용)이면 빈 값 — 폼이 required 로 막는다. */
  it("이메일이 없으면 빈 문자열이다", async () => {
    currentUserMock.mockResolvedValue({ user: { id: "user-1", email: null } });

    expect((await formProps()).defaultEmail).toBe("");
  });

  /**
   * 액션은 `nodeId` 로 묶여야 한다 — 안 묶으면 폼의 `(prev, formData)` 가 그대로
   * 첫 두 인자로 들어가 **노드 id 자리에 이전 상태 객체**가 온다.
   */
  it("발급 액션이 이 노드로 묶여 있다", async () => {
    expect((await formProps()).action).toEqual({id:"node.createNodeCertificate",args:[NODE_ID]});
  });
});
