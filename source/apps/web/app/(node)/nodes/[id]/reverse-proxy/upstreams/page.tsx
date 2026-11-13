import Link from "next/link";
import { PROXY_TABS } from "@/app/(app)/nav-links";
import { PageHeader, SectionNav } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
import { can } from "@/lib/permissions";
import {
  UpstreamList,
  type UpstreamRowVM,
} from "@/components/upstreams/upstream-list";
import { loadNodePage } from "@/lib/node-page";

// 노드의 upstream 목록. `GET /api/v1/upstreams` 하나로 그린다
// (app/api/v1/upstreams/route.ts 가 정본). 화면 본문은
// components/upstreams/upstream-list.tsx — 로컬
// (app)/reverse-proxy/upstreams/page.tsx 와 같은 컴포넌트를 쓴다.
//
// **경로가 `/nodes/<id>/reverse-proxy/upstreams` 인 것이 요건이다.**
// `/nodes/<id>/upstreams` 로 두면 공유 컴포넌트의 `basePath` 치환이 한 번으로
// 안 끝나고, 화면 안의 링크가 노드에서 로컬로 새어나간다. 사이드바 강조도
// 최장 접두사로 고르므로(`(app)/nav-links.ts` 의 `activeHref`) 이 모양이어야
// 로컬과 같게 동작한다.
export const runtime = "nodejs";

/** `GET /api/v1/upstreams` 의 한 행. 노드는 `upstreams` 테이블 행을 그대로 준다. */
interface NodeUpstreamRow {
  id: string;
  name: string;
  /** jsonb 컬럼이 그대로 온다 — 배열이 아닐 수도 있어 VM 으로 옮길 때 좁힌다. */
  servers: unknown;
}

interface NodeUpstreamsResponse {
  upstreams: NodeUpstreamRow[];
}

export default async function NodeUpstreamsPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const basePath = `/nodes/${id}/reverse-proxy/upstreams`;

  const loaded = await loadNodePage<UpstreamRowVM[]>({
    nodeId: id,
    action: "config:read",
    path: "/api/v1/upstreams",
    retryHref: basePath,
    // 목록 화면이라 둘이 같은 곳이다 — 같은 값을 그대로 넘긴다
    // (우연이 아니라 단언, `NodeErrorCardProps` 문서).
    listHref: basePath,
    // 응답 모양이 어긋나면 여기서 던지고 `loadNodePage` 가 `node_error` 카드로
    // 접는다 — 방어 코드를 두지 않는다(Global Constraints).
    toViewModel: (raw) =>
      (raw as NodeUpstreamsResponse).upstreams.map((u) => ({
        id: u.id,
        name: u.name,
        servers: Array.isArray(u.servers) ? (u.servers as string[]) : [],
      })),
  });

  if (!loaded.ok)
    return (
      <section className={layout.page}>
        <PageHeader title="Upstreams" />
        {loaded.render}
      </section>
    );

  // 「+ Upstream 추가」를 보여줄지는 목록을 읽는 데 필요한 `config:read` 와
  // 별개로 판정한다. `resolveNode(id, "config:write")` 를 여기서 또 부르면
  // `requireRole` 이 viewer 를 통째로 /login 으로 보내 목록조차 못 보게 된다.
  //
  // **`effectiveRole` 을 손으로 다시 만들지 않는다.** `resolveNode` 가 이미
  // min(사용자, 토큰)을 계산해 `loadNodePage` 가 실어 준다(`NodePageData` 문서).
  // 여기서 `currentUser()` + `minRole` 로 재조립하면 §8 에 조건이 하나 늘 때
  // 버튼은 그려지는데 액션은 막고, 그 틈을 타입이 안 잡는다.
  const canWrite = can(loaded.effectiveRole, "config:write");

  return (
    <UpstreamList navigation={<SectionNav items={PROXY_TABS.filter(tab => tab.href !== "/reverse-proxy/templates").map(tab => ({ ...tab, href: `/nodes/${id}${tab.href}` }))} currentPath={basePath} label="프록시 구성" renderLink={link => <Link {...link} />} />} data={{ items: loaded.data, canWrite }} basePath={basePath} />
  );
}
