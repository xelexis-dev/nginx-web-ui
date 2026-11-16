import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import { DEFAULT_DOMAIN_FAILURE_TITLE, NodeErrorCard } from "./node-error";

/**
 * **왜 헬퍼의 props 단언만으로는 부족한가.** `lib/node-page.test.tsx` 와
 * `lib/node-action.test.ts` 는 `retryHref`/`listHref` 가 **카드까지 온다**는 것을
 * 붙든다. 그런데 이 라운드가 고친 결함은 그 다음 한 칸에 있었다 — 카드가
 * 「목록으로 돌아가기」에 **`retryHref` 를 쓰던 것**. 헬퍼가 둘을 제대로 실어
 * 보내도 카드가 잘못된 쪽을 읽으면 사용자는 여전히 막다른 길을 본다.
 *
 * 이 저장소에는 컴포넌트 렌더 환경이 없다(vitest `environment: "node"`, jsdom
 * 없음). 하지만 `NodeErrorCard` 는 훅도 상태도 없는 순수 함수라 **직접 불러
 * 반환된 엘리먼트 트리를 훑으면** DOM 없이도 링크의 목적지를 확인할 수 있다.
 */

/**
 * 트리에서 `href` 를 가진 **첫** 엘리먼트의 `href` 와 텍스트를 찾는다.
 *
 * **첫 번째만 본다.** 지금은 어느 분기도 링크를 둘 이상 두지 않아 문제가 없지만,
 * 카드에 두 번째 이동(예: 「목록으로」 + 「다시 시도」)이 생기는 날 이 헬퍼는
 * 조용히 반쪽만 검사한다 — 그때는 전부 모으도록 고칠 것.
 */
function findLink(node: ReactNode): { href: string; text: string } | null {
  if (!isValidElement(node)) {
    if (Array.isArray(node)) {
      for (const child of node) {
        const found = findLink(child as ReactNode);
        if (found) return found;
      }
    }
    return null;
  }
  const props = (node as ReactElement<{ href?: unknown; children?: ReactNode }>)
    .props;
  if (typeof props.href === "string") {
    return {
      href: props.href,
      text: typeof props.children === "string" ? props.children : "",
    };
  }
  return findLink(props.children ?? null);
}

/**
 * 트리 전체의 문자열 조각을 이어 붙인다. 문구를 확인하는 단언용 — `findLink` 와
 * 달리 **전부** 모은다(제목은 링크 밖에 있다).
 */
