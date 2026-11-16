import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { Role } from "@repo/contracts";
import type { TokenListRow } from "@/lib/token-presentation";
import { tokenWireFixture, tokenScopeFixture } from "@/lib/token-api-fixtures.test-support";

/**
 * `/admin/tokens` 세 화면과 그 서버 액션 — **서버 토큰**이다.
 *
 * ── 무엇이 Task 5 와 다른가 ─────────────────────────────────────────────
 *
 * 화면 컴포넌트(`TokenList`·`TokenDetail`·`TokenForm`)와 순수 함수(`tokens.ts`)는
 * `app/(app)/account/tokens/` 의 것을 **그대로 쓴다** — 그쪽 검사가 이미 그것들을
 * 덮고 있으므로 여기서 다시 덮지 않는다(복제한 단언은 한쪽만 고쳐지는 날 거짓말을
 * 시작한다). 이 파일이 붙드는 것은 **이 태스크가 새로 쓴 것**뿐이다:
 *
 * 1. 등급 게이트(`requireRole("user:manage")`)가 `listTokens` **앞에** 있는가.
 * 2. 서비스에 가는 `kind`/`scope` 가 **코드**에서 오는가(폼이 아니라).
 * 3. `node:pair` 가 **이 화면에만** 있는가.
 * 4. 세 페이지가 공용 컴포넌트를 실제로 조립하고 `basePath` 를 넘기는가.
 *
 * ── 하네스 ───────────────────────────────────────────────────────────────
 *
 * Task 5 와 같다: 부분 `vi.mock("react")` 로 `useActionState` 만 바꿔 끼우고
 * (서버 렌더는 언제나 초기 상태라 「발급 직후」 분기에 닿을 방법이 렌더 경로에 없다)
 * 나머지는 진짜 React 로 `renderToStaticMarkup` 한다. 그래서 **prop 이 아니라
 * 그려진 마크업**을 본다.
 *
 * `useState`(복사 상태) 바꿔 끼우기는 가져오지 않았다 — 그 축은 `TokenForm` 의 것이고
 * Task 5 의 검사가 세 상태를 모두 덮는다. 여기서 다시 덮으면 같은 것을 두 번 적는다.
 *
 * ⚠ **표의 행과 정의목록은 통째로 `toEqual` 한다.** 칸을 하나씩 `toContain` 으로
 * 훑으면 열이 통째로 사라져도 초록이다(Task 5 리뷰 실측).
 */

let actionStateOverride: unknown;
/**
 * 마지막으로 `useActionState` 에 들어온 액션.
 *
 * ⚠ **이 파일에서 유일하게 「그려진 값」이 아닌 것을 보는 자리다.** 서버 액션은
 * 마크업에 아무 흔적도 남기지 않아(제출 경로는 React 런타임 안에 있다) 렌더 결과로는
 * 「어느 액션이 걸렸는가」를 볼 방법이 없다. 그런데 여기가 바로 사용자 토큰 액션을
 * 잘못 넘겨도 **화면이 완전히 정상으로 보이는** 자리다 — 그러면 관리자 화면의 발급
 * 버튼이 `kind:"user"` 토큰을 만든다. 그래서 동일성으로 붙든다.
 */
let lastActionArg: unknown;

vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react")>();
  return {
    ...actual,
    useActionState: (action: unknown, initial: unknown) => {
      lastActionArg = action;
      return [actionStateOverride ?? initial, () => {}, false];
    },
  };
});

/**
 * 지금 로그인한 사람. `signedInAs` 가 세운다.
 *
 * `requireRole` 을 **진짜 `can()` 으로** 흉내 내는 것이 요점이다 — 고정된
 * `mockResolvedValue` 로 두면 「editor 는 막힌다」가 RBAC 표가 아니라 검사가 적어
 * 둔 값을 확인하는 꼴이 된다.
 */
let sessionUser: { id: string; email: string | null; role: Role } | null = null;

const redirectMock = vi.fn((_to: string): never => {
  throw new Error("NEXT_REDIRECT");
});
const notFoundMock = vi.fn((): never => {
  throw new Error("NEXT_NOT_FOUND");
});
vi.mock("next/navigation", () => ({ redirect: redirectMock, notFound: notFoundMock }));

/** 사용자 화면(`/account/tokens`)을 한 번 그려 보기 위해 함께 둔다 — 아래 교차 검사. */
const currentUserMock = vi.fn(async () => (sessionUser === null ? null : { user: sessionUser }));
vi.mock("@/lib/guard", () => ({ currentUser: currentUserMock }));

