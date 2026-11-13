import { describe, expect, it } from "vitest";
import { PROXY_RESOURCES } from "../../../(app)/nav-links";
import { nodeNavCategories } from "./node-nav";

/**
 * 노드 섹션 내비게이션가 **로컬과 같은 모양**이어야 한다는 계약(스펙 §7 — "여기는 저
 * 서버의 같은 화면"). 계획 4b 의 태스크들이 화면을 하나씩 만들면서 항목을 하나씩
 * 더해 왔는데, **그 항목들에 아무 검사도 없었다** — 한 줄을 지워도 전 검사가
 * 통과했다(계획 4b Task 6 리뷰 F5).
 *
 * 여기서 붙드는 것은 둘이다:
 *
 * 1. **프록시 항목 넷이 로컬과 같은 라벨·같은 순서**여야 한다. 순서가 갈리면
 *    같은 제품의 두 화면이 다른 메뉴로 보인다.
 * 2. **경로가 전부 이 노드 아래**여야 한다. 하나라도 로컬 주소가 새면 노드를
 *    보다가 게이트웨이 자신의 설정으로 넘어가는데, 링크는 멀쩡히 눌린다.
 *
 * `node-nav.ts:8-9` 의 「아직 만들지 않은 화면은 넣지 않는다」는 그대로 유효하다 —
 * 이 파일은 **이미 만든** 것이 빠지지 않는지만 본다. 최종 순서 대조는 Task 8 이다.
 */

const NODE_ID = "node-1";

function itemsOf(categoryId: string) {
  const category = nodeNavCategories(NODE_ID).find((c) => c.id === categoryId);
  if (!category) throw new Error(`카테고리가 없습니다: ${categoryId}`);
  return category.items;
}

function localItemsOf(categoryId: string) {
  expect(categoryId).toBe("proxy");
  return PROXY_RESOURCES;
}

describe("노드 섹션 내비게이션 — 프록시 계열 넷", () => {
  it("Reverse Proxy → Upstreams → Stream → Certificates 를 이 노드 아래로 그린다", () => {
    expect(itemsOf("proxy")).toEqual([
      { href: `/nodes/${NODE_ID}/reverse-proxy`, label: "리버스 프록시" },
      { href: `/nodes/${NODE_ID}/reverse-proxy/upstreams`, label: "Upstreams" },
      { href: `/nodes/${NODE_ID}/stream`, label: "Stream" },
      { href: `/nodes/${NODE_ID}/certificates`, label: "인증서" },
    ]);
  });

  /**
   * 라벨과 순서는 **로컬에서 온다.** 여기 값을 손으로 적어만 두면 로컬이 바뀌는
   * 날 둘이 조용히 갈린다 — 그래서 로컬 목록과 직접 맞춰 본다.
   */
  it("라벨과 순서가 로컬 프록시 메뉴와 같다", () => {
    expect(itemsOf("proxy").map((i) => i.label)).toEqual(
      localItemsOf("proxy").map((i) => i.label),
    );
  });

  it("프록시 항목의 꼬리가 로컬 경로와 같다 — basePath 치환 한 번으로 끝나야 한다", () => {
    expect(itemsOf("proxy").map((i) => i.href)).toEqual(
      localItemsOf("proxy").map((i) => `/nodes/${NODE_ID}${i.href}`),
    );
  });
});

describe("노드 섹션 내비게이션 — 모든 링크가 이 노드 아래다", () => {
  it("로컬 주소로 새는 항목이 없다", () => {
    const hrefs = nodeNavCategories(NODE_ID).flatMap((c) =>
      c.items.map((i) => i.href),
    );

    expect(hrefs.length).toBeGreaterThan(0);
    for (const href of hrefs) {
      expect(href.startsWith(`/nodes/${NODE_ID}`)).toBe(true);
    }
  });

  it("노드 id 를 경로에 실제로 쓴다 — 상수로 굳어 있으면 다른 노드가 같은 화면을 연다", () => {
    const other = nodeNavCategories("node-2").flatMap((c) =>
      c.items.map((i) => i.href),
    );

    expect(other.every((h) => h.startsWith("/nodes/node-2"))).toBe(true);
  });
});
