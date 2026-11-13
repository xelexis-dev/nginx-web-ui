import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { ActionState } from "./types";
vi.mock("@/lib/use-api-action", () => ({ useApiAction: (action: unknown) => action }));
vi.mock("@/lib/api-page", () => ({ readPageView: async () => ({defaultGatewayName:""}) }));

/**
 * `/nodes/new` 의 두 조각 — 리드 문단(서버 컴포넌트)과 페어링 폼(클라이언트
 * 컴포넌트) — 이 **그려 내는 마크업**을 본다. prop 이 아니라 마크업이다.
 *
 * ── 왜 이 파일이 따로 있나 ───────────────────────────────────────────────
 *
 * `form.test.ts`·`service.test.ts` 는 순수 함수와 서비스를 보는 파일이고, 여기서
 * 필요한 것(`vi.mock("react")` 로 `useActionState` 바꿔 끼우기)은 **파일 전체에
 * 걸리는** 모킹이라 그쪽에 섞으면 순수 함수 검사까지 가짜 React 위에서 돌게 된다.
 * 하네스는 `app/admin/tokens/tokens.test.tsx` 의 것을 그대로 가져왔다.
 *
 * ── 무엇에 이빨을 박고 무엇에 안 박나 ────────────────────────────────────
 *
 * **시스템 동작에 대한 사실 주장에만** 박는다:
 *  - **방향** — 토큰은 *연결할 그 노드*에서 발급한다. 이 게이트웨이가 아니다.
 *  - **실패하면 토큰 칸이 비워진다** — 그리고 실제로 비워지는지(제어되지 않는
 *    입력인지)도 함께 잰다. 말만 맞고 동작이 다르면 더 나쁘다.
 *  - **필드 이름이 `token`** — 노드의 `fieldErrors` 키와 같아야 한다.
 *
 * 안내·톤에는 안 박는다. 문구를 다듬을 때마다 깨지는 검사는 문서 편집을 방해하는
 * 물건이 된다(Task 6 수정 2 에서 세운 기준 그대로).
 *
 * ⚠ 문장 단언은 **그 문장이 있는 문단으로 범위를 좁혀서** 건다. 범위를 안 좁힌
 * `toContain` 은 같은 화면의 이웃 문장이 우연히 만족시킬 수 있다(실측된 사고다).
 */

// ──────────────────────────────────────────────────────────────────────────
// 하네스
// ──────────────────────────────────────────────────────────────────────────

/**
 * 서버 렌더는 언제나 초기 상태라, 「실패한 뒤」 분기에 닿을 방법이 렌더 경로에
 * 없다. `useActionState` 만 바꿔 끼워 그 상태를 만들어 준다 — 나머지는 진짜
 * React 다.
 */
let actionStateOverride: ActionState | undefined;

/**
 * 이 렌더에서 컴포넌트가 React 에 넘긴 효과들 — 함수와 **의존성 배열**.
 *
 * ⚠ **이 파일에서 「그려진 값」이 아닌 것을 보는 두 번째 자리다**(첫째는
 * `useActionState`). 정당한 이유는 같다: **마크업에 흔적이 남지 않는데 조용히
 * 깨지는 자리**이기 때문이다. 「같은 사유로 두 번째 실패할 때 배너로 다시
 * 스크롤하는가」는 의존성 배열 하나에 달려 있고, 틀려도 화면은 완전히 정상으로
 * 보인다 — 사용자만 「아무 말이 없다」를 겪는다.
 *
 * 소스 텍스트를 읽는 것이 아니라 **컴포넌트가 실행 중에 실제로 넘기는 값**을 본다.
 */
interface CapturedEffect {
  fn: () => void;
  deps: unknown[] | undefined;
}
let effects: CapturedEffect[] = [];

/** `errorRef.current` 자리에 끼우는 가짜 엘리먼트. 효과를 직접 돌려 볼 수 있게 한다. */
const scrollIntoView = vi.fn();

vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react")>();
  return {
    ...actual,
    useActionState: (_action: unknown, initial: unknown) => [actionStateOverride ?? initial, () => {}, false],
    useEffect: (fn: () => void, deps?: unknown[]) => {
      effects.push({ fn, deps });
    },
    useRef: () => ({ current: { scrollIntoView } }),
  };
});

