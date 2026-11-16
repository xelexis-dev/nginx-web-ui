import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { TokenListRow } from "@/lib/token-presentation";
import {
  tokenWireFixture,
  tokenScopeFixture,
} from "@/lib/token-api-fixtures.test-support";
import type { ExpiryPreset, TokenRowView } from "./tokens";

/**
 * `/account/tokens` 세 화면과 그 서버 액션.
 *
 * ── 무엇을 단언하는가 ────────────────────────────────────────────────────
 *
 * **prop 이 아니라 그려진 마크업**을 본다. 「폼에 `issuableScopes` 를 넘겼다」는
 * 절반이다 — 넘긴 것이 체크박스가 되는지, 그 목록에 `node:pair` 가 없는지는
 * 렌더된 HTML 에만 있다. 그래서 페이지(서버 컴포넌트)를 실제로 부르고
 * `renderToStaticMarkup` 으로 클라이언트 컴포넌트까지 끝까지 그린다. jsdom 도
 * 새 의존도 필요 없다(`react-dom` 은 이미 있다).
 *
 * **표의 칸과 정의목록의 쌍은 통째로 `toEqual` 한다.** 한 칸씩 `toContain` 으로
 * 훑으면 「그 칸이 사라졌다」를 못 본다 — 1차 리뷰가 목록의 권한 열과 상세 8행 중
 * 5행이 지워져도 초록인 것을 실측했다. 화면 컴포넌트가 `TokenRowView`(문자열만)를
 * 받으므로 기대값을 **글자 그대로** 쓸 수 있고, 그래서 칸이 하나 빠지거나 두 칸이
 * 뒤바뀌면 그 자리에서 죽는다.
 *
 * ── 훅을 왜 바꿔 끼우나 ─────────────────────────────────────────────────
 *
 * 서버 렌더는 언제나 **초기 상태**를 그린다. 그래서 「발급 직후」와 「복사 실패」에
 * 도달할 방법이 렌더 경로에 없다. 컴포넌트에 `initialState` 같은 뒷문을 뚫는 대신
 * **훅만** 바꿔 끼운다: 그러면 `if (state.token)` 분기까지
 * **진짜 컴포넌트**가 실행되고, 그 줄을 지우면 검사가 죽는다. 나머지(`useState` 의
 * 다른 호출 전부 포함)는 실제 React 그대로다.
 */

let actionStateOverride: unknown;
let copyStateOverride: "failed" | undefined;

vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react")>();
  return {
    ...actual,
    useState: (initial: unknown) =>
      copyStateOverride && initial === "idle"
        ? [copyStateOverride, () => {}]
        : actual.useState(initial as never),
    useActionState: (_action: unknown, initial: unknown) => [
      actionStateOverride ?? initial,
      () => {},
      false,
    ],
  };
});

const currentUserMock = vi.fn();
vi.mock("@/lib/guard", () => ({ currentUser: currentUserMock }));

const listTokensMock = vi.fn();
// API-response fixtures replace the old direct-DB page boundary.
// Backend role/ownership enforcement is covered by Rust auth_tokens_db/admin_http.
const readPageApiMock = vi.fn(async (path: string) => {
  const user = (await currentUserMock())?.user;
  if (!user || (path.includes("/admin/") && user.role !== "admin"))
    redirectMock("/login");

  const tokens = ((await listTokensMock()) as TokenListRow[]).map(
    tokenWireFixture,
  );
  const id = path.split("/tokens/")[1];
  if (id !== undefined)
    return (
      tokens.find((row) => row.id === decodeURIComponent(id)) ?? notFoundMock()
    );
  return {
    tokens,
    issuableScopes: tokenScopeFixture(
      path.includes("/admin/") ? "server" : "user",
      user!.role,
    ),
  };
});
vi.mock("@/lib/api-page", () => ({ readPageApi: readPageApiMock }));
vi.mock("@/lib/use-api-action", () => ({
  useApiAction: (action: unknown) => action,
}));

const redirectMock = vi.fn((_to: string) => {
  throw new Error("NEXT_REDIRECT");
});
const notFoundMock = vi.fn(() => {
  throw new Error("NEXT_NOT_FOUND");
});
vi.mock("next/navigation", () => ({
  redirect: redirectMock,
  notFound: notFoundMock,
}));

