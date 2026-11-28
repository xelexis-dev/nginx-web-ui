import {
  Banner,
  CodePane,
  EmptyState,
  Facts,
  PageHeader,
  StatusBadge,
  Surface,
  Table,
  Td,
  Th,
  Tr,
} from "@repo/ui";
import { describe, expect, it } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import { CertificateDetail } from "./certificate-detail";
import { IssuancePoller } from "./issuance-poller";
import { IssueLogPanel } from "./issue-log-panel";
import type { CertificateDetailVM, IssueLogLineVM } from "./certificate-vm";

/**
 * **뷰모델이 맞아도 화면이 안 그리면 아무 일도 안 일어난다.** 노드 페이지
 * 검사(`(node)/nodes/[id]/certificates/**`)는 `loadNodePageAll` 에 무엇을
 * 넘기는지까지만 본다 — 폴러가 실제로 트리에 들어가는지, 연결 서버 링크가 어느
 * 경로로 만들어지는지는 여기서만 확인된다.
 *
 * `CertificateDetail` 은 훅도 상태도 없는 서버 컴포넌트라 **직접 불러 반환된
 * 엘리먼트 트리를 훑으면** DOM 없이 확인할 수 있다(`components/node-error.test.tsx`
 * ·`components/apply-bar.test.tsx` 와 같은 방식 — jsdom 도 툴체인 변경도 필요 없다).
 *
 * **판별은 의미론으로 한다** — `element.type === IssuancePoller`, `href` 값.
 * CSS 모듈 클래스는 이 환경(`environment: node`, CSS 처리 없음)에서 빈 객체라
 * `className` 단언은 무의미하다.
 */

type AnyElement = ReactElement<{ children?: ReactNode; href?: string; "data-tag"?: string }>;

/**
 * The shared presentational components this screen is built from (@repo/ui). They are plain
 * functions without hooks, so a check reads what they draw by calling them, as PageHeader always
 * was: the `dt`/`dd` pairs of Facts, the `tr` of Table rows, the titles of Surface and Banner.
 */
const EXPAND = new Set<unknown>([
  PageHeader,
  Surface,
  Facts,
  Banner,
  EmptyState,
  Table,
  Tr,
  Td,
  Th,
  StatusBadge,
]);
const expand = (el: AnyElement): ReactNode =>
  (el.type as unknown as (props: unknown) => ReactNode)(el.props);

/** 트리 전체를 훑어 조건에 맞는 엘리먼트를 전부 모은다. */
function collect(
  node: ReactNode,
  pick: (el: AnyElement) => boolean,
): AnyElement[] {
  if (Array.isArray(node))
    return node.flatMap((c) => collect(c as ReactNode, pick));
  if (!isValidElement(node)) return [];
  const el = node as AnyElement;
  if (EXPAND.has(el.type)) return collect(expand(el), pick);
  const here = pick(el) ? [el] : [];
  return [...here, ...collect(el.props.children ?? null, pick)];
}

function hrefs(tree: ReactNode): string[] {
  return collect(tree, (el) => typeof el.props.href === "string").map(
    (el) => el.props.href!,
  );
}

/** 트리 아래 모든 텍스트 — **값이 화면까지 갔는지**를 보는 검사가 쓴다. */
function textOf(node: ReactNode): string {
  if (node === null || node === undefined || typeof node === "boolean")
    return "";
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node))
    return node.map((c) => textOf(c as ReactNode)).join("");
  if (isValidElement(node) && EXPAND.has(node.type))
    return textOf(expand(node as AnyElement));
  if (isValidElement(node))
    return textOf((node as AnyElement).props.children ?? null);
  return "";
}

/**
 * 정의목록의 `dt` → `dd` 쌍. **텍스트 전체에 `toContain` 하는 것으로는 모자란다** —
 * 이 라운드에서 실패 횟수 `3` 이 포트 `8443` 안의 `3` 때문에 통과했고, 그래서
 * 「실패 횟수를 안 그린다」 변이가 살아남았다. 라벨에 붙여서 봐야 그 값이다.
 */