// 서버 액션 모듈은 `@repo/db`·`next/navigation`·`@/lib/guard` 를 끌어온다. 폼이
// 그리는 마크업과는 무관하므로 통째로 끊는다.
vi.mock("./actions", () => ({
  pairNode: vi.fn(),
  updateNode: vi.fn(),
  unpairNode: vi.fn(),
}));

const getDbMock = vi.fn(() => ({ query: { appSettings: { findFirst: vi.fn(async () => undefined) } } }));
vi.mock("@repo/db", () => ({ getDb: getDbMock }));
vi.mock("@/lib/guard", () => ({ requireRole: vi.fn(async () => ({ user: { id: "admin-1" } })) }));

const { NodeForm } = await import("./node-form");
const { default: NewNodePage } = await import("./new/page");

// ── 마크업 읽기 (tokens.test.tsx 의 헬퍼를 그대로 쓴다) ────────────────────

function textOfHtml(fragment: string): string {
  return fragment
    .replace(/<[^>]*>/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

/** 문단별 텍스트. **문장 단언의 범위를 좁히는 데 쓴다.** */
function pTexts(html: string): string[] {
  return [...html.matchAll(/<p[^>]*>([\s\S]*?)<\/p>/g)].map((m) => textOfHtml(m[1] ?? ""));
}

function inputTags(html: string): string[] {
  return [...html.matchAll(/<input[^>]*>/g)].map((m) => m[0]);
}

/** `name="…"` 을 단 입력들의 이름. 폼이 실제로 무엇을 제출하는지다. */
function inputNames(html: string): string[] {
  return inputTags(html)
    .map((tag) => /name="([^"]*)"/.exec(tag)?.[1])
    .filter((name): name is string => name !== undefined);
}

function tagWithName(html: string, name: string): string {
  const tag = inputTags(html).find((t) => t.includes(`name="${name}"`));
  if (!tag) throw new Error(`no <input name="${name}">`);
  return tag;
}

function renderForm(state?: ActionState): string {
  actionStateOverride = state;
  effects = [];
  return renderToStaticMarkup(<NodeForm defaultGatewayName="gateway.example.com" />);
}

/**
 * 이 렌더에서 **`NodeForm` 자신이** 등록한 효과. 부모의 함수 본문이 자식 렌더보다
 * 먼저 끝나므로 첫 번째가 그것이다(`Select` 의 효과는 그 뒤에 온다). 이 순서가
 * 언젠가 달라지면 아래 단언들이 **조용히 통과하는 게 아니라 실패한다** — 의존성이
 * `attempt` 가 아니게 되기 때문이다.
 */
function firstEffect(): CapturedEffect {
  const effect = effects[0];
  if (!effect) throw new Error("효과가 등록되지 않았다");
  return effect;
}

function ownEffect(state?: ActionState): CapturedEffect {
  renderForm(state);
  return firstEffect();
}

const FAILED: ActionState = { ok: false, error: "토큰이 유효하지 않거나 해지되었습니다.", attempt: 1 };

beforeEach(() => {
  actionStateOverride = undefined;
  effects = [];
  scrollIntoView.mockClear();
});

// ──────────────────────────────────────────────────────────────────────────

describe("NodeForm — 토큰 한 칸", () => {
  /**
   * ⚠ **이름이 `token` 이어야 한다.** 노드는 빈 토큰을 `fieldErrors.token` 으로
   * 내고 `service.ts` 는 그 키를 이름 변환 없이 그대로 붙인다. 다른 이름을 쓰면
   * 노드의 필드 오류가 아무 칸에도 닿지 못하고 **조용히 사라진다** — 그리고 그
   * 사고는 fetch 를 건너뛰는 이 스위트의 다른 어떤 검사도 보지 못한다.
   */
  it("제출하는 입력 이름이 계약 그대로다 — 자격증명 칸은 없다", () => {
    // 체크박스까지 포함한 `<input name>` 전부. 순서까지 본다 — 칸이 사라지면
    // 그 자리에서 보인다(`role` 은 `Select` 의 hidden input 이라 여기 함께 온다).
    expect(inputNames(renderForm())).toEqual(["name", "baseUrl", "gatewayName", "role", "allowInsecureTls", "token"]);
  });

  it("토큰 칸은 password 타입이다 — 화면에 평문으로 서 있지 않는다", () => {
    expect(tagWithName(renderForm(), "token")).toContain('type="password"');
  });

  /**
   * **실패한 뒤 토큰 평문을 폼 상태에 남기지 않는다.** `<form action={서버액션}>`
   * 은 제출이 끝나면 React 가 폼을 리셋하므로, **제어되지 않는** 입력은 그때
   * 비워진다. `value=` 가 붙는 순간 그 리셋이 무력해지고 평문이 리렌더되는
   * 마크업에 계속 살아 있는다 — 서버 쪽에서 닫아 둔 축이 화면에서 다시 열린다.
   *
   * 대조로 `name` 칸을 함께 본다: 그쪽은 **제어형이어야** 한다(실패해도 다시 치지
   * 않게). 둘을 같이 재야 「전부 제어형」·「전부 비제어형」 두 방향의 결함이 다
   * 잡힌다.
   */
  it("토큰 칸에는 value 가 없다(비제어) — 이름 칸에는 있다(제어)", () => {
    const html = renderForm(FAILED);

    expect(tagWithName(html, "token")).not.toContain("value=");
    expect(tagWithName(html, "name")).toContain("value=");
  });

  /**
   * **방향.** 토큰은 연결할 *그 노드*의 관리자 패널에서 나온다. 이 게이트웨이의
   * `/admin/tokens` 에서 만든 토큰은 「다른 게이트웨이가 우리를 자기 노드로 붙일
   * 때」쓰는 것이라 여기서 아무 효과가 없고, 그 값은 하필 우리 서버의 제어권을
   * 넘기는 자격증명이다. 거꾸로 적히면 그것을 남의 화면에 붙여넣게 된다.
   */
  it("토큰을 어디서 발급하는지 말한다 — 그 노드에서다", () => {
    const paras = pTexts(renderForm());
    const hint = paras.find((p) => p.includes("붙여넣으세요"));

    expect(hint).toContain(
      "연결할 노드의 관리자 패널 → 액세스 토큰에서 node:pair 권한으로 발급한 토큰을 붙여넣으세요.",
    );
    expect(hint).toContain("이 게이트웨이의 액세스 토큰이 아니라 그 노드에서 발급한 것입니다.");
  });

  /**
   * **비워진다는 것을 화면이 말해 준다.** 안 그러면 「입력이 지워졌다」는 버그로
   * 읽힌다 — 그게 이 화면에 실제로 들어왔던 신고다. 위의 `value=` 단언과 짝이다:
   * 저기가 동작, 여기가 그 동작에 대한 진술.
   */
  it("실패 안내가 토큰 칸만 비웠다고 말한다", () => {
    const paras = pTexts(renderForm(FAILED));
    const hint = paras.find((p) => p.includes("입력값은 그대로 두었습니다"));

    expect(hint).toContain("토큰 칸만 비웠습니다 — 다시 붙여넣고 연결을 다시 시도하세요.");
  });

  it("노드가 보낸 fieldErrors.token 이 토큰 칸 아래에 그려진다", () => {
    const paras = pTexts(renderForm({ ok: false, fieldErrors: { token: "이 토큰에는 노드 연결(node:pair) 권한이 없습니다." } }));

    expect(paras).toContain("이 토큰에는 노드 연결(node:pair) 권한이 없습니다.");
  });
});

describe("/nodes/new 리드", () => {
  /**
   * 같은 방향 주장을 **폼에 닿기 전에** 한 번 한다. 여기가 거꾸로 적히면 사용자는
   * 이 게이트웨이의 `/admin/tokens` 로 가서 자기 서버의 제어권을 넘기는 토큰을
   * 만든다 — 폼의 안내를 읽기도 전에.
   */
  it("연결할 노드에서 발급한 토큰이 필요하다고 말한다 — 여기서 만드는 것이 아니다", async () => {
    const lead = pTexts(renderToStaticMarkup(await NewNodePage()))[0];

    expect(lead).toContain(
      "연결할 노드의 관리자 패널 → 액세스 토큰에서 node:pair 권한으로 발급한 서버 토큰이 필요합니다 — 이 게이트웨이에서 발급한 토큰이 아닙니다.",
    );
  });
});

/**
 * ── 실패를 **다시** 알리는가 (수정 1) ────────────────────────────────────
 *
 * 폼은 창보다 길고(실측 935px vs 552px) 제출 버튼은 맨 아래에 있다. 그래서 실패
 * 배너는 **창 위쪽 바깥**에 그려지고, 스크롤을 옮겨 주지 않으면 사용자에게는
 * 「토큰 칸만 비고 아무 말도 없다」로 보인다.
 *
 * ⚠ **그 스크롤이 오류 **문구**에 매여 있으면 두 번째 실패에서 돌지 않는다** —
 * `Object.is` 가 성립하기 때문이다. 그런데 틀린 토큰을 붙여넣고 또 붙여넣는 것이
 * 이 화면에서 **가장 흔한 경로**다. 그러니까 하필 제일 흔한 자리에서 조용해진다.
 *
 * 여기가 이 라운드에 이빨이 없었던 자리다.
 */
describe("NodeForm — 같은 사유로 두 번 실패해도 다시 알린다", () => {
  const SAME = "토큰이 유효하지 않거나 해지되었습니다.";

  it("마크업이 한 글자도 안 달라져도 스크롤 효과의 의존성은 달라진다", () => {
    const firstHtml = renderForm({ ok: false, error: SAME, attempt: 1 });
    const first = firstEffect();
    const secondHtml = renderForm({ ok: false, error: SAME, attempt: 2 });
    const second = firstEffect();

    /**
     * **전제이자 이 버그의 정체다.** 두 실패가 내는 마크업은 **바이트까지 같다** —
     * `attempt` 는 `key` 로만 쓰여 직렬화되지 않는다. 그러므로 화면에 그려진 것 중
     * 무엇도 「또 실패했다」를 말해 줄 수 없고, 문구를 의존성으로 삼은 효과는
     * 돌 이유를 찾지 못한다.
     */
    expect(secondHtml).toBe(firstHtml);

    // 주장 그 자체: 그럼에도 React 가 다시 돌릴 근거가 있는가.
    expect(second.deps).not.toEqual(first.deps);
    // 그 근거가 **제출 횟수**여야 한다. 배열 모양까지 박지는 않는다 —
    // `[attempt, error]` 로 늘려도 동작은 옳다.
    expect(first.deps).toContain(1);
    expect(second.deps).toContain(2);
  });

  it("효과를 돌리면 배너로 스크롤한다", () => {
    ownEffect({ ok: false, error: SAME, attempt: 1 }).fn();

    expect(scrollIntoView).toHaveBeenCalledExactlyOnceWith({ behavior: "smooth", block: "center" });
  });

  /**
   * 필드 오류만 있는 실패(빈 토큰 등)는 그 칸 옆에 그려지므로 배너가 없다.
   * 여기서 스크롤하면 **없는 것으로** 화면을 옮기게 된다.
   */
  it("일반 오류가 없으면 스크롤하지 않는다", () => {
    ownEffect({ ok: false, fieldErrors: { token: "이 노드의 관리자 화면에서 발급한 서버 토큰을 입력하세요." }, attempt: 1 }).fn();

    expect(scrollIntoView).not.toHaveBeenCalled();
  });

  /**
   * 배너 하나로 끝난다. 예전에는 제출 버튼 위에 **같은 문장이 한 번 더** 있었는데,
   * 그것은 스크롤이 두 번째 실패에서 안 돌던 시절의 유일한 피드백이었다. 위
   * 검사들이 그 구멍을 닫았으므로 이제 중복은 중복일 뿐이다.
   */
  it("일반 오류를 화면에 한 번만 그린다", () => {
    const paras = pTexts(renderForm({ ok: false, error: SAME, attempt: 1 }));

    expect(paras.filter((p) => p === SAME)).toHaveLength(1);
  });
});