const { default: TokensPage } = await import("./page");
const { default: NewTokenPage } = await import("./new/page");
const { default: TokenDetailPage } = await import("./[id]/page");
const { TokenList } = await import("./token-list");
const { TokenDetail } = await import("./token-detail");
const { TokenForm } = await import("./token-form");

// ──────────────────────────────────────────────────────────────────────────
// 고정값
// ──────────────────────────────────────────────────────────────────────────

const NOW = new Date("2026-08-30T09:00:00.000Z");

/**
 * 평문처럼 생긴 문자열. 목록·상세가 평문을 흘리면 「가림 표시는 있다」만 보고
 * 통과하는 검사가 되지 않도록, 접두사부터 몸통까지 통째로 찾는 `TOKEN_LIKE` 로 본다.
 */
const PLAINTEXT = "nwu_u_dGVzdC1vbmx5LW5vdC1hLXJlYWwtdG9rZW4tdmFsdWUxcd12";
const MASKED = "nwu_u_…cd12";
/** 접두사 + 20자 이상의 몸통 = 평문의 모양. 가림 표시(`…`)는 여기 걸리지 않는다. */
const TOKEN_LIKE = /nwu_[us]_[A-Za-z0-9_-]{20,}/;

/** 서비스가 내는 행(= `Date` 를 든 원본). 페이지가 이것을 뷰로 바꾼다. */
function row(over: Partial<TokenListRow> = {}): TokenListRow {
  return {
    id: "tok-1",
    name: "노트북 CLI",
    scopes: ["config:read"],
    // **연도를 전부 다르게** 둔다 — 필드가 뒤바뀌면 그 자리에서 보인다.
    // 정오(UTC)라 어떤 TZ 에서도 연도가 넘어가지 않는다(검사가 TZ 에 안 흔들린다).
    createdAt: new Date("2024-01-02T12:00:00.000Z"),
    lastUsedAt: new Date("2025-06-07T12:00:00.000Z"),
    expiresAt: new Date("2026-12-01T12:00:00.000Z"),
    revokedAt: null,
    masked: MASKED,
    ...over,
  };
}

/**
 * 화면 컴포넌트가 받는 뷰. **날짜 칸이 서로 다른 글자열**이라 칸이 뒤바뀌거나
 * 빠지면 아래 `toEqual` 이 그대로 잡는다(TZ 에도 안 흔들린다 — 포맷을 안 거친다).
 */
function view(over: Partial<TokenRowView> = {}): TokenRowView {
  return {
    id: "tok-1",
    name: "노트북 CLI",
    masked: MASKED,
    scopes: ["config:read"],
    status: "active",
    createdAt: "만든날-값",
    lastUsedAt: "마지막사용-값",
    expiresAt: "만료-값",
    revokedAt: "해지-값",
    revoked: false,
    ...over,
  };
}

function signedInAs(role: "admin" | "editor" | "viewer") {
  currentUserMock.mockResolvedValue({
    session: { user: { id: "user-1" } },
    user: { id: "user-1", email: "me@example.com", role },
  });
}

/** 서버 컴포넌트를 실제로 부르고 끝까지 그린다. */
async function renderPage(page: () => Promise<ReactNode>): Promise<string> {
  return renderToStaticMarkup(await page());
}

const noopIssue = async () => ({ ok: false });
const noopRevoke = async () => ({ ok: false });

/**
 * 만료 선택지 하나를 고른 상태의 발급 폼. `defaultPreset` 은 검사용 뒷문이 아니라
 * 종류마다 알맞은 기본이 다를 수 있어 열어 둔 prop 이다(Task 6).
 *
 * **타입이 `ExpiryPreset` 인 것이 중요하다.** 1차 리뷰 때는 `"never" | "90d"` 라
 * `custom` 분기를 애초에 겨눌 수 없었고, 그래서 날짜 입력칸이 통째로 사라져도
 * 아무 검사도 죽지 않았다.
 */
function renderFormWithPreset(
  preset: ExpiryPreset,
  basePath = "/account/tokens",
): string {
  return renderToStaticMarkup(
    <TokenForm
      kind="user"
      scopes={["config:read"]}
      action={noopIssue}
      basePath={basePath}
      defaultPreset={preset}
    />,
  );
}