const listTokensMock = vi.fn();
// API-response fixtures replace the old direct-DB page boundary.
// Backend role/ownership enforcement is covered by Rust auth_tokens_db/admin_http.
const readPageApiMock = vi.fn(async (path: string) => {
  const user = sessionUser;
  if (!user || (path.includes("/admin/") && user.role !== "admin")) redirectMock("/login");

  const tokens = ((await listTokensMock()) as TokenListRow[]).map(tokenWireFixture);
  const id = path.split("/tokens/")[1];
  if (id !== undefined) return tokens.find((row) => row.id === decodeURIComponent(id)) ?? notFoundMock();
  return { tokens, issuableScopes: tokenScopeFixture(path.includes("/admin/") ? "server" : "user", user!.role) };
});
vi.mock("@/lib/api-page", () => ({ readPageApi: readPageApiMock }));
vi.mock("@/lib/use-api-action", () => ({ useApiAction: (action: unknown) => action }));

const { default: ServerTokensPage } = await import("./page");
const { default: NewServerTokenPage } = await import("./new/page");
const { default: ServerTokenDetailPage } = await import("./[id]/page");
const { default: NewAccountTokenPage } = await import("@/app/(app)/account/tokens/new/page");

// ──────────────────────────────────────────────────────────────────────────
// 고정값
// ──────────────────────────────────────────────────────────────────────────

const NOW = new Date("2026-08-30T09:00:00.000Z");

/** 서버 토큰의 평문 모양(`nwu_s_`). 목록·상세에 이런 것이 있으면 샌 것이다. */
const PLAINTEXT = "nwu_s_dGVzdC1vbmx5LW5vdC1hLXJlYWwtdG9rZW4tdmFsdWUxcd12";
const MASKED = "nwu_s_…cd12";
/** 접두사 + 20자 이상의 몸통 = 평문의 모양. 가림 표시(`…`)는 여기 걸리지 않는다. */
const TOKEN_LIKE = /nwu_[us]_[A-Za-z0-9_-]{20,}/;

/** 세션에서 조립된 주체. **`role` 이 붙어 있다** — 서비스가 등급을 다시 본다. */
/**
 * ⚠ **`ownerUserId` 가 없다.** 서버 토큰에는 주인이 없고, 이 값은 폼이 아니라
 * 코드에서 온다 — 이 한 줄이 「이 화면은 사용자 토큰을 보여주지 않는다」의 전부다.
 */

/**
 * 서비스가 내는 행. 이름에 **노드 정체**가 실리는 것이 서버 토큰의 성격이고
 * (스펙 예시), 이 화면이 admin 전용인 이유의 절반이다.
 */
function row(over: Partial<TokenListRow> = {}): TokenListRow {
  return {
    id: "tok-1",
    name: "node.example.com 연결용",
    scopes: ["node:pair"],
    // **연도를 전부 다르게** 둔다 — 칸이 뒤바뀌면 그 자리에서 보인다.
    // 정오(UTC)라 어떤 TZ 에서도 연도가 넘어가지 않는다.
    createdAt: new Date("2024-01-02T12:00:00.000Z"),
    lastUsedAt: new Date("2025-06-07T12:00:00.000Z"),
    expiresAt: new Date("2026-12-01T12:00:00.000Z"),
    revokedAt: null,
    masked: MASKED,
    ...over,
  };
}

function signedInAs(role: Role) {
  sessionUser = { id: role === "admin" ? "admin-1" : "user-1", email: `${role}@example.com`, role };
}

/** 서버 컴포넌트를 실제로 부르고 끝까지 그린다. */
async function renderPage(page: () => Promise<ReactNode>): Promise<string> {
  return renderToStaticMarkup(await page());
}

// ── 마크업 읽기 (Task 5 의 헬퍼를 그대로 쓴다) ────────────────────────────