function dlPairs(tree: ReactNode): Record<string, string> {
  const cells = collect(tree, (el) => el.type === "dt" || el.type === "dd");
  const pairs: Record<string, string> = {};
  for (let i = 0; i + 1 < cells.length; i += 2) {
    pairs[textOf(cells[i]!)] = textOf(cells[i + 1]!);
  }
  return pairs;
}

const BASE = "/nodes/node-1/certificates";
const SERVER_BASE = "/nodes/node-1/reverse-proxy";

function vm(over: Partial<CertificateDetailVM> = {}): CertificateDetailVM {
  return {
    id: "cert-1",
    domains: { primary: "a.example.com", all: "a.example.com", extra: 0 },
    alias: null,
    status: "issued",
    source: "acme",
    staging: false,
    challengeType: "http-01",
    dnsProvider: null,
    issuedAt: new Date("2026-08-01T00:00:00.000Z"),
    expiresAt: new Date("2026-11-01T00:00:00.000Z"),
    lastAttemptAt: null,
    failureCount: 0,
    lastError: null,
    issueLog: [],
    dnsOrder: null,
    linkedServers: [{ id: "srv-1", serverName: "a.example.com", listen: 80 }],
    ...over,
  };
}

function render(
  over: Partial<CertificateDetailVM> = {},
  slots: { dnsPanel?: ReactNode; actions?: ReactNode } = {},
) {
  return CertificateDetail({
    data: vm(over),
    basePath: BASE,
    serverBasePath: SERVER_BASE,
    uncovered: [],
    dnsPanel: slots.dnsPanel ?? null,
    actions: slots.actions ?? null,
  });
}

function hasPoller(tree: ReactNode): boolean {
  return collect(tree, (el) => el.type === IssuancePoller).length > 0;
}

function issueLogPanel(
  tree: ReactNode,
): ReactElement<{ live: boolean }> | null {
  const found = collect(tree, (el) => el.type === IssueLogPanel);
  return (found[0] as ReactElement<{ live: boolean }> | undefined) ?? null;
}

describe("인증서 상세 — 자동 새로고침을 실제로 붙이는가", () => {
  it("발급 중에는 폴러를 그린다", () => {
    expect(hasPoller(render({ status: "issuing" }))).toBe(true);
  });

  it("끝난 인증서에는 폴러가 없다 — 있으면 아무도 안 보는 화면이 2초마다 노드를 부른다", () => {
    expect(hasPoller(render())).toBe(false);
  });

  /**
   * TXT 대기는 **사용자가 움직이기 전에는 아무것도 안 바뀌는** 상태다. 여기서
   * 폴러가 돌면 사람이 DNS 를 만지는 몇 분 동안 계속 조회한다.
   */
  it("TXT 대기 중에는 폴러가 없다", () => {
    expect(
      hasPoller(render({ dnsOrder: { phase: "awaiting-txt", records: [] } })),
    ).toBe(false);
  });

  it("verifying 단계에서는 폴러를 그린다", () => {
    expect(hasPoller(render({ dnsOrder: { phase: "verifying" } }))).toBe(true);
  });

  /** 폴러와 「(실시간)」 표시가 **같은 판정**을 써야 한다 — 갈리면 멈춘 로그에 실시간이라고 붙는다. */
  it("발급 로그의 「실시간」 표시가 폴러와 같이 움직인다", () => {
    const live = render({ status: "issuing" });
    expect(hasPoller(live)).toBe(true);
    expect(issueLogPanel(live)?.props.live).toBe(true);

    const done = render({
      issueLog: [{ at: "2026-08-01 00:00:01", level: "info", msg: "완료" }],
    });
    expect(hasPoller(done)).toBe(false);
    expect(issueLogPanel(done)?.props.live).toBe(false);
  });

  /** ACME 가 아니면 발급 로그가 아예 없다 — 수동 등록 인증서는 발급 과정이 없다. */
  it("수동 등록 인증서에는 발급 로그 패널이 없다", () => {
    expect(
      issueLogPanel(render({ source: "manual", status: "issuing" })),
    ).toBeNull();
  });
});