// ── 마크업 읽기 ───────────────────────────────────────────────────────────

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
    [...(tr[1] ?? "").matchAll(/<td[^>]*>([\s\S]*?)<\/td>/g)].map((td) =>
      textOfHtml(td[1] ?? ""),
    ),
  );
}

/** 정의목록의 term→value. 키 순서까지 봐야 행 삭제가 잡힌다. */
function dlPairs(html: string): Record<string, string> {
  const dl = /<dl[^>]*>([\s\S]*?)<\/dl>/.exec(html)?.[1] ?? "";
  const terms = [...dl.matchAll(/<dt[^>]*>([\s\S]*?)<\/dt>/g)].map((m) =>
    textOfHtml(m[1] ?? ""),
  );
  const values = [...dl.matchAll(/<dd[^>]*>([\s\S]*?)<\/dd>/g)].map((m) =>
    textOfHtml(m[1] ?? ""),
  );
  return Object.fromEntries(terms.map((t, i) => [t, values[i] ?? ""]));
}

/** `<input name="scopes" …>` 들의 `value`. **그려진 체크박스**만 잡힌다. */
function scopeCheckboxes(html: string): string[] {
  return inputTags(html)
    .filter((tag) => tag.includes('name="scopes"'))
    .map((tag) => /value="([^"]*)"/.exec(tag)?.[1] ?? "")
    .filter(Boolean);
}

function inputTags(html: string): string[] {
  return [...html.matchAll(/<input[^>]*>/g)].map((m) => m[0]);
}

/**
 * `<h1>{title}</h1>` **바로 옆** 정보 아이콘의 설명(`role="tooltip"`) 하나. 페이지
 * 설명문은 2026-11-02 부터 제목 옆 정보 아이콘에 있다(DESIGN.md 「페이지 설명」).
 *
 * **설명 단위로 잘라내는 것이 요점이다.** 화면 전체를 `toContain` 으로 훑으면,
 * 이 기능에는 글자까지 같은 문장이 다른 화면(`/account` 의 「액세스 토큰」 절)에도
 * 있어서 「이 문단이 통째로 없어졌다」를 못 본다. 잘라낸 문단을 **통째로** 비교하면
 * 한 문장이 빠지거나 뒤바뀌어도 그 자리에서 죽는다.
 */
function tipAfterTitle(html: string, title: string): string {
  const after = html.split(`${title}</h1>`)[1] ?? "";
  return textOfHtml(
    /<div[^>]*role="tooltip"[^>]*>([\s\S]*?)<\/div>/.exec(after)?.[1] ?? "",
  );
}

/** 그 id 를 가진 `<input>` 태그 전체. 속성(`value=`)을 보려고 태그째 낸다. */
function inputById(html: string, id: string): string {
  return inputTags(html).find((tag) => tag.includes(`id="${id}"`)) ?? "";
}

/** 모든 `href` 값. */
function hrefs(html: string): string[] {
  return [...html.matchAll(/href="([^"]*)"/g)].map((m) => m[1] ?? "");
}