function textOf(node: ReactNode): string {
  return renderToStaticMarkup(node)
    .replace(/<[^>]+>/g, "")
    .replace(/&quot;/g, '"')
    .replace(/&#x27;/g, "'")
    .replace(/&amp;/g, "&");
}

const REFS = {
  nodeId: "node-1",
  retryHref: "/nodes/node-1/reverse-proxy/upstreams/up-1",
  listHref: "/nodes/node-1/reverse-proxy/upstreams",
};

describe("NodeErrorCard — 이동 대상", () => {
  it("not_found 의 「목록으로 돌아가기」는 listHref 로 간다", () => {
    const link = findLink(
      NodeErrorCard({
        ...REFS,
        failure: { kind: "not_found", message: "대상을 찾을 수 없습니다." },
      }),
    );

    expect(link?.text).toBe("목록으로 돌아가기");
    // **`retryHref` 면 막다른 길이다** — 그 항목이 없어서 이 카드가 떴으므로
    // 같은 주소로 돌아가면 같은 카드가 다시 뜨고, 사이드바 말고는 나갈 길이
    // 없다. 상세·수정·삭제 화면 전부가 이 갈래를 밟는다.
    expect(link?.href).toBe(REFS.listHref);
  });

  it("unreachable 의 「다시 시도」는 retryHref 로 간다 — 둘을 뒤바꾸지 않는다", () => {
    const link = findLink(
      NodeErrorCard({
        ...REFS,
        failure: { kind: "unreachable", message: "연결이 거부되었습니다." },
      }),
    );

    expect(link?.text).toBe("다시 시도");
    // 재시도는 **이 화면을 다시 여는 것**이다. 목록으로 보내면 방금 하려던
    // 일에서 사용자를 쫓아낸다 — `not_found` 보다 훨씬 흔한 갈래다.
    expect(link?.href).toBe(REFS.retryHref);
  });

  it("node_error 의 「다시 시도」도 retryHref 로 간다", () => {
    const link = findLink(
      NodeErrorCard({
        ...REFS,
        failure: { kind: "node_error", message: "오류" },
      }),
    );

    expect(link?.text).toBe("다시 시도");
    expect(link?.href).toBe(REFS.retryHref);
  });

  /**
   * **경로가 셋인 화면이 생기면서 카드 하나가 조회 셋을 대표하게 됐다**(계획 4b
   * Task 4). 그때 「그 항목이 노드에 없습니다」는 사용자가 **눈앞의 항목**이
   * 사라진 것으로 읽는다 — 실제로는 인증서 조회가 404 였을 수 있고, 그 밑의
   * 「목록으로 돌아가기」가 그 오해를 확인해 준다. 카드가 무엇이 없는지 말하면
   * 그 링크는 다시 「지금 화면에서 빠져나가는 길」이라는 원래 뜻이 된다.
   */
  it("not_found 는 subject 를 받으면 무엇이 없는지 말한다", () => {
    const rendered = NodeErrorCard({
      ...REFS,
      failure: { kind: "not_found", message: "없습니다." },
      subject: "인증서 목록을",
    });

    expect(textOf(rendered)).toContain("인증서 목록");
    // 이동은 그대로 목록이다 — 실패한 조회의 리소스로 보내면 사용자를 하던
    // 일에서 쫓아낸다(`NodeErrorCardProps.subject` 문서).
    expect(findLink(rendered)?.href).toBe(REFS.listHref);
  });

  it("subject 가 없으면 예전 문구 그대로다 — 경로가 하나인 화면·쓰기 액션이 그 경로다", () => {
    const rendered = NodeErrorCard({
      ...REFS,
      failure: { kind: "not_found", message: "없습니다." },
    });

    expect(textOf(rendered)).toContain("그 항목이 노드에 없습니다");
  });

  it("unauthorized 는 둘 다 아니라 그 노드의 연결 설정으로 보낸다", () => {
    const link = findLink(
      NodeErrorCard({
        ...REFS,
        failure: { kind: "unauthorized", message: "끊김" },
      }),
    );

    expect(link?.href).toBe("/nodes/node-1/edit");
  });
});

/**
 * **배너와 카드의 구분은 이 컴포넌트가 스스로 「누군가 정리해 지울 것」이라고
 * 예고해 둔 자리다**(그 파일의 `kind` 별 생김새 문단). `conflict` 와
 * `version_mismatch` 는 둘 다 409 지만 뜻이 다르다 — 하나는 사용자가 고치면
 * 사라지는 일시적 거절(노드 상태를 안 건드린다), 하나는 다시 붙일 때까지 남는
 * 지속 상태다. 카드로 "정리" 하면 그 구분이 화면에서 지워진다.
 *
 * 판별은 `role="status"` 로 한다 — CSS 모듈 클래스 이름은 빌드마다 달라질 수
 * 있지만 이건 의미론적 표식이고 배너에만 있다.
 */
describe("NodeErrorCard — 배너와 카드는 다른 것이다", () => {
  const BANNERS = [
    {
      kind: "conflict" as const,
      message: "이 upstream을 사용 중인 서버가 있어 삭제할 수 없습니다.",
    },
    { kind: "rate_limited" as const, message: "요청이 너무 많습니다." },
    {
      kind: "domain_failure" as const,
      message: "nginx: [emerg] unknown directive",
    },
  ];

  it.each(BANNERS)(
    "$kind 는 배너다 — 톤의 live role을 쓰고 이동 링크를 달지 않는다",
    (failure) => {
      const rendered = NodeErrorCard({ ...REFS, failure });

      if (!isValidElement<{ role?: string }>(rendered))
        throw new Error("엘리먼트가 아닙니다");
      expect(rendered.props.role).toBe(failure.kind === "domain_failure" ? "alert" : "status");
      // 배너는 「폼/목록은 이미 떠 있고 액션 하나만 거부된」 경우다 — 화면을
      // 떠나는 링크를 달면 카드가 된다.
      expect(findLink(rendered)).toBeNull();
    },
  );

  it.each([
    { kind: "unreachable" as const, message: "연결이 거부되었습니다." },
    { kind: "not_found" as const, message: "없습니다." },
    { kind: "forbidden" as const, message: "권한이 없습니다." },
  ])("$kind 는 구분선 상태다 — 톤의 live role을 쓴다", (failure) => {
    const rendered = NodeErrorCard({ ...REFS, failure });

    if (!isValidElement<{ role?: string }>(rendered))
      throw new Error("엘리먼트가 아닙니다");
    expect(renderToStaticMarkup(rendered)).toContain(`role="${failure.kind === "unreachable" ? "alert" : "status"}"`);
  });

  /**
   * `validation` 은 폼 필드에 붙는 종류라 **아무것도 그리지 않는다.** 카드로
   * 그리면 `runNodeWrite` 가 `fieldErrors` 로 보낸 것과 화면에 둘이 뜨고,
   * 반대로 호출부가 실수로 넘겼을 때 조용히 사라지지 않도록 콘솔에 알린다.
   */
  it("validation 은 아무것도 그리지 않고 콘솔로 알린다", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});

    const rendered = NodeErrorCard({
      ...REFS,
      failure: { kind: "validation", message: "m", fieldErrors: {} },
    });

    expect(rendered).toBeNull();
    expect(warn).toHaveBeenCalledTimes(1);
    warn.mockRestore();
  });
});

