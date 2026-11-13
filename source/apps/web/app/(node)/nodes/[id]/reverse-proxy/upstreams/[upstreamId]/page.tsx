import { PageHeader } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
import { can } from "@/lib/permissions";
import {
  UpstreamDetail,
  type UpstreamDetailVM,
} from "@/components/upstreams/upstream-detail";
import { loadNodePage } from "@/lib/node-page";

// 노드의 upstream 상세. `GET /api/v1/upstreams/:id` 하나로 그린다
// (app/api/v1/upstreams/[id]/route.ts 가 정본). 화면 본문은
// components/upstreams/upstream-detail.tsx — 로컬 상세와 같은 컴포넌트다.
//
// **라우트 파라미터 이름에 주의.** `[id]` 는 노드, `[upstreamId]` 는 이 화면이
// 보는 upstream 이다 — 둘 다 `[id]` 로 두면 Next 가 라우트를 거부하고, 이름을
// 헷갈리면 조용히 엉뚱한 걸 불러온다(노드 감사 상세와 같은 관용).
//
// 로컬은 행이 없으면 `notFound()` 지만 여기서는 노드가 404 를 내고
// `NodeErrorCard` 의 `not_found` 카드가 뜬다 — 스펙 §9 가 정한 모양이다.
export const runtime = "nodejs";

/** `GET /api/v1/upstreams/:id` 의 `upstream`. */
interface NodeUpstreamDetailResponse {
  upstream: {
    id: string;
    name: string;
    servers: unknown;
    resolveAtRuntime?: boolean;
  };
}

export default async function NodeUpstreamDetailPage({
  params,
}: {
  params: Promise<{ id: string; upstreamId: string }>;
}) {
  const { id, upstreamId } = await params;
  const basePath = `/nodes/${id}/reverse-proxy/upstreams`;

  const loaded = await loadNodePage<UpstreamDetailVM>({
    nodeId: id,
    action: "config:read",
    path: `/api/v1/upstreams/${encodeURIComponent(upstreamId)}`,
    // 「다시 시도」는 이 화면, 「목록으로 돌아가기」는 목록이다. 이 항목이
    // 사라져서 카드가 뜬 것이므로 되돌려 보내면 같은 카드가 다시 뜬다.
    retryHref: `${basePath}/${upstreamId}`,
    listHref: basePath,
    toViewModel: (raw) => {
      const { upstream } = raw as NodeUpstreamDetailResponse;
      return {
        id: upstream.id,
        name: upstream.name,
        resolveAtRuntime: upstream.resolveAtRuntime,
        servers: Array.isArray(upstream.servers)
          ? (upstream.servers as string[])
          : [],
      };
    },
  });

  if (!loaded.ok)
    return (
      <section className={layout.page}>
        <PageHeader title="Upstreams" />
        {loaded.render}
      </section>
    );

  // 수정/삭제를 보여줄지는 읽기 권한과 별개다. `resolveNode` 가 계산한
  // `effectiveRole`(= min(사용자, 토큰), 스펙 §8)을 그대로 쓴다 — 목록 화면과
  // 같은 이유로 손으로 재조립하지 않는다.
  const canWrite = can(loaded.effectiveRole, "config:write");

  return (
    <UpstreamDetail
      runtimeDns={loaded.runtimeDns}
      data={loaded.data}
      basePath={basePath}
      canWrite={canWrite}
      deleteAction={{ id: "node.deleteNodeUpstream", args: [id] }}
    />
  );
}
