import { ImportForm } from "@/components/certificates/import-form";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement } from "react";

/**
 * 노드 인증서 **가져오기**(수동 PEM 등록) 화면이 붙드는 것.
 *
 * 이 화면은 노드에서 읽어오는 것이 없다 — 폼에 채울 후보 목록이 없기 때문이다
 * (발급 화면과 같다). 그래서 `loadNodePage*` 가 아니라 `authorizeNodePage` 로
 * 접근만 연다.
 *
 * **이 화면이 나르는 것은 개인키다.** 폼이 무엇을 어디로 보내는지가 곧 그 키가
 * 어디로 가는지라, 액션과 취소 경로 둘 다 노드 것인지 확인한다 — 로컬 액션이
 * 들어가면 「이 노드에 가져오기」를 누른 사람이 **게이트웨이 자신에게** 인증서를
 * 등록하게 되고, 화면은 멀쩡히 성공한다.
 */
const authorizeNodePageMock = vi.fn();
vi.mock("@/lib/node-page", () => ({ authorizeNodePage: authorizeNodePageMock }));

// `@repo/ui`·CSS 모듈 사슬을 이 프로세스로 끌고 오지 않기 위한 목이다.
vi.mock("@/components/certificates/import-form", () => ({ ImportForm: () => null }));

const importNodeCertificateMock = vi.fn();
vi.mock("../../actions", () => ({ importNodeCertificate: importNodeCertificateMock }));

const { default: ImportNodeCertificatePage } = await import("./page");

const NODE_ID = "node-1";
const BASE = `/nodes/${NODE_ID}/certificates`;

beforeEach(() => {
  authorizeNodePageMock.mockReset();
  importNodeCertificateMock.mockReset();
});

function render() {
  return ImportNodeCertificatePage({ params: Promise.resolve({ id: NODE_ID }) });
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
    if (el.type === ImportForm) found.push(el.props);
    walk(el.props.children);
  };
  walk(tree);
  if (found.length !== 1) throw new Error(`폼을 찾지 못했습니다(${found.length}개)`);
  return found[0]!;
}

describe("노드 인증서 가져오기 화면 — 접근", () => {
  /**
   * **읽기가 아니라 쓰기 등급으로 연다.** `config:read` 로 열면 viewer 가 PEM 을
   * 다 붙여넣고 제출해야만 거부당한다 — 로컬 `import/page.tsx` 의
   * `requireRole("config:write")` 이자 노드 라우트의
   * `requireNodeAction(caller, "config:write")` 와 같은 등급이다.
   */
  it("config:write 로 열고, 「다시 시도」와 「목록으로」가 다르다", async () => {
    authorizeNodePageMock.mockResolvedValue({ ok: false, render: null });

    await render();

    expect(authorizeNodePageMock).toHaveBeenCalledExactlyOnceWith({
      nodeId: NODE_ID,
      action: "config:write",
      retryHref: `${BASE}/import`,
      listHref: BASE,
    });
  });

  it("접근이 막히면 그 카드를 그대로 그린다", async () => {
    const card = { card: true };
    authorizeNodePageMock.mockResolvedValue({ ok: false, render: card });

    expect(await render()).toBe(card);
  });
});

describe("노드 인증서 가져오기 화면 — 폼에 무엇을 넘기나", () => {
  it("취소는 노드 인증서 목록으로 간다 — 로컬 목록이 아니다", async () => {
    const props = await formProps();

    expect(props.cancelHref).toBe(BASE);
    expect(props.cancelHref).not.toBe("/certificates");
  });

  /** 가져오기는 **새** 인증서를 만든다 — 교체할 대상이 없다(`certId` 없음). */
  it("certId 를 넘기지 않는다 — 교체가 아니라 등록이다", async () => {
    expect((await formProps()).certId).toBeUndefined();
  });

  /**
   * 액션은 `nodeId` 로 묶여야 한다 — 안 묶으면 폼의 `(prev, formData)` 가 그대로
   * 첫 두 인자로 들어가 **노드 id 자리에 이전 상태 객체**가 온다.
   */
  it("가져오기 액션이 이 노드로 묶여 있다", async () => {
    expect((await formProps()).action).toEqual({id:"node.importNodeCertificate",args:[NODE_ID]});
  });
});