describe("인증서 상세 — 링크가 어디로 가는가", () => {
  /**
   * **연결된 서버 링크는 인증서 경로에서 파생하면 안 된다.** `${basePath}/srv-1`
   * 은 존재하지 않는 인증서를 찾고, 로컬 `/reverse-proxy/srv-1` 은 노드를 보다가
   * 게이트웨이 자신의 서버로 새어 나간다.
   */
  it("연결된 서버는 서버 경로로, 「← 인증서 목록」은 인증서 경로로 간다", () => {
    const all = hrefs(render());

    expect(all).toContain(`${SERVER_BASE}/srv-1`);
    expect(all).toContain(BASE);
    expect(all).not.toContain(`${BASE}/srv-1`);
  });
});

describe("인증서 상세 — 쓰기 슬롯", () => {
  /**
   * 슬롯이 트리에 안 들어가면 **로컬 화면의 액션 버튼과 DNS 패널이 통째로
   * 사라진다** — 화면은 멀쩡히 뜨고 타입도 통과한다. Task 7 이 노드 쪽을 채울 때
   * 기대는 것도 이 자리다.
   */
  it("넘긴 DNS 패널과 액션 행을 실제로 그린다", () => {
    const panel = <p data-slot="dns" />;
    const actions = <p data-slot="actions" />;
    const tree = render({}, { dnsPanel: panel, actions });

    const slots = collect(
      tree,
      (el) => el.type === "p" && "data-slot" in (el.props as object),
    );
    expect(slots).toHaveLength(2);
  });

  it("슬롯이 null 이면 아무것도 더 그리지 않는다", () => {
    const slots = collect(
      render(),
      (el) => el.type === "p" && "data-slot" in (el.props as object),
    );
    expect(slots).toHaveLength(0);
  });
});

/**
 * **값이 화면까지 가는가.** 위 검사들은 폴러·슬롯·href 만 본다 — 상태 문구를
 * 원시 enum 으로 흘리거나 실패 횟수를 안 그려도 통과한다. `certificate-list`
 * 쪽의 같은 이름 블록과 짝이다.
 */
