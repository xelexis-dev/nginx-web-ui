import { createElement, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

/**
 * **스펙 §9 실패 카드가 화면까지 실제로 닿는지**를 노드 화면 **전부**에 대해
 * 한 번에 붙든다.
 *
 * 왜 필요한가: `loadNodePage`/`loadNodePageAll`/`authorizeNodePage` 는 세 가지
 * 실패(접근 거부·`callNode` 실패·뷰모델 변환 실패)를 전부 `{ ok: false, render }`
 * 로 접어 돌려주고, 화면은 `if (!loaded.ok) return loaded.render;` 한 줄로 그걸
 * 내보낸다. **그 한 줄을 `return null;` 로 바꿔도 444/444 가 초록이었다** —
 * 노드가 죽었을 때 사용자는 안내도 「다시 시도」도 없는 **빈 화면**을 본다.
 * 타입도 안 잡는다(`ReactNode` 에 `null` 이 들어간다).
 *
 * **화면 수만큼의 비슷한 테스트를 쓰지 않는다.** 그 대신 화면들이 공유하는 구조를
 * 그대로 검사한다: 세 헬퍼가 실패 카드를 건네주게 목으로 고정해 두고, 발견된
 * 모든 노드 페이지 모듈을 실제로 호출해 **건네받은 그 카드가 출력에 들어 있는지**
 * 본다. 화면이 하나 늘면 이 목록도 자동으로 늘어난다 — 새 화면은 아무것도 안
 * 해도 이 검사를 받는다.
 *
 * 카드를 **감싸는 것**은 허용한다(트리를 훑는다). 붙드는 것은 「감싸지 않았다」가
 * 아니라 「버리지 않았다」이기 때문이다.
 */

/**
 * 헬퍼가 건네주는 카드. **동일성으로 찾는다** — 모양이 같은 다른 엘리먼트를
 * 화면이 직접 만들어 낸 경우와 구분되어야 한다.
 */
const FAILURE_CARD = createElement("div", { "data-testid": "node-failure-card" });
const FAILURE = { ok: false as const, render: FAILURE_CARD as ReactNode };

const loadNodePageMock = vi.fn(async () => FAILURE);
const loadNodePageAllMock = vi.fn(async () => FAILURE);
const authorizeNodePageMock = vi.fn(async () => FAILURE);
vi.mock("@/lib/node-page", () => ({
  loadNodePage: loadNodePageMock,
  loadNodePageAll: loadNodePageAllMock,
  authorizeNodePage: authorizeNodePageMock,
}));

/**
 * **`lib/node-page` 를 지나지 않는 화면들.** 각각 스스로 `resolveNode` 를 부르고
 * 자기 실패 처리를 갖는다 — 이 검사의 전제(세 헬퍼가 카드를 건넨다)가 성립하지
 * 않으므로 제외한다. **줄이려면 이유가 있어야 한다**: 새 화면이 여기 들어오면
 * 그 화면의 §9 실패 처리는 아무도 안 보고 있다는 뜻이다.
 *
 * - `page.tsx`(노드 개요)·`logs`·`metrics`: 셋 다 `resolveNode` 를 직접 부르고
 *   `denied` 일 때 `NodeErrorCard` 를 **자기 파일 안에서** 직접 만든다(읽어올
 *   데이터가 없거나 클라이언트가 중계 라우트로 따로 가져오는 화면들이라
 *   `loadNodePage*` 를 지날 이유가 없다).
 * - `edit`: **노드 자신이 아니라 게이트웨이의 노드 행**을 고치는 화면이라
 *   원격 호출이 없다 — `requireRole("user:manage")` + 게이트웨이 DB 만 본다.
 */
const EXEMPT = new Set(["./page.tsx", "./logs/page.tsx", "./metrics/page.tsx", "./edit/page.tsx"]);

/**
 * 경로에 `[id]` 같은 대괄호가 들어 있어 glob 의 문자 클래스로 읽히므로,
 * 디렉터리 이름을 `*`/`**` 로 지나간다.
 */
// Parallel slots render navigation; only children pages own node failure cards.
const discovered = { ...import.meta.glob("./page.tsx"), ...import.meta.glob(["./**/page.tsx", "!./@*/**/page.tsx"]) } as Record<
  string,
  () => Promise<{ default: (props: never) => Promise<ReactNode> }>
>;

const covered = Object.keys(discovered)
  .filter((p) => !EXEMPT.has(p))
  .sort();

/** 화면마다 dynamic 세그먼트 이름이 다르다 — 전부 한 객체에 담아 넘긴다. */
const PARAMS = {
  id: "node-1",
  cid: "cert-9",
  sid: "server-3",
  upstreamId: "upstream-2",
  streamId: "stream-4",
  logId: "log-5",
};

/** 카드가 어디에 있든(그대로든, 감싸였든) 찾아낸다. */
function containsCard(node: unknown): boolean {
  if (node === FAILURE_CARD) return true;
  if (Array.isArray(node)) return node.some(containsCard);
  if (isValidElement(node)) {
    const children = (node.props as { children?: unknown }).children;
    return children === undefined ? false : containsCard(children);
  }
  return false;
}

beforeEach(() => {
  loadNodePageMock.mockClear();
  loadNodePageAllMock.mockClear();
  authorizeNodePageMock.mockClear();
});

describe("노드 화면 — §9 실패 카드는 반드시 화면까지 간다", () => {
  /**
   * glob 이 조용히 아무것도 못 찾으면 아래 `it.each` 가 0개가 되고, 이 파일은
   * 「전부 통과」처럼 보이면서 아무것도 검사하지 않는다. **화면을 더하거나 뺄
   * 때 이 숫자를 함께 고친다** — 그게 이 검사가 요구하는 유일한 손질이다.
   */
  it("노드 화면 인벤토리 — 21벌이 헬퍼를 지난다", () => {
    expect(covered).toHaveLength(21);
    // 제외 목록이 낡으면(경로가 바뀌었는데 그대로면) 그 화면은 조용히 검사에서
    // 빠진다 — 제외 항목이 실제로 존재하는 파일인지 함께 확인한다.
    for (const path of EXEMPT) expect(Object.keys(discovered)).toContain(path);
  });

  it.each(covered)("%s 은 건네받은 실패 카드를 그대로 내보낸다", async (path) => {
    const mod = await discovered[path]!();

    const rendered = await mod.default({
      params: Promise.resolve(PARAMS),
      searchParams: Promise.resolve({}),
    } as never);

    // 전제 확인: 이 화면은 정말 세 헬퍼 중 하나를 지났다. 안 지났다면 카드가
    // 없는 것이 당연하므로, 그 경우를 「검사 통과」로 착각하지 않게 못박는다.
    const wentThroughHelper =
      loadNodePageMock.mock.calls.length +
        loadNodePageAllMock.mock.calls.length +
        authorizeNodePageMock.mock.calls.length >
      0;
    expect(wentThroughHelper).toBe(true);

    expect(containsCard(rendered)).toBe(true);
    expect(renderToStaticMarkup(rendered).match(/<h1\b/g) ?? []).toHaveLength(1);
  });
});
