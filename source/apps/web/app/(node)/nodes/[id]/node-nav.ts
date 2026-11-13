import { PROXY_RESOURCES, type NavCategory } from "../../../(app)/nav-links";

/** Existing node sections; the four proxy resources share the local resource list. */
export function nodeNavCategories(nodeId: string): NavCategory[] {
  const base = `/nodes/${nodeId}`;
  return [
    {
      id: "overview",
      label: "개요",
      items: [{ href: base, label: "노드 요약" }],
    },
    // 로컬 PROXY_RESOURCES 의 "프록시" 카테고리와 같은 자리(개요 다음)·같은
    // 라벨·같은 순서(Reverse Proxy → Upstreams → Stream → Certificates).
    // 계획 4b 가 리소스를 하나씩 채웠고 이제 넷이 다 있다.
    {
      id: "proxy",
      label: "프록시",
      items: PROXY_RESOURCES.map((item) => ({
        ...item,
        href: base + item.href,
      })),
    },
    // Node sections keep their established order, independently of the global rail groups.
    {
      id: "monitoring",
      label: "모니터링",
      items: [
        { href: `${base}/metrics`, label: "접속 지표" },
        { href: `${base}/logs`, label: "로그" },
        { href: `${base}/audit`, label: "감사 로그" },
        { href: `${base}/history`, label: "적용 이력" },
      ],
    },
    {
      id: "settings",
      label: "설정",
      items: [{ href: `${base}/edit`, label: "연결 설정" }],
    },
  ];
}