describe("인증서 상세 — 값이 화면까지 가는가", () => {
  it("상태·도메인을 라벨에 붙은 값으로 그린다", () => {
    const tree = render({
      status: "awaiting-dns",
      staging: true,
      domains: {
        primary: "a.example.com",
        all: "a.example.com, b.example.com",
        extra: 1,
      },
    });
    const pairs = dlPairs(tree);

    const header = collect(tree, (el) => el.type === "header")[0]!;
    expect(textOf(header)).toContain("TXT 대기");
    expect(textOf(header)).not.toContain("awaiting-dns");
    expect(textOf(header)).toContain("STAGING");
    // 제목은 대표 도메인, 「도메인」 행은 전부 — 둘이 다른 값이라 자리를 갈라 본다.
    expect(pairs["도메인"]).toBe("a.example.com, b.example.com");
    expect(textOf(tree)).toContain("a.example.com");
  });

  /**
   * 발급 로그는 **자식 컴포넌트 안**이라 이 얕은 트리에는 글자로 안 나온다 —
   * 그 대신 패널이 받은 줄을 보고, 패널 자신이 그것을 그리는지는 바로 아래에서 본다.
   */
  it("발급 로그를 이미 화면 표기로 바뀐 줄로 패널에 넘긴다", () => {
    const panel = issueLogPanel(
      render({
        issueLog: [
          { at: "2026-08-29 00:00:01", level: "info", msg: "TXT 레코드 발급" },
        ],
      }),
    ) as ReactElement<{ entries: IssueLogLineVM[] }> | null;

    expect(panel?.props.entries).toEqual([
      { at: "2026-08-29 00:00:01", level: "info", msg: "TXT 레코드 발급" },
    ]);
  });

  it("패널이 그 줄을 그대로 그린다", () => {
    const panel = IssueLogPanel({
        entries: [
          { at: "2026-08-29 00:00:01", level: "info", msg: "TXT 레코드 발급" },
        ],
        live: true,
      });
    expect(panel.type).toBe(CodePane);
    const text = `${panel.props.title} ${panel.props.code}`;
    expect(text).toContain("[info]");
    expect(text).toContain("2026-08-29 00:00:01");
    expect(text).toContain("TXT 레코드 발급");
    expect(text).toContain("(실시간)");
  });

  it("로그가 비면 패널이 그렇게 말한다", () => {
    expect(textOf(IssueLogPanel({ entries: [], live: false }))).toContain(
      "아직 로그가 없습니다",
    );
  });

  /** 넷 다 이번에 원소 깊이로 읽게 된 값들이다 — 읽어만 놓고 안 그리면 의미가 없다. */
  it("실패 횟수·마지막 오류·연결 서버 셀을 그린다", () => {
    const tree = render({
      status: "failed",
      failureCount: 17,
      lastError: "acme: challenge failed",
      linkedServers: [
        { id: "srv-1", serverName: "a.example.com", listen: 8443 },
      ],
    });
    const text = textOf(tree);
    const rows = collect(tree, (el) => el.type === "tr").map(textOf);

    // **라벨에 붙여서 본다** — 전체 텍스트에 `toContain("17")` 은 다른 숫자
    // 안에서도 참이 될 수 있다(이 검사가 처음엔 그렇게 통과했다).
    expect(dlPairs(tree)["실패 횟수"]).toBe("17");
    expect(text).toContain("acme: challenge failed");
    expect(collect(tree, (el) => el.type === "h2").map(textOf)).toContain("연결된 서버");
    expect(collect(tree, (el) => el.type === "span" && el.props["data-tag"] === "").map(textOf)).toContain("1개");
    // 표의 두 셀이 한 행에 함께 있다.
    expect(
      rows.some((r) => r.includes("a.example.com") && r.includes("8443")),
    ).toBe(true);
  });

  /** 수동 등록은 ACME 가 아니다 — 마지막 시도·실패 횟수·오류 배너가 통째로 빠진다. */
  it("수동 등록에는 ACME 전용 행과 오류 배너를 그리지 않는다", () => {
    const tree = render({
      source: "manual",
      failureCount: 17,
      lastError: "acme: challenge failed",
    });

    expect(dlPairs(tree)["종류"]).toBe("수동 등록");
    expect(dlPairs(tree)["실패 횟수"]).toBeUndefined();
    expect(textOf(tree)).not.toContain("acme: challenge failed");
  });

  it("연결된 서버가 없으면 안내 문구를 그린다", () => {
    expect(textOf(render({ linkedServers: [] }))).toContain(
      "연결된 서버가 없습니다",
    );
  });

  /** 별칭은 사람이 붙인 이름이라 **제목**이 되고, 대표 도메인은 메타 줄에 남는다. */
  it("별칭이 있으면 제목과 「별칭」 행에 그린다", () => {
    const tree = render({ alias: "운영 API" });

    expect(dlPairs(tree)["별칭"]).toBe("운영 API");
    const heading = collect(tree, (el) => el.type === "h1").map(textOf);
    expect(heading).toEqual(["운영 API"]);
    const header = collect(tree, (el) => el.type === "header")[0]!;
    expect(textOf(header)).toContain("a.example.com");
  });

  it("별칭이 없으면 「별칭」 행이 없고 제목은 대표 도메인이다", () => {
    const tree = render({ alias: null });

    expect(dlPairs(tree)["별칭"]).toBeUndefined();
    expect(collect(tree, (el) => el.type === "h1").map(textOf)).toContain(
      "a.example.com",
    );
  });
});