/** 태그를 공백으로 바꾸고 접는다 — 인접한 `<li>` 들이 한 낱말로 붙지 않게. */
function textOfHtml(fragment: string): string {
  return fragment
    .replace(/<[^>]*>/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

/** 표 본문의 행별 셀 텍스트. **행을 통째로 비교**하려고 배열로 낸다. */
function bodyRows(html: string): string[][] {
  const tbody = /<tbody>([\s\S]*?)<\/tbody>/.exec(html)?.[1] ?? "";
  return [...tbody.matchAll(/<tr>([\s\S]*?)<\/tr>/g)].map((tr) =>
    [...(tr[1] ?? "").matchAll(/<td[^>]*>([\s\S]*?)<\/td>/g)].map((td) => textOfHtml(td[1] ?? "")),
  );
}

/** 정의목록의 term→value. 키 순서까지 봐야 행 삭제가 잡힌다. */
function dlPairs(html: string): Record<string, string> {
  const dl = /<dl[^>]*>([\s\S]*?)<\/dl>/.exec(html)?.[1] ?? "";
  const terms = [...dl.matchAll(/<dt[^>]*>([\s\S]*?)<\/dt>/g)].map((m) => textOfHtml(m[1] ?? ""));
  const values = [...dl.matchAll(/<dd[^>]*>([\s\S]*?)<\/dd>/g)].map((m) => textOfHtml(m[1] ?? ""));
  return Object.fromEntries(terms.map((t, i) => [t, values[i] ?? ""]));
}

function inputTags(html: string): string[] {
  return [...html.matchAll(/<input[^>]*>/g)].map((m) => m[0]);
}

/** `<input name="scopes" …>` 들의 `value`. **그려진 체크박스**만 잡힌다. */
function scopeCheckboxes(html: string): string[] {
  return inputTags(html)
    .filter((tag) => tag.includes('name="scopes"'))
    .map((tag) => /value="([^"]*)"/.exec(tag)?.[1] ?? "")
    .filter(Boolean);
}

/**
 * 문단별 텍스트. **문장을 통째로 `toBe` 하려고** 낸다.
 *
 * `toContain` 으로 보면 이웃이 만족시킨다 — 이 파일에서 실제로 두 번 그랬다(빈 목록
 * 문구가 `node:pair` 를 갖고 있었다). 그리고 리뷰가 실측한 대로, `toContain` 은
 * 문단이 **통째로 사라진** 경우에도 다른 문단이 그 조각을 갖고 있으면 살아남는다.
 */
function pTexts(html: string): string[] {
  return [...html.matchAll(/<p[^>]*>([\s\S]*?)<\/p>/g)].map((m) => textOfHtml(m[1] ?? ""));
}

/** 모든 `href` 값. */
function hrefs(html: string): string[] {
  return [...html.matchAll(/href="([^"]*)"/g)].map((m) => m[1] ?? "");
}

beforeEach(() => {
  vi.clearAllMocks();
  actionStateOverride = undefined;
  lastActionArg = undefined;
  signedInAs("admin");
  listTokensMock.mockResolvedValue([]);
});

afterEach(() => {
  vi.useRealTimers();
});

// ──────────────────────────────────────────────────────────────────────────
// 등급 게이트 — 막히는 것과 깨지는 것은 다르다
// ──────────────────────────────────────────────────────────────────────────

describe("등급 게이트", () => {
  /**
   * ⚠ **`listTokens` 앞이어야 한다.** 서비스는 비-admin 의 서버 범위 호출에
   * **던지므로**, 게이트가 없거나 뒤에 있으면 화면이 막히는 게 아니라 **500 이
   * 난다.** 그래서 「리다이렉트됐다」만 보지 않고 **읽지 않았다**를 함께 붙든다.
   */
  it.each([
    ["editor", "editor"],
    ["viewer", "viewer"],
  ] as const)("%s 는 목록을 볼 수 없다 — 그리고 읽지도 않는다", async (_label, role) => {
    signedInAs(role);

    await expect(renderPage(ServerTokensPage)).rejects.toThrow("NEXT_REDIRECT");
    expect(readPageApiMock).toHaveBeenCalledExactlyOnceWith("/api/ui/v1/admin/tokens");
    expect(listTokensMock).not.toHaveBeenCalled();
  });

  it("로그인하지 않았으면 목록을 읽지 않는다", async () => {
    sessionUser = null;

    await expect(renderPage(ServerTokensPage)).rejects.toThrow("NEXT_REDIRECT");
    expect(listTokensMock).not.toHaveBeenCalled();
  });

  it("editor 는 발급 화면에 들어갈 수 없다", async () => {
    signedInAs("editor");

    await expect(renderPage(NewServerTokenPage)).rejects.toThrow("NEXT_REDIRECT");
    expect(readPageApiMock).toHaveBeenCalledExactlyOnceWith("/api/ui/v1/admin/tokens");
  });

  it("editor 는 상세를 볼 수 없다 — 그리고 읽지도 않는다", async () => {
    signedInAs("editor");
    listTokensMock.mockResolvedValue([row()]);

    await expect(
      renderPage(() => ServerTokenDetailPage({ params: Promise.resolve({ id: "tok-1" }) })),
    ).rejects.toThrow("NEXT_REDIRECT");
    expect(listTokensMock).not.toHaveBeenCalled();
  });




});

// ──────────────────────────────────────────────────────────────────────────
// 목록 페이지
// ──────────────────────────────────────────────────────────────────────────

describe("목록 페이지", () => {
  /**
   * ⚠ **해지된 행과 만료된 행을 섞는다.** 전부 `revokedAt: null`·`expiresAt: null`
   * 인 픽스처는 언제나 `active` 라, 페이지가 상태를 상수로 박아도 살아남는다
   * (Task 5 인계 9, 실측).
   */
  it("서버 범위로만 읽고, 읽은 것을 표로 그린다", async () => {
    listTokensMock.mockResolvedValue([
      row({ id: "tok-1", lastUsedAt: null, expiresAt: null }),
      row({
        id: "tok-2",
        name: "옛 노드 연결용",
        scopes: ["node:pair", "config:read"],
        lastUsedAt: null,
        expiresAt: null,
        revokedAt: new Date("2026-02-03T12:00:00.000Z"),
      }),
    ]);
    const html = await renderPage(ServerTokensPage);

    expect(readPageApiMock).toHaveBeenCalledExactlyOnceWith("/api/ui/v1/admin/tokens");
    // 날짜 문자열은 도는 곳의 TZ 를 타므로 **없을 때의 문구**로 붙든다.
    expect(bodyRows(html)).toEqual([
      ["node.example.com 연결용", MASKED, "node:pair", "사용한 적 없음", "무기한", "유효"],
      ["옛 노드 연결용", MASKED, "node:pair config:read", "사용한 적 없음", "무기한", "해지됨"],
    ]);
  });

  it("만료된 행을 만료됨으로 그린다 — 상태가 상수가 아니다", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(NOW);
    listTokensMock.mockResolvedValue([
      row({ lastUsedAt: null, expiresAt: new Date("2026-01-01T12:00:00.000Z") }),
    ]);

    expect(bodyRows(await renderPage(ServerTokensPage)).map((cells) => cells[5])).toEqual(["만료됨"]);
  });

  it("행 링크는 id 로 간다 — 가림 표시로 가지 않는다", async () => {
    listTokensMock.mockResolvedValue([row({ id: "tok-42" })]);
    const all = hrefs(await renderPage(ServerTokensPage));

    expect(all).toContain("/admin/tokens/tok-42");
    expect(all.some((h) => h.includes("cd12"))).toBe(false);
  });

  it("발급은 별도 페이지 링크다 — 목록 안에 폼이 없다", async () => {
    const html = await renderPage(ServerTokensPage);

    expect(hrefs(html)).toContain("/admin/tokens/new");
    expect(scopeCheckboxes(html)).toEqual([]);
  });

  it("비었으면 무엇이 없는지 말한다", async () => {
    expect(bodyRows(await renderPage(ServerTokensPage))).toEqual([
      [
        "아직 발급한 서버 토큰이 없습니다. 다른 게이트웨이가 이 서버에 연결하게 하려면 node:pair 권한을 가진 토큰을 발급해 그쪽에 전달하세요.",
      ],
    ]);
  });

  /**
   * `/admin/security` 의 「연결된 게이트웨이」와 **방향이 반대**다 — 그쪽은 남이 이
   * 서버에 붙은 것이고 이쪽은 이 서버가 발급한 것이라, 「지우면 무엇이 끊기나」가
   * 다르게 읽힌다. 그 구별을 화면이 실제로 말하는지 붙든다.
   */
  /**
   * ⚠ **방향**. 여기서 발급한 토큰으로 이 서버가 노드를 붙이는 것이 **아니다** —
   * `POST /api/v1/pair` 는 이 인스턴스 위의 엔드포인트이고 그것을 부르는 쪽이
   * 게이트웨이다. 이 문장이 거꾸로 적히면, 노드를 추가하려던 관리자가 아무 효과도
   * 없는 토큰을 만들고 **그 값은 자기 서버의 제어권을 넘기는 자격증명**이다.
   *
   * ⚠ **문장 통째로 본다.** 「node:pair 가 있다」만 보면 **빈 목록의 안내 문구**가
   * 그 낱말을 이미 갖고 있어 통과한다(인계 7 의 「이웃 조건」이 이 화면에서
   * 나타나는 모습이다 — 실제로 그렇게 썼다가 고쳤다).
   */
  it("다른 게이트웨이가 이 서버를 노드로 연결한다고 말한다 — 방향이 반대가 아니다", async () => {
    const paras = pTexts(await renderPage(ServerTokensPage));

    // ① **이 토큰이 무엇인가** — 방향 그 자체.
    expect(paras[0]).toContain(
      "이 서버가 발급한 서버 토큰입니다. node:pair 권한이 실린 토큰이 있어야 다른 게이트웨이가 이 서버를 자기 노드로 연결할 수 있습니다.",
    );
    // ② **평문은 한 번만 보인다.** 거짓이 되면 운영자가 다시 볼 수 없는 값을 저장하지 않는다.
    expect(paras[0]).toContain("토큰 값은 발급할 때 한 번만 보여줍니다.");
    // ③ **여기서 내준 것이 저기 한 줄이 된다** — 두 화면이 같은 관계의 두 단계라는 것.
    //    이 문장이 거꾸로 적히면 §1 의 결함이 그대로 돌아온다.
    expect(paras[1]).toContain(
      "여기서 내준 토큰은 상대가 연결에 성공하면 「보안 (2FA)」의 「연결된 게이트웨이」에 한 줄로 남습니다.",
    );
    // ④ 해지는 이 토큰과 연결 기록이 있는 게이트웨이만 함께 끊는다(tokens.rs 의 revoke).
    //    업데이트 전에 연결된 것은 그 연결 정보가 없어(0032 는 access_token_id 를 채우지
    //    않는다) 남으므로, 끊는 자리를 함께 말한다.
    expect(paras[1]).toContain(
      "토큰을 해지하면 이 토큰과 연결 기록이 있는 게이트웨이의 접근도 함께 해지됩니다. " +
        "업데이트 전에 연결된 게이트웨이는 이 토큰과의 연결 정보가 없어 함께 해지되지 않으니, " +
        "「보안 (2FA)」의 「연결된 게이트웨이」에서 해지하세요.",
    );
  });

  it("가림 표시만 그린다 — 평문 모양의 문자열이 없다", async () => {
    listTokensMock.mockResolvedValue([row()]);
    const html = await renderPage(ServerTokensPage);

    expect(html).toContain(MASKED);
    expect(html).not.toMatch(TOKEN_LIKE);
  });

  /**
   * 이 화면이 내놓는 **길 전부**를 통째로 본다 — 하나가 사라지거나 엉뚱한 곳으로
   * 바뀌면 그 자리에서 죽는다. `toContain` 만 두면 링크가 통째로 없어져도 초록이다.
   */
  /**
   * `← 관리자 패널로`(`/admin`)를 두지 않는다 — `app/admin/page.tsx` 가 즉시
   * `/admin/users` 로 리다이렉트해 라벨이 거짓이 되고, `href="/admin"` 을 가진
   * 관리자 화면은 이 화면 하나뿐이었다(나머지는 사이드바에 맡긴다).
   */
  it("제목과 나가는 길이 전부 제자리에 있다", async () => {
    const html = await renderPage(ServerTokensPage);

    expect(/<h1[^>]*>([\s\S]*?)<\/h1>/.exec(html)?.[1]).toBe("서버 액세스 토큰");
    expect(hrefs(html)).toEqual(["/admin/tokens/new"]);
    expect(textOfHtml(html)).toContain("새 서버 토큰");
  });
});