beforeEach(() => {
  vi.clearAllMocks();
  actionStateOverride = undefined;
  copyStateOverride = undefined;
  signedInAs("editor");
  listTokensMock.mockResolvedValue([]);
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

// ──────────────────────────────────────────────────────────────────────────
// 발급 폼 — 어떤 scope 가 **그려지는가**
// ──────────────────────────────────────────────────────────────────────────

describe("발급 폼 — scope 체크박스", () => {
  it("node:pair 가 없다 — admin 이어도", async () => {
    signedInAs("admin");
    const boxes = scopeCheckboxes(await renderPage(NewTokenPage));

    expect(boxes).not.toContain("node:pair");
    // 목록 자체가 비어서 통과하는 검사가 아니라는 것을 함께 붙든다.
    expect(boxes).toContain("config:read");
  });

  it("admin 에게는 config:write·user:manage 가 있다", async () => {
    signedInAs("admin");
    const boxes = scopeCheckboxes(await renderPage(NewTokenPage));

    expect(boxes).toEqual([
      "config:read",
      "config:write",
      "nginx:reload",
      "user:manage",
    ]);
  });

  it("viewer 에게는 config:write 체크박스가 없다", async () => {
    signedInAs("viewer");
    const boxes = scopeCheckboxes(await renderPage(NewTokenPage));

    expect(boxes).not.toContain("config:write");
    expect(boxes).toEqual(["config:read"]);
  });

  it("editor 에게는 user:manage 가 없다", async () => {
    signedInAs("editor");
    const boxes = scopeCheckboxes(await renderPage(NewTokenPage));

    expect(boxes).not.toContain("user:manage");
    expect(boxes).toEqual(["config:read", "config:write", "nginx:reload"]);
  });

  it("scope 이름 옆에 사람 말이 함께 그려진다", async () => {
    signedInAs("viewer");
    const html = await renderPage(NewTokenPage);

    expect(html).toContain("config:read");
    expect(html).toContain("설정 읽기");
  });
});

describe("발급 폼 — 폼이 보내는 것", () => {
  it("kind·ownerUserId·scope 를 폼에서 받지 않는다", async () => {
    signedInAs("admin");
    const html = await renderPage(NewTokenPage);

    expect(html).not.toContain('name="kind"');
    expect(html).not.toContain('name="ownerUserId"');
    expect(html).not.toContain('name="scope"');
  });

  it("만료 드롭다운은 native select 가 아니다 — 공통 Select 의 hidden input 으로 제출된다", async () => {
    const html = await renderPage(NewTokenPage);

    expect(html).not.toContain("<select");
    expect(html).toContain('name="expiry"');
    expect(html).toContain("90일");
  });

  it("종류에 따라 이름 예시가 달라진다 — kind prop 이 실제로 쓰인다", () => {
    const user = renderToStaticMarkup(
      <TokenForm
        kind="user"
        scopes={["config:read"]}
        action={noopIssue}
        basePath="/account/tokens"
      />,
    );
    const server = renderToStaticMarkup(
      <TokenForm
        kind="server"
        scopes={["node:pair"]}
        action={noopIssue}
        basePath="/admin/tokens"
      />,
    );

    expect(user).toContain('placeholder="노트북 CLI"');
    // 서버 토큰의 이름에 실리는 정체는 **받는 쪽**(게이트웨이)이다 — 그 토큰은 다른
    // 게이트웨이가 이 인스턴스를 자기 노드로 붙일 때 제시한다(스펙 §8-4).
    // 노드 호스트명을 예시로 두면 방향이 거꾸로 읽힌다(Task 6 리뷰 실측).
    expect(server).toContain('placeholder="gw.example.com 연결용"');
  });

  it("새 서버 토큰은 무기한을 고를 수 없고 사용자 토큰은 유지한다", () => {
    const server = renderToStaticMarkup(
      <TokenForm
        kind="server"
        scopes={["node:pair"]}
        action={noopIssue}
        basePath="/admin/tokens"
        defaultPreset="never"
      />,
    );
    expect(server).toContain('name="expiry" value="90d"');
    expect(server).not.toContain("무기한");
    expect(renderFormWithPreset("never")).toContain(
      'name="expiry" value="never"',
    );
  });

  it("새 서버 토큰은 기본 editor 연결 권한을 미리 고른다", () => {
    const html = renderToStaticMarkup(
      <TokenForm
        kind="server"
        scopes={[
          "node:pair",
          "config:read",
          "config:write",
          "nginx:reload",
          "user:manage",
        ]}
        action={noopIssue}
        basePath="/admin/tokens"
      />,
    );
    const checked = [...html.matchAll(/<input\b[^>]*>/g)]
      .map((match) => match[0])
      .filter(
        (input) => input.includes('name="scopes"') && input.includes("checked"),
      )
      .map((input) => input.match(/value="([^"]+)"/)?.[1])
      .sort();
    expect(checked).toEqual(
      ["node:pair", "config:read", "config:write", "nginx:reload"].sort(),
    );
  });

  it("「취소」도 basePath 로 간다 — 경로가 박혀 있지 않다", () => {
    expect(hrefs(renderFormWithPreset("90d", "/admin/tokens"))).toEqual([
      "/admin/tokens",
    ]);
  });

  it("실패해도 다시 칠 필요가 없도록 입력이 제어형이다", () => {
    const html = renderFormWithPreset("custom");

    // 비제어 입력에는 `value` 속성이 아예 없다 — 제어형이면 빈 값이라도 나온다.
    expect(inputById(html, "token-name")).toContain('value=""');
    expect(inputById(html, "token-expires-on")).toContain('value=""');
  });
});

describe("발급 폼 — 만료 입력칸", () => {
  it("「직접 입력」을 고르면 날짜 입력칸이 나온다", () => {
    const html = renderFormWithPreset("custom");
    const dateInput = inputById(html, "token-expires-on");

    expect(dateInput).toContain('name="expiresOn"');
    expect(dateInput).toContain('type="date"');
  });

  /**
   * 문구와 파싱은 **함께** 움직여야 한다. `tokens.ts` 는 고른 날의 **끝**을 만료로
   * 잡으므로(자정이 아니다) 화면도 그렇게 말해야 한다.
   *
   * ⚠ **「오늘부터」가 없다는 것도 함께 단언한다.** 그 약속은 UTC **서쪽** 사용자에게
   * 하루 7–8시간 동안 거짓이다(실측: `America/Los_Angeles` 현지 17:00~,
   * `America/New_York` 20:00~). `toContain` 만 두면 뒤에 그 문장을 도로 붙여도
   * 통과하므로 부재를 따로 붙든다.
   */
  it("직접 입력 안내가 언제 만료되는지 말한다 — 지키지 못할 약속은 없다", () => {
    const html = renderFormWithPreset("custom");

    expect(html).toContain("고른 날짜가 끝나면 만료됩니다.");
    expect(html).not.toContain("오늘부터");
  });

  it("기간을 고르면 날짜 입력칸이 없다", () => {
    expect(renderFormWithPreset("90d")).not.toContain('name="expiresOn"');
    expect(renderFormWithPreset("never")).not.toContain('name="expiresOn"');
  });

  it("무기한을 고르면 경고를 띄운다 — 막지는 않는다", () => {
    const html = renderFormWithPreset("never");

    expect(html).toContain("무기한 토큰은");
    // 막지 않는다: 제출 버튼은 그대로 있다.
    expect(html).toContain("토큰 발급");
  });

  it("기간을 고르면 그 경고가 없다", () => {
    expect(renderFormWithPreset("90d")).not.toContain("무기한 토큰은");
  });
});

describe("발급 폼 — 오류 표시", () => {
  it("폼 레벨 오류를 그린다 — Task 6 의 액션이 쓰는 자리다", () => {
    actionStateOverride = {
      ok: false,
      error: "서버 토큰은 관리자만 발급할 수 있습니다.",
    };
    const html = renderFormWithPreset("90d");

    expect(html).toContain("토큰을 발급하지 못했습니다");
    expect(html).toContain("서버 토큰은 관리자만 발급할 수 있습니다.");
  });
});

// ──────────────────────────────────────────────────────────────────────────
// 발급 직후 — 평문이 존재하는 유일한 화면
// ──────────────────────────────────────────────────────────────────────────

describe("발급 직후", () => {
  const ISSUED = {
    ok: true,
    token: PLAINTEXT,
    id: "tok-new",
    name: "노트북 CLI",
  };

  it("평문을 한 번만 그리고 다시 볼 수 없다고 말한다", async () => {
    actionStateOverride = ISSUED;
    const html = await renderPage(NewTokenPage);

    expect(html.split(PLAINTEXT)).toHaveLength(2); // 정확히 한 번 등장
    expect(html).toContain("이 값은 다시 볼 수 없습니다");
  });

  it("평문을 value 속성에 담지 않는다 — 본문 텍스트로만 그린다", async () => {
    actionStateOverride = ISSUED;
    const html = await renderPage(NewTokenPage);

    expect(html).not.toContain(`value="${PLAINTEXT}"`);
    expect(html).toContain(`>${PLAINTEXT}<`);
  });

  it("평문을 링크에 싣지 않는다", async () => {
    actionStateOverride = ISSUED;
    const all = hrefs(await renderPage(NewTokenPage));

    expect(all.some((h) => h.includes(PLAINTEXT))).toBe(false);
  });

  /**
   * ⚠ **페이지가 아니라 폼만 그린다.** 페이지에는 자기 「← 토큰 목록으로」 백링크가
   * 있어서 `/account/tokens` 가 이미 나온다 — 1차 리뷰가 실측한 대로, 그 값으로
   * 「완료」를 대표시키면 **「완료」를 통째로 지워도 검사가 통과한다.**
   * `basePath` 도 일부러 다른 값을 넘겨 재사용 배선까지 함께 붙든다.
   */
  it("「완료」와 「상세 보기」가 basePath 아래로 간다 — 둘 다", () => {
    actionStateOverride = ISSUED;
    const html = renderFormWithPreset("90d", "/admin/tokens");

    expect(hrefs(html)).toEqual(["/admin/tokens", "/admin/tokens/tok-new"]);
  });

  it("발급 결과 화면에는 입력 폼이 없다 — 다시 제출할 자리가 남지 않는다", async () => {
    actionStateOverride = ISSUED;
    const html = await renderPage(NewTokenPage);

    expect(scopeCheckboxes(html)).toEqual([]);
  });

  it("발급 전에는 평문 자리가 아예 없다", async () => {
    const html = await renderPage(NewTokenPage);

    expect(html).not.toContain("이 값은 다시 볼 수 없습니다");
    expect(html).not.toMatch(TOKEN_LIKE);
  });

  it("announces manual-copy guidance when the one-time secret cannot be copied", () => {
    actionStateOverride = ISSUED;
    copyStateOverride = "failed";
    const html = renderFormWithPreset("90d");
    expect(html).toMatch(
      /<p[^>]*role="alert"[^>]*>복사할 수 없습니다 — 위 값을 직접 선택해 복사하세요\.<\/p>/,
    );
    expect(html).toContain(PLAINTEXT);
  });
  it("delegates the one-time value to the secret CodePane", () => {
    actionStateOverride = ISSUED;
    const html = renderFormWithPreset("90d");
    expect(html).toContain('data-variant="secret"');
    expect(html).toContain(PLAINTEXT);
    expect(html).toContain("복사");
  });
});

// ──────────────────────────────────────────────────────────────────────────
// 목록
// ──────────────────────────────────────────────────────────────────────────

describe("목록", () => {
  it("행의 여섯 칸을 전부 그린다 — 권한 열 포함", () => {
    const html = renderToStaticMarkup(
      <TokenList
        rows={[view({ scopes: ["config:read", "nginx:reload"] })]}
        basePath="/account/tokens"
      />,
    );

    expect(bodyRows(html)).toEqual([
      [
        "노트북 CLI",
        MASKED,
        "config:read nginx:reload",
        "마지막사용-값",
        "만료-값",
        "유효",
      ],
    ]);
  });

  it("가림 표시만 그린다 — 평문 모양의 문자열이 없다", () => {
    const html = renderToStaticMarkup(
      <TokenList rows={[view()]} basePath="/account/tokens" />,
    );

    expect(html).toContain(MASKED);
    expect(html).not.toMatch(TOKEN_LIKE);
  });

  it("행 링크는 id 로 간다 — masked 로 가지 않는다", () => {
    const html = renderToStaticMarkup(
      <TokenList rows={[view({ id: "tok-42" })]} basePath="/account/tokens" />,
    );

    expect(hrefs(html)).toContain("/account/tokens/tok-42");
    expect(hrefs(html).some((h) => h.includes("cd12"))).toBe(false);
  });

  it("basePath 아래로만 간다 — 경로가 박혀 있지 않다", () => {
    const html = renderToStaticMarkup(
      <TokenList rows={[view({ id: "tok-42" })]} basePath="/admin/tokens" />,
    );

    expect(hrefs(html)).toEqual(["/admin/tokens/tok-42"]);
  });

  it("상태 배지를 행마다 그린다", () => {
    const html = renderToStaticMarkup(
      <TokenList
        rows={[
          view({ id: "a", name: "유효한 것", status: "active" }),
          view({ id: "b", name: "지난 것", status: "expired" }),
          view({ id: "c", name: "끊긴 것", status: "revoked", revoked: true }),
        ]}
        basePath="/account/tokens"
      />,
    );

    expect(bodyRows(html).map((cells) => [cells[0], cells[5]])).toEqual([
      ["유효한 것", "유효"],
      ["지난 것", "만료됨"],
      ["끊긴 것", "해지됨"],
    ]);
  });

  it("비었으면 그 사실을 말한다", () => {
    const html = renderToStaticMarkup(
      <TokenList
        rows={[]}
        basePath="/account/tokens"
        emptyMessage="아직 만든 토큰이 없습니다."
      />,
    );

    expect(bodyRows(html)).toEqual([["아직 만든 토큰이 없습니다."]]);
  });
});

describe("목록 페이지", () => {
  it("세션 범위로만 읽고, 읽은 것을 표로 그린다", async () => {
    listTokensMock.mockResolvedValue([
      row({ lastUsedAt: null, expiresAt: null }),
    ]);
    const html = await renderPage(TokensPage);

    expect(readPageApiMock).toHaveBeenCalledExactlyOnceWith(
      "/api/ui/v1/account/tokens",
    );
    // 날짜 문자열은 도는 곳의 TZ 를 타므로 **없을 때의 문구**로 붙든다 — 그쪽이
    // TZ 와 무관하고, 칸이 비었는지도 함께 본다.
    expect(bodyRows(html)).toEqual([
      ["노트북 CLI", MASKED, "config:read", "사용한 적 없음", "무기한", "유효"],
    ]);
    expect(hrefs(html)).toContain("/account/tokens/tok-1");
  });

  it("요청한 액세스 토큰 설명을 표시한다", async () => {
    const html = await renderPage(TokensPage);

    expect(tipAfterTitle(html, "액세스 토큰")).toBe(
      "외부 서비스에서 이용 가능한 엑세스 토큰을 관리합니다",
    );
  });

  it("발급은 별도 페이지 링크다 — 목록 안에 폼이 없다", async () => {
    const html = await renderPage(TokensPage);

    expect(hrefs(html)).toContain("/account/tokens/new");
    expect(scopeCheckboxes(html)).toEqual([]);
  });

  it("로그인하지 않았으면 읽지 않는다", async () => {
    currentUserMock.mockResolvedValue(null);

    await expect(renderPage(TokensPage)).rejects.toThrow("NEXT_REDIRECT");
    expect(redirectMock).toHaveBeenCalledWith("/login");
    expect(listTokensMock).not.toHaveBeenCalled();
  });
});

// ──────────────────────────────────────────────────────────────────────────
// 상세
// ──────────────────────────────────────────────────────────────────────────

describe("상세", () => {
  it("정의목록 여덟 쌍을 전부 그린다", () => {
    const html = renderToStaticMarkup(
      <TokenDetail
        view={view({ scopes: ["config:read", "nginx:reload"] })}
        basePath="/account/tokens"
        revoke={noopRevoke}
      />,
    );

    expect(dlPairs(html)).toEqual({
      이름: "노트북 CLI",
      토큰: MASKED,
      상태: "유효",
      권한: "config:read (설정 읽기) nginx:reload (nginx 다시 읽기)",
      "만든 날": "만든날-값",
      "마지막 사용": "마지막사용-값",
      만료: "만료-값",
      해지: "해지-값",
    });
  });

  it("가림 표시만 그린다 — 평문은 여기 없다", () => {
    const html = renderToStaticMarkup(
      <TokenDetail
        view={view()}
        basePath="/account/tokens"
        revoke={noopRevoke}
      />,
    );

    expect(html).toContain(MASKED);
    expect(html).not.toMatch(TOKEN_LIKE);
  });

  it("해지 폼은 id 만 보낸다 — 범위를 폼에서 받지 않는다", () => {
    const html = renderToStaticMarkup(
      <TokenDetail
        view={view({ id: "tok-7" })}
        basePath="/account/tokens"
        revoke={noopRevoke}
      />,
    );

    expect(html).toContain('name="id"');
    expect(html).toContain('value="tok-7"');
    expect(html).not.toContain('name="kind"');
    expect(html).not.toContain('name="ownerUserId"');
  });

  it("해지된 토큰에는 해지 버튼이 없다", () => {
    const live = renderToStaticMarkup(
      <TokenDetail
        view={view()}
        basePath="/account/tokens"
        revoke={noopRevoke}
      />,
    );
    const dead = renderToStaticMarkup(
      <TokenDetail
        view={view({
          status: "revoked",
          revoked: true,
          revokedAt: "끊긴-시각",
        })}
        basePath="/account/tokens"
        revoke={noopRevoke}
      />,
    );

    expect(live).toContain("토큰 해지");
    expect(dead).not.toContain("토큰 해지");
    expect(dlPairs(dead)["상태"]).toBe("해지됨");
    expect(dlPairs(dead)["해지"]).toBe("끊긴-시각");
  });

  it("「목록으로」는 basePath 로 간다 — 경로가 박혀 있지 않다", () => {
    const html = renderToStaticMarkup(
      <TokenDetail
        view={view()}
        basePath="/admin/tokens"
        revoke={noopRevoke}
      />,
    );

    expect(hrefs(html)).toEqual(["/admin/tokens", "/admin/security"]);
  });

  it("해지 실패를 화면에 말한다", () => {
    actionStateOverride = {
      ok: false,
      error: "해지할 항목이 없습니다 — 이미 해지되었거나 없는 토큰입니다.",
    };
    const html = renderToStaticMarkup(
      <TokenDetail
        view={view()}
        basePath="/account/tokens"
        revoke={noopRevoke}
      />,
    );

    expect(html).toContain("해지하지 못했습니다");
    expect(html).toContain("이미 해지되었거나 없는 토큰입니다");
  });
});

describe("상세 페이지", () => {
  // fixture 가 상태를 지금 시각으로 계산한다(`tokenWireFixture`). fixture 만료일이
  // 지나는 날부터 「유효」가 「만료됨」으로 바뀌어 깨지지 않도록 Date 만 NOW 에 고정한다.
  beforeEach(() => {
    vi.useFakeTimers({ now: NOW, toFake: ["Date"] });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  /**
   * ⚠ **페이지가 `<TokenDetail>` 을 실제로 조립하는지**를 본다. 1차 리뷰는 그
   * 엘리먼트를 통째로 지워도 47검사와 `tsc --noEmit` 이 **둘 다 통과**하는 것을
   * 실측했다 — 그때 남아 있던 단언은 `listTokens` 인자·`notFound()`·그리고
   * 페이지 자신의 `<h1>` 에서 오는 이름 문자열뿐이었다.
   */
  it("찾은 행을 상세 카드로 그린다", async () => {
    listTokensMock.mockResolvedValue([
      row({ id: "tok-1" }),
      row({ id: "tok-2", name: "다른 것" }),
    ]);
    const html = await renderPage(() =>
      TokenDetailPage({ params: Promise.resolve({ id: "tok-2" }) }),
    );
    const pairs = dlPairs(html);

    expect(readPageApiMock).toHaveBeenCalledExactlyOnceWith(
      "/api/ui/v1/account/tokens/tok-2",
    );
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
    expect(pairs["이름"]).toBe("다른 것");
    expect(pairs["토큰"]).toBe(MASKED);
    expect(pairs["상태"]).toBe("유효");
    expect(pairs["권한"]).toBe("config:read (설정 읽기)");
    expect(pairs["해지"]).toBe("해지되지 않음");
    // 날짜 셋은 TZ 를 타므로 **연도**로 붙든다 — 고정 시각이 정오(UTC)라 어떤 TZ 에서도
    // 연도가 넘어가지 않고, 셋이 서로 다른 연도라 칸이 뒤바뀌면 그대로 죽는다.
    expect(pairs["만든 날"]).toContain("2024");
    expect(pairs["마지막 사용"]).toContain("2025");
    expect(pairs["만료"]).toContain("2026");
    expect(html).toContain("토큰 해지");
    // 왜 값을 다시 볼 수 없는지는 카드가 아니라 제목 옆 정보 아이콘이 말한다.
    expect(tipAfterTitle(html, "다른 것")).toContain(
      "토큰 값은 발급할 때 한 번만 보여줍니다. 저장된 것은 해시와 뒤 4자뿐이라 다시 볼 수 없습니다.",
    );
    expect(notFoundMock).not.toHaveBeenCalled();
  });

  it("내 범위에 없는 id 는 404 다 — 없는 것과 남의 것을 가르지 않는다", async () => {
    listTokensMock.mockResolvedValue([row({ id: "tok-1" })]);

    await expect(
      renderPage(() =>
        TokenDetailPage({ params: Promise.resolve({ id: "someone-elses" }) }),
      ),
    ).rejects.toThrow("NEXT_NOT_FOUND");
  });
});

// ──────────────────────────────────────────────────────────────────────────
// 서버 액션
// ──────────────────────────────────────────────────────────────────────────

// ──────────────────────────────────────────────────────────────────────────
// 만료 입력 해석
// ──────────────────────────────────────────────────────────────────────────
