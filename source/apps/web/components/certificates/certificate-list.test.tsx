import { EmptyState, PageHeader, Td, Th, Tr, type PageHeaderProps } from "@repo/ui";
import { describe, expect, it } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import { CertificateList, statusTone } from "./certificate-list";
import type { CertificateListWrite } from "./certificate-list";
import type { CertificateRowVM } from "./certificate-vm";

/**
 * 목록이 **실제로 그리는 것**을 본다 — 행 링크의 목적지와 쓰기 버튼 둘.
 * 노드 페이지 검사는 `write` 에 무엇을 넘기는지까지고, 그 값이 버튼으로
 * 바뀌는지는 여기서만 확인된다.
 *
 * `CertificateList` 는 훅이 없는 서버 컴포넌트라 직접 불러 트리를 훑는다
 * (`certificate-detail.test.tsx` 와 같은 방식).
 */

type AnyElement = ReactElement<{ children?: ReactNode; href?: string }>;

function collect(node: ReactNode, pick: (el: AnyElement) => boolean): AnyElement[] {
  if (Array.isArray(node)) return node.flatMap((c) => collect(c as ReactNode, pick));
  if (!isValidElement(node)) return [];
  const el = node as AnyElement;
  if (el.type === PageHeader) return collect(PageHeader(el.props as unknown as PageHeaderProps), pick);
  const here = pick(el) ? [el] : [];
  return [...here, ...collect(el.props.children ?? null, pick)];
}

// A row, cell or header cell, whether written as the raw element or the shared @repo/ui one.
const SHARED: Record<string, unknown> = { tr: Tr, td: Td, th: Th };
const is = (el: AnyElement, tag: "tbody" | "tr" | "td" | "th") => el.type === tag || el.type === SHARED[tag];

function hrefs(tree: ReactNode): string[] {
  return collect(tree, (el) => typeof el.props.href === "string").map((el) => el.props.href!);
}

/** 트리 아래 모든 텍스트. **값이 화면까지 갔는지**를 보는 검사가 이걸 쓴다. */
function textOf(node: ReactNode): string {
  if (node === null || node === undefined || typeof node === "boolean") return "";
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map((c) => textOf(c as ReactNode)).join("");
  if (isValidElement(node) && node.type === PageHeader) return textOf(PageHeader(node.props as PageHeaderProps));
  if (isValidElement(node)) return textOf((node as AnyElement).props.children ?? null);
  return "";
}

/** 표 본문의 행별 텍스트. 행으로 갈라야 「이 인증서의 연결 서버 수」를 볼 수 있다. */
function rowTexts(tree: ReactNode): string[] {
  const bodies = collect(tree, (el) => is(el, "tbody"));
  return bodies.flatMap((tbody) =>
    collect(tbody.props.children ?? null, (el) => is(el, "tr")).map((tr) => textOf(tr)),
  );
}

/**
 * 행마다 `header` 열의 셀 텍스트. 행 전체 텍스트로 보면 다른 열의 숫자에 걸린다 — 만료 열의
 * `D-37` 이 연결 서버 수 `7` 로 읽혀, 날짜에 따라 검사가 깨지거나 헛되이 통과했다.
 */
function columnTexts(tree: ReactNode, header: string): string[] {
  const at = collect(tree, (el) => is(el, "th")).map((th) => textOf(th)).indexOf(header);
  if (at < 0) throw new Error(`열이 없습니다: ${header}`);
  return collect(tree, (el) => is(el, "tbody")).flatMap((tbody) =>
    collect(tbody.props.children ?? null, (el) => is(el, "tr")).map((tr) => {
      const cells = collect(tr.props.children ?? null, (el) => is(el, "td"));
      return textOf(cells[at] ?? null);
    }),
  );
}

const BASE = "/nodes/node-1/certificates";

const ROW: CertificateRowVM = {
  id: "cert-1",
  domains: { primary: "a.example.com", all: "a.example.com", extra: 0 },
  alias: null,
  status: "issued",
  source: "acme",
  staging: false,
  expiresAt: new Date("2026-11-01T00:00:00.000Z"),
  linkedCount: 2,
};

function row(over: Partial<CertificateRowVM> = {}): CertificateRowVM {
  return { ...ROW, ...over };
}

/**
 * 오늘 기준 N일 뒤/전. 만료 열은 `Date.now()` 를 쓰므로 고정 날짜로는 못 본다.
 *
 * **1분을 얹는 것이 요점이다.** `daysLeft` 는 이 값을 만든 뒤 **조금 지나서**
 * `Date.now()` 를 부르고 그 차이를 내림한다 — 딱 N일로 만들면 그 몇 ms 때문에
 * N-1 이 나와 검사가 가끔 깨진다(이 라운드에서 실제로 두 번 깨졌다).
 */
function inDays(n: number): Date {
  return new Date(Date.now() + n * 24 * 60 * 60 * 1000 + 60_000);
}

function render(write: CertificateListWrite | null, items: CertificateRowVM[] = [ROW]) {
  return CertificateList({ data: { items }, basePath: BASE, write });
}

describe("인증서 목록 — 행 링크", () => {
  it("행이 이 화면의 경로 아래로 간다 — 로컬 주소로 새지 않는다", () => {
    const all = hrefs(render(null));

    expect(all).toContain(`${BASE}/cert-1`);
    expect(all).not.toContain("/certificates/cert-1");
  });
});