describe("사이드바", () => {
  /**
   * 관리자 메뉴 **전체**를 `toEqual` 한다. 「`/admin/tokens` 가 들어 있다」만 보면
   * 이 화면이 목록 맨 뒤로 밀려도, 다른 항목이 사라져도 초록이다.
   *
   * 자리는 「보안 (2FA)」 **다음**이다 — 이웃이지만 별항이라는 것이 이 화면의 성격
   * 그대로다(같은 주제, 반대 방향).
   */
  it("관리자 메뉴에 「액세스 토큰」이 보안 다음으로 있다", async () => {
    const { ADMIN_CATEGORIES } = await import("@/app/(app)/nav-links");
    const admin = ADMIN_CATEGORIES.find((c) => c.id === "admin");

    expect(admin?.items).toEqual([
      { href: "/admin/users", label: "사용자" },
      { href: "/admin/pending", label: "승인 대기" },
      { href: "/admin/oauth", label: "OAuth 로그인" },
      { href: "/admin/security", label: "보안 (2FA)" },
      { href: "/admin/tokens", label: "액세스 토큰" },
      { href: "/admin/secrets", label: "저장된 비밀값" },
    ]);
  });
});

// ──────────────────────────────────────────────────────────────────────────
// 발급 화면 — `node:pair` 가 사는 유일한 자리
// ──────────────────────────────────────────────────────────────────────────