/**
 * **422 는 `nginx -t` 실패 「등」이다**(스펙 §9 표). 노드는 인증서 갱신·검증
 * 시작·교체 실패도 같은 코드로 내는데, 제목이 고정이던 동안 그 실패들이
 * 「설정 적용 실패 — nginx -t 실패」로 그려졌다 — 본문(노드 원문)은 맞고 제목만
 * **게이트웨이가 관측하지도 않은 하위 시스템**을 가리켰다.
 */
describe("NodeErrorCard — domain_failure 제목", () => {
  const NGINX = {
    kind: "domain_failure" as const,
    message: "nginx: [emerg] unknown directive",
  };

  /** 적용·롤백은 실제로 `nginx -t` 를 거친 결과다 — 그 문구가 맞다. */
  it("안 주면 적용 경로의 문구가 그대로 나온다", () => {
    expect(textOf(NodeErrorCard({ ...REFS, failure: NGINX }))).toContain(
      DEFAULT_DOMAIN_FAILURE_TITLE,
    );
  });

  it("주면 그 문구가 나오고 nginx 를 이름 대지 않는다", () => {
    const text = textOf(
      NodeErrorCard({
        ...REFS,
        failure: {
          kind: "domain_failure",
          message: "acme: dns-01 challenge failed",
        },
        domainFailureTitle: "인증서 갱신 실패",
      }),
    );

    expect(text).toContain("인증서 갱신 실패");
    expect(text).not.toContain("nginx -t");
    // 사연은 제목이 아니라 노드가 준 원문이 나른다.
    expect(text).toContain("acme: dns-01 challenge failed");
  });

  /** 제목을 바꿔도 배너는 배너다 — 카드로 승격하면 화면을 떠나는 링크가 생긴다. */
  it("제목을 바꿔도 배너 모양은 그대로다", () => {
    const rendered = NodeErrorCard({
      ...REFS,
      failure: NGINX,
      domainFailureTitle: "인증서 갱신 실패",
    });

    if (!isValidElement<{ role?: string }>(rendered))
      throw new Error("엘리먼트가 아닙니다");
    expect(rendered.props.role).toBe("alert");
    expect(findLink(rendered)).toBeNull();
  });

  /** 다른 갈래는 이 prop 을 읽지 않는다 — 읽으면 카드 제목이 엉뚱하게 바뀐다. */
  it("domain_failure 가 아닌 갈래는 이 문구를 쓰지 않는다", () => {
    const text = textOf(
      NodeErrorCard({
        ...REFS,
        failure: { kind: "unreachable", message: "연결이 거부되었습니다." },
        domainFailureTitle: "인증서 갱신 실패",
      }),
    );

    expect(text).not.toContain("인증서 갱신 실패");
    expect(text).toContain("노드에 연결할 수 없습니다");
  });
});