describe("인증서 목록 — 쓰기 버튼", () => {
  /**
   * **목적지를 값으로 들고 있는 것이 요점이다**(`CertificateListWrite` 문서).
   * 컴포넌트가 `${basePath}/new` 로 파생하면, 그 경로가 아직 없는 화면도
   * `write` 만 채우면 버튼이 그려지고 아무 검사도 그걸 못 잡는다.
   */
  it("넘긴 주소 그대로 두 버튼을 그린다", () => {
    const all = hrefs(render({ newHref: "/certificates/new", importHref: "/certificates/import" }));

    expect(all).toContain("/certificates/new");
    expect(all).toContain("/certificates/import");
  });

  /** 가져오기 화면이 없는 곳(노드)에서는 그 버튼만 사라진다 — 발급은 남는다. */
  it("importHref 가 null 이면 가져오기 버튼만 사라진다", () => {
    const all = hrefs(render({ newHref: `${BASE}/new`, importHref: null }));

    expect(all).toContain(`${BASE}/new`);
    expect(all.filter((h) => h.endsWith("/import"))).toEqual([]);
  });

  it("write 가 null 이면 둘 다 사라진다", () => {
    const all = hrefs(render(null));

    expect(all.filter((h) => h.endsWith("/new") || h.endsWith("/import"))).toEqual([]);
  });
});

/**
 * **여기부터가 「뷰모델이 화면까지 갔는가」다.** 위 검사들은 href 와 유무만 보므로,
 * 상태 문구를 원시 enum 으로 바꾸거나 연결 서버 수를 `0` 으로 박아도 통과한다
 * (리뷰가 그 셋을 한꺼번에 바꿔 283건 전부 초록인 것을 확인했다). 값 자체를 본다.
 */
describe("인증서 목록 — 값이 화면까지 가는가", () => {
  it("상태를 사람이 읽는 문구로 그린다 — 원시 enum 이 새면 안 된다", () => {
    const cells = rowTexts(render(null, [row({ status: "awaiting-dns" })]));

    expect(cells[0]).toContain("TXT 대기");
    expect(cells[0]).not.toContain("awaiting-dns");
  });

  it("연결 서버 수를 그 인증서의 값으로 그린다", () => {
    const linked = columnTexts(
      render(null, [row({ linkedCount: 7 }), row({ id: "cert-2", linkedCount: 0 })]),
      "연결 서버",
    );

    // 각 행이 자기 값을 그린다 — 하드코딩이거나 두 번째 행이 첫 행 값을 물려받으면 둘이 같아진다.
    expect(linked).toEqual(["7", "0"]);
  });

  /** STAGING 은 「이 인증서는 브라우저가 신뢰하지 않는다」는 경고다 — 빠지면 조용히 위험해진다. */
  it("staging 일 때만 STAGING 배지를 그린다", () => {
    expect(rowTexts(render(null, [row({ staging: true })]))[0]).toContain("STAGING");
    expect(rowTexts(render(null, [row({ staging: false })]))[0]).not.toContain("STAGING");
  });

  it("수동 등록만 「수동」 배지를 단다", () => {
    expect(rowTexts(render(null, [row({ source: "manual" })]))[0]).toContain("수동");
    expect(rowTexts(render(null, [row({ source: "acme" })]))[0]).not.toContain("수동");
  });

  it("도메인이 여럿이면 「외 N」이 붙는다", () => {
    const cells = rowTexts(
      render(null, [row({ domains: { primary: "a.example.com", all: "a, b, c", extra: 2 } })]),
    );

    expect(cells[0]).toContain("a.example.com");
    expect(cells[0]).toContain("외 2");
  });

  /** 만료 열은 세 갈래다 — 남은 일수 / 「만료」 / 「—」. 셋이 갈리는지 본다. */
  it("만료를 남은 일수·만료·없음 셋으로 가른다", () => {
    expect(rowTexts(render(null, [row({ expiresAt: inDays(61) })]))[0]).toContain("D-61");
    expect(rowTexts(render(null, [row({ expiresAt: inDays(-3) })]))[0]).toContain("만료");
    expect(rowTexts(render(null, [row({ expiresAt: inDays(-3) })]))[0]).not.toContain("D-");
    expect(rowTexts(render(null, [row({ expiresAt: null })]))[0]).toContain("—");
  });

  it("한 건도 없으면 빈 표 문구를 그린다", () => {
    const empty = collect(render(null, []), (el) => el.type === EmptyState);
    expect(empty.map((el) => (el.props as { description?: string }).description)).toEqual(["발급된 인증서가 없습니다."]);
    const cells = collect(render(null, []), (el) => is(el, "td"));
    expect(cells).toHaveLength(1);
    expect((cells[0]!.props as { colSpan?: number }).colSpan).toBe(4);
  });

  /** 별칭은 행에서 **도메인과 함께** 보인다 — 서버 폼의 Select 라벨과 같은 두 값이다. */
  it("별칭이 있으면 별칭과 대표 도메인을 함께 그린다", () => {
    const cells = rowTexts(render(null, [row({ alias: "운영 API" }), row({ id: "cert-2", alias: null })]));

    expect(cells[0]).toContain("운영 API");
    expect(cells[0]).toContain("a.example.com");
    // 별칭 없는 행이 다른 행의 별칭을 물려받지 않는다.
    expect(cells[1]).not.toContain("운영 API");
  });
});

it("issuing uses the shared info status tone", () => {
  expect(statusTone("issuing")).toBe("info");
});