describe("발급 화면", () => {
  it("서버 토큰 화면에는 node:pair 체크박스가 있다", async () => {
    const boxes = scopeCheckboxes(await renderPage(NewServerTokenPage));

    expect(boxes).toEqual(["config:read", "config:write", "nginx:reload", "user:manage", "node:pair"]);
  });

  /**
   * ⚠ **두 화면을 한 검사에서 그린다.** `node:pair` 가 이 화면에 **생겼다**와
   * 사용자 화면에서 **여전히 없다**는 한 쌍이고, 갈라 두면 어느 날 한쪽만 읽힌다.
   */
  it("node:pair 는 여기에만 있다 — 사용자 화면에는 여전히 없다", async () => {
    const server = scopeCheckboxes(await renderPage(NewServerTokenPage));
    const user = scopeCheckboxes(await renderPage(NewAccountTokenPage));

    expect(server).toContain("node:pair");
    expect(user).not.toContain("node:pair");
    // 사용자 화면이 통째로 비어서 통과하는 검사가 아니라는 것을 함께 붙든다.
    expect(user).toContain("config:read");
  });

  /**
   * `issuableScopes("server", role)` 은 **role 을 보지 않는다**(스펙: 그 권한은
   * 발급한 사람이 아니라 인스턴스에 속한다). 그래서 admin 제한은 화면과 서비스
   * 두 겹에 있고, 이 화면이 그 첫 겹이다.
   */
  it("서버 토큰 이름 예시가 나온다 — kind='server' 가 폼까지 간다", async () => {
    // 예시도 **받는 쪽**(게이트웨이)이다. 노드 호스트명을 두면 방향이 거꾸로 읽힌다.
    expect(await renderPage(NewServerTokenPage)).toContain('placeholder="gw.example.com 연결용"');
  });

  it("kind·ownerUserId·scope 를 폼에서 받지 않는다", async () => {
    const html = await renderPage(NewServerTokenPage);

    expect(html).not.toContain('name="kind"');
    expect(html).not.toContain('name="ownerUserId"');
    expect(html).not.toContain('name="scope"');
  });

  it("만료 드롭다운은 native select 가 아니다", async () => {
    const html = await renderPage(NewServerTokenPage);

    expect(html).not.toContain("<select");
    expect(html).toContain('name="expiry"');
  });

  it("발급 전에는 평문 자리가 아예 없다", async () => {
    const html = await renderPage(NewServerTokenPage);

    expect(html).not.toContain("이 값은 다시 볼 수 없습니다");
    expect(html).not.toMatch(TOKEN_LIKE);
  });

  it("제목과 안내가 무엇을 만드는 화면인지 말한다", async () => {
    const html = await renderPage(NewServerTokenPage);

    expect(/<h1[^>]*>([\s\S]*?)<\/h1>/.exec(html)?.[1]).toBe("새 서버 토큰");
    // ① 방향: 이 토큰을 **제시하는 쪽**이 다른 게이트웨이다. 이름에 적을 것도 그쪽이다.
    expect(textOfHtml(html)).toContain(
      "다른 게이트웨이가 이 서버를 노드로 연결할 때 제시하는 토큰입니다. 이름에 어느 게이트웨이에 줄 것인지 적어 두세요.",
    );
    expect(textOfHtml(html).split("다른 게이트웨이가 이 서버를 노드로 연결할 때 제시하는 토큰입니다.")).toHaveLength(2);
    // ② **평문은 한 번만 보인다.** 목록 리드와 같은 주장이고, 같은 이유로 붙든다.
    expect(textOfHtml(html)).toContain("발급이 끝나면 토큰 값을 한 번만 보여줍니다.");
    expect(textOfHtml(html)).not.toContain("서버 토큰 목록으로");
  });

  /**
   * ⚠ **이 화면의 액션이어야 한다.** 사용자 토큰 액션(`kind:"user"` 를 코드에 박은
   * 그것)을 넘겨도 화면은 완전히 정상으로 보인다 — 그리고 관리자가 누른 「발급」이
   * 사용자 토큰을 만든다. 마크업에 흔적이 없는 배선이라 동일성으로 붙든다.
   */
  it("폼에 이 화면의 발급 액션을 넘긴다", async () => {
    await renderPage(NewServerTokenPage);

    expect(lastActionArg).toEqual({ id: "admin.tokens.issue", args: [] });
  });
});