/**
 * 계획 4b Task 8 실측(D-2)에서 나온 것: 이 세 갈래는 제목·본문만 그리고 **나갈
 * 링크가 없었다.** viewer 토큰으로 노드의 쓰기 화면을 열면 화면 전체가 한 줄이고
 * 사이드바 말고는 갈 곳이 없었다.
 *
 * **어디로 보내는지가 갈래마다 다르다** — 그게 이 묶음의 요점이다.
 */
describe("NodeErrorCard — 막다른 갈래에도 나갈 길이 있다", () => {
  it("forbidden 은 목록으로 보낸다 — 막힌 것은 이 동작이지 노드가 아니다", () => {
    const link = findLink(
      NodeErrorCard({
        ...REFS,
        failure: { kind: "forbidden", message: "권한 없음" },
      }),
    );

    // 등급이 모자라 막힌 것은 쓰기 화면이고 목록은 `config:read` 라 열린다 —
    // 되돌아오는 고리가 아니다.
    expect(link?.href).toBe(REFS.listHref);
  });

  it("version_mismatch 는 목록이 아니라 노드 밖으로 보낸다", () => {
    const link = findLink(
      NodeErrorCard({
        ...REFS,
        failure: {
          kind: "version_mismatch",
          message: "안 맞음",
          nodeVersion: "1.14.0",
        },
      }),
    );

    // 버전이 안 맞으면 이 노드의 **모든** 화면이 같은 카드를 낸다.
    // `listHref` 로 보내면 같은 카드로 돌아온다 — 그게 이 검사의 전부다.
    expect(link?.href).toBe("/nodes");
    expect(link?.href).not.toBe(REFS.listHref);
  });

  it("too_old 도 노드 밖으로 보낸다 — 형제 화면도 전부 같은 카드다", () => {
    const link = findLink(
      NodeErrorCard({
        ...REFS,
        failure: {
          kind: "too_old",
          message: "이 노드에는 노드 기능이 없습니다.",
        },
      }),
    );

    expect(link?.href).toBe("/nodes");
    expect(link?.href).not.toBe(REFS.listHref);
  });
});

it("returns no React child for an unknown runtime kind",()=>{
  const failure={kind:"future_kind",message:"unknown"} as unknown as import("@/lib/node-result").NodeFailure;
  expect(renderToStaticMarkup(<NodeErrorCard failure={failure} nodeId="n1" retryHref="/nodes/n1" listHref="/nodes"/>)).toBe("");
});

it("preserves the existing empty explanation paragraph on an unreachable card", () => {
  const html = renderToStaticMarkup(<NodeErrorCard {...REFS} failure={{ kind: "unreachable", message: "" }} />);
  expect(html.match(/<p\b/g)).toHaveLength(2);
});

it.each([
  [{ kind: "unreachable", message: "offline" }, "alert"],
  [{ kind: "unauthorized", message: "revoked" }, "alert"],
  [{ kind: "node_error", message: "failed" }, "alert"],
  [{ kind: "version_mismatch", message: "mismatch", nodeVersion: "3.0.0" }, "status"],
  [{ kind: "too_old", message: "old" }, "status"],
  [{ kind: "forbidden", message: "denied" }, "status"],
  [{ kind: "not_found", message: "missing" }, "status"],
  [{ kind: "domain_failure", message: "nginx failed" }, "alert"],
  [{ kind: "conflict", message: "busy" }, "status"],
  [{ kind: "rate_limited", message: "wait" }, "status"],
] as const)("announces %s once with the tone's live-region role", (failure, role) => {
  const html = renderToStaticMarkup(<NodeErrorCard {...REFS} failure={failure} />);
  expect(html.match(/role="(?:alert|status)"/g)).toEqual([`role="${role}"`]);
});
