import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { isValidElement, type ReactNode } from "react";
import { ApplyResult } from "./apply-bar";
import { NodeErrorCard, type NodeErrorCardProps } from "./node-error";

/**
 * **이 태스크가 세운 규약 전체가 `ApplyResult` 의 `if` 하나에 걸려 있다** —
 * 노드 실패는 §9 컴포넌트로, 그 밖은 로컬 배너로. 그런데 그 두 갈래를 **바꿔
 * 끼워도** 타입은 통과하고 액션 쪽 검사(`stream-actions.test.ts`)도 전부 통과한다:
 * 액션이 무엇을 돌려주는지는 그대로고, **그리는 쪽만** 틀리기 때문이다. 그러면
 * 노드의 `nginx -t` 실패가 「적용 실패 — nginx -t 실패 (변경 사항은 롤백됨)」
 * 배너에 **빈 `<pre>`** 로 그려진다 — 게이트웨이가 하지도 관측하지도 않은 롤백을
 * 단언하면서 정작 노드가 낸 출력은 사라진다. 그 자리를 여기서 붙든다.
 *
 * `ApplyBar` 자체는 `useActionState` 때문에 렌더 환경 없이 부를 수 없다. 그래서
 * 분기만 순수 함수로 떼어냈고, 이 파일은 그걸 직접 불러 반환된 엘리먼트를
 * 확인한다(`components/node-error.test.tsx`·`lib/node-page.test.tsx` 와 같은 방식 —
 * jsdom 도 툴체인 변경도 필요 없다).
 *
 * **판별은 의미론으로 한다.** 노드 실패는 `type === NodeErrorCard`(어느
 * 컴포넌트가 골라졌나), 로컬 배너는 `role="status"` 를 단 `div`. CSS 모듈 클래스
 * 이름은 이 환경에서 무의미하므로 쓰지 않는다.
 */

function decoded(markup: string): string {
  return markup
    .replace(/&quot;/g, '"')
    .replace(/&#x27;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
}
function textOf(node: ReactNode): string {
  return decoded(renderToStaticMarkup(node).replace(/<[^>]+>/g, ""));
}
function preText(node: ReactNode): string | null {
  const match = renderToStaticMarkup(node).match(/<pre[^>]*>([\s\S]*?)<\/pre>/);
  return match ? decoded(match[1]!) : null;
}

const NGINX_OUTPUT =
  'nginx: [emerg] unknown directive "totally_bogus_directive_xyz"';

function nodeFailure(
  failure: NodeErrorCardProps["failure"],
): NodeErrorCardProps {
  return {
    failure,
    nodeId: "node-1",
    retryHref: "/nodes/node-1/stream",
    listHref: "/nodes/node-1/stream",
  };
}

describe("ApplyResult — 노드 실패는 §9 로, 로컬 결과는 배너로", () => {
  /**
   * 422 의 출력이 로컬 배너로 새면 두 가지가 동시에 일어난다: nginx 출력이
   * 사라지고(`result.output` 이 비어 있어 `<pre>` 가 안 그려진다), 하지도 않은
   * 롤백을 단언한다. 그래서 **`NodeErrorCard` 가 골라졌는가**를 본다 — 그
   * 컴포넌트가 `domain_failure` 를 `<pre>` 원문 배너로 그린다는 것은
   * `node-error.test.tsx` 가 따로 붙든다.
   */
  it("nodeError 가 있으면 NodeErrorCard 를 고른다 — 로컬 배너로 새지 않는다", () => {
    const nodeError = nodeFailure({
      kind: "domain_failure",
      message: NGINX_OUTPUT,
    });

    const rendered = ApplyResult({ result: { ok: false, nodeError } });

    if (!isValidElement(rendered)) throw new Error("엘리먼트가 아닙니다");
    expect(rendered.type).toBe(NodeErrorCard);
    // 실패 한 덩어리가 **그대로** 펼쳐져야 카드가 링크를 만들 수 있다.
    expect(rendered.props).toEqual(nodeError);
    // 로컬 배너의 표식을 달면 안 된다 — 두 갈래가 겹친 것이다.
    expect((rendered.props as { role?: string }).role).toBeUndefined();
  });

  it("닿지 못함도 같은 갈래로 간다 — 422 와 kind 만 다르다", () => {
    const nodeError = nodeFailure({
      kind: "unreachable",
      message: "연결이 거부되었습니다.",
    });

    const rendered = ApplyResult({ result: { ok: false, nodeError } });

    if (!isValidElement(rendered)) throw new Error("엘리먼트가 아닙니다");
    expect(rendered.type).toBe(NodeErrorCard);
    expect(rendered.props).toEqual(nodeError);
  });

  /**
   * 로컬 액션(`(app)/stream/actions.ts` 의 `applyConfig`)은 `nodeError` 를 채우는
   * 일이 없다. 추출 전 로컬 화면이 보여주던 문구가 **그대로** 나와야 한다.
   */
  it("nodeError 가 없는 실패는 로컬 배너 — 롤백 문구와 출력이 함께 온다", () => {
    const rendered = ApplyResult({
      result: { ok: false, output: NGINX_OUTPUT },
    });

    if (!isValidElement<{ role?: string }>(rendered))
      throw new Error("엘리먼트가 아닙니다");
    expect(renderToStaticMarkup(rendered)).toContain('role="status"');
    expect(rendered.props.role).toBe("status");
    expect(textOf(rendered)).toContain(
      "적용 실패 — nginx -t 실패 (변경 사항은 롤백됨)",
    );
    expect(preText(rendered)).toBe(NGINX_OUTPUT);
  });

  it("성공은 초록 배너 문구와 출력", () => {
    const output =
      "nginx: configuration file /etc/nginx/nginx.conf test is successful";

    const rendered = ApplyResult({ result: { ok: true, output } });

    if (!isValidElement<{ role?: string }>(rendered))
      throw new Error("엘리먼트가 아닙니다");
    expect(rendered.props.role).toBe("status");
    expect(textOf(rendered)).toContain("nginx -t 통과, reload 완료");
    expect(preText(rendered)).toBe(output);
  });

  it("출력이 없으면 <pre> 를 그리지 않는다", () => {
    const rendered = ApplyResult({ result: { ok: true } });

    expect(preText(rendered)).toBeNull();
  });

  it("아직 적용을 누르지 않았으면 아무것도 그리지 않는다", () => {
    expect(ApplyResult({ result: null })).toBeNull();
  });
});