describe("발급 직후", () => {
  const ISSUED = { ok: true, token: PLAINTEXT, id: "tok-new", name: "node.example.com 연결용" };

  it("평문을 한 번만 그린다", async () => {
    actionStateOverride = ISSUED;
    const html = await renderPage(NewServerTokenPage);

    expect(html.split(PLAINTEXT)).toHaveLength(2); // 정확히 한 번 등장
    expect(html).toContain("이 값은 다시 볼 수 없습니다");
    expect(html).not.toContain(`value="${PLAINTEXT}"`);
  });

  /**
   * ⚠ **링크 전체를 `toEqual` 한다.** 「평문이 href 에 없다」만 보면 뒤로가기 링크
   * 하나로도 통과하고(Task 5 인계 7), 「완료」가 통째로 사라져도 초록이다.
   * 세 값은 각각 뒤로가기·완료·상세 보기다.
   */
  it("링크는 둘뿐이고 어느 것도 평문을 싣지 않는다", async () => {
    actionStateOverride = ISSUED;
    const all = hrefs(await renderPage(NewServerTokenPage));

    expect(all).toEqual(["/admin/tokens", "/admin/tokens/tok-new"]);
    expect(all.some((h) => h.includes(PLAINTEXT))).toBe(false);
  });

  it("결과 화면에는 입력 폼이 없다 — 다시 제출할 자리가 남지 않는다", async () => {
    actionStateOverride = ISSUED;

    expect(scopeCheckboxes(await renderPage(NewServerTokenPage))).toEqual([]);
  });
});

// ──────────────────────────────────────────────────────────────────────────
// 상세 페이지
// ──────────────────────────────────────────────────────────────────────────

describe("상세 페이지", () => {
  // fixture 가 상태를 지금 시각으로 계산한다(`tokenWireFixture`). fixture 만료일이
  // 지나는 날부터 「유효」가 「만료됨」으로 바뀌어 깨지지 않도록 Date 만 NOW 에 고정한다.
  beforeEach(() => {
    vi.useFakeTimers({ now: NOW, toFake: ["Date"] });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("찾은 행을 상세 카드로 그린다", async () => {
    listTokensMock.mockResolvedValue([
      row({ id: "tok-1" }),
      row({ id: "tok-2", name: "다른 노드 연결용", scopes: ["node:pair", "config:read"] }),
    ]);
    const html = await renderPage(() =>
      ServerTokenDetailPage({ params: Promise.resolve({ id: "tok-2" }) }),
    );
    const pairs = dlPairs(html);

    expect(readPageApiMock).toHaveBeenCalledExactlyOnceWith("/api/ui/v1/admin/tokens/tok-2");
    expect(Object.keys(pairs)).toEqual([
      "이름",
      "토큰",
      "상태",
      "권한",
      "만든 날",
      "마지막 사용",
      "만료",
      "해지",
    ]);
    expect(pairs["이름"]).toBe("다른 노드 연결용");
    expect(pairs["토큰"]).toBe(MASKED);
    expect(pairs["상태"]).toBe("유효");
    expect(pairs["권한"]).toBe("node:pair (노드 연결) config:read (설정 읽기)");
    expect(pairs["해지"]).toBe("해지되지 않음");
    // 날짜 셋은 TZ 를 타므로 **연도**로 붙든다 — 고정 시각이 정오(UTC)라 어떤 TZ 에서도
    // 연도가 넘어가지 않고, 셋이 서로 다른 연도라 칸이 뒤바뀌면 그대로 죽는다.
    expect(pairs["만든 날"]).toContain("2024");
    expect(pairs["마지막 사용"]).toContain("2025");
    expect(pairs["만료"]).toContain("2026");
    expect(html).toContain("토큰 해지");
    expect(html).not.toMatch(TOKEN_LIKE);
    expect(notFoundMock).not.toHaveBeenCalled();
  });

  it("해지된 행에는 해지 버튼이 없다 — 페이지가 상태를 그대로 넘긴다", async () => {
    listTokensMock.mockResolvedValue([
      row({ revokedAt: new Date("2026-02-03T12:00:00.000Z") }),
    ]);
    const html = await renderPage(() =>
      ServerTokenDetailPage({ params: Promise.resolve({ id: "tok-1" }) }),
    );

    expect(dlPairs(html)["상태"]).toBe("해지됨");
    expect(html).not.toContain("토큰 해지");
  });

  it("해지 폼은 id 만 보낸다 — 범위를 폼에서 받지 않는다", async () => {
    listTokensMock.mockResolvedValue([row({ id: "tok-7" })]);
    const html = await renderPage(() =>
      ServerTokenDetailPage({ params: Promise.resolve({ id: "tok-7" }) }),
    );

    expect(html).toContain('value="tok-7"');
    expect(html).not.toContain('name="kind"');
    expect(html).not.toContain('name="ownerUserId"');
  });

  it("제목과 나가는 길이 제자리에 있다", async () => {
    listTokensMock.mockResolvedValue([row({ id: "tok-1" })]);
    const html = await renderPage(() =>
      ServerTokenDetailPage({ params: Promise.resolve({ id: "tok-1" }) }),
    );

    expect(/<h1[^>]*>([\s\S]*?)<\/h1>/.exec(html)?.[1]).toBe("node.example.com 연결용");
    // 뒤로가기 · **실제로 끊는 화면**.
    // 가운데 것이 없으면 리드가 「거기서 끊으세요」라고만 말하고 길은 주지 않는다.
    expect(hrefs(html)).toEqual(["/admin/tokens", "/admin/security"]);
  });

  /**
   * ⚠ **이 검사가 리뷰가 찾은 사각지대다.** 이 문단을 통째로 지우거나 **정반대 뜻으로
   * 바꿔도** 122검사가 전부 통과했다(실측). 네 리드 중 셋은 붙들려 있었고 **틀린 하나만
   * 자유로웠다.**
   *
   * 무엇을 지키는가: 서버 토큰은 페어링 **한 번**에만 쓰이고, 성공하면 상대가
   * `gateway_tokens` 로 옮겨 탄다(스펙 §5·§11). 해지는 이 토큰과 연결 기록이 있는
   * `gateway_tokens` 행도 함께 해지한다(`apps/api/src/auth/tokens.rs` 의 revoke).
   * 업데이트 전에 연결된 게이트웨이는 이 토큰과의 연결 정보가 없어
   * (`0032_auth_compatibility.sql` 은 `access_token_id` 를 채우지 않는다) **남는다.**
   * 이 문장이 「모두 끊긴다」로 바뀌면, 침해된 이전 연결을 끊으려는 운영자가 해지하고
   * 「해지됨」을 보고 **끊었다고 믿는다** — 그리고 실제로 끊는 화면에서 멀어진다.
   *
   * 이 문단은 정보 아이콘이 아니라 **보이는 설명**이다(해지 버튼이 이 화면에 있다).
   * **문단을 통째로 `toBe` 한다.** `toContain` 은 문단이 사라져도 다른 문단이 조각을
   * 갖고 있으면 통과한다.
   */
  it("해지가 연결 기록이 있는 게이트웨이만 함께 끊는다고 말한다 — 업데이트 전 연결은 남는다", async () => {
    listTokensMock.mockResolvedValue([row({ id: "tok-1" })]);
    const html = await renderPage(() =>
      ServerTokenDetailPage({ params: Promise.resolve({ id: "tok-1" }) }),
    );

    const lead = pTexts(html).filter((text) => text.startsWith("해지하면"));
    expect(lead).toEqual([
      "해지하면 이 토큰으로는 더 이상 새로 연결할 수 없습니다. " +
        "이 토큰과 연결 기록이 있는 게이트웨이의 접근도 함께 해지됩니다. " +
        "업데이트 전에 연결된 게이트웨이는 이 토큰과의 연결 정보가 없어 함께 해지되지 않으니, " +
        "「보안 (2FA)」의 「연결된 게이트웨이」에서 해지하세요.",
    ]);
    // 보이는 설명이어야 한다: 정보 아이콘의 말풍선(role="tooltip") 안에 있으면 안 된다.
    const tooltip = /<div[^>]*role="tooltip"[^>]*>([\s\S]*?)<\/div>/.exec(html)?.[1] ?? "";
    expect(textOfHtml(tooltip)).not.toContain("해지하면");
    // 말만 하고 길을 주지 않으면 반쪽이다.
    expect(hrefs(html)).toContain("/admin/security");
  });

  /** 위 발급 액션과 같은 이유 — 배선이 마크업에 흔적을 남기지 않는다. */
  it("상세에 이 화면의 해지 액션을 넘긴다", async () => {
    listTokensMock.mockResolvedValue([row({ id: "tok-1" })]);
    await renderPage(() => ServerTokenDetailPage({ params: Promise.resolve({ id: "tok-1" }) }));

    expect(lastActionArg).toEqual({ id: "admin.tokens.revoke", args: [] });
  });

  it("서버 범위에 없는 id 는 404 다", async () => {
    listTokensMock.mockResolvedValue([row({ id: "tok-1" })]);

    await expect(
      renderPage(() => ServerTokenDetailPage({ params: Promise.resolve({ id: "nope" }) })),
    ).rejects.toThrow("NEXT_NOT_FOUND");
  });
});

// ──────────────────────────────────────────────────────────────────────────
// 서버 액션
// ──────────────────────────────────────────────────────────────────────────
